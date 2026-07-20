use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub error: bool,
    pub divider: bool,
}

pub struct Scrollback {
    lines: VecDeque<Line>,
    max_lines: usize,
    max_bytes: usize,
    bytes: usize,
    pub offset: usize,
}

impl Scrollback {
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            max_lines,
            max_bytes,
            bytes: 0,
            offset: 0,
        }
    }

    pub fn push_text(&mut self, text: &str, error: bool) {
        for line in text.split_inclusive('\n') {
            let text = line.trim_end_matches(['\r', '\n']).to_owned();
            self.bytes += text.len();
            self.lines.push_back(Line {
                text,
                error,
                divider: false,
            });
        }
        self.enforce_limits();
    }

    pub fn push_divider(&mut self) {
        self.lines.push_back(Line {
            text: String::new(),
            error: false,
            divider: true,
        });
        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        while self.lines.len() > self.max_lines || self.bytes > self.max_bytes {
            if let Some(line) = self.lines.pop_front() {
                self.bytes = self.bytes.saturating_sub(line.text.len());
            }
        }
        self.offset = 0;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.bytes = 0;
        self.offset = 0;
    }

    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &Line> {
        self.lines.iter()
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.offset = (self.offset + amount).min(self.lines.len().saturating_sub(1));
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.offset = self.offset.saturating_sub(amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_by_lines() {
        let mut scrollback = Scrollback::new(2, 100);
        scrollback.push_text("a\nb\nc\n", false);
        assert_eq!(scrollback.lines().count(), 2);
    }

    #[test]
    fn divider_is_semantic() {
        let mut scrollback = Scrollback::new(10, 100);
        scrollback.push_divider();
        assert!(scrollback.lines().next().unwrap().divider);
    }
}
