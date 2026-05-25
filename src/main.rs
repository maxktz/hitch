use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::env;
use std::ffi::{CString, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::mem;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const BUF_SIZE: usize = 4096;
const MSG_PUSH: u8 = 0;
const MSG_ATTACH: u8 = 1;
const MSG_DETACH: u8 = 2;
const MSG_WINCH: u8 = 3;
const MSG_DETACH_SESSION: u8 = 4;
const SKILL_INSTALL_URL: &str = "https://github.com/maxktz/hitch";
const SKILL_NAME: &str = "hitch";

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const ATTACH_HISTORY_BYTES: u64 = 64 * 1024;
const ACTIVE_COMMAND_HEAD_LINES: usize = 5;
const EXIT_PARENT_CODE: i32 = 42;
static WINCH_PENDING: AtomicBool = AtomicBool::new(false);

struct Style {
    enabled: bool,
}

#[derive(Parser)]
#[command(name = "hitch", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Start,
    Stop,
    List(ListArgs),
    SendKeys(SendKeysArgs),
    Capture(CapturePaneArgs),
    #[command(hide = true)]
    CapturePane(CapturePaneArgs),
    Kill(SessionArg),
    #[command(hide = true)]
    KillSession(SessionArg),
    #[command(hide = true)]
    KillSessions(SessionArg),
    Status(StatusArgs),
    Init,
    #[command(hide = true)]
    InitPrompt,
    Setup(SetupArgs),
}

#[derive(Args)]
struct SetupArgs {
    #[command(subcommand)]
    command: SetupCommand,
}

#[derive(Subcommand)]
enum SetupCommand {
    /// Install shell integration.
    Shell,
    /// Install the optional agent skill.
    Skill,
}

#[derive(Args)]
struct ListArgs {
    #[arg(long)]
    all: bool,
    #[arg(long)]
    dir: Option<PathBuf>,
    #[arg(long, default_value_t = 20)]
    tail: usize,
}

#[derive(Args)]
struct SessionArg {
    #[arg(value_name = "TERMINAL")]
    session: String,
}

#[derive(Args)]
struct SendKeysArgs {
    #[arg(short = 't', long = "target")]
    target: String,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    keys: Vec<String>,
}

#[derive(Args)]
struct CapturePaneArgs {
    #[arg(short = 't', long = "target")]
    target: String,
    #[arg(short = 'p')]
    print: bool,
    #[arg(short = 'S', allow_hyphen_values = true)]
    start: Option<isize>,
    #[arg(long, hide = true)]
    tail: Option<usize>,
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
struct StatusArgs {
    #[arg(long)]
    debug: bool,
}

impl Style {
    fn plain() -> Self {
        Self { enabled: false }
    }

    fn stdout() -> Self {
        Self {
            enabled: env::var_os("NO_COLOR").is_none()
                && unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1,
        }
    }

    fn stderr() -> Self {
        Self {
            enabled: env::var_os("NO_COLOR").is_none()
                && unsafe { libc::isatty(libc::STDERR_FILENO) } == 1,
        }
    }

    fn paint(&self, value: impl AsRef<str>, codes: &[&str]) -> String {
        let value = value.as_ref();
        if !self.enabled {
            return value.to_string();
        }

        format!("{}{}{}", codes.join(""), value, RESET)
    }

    fn brand(&self) -> String {
        self.paint("hitch", &[GREEN])
    }

    fn id(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[GREEN])
    }

    fn path(&self, value: impl AsRef<str>) -> String {
        value.as_ref().to_string()
    }

    fn command(&self, value: impl AsRef<str>) -> String {
        value.as_ref().to_string()
    }

