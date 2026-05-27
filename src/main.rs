use clap::{Args, Parser, Subcommand};
use inquire::Confirm;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
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
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BUF_SIZE: usize = 4096;
const MSG_PUSH: u8 = 0;
const MSG_ATTACH: u8 = 1;
const MSG_DETACH: u8 = 2;
const MSG_WINCH: u8 = 3;
const MSG_DETACH_SESSION: u8 = 4;
const SKILL_NAME: &str = "hitch";
const SKILL_VERSION: &str = "1";
const SKILL_MD: &str = include_str!("../SKILL.md");
const HITCH_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_SOURCE: &str = match option_env!("HITCH_INSTALL_SOURCE") {
    Some(source) => source,
    None => "dev",
};
const NPM_PACKAGE_NAME: &str = "hitch-cli";
const UPDATE_CACHE_TTL_SECS: u64 = 6 * 60 * 60;
const NPM_REGISTRY_URL: &str = "https://registry.npmjs.org/hitch-cli";
const HITCH_CWD_SYNC_FILE: &str = "HITCH_CWD_SYNC_FILE";

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const ATTACH_HISTORY_BYTES: u64 = 64 * 1024;
const ACTIVE_COMMAND_HEAD_LINES: usize = 5;
const CONTEXT_TAIL_LINES: usize = 20;
const CONTEXT_SINGLE_HEAD_LINES: usize = 10;
const CONTEXT_SINGLE_TAIL_LINES: usize = 80;
const CONTEXT_LINE_MAX_CHARS: usize = 1000;
const CONTEXT_OUTPUT_WINDOW_BYTES: u64 = 64 * 1024;
const CONTEXT_OUTPUT_MAX_BYTES: u64 = 1024 * 1024;
const EXIT_PARENT_CODE: i32 = 42;
static WINCH_PENDING: AtomicBool = AtomicBool::new(false);

struct Style {
    enabled: bool,
}

#[derive(Parser)]
#[command(
    name = "hitch",
    version,
    after_help = "Agents: run `hitch context` before starting dev servers, watchers, tunnels, REPLs, or log tails. Use `capture` to inspect output and `send-keys` to interact."
)]
struct Cli {
    #[arg(long)]
    skill: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Share this terminal with agents.
    #[command(alias = "start")]
    On,
    /// Stop sharing this terminal.
    #[command(alias = "stop")]
    Off,
    /// Show whether this terminal is being shared.
    Status(StatusArgs),
    /// Run setup wizard or install shell integration / agent skill directly.
    Setup(SetupArgs),
    /// Show shared terminals and compact context.
    Context(ContextArgs),
    /// Send input to a shared terminal.
    SendKeys(SendKeysArgs),
    /// Print a faithful terminal transcript.
    Capture(CapturePaneArgs),
    #[command(hide = true)]
    CapturePane(CapturePaneArgs),
    /// Kill a shared terminal.
    Kill(SessionArg),
    #[command(hide = true)]
    KillSession(SessionArg),
    #[command(hide = true)]
    KillSessions(SessionArg),
}

#[derive(Args)]
struct SetupArgs {
    #[command(subcommand)]
    command: Option<SetupCommand>,
}

#[derive(Subcommand)]
enum SetupCommand {
    /// Install shell integration.
    Shell,
    /// Install the optional agent skill.
    Skill,
}

#[derive(Args)]
struct ContextArgs {
    #[arg(value_name = "TERMINAL")]
    terminal: Option<String>,
    #[arg(long)]
    all: bool,
    #[arg(long)]
    dir: Option<PathBuf>,
    #[arg(long)]
    head: Option<usize>,
    #[arg(long)]
    tail: Option<usize>,
    #[arg(long)]
    no_output: bool,
}

#[derive(Args)]
struct SessionArg {
    #[arg(value_name = "TERMINAL")]
    session: String,
}

#[derive(Args)]
struct SendKeysArgs {
    #[arg(short = 't', long = "target", help = "Terminal id")]
    target: String,
    #[arg(
        long,
        help = "Wait mode: output, finish, quiet:<duration>, or time:<duration>"
    )]
    wait: Option<String>,
    #[arg(long, help = "Maximum wait duration. Supports ms, s, or m")]
    timeout: Option<String>,
    #[arg(long, help = "Print this many new visible output lines after sending")]
    tail: Option<usize>,
    #[arg(long, help = "Send input even when a process is running")]
    force: bool,
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Keys or text to send. Options must come before this"
    )]
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
    #[arg(short = 'E', allow_hyphen_values = true)]
    end: Option<isize>,
    #[arg(short = 'e')]
    escapes: bool,
    #[arg(short = 'C', hide = true)]
    _escape_non_printable: bool,
    #[arg(short = 'J', hide = true)]
    _join_wrapped: bool,
    #[arg(short = 'N', hide = true)]
    _preserve_trailing_spaces: bool,
    #[arg(short = 'T', hide = true)]
    _trim_trailing_empty: bool,
    #[arg(short = 'a', hide = true)]
    _alternate_screen: bool,
    #[arg(short = 'q', hide = true)]
    _quiet: bool,
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

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCache {
    #[serde(rename = "checkedAt")]
    checked_at: u64,
    #[serde(rename = "installSource")]
    install_source: String,
    #[serde(rename = "latestVersion")]
    latest_version: Option<String>,
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
    payload: Vec<u8>,
}

enum WaitMode {
    Output,
    Quiet(Duration),
    Time(Duration),
    Finish,
}

enum WaitOutcome {
    Satisfied,
    TimedOut,
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

        if let Some(cwd) = cwd_for_pgrp(pgrp) {
            self.state.current_dir = Some(cwd);
        }

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

struct TitleFilter {
    state: u8,
    code: u8,
    title: Vec<u8>,
    mode: TitleFilterMode,
    cwd: Option<String>,
}

enum TitleFilterMode {
    Strip,
    Rewrite { prefix: Vec<u8> },
}

#[derive(Default)]
struct AltScreenLogFilter {
    alt_screen: bool,
    state: u8,
    seq: Vec<u8>,
}

#[derive(Default)]
struct AltScreenTracker {
    alt_screen: bool,
    state: u8,
    seq: Vec<u8>,
}

impl TitleFilter {
    fn strip() -> Self {
        Self {
            state: 0,
            code: 0,
            title: Vec::new(),
            mode: TitleFilterMode::Strip,
            cwd: None,
        }
    }

    fn rewrite(session_id: &str) -> Self {
        Self {
            state: 0,
            code: 0,
            title: Vec::new(),
            mode: TitleFilterMode::Rewrite {
                prefix: format!("#{session_id} ").into_bytes(),
            },
            cwd: None,
        }
    }

