use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub type Aliases = HashMap<String, String>;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseError {
    #[error("unsupported shell operator")]
    UnsupportedOperator,
    #[error("unclosed quote")]
    UnclosedQuote,
    #[error("unclosed environment variable")]
    UnclosedVariable,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Quote {
    None,
    Single,
    Double,
}

pub fn parse(line: &str) -> Result<Vec<String>, ParseError> {
    let chars: Vec<char> = line.chars().collect();
    let mut args = Vec::new();
    let mut token = String::new();
    let mut started = false;
    let mut quote = Quote::None;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if quote == Quote::None && ch.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut token));
                started = false;
            }
            i += 1;
            continue;
        }
        if quote == Quote::None && "|><;&".contains(ch) {
            return Err(ParseError::UnsupportedOperator);
        }
        if ch == '\'' && quote != Quote::Double {
            quote = if quote == Quote::Single {
                Quote::None
            } else {
                Quote::Single
            };
            started = true;
            i += 1;
            continue;
        }
        if ch == '"' && quote != Quote::Single {
            quote = if quote == Quote::Double {
                Quote::None
            } else {
                Quote::Double
            };
            started = true;
            i += 1;
            continue;
        }
        if ch == '\\'
            && i + 1 < chars.len()
            && ((quote == Quote::Double && chars[i + 1] == '"')
                || (quote == Quote::None && matches!(chars[i + 1], '\'' | '"')))
        {
            token.push(chars[i + 1]);
            started = true;
            i += 2;
            continue;
        }
        if quote != Quote::Single && ch == '$' && i + 1 < chars.len() {
            if chars[i + 1] == '{' {
                let end = chars[i + 2..]
                    .iter()
                    .position(|c| *c == '}')
                    .map(|n| i + 2 + n)
                    .ok_or(ParseError::UnclosedVariable)?;
                token.push_str(&env(&chars[i + 2..end].iter().collect::<String>()));
                started = true;
                i = end + 1;
                continue;
            }
            if chars[i + 1] == '_' || chars[i + 1].is_alphabetic() {
                let mut end = i + 2;
                while end < chars.len() && (chars[end] == '_' || chars[end].is_alphanumeric()) {
                    end += 1;
                }
                token.push_str(&env(&chars[i + 1..end].iter().collect::<String>()));
                started = true;
                i = end;
                continue;
            }
        }
        if quote != Quote::Single && ch == '%' {
            if let Some(n) = chars[i + 1..].iter().position(|c| *c == '%') {
                let end = i + 1 + n;
                if end > i + 1 {
                    token.push_str(&env(&chars[i + 1..end].iter().collect::<String>()));
                    started = true;
                    i = end + 1;
                    continue;
                }
            }
        }
        token.push(ch);
        started = true;
        i += 1;
    }
    if quote != Quote::None {
        return Err(ParseError::UnclosedQuote);
    }
    if started {
        args.push(token);
    }
    for arg in &mut args {
        *arg = expand_home(arg);
    }
    Ok(args)
}

/// Parse a command line and expand aliases in command position. Alias values
/// are parsed with the same quoting and environment expansion rules as normal
/// input, while arguments supplied by the caller are appended unchanged.
pub fn parse_with_aliases(line: &str, aliases: &Aliases) -> Result<Vec<String>, ParseError> {
    let mut args = parse(line)?;
    let mut expanded = HashSet::new();

    while let Some(command) = args.first() {
        let Some(value) = aliases.get(command) else {
            break;
        };
        if !expanded.insert(command.clone()) {
            break;
        }

        let mut replacement = parse(value)?;
        if replacement.is_empty() {
            break;
        }
        replacement.extend(args.drain(1..));
        args = replacement;
    }

    Ok(args)
}

fn env(name: &str) -> String {
    std::env::var(name)
        .or_else(|_| {
            if name.eq_ignore_ascii_case("HOME") {
                std::env::var("USERPROFILE")
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .unwrap_or_default()
}
fn expand_home(value: &str) -> String {
    if value == "~" {
        return env("HOME");
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return std::path::Path::new(&env("HOME"))
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_and_empty() {
        assert_eq!(parse("a 'b c' \"\"").unwrap(), ["a", "b c", ""]);
    }
    #[test]
    fn rejects_operators() {
        assert_eq!(parse("a | b"), Err(ParseError::UnsupportedOperator));
    }
    #[test]
    fn windows_backslashes_survive() {
        assert_eq!(parse(r#"cd C:\work\x"#).unwrap()[1], r#"C:\work\x"#);
    }
    #[test]
    fn aliases_expand_recursively_and_keep_user_arguments() {
        let aliases = Aliases::from([("l".into(), "ll".into()), ("ll".into(), "ls -la".into())]);
        assert_eq!(
            parse_with_aliases("l src", &aliases).unwrap(),
            ["ls", "-la", "src"]
        );
    }
    #[test]
    fn self_referencing_aliases_expand_only_once() {
        let aliases = Aliases::from([("ls".into(), "ls -a".into())]);
        assert_eq!(
            parse_with_aliases("ls docs", &aliases).unwrap(),
            ["ls", "-a", "docs"]
        );
    }
}
