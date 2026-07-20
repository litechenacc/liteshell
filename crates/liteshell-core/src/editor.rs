use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
}
impl Editor {
    pub fn insert(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }
    pub fn previous(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
    pub fn next(&self) -> usize {
        self.text[self.cursor..]
            .grapheme_indices(true)
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.text.len())
    }
    pub fn left(&mut self) {
        self.cursor = self.previous();
    }
    pub fn right(&mut self) {
        self.cursor = self.next();
    }
    pub fn backspace(&mut self) {
        let p = self.previous();
        self.text.drain(p..self.cursor);
        self.cursor = p;
    }
    pub fn delete(&mut self) {
        let n = self.next();
        self.text.drain(self.cursor..n);
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
    pub fn set(&mut self, s: String) {
        self.text = s;
        self.cursor = self.text.len();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grapheme_edit() {
        let mut e = Editor {
            text: "a👩‍💻e\u{301}".into(),
            cursor: "a👩‍💻e\u{301}".len(),
        };
        e.left();
        e.backspace();
        assert_eq!(e.text, "ae\u{301}");
    }
}