    fn set_cwd(&mut self, cwd: Option<String>) {
        self.cwd = cwd;
    }

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
                        self.code = b;
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
                        self.title.clear();
                        self.state = 4;
                    } else {
                        out.push(0x1b);
                        out.push(b']');
                        out.push(self.code);
                        out.push(b);
                        self.state = 0;
                    }
                }
                4 => {
                    if b == 0x07 {
                        self.emit_title(&mut out, &[0x07]);
                        self.state = 0;
                    } else if b == 0x1b {
                        self.state = 5;
                    } else {
                        self.title.push(b);
                    }
                }
                5 => {
                    if b == b'\\' {
                        self.emit_title(&mut out, &[0x1b, b'\\']);
                        self.state = 0;
                    } else {
                        self.title.push(0x1b);
                        self.title.push(b);
                        self.state = 4;
                    }
                }
                _ => self.state = 0,
            }
        }
        out
    }

    fn emit_title(&self, out: &mut Vec<u8>, terminator: &[u8]) {
        let TitleFilterMode::Rewrite { prefix } = &self.mode else {
            return;
        };
        out.push(0x1b);
        out.push(b']');
        out.push(self.code);
        out.push(b';');
        out.extend_from_slice(prefix);
        if let Some(title) = self.normalized_title() {
            out.extend_from_slice(title.as_bytes());
        } else {
            out.extend_from_slice(&self.title);
        }
        out.extend_from_slice(terminator);
    }

    fn normalized_title(&self) -> Option<String> {
        let cwd = self.cwd.as_deref()?;
        let title = std::str::from_utf8(&self.title).ok()?.trim();
        if is_shell_idle_title(title) {
            return Some(shorten_home(cwd));
        }
        None
    }
}

fn is_shell_idle_title(title: &str) -> bool {
    let Some((user, rest)) = title.split_once('@') else {
        return false;
    };
    let Some((host, path)) = rest.split_once(':') else {
        return false;
    };
    is_shell_title_name(user)
        && is_shell_title_host(host)
        && !path.is_empty()
        && !path.chars().any(char::is_whitespace)
}

fn is_shell_title_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn is_shell_title_host(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

impl AltScreenLogFilter {
    fn filter(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        for &b in input {
            match self.state {
                0 => {
                    if b == 0x1b {
                        self.seq.clear();
                        self.seq.push(b);
                        self.state = 1;
                    } else if !self.alt_screen {
                        out.push(b);
                    }
                }
                1 => {
                    self.seq.push(b);
                    if b == b'[' {
                        self.state = 2;
                    } else {
                        self.flush_sequence(&mut out);
                    }
                }
                2 => {
                    self.seq.push(b);
                    if (0x40..=0x7e).contains(&b) {
                        if self.is_alt_screen_sequence() {
                            self.alt_screen = b == b'h';
                            self.seq.clear();
                        } else {
                            self.flush_sequence(&mut out);
                        }
                        self.state = 0;
                    }
                }
                _ => self.state = 0,
            }
        }
        out
    }

    fn flush_sequence(&mut self, out: &mut Vec<u8>) {
        if !self.alt_screen {
            out.extend_from_slice(&self.seq);
        }
        self.seq.clear();
        self.state = 0;
    }

    fn is_alt_screen_sequence(&self) -> bool {
        is_alt_screen_sequence(&self.seq)
    }
}

impl AltScreenTracker {
    fn observe(&mut self, input: &[u8]) {
        for &b in input {
            match self.state {
                0 => {
                    if b == 0x1b {
                        self.seq.clear();
                        self.seq.push(b);
                        self.state = 1;
                    }
                }
                1 => {
                    self.seq.push(b);
                    self.state = if b == b'[' { 2 } else { 0 };
                }
                2 => {
                    self.seq.push(b);
                    if (0x40..=0x7e).contains(&b) {
                        if is_alt_screen_sequence(&self.seq) {
                            self.alt_screen = b == b'h';
                        }
                        self.state = 0;
                    }
                }
                _ => self.state = 0,
            }
        }
    }
}

fn is_alt_screen_sequence(seq: &[u8]) -> bool {
    matches!(
        seq,
        b"\x1b[?47h"
            | b"\x1b[?47l"
            | b"\x1b[?1047h"
            | b"\x1b[?1047l"
            | b"\x1b[?1049h"
            | b"\x1b[?1049l"
    )
}

fn main() {
    if let Err(err) = run() {
        if err.kind() == io::ErrorKind::WouldBlock {
            process::exit(1);
        }
        let style = Style::stderr();
        eprintln!("{} {}", style.error("error:"), err);
        process::exit(1);
    }
}

fn run() -> io::Result<()> {
    if top_level_skill_requested() {
        return cmd_print_skill();
    }

    if top_level_version_requested() {
        print_version();
        return Ok(());
    }

    if top_level_help_requested() {
        print_top_level_help();
        return Ok(());
    }

    let cli = Cli::parse();
    if let Some(command) = cli.command {
        return match command {
            Commands::On => cmd_start(),
            Commands::Off => cmd_stop(),
            Commands::Context(args) => cmd_context(&args),
            Commands::SendKeys(args) => cmd_send_keys(&args),
            Commands::Capture(args) | Commands::CapturePane(args) => cmd_capture_pane(&args),
            Commands::Kill(args) | Commands::KillSession(args) | Commands::KillSessions(args) => {
                cmd_kill_session(Some(&args.session))
            }
            Commands::Status(args) => cmd_status(&args),
            Commands::Setup(args) => cmd_setup(&args),
        };
    }

    if env::var_os("HITCH_SESSION").is_some() {
        cmd_status(&StatusArgs { debug: false })
    } else {
        cmd_start()
    }
}

fn top_level_skill_requested() -> bool {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return false;
    }
    matches!(args[0].to_str(), Some("--skill"))
}

fn top_level_help_requested() -> bool {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return false;
    }
    matches!(args[0].to_str(), Some("-h") | Some("--help") | Some("help"))
}

fn top_level_version_requested() -> bool {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return false;
    }
    matches!(
        args[0].to_str(),
        Some("-v") | Some("-V") | Some("--version")
    )
}

fn print_version() {
    println!("hitch {HITCH_VERSION}");
}

fn print_top_level_help() {
    println!("Usage: hitch [COMMAND]");
    println!();
    println!("User commands:");
    println!("  on           Share this terminal with agents");
    println!("  off          Stop sharing this terminal");
    println!("  status       Show whether this terminal is being shared");
    println!("  setup        Run setup wizard or install shell integration / agent skill");
    println!();
    println!("Agent commands:");
    println!("  context      Show shared terminals and compact context");
    println!("  capture      Print a faithful terminal transcript");
    println!("  send-keys    Send input to a shared terminal");
    println!("  kill         Kill a shared terminal");
    println!();
    println!("Other commands:");
    println!("  help         Print help for a command");
    println!();
    println!("Options:");
    println!("      --skill    Print agent instructions");
    println!("  -h, --help     Print help");
    println!("  -v, --version  Print version");
    println!();
    println!(
        "For agent: run `hitch context` before starting dev servers, watchers, tunnels, REPLs, or log tails. Use `hitch --skill` to learn how to use hitch."
    );
}

