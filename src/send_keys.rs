use crate::cli::SendKeysArgs;
use crate::models::SessionRecord;
use crate::{
    MSG_PUSH, file_len, find_session, print_context_session_to_stderr, read_session_state,
    rendered_lines_from_offset, send_packet,
};
use std::io;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
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

pub(crate) fn cmd_send_keys(args: &SendKeysArgs) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(keys: &[&str]) -> SendKeysArgs {
        SendKeysArgs {
            target: "1".to_string(),
            wait: None,
            timeout: None,
            tail: None,
            force: false,
            keys: keys.iter().map(|key| key.to_string()).collect(),
        }
    }

    #[test]
    fn key_to_bytes_maps_named_keys_and_control_keys() {
        assert_eq!(key_to_bytes("Enter"), b"\n");
        assert_eq!(key_to_bytes("Tab"), b"\t");
        assert_eq!(key_to_bytes("Escape"), &[0x1b]);
        assert_eq!(key_to_bytes("Backspace"), &[0x7f]);
        assert_eq!(key_to_bytes("C-c"), &[0x03]);
        assert_eq!(key_to_bytes("hello"), b"hello");
    }

    #[test]
    fn detects_control_only_sequences() {
        assert!(control_only_keys(&["C-c".to_string(), "C-l".to_string()]));
        assert!(!control_only_keys(&[]));
        assert!(!control_only_keys(&[
            "C-c".to_string(),
            "Enter".to_string()
        ]));
    }

    #[test]
    fn validates_options_come_before_key_arguments() {
        assert!(validate_send_keys_args(&args(&["echo", "ok"])).is_ok());

        let err = validate_send_keys_args(&args(&["echo", "--wait=finish"]))
            .expect_err("late option should be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            err.to_string(),
            "send-keys options must come before key arguments"
        );
    }

    #[test]
    fn parses_wait_modes() {
        assert!(matches!(
            parse_wait_mode("output").unwrap(),
            WaitMode::Output
        ));
        assert!(matches!(
            parse_wait_mode("finish").unwrap(),
            WaitMode::Finish
        ));
        assert!(matches!(
            parse_wait_mode("quiet:250ms").unwrap(),
            WaitMode::Quiet(duration) if duration == Duration::from_millis(250)
        ));
        assert!(matches!(
            parse_wait_mode("time:2s").unwrap(),
            WaitMode::Time(duration) if duration == Duration::from_secs(2)
        ));
    }

    #[test]
    fn rejects_invalid_wait_modes_and_durations() {
        assert_eq!(
            parse_wait_mode("quiet:abc").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            parse_wait_mode("later").unwrap_err().to_string(),
            "wait must be output, finish, quiet:<duration>, or time:<duration>"
        );
        assert_eq!(
            parse_duration("10").unwrap_err().to_string(),
            "duration must end with ms, s, or m"
        );
    }

    #[test]
    fn formats_durations_for_user_messages() {
        assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
        assert_eq!(format_duration(Duration::from_secs(2)), "2s");
        assert_eq!(format_duration(Duration::from_secs(120)), "2m");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1500ms");
    }

    #[test]
    fn wait_timeout_messages_include_mode_context() {
        assert_eq!(
            wait_timeout_message(&WaitMode::Output, Duration::from_secs(5)),
            "timed out after 5s waiting for output"
        );
        assert_eq!(
            wait_timeout_message(
                &WaitMode::Quiet(Duration::from_secs(1)),
                Duration::from_secs(5)
            ),
            "timed out after 5s waiting for output to become quiet"
        );
        assert_eq!(
            wait_timeout_message(
                &WaitMode::Time(Duration::from_secs(2)),
                Duration::from_secs(5)
            ),
            "timed out after 5s waiting for 2s"
        );
        assert_eq!(
            wait_timeout_message(&WaitMode::Finish, Duration::from_secs(5)),
            "timed out after 5s waiting for command to finish"
        );
    }
}
