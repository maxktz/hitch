use std::env;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const LIGHT_YELLOW: &str = "\x1b[93m";
const RED: &str = "\x1b[31m";

pub(crate) struct Style {
    enabled: bool,
}

impl Style {
    pub(crate) fn plain() -> Self {
        Self { enabled: false }
    }

    pub(crate) fn stdout() -> Self {
        Self {
            enabled: env::var_os("NO_COLOR").is_none()
                && unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1,
        }
    }

    pub(crate) fn stderr() -> Self {
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

    pub(crate) fn brand(&self) -> String {
        self.paint("hitch", &[GREEN])
    }

    pub(crate) fn logo(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[YELLOW])
    }

    pub(crate) fn success(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[GREEN])
    }

    pub(crate) fn light_yellow(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[LIGHT_YELLOW])
    }

    pub(crate) fn id(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[GREEN])
    }

    pub(crate) fn session_id(&self, value: impl AsRef<str>) -> String {
        self.id(format!("#{}", value.as_ref()))
    }

    pub(crate) fn path(&self, value: impl AsRef<str>) -> String {
        value.as_ref().to_string()
    }

    pub(crate) fn command(&self, value: impl AsRef<str>) -> String {
        value.as_ref().to_string()
    }

    pub(crate) fn muted(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[DIM])
    }

    pub(crate) fn error(&self, value: impl AsRef<str>) -> String {
        self.paint(value, &[RED, BOLD])
    }
}