fn state_dir() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("hitch")
    } else {
        PathBuf::from(env::var_os("HOME").unwrap_or_else(|| OsString::from(".")))
            .join(".local/state/hitch")
    }
}

fn cache_dir() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("hitch")
    } else {
        PathBuf::from(env::var_os("HOME").unwrap_or_else(|| OsString::from(".")))
            .join(".cache/hitch")
    }
}

fn update_cache_path() -> PathBuf {
    cache_dir().join("update.json")
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

    ensure_shell_integration()?;

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
    if let Some(warning) = outdated_skill_warning() {
        println!("{}", style.muted(warning));
    }
    if let Some(warning) = update_warning() {
        println!("{}", style.muted(warning));
    }

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
    maybe_refresh_update_cache();
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
    let mut filter = TitleFilter::rewrite(&record.id);
    let mut log_title_filter = TitleFilter::strip();
    let mut log_filter = AltScreenLogFilter::default();
    let mut alt_screen = AltScreenTracker::default();
    let mut commands = CommandTracker::new(record, child_pid);
    let mut last_broadcast_cwd = commands.state.current_dir.clone();
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
            filter.set_cwd(commands.state.current_dir.clone());
            broadcast_cwd_if_changed(&commands, &mut last_broadcast_cwd, &mut clients);
            continue;
        }

        if unsafe { libc::FD_ISSET(listener.as_raw_fd(), &readfds) } {
            match listener.accept() {
                Ok((stream, _)) => {
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
            filter.set_cwd(commands.state.current_dir.clone());
            broadcast_cwd_if_changed(&commands, &mut last_broadcast_cwd, &mut clients);
            if !drain_pty_output(
                pty_fd,
                &mut filter,
                &mut log_title_filter,
                &mut log_filter,
                &mut alt_screen,
                &mut log,
                &mut commands,
                &mut clients,
            )? {
                break;
            }
        }

        let mut i = 0;
        while i < clients.len() {
            let fd = clients[i].stream.as_raw_fd();
            if unsafe { libc::FD_ISSET(fd, &readfds) } {
                match read_packet(&mut clients[i].stream) {
                    Ok(Some(packet)) if packet.typ == MSG_DETACH_SESSION => {
                        commands.refresh(pty_fd);
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

fn drain_pty_output(
    pty_fd: RawFd,
    filter: &mut TitleFilter,
    log_title_filter: &mut TitleFilter,
    log_filter: &mut AltScreenLogFilter,
    alt_screen: &mut AltScreenTracker,
    log: &mut fs::File,
    commands: &mut CommandTracker,
    clients: &mut Vec<Client>,
) -> io::Result<bool> {
    loop {
        let mut buf = [0u8; BUF_SIZE];
        let len = unsafe { libc::read(pty_fd, buf.as_mut_ptr().cast(), buf.len()) };
        if len <= 0 {
            return Ok(false);
        }
        process_pty_output_chunk(
            &buf[..len as usize],
            filter,
            log_title_filter,
            log_filter,
            alt_screen,
            log,
            commands,
            clients,
        );
        if !fd_readable_now(pty_fd)? {
            return Ok(true);
        }
    }
}

fn process_pty_output_chunk(
    raw: &[u8],
    filter: &mut TitleFilter,
    log_title_filter: &mut TitleFilter,
    log_filter: &mut AltScreenLogFilter,
    alt_screen: &mut AltScreenTracker,
    log: &mut fs::File,
    commands: &mut CommandTracker,
    clients: &mut Vec<Client>,
) {
    let was_alt_screen = alt_screen.alt_screen;
    alt_screen.observe(raw);
    let filtered: Cow<'_, [u8]> = if was_alt_screen && alt_screen.alt_screen {
        Cow::Borrowed(raw)
    } else {
        Cow::Owned(filter.filter(raw))
    };
    if filtered.is_empty() {
        return;
    }
    if !was_alt_screen || !alt_screen.alt_screen {
        let title_free = log_title_filter.filter(raw);
        let loggable = log_filter.filter(&title_free);
        if !loggable.is_empty() {
            let _ = log.write_all(&loggable);
            commands.capture_output(&loggable);
        }
    }
    clients.retain_mut(|client| {
        if !client.attached {
            return true;
        }
        client.stream.write_all(filtered.as_ref()).is_ok()
    });
}

fn broadcast_cwd_if_changed(
    commands: &CommandTracker,
    last_broadcast_cwd: &mut Option<String>,
    clients: &mut Vec<Client>,
) {
    if commands.state.current_dir == *last_broadcast_cwd {
        return;
    }
    *last_broadcast_cwd = commands.state.current_dir.clone();
    let Some(cwd) = commands.state.current_dir.as_deref() else {
        return;
    };
    broadcast_to_attached(clients, &osc7_cwd(cwd));
}

fn broadcast_to_attached(clients: &mut Vec<Client>, bytes: &[u8]) {
    clients.retain_mut(|client| {
        if !client.attached {
            return true;
        }
        client.stream.write_all(bytes).is_ok()
    });
}

fn osc7_cwd(cwd: &str) -> Vec<u8> {
    format!("\x1b]7;file://localhost{}\x07", percent_encode_path(cwd)).into_bytes()
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'-' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn fd_readable_now(fd: RawFd) -> io::Result<bool> {
    let mut readfds = unsafe { mem::zeroed::<libc::fd_set>() };
    unsafe {
        libc::FD_ZERO(&mut readfds);
        libc::FD_SET(fd, &mut readfds);
    }
    let mut timeout = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let rc = unsafe {
        libc::select(
            fd + 1,
            &mut readfds,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut timeout,
        )
    };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(err);
    }
    Ok(rc > 0 && unsafe { libc::FD_ISSET(fd, &readfds) })
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
            commands.note_input();
            write_all_fd(pty_fd, &packet.payload)?;
        }
        MSG_ATTACH => {
            client.attached = true;
            commands.refresh(pty_fd);
            if let Some(cwd) = commands.state.current_dir.as_deref() {
                let _ = client.stream.write_all(&osc7_cwd(cwd));
            }
        }
        MSG_DETACH => {
            commands.refresh(pty_fd);
            client.attached = false;
        }
        MSG_WINCH => unsafe {
            if packet.payload.len() == mem::size_of::<libc::winsize>() {
                let ws = packet.payload.as_ptr().cast::<libc::winsize>();
                libc::ioctl(pty_fd, libc::TIOCSWINSZ, ws);
            }
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
            if !drain_socket_to_stdout(&mut stream)? {
                write_parent_cwd_sync(session_id);
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
            io::stdout().flush()?;
        }
        if unsafe { libc::FD_ISSET(stdin_fd, &readfds) } {
            let mut buf = [0u8; BUF_SIZE];
            let len = unsafe { libc::read(stdin_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if len <= 0 {
                write_parent_cwd_sync(session_id);
                return Ok(1);
            }
            if is_detach_key(buf[0]) {
                send_packet(&mut stream, MSG_DETACH, &[])?;
                write_parent_cwd_sync(session_id);
                println!("\r\n{}", exit_message());
                return Ok(0);
            }
            send_packet(&mut stream, MSG_PUSH, &buf[..len as usize])?;
        }
    }
}

fn write_parent_cwd_sync(session_id: &str) {
    let Some(path) = env::var_os(HITCH_CWD_SYNC_FILE).map(PathBuf::from) else {
        return;
    };
    let state = read_session_state(session_id);
    let cwd = state
        .foreground_pgrp
        .and_then(cwd_for_pgrp)
        .or(state.current_dir);
    let Some(cwd) = cwd else {
        return;
    };
    if Path::new(&cwd).is_dir() {
        let _ = fs::write(path, cwd);
    }
}

fn drain_socket_to_stdout(stream: &mut UnixStream) -> io::Result<bool> {
    loop {
        let mut buf = [0u8; BUF_SIZE];
        let len = stream.read(&mut buf)?;
        if len == 0 {
            return Ok(false);
        }
        io::stdout().write_all(&buf[..len])?;
        if !fd_readable_now(stream.as_raw_fd())? {
            return Ok(true);
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

fn head_raw_bytes(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new().read(true).open(path)?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn file_len(path: &str) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn read_bytes_from(path: &str, offset: u64) -> io::Result<Vec<u8>> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    use std::io::Seek;
    file.seek(io::SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
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
    let len: u32 = payload.len().try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "packet payload is too large to send",
        )
    })?;
    stream.write_all(&[typ])?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(payload)
}

fn read_packet(stream: &mut UnixStream) -> io::Result<Option<Packet>> {
    let mut header = [0u8; 5];
    match stream.read_exact(&mut header) {
        Ok(()) => {
            let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
            let mut payload = vec![0u8; len];
            stream.read_exact(&mut payload)?;
            Ok(Some(Packet {
                typ: header[0],
                payload,
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

fn cmd_context(args: &ContextArgs) -> io::Result<()> {
    if let Some(id) = &args.terminal {
        let session = find_session(Some(id))?;
        let head = args.head.unwrap_or(CONTEXT_SINGLE_HEAD_LINES);
        let tail = args.tail.unwrap_or(CONTEXT_SINGLE_TAIL_LINES);
        println!("terminal: {}", session.id);
        println!();
        print_context_session(&session, head, tail, args.no_output)?;
        return Ok(());
    }

    let all_sessions = read_sessions()?;
    let total_count = all_sessions.len();
    let mut sessions = all_sessions;
    let head = args.head.unwrap_or(ACTIVE_COMMAND_HEAD_LINES);
    let tail = args.tail.unwrap_or(CONTEXT_TAIL_LINES);

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
        println!("----- terminal {} -----", style.id(&session.id));
        print_context_session(&session, head, tail, args.no_output)?;
        println!();
    }

    Ok(())
}

fn print_context_session(
    session: &SessionRecord,
    head_limit: usize,
    tail_limit: usize,
    no_output: bool,
) -> io::Result<()> {
    print_context_session_with(session, head_limit, tail_limit, no_output, false)
}

fn print_context_session_to_stderr(
    session: &SessionRecord,
    head_limit: usize,
    tail_limit: usize,
    no_output: bool,
) -> io::Result<()> {
    print_context_session_with(session, head_limit, tail_limit, no_output, true)
}

fn context_line(stderr: bool, line: impl AsRef<str>) {
    if stderr {
        eprintln!("{}", line.as_ref());
    } else {
        println!("{}", line.as_ref());
    }
}

fn print_context_session_with(
    session: &SessionRecord,
    head_limit: usize,
    tail_limit: usize,
    no_output: bool,
    stderr: bool,
) -> io::Result<()> {
    let style = Style::plain();
    let state = read_session_state(&session.id);
    let cwd = current_dir_for_session(session, &state);
    let activity = state
        .last_activity_at
        .map(time_ago)
        .unwrap_or_else(|| "unknown".to_string());

    context_line(
        stderr,
        format!("current dir: {}", style.path(shorten_home(&cwd))),
    );
    context_line(stderr, format!("last input was {activity}"));
    if state.command_running {
        let duration = state
            .command_started_at
            .map(running_for)
            .unwrap_or_else(|| "unknown time".to_string());
        context_line(
            stderr,
            format!("process is running for {}", style.command(duration)),
        );
    } else {
        context_line(stderr, "no actively running commands");
    }

    if no_output {
        return Ok(());
    }

    let mut printed_output = false;
    if state.command_running {
        let head = list_head_lines(&active_output_path(&session.id), head_limit);
        let tail = list_tail_lines(&session.log, tail_limit);
        if !head.is_empty() && !contains_line_sequence(&tail, &head) {
            context_line(stderr, "");
            context_line(
                stderr,
                format!("--- active output head ({} lines) ---", head.len()),
            );
            for line in head {
                context_output_line(stderr, &line);
            }
            printed_output = true;
        }
        if !tail.is_empty() {
            context_line(stderr, "");
            context_line(
                stderr,
                format!("--- recent output ({} lines) ---", tail.len()),
            );
            for line in tail {
                context_output_line(stderr, &line);
            }
            printed_output = true;
        }
    } else {
        let tail = list_tail_lines(&session.log, tail_limit);
        if !tail.is_empty() {
            context_line(stderr, "");
            context_line(
                stderr,
                format!("--- recent output ({} lines) ---", tail.len()),
            );
            for line in tail {
                context_output_line(stderr, &line);
            }
            printed_output = true;
        }
    }
    if !printed_output {
        context_line(stderr, "");
        if state.command_running {
            context_line(stderr, "no visible output yet");
        } else {
            context_line(stderr, "no visible output");
        }
    }
    Ok(())
}

fn context_output_line(stderr: bool, line: &str) {
    context_line(stderr, truncate_context_line(line));
}

fn truncate_context_line(line: &str) -> Cow<'_, str> {
    let mut chars = line.chars();
    let truncated = chars
        .by_ref()
        .take(CONTEXT_LINE_MAX_CHARS)
        .collect::<String>();
    let remaining = chars.count();
    if remaining == 0 {
        Cow::Borrowed(line)
    } else {
        Cow::Owned(format!("{truncated}... [truncated {remaining} chars]"))
    }
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
                "can't turn on while already sharing terminal {} {}",
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
    validate_send_keys_args(args)?;
    guard_send_keys(&session, args)?;
    maybe_print_idle_interrupt_note(&session, args);
    let start_offset = file_len(&session.log);
    let mut payload = Vec::new();
    for arg in &args.keys {
        payload.extend(key_to_bytes(arg));
    }
    let mut stream = UnixStream::connect(&session.socket)?;
    send_packet(&mut stream, MSG_PUSH, &payload)?;
    let wait = args.wait.as_deref().map(parse_wait_mode).transpose()?;
    let output_limit = args.tail.or(wait.as_ref().map(|_| 40));
    let mut wait_timeout_note = None;
    if let Some(mode) = wait {
        let timeout = parse_duration(args.timeout.as_deref().unwrap_or("30s"))?;
        if matches!(
            wait_for_send_keys(&session, start_offset, &mode, timeout),
            WaitOutcome::TimedOut
        ) {
            wait_timeout_note = Some(wait_timeout_message(&mode, timeout));
        }
    }
    if let Some(limit) = output_limit {
        println!("terminal: {}", session.id);
        if let Some(note) = wait_timeout_note {
            println!("note: {note}");
        }
        let lines = rendered_lines_from_offset(&session.log, start_offset, limit);
        if lines.is_empty() {
            println!("no new output");
        } else {
            println!();
            println!("--- new output ({} lines) ---", lines.len());
            for line in lines {
                println!("{line}");
            }
        }
    }
    Ok(())
}

fn validate_send_keys_args(args: &SendKeysArgs) -> io::Result<()> {
    for key in &args.keys {
        if matches!(key.as_str(), "--wait" | "--timeout" | "--tail" | "--force")
            || key.starts_with("--wait=")
            || key.starts_with("--timeout=")
            || key.starts_with("--tail=")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "send-keys options must come before key arguments",
            ));
        }
    }
    Ok(())
}

fn guard_send_keys(session: &SessionRecord, args: &SendKeysArgs) -> io::Result<()> {
    if args.force {
        return Ok(());
    }
    let state = read_session_state(&session.id);
    if !state.command_running || control_only_keys(&args.keys) || starts_with_interrupt(&args.keys)
    {
        return Ok(());
    }

    eprintln!(
        "error: terminal {} has a running process; use --force to send input anyway",
        session.id
    );
    eprintln!();
    eprintln!("terminal: {}", session.id);
    print_context_session_to_stderr(session, 3, 10, false)?;
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "send-keys refused while process is running",
    ))
}

fn maybe_print_idle_interrupt_note(session: &SessionRecord, args: &SendKeysArgs) {
    if args.keys.len() == 1 && starts_with_interrupt(&args.keys) {
        let state = read_session_state(&session.id);
        if !state.command_running {
            if args.wait.is_some() || args.tail.is_some() {
                println!("note: no running process to interrupt");
            } else {
                println!(
                    "note: terminal {} has no running process to interrupt",
                    session.id
                );
            }
        }
    }
}

fn control_only_keys(keys: &[String]) -> bool {
    !keys.is_empty()
        && keys
            .iter()
            .all(|key| key.starts_with("C-") && key.len() == 3)
}

fn starts_with_interrupt(keys: &[String]) -> bool {
    matches!(keys.first().map(String::as_str), Some("C-c"))
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

fn parse_wait_mode(value: &str) -> io::Result<WaitMode> {
    if value == "output" {
        return Ok(WaitMode::Output);
    }
    if value == "finish" {
        return Ok(WaitMode::Finish);
    }
    if let Some(duration) = value.strip_prefix("quiet:") {
        return Ok(WaitMode::Quiet(parse_duration(duration)?));
    }
    if let Some(duration) = value.strip_prefix("time:") {
        return Ok(WaitMode::Time(parse_duration(duration)?));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "wait must be output, finish, quiet:<duration>, or time:<duration>",
    ))
}

fn parse_duration(value: &str) -> io::Result<Duration> {
    let Some((number, unit)) = split_duration(value) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "duration must end with ms, s, or m",
        ));
    };
    let amount = number.parse::<u64>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "duration must start with a positive integer",
        )
    })?;
    match unit {
        "ms" => Ok(Duration::from_millis(amount)),
        "s" => Ok(Duration::from_secs(amount)),
        "m" => Ok(Duration::from_secs(amount * 60)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "duration must end with ms, s, or m",
        )),
    }
}

