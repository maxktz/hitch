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

pub(crate) fn render_terminal_text(bytes: &[u8]) -> String {
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
    fn carriage_return_overwrites_current_line() {
        assert_eq!(
            render_terminal_text(b"progress 10%\rprogress 20%"),
            "progress 20%"
        );
    }

    #[test]
    fn backspace_replaces_previous_character() {
        assert_eq!(render_terminal_text(b"abc\x08d"), "abd");
    }

    #[test]
    fn skips_osc_title_sequences() {
        assert_eq!(
            render_terminal_text(b"before\x1b]2;ignored title\x07after"),
            "beforeafter"
        );
    }

    #[test]
    fn handles_csi_cursor_movement_and_line_clear() {
        assert_eq!(render_terminal_text(b"hello\x1b[2DXY\x1b[K"), "helXY");
    }

    #[test]
    fn keeps_multibyte_characters_intact() {
        assert_eq!(render_terminal_text("hi é漢".as_bytes()), "hi é漢");
    }
}
