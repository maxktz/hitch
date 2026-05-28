use crate::shorten_home;

pub(crate) struct TitleFilter {
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
pub(crate) struct AltScreenLogFilter {
    pub(crate) alt_screen: bool,
    state: u8,
    seq: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct AltScreenTracker {
    pub(crate) alt_screen: bool,
    state: u8,
    seq: Vec<u8>,
}

impl TitleFilter {
    pub(crate) fn strip() -> Self {
        Self {
            state: 0,
            code: 0,
            title: Vec::new(),
            mode: TitleFilterMode::Strip,
            cwd: None,
        }
    }

    pub(crate) fn rewrite(session_id: &str) -> Self {
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

    pub(crate) fn set_cwd(&mut self, cwd: Option<String>) {
        self.cwd = cwd;
    }

    pub(crate) fn filter(&mut self, input: &[u8]) -> Vec<u8> {
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
    pub(crate) fn filter(&mut self, input: &[u8]) -> Vec<u8> {
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
    pub(crate) fn observe(&mut self, input: &[u8]) {
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
    pub(crate) fn rewrites_idle_title_to_current_dir() {
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
}