    fn muted(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[DIM])
    }

    fn error(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[RED, BOLD])
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionRecord {
    id: String,
    cwd: String,
    socket: String,
    log: String,
    #[serde(rename = "pidFile")]
    pid_file: String,
    #[serde(rename = "masterPidFile")]
    master_pid_file: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    shell: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionState {
    #[serde(rename = "activeCommand")]
    active_command: Option<String>,
    #[serde(rename = "commandRunning")]
    command_running: bool,
    #[serde(rename = "commandStartedAt")]
    command_started_at: Option<u64>,
    #[serde(rename = "commandFinishedAt")]
    command_finished_at: Option<u64>,
    #[serde(rename = "lastActivityAt")]
    last_activity_at: Option<u64>,
    #[serde(rename = "foregroundPgrp")]
    foreground_pgrp: Option<i32>,
    #[serde(rename = "currentDir")]
    current_dir: Option<String>,
}

struct Packet {
    typ: u8,
    len: u8,
    buf: [u8; mem::size_of::<libc::winsize>()],
}

struct Client {
    stream: UnixStream,
    attached: bool,
}

struct CommandTracker {
    state: SessionState,
    path: PathBuf,
    output_path: PathBuf,
    shell_pid: libc::pid_t,
    current_pgrp: Option<i32>,
}

impl CommandTracker {
    fn new(record: &SessionRecord, shell_pid: libc::pid_t) -> Self {
        let state = read_session_state(&record.id);
        Self {
            state,
            path: session_path(&record.id).join("state.json"),
            output_path: active_output_path(&record.id),
            shell_pid,
            current_pgrp: None,
        }
    }

    fn note_input(&mut self) {
        self.state.last_activity_at = Some(now_epoch());
        self.save();
    }

    fn refresh(&mut self, pty_fd: RawFd) {
        let now = now_epoch();
        let fg = foreground_pgrp(pty_fd);
        self.state.foreground_pgrp = fg;

        let Some(pgrp) = fg else {
            self.save();
            return;
        };

        let shell_pgrp = self.shell_pid as i32;
        if pgrp != shell_pgrp {
            if self.current_pgrp != Some(pgrp) {
                self.current_pgrp = Some(pgrp);
                self.state.active_command = command_for_pgrp(pgrp);
                self.state.command_started_at = Some(now);
                self.state.command_finished_at = None;
                let _ = fs::write(&self.output_path, "");
            }
            self.state.command_running = true;
            self.state.current_dir = cwd_for_pgrp(pgrp);
        } else {
            self.state.active_command = None;
            self.state.command_running = false;
            if self.current_pgrp.is_some() {
                self.state.command_finished_at = Some(now);
            }
            self.current_pgrp = None;
        }

        self.save();
    }

    fn capture_output(&self, bytes: &[u8]) {
        if !self.state.command_running {
            return;
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.output_path)
        {
            let _ = file.write_all(bytes);
        }
    }

    fn save(&self) {
        if let Ok(raw) = serde_json::to_string_pretty(&self.state) {
            let _ = fs::write(&self.path, raw);
        }
    }
}

#[derive(Default)]
struct TitleFilter {
    state: u8,
}

impl TitleFilter {
    fn filter(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        for &b in input {
            match self.state {
                0 => {
                    if b == 0x1b {
                        self.state = 1;
                    } else {
                        out.push(b);
                    }
                }
                1 => {
                    if b == b']' {
                        self.state = 2;
                    } else {
                        out.push(0x1b);
                        out.push(b);
                        self.state = 0;
                    }
                }
                2 => {
                    if matches!(b, b'0' | b'1' | b'2') {
                        self.state = 3;
                    } else {
                        out.push(0x1b);
                        out.push(b']');
                        out.push(b);
                        self.state = 0;
                    }
                }
                3 => {
                    if b == b';' {
                        self.state = 4;
                    } else {
                        out.push(0x1b);
                        out.push(b']');
                        out.push(b);
                        self.state = 0;
                    }
                }
                4 => {
                    if b == 0x07 {
                        self.state = 0;
                    } else if b == 0x1b {
                        self.state = 5;
                    }
                }
                5 => {
                    self.state = if b == b'\\' { 0 } else { 4 };
                }
                _ => self.state = 0,
            }
        }
        out
    }
}

fn main() {
    if let Err(err) = run() {
        let style = Style::stderr();
        eprintln!("{} {}", style.error("error:"), err);
        process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let cli = Cli::parse();
    if let Some(command) = cli.command {
        return match command {
            Commands::Start => cmd_start(),
            Commands::Stop => cmd_stop(),
            Commands::List(args) => cmd_list(&args),
            Commands::SendKeys(args) => cmd_send_keys(&args),
            Commands::Capture(args) | Commands::CapturePane(args) => cmd_capture_pane(&args),
            Commands::Kill(args) | Commands::KillSession(args) | Commands::KillSessions(args) => {
                cmd_kill_session(Some(&args.session))
            }
            Commands::Status(args) => cmd_status(&args),
            Commands::Init => {
                let style = Style::stdout();
                println!("# {}: auto-wrap is disabled.", style.brand());
                println!("# Run \"hitch\" when you want a terminal to be visible to agents.");
                Ok(())
            }
            Commands::InitPrompt => cmd_init_prompt(),
            Commands::Setup(args) => cmd_setup(&args),
        };
    }

    if env::var_os("HITCH_SESSION").is_some() {
        cmd_status(&StatusArgs { debug: false })
    } else {
        cmd_start()
    }
}

fn state_dir() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("hitch")
    } else {
        PathBuf::from(env::var_os("HOME").unwrap_or_else(|| OsString::from(".")))
            .join(".local/state/hitch")
    }
}

fn sessions_dir() -> PathBuf {
    state_dir().join("sessions")
}

fn ensure_state_dirs() -> io::Result<()> {
    fs::create_dir_all(sessions_dir())
}

fn session_path(id: &str) -> PathBuf {
    sessions_dir().join(id)
}

fn stop_marker_path(id: &str) -> PathBuf {
    session_path(id).join("stopped")
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn next_session_id() -> io::Result<String> {
    let mut used = read_sessions()?
        .into_iter()
        .filter_map(|session| session.id.parse::<u32>().ok())
        .collect::<Vec<_>>();
    used.sort_unstable();

    let mut next = 1;
    for id in used {
        if id == next {
            next += 1;
        } else if id > next {
            break;
        }
    }

    Ok(next.to_string())
}

fn cmd_start() -> io::Result<()> {
    if let Ok(session) = env::var("HITCH_SESSION") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("already sharing terminal {session}"),
        ));
    }

    ensure_state_dirs()?;
    let cwd = env::current_dir()?;
    let id = next_session_id()?;
    let dir = session_path(&id);
    fs::create_dir_all(&dir)?;
    let _ = fs::remove_file(stop_marker_path(&id));

    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let record = SessionRecord {
        id: id.clone(),
        cwd: cwd.to_string_lossy().into_owned(),
        socket: dir.join("session.sock").to_string_lossy().into_owned(),
        log: dir.join("output.log").to_string_lossy().into_owned(),
        pid_file: dir.join("child.pid").to_string_lossy().into_owned(),
        master_pid_file: dir.join("master.pid").to_string_lossy().into_owned(),
        created_at: now_epoch().to_string(),
        shell: shell.clone(),
    };
    fs::write(
        dir.join("session.json"),
        serde_json::to_string_pretty(&record).unwrap(),
    )?;

    let style = Style::stdout();
    println!(
        "{} sharing terminal {} {}",
        style.brand(),
        style.id(&id),
        style.muted("(Ctrl-\\ to stop)")
    );

    let initial_winsize = terminal_winsize();
    let listener = UnixListener::bind(&record.socket)?;
    let master_pid = unsafe { libc::fork() };
    if master_pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if master_pid == 0 {
        unsafe {
            let _ = libc::setsid();
        }
        let code = match master_loop(listener, &record, &shell, initial_winsize.as_ref()) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("hitch master: {err}");
                1
            }
        };
        process::exit(code);
    }

    fs::write(&record.master_pid_file, format!("{master_pid}\n"))?;
    drop(listener);
    let status = attach_socket(&record.socket, &record.id, None)?;
    let _ = fs::remove_dir_all(&dir);
    process::exit(status);
}

