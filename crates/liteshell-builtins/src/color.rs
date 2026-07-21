use liteshell_core::output::{
    SemanticColor as Color, StyledLine, StyledSpan, StyledText, TextStyle,
};
use std::path::Path;

pub fn span(text: impl Into<String>, color: Color) -> StyledSpan {
    StyledSpan::new(text, TextStyle::foreground(color))
}

pub fn strong(text: impl Into<String>, color: Color) -> StyledSpan {
    StyledSpan::new(text, TextStyle::foreground(color).bold())
}

pub fn highlight_text(path: &Path, text: &str) -> StyledText {
    let mut spans = Vec::new();
    for line in text.split_inclusive('\n') {
        spans.extend(highlight_line(path, line));
    }
    StyledText::new(spans)
}

pub fn highlight_lines<'a>(
    path: &Path,
    lines: impl IntoIterator<Item = &'a str>,
) -> Vec<StyledLine> {
    lines
        .into_iter()
        .map(|line| StyledLine {
            spans: highlight_line(path, line),
        })
        .collect()
}

pub fn file_color(path: &Path, metadata: &std::fs::Metadata) -> Color {
    if metadata.file_type().is_symlink() {
        Color::Symlink
    } else if metadata.is_dir() {
        Color::Directory
    } else if is_executable(path) {
        Color::Executable
    } else {
        Color::Path
    }
}

fn is_executable(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "com" | "cmd" | "bat" | "ps1" | "msi"
            )
        })
}

fn highlight_line(path: &Path, line: &str) -> Vec<StyledSpan> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let body = line.strip_suffix('\n').unwrap_or(line);
    let body = body.strip_suffix('\r').unwrap_or(body);
    let newline = &line[body.len()..];

    if matches!(extension.as_str(), "md" | "markdown") && body.starts_with('#') {
        return with_newline(vec![strong(body, Color::Heading)], newline);
    }
    if matches!(extension.as_str(), "diff" | "patch") {
        let color = if body.starts_with('+') && !body.starts_with("+++") {
            Some(Color::Added)
        } else if body.starts_with('-') && !body.starts_with("---") {
            Some(Color::Removed)
        } else {
            None
        };
        if let Some(color) = color {
            return with_newline(vec![span(body, color)], newline);
        }
    }

    let hash_comments = matches!(
        extension.as_str(),
        "py" | "pyw" | "ps1" | "sh" | "bash" | "zsh" | "toml" | "yaml" | "yml"
    );
    let semicolon_comments = matches!(extension.as_str(), "ini" | "cfg");
    let mut result = Vec::new();
    let mut index = 0;
    while index < body.len() {
        let rest = &body[index..];
        if rest.starts_with("//")
            || (hash_comments && rest.starts_with('#'))
            || (semicolon_comments && rest.starts_with(';'))
        {
            result.push(StyledSpan::new(
                rest,
                TextStyle::foreground(Color::Comment).dim(),
            ));
            break;
        }

        let character = rest.chars().next().expect("non-empty remainder");
        if character == '\'' || character == '"' || character == '`' {
            let end = quoted_end(rest, character);
            result.push(span(&rest[..end], Color::String));
            index += end;
        } else if character.is_ascii_digit() {
            let end = take_while(rest, |value| {
                value.is_ascii_alphanumeric() || matches!(value, '.' | '_' | '+' | '-')
            });
            result.push(span(&rest[..end], Color::Number));
            index += end;
        } else if character.is_alphabetic() || character == '_' {
            let end = take_while(rest, |value| value.is_alphanumeric() || value == '_');
            let word = &rest[..end];
            if is_keyword(word) {
                result.push(strong(word, Color::Keyword));
            } else if starts_uppercase(word) {
                result.push(span(word, Color::Command));
            } else {
                result.push(StyledSpan::plain(word));
            }
            index += end;
        } else {
            let length = character.len_utf8();
            let color = if "{}[](),.:;=<>+-*/|&!?".contains(character) {
                Color::Punctuation
            } else {
                Color::Default
            };
            result.push(span(&rest[..length], color));
            index += length;
        }
    }
    with_newline(result, newline)
}

fn with_newline(mut spans: Vec<StyledSpan>, newline: &str) -> Vec<StyledSpan> {
    if !newline.is_empty() {
        spans.push(StyledSpan::plain(newline));
    }
    spans
}

fn quoted_end(value: &str, quote: char) -> usize {
    let mut escaped = false;
    for (index, character) in value.char_indices().skip(1) {
        if character == quote && !escaped {
            return index + character.len_utf8();
        }
        escaped = character == '\\' && !escaped;
        if character != '\\' {
            escaped = false;
        }
    }
    value.len()
}

fn take_while(value: &str, predicate: impl Fn(char) -> bool) -> usize {
    value
        .char_indices()
        .take_while(|(_, character)| predicate(*character))
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0)
}

fn starts_uppercase(value: &str) -> bool {
    value.chars().next().is_some_and(char::is_uppercase)
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "async"
            | "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "crate"
            | "def"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "fn"
            | "for"
            | "from"
            | "function"
            | "if"
            | "impl"
            | "import"
            | "in"
            | "interface"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "new"
            | "None"
            | "null"
            | "pub"
            | "raise"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "Some"
            | "static"
            | "struct"
            | "super"
            | "switch"
            | "throw"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "use"
            | "var"
            | "where"
            | "while"
            | "with"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlighting_preserves_text_and_marks_tokens() {
        let source = "fn main() { // hello\n  let n = 42;\n}\n";
        let styled = highlight_text(Path::new("main.rs"), source);
        assert_eq!(styled.text(), source);
        assert!(styled
            .spans
            .iter()
            .any(|item| item.style.foreground == Color::Keyword));
        assert!(styled
            .spans
            .iter()
            .any(|item| item.style.foreground == Color::Comment));
        assert!(styled
            .spans
            .iter()
            .any(|item| item.style.foreground == Color::Number));
    }

    #[test]
    fn files_are_classified_for_ls() {
        let directory = std::fs::metadata(std::env::current_dir().unwrap()).unwrap();
        assert_eq!(
            file_color(Path::new("folder"), &directory),
            Color::Directory
        );

        let file = std::fs::metadata(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(file_color(Path::new("tool.exe"), &file), Color::Executable);
    }
}
