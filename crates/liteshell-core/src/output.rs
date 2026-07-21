#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SemanticColor {
    #[default]
    Default,
    Directory,
    Executable,
    Symlink,
    Metadata,
    Heading,
    Command,
    Option,
    Path,
    String,
    Number,
    Keyword,
    Comment,
    Punctuation,
    Added,
    Removed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextStyle {
    pub foreground: SemanticColor,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
}

impl TextStyle {
    pub const fn foreground(foreground: SemanticColor) -> Self {
        Self {
            foreground,
            bold: false,
            dim: false,
            italic: false,
        }
    }

    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyledSpan {
    pub text: String,
    pub style: TextStyle,
}

impl StyledSpan {
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(text, TextStyle::default())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyledLine {
    pub spans: Vec<StyledSpan>,
}

impl StyledLine {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            spans: vec![StyledSpan::plain(text)],
        }
    }

    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyledText {
    pub spans: Vec<StyledSpan>,
}

impl StyledText {
    pub fn new(spans: Vec<StyledSpan>) -> Self {
        Self { spans }
    }

    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutputEvent {
    Text(String),
    Styled(StyledText),
    Error(String),
    Clear,
    Pager {
        title: String,
        lines: Vec<StyledLine>,
    },
    Status(String),
}

pub trait OutputSink {
    fn emit(&mut self, event: OutputEvent);

    /// Write raw standard-output bytes when the sink supports byte streams.
    /// TUI sinks retain their text-only contract through this default method;
    /// command-mode sinks override it to preserve binary pipeline data.
    fn write_stdout(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        self.emit(OutputEvent::Text(text.to_owned()));
        Ok(())
    }
}

#[derive(Default)]
pub struct VecOutput(pub Vec<OutputEvent>);
impl OutputSink for VecOutput {
    fn emit(&mut self, event: OutputEvent) {
        self.0.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styled_values_flatten_without_control_sequences() {
        let text = StyledText::new(vec![
            StyledSpan::new("hello", TextStyle::foreground(SemanticColor::Heading)),
            StyledSpan::plain(" world\n"),
        ]);
        assert_eq!(text.text(), "hello world\n");
        assert_eq!(StyledLine { spans: text.spans }.text(), "hello world\n");
    }
}