fn master_loop(
    listener: UnixListener,
    record: &SessionRecord,
    shell: &str,
    initial_winsize: Option<&libc::winsize>,
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let (pty_fd, child_pid) = fork_shell(shell, record, initial_winsize)?;
    fs::write(&record.pid_file, format!("{child_pid}\n"))?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&record.log)?;
    let mut clients: Vec<Client> = Vec::new();
    let mut filter = TitleFilter::default();
    let mut commands = CommandTracker::new(record, child_pid);
    let mut had_attached = false;

    loop {
        let mut readfds = unsafe { mem::zeroed::<libc::fd_set>() };
        unsafe {
            libc::FD_ZERO(&mut readfds);
            libc::FD_SET(listener.as_raw_fd(), &mut readfds);
            libc::FD_SET(pty_fd, &mut readfds);
        }
        let mut max_fd = listener.as_raw_fd().max(pty_fd);
        for client in &clients {
            let fd = client.stream.as_raw_fd();
            unsafe { libc::FD_SET(fd, &mut readfds) };
            max_fd = max_fd.max(fd);
        }

        let mut timeout = libc::timeval {
            tv_sec: 1,
            tv_usec: 0,
        };
        let rc = unsafe {
            libc::select(
                max_fd + 1,
                &mut readfds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut timeout,
            )
        };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if rc == 0 {
            commands.refresh(pty_fd);
            continue;
        }

        if unsafe { libc::FD_ISSET(listener.as_raw_fd(), &readfds) } {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    clients.push(Client {
                        stream,
                        attached: false,
                    });
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                Err(err) => return Err(err),
            }
        }

        if unsafe { libc::FD_ISSET(pty_fd, &readfds) } {
            commands.refresh(pty_fd);
            let mut buf = [0u8; BUF_SIZE];
            let len = unsafe { libc::read(pty_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if len <= 0 {
                break;
            }
            let filtered = filter.filter(&buf[..len as usize]);
            if !filtered.is_empty() {
                let _ = log.write_all(&filtered);
                commands.capture_output(&filtered);
                clients.retain_mut(|client| {
                    if !client.attached {
                        return true;
                    }
                    client.stream.write_all(&filtered).is_ok()
                });
            }
        }

        let mut i = 0;
        while i < clients.len() {
            let fd = clients[i].stream.as_raw_fd();
            if unsafe { libc::FD_ISSET(fd, &readfds) } {
                match read_packet(&mut clients[i].stream) {
                    Ok(Some(packet)) if packet.typ == MSG_DETACH_SESSION => {
                        let _ = fs::write(stop_marker_path(&record.id), "");
                        clients.clear();
                        continue;
                    }
                    Ok(Some(packet)) => {
                        handle_packet(pty_fd, &mut clients[i], packet, &mut commands)?
                    }
                    Ok(None) => {
                        clients.remove(i);
                        continue;
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => {
                        clients.remove(i);
                        continue;
                    }
                }
            }
            i += 1;
        }

        let attached = clients.iter().any(|c| c.attached);
        if attached {
            had_attached = true;
        } else if had_attached {
            let _ = unsafe { libc::kill(child_pid, libc::SIGTERM) };
            break;
        }
    }

    let _ = fs::remove_file(&record.socket);
    Ok(())
}

fn fork_shell(
    shell: &str,
    record: &SessionRecord,
    initial_winsize: Option<&libc::winsize>,
) -> io::Result<(RawFd, libc::pid_t)> {
    let mut master: libc::c_int = -1;
    let winsize_ptr = initial_winsize
        .map(|ws| ws as *const libc::winsize as *mut libc::winsize)
        .unwrap_or(std::ptr::null_mut());
    let pid = unsafe {
        libc::forkpty(
            &mut master,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            winsize_ptr,
        )
    };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        unsafe {
            env::set_var("HITCH", "1");
            env::set_var("HITCH_SESSION", &record.id);
            env::set_var("HITCH_SOCKET", &record.socket);
        }
        let c_shell = CString::new(shell.as_bytes()).unwrap();
        unsafe {
            libc::execl(
                c_shell.as_ptr(),
                c_shell.as_ptr(),
                std::ptr::null::<libc::c_char>(),
            );
            libc::_exit(127);
        }
    }
    Ok((master, pid))
}

fn handle_packet(
    pty_fd: RawFd,
    client: &mut Client,
    packet: Packet,
    commands: &mut CommandTracker,
) -> io::Result<()> {
    match packet.typ {
        MSG_PUSH => {
            let len = packet.len as usize;
            let bytes = &packet.buf[..len];
            commands.note_input();
            write_all_fd(pty_fd, bytes)?;
        }
        MSG_ATTACH => client.attached = true,
        MSG_DETACH => client.attached = false,
        MSG_WINCH => unsafe {
            let ws = packet.buf.as_ptr().cast::<libc::winsize>();
            libc::ioctl(pty_fd, libc::TIOCSWINSZ, ws);
        },
        _ => {}
    }
    Ok(())
}

fn attach_socket(socket: &str, session_id: &str, log_path: Option<&str>) -> io::Result<i32> {
    let mut stream = UnixStream::connect(socket)?;
    let original = terminal_raw()?;
    let _restore = TermRestore(original);
    let _winch_restore = WinchRestore::install()?;
    let style = Style::plain();
    let exit_message = || style.muted(format!("[hitch stopped sharing {session_id}]"));

    send_packet(&mut stream, MSG_ATTACH, &[])?;
    send_winch(&mut stream)?;
    if let Some(log_path) = log_path {
        print_attach_history(session_id, log_path, &style)?;
    }

    loop {
        flush_pending_winch(&mut stream)?;

        let stdin_fd = libc::STDIN_FILENO;
        let sock_fd = stream.as_raw_fd();
        let mut readfds = unsafe { mem::zeroed::<libc::fd_set>() };
        unsafe {
            libc::FD_ZERO(&mut readfds);
            libc::FD_SET(stdin_fd, &mut readfds);
            libc::FD_SET(sock_fd, &mut readfds);
        }
        let rc = unsafe {
            libc::select(
                sock_fd.max(stdin_fd) + 1,
                &mut readfds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                flush_pending_winch(&mut stream)?;
                continue;
            }
            return Err(err);
        }
        if unsafe { libc::FD_ISSET(sock_fd, &readfds) } {
            let mut buf = [0u8; BUF_SIZE];
            let len = stream.read(&mut buf)?;
            if len == 0 {
                if stop_marker_path(session_id).exists() {
                    let _ = fs::remove_file(stop_marker_path(session_id));
                    println!("\r\n{}", exit_message());
                    return Ok(0);
                }
                return Ok(if log_path.is_some() {
                    0
                } else {
                    EXIT_PARENT_CODE
                });
            }
            io::stdout().write_all(&buf[..len])?;
            io::stdout().flush()?;
        }
        if unsafe { libc::FD_ISSET(stdin_fd, &readfds) } {
            let mut buf = [0u8; BUF_SIZE];
            let len = unsafe { libc::read(stdin_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if len <= 0 {
                return Ok(1);
            }
            if is_detach_key(buf[0]) {
                send_packet(&mut stream, MSG_DETACH, &[])?;
                println!("\r\n{}", exit_message());
                return Ok(0);
            }
            send_packet(&mut stream, MSG_PUSH, &buf[..len as usize])?;
        }
    }
}

fn print_attach_history(session_id: &str, log_path: &str, style: &Style) -> io::Result<()> {
    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "{} sharing terminal {} {}",
        style.brand(),
        style.id(session_id),
        style.muted("(Ctrl-\\ to stop)")
    )?;

    let history = tail_raw_bytes(log_path, ATTACH_HISTORY_BYTES)?;
    if !history.is_empty() {
        stdout.write_all(&history)?;
        if !history.ends_with(b"\n") {
            stdout.write_all(b"\r\n")?;
        }
    }
    stdout.flush()
}

fn tail_raw_bytes(path: &str, limit: u64) -> io::Result<Vec<u8>> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(limit);
    use std::io::Seek;
    file.seek(io::SeekFrom::Start(start))?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if start > 0 {
        if let Some(pos) = bytes.iter().position(|b| *b == b'\n') {
            bytes.drain(..=pos);
        }
    }
    Ok(bytes)
}

