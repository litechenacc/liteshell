use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub type Aliases = HashMap<String, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Connector {
    Always,
    And,
    Or,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLine {
    pub pipelines: Vec<(Connector, Pipeline)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pipeline {
    pub commands: Vec<Command>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    pub args: Vec<String>,
    pub redirections: Vec<Redirection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Redirection {
    Input(String),
    Output { path: String, append: bool },
    Error { path: String, append: bool },
    ErrorToOutput,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseError {
    #[error("unsupported shell operator")]
    UnsupportedOperator,
    #[error("background jobs are not supported")]
    UnsupportedBackground,
    #[error("unclosed quote")]
    UnclosedQuote,
    #[error("unclosed environment variable")]
    UnclosedVariable,
    #[error("expected a command")]
    MissingCommand,
    #[error("expected a redirection target")]
    MissingRedirectionTarget,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Quote {
    None,
    Single,
    Double,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    Word(String),
    Pipe,
    And,
    Or,
    Semi,
    Input,
    Output,
    Append,
    Error,
    ErrorAppend,
    ErrorToOutput,
}

/// Parse a single command. Compound operators and redirections are rejected so
/// existing editor/completion call sites can keep using an argv-only API.
pub fn parse(line: &str) -> Result<Vec<String>, ParseError> {
    let tokens = lex(line)?;
    if tokens.iter().any(|token| !matches!(token, Token::Word(_))) {
        return Err(ParseError::UnsupportedOperator);
    }
    Ok(tokens
        .into_iter()
        .map(|token| match token {
            Token::Word(word) => word,
            _ => unreachable!(),
        })
        .collect())
}

/// Parse a command line into pipelines and conditional command lists.
pub fn parse_command_line_with_aliases(
    line: &str,
    aliases: &Aliases,
) -> Result<CommandLine, ParseError> {
    let tokens = lex(line)?;
    if tokens.is_empty() {
        return Ok(CommandLine {
            pipelines: Vec::new(),
        });
    }

    let mut index = 0;
    let mut connector = Connector::Always;
    let mut pipelines = Vec::new();
    while index < tokens.len() {
        let mut commands = Vec::new();
        loop {
            let mut args = Vec::new();
            let mut redirections = Vec::new();
            while index < tokens.len() {
                match &tokens[index] {
                    Token::Word(word) => {
                        args.push(word.clone());
                        index += 1;
                    }
                    Token::Input
                    | Token::Output
                    | Token::Append
                    | Token::Error
                    | Token::ErrorAppend => {
                        let operator = tokens[index].clone();
                        index += 1;
                        let Some(Token::Word(path)) = tokens.get(index) else {
                            return Err(ParseError::MissingRedirectionTarget);
                        };
                        let path = path.clone();
                        index += 1;
                        redirections.push(match operator {
                            Token::Input => Redirection::Input(path),
                            Token::Output => Redirection::Output {
                                path,
                                append: false,
                            },
                            Token::Append => Redirection::Output { path, append: true },
                            Token::Error => Redirection::Error {
                                path,
                                append: false,
                            },
                            Token::ErrorAppend => Redirection::Error { path, append: true },
                            _ => unreachable!(),
                        });
                    }
                    Token::ErrorToOutput => {
                        redirections.push(Redirection::ErrorToOutput);
                        index += 1;
                    }
                    Token::Pipe | Token::And | Token::Or | Token::Semi => break,
                }
            }
            if args.is_empty() {
                return Err(ParseError::MissingCommand);
            }
            expand_alias(&mut args, aliases)?;
            commands.push(Command { args, redirections });

            if matches!(tokens.get(index), Some(Token::Pipe)) {
                index += 1;
                if index == tokens.len() {
                    return Err(ParseError::MissingCommand);
                }
                continue;
            }
            break;
        }
        pipelines.push((connector, Pipeline { commands }));
        connector = match tokens.get(index) {
            Some(Token::And) => Connector::And,
            Some(Token::Or) => Connector::Or,
            Some(Token::Semi) => Connector::Always,
            None => break,
            Some(_) => return Err(ParseError::MissingCommand),
        };
        index += 1;
        if index == tokens.len() {
            return Err(ParseError::MissingCommand);
        }
    }
    Ok(CommandLine { pipelines })
}

/// Parse a command line and expand aliases in command position. Alias values
/// are parsed with the same quoting and environment expansion rules as normal
/// input, while arguments supplied by the caller are appended unchanged.
pub fn parse_with_aliases(line: &str, aliases: &Aliases) -> Result<Vec<String>, ParseError> {
    let mut args = parse(line)?;
    expand_alias(&mut args, aliases)?;
    Ok(args)
}

fn expand_alias(args: &mut Vec<String>, aliases: &Aliases) -> Result<(), ParseError> {
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
        *args = replacement;
    }
    Ok(())
}

fn lex(line: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = Quote::None;
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if quote == Quote::None && ch.is_whitespace() {
            push_word(&mut tokens, &mut word, &mut started);
            i += 1;
            continue;
        }
        if quote == Quote::None {
            let rest: String = chars[i..].iter().collect();
            let operator = [
                ("2>&1", Token::ErrorToOutput),
                ("2>>", Token::ErrorAppend),
                ("2>", Token::Error),
                (">>", Token::Append),
                ("&&", Token::And),
                ("||", Token::Or),
                ("|", Token::Pipe),
                (";", Token::Semi),
                ("<", Token::Input),
                (">", Token::Output),
            ]
            .into_iter()
            .find(|(text, _)| rest.starts_with(text));
            if let Some((text, token)) = operator {
                push_word(&mut tokens, &mut word, &mut started);
                tokens.push(token);
                i += text.chars().count();
                continue;
            }
            if ch == '&' {
                return Err(ParseError::UnsupportedBackground);
            }
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
            word.push(chars[i + 1]);
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
                word.push_str(&env(&chars[i + 2..end].iter().collect::<String>()));
                started = true;
                i = end + 1;
                continue;
            }
            if chars[i + 1] == '_' || chars[i + 1].is_alphabetic() {
                let mut end = i + 2;
                while end < chars.len() && (chars[end] == '_' || chars[end].is_alphanumeric()) {
                    end += 1;
                }
                word.push_str(&env(&chars[i + 1..end].iter().collect::<String>()));
                started = true;
                i = end;
                continue;
            }
        }
        if quote != Quote::Single && ch == '%' {
            if let Some(n) = chars[i + 1..].iter().position(|c| *c == '%') {
                let end = i + 1 + n;
                if end > i + 1 {
                    word.push_str(&env(&chars[i + 1..end].iter().collect::<String>()));
                    started = true;
                    i = end + 1;
                    continue;
                }
            }
        }
        word.push(ch);
        started = true;
        i += 1;
    }
    if quote != Quote::None {
        return Err(ParseError::UnclosedQuote);
    }
    push_word(&mut tokens, &mut word, &mut started);
    Ok(tokens)
}

fn push_word(tokens: &mut Vec<Token>, word: &mut String, started: &mut bool) {
    if *started {
        tokens.push(Token::Word(expand_home(word)));
        word.clear();
        *started = false;
    }
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
    fn argv_parser_rejects_operators() {
        assert_eq!(parse("a | b"), Err(ParseError::UnsupportedOperator));
    }

    #[test]
    fn parses_pipeline_redirections_and_conditions() {
        let line =
            parse_command_line_with_aliases("a < in | b 2>&1 >> out && c || d; e", &Aliases::new())
                .unwrap();
        assert_eq!(line.pipelines.len(), 4);
        assert_eq!(line.pipelines[0].1.commands.len(), 2);
        assert_eq!(line.pipelines[1].0, Connector::And);
        assert_eq!(line.pipelines[2].0, Connector::Or);
        assert_eq!(line.pipelines[3].0, Connector::Always);
        assert_eq!(
            line.pipelines[0].1.commands[1].redirections,
            [
                Redirection::ErrorToOutput,
                Redirection::Output {
                    path: "out".into(),
                    append: true
                }
            ]
        );
    }

    #[test]
    fn operators_inside_quotes_are_words() {
        let line = parse_command_line_with_aliases("a 'x|y' \"a>b\"", &Aliases::new()).unwrap();
        assert_eq!(line.pipelines[0].1.commands[0].args, ["a", "x|y", "a>b"]);
    }

    #[test]
    fn windows_backslashes_survive() {
        assert_eq!(parse(r#"cd C:\work\x"#).unwrap()[1], r#"C:\work\x"#);
    }

    #[test]
    fn aliases_expand_each_pipeline_stage() {
        let aliases = Aliases::from([("l".into(), "ll".into()), ("ll".into(), "ls -la".into())]);
        let line = parse_command_line_with_aliases("l src | l docs", &aliases).unwrap();
        assert_eq!(line.pipelines[0].1.commands[0].args, ["ls", "-la", "src"]);
        assert_eq!(line.pipelines[0].1.commands[1].args, ["ls", "-la", "docs"]);
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