fn split_duration(value: &str) -> Option<(&str, &str)> {
    for unit in ["ms", "s", "m"] {
        if let Some(number) = value.strip_suffix(unit) {
            if !number.is_empty() {
                return Some((number, unit));
            }
        }
    }
    None
}

fn wait_timeout_message(mode: &WaitMode, timeout: Duration) -> String {
    let timeout = format_duration(timeout);
    match mode {
        WaitMode::Output => format!("timed out after {timeout} waiting for output"),
        WaitMode::Quiet(_) => {
            format!("timed out after {timeout} waiting for output to become quiet")
        }
        WaitMode::Time(duration) => {
            format!(
                "timed out after {timeout} waiting for {}",
                format_duration(*duration)
            )
        }
        WaitMode::Finish => format!("timed out after {timeout} waiting for command to finish"),
    }
}

fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        return format!("{millis}ms");
    }
    if millis % 60000 == 0 {
        return format!("{}m", millis / 60000);
    }
    if millis % 1000 == 0 {
        return format!("{}s", millis / 1000);
    }
    format!("{millis}ms")
}

fn wait_for_send_keys(
    session: &SessionRecord,
    start_offset: u64,
    mode: &WaitMode,
    timeout: Duration,
) -> WaitOutcome {
    let start = Instant::now();
    let mut last_len = file_len(&session.log);
    let mut last_change = Instant::now();
    let mut saw_output = last_len > start_offset;
    let mut saw_running = false;
    let mut idle_since: Option<Instant> = None;
    let mut output_baseline: Option<u64> = None;

    loop {
        if start.elapsed() >= timeout {
            return WaitOutcome::TimedOut;
        }

        let len = file_len(&session.log);
        if len != last_len {
            last_len = len;
            last_change = Instant::now();
        }
        if len > start_offset {
            saw_output = true;
        }

        match mode {
            WaitMode::Output => {
                if start.elapsed() >= Duration::from_millis(250)
                    && rendered_lines_from_offset(&session.log, start_offset, 3).len() >= 2
                {
                    return WaitOutcome::Satisfied;
                }
                let baseline = output_baseline.get_or_insert_with(|| {
                    let state = read_session_state(&session.id);
                    if state.command_running {
                        len
                    } else {
                        // Echoed input/prompt redraws happen immediately after sending keys.
                        // Give the shell a short window to start the foreground command before
                        // treating later bytes as command output.
                        if start.elapsed() >= Duration::from_millis(250) {
                            len
                        } else {
                            u64::MAX
                        }
                    }
                });
                if *baseline != u64::MAX && len > *baseline {
                    return WaitOutcome::Satisfied;
                }
                if *baseline == u64::MAX && start.elapsed() >= Duration::from_millis(250) {
                    *baseline = len;
                }
            }
            WaitMode::Quiet(quiet) if last_change.elapsed() >= *quiet => {
                return WaitOutcome::Satisfied;
            }
            WaitMode::Time(duration) if start.elapsed() >= *duration => {
                return WaitOutcome::Satisfied;
            }
            WaitMode::Finish => {
                let state = read_session_state(&session.id);
                if state.command_running {
                    saw_running = true;
                    idle_since = None;
                } else if saw_running {
                    let idle_since = idle_since.get_or_insert_with(Instant::now);
                    if idle_since.elapsed() >= Duration::from_millis(200)
                        && last_change.elapsed() >= Duration::from_millis(200)
                    {
                        return WaitOutcome::Satisfied;
                    }
                } else if saw_output
                    && start.elapsed() >= Duration::from_millis(1500)
                    && last_change.elapsed() >= Duration::from_millis(500)
                {
                    return WaitOutcome::Satisfied;
                }
            }
            _ => {}
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn cmd_capture_pane(args: &CapturePaneArgs) -> io::Result<()> {
    let session = find_session(Some(&args.target))?;
    let raw = args.raw || args.escapes;
    if let Some(start) = args.start {
        let lines = if raw {
            raw_lines_range(&session.log, start, args.end)
        } else {
            rendered_lines_range(&session.log, start, args.end)
        };
        for line in lines {
            println!("{line}");
        }
    } else if let Some(tail) = args.tail {
        let lines = if raw {
            raw_tail_lines(&session.log, tail)
        } else {
            rendered_tail_lines(&session.log, tail)
        };
        for line in lines {
            println!("{line}");
        }
    } else {
        let text = if raw {
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

fn cmd_install_skill() -> io::Result<()> {
    let source = write_embedded_skill_dir()?;
    let source_arg = source.to_string_lossy().into_owned();
    let args = ["--yes", "skills", "add", &source_arg, "--skill", SKILL_NAME];
    println!("running: npx {}", args.join(" "));

    let result = match Command::new("npx").args(args).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(io::Error::other(format!(
            "skill installer exited with status {status}"
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "npx not found. Install Node.js and rerun `hitch setup skill`",
        )),
        Err(err) => Err(err),
    };

    let _ = fs::remove_dir_all(source);
    result
}

fn write_embedded_skill_dir() -> io::Result<PathBuf> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let dir = env::temp_dir().join(format!("hitch-skill-{}-{now}", process::id()));
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("SKILL.md"), SKILL_MD)?;
    Ok(dir)
}

fn update_warning() -> Option<String> {
    if INSTALL_SOURCE != "npm" {
        return None;
    }

    let cache = read_update_cache()?;
    if cache.install_source != INSTALL_SOURCE {
        return None;
    }
    if !update_cache_fresh(&cache) {
        return None;
    }
    let latest = cache.latest_version?;
    if version_less_than(HITCH_VERSION, &latest) {
        Some(format!(
            "update available {HITCH_VERSION} -> {latest}, run `npm install -g {NPM_PACKAGE_NAME}`"
        ))
    } else {
        None
    }
}

fn maybe_refresh_update_cache() {
    if INSTALL_SOURCE != "npm" || !update_cache_stale() {
        return;
    }

    let path = update_cache_path();
    let _ = thread::Builder::new()
        .name("hitch-update-check".to_string())
        .spawn(move || {
            let _ = refresh_npm_update_cache(&path);
        });
}

fn update_cache_stale() -> bool {
    let Some(cache) = read_update_cache() else {
        return true;
    };
    if cache.install_source != INSTALL_SOURCE {
        return true;
    }
    !update_cache_fresh(&cache)
}

fn update_cache_fresh(cache: &UpdateCache) -> bool {
    let now = now_epoch();
    cache.checked_at <= now && now.saturating_sub(cache.checked_at) < UPDATE_CACHE_TTL_SECS
}

fn read_update_cache() -> Option<UpdateCache> {
    let raw = fs::read_to_string(update_cache_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

fn refresh_npm_update_cache(path: &Path) -> io::Result<()> {
    let latest = fetch_npm_latest_version();
    let cache = UpdateCache {
        checked_at: now_epoch(),
        install_source: INSTALL_SOURCE.to_string(),
        latest_version: latest,
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(&cache).unwrap())?;
    Ok(())
}

fn fetch_npm_latest_version() -> Option<String> {
    let response = minreq::get(NPM_REGISTRY_URL).with_timeout(3).send().ok()?;
    let raw = response.as_str().ok()?;
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let tag = if HITCH_VERSION.contains('-') {
        "beta"
    } else {
        "latest"
    };
    value
        .get("dist-tags")
        .and_then(|tags| tags.get(tag).or_else(|| tags.get("latest")))
        .and_then(|version| version.as_str())
        .map(str::to_string)
}

fn outdated_skill_warning() -> Option<String> {
    for path in installed_skill_paths() {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(installed) = skill_version(&raw) else {
            continue;
        };
        if version_less_than(&installed, SKILL_VERSION) {
            let root = format_skill_root(&path);
            return Some(format!(
                "agent skill in \"{root}\" is outdated, run `hitch setup` to update"
            ));
        }
    }
    None
}

fn installed_skill_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(cwd) = env::current_dir() {
        paths.push(cwd.join(".agents/skills/hitch/SKILL.md"));
        paths.push(cwd.join(".claude/skills/hitch/SKILL.md"));
    }

    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return paths;
    };

    paths.push(home.join(".agents/skills/hitch/SKILL.md"));
    paths.push(home.join(".claude/skills/hitch/SKILL.md"));
    paths
}

fn format_skill_root(path: &Path) -> String {
    let root = path
        .parent()
        .and_then(Path::parent)
        .unwrap_or(path)
        .to_path_buf();

    if let Ok(cwd) = env::current_dir() {
        if let Ok(relative) = root.strip_prefix(&cwd) {
            if relative.as_os_str().is_empty() {
                return ".".to_string();
            }
            return format!("./{}", relative.display());
        }
    }

    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        if let Ok(relative) = root.strip_prefix(home) {
            if relative.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", relative.display());
        }
    }

    root.display().to_string()
}

fn skill_version(raw: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let line = line.trim();
        let version = line.strip_prefix("version:")?.trim();
        if version.is_empty() {
            return None;
        }
        Some(version.trim_matches(['"', '\'']).to_string())
    })
}

fn version_less_than(left: &str, right: &str) -> bool {
    let left = parse_version(left);
    let right = parse_version(right);
    let len = left.len().max(right.len());
    for index in 0..len {
        let left_part = left.get(index).copied().unwrap_or(0);
        let right_part = right.get(index).copied().unwrap_or(0);
        if left_part != right_part {
            return left_part < right_part;
        }
    }
    false
}

fn parse_version(version: &str) -> Vec<u64> {
    version
        .split(['.', '-'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn cmd_print_skill() -> io::Result<()> {
    print!("{}", SKILL_MD);
    if !SKILL_MD.ends_with('\n') {
        println!();
    }
    Ok(())
}

fn cmd_setup(args: &SetupArgs) -> io::Result<()> {
    match args.command {
        Some(SetupCommand::Shell) => cmd_setup_prompt(),
        Some(SetupCommand::Skill) => cmd_install_skill(),
        None => cmd_setup_wizard(),
    }
}

fn cmd_setup_wizard() -> io::Result<()> {
    println!("hitch setup");
    println!();

    cmd_setup_prompt()?;
    println!();

    let install_skill = confirm("Install agent skill?", true)?;
    if install_skill {
        cmd_install_skill()?;
    } else {
        println!("agent skill skipped");
    }

    Ok(())
}

fn ensure_shell_integration() -> io::Result<()> {
    match shell_integration_state()? {
        ShellIntegrationState::Current => Ok(()),
        ShellIntegrationState::Outdated => {
            if env::var_os("HITCH_NO_AUTO_UPDATE_SHELL").is_none() {
                update_shell_integration_silent()?;
            }
            Ok(())
        }
        ShellIntegrationState::Missing => {
            println!("welcome to hitch");
            println!("running setup first");
            println!();
            cmd_setup_wizard()?;
            println!();
            println!("setup complete, run `hitch` again after restarting existing terminals");
            process::exit(0);
        }
    }
}

fn confirm(prompt: &str, default: bool) -> io::Result<bool> {
    Confirm::new(prompt)
        .with_default(default)
        .prompt()
        .map_err(io::Error::other)
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

enum ShellIntegrationState {
    Current,
    Outdated,
    Missing,
}

fn shell_integration_state() -> io::Result<ShellIntegrationState> {
    let Some((path, block)) = essential_shell_integration() else {
        return Ok(ShellIntegrationState::Current);
    };
    let raw = fs::read_to_string(path).unwrap_or_default();
    if !has_hitch_marked_block(&raw) {
        return Ok(ShellIntegrationState::Missing);
    }
    if upsert_marked_block(&raw, block) == raw {
        Ok(ShellIntegrationState::Current)
    } else {
        Ok(ShellIntegrationState::Outdated)
    }
}

fn update_shell_integration_silent() -> io::Result<()> {
    let Some((path, block)) = essential_shell_integration() else {
        return Ok(());
    };
    setup_rc_prompt(&path, block)
}

fn essential_shell_integration() -> Option<(PathBuf, &'static str)> {
    match detect_shell().as_str() {
        "zsh" => home_file(".zshrc").map(|path| (path, zsh_prompt_block())),
        "bash" => home_file(".bashrc").map(|path| (path, bash_prompt_block())),
        "fish" => {
            home_file(".config/fish/conf.d/hitch.fish").map(|path| (path, fish_prompt_block()))
        }
        _ => None,
    }
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

fn has_hitch_marked_block(raw: &str) -> bool {
    raw.contains("# >>> hitch shell integration >>>")
        || raw.contains("# >>> hitch prompt integration >>>")
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

function _hitch_run() {
    local _hitch_command="$1"
    shift
    local _hitch_bin="${commands[$_hitch_command]:-}"
    if [[ -z "${_hitch_bin:-}" ]]; then
      print -u2 "$_hitch_command: command not found"
      return 127
    fi
    if [[ -z "${HITCH_SESSION:-}" && ( "$#" -eq 0 || "$1" == "on" || "$1" == "start" ) ]]; then
      fc -W 2>/dev/null
    fi

    local _hitch_cwd_file=""
    if [[ -z "${HITCH_SESSION:-}" ]]; then
      _hitch_cwd_file="${TMPDIR:-/tmp}/hitch-cwd-$$-$RANDOM"
      HITCH_CWD_SYNC_FILE="$_hitch_cwd_file" "$_hitch_bin" "$@"
    else
      "$_hitch_bin" "$@"
    fi
    local code=$?
    if [[ -n "$_hitch_cwd_file" && -s "$_hitch_cwd_file" ]]; then
      local _hitch_cwd
      _hitch_cwd="$(cat "$_hitch_cwd_file" 2>/dev/null)"
      if [[ -d "$_hitch_cwd" ]]; then
        cd "$_hitch_cwd"
      fi
    fi
    [[ -n "$_hitch_cwd_file" ]] && rm -f "$_hitch_cwd_file"
    if [[ "$code" -eq 42 ]]; then
      exit
    fi
    return "$code"
}

function hitch() {
  _hitch_run hitch "$@"
}

alias unhitch='hitch off'

function hitch-dev() {
  _hitch_run hitch-dev "$@"
}
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

_hitch_run() {
    local _hitch_command="$1"
    shift
    local _hitch_bin
    _hitch_bin="$(type -P "$_hitch_command" 2>/dev/null || true)"
    if [[ -z "${_hitch_bin:-}" ]]; then
      printf '%s: command not found\n' "$_hitch_command" >&2
      return 127
    fi
    if [[ -z "${HITCH_SESSION:-}" && ( "$#" -eq 0 || "$1" == "on" || "$1" == "start" ) ]]; then
      history -a 2>/dev/null
    fi

    local _hitch_cwd_file=""
    if [[ -z "${HITCH_SESSION:-}" ]]; then
      _hitch_cwd_file="${TMPDIR:-/tmp}/hitch-cwd-$$-$RANDOM"
      HITCH_CWD_SYNC_FILE="$_hitch_cwd_file" "$_hitch_bin" "$@"
    else
      "$_hitch_bin" "$@"
    fi
    local code=$?
    if [[ -n "$_hitch_cwd_file" && -s "$_hitch_cwd_file" ]]; then
      local _hitch_cwd
      _hitch_cwd="$(cat "$_hitch_cwd_file" 2>/dev/null)"
      if [[ -d "$_hitch_cwd" ]]; then
        cd "$_hitch_cwd"
      fi
    fi
    [[ -n "$_hitch_cwd_file" ]] && rm -f "$_hitch_cwd_file"
    if [[ "$code" -eq 42 ]]; then
      exit
    fi
    return "$code"
}

hitch() {
  _hitch_run hitch "$@"
}

alias unhitch='hitch off'

hitch-dev() {
  _hitch_run hitch-dev "$@"
}
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

function __hitch_run
        set -l __hitch_command $argv[1]
        set -e argv[1]
        set -l __hitch_bin (command -s "$__hitch_command")
        if test -z "$__hitch_bin"
            printf '%s: command not found\n' "$__hitch_command" >&2
            return 127
        end
        if not set -q HITCH_SESSION
            if test (count $argv) -eq 0; or test "$argv[1]" = on; or test "$argv[1]" = start
                history save 2>/dev/null
            end
        end

        set -l __hitch_cwd_file
        if not set -q HITCH_SESSION
            set __hitch_cwd_file (mktemp -t hitch-cwd.XXXXXX 2>/dev/null)
            if test -n "$__hitch_cwd_file"
                env HITCH_CWD_SYNC_FILE="$__hitch_cwd_file" "$__hitch_bin" $argv
            else
                "$__hitch_bin" $argv
            end
        else
            "$__hitch_bin" $argv
        end
        set code $status
        if test -n "$__hitch_cwd_file"; and test -s "$__hitch_cwd_file"
            set -l __hitch_cwd (cat "$__hitch_cwd_file" 2>/dev/null)
            if test -d "$__hitch_cwd"
                cd "$__hitch_cwd"
            end
        end
        if test -n "$__hitch_cwd_file"
            rm -f "$__hitch_cwd_file"
        end
        if test $code -eq 42
            exit
        end
        return $code
end

function hitch
    __hitch_run hitch $argv
end

alias unhitch 'hitch off'

function hitch-dev
    __hitch_run hitch-dev $argv
end
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
    let Ok(bytes) = head_raw_bytes(path, CONTEXT_OUTPUT_WINDOW_BYTES) else {
        return Vec::new();
    };
    let text = render_terminal_text(&bytes);
    normalize_list_lines(text.lines().map(str::to_string).collect())
        .into_iter()
        .take(limit)
        .collect()
}

fn list_tail_lines(path: &str, limit: usize) -> Vec<String> {
    bounded_rendered_tail_lines(path, limit, true)
}

fn rendered_lines_from_offset(path: &str, offset: u64, limit: usize) -> Vec<String> {
    let Ok(bytes) = read_bytes_from(path, offset) else {
        return Vec::new();
    };
    let text = render_terminal_text(&bytes);
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

fn bounded_rendered_tail_lines(path: &str, limit: usize, normalize: bool) -> Vec<String> {
    let len = file_len(path);
    if len == 0 {
        return Vec::new();
    }

    let mut window = CONTEXT_OUTPUT_WINDOW_BYTES.min(CONTEXT_OUTPUT_MAX_BYTES);
    loop {
        let Ok(bytes) = tail_raw_bytes(path, window) else {
            return Vec::new();
        };
        let text = render_terminal_text(&bytes);
        let lines = if normalize {
            normalize_list_lines(text.lines().map(str::to_string).collect())
        } else {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_string)
                .collect()
        };

        if lines.len() >= limit || window >= len || window >= CONTEXT_OUTPUT_MAX_BYTES {
            let start = lines.len().saturating_sub(limit);
            return lines.into_iter().skip(start).collect();
        }

        window = (window * 2).min(CONTEXT_OUTPUT_MAX_BYTES);
    }
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
    bounded_rendered_tail_lines(path, limit, false)
}

fn rendered_lines_range(path: &str, start: isize, end: Option<isize>) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let text = render_terminal_text(text.as_bytes());
    lines_range(&text, start, end)
}

fn raw_lines_range(path: &str, start: isize, end: Option<isize>) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    lines_range(&text, start, end)
}

fn lines_range(text: &str, start: isize, end: Option<isize>) -> Vec<String> {
    let lines = text.lines().collect::<Vec<_>>();
    let start = line_index(lines.len(), start);
    let end = end
        .map(|end| line_index(lines.len(), end).saturating_add(1))
        .unwrap_or(lines.len())
        .min(lines.len());
    lines
        .into_iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(str::to_string)
        .collect()
}

fn line_index(len: usize, index: isize) -> usize {
    if index < 0 {
        len.saturating_sub(index.unsigned_abs())
    } else {
        (index as usize).min(len)
    }
}

fn raw_tail_lines(path: &str, limit: usize) -> Vec<String> {
    let Ok(bytes) = tail_raw_bytes(path, CONTEXT_OUTPUT_MAX_BYTES) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_shell_idle_titles() {
        assert!(is_shell_idle_title("maxktz@Maxs-MacBook-Pro:~/dev/testapp"));
        assert!(is_shell_idle_title("max.ktz@host.local:/tmp/app"));

        assert!(!is_shell_idle_title("build ready @ 12:30"));
        assert!(!is_shell_idle_title("server@example.com: listening"));
        assert!(!is_shell_idle_title("maxktz@:~/dev/testapp"));
        assert!(!is_shell_idle_title("maxktz@Maxs-MacBook-Pro"));
    }

    #[test]
    fn rewrites_idle_title_to_current_dir() {
        let mut filter = TitleFilter::rewrite("3");
        filter.set_cwd(Some("/tmp/testapp".to_string()));

        let output = filter.filter(b"\x1b]2;maxktz@Maxs-MacBook-Pro:~/dev/testapp\x07");

        assert_eq!(output, b"\x1b]2;#3 /tmp/testapp\x07");
    }

    #[test]
    fn preserves_non_idle_titles() {
        let mut filter = TitleFilter::rewrite("3");
        filter.set_cwd(Some("/tmp/testapp".to_string()));

        let output = filter.filter(b"\x1b]2;server@example.com: listening\x07");

        assert_eq!(output, b"\x1b]2;#3 server@example.com: listening\x07");
    }

    #[test]
    fn context_lines_at_limit_are_unchanged() {
        let line = "a".repeat(CONTEXT_LINE_MAX_CHARS);

        assert_eq!(truncate_context_line(&line), Cow::Borrowed(line.as_str()));
    }

    #[test]
    fn context_lines_are_truncated_by_characters() {
        let line = format!("{}é漢", "a".repeat(CONTEXT_LINE_MAX_CHARS));
        let expected = format!(
            "{}... [truncated 2 chars]",
            "a".repeat(CONTEXT_LINE_MAX_CHARS)
        );

        assert_eq!(truncate_context_line(&line), Cow::Owned::<str>(expected));
    }
}