fn is_detach_key(byte: u8) -> bool {
    byte == (b'\\' & 0x1f)
}

fn terminal_raw() -> io::Result<libc::termios> {
    let mut original = unsafe { mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut original) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut raw = original;
    raw.c_iflag &= !(libc::IGNBRK
        | libc::BRKINT
        | libc::PARMRK
        | libc::ISTRIP
        | libc::INLCR
        | libc::IGNCR
        | libc::ICRNL
        | libc::IXON
        | libc::IXOFF);
    raw.c_oflag &= !libc::OPOST;
    raw.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
    raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
    raw.c_cflag |= libc::CS8;
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &raw) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(original)
}

struct TermRestore(libc::termios);

impl Drop for TermRestore {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &self.0);
        }
        print!("\x1b[?25h");
        let _ = io::stdout().flush();
    }
}

struct WinchRestore(libc::sighandler_t);

impl WinchRestore {
    fn install() -> io::Result<Self> {
        WINCH_PENDING.store(false, Ordering::SeqCst);
        let previous = unsafe {
            libc::signal(
                libc::SIGWINCH,
                handle_sigwinch as *const () as libc::sighandler_t,
            )
        };
        if previous == libc::SIG_ERR {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(previous))
        }
    }
}

impl Drop for WinchRestore {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGWINCH, self.0);
        }
    }
}

extern "C" fn handle_sigwinch(_: libc::c_int) {
    WINCH_PENDING.store(true, Ordering::SeqCst);
}

fn flush_pending_winch(stream: &mut UnixStream) -> io::Result<()> {
    if WINCH_PENDING.swap(false, Ordering::SeqCst) {
        send_winch(stream)?;
    }
    Ok(())
}

fn send_winch(stream: &mut UnixStream) -> io::Result<()> {
    if let Some(ws) = terminal_winsize() {
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&ws as *const libc::winsize).cast::<u8>(),
                mem::size_of::<libc::winsize>(),
            )
        };
        send_packet(stream, MSG_WINCH, bytes)?;
    }
    Ok(())
}

fn terminal_winsize() -> Option<libc::winsize> {
    let mut ws = unsafe { mem::zeroed::<libc::winsize>() };
    if unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0
        && ws.ws_col > 0
        && ws.ws_row > 0
    {
        Some(ws)
    } else {
        None
    }
}

fn send_packet(stream: &mut UnixStream, typ: u8, payload: &[u8]) -> io::Result<()> {
    let mut buf = [0u8; 2 + mem::size_of::<libc::winsize>()];
    buf[0] = typ;
    buf[1] = payload.len().min(mem::size_of::<libc::winsize>()) as u8;
    let len = buf[1] as usize;
    buf[2..2 + len].copy_from_slice(&payload[..len]);
    stream.write_all(&buf)
}

fn read_packet(stream: &mut UnixStream) -> io::Result<Option<Packet>> {
    let mut raw = [0u8; 2 + mem::size_of::<libc::winsize>()];
    match stream.read_exact(&mut raw) {
        Ok(()) => {
            let mut buf = [0u8; mem::size_of::<libc::winsize>()];
            buf.copy_from_slice(&raw[2..]);
            Ok(Some(Packet {
                typ: raw[0],
                len: raw[1],
                buf,
            }))
        }
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(err) => Err(err),
    }
}

fn write_all_fd(fd: RawFd, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let rc = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        bytes = &bytes[rc as usize..];
    }
    Ok(())
}

fn cmd_list(args: &ListArgs) -> io::Result<()> {
    let all_sessions = read_sessions()?;
    let total_count = all_sessions.len();
    let mut sessions = all_sessions;

    let filter_dir = if let Some(dir) = args.dir.clone() {
        Some(dir)
    } else if args.all {
        None
    } else {
        Some(env::current_dir()?)
    };

    if let Some(filter) = filter_dir {
        let filter = filter.canonicalize().unwrap_or(filter);
        sessions.retain(|session| {
            let state = read_session_state(&session.id);
            let origin = canonical_path(&session.cwd);
            let current = canonical_path(current_dir_for_session(session, &state));
            origin.starts_with(&filter) || current.starts_with(&filter)
        });
    }

    sessions.sort_by_key(|session| session.id.parse::<u32>().unwrap_or(u32::MAX));

    let style = Style::plain();
    if args.all {
        println!("terminals: {} total", style.id(total_count.to_string()));
    } else {
        println!(
            "terminals: {} project, {} total",
            style.id(sessions.len().to_string()),
            style.id(total_count.to_string())
        );
    }

    if sessions.is_empty() {
        return Ok(());
    }

    for session in sessions {
        let state = read_session_state(&session.id);
        let cwd = current_dir_for_session(&session, &state);
        let activity = state
            .last_activity_at
            .map(time_ago)
            .unwrap_or_else(|| "unknown".to_string());

        println!("----- terminal {} -----", style.id(&session.id));
        println!("current dir: {}", style.path(shorten_home(&cwd)));
        println!("last input was {activity}");
        if state.command_running {
            let duration = state
                .command_started_at
                .map(running_for)
                .unwrap_or_else(|| "unknown time".to_string());
            println!("process is running for {}", style.command(duration));
        } else {
            println!("no actively running commands");
        }

        let mut printed_output = false;
        if state.command_running {
            let head = list_head_lines(&active_output_path(&session.id), ACTIVE_COMMAND_HEAD_LINES);
            let tail = list_tail_lines(&session.log, args.tail);
            if !head.is_empty() && !contains_line_sequence(&tail, &head) {
                println!();
                println!("--- active output head ({} lines) ---", head.len());
                for line in head {
                    println!("{line}");
                }
                printed_output = true;
            }
            if !tail.is_empty() {
                println!();
                println!("--- recent output ({} lines) ---", tail.len());
                for line in tail {
                    println!("{line}");
                }
                printed_output = true;
            }
        } else {
            let tail = list_tail_lines(&session.log, args.tail);
            if !tail.is_empty() {
                println!();
                println!("--- recent output ({} lines) ---", tail.len());
                for line in tail {
                    println!("{line}");
                }
                printed_output = true;
            }
        }
        if !printed_output {
            println!();
            if state.command_running {
                println!("no visible output yet");
            } else {
                println!("no visible output");
            }
        }
        println!();
    }

    Ok(())
}

