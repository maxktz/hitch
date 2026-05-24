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
use std::time::{SystemTime, UNIX_EPOCH};

const BUF_SIZE: usize = 4096;
const MSG_PUSH: u8 = 0;
const MSG_ATTACH: u8 = 1;
const MSG_DETACH: u8 = 2;
const MSG_WINCH: u8 = 3;

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const ATTACH_HISTORY_BYTES: u64 = 64 * 1024;

struct Style {
    enabled: bool,
}

#[derive(Parser)]
#[command(name = "muxi", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    New,
    List(ListArgs),
    ListSessions(ListArgs),
    ListPanes(ListArgs),
    Attach(SessionArg),
    SendKeys(SendKeysArgs),
    CapturePane(CapturePaneArgs),
    KillSession(SessionArg),
    Info(InfoArgs),
    Init,
    Exit,
}

#[derive(Args)]
struct ListArgs {
    #[arg(long)]
    all: bool,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dir: Option<PathBuf>,
    #[arg(long, default_value_t = 5)]
    tail: usize,
}

#[derive(Args)]
struct SessionArg {
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
    #[arg(long)]
    tail: Option<usize>,
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
struct InfoArgs {
    #[arg(long)]
    debug: bool,
}

impl Style {
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
        self.paint("muxi", &[GREEN])
    }

    fn id(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[GREEN])
    }

    fn label(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[DIM])
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

    fn success(&self, value: impl AsRef<str>) -> String {
        value.as_ref().to_string()
    }

    fn error(&self, value: impl AsRef<str>) -> String {
        value.as_ref().to_string()
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

struct Packet {
    typ: u8,
    len: u8,
    buf: [u8; mem::size_of::<libc::winsize>()],
}

struct Client {
    stream: UnixStream,
    attached: bool,
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
            Commands::New => cmd_new(),
            Commands::List(args) | Commands::ListSessions(args) | Commands::ListPanes(args) => {
                cmd_list(&args)
            }
            Commands::Attach(args) => cmd_attach(Some(&args.session)),
            Commands::SendKeys(args) => cmd_send_keys(&args),
            Commands::CapturePane(args) => cmd_capture_pane(&args),
            Commands::KillSession(args) => cmd_kill_session(Some(&args.session)),
            Commands::Info(args) => cmd_info(&args),
            Commands::Init => {
                let style = Style::stdout();
                println!("# {}: auto-wrap is disabled.", style.brand());
                println!("# Run \"muxi\" when you want a terminal to be visible to agents.");
                Ok(())
            }
            Commands::Exit => {
                let style = Style::stdout();
                println!("{} detach with {}", style.brand(), style.command("Ctrl-\\"));
                Ok(())
            }
        };
    }

    if env::var_os("MUXI_SESSION").is_some() {
        cmd_info(&InfoArgs { debug: false })
    } else {
        cmd_new()
    }
}

