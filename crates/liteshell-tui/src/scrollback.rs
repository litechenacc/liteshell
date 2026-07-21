use liteshell_core::{StyledSpan, StyledText};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub spans: Vec<StyledSpan>,
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
        self.push_styled(&StyledText::new(vec![StyledSpan::plain(text)]), error);
    }

    pub fn push_styled(&mut self, styled: &StyledText, error: bool) {
        let mut line_spans = Vec::new();
        let mut line_text = String::new();
        for span in &styled.spans {
            for part in span.text.split_inclusive('\n') {
                let ends_line = part.ends_with('\n');
                let content = part.trim_end_matches(['\r', '\n']);
                if !content.is_empty() {
                    line_text.push_str(content);
                    line_spans.push(StyledSpan::new(content, span.style));
                }
                if ends_line {
                    self.push_line(
                        std::mem::take(&mut line_text),
                        std::mem::take(&mut line_spans),
                        error,
                    );
                }
            }
        }
        if !line_text.is_empty() || !line_spans.is_empty() {
            self.push_line(line_text, line_spans, error);
        }
        self.enforce_limits();
    }

    fn push_line(&mut self, text: String, spans: Vec<StyledSpan>, error: bool) {
        self.bytes += text.len();
        self.lines.push_back(Line {
            text,
            spans,
            error,
            divider: false,
        });
    }

    pub fn push_divider(&mut self) {
        self.lines.push_back(Line {
            text: String::new(),
            spans: Vec::new(),
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

    /// Move toward the oldest content without allowing the visible window to
    /// collapse into a partially empty page. `trailing_lines` accounts for the
    /// active two-line prompt, which is rendered as part of the transcript.
    /// Returns true when the viewport reaches the oldest available page.
    pub fn scroll_up(
        &mut self,
        amount: usize,
        visible_lines: usize,
        trailing_lines: usize,
    ) -> bool {
        let total = self.lines.len().saturating_add(trailing_lines);
        let maximum = total.saturating_sub(visible_lines);
        self.offset = self.offset.saturating_add(amount).min(maximum);
        self.offset == maximum
    }

    /// Move toward the newest content. Returns true when the live prompt is in
    /// view at the bottom of the transcript.
    pub fn scroll_down(
        &mut self,
        amount: usize,
        visible_lines: usize,
        trailing_lines: usize,
    ) -> bool {
        let total = self.lines.len().saturating_add(trailing_lines);
        let maximum = total.saturating_sub(visible_lines);
        self.offset = self.offset.min(maximum).saturating_sub(amount);
        self.offset == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use liteshell_core::{SemanticColor, TextStyle};

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

    #[test]
    fn styled_spans_survive_line_splitting() {
        let mut scrollback = Scrollback::new(10, 100);
        scrollback.push_styled(
            &StyledText::new(vec![StyledSpan::new(
                "blue\nnext",
                TextStyle::foreground(SemanticColor::Directory),
            )]),
            false,
        );
        let lines: Vec<_> = scrollback.lines().collect();
        assert_eq!(lines[0].text, "blue");
        assert_eq!(lines[1].spans[0].style.foreground, SemanticColor::Directory);
    }

    #[test]
    fn scrolling_to_top_keeps_a_full_viewport() {
        let mut scrollback = Scrollback::new(100, 4096);
        for index in 0..10 {
            scrollback.push_text(&format!("line {index}\n"), false);
        }

        assert!(scrollback.scroll_up(usize::MAX, 5, 2));
        assert_eq!(scrollback.offset, 7);
        assert!(scrollback.scroll_down(usize::MAX, 5, 2));
        assert_eq!(scrollback.offset, 0);
    }
}