fn read_sessions() -> io::Result<Vec<SessionRecord>> {
    ensure_state_dirs()?;
    let mut records = Vec::new();
    for entry in fs::read_dir(sessions_dir())? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path().join("session.json");
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<SessionRecord>(&raw) else {
            continue;
        };
        if Path::new(&record.socket).exists() {
            records.push(record);
        } else {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(records)
}

fn find_session(id: Option<&str>) -> io::Result<SessionRecord> {
    let id = id.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing terminal"))?;
    let exact = session_path(id).join("session.json");
    if exact.exists() {
        return serde_json::from_str(&fs::read_to_string(exact)?)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
    }
    let matches = read_sessions()?
        .into_iter()
        .filter(|s| s.id.starts_with(id))
        .collect::<Vec<_>>();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("terminal {id} does not exist"),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("ambiguous hitch terminal: {id}"),
        )),
    }
}

// Kept for manual debugging while the public CLI no longer exposes interactive attach/join.
#[allow(dead_code)]
fn cmd_attach(id: Option<&str>) -> io::Result<()> {
    if let Ok(session) = env::var("HITCH_SESSION") {
        let style = Style::stderr();
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "can't start while already sharing terminal {} {}",
                style.id(session),
                style.muted("(Ctrl-\\ to stop)")
            ),
        ));
    }

    let session = find_session(id)?;
    let status = attach_socket(&session.socket, &session.id, Some(&session.log))?;
    process::exit(status);
}

fn cmd_send_keys(args: &SendKeysArgs) -> io::Result<()> {
    let session = find_session(Some(&args.target))?;
    let mut payload = Vec::new();
    for arg in &args.keys {
        payload.extend(key_to_bytes(arg));
    }
    let mut stream = UnixStream::connect(session.socket)?;
    for chunk in payload.chunks(mem::size_of::<libc::winsize>()) {
        send_packet(&mut stream, MSG_PUSH, chunk)?;
    }
    Ok(())
}

fn key_to_bytes(key: &str) -> Vec<u8> {
    match key {
        "Enter" | "C-m" => vec![b'\n'],
        "Tab" => vec![b'\t'],
        "Escape" | "Esc" => vec![0x1b],
        "Space" => vec![b' '],
        "Backspace" | "BSpace" => vec![0x7f],
        _ if key.starts_with("C-") && key.len() == 3 => {
            vec![key.as_bytes()[2].to_ascii_uppercase() & 0x1f]
        }
        _ => key.as_bytes().to_vec(),
    }
}

fn cmd_capture_pane(args: &CapturePaneArgs) -> io::Result<()> {
    let session = find_session(Some(&args.target))?;
    if let Some(start) = args.start {
        let lines = if args.raw {
            raw_lines_from_start(&session.log, start)
        } else {
            rendered_lines_from_start(&session.log, start)
        };
        for line in lines {
            println!("{line}");
        }
    } else if let Some(tail) = args.tail {
        let lines = if args.raw {
            raw_tail_lines(&session.log, tail)
        } else {
            rendered_tail_lines(&session.log, tail)
        };
        for line in lines {
            println!("{line}");
        }
    } else {
        let text = if args.raw {
            fs::read_to_string(session.log).unwrap_or_default()
        } else {
            rendered_log(&session.log)
        };
        print!("{text}");
        if !text.is_empty() && !text.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

fn cmd_kill_session(id: Option<&str>) -> io::Result<()> {
    let session = find_session(id)?;
    for pid in [
        read_pid(&session.master_pid_file),
        read_pid(&session.pid_file),
    ]
    .into_iter()
    .flatten()
    {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    let _ = fs::remove_dir_all(session_path(&session.id));
    Ok(())
}

fn cmd_init_prompt() -> io::Result<()> {
    println!("run `hitch setup shell` to install shell integration");
    Ok(())
}

fn cmd_install_skill() -> io::Result<()> {
    let args = [
        "--yes",
        "skills",
        "add",
        SKILL_INSTALL_URL,
        "--skill",
        SKILL_NAME,
    ];
    println!("running: npx {}", args.join(" "));

    match Command::new("npx").args(args).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(io::Error::other(format!(
            "skill installer exited with status {status}"
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "npx not found. Install Node.js or run manually: npx {}",
                args.join(" ")
            ),
        )),
        Err(err) => Err(err),
    }
}

fn cmd_setup(args: &SetupArgs) -> io::Result<()> {
    match args.command {
        SetupCommand::Shell => cmd_setup_prompt(),
        SetupCommand::Skill => cmd_install_skill(),
    }
}

fn cmd_setup_prompt() -> io::Result<()> {
    let shell = detect_shell();
    if shell == "zsh" {
        return setup_zsh_family_prompt();
    }

    if shell == "bash" {
        return setup_bash_prompt();
    }

    if shell == "fish" {
        return setup_fish_prompt();
    }

    println!("unsupported shell: {shell}");
    println!("manual prompt segment: show `#$HITCH_SESSION` when HITCH_SESSION is set");
    Ok(())
}

fn detect_shell() -> String {
    env::var("SHELL")
        .ok()
        .and_then(|shell| {
            Path::new(&shell)
                .file_name()
                .map(|name| name.to_string_lossy().into())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn home_file(name: &str) -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(name))
}

fn setup_zsh_family_prompt() -> io::Result<()> {
    setup_zsh_prompt()?;

    if let Some(p10k_path) = home_file(".p10k.zsh").filter(|path| path.exists()) {
        setup_p10k_prompt(&p10k_path)?;
    }

    println!("shell integration updated");
    println!("restart existing terminals to pick up shell integration");

    Ok(())
}

fn setup_p10k_prompt(path: &Path) -> io::Result<()> {
    let raw = fs::read_to_string(path)?;
    let mut updated = ensure_p10k_left_segment(&raw)?;
    updated = upsert_p10k_prompt_block(&updated)?;
    if updated != raw {
        let _backup = backup_file(path)?;
        fs::write(path, updated)?;
    }
    Ok(())
}

fn setup_zsh_prompt() -> io::Result<()> {
    let Some(path) = home_file(".zshrc") else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
    };
    setup_rc_prompt(&path, zsh_prompt_block())
}

fn setup_bash_prompt() -> io::Result<()> {
    let Some(path) = home_file(".bashrc") else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
    };
    setup_rc_prompt(&path, bash_prompt_block())?;
    println!("shell integration updated");
    println!("restart existing terminals to pick up shell integration");
    Ok(())
}

fn setup_fish_prompt() -> io::Result<()> {
    let Some(path) = home_file(".config/fish/conf.d/hitch.fish") else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    let updated = upsert_marked_block(&raw, fish_prompt_block());
    if updated != raw {
        if path.exists() {
            let _backup = backup_file(&path)?;
        }
        fs::write(&path, updated)?;
    }
    println!("shell integration updated");
    println!("restart existing fish terminals to pick up shell integration");
    Ok(())
}

fn setup_rc_prompt(path: &Path, block: &str) -> io::Result<()> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let updated = upsert_marked_block(&raw, block);
    if updated != raw {
        if path.exists() {
            let _backup = backup_file(path)?;
        }
        fs::write(path, updated)?;
    }
    Ok(())
}

