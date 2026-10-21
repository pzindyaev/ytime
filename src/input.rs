use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single-line editable text field with a byte-offset cursor that stays on
/// UTF-8 char boundaries.
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub value: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub fn parts(&self) -> (&str, &str, &str) {
        let before = &self.value[..self.cursor];
        let at = self.value[self.cursor..].chars().next();
        let after = &self.value[self.cursor + at.map_or(0, char::len_utf8)..];
        (
            before,
            at.map_or(" ", |c| {
                &self.value[self.cursor..self.cursor + c.len_utf8()]
            }),
            after,
        )
    }

    fn clamp_cursor(&mut self) {
        if self.cursor > self.value.len() {
            self.cursor = self.value.len();
        }
        while self.cursor > 0 && !self.value.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    fn left(&mut self) {
        self.cursor = self.value[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
    }

    fn right(&mut self) {
        if let Some(c) = self.value[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Char(c), m) if m.contains(KeyModifiers::CONTROL) => match c {
                'u' => self.clear(),
                'a' => self.cursor = 0,
                'e' => self.cursor = self.value.len(),
                _ => {}
            },
            (KeyCode::Char(c), _) => {
                self.value.insert(self.cursor, c);
                self.right();
            }
            (KeyCode::Backspace, _) => {
                if self.cursor > 0 {
                    self.left();
                    let c = self.value[self.cursor..].chars().next().unwrap();
                    let end = self.cursor + c.len_utf8();
                    self.value.replace_range(self.cursor..end, "");
                }
            }
            (KeyCode::Delete, _) => {
                if let Some(c) = self.value[self.cursor..].chars().next() {
                    let end = self.cursor + c.len_utf8();
                    self.value.replace_range(self.cursor..end, "");
                }
            }
            (KeyCode::Left, _) => self.left(),
            (KeyCode::Right, _) => self.right(),
            (KeyCode::Home, _) => self.cursor = 0,
            (KeyCode::End, _) => self.cursor = self.value.len(),
            _ => {}
        }
        self.clamp_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_and_editing() {
        let mut t = TextInput::new("");
        for c in "héllo".chars() {
            t.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(t.value, "héllo");
        t.handle_key(key(KeyCode::Backspace));
        assert_eq!(t.value, "héll");
        t.handle_key(key(KeyCode::Left));
        t.handle_key(key(KeyCode::Left));
        t.handle_key(key(KeyCode::Char('x')));
        assert_eq!(t.value, "héxll");
        t.handle_key(key(KeyCode::Delete));
        assert_eq!(t.value, "héxl");
        t.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(t.value, "");
    }

    #[test]
    fn parts_split_on_char_boundary() {
        let mut t = TextInput::new("日本語");
        t.handle_key(key(KeyCode::Left));
        let (before, _at, after) = t.parts();
        assert_eq!(before, "日本");
        assert_eq!(after, "");
    }
}
