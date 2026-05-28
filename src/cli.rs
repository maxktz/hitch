use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "hitch",
    version,
    after_help = "Agents: run `hitch context` before starting dev servers, watchers, tunnels, REPLs, or log tails. Use `capture` to inspect output and `send-keys` to interact."
)]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) skill: bool,
    #[arg(long, hide = true)]
    pub(crate) welcome: bool,
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
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
pub(crate) struct SetupArgs {
    #[command(subcommand)]
    pub(crate) command: Option<SetupCommand>,
}

#[derive(Subcommand)]
pub(crate) enum SetupCommand {
    /// Install shell integration.
    Shell,
    /// Install the optional agent skill.
    Skill,
}

#[derive(Args)]
pub(crate) struct ContextArgs {
    #[arg(value_name = "TERMINAL")]
    pub(crate) terminal: Option<String>,
    #[arg(long)]
    pub(crate) all: bool,
    #[arg(long)]
    pub(crate) dir: Option<PathBuf>,
    #[arg(long)]
    pub(crate) head: Option<usize>,
    #[arg(long)]
    pub(crate) tail: Option<usize>,
    #[arg(long)]
    pub(crate) no_output: bool,
}

#[derive(Args)]
pub(crate) struct SessionArg {
    #[arg(value_name = "TERMINAL")]
    pub(crate) session: String,
}

#[derive(Args)]
pub(crate) struct SendKeysArgs {
    #[arg(short = 't', long = "target", help = "Terminal id")]
    pub(crate) target: String,
    #[arg(
        long,
        help = "Wait mode: output, finish, quiet:<duration>, or time:<duration>"
    )]
    pub(crate) wait: Option<String>,
    #[arg(long, help = "Maximum wait duration. Supports ms, s, or m")]
    pub(crate) timeout: Option<String>,
    #[arg(long, help = "Print this many new visible output lines after sending")]
    pub(crate) tail: Option<usize>,
    #[arg(long, help = "Send input even when a process is running")]
    pub(crate) force: bool,
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Keys or text to send. Options must come before this"
    )]
    pub(crate) keys: Vec<String>,
}

#[derive(Args)]
pub(crate) struct CapturePaneArgs {
    #[arg(short = 't', long = "target")]
    pub(crate) target: String,
    #[arg(short = 'p')]
    pub(crate) print: bool,
    #[arg(short = 'S', allow_hyphen_values = true)]
    pub(crate) start: Option<isize>,
    #[arg(short = 'E', allow_hyphen_values = true)]
    pub(crate) end: Option<isize>,
    #[arg(short = 'e')]
    pub(crate) escapes: bool,
    #[arg(short = 'C', hide = true)]
    pub(crate) _escape_non_printable: bool,
    #[arg(short = 'J', hide = true)]
    pub(crate) _join_wrapped: bool,
    #[arg(short = 'N', hide = true)]
    pub(crate) _preserve_trailing_spaces: bool,
    #[arg(short = 'T', hide = true)]
    pub(crate) _trim_trailing_empty: bool,
    #[arg(short = 'a', hide = true)]
    pub(crate) _alternate_screen: bool,
    #[arg(short = 'q', hide = true)]
    pub(crate) _quiet: bool,
    #[arg(long, hide = true)]
    pub(crate) tail: Option<usize>,
    #[arg(long)]
    pub(crate) raw: bool,
}

#[derive(Args)]
pub(crate) struct StatusArgs {
    #[arg(long)]
    pub(crate) debug: bool,
}