fn backup_file(path: &Path) -> io::Result<PathBuf> {
    let dir = state_dir().join("backups");
    fs::create_dir_all(&dir)?;
    let backup = dir.join(format!(
        "{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        now_epoch()
    ));
    fs::copy(path, &backup)?;
    Ok(backup)
}

fn upsert_marked_block(raw: &str, block: &str) -> String {
    upsert_marked_block_before(raw, block, "")
}

fn upsert_marked_block_before(raw: &str, block: &str, anchor: &str) -> String {
    const MARKERS: [(&str, &str); 2] = [
        (
            "# >>> hitch shell integration >>>",
            "# <<< hitch shell integration <<<",
        ),
        (
            "# >>> hitch prompt integration >>>",
            "# <<< hitch prompt integration <<<",
        ),
    ];

    for (start_marker, end_marker) in MARKERS {
        if let Some(start) = raw.find(start_marker) {
            if let Some(end_rel) = raw[start..].find(end_marker) {
                let line_start = raw[..start].rfind('\n').map(|index| index + 1).unwrap_or(0);
                let replace_start = if raw[line_start..start].trim().is_empty() {
                    line_start
                } else {
                    start
                };
                let end = start + end_rel + end_marker.len();
                let mut out = String::new();
                out.push_str(&raw[..replace_start]);
                out.push_str(block.trim_end());
                out.push_str(&raw[end..]);
                return out;
            }
        }
    }

    if !anchor.is_empty() {
        if let Some(index) = raw.find(anchor) {
            let mut out = String::new();
            out.push_str(raw[..index].trim_end());
            out.push_str("\n\n");
            out.push_str(block.trim_end());
            out.push_str("\n\n");
            out.push_str(&raw[index..]);
            return out;
        }
    }

    let mut out = raw.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block.trim_end());
    out.push('\n');
    out
}

fn upsert_p10k_prompt_block(raw: &str) -> io::Result<String> {
    if raw.contains("# >>> hitch shell integration >>>")
        || raw.contains("# >>> hitch prompt integration >>>")
    {
        return Ok(upsert_marked_block(raw, p10k_prompt_block()));
    }

    for anchor in [
        "  # Example of a user-defined prompt segment.",
        "  # Transient prompt works similarly",
        "  # If p10k is already loaded, reload configuration.",
    ] {
        if raw.contains(anchor) {
            return Ok(upsert_marked_block_before(raw, p10k_prompt_block(), anchor));
        }
    }

    Err(io::Error::other(
        "could not find a safe insertion point in ~/.p10k.zsh",
    ))
}

fn ensure_p10k_left_segment(raw: &str) -> io::Result<String> {
    let Some(start) = raw.find("POWERLEVEL9K_LEFT_PROMPT_ELEMENTS=(") else {
        return Err(io::Error::other(
            "could not find POWERLEVEL9K_LEFT_PROMPT_ELEMENTS in ~/.p10k.zsh",
        ));
    };
    let Some(end_rel) = raw[start..].find("\n  )") else {
        return Err(io::Error::other(
            "could not parse POWERLEVEL9K_LEFT_PROMPT_ELEMENTS in ~/.p10k.zsh",
        ));
    };
    let end = start + end_rel;
    let block = &raw[start..end];
    let mut lines: Vec<String> = block.lines().map(str::to_string).collect();
    lines.retain(|line| line.split('#').next().unwrap_or("").trim() != "hitch");

    let insert_at = lines
        .iter()
        .position(|line| line.split('#').next().unwrap_or("").trim() == "newline")
        .unwrap_or(lines.len());
    lines.insert(
        insert_at,
        "    hitch                   # hitch terminal id".to_string(),
    );

    let mut out = String::new();
    out.push_str(&raw[..start]);
    out.push_str(&lines.join("\n"));
    out.push_str(&raw[end..]);
    Ok(out)
}

fn p10k_prompt_block() -> &'static str {
    r##"  # >>> hitch shell integration >>>
  function prompt_hitch() {
    [[ -n "${HITCH_SESSION:-}" ]] && p10k segment -f 2 -t "#${HITCH_SESSION}"
  }
  # <<< hitch shell integration <<<"##
}