fn state_dir() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("muxi")
    } else {
        PathBuf::from(env::var_os("HOME").unwrap_or_else(|| OsString::from(".")))
            .join(".local/state/muxi")
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

fn cmd_new() -> io::Result<()> {
    if let Ok(session) = env::var("MUXI_SESSION") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("already inside muxi session {session}"),
        ));
    }

    ensure_state_dirs()?;
    let cwd = env::current_dir()?;
    let id = next_session_id()?;
    let dir = session_path(&id);
    fs::create_dir_all(&dir)?;

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
        "{} joined session {} {}",
        style.brand(),
        style.id(&id),
        style.muted("(Ctrl-\\ to exit)")
    );

    let listener = UnixListener::bind(&record.socket)?;
    let master_pid = unsafe { libc::fork() };
    if master_pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if master_pid == 0 {
        unsafe {
            let _ = libc::setsid();
        }
        let code = match master_loop(listener, &record, &shell) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("muxi master: {err}");
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

fn master_loop(listener: UnixListener, record: &SessionRecord, shell: &str) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let (pty_fd, child_pid) = fork_shell(shell, record)?;
    fs::write(&record.pid_file, format!("{child_pid}\n"))?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&record.log)?;
    let mut clients: Vec<Client> = Vec::new();
    let mut filter = TitleFilter::default();
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

        let rc = unsafe {
            libc::select(
                max_fd + 1,
                &mut readfds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
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
            let mut buf = [0u8; BUF_SIZE];
            let len = unsafe { libc::read(pty_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if len <= 0 {
                break;
            }
            let filtered = filter.filter(&buf[..len as usize]);
            if !filtered.is_empty() {
                let _ = log.write_all(&filtered);
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
                    Ok(Some(packet)) => handle_packet(pty_fd, &mut clients[i], packet)?,
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

fn fork_shell(shell: &str, record: &SessionRecord) -> io::Result<(RawFd, libc::pid_t)> {
    let mut master: libc::c_int = -1;
    let pid = unsafe {
        libc::forkpty(
            &mut master,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        unsafe {
            env::set_var("MUXI", "1");
            env::set_var("MUXI_SESSION", &record.id);
            env::set_var("MUXI_SOCKET", &record.socket);
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

fn handle_packet(pty_fd: RawFd, client: &mut Client, packet: Packet) -> io::Result<()> {
    match packet.typ {
        MSG_PUSH => {
            let len = packet.len as usize;
            write_all_fd(pty_fd, &packet.buf[..len])?;
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
    let style = Style::stdout();
    let exit_message = || style.muted(format!("[muxi exited session {session_id}]"));

    send_packet(&mut stream, MSG_ATTACH, &[])?;
    send_winch(&mut stream)?;
    if let Some(log_path) = log_path {
        print_attach_history(session_id, log_path, &style)?;
    }

    loop {
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
                continue;
            }
            return Err(err);
        }
        if unsafe { libc::FD_ISSET(sock_fd, &readfds) } {
            let mut buf = [0u8; BUF_SIZE];
            let len = stream.read(&mut buf)?;
            if len == 0 {
                println!("\r\n{}", exit_message());
                return Ok(0);
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
        "{} attached session {} {}",
        style.brand(),
        style.id(session_id),
        style.muted("(Ctrl-\\ to exit)")
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
    byte == 0x04 || byte == (b'\\' & 0x1f)
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

fn send_winch(stream: &mut UnixStream) -> io::Result<()> {
    let mut ws = unsafe { mem::zeroed::<libc::winsize>() };
    if unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0 {
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
    let mut sessions = read_sessions()?;

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
            let cwd = Path::new(&session.cwd);
            let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
            cwd.starts_with(&filter)
        });
    }

    sessions.sort_by_key(|session| session.id.parse::<u32>().unwrap_or(u32::MAX));

    if args.json {
        let values = sessions
            .iter()
            .map(|s| session_json(s, args.tail))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&values).unwrap());
        return Ok(());
    }

    if sessions.is_empty() {
        let style = Style::stdout();
        println!("{} {}", style.brand(), style.muted("no matching sessions"));
        return Ok(());
    }

    let style = Style::stdout();
    for session in sessions {
        println!(
            "{} {} {}",
            style.muted("session"),
            style.id(&session.id),
            style.muted("─".repeat(32))
        );
        println!("{} {}", style.label("dir"), style.path(&session.cwd));
        println!(
            "{} {}",
            style.label("cmd"),
            style.command(command_for_session(&session))
        );
        println!("{} {}", style.label("status"), style.success("running"));
        println!(
            "{} {}",
            style.label("attached"),
            if is_attached(&session.socket) {
                style.success("yes")
            } else {
                style.muted("no")
            }
        );
        let lines = tail_lines(&session.log, args.tail, false);
        if !lines.is_empty() {
            println!(
                "\n{}",
                style.muted(format!("output tail ({} lines)", lines.len()))
            );
            for line in lines {
                println!("  {}", style.muted(line));
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

fn session_json(session: &SessionRecord, tail: usize) -> serde_json::Value {
    serde_json::json!({
        "session": session.id,
        "dir": session.cwd,
        "latestCommand": command_for_session(session),
        "running": Path::new(&session.socket).exists(),
        "attached": is_attached(&session.socket),
        "pid": read_pid(&session.pid_file),
        "lastActivity": "now",
        "output": { "tail": tail_lines(&session.log, tail, false) }
    })
}

fn find_session(id: Option<&str>) -> io::Result<SessionRecord> {
    let id = id.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing session"))?;
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
            format!("no such muxi session: {id}"),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("ambiguous muxi session: {id}"),
        )),
    }
}

fn cmd_attach(id: Option<&str>) -> io::Result<()> {
    if let Ok(session) = env::var("MUXI_SESSION") {
        let style = Style::stderr();
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "already inside muxi session {}; detach first with {}",
                style.id(session),
                style.command("Ctrl-\\")
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
    let text = fs::read_to_string(session.log).unwrap_or_default();
    let text = if args.raw {
        text
    } else {
        strip_ansi(&text).replace('\r', "")
    };
    if let Some(tail) = args.tail {
        let lines = text.lines().rev().take(tail).collect::<Vec<_>>();
        for line in lines.into_iter().rev() {
            println!("{line}");
        }
    } else {
        print!("{text}");
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

fn cmd_info(args: &InfoArgs) -> io::Result<()> {
    let session = env::var("MUXI_SESSION")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "not inside a muxi session"))?;
    let style = Style::stdout();
    println!("currently in the session {}", style.id(session));
    if args.debug {
        if let Ok(socket) = env::var("MUXI_SOCKET") {
            println!("socket {}", style.path(socket));
        }
    }
    Ok(())
}

fn read_pid(path: &str) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn is_attached(socket: &str) -> bool {
    fs::metadata(socket)
        .map(|m| m.permissions().mode() & 0o100 != 0)
        .unwrap_or(false)
}

trait PermissionsExt {
    fn mode(&self) -> u32;
}

impl PermissionsExt for fs::Permissions {
    fn mode(&self) -> u32 {
        std::os::unix::fs::PermissionsExt::mode(self)
    }
}

fn command_for_session(session: &SessionRecord) -> String {
    if let Some(pid) = read_pid(&session.pid_file) {
        if let Ok(output) = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }
    session.shell.clone()
}

fn tail_lines(path: &str, limit: usize, raw: bool) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let text = if raw {
        text
    } else {
        strip_ansi(&text).replace('\r', "")
    };
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

fn strip_ansi(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                i += 2;
                while i < bytes.len() && bytes[i] != 0x07 {
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == 0x07 {
                    i += 1;
                }
                continue;
            }
            i += 2;
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if (0x40..=0x7e).contains(&b) {
                    break;
                }
            }
            continue;
        }
        if bytes[i] >= 0x20 || bytes[i] == b'\n' || bytes[i] == b'\t' {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