fn zsh_prompt_block() -> &'static str {
    r#"# >>> hitch shell integration >>>
function _hitch_prompt_segment() {
  [[ -n "${HITCH_SESSION:-}" ]] && print -n "%F{2}#${HITCH_SESSION}%f "
}

if [[ -z "${HITCH_PROMPT_INSTALLED:-}" && -z "${POWERLEVEL9K_LEFT_PROMPT_ELEMENTS:-}" ]]; then
  HITCH_PROMPT_INSTALLED=1
  PROMPT='$(_hitch_prompt_segment)'"$PROMPT"
fi

function hitch() {
  if [[ -z "${HITCH_SESSION:-}" && ( "$#" -eq 0 || "$1" == "start" ) ]]; then
    fc -W 2>/dev/null
  fi

  command hitch "$@"
  local code=$?
  if [[ "$code" -eq 42 ]]; then
    exit
  fi
  return "$code"
}

alias unhitch='hitch stop'
# <<< hitch shell integration <<<"#
}

fn bash_prompt_block() -> &'static str {
    r#"# >>> hitch shell integration >>>
_hitch_prompt_segment() {
  [[ -n "${HITCH_SESSION:-}" ]] && printf '\[\033[32m\]#%s\[\033[0m\] ' "$HITCH_SESSION"
}

if [[ -z "${HITCH_PROMPT_INSTALLED:-}" ]]; then
  HITCH_PROMPT_INSTALLED=1
  PS1='$(_hitch_prompt_segment)'"$PS1"
fi

hitch() {
  if [[ -z "${HITCH_SESSION:-}" && ( "$#" -eq 0 || "$1" == "start" ) ]]; then
    history -a 2>/dev/null
  fi

  command hitch "$@"
  local code=$?
  if [[ "$code" -eq 42 ]]; then
    exit
  fi
  return "$code"
}

alias unhitch='hitch stop'
# <<< hitch shell integration <<<"#
}

fn fish_prompt_block() -> &'static str {
    r#"# >>> hitch shell integration >>>
if not functions -q __hitch_original_fish_prompt
    functions -c fish_prompt __hitch_original_fish_prompt
end

function fish_prompt
    if set -q HITCH_SESSION
        set_color green
        printf '#%s ' $HITCH_SESSION
        set_color normal
    end
    __hitch_original_fish_prompt
end

function hitch
    if not set -q HITCH_SESSION
        if test (count $argv) -eq 0; or test "$argv[1]" = start
            history save 2>/dev/null
        end
    end

    command hitch $argv
    set code $status
    if test $code -eq 42
        exit
    end
    return $code
end

alias unhitch 'hitch stop'
# <<< hitch shell integration <<<"#
}

fn cmd_detach() -> io::Result<()> {
    let style = Style::stdout();
    let Ok(socket) = env::var("HITCH_SOCKET") else {
        println!("not sharing this terminal, run `{}`", style.brand());
        return Ok(());
    };

    let mut stream = UnixStream::connect(socket)?;
    send_packet(&mut stream, MSG_DETACH_SESSION, &[])
}

fn cmd_status(args: &StatusArgs) -> io::Result<()> {
    let style = Style::stdout();
    let Ok(session) = env::var("HITCH_SESSION") else {
        println!("not sharing this terminal, run `{}`", style.brand());
        return Ok(());
    };
    println!("sharing this terminal as {}", style.id(session));
    if args.debug {
        if let Ok(socket) = env::var("HITCH_SOCKET") {
            println!("socket {}", style.path(socket));
        }
    }
    Ok(())
}

fn cmd_stop() -> io::Result<()> {
    cmd_detach()
}

fn read_pid(path: &str) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_session_state(id: &str) -> SessionState {
    let path = session_path(id).join("state.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return SessionState::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn active_output_path(id: &str) -> PathBuf {
    session_path(id).join("active-output.log")
}

fn current_dir_for_session(session: &SessionRecord, state: &SessionState) -> String {
    state
        .foreground_pgrp
        .and_then(cwd_for_pgrp)
        .or_else(|| state.current_dir.clone())
        .unwrap_or_else(|| session.cwd.clone())
}

fn canonical_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn shorten_home(path: &str) -> String {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return path.to_string();
    };
    let path_buf = Path::new(path);
    if let Ok(stripped) = path_buf.strip_prefix(&home) {
        if stripped.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", stripped.to_string_lossy())
        }
    } else {
        path.to_string()
    }
}

fn time_ago(timestamp: u64) -> String {
    let elapsed = now_epoch().saturating_sub(timestamp);
    if elapsed < 60 {
        format!("{elapsed}s ago")
    } else if elapsed < 60 * 60 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 60 * 60 * 24 {
        format!("{}h ago", elapsed / 60 / 60)
    } else {
        format!("{}d ago", elapsed / 60 / 60 / 24)
    }
}

fn running_for(started_at: u64) -> String {
    let elapsed = now_epoch().saturating_sub(started_at);
    if elapsed < 60 {
        format!("{elapsed}s")
    } else if elapsed < 60 * 60 {
        format!("{}m {}s", elapsed / 60, elapsed % 60)
    } else if elapsed < 60 * 60 * 24 {
        format!("{}h {:02}m", elapsed / 60 / 60, (elapsed / 60) % 60)
    } else {
        format!("{}d {}h", elapsed / 60 / 60 / 24, (elapsed / 60 / 60) % 24)
    }
}

fn rendered_log(path: &str) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    render_terminal_text(text.as_bytes())
}

fn list_head_lines(path: &Path, limit: usize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let text = render_terminal_text(text.as_bytes());
    normalize_list_lines(text.lines().map(str::to_string).collect())
        .into_iter()
        .take(limit)
        .collect()
}

fn list_tail_lines(path: &str, limit: usize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let text = render_terminal_text(text.as_bytes());
    let lines = normalize_list_lines(text.lines().map(str::to_string).collect());
    let start = lines.len().saturating_sub(limit);
    lines.into_iter().skip(start).collect()
}

fn normalize_list_lines(lines: Vec<String>) -> Vec<String> {
    trim_visual_empty_edges(lines)
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect()
}

fn trim_visual_empty_edges(lines: Vec<String>) -> Vec<String> {
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(start);
    lines
        .into_iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn contains_line_sequence(lines: &[String], sequence: &[String]) -> bool {
    if sequence.is_empty() {
        return true;
    }
    if sequence.len() > lines.len() {
        return false;
    }
    lines
        .windows(sequence.len())
        .any(|window| window == sequence)
}

fn rendered_tail_lines(path: &str, limit: usize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let text = render_terminal_text(text.as_bytes());
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(limit)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn rendered_lines_from_start(path: &str, start: isize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let text = render_terminal_text(text.as_bytes());
    lines_from_start(&text, start)
}

fn raw_lines_from_start(path: &str, start: isize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    lines_from_start(&text, start)
}

fn lines_from_start(text: &str, start: isize) -> Vec<String> {
    let lines = text.lines().collect::<Vec<_>>();
    let index = if start < 0 {
        lines.len().saturating_sub(start.unsigned_abs())
    } else {
        start as usize
    };
    lines.into_iter().skip(index).map(str::to_string).collect()
}

fn raw_tail_lines(path: &str, limit: usize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .take(limit)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn foreground_pgrp(fd: RawFd) -> Option<i32> {
    let pgrp = unsafe { libc::tcgetpgrp(fd) };
    if pgrp > 0 { Some(pgrp as i32) } else { None }
}

fn command_for_pgrp(pgrp: i32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pgrp.to_string(), "-o", "command="])
        .output()
        .ok()?;
    let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if command.is_empty() {
        None
    } else {
        Some(command)
    }
}

fn cwd_for_pgrp(pgrp: i32) -> Option<String> {
    let output = Command::new("lsof")
        .args(["-a", "-p", &pgrp.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix('n').map(str::to_string))
        .filter(|path| !path.is_empty())
}

struct TextTerminal {
    rows: Vec<Vec<char>>,
    row: usize,
    col: usize,
    saved: Option<(usize, usize)>,
}

impl TextTerminal {
    fn new() -> Self {
        Self {
            rows: vec![Vec::new()],
            row: 0,
            col: 0,
            saved: None,
        }
    }

    fn ensure_row(&mut self) {
        while self.row >= self.rows.len() {
            self.rows.push(Vec::new());
        }
    }

    fn put(&mut self, ch: char) {
        self.ensure_row();
        let line = &mut self.rows[self.row];
        while self.col > line.len() {
            line.push(' ');
        }
        if self.col == line.len() {
            line.push(ch);
        } else {
            line[self.col] = ch;
        }
        self.col += 1;
    }

    fn newline(&mut self) {
        self.row += 1;
        self.col = 0;
        self.ensure_row();
    }

    fn clear_line_from_cursor(&mut self) {
        self.ensure_row();
        self.rows[self.row].truncate(self.col);
    }

    fn clear_line_to_cursor(&mut self) {
        self.ensure_row();
        let line = &mut self.rows[self.row];
        let end = self.col.min(line.len());
        for ch in &mut line[..end] {
            *ch = ' ';
        }
    }

    fn clear_line(&mut self) {
        self.ensure_row();
        self.rows[self.row].clear();
        self.col = 0;
    }

    fn clear_screen_from_cursor(&mut self) {
        self.clear_line_from_cursor();
        self.rows.truncate(self.row + 1);
    }

    fn finish(self) -> String {
        self.rows
            .into_iter()
            .map(|line| line.into_iter().collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn render_terminal_text(bytes: &[u8]) -> String {
    let mut term = TextTerminal::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                term.col = 0;
                i += 1;
            }
            b'\n' => {
                term.newline();
                i += 1;
            }
            0x08 => {
                term.col = term.col.saturating_sub(1);
                i += 1;
            }
            b'\t' => {
                let spaces = 8 - (term.col % 8);
                for _ in 0..spaces {
                    term.put(' ');
                }
                i += 1;
            }
            0x1b => {
                i = apply_escape(bytes, i, &mut term);
            }
            byte if byte >= 0x20 => {
                if let Ok(text) = std::str::from_utf8(&bytes[i..]) {
                    if let Some(ch) = text.chars().next() {
                        term.put(ch);
                        i += ch.len_utf8();
                    } else {
                        i += 1;
                    }
                } else {
                    term.put(byte as char);
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    term.finish()
}

fn apply_escape(bytes: &[u8], mut i: usize, term: &mut TextTerminal) -> usize {
    i += 1;
    if i >= bytes.len() {
        return i;
    }

    match bytes[i] {
        b']' => skip_osc(bytes, i + 1),
        b'7' => {
            term.saved = Some((term.row, term.col));
            i + 1
        }
        b'8' => {
            if let Some((row, col)) = term.saved {
                term.row = row;
                term.col = col;
                term.ensure_row();
            }
            i + 1
        }
        b'[' => apply_csi(bytes, i + 1, term),
        _ => i + 1,
    }
}

fn skip_osc(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == 0x07 {
            return i + 1;
        }
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
            return i + 2;
        }
        i += 1;
    }
    i
}

fn apply_csi(bytes: &[u8], mut i: usize, term: &mut TextTerminal) -> usize {
    let start = i;
    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return i;
    }

    let final_byte = bytes[i];
    let params = parse_csi_params(&bytes[start..i]);
    let n = params.first().copied().unwrap_or(1).max(1) as usize;
    match final_byte {
        b'A' => term.row = term.row.saturating_sub(n),
        b'B' => {
            term.row += n;
            term.ensure_row();
        }
        b'C' => term.col += n,
        b'D' => term.col = term.col.saturating_sub(n),
        b'G' => term.col = n.saturating_sub(1),
        b'H' | b'f' => {
            term.row = params.first().copied().unwrap_or(1).saturating_sub(1) as usize;
            term.col = params.get(1).copied().unwrap_or(1).saturating_sub(1) as usize;
            term.ensure_row();
        }
        b'J' => {
            if params.first().copied().unwrap_or(0) == 0 {
                term.clear_screen_from_cursor();
            }
        }
        b'K' => match params.first().copied().unwrap_or(0) {
            0 => term.clear_line_from_cursor(),
            1 => term.clear_line_to_cursor(),
            2 => term.clear_line(),
            _ => {}
        },
        _ => {}
    }
    i + 1
}

fn parse_csi_params(bytes: &[u8]) -> Vec<u16> {
    let text = String::from_utf8_lossy(bytes);
    text.trim_start_matches('?')
        .split(';')
        .filter_map(|part| part.parse::<u16>().ok())
        .collect()
}
