use crate::parser::{self, Aliases};
use std::{collections::HashMap, io, path::PathBuf};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct Config {
    pub history_capacity: usize,
    pub scrollback_lines: usize,
    pub scrollback_bytes: usize,
    pub default_tail_lines: usize,
    pub deep_search_exclude_dirs: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            history_capacity: 5_000,
            scrollback_lines: 10_000,
            scrollback_bytes: 4 * 1024 * 1024,
            default_tail_lines: 10,
            deep_search_exclude_dirs: deep_search_exclude_dirs(),
        }
    }
}

pub fn history_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
        })
        .join("LiteShell")
        .join("history")
}

pub fn directory_db_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
        })
        .join("LiteShell")
        .join("directories.db")
}

fn deep_search_exclude_dirs() -> Vec<String> {
    match std::env::var("LITESHELL_DEEP_SEARCH_EXCLUDE_DIRS") {
        Ok(value) => value,
        Err(_) => ".git;node_modules;__pycache__".to_owned(),
    }
    .split(';')
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_owned)
    .collect()
}

pub fn startup_path() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".liteshellrc")
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("{path}:{line}: expected NAME=value")]
    InvalidAssignment { path: PathBuf, line: usize },
    #[error("{path}:{line}: invalid environment variable name '{name}'")]
    InvalidName {
        path: PathBuf,
        line: usize,
        name: String,
    },
    #[error("{path}:{line}: expected alias NAME=value")]
    InvalidAliasAssignment { path: PathBuf, line: usize },
    #[error("{path}:{line}: invalid alias name '{name}'")]
    InvalidAliasName {
        path: PathBuf,
        line: usize,
        name: String,
    },
    #[error("{path}:{line}: invalid alias value: {source}")]
    InvalidAliasValue {
        path: PathBuf,
        line: usize,
        #[source]
        source: parser::ParseError,
    },
    #[error("{path}:{line}: expected complete COMMAND clap-env ENVIRONMENT_VARIABLE")]
    InvalidCompletion { path: PathBuf, line: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionProviderConfig {
    pub command: String,
    pub environment_variable: String,
}

#[derive(Clone, Debug, Default)]
pub struct StartupConfig {
    pub path: Option<PathBuf>,
    pub aliases: Aliases,
    pub completions: Vec<CompletionProviderConfig>,
}

/// Load environment assignments from `~/.liteshellrc` before command
/// resolution. The file is intentionally data-only: it does not execute shell
/// commands. Values may reference earlier assignments or inherited variables
/// using `%NAME%`, `$NAME`, or `${NAME}`.
pub fn load_startup() -> Result<StartupConfig, StartupError> {
    let path = startup_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(StartupConfig::default())
        }
        Err(source) => {
            return Err(StartupError::Read {
                path: path.clone(),
                source,
            })
        }
    };

    let mut environment: HashMap<String, String> = std::env::vars()
        .map(|(name, value)| (name.to_ascii_uppercase(), value))
        .collect();
    let mut assignments = Vec::new();
    let mut aliases = Aliases::new();
    let mut completions = Vec::new();
    for (index, source) in contents.lines().enumerate() {
        let line_number = index + 1;
        let mut line = source.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(value) = strip_keyword(line, "complete") {
            let Some(provider) = parse_completion_provider(value) else {
                return Err(StartupError::InvalidCompletion {
                    path,
                    line: line_number,
                });
            };
            completions.push(provider);
            continue;
        }
        if let Some(alias) = strip_keyword(line, "alias") {
            let Some((name, raw_value)) = alias.split_once('=') else {
                return Err(StartupError::InvalidAliasAssignment {
                    path,
                    line: line_number,
                });
            };
            let name = name.trim();
            if !valid_alias_name(name) {
                return Err(StartupError::InvalidAliasName {
                    path,
                    line: line_number,
                    name: name.to_owned(),
                });
            }
            let value = unquote(raw_value.trim());
            match parser::parse(value) {
                Ok(arguments) if !arguments.is_empty() => {}
                Ok(_) => {
                    return Err(StartupError::InvalidAliasAssignment {
                        path,
                        line: line_number,
                    })
                }
                Err(source) => {
                    return Err(StartupError::InvalidAliasValue {
                        path,
                        line: line_number,
                        source,
                    })
                }
            }
            aliases.insert(name.to_owned(), value.to_owned());
            continue;
        }
        if let Some(value) = line.strip_prefix("export ") {
            line = value.trim_start();
        } else if let Some(value) = line.strip_prefix("set ") {
            line = value.trim_start();
        }
        if line.starts_with('"') && line.ends_with('"') && line.len() >= 2 {
            line = &line[1..line.len() - 1];
        }
        let Some((name, raw_value)) = line.split_once('=') else {
            return Err(StartupError::InvalidAssignment {
                path,
                line: line_number,
            });
        };
        let name = name.trim();
        if !valid_environment_name(name) {
            return Err(StartupError::InvalidName {
                path,
                line: line_number,
                name: name.to_owned(),
            });
        }
        let raw_value = raw_value.trim();
        let (raw_value, expand) =
            if raw_value.starts_with('\'') && raw_value.ends_with('\'') && raw_value.len() >= 2 {
                (&raw_value[1..raw_value.len() - 1], false)
            } else if raw_value.starts_with('"') && raw_value.ends_with('"') && raw_value.len() >= 2
            {
                (&raw_value[1..raw_value.len() - 1], true)
            } else {
                (raw_value, true)
            };
        let value = if expand {
            expand_variables(raw_value, |variable| {
                environment
                    .get(&variable.to_ascii_uppercase())
                    .cloned()
                    .unwrap_or_default()
            })
        } else {
            raw_value.to_owned()
        };
        environment.insert(name.to_ascii_uppercase(), value.clone());
        assignments.push((name.to_owned(), value));
    }

    for (name, value) in assignments {
        std::env::set_var(name, value);
    }
    Ok(StartupConfig {
        path: Some(path),
        aliases,
        completions,
    })
}

/// Compatibility helper for callers that only need startup environment values.
pub fn load_startup_environment() -> Result<Option<PathBuf>, StartupError> {
    load_startup().map(|startup| startup.path)
}

fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    line.strip_prefix(keyword).and_then(|rest| {
        rest.chars()
            .next()
            .filter(|ch| ch.is_whitespace())
            .map(|_| rest.trim_start())
    })
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn valid_alias_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-.".contains(character))
}

fn parse_completion_provider(value: &str) -> Option<CompletionProviderConfig> {
    let fields: Vec<_> = value.split_whitespace().collect();
    (fields.len() == 3
        && fields[1] == "clap-env"
        && valid_alias_name(fields[0])
        && valid_environment_name(fields[2]))
    .then(|| CompletionProviderConfig {
        command: fields[0].to_owned(),
        environment_variable: fields[2].to_owned(),
    })
}

fn valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn expand_variables(value: &str, mut lookup: impl FnMut(&str) -> String) -> String {
    let characters: Vec<char> = value.chars().collect();
    let mut output = String::new();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '%' {
            if let Some(relative_end) = characters[index + 1..]
                .iter()
                .position(|character| *character == '%')
            {
                let end = index + 1 + relative_end;
                if end > index + 1 {
                    output.push_str(&lookup(
                        &characters[index + 1..end].iter().collect::<String>(),
                    ));
                    index = end + 1;
                    continue;
                }
            }
        } else if characters[index] == '$' && index + 1 < characters.len() {
            if characters[index + 1] == '{' {
                if let Some(relative_end) = characters[index + 2..]
                    .iter()
                    .position(|character| *character == '}')
                {
                    let end = index + 2 + relative_end;
                    output.push_str(&lookup(
                        &characters[index + 2..end].iter().collect::<String>(),
                    ));
                    index = end + 1;
                    continue;
                }
            } else if characters[index + 1] == '_' || characters[index + 1].is_ascii_alphabetic() {
                let mut end = index + 2;
                while end < characters.len()
                    && (characters[end] == '_' || characters[end].is_ascii_alphanumeric())
                {
                    end += 1;
                }
                output.push_str(&lookup(
                    &characters[index + 1..end].iter().collect::<String>(),
                ));
                index = end;
                continue;
            }
        }
        output.push(characters[index]);
        index += 1;
    }
    output
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn startup_values_expand_windows_and_posix_variables() {
        let result = expand_variables(r"%ROOT%\bin;$ROOT\tools;${ROOT}\lib", |name| {
            if name == "ROOT" {
                r"D:\home".to_owned()
            } else {
                String::new()
            }
        });
        assert_eq!(result, r"D:\home\bin;D:\home\tools;D:\home\lib");
    }

    #[test]
    fn startup_names_are_restricted_to_environment_identifiers() {
        assert!(valid_environment_name("PATH"));
        assert!(valid_environment_name("SDK_VERSION"));
        assert!(!valid_environment_name("BAD-NAME"));
        assert!(!valid_environment_name("1BAD"));
    }

    #[test]
    fn alias_names_allow_command_like_characters() {
        assert!(valid_alias_name("ll"));
        assert!(valid_alias_name("git-lg"));
        assert!(valid_alias_name(".."));
        assert!(!valid_alias_name("bad name"));
        assert!(!valid_alias_name("bad=alias"));
    }

    #[test]
    fn completion_provider_directives_are_strict_and_data_only() {
        assert_eq!(
            parse_completion_provider("just clap-env JUST_COMPLETE"),
            Some(CompletionProviderConfig {
                command: "just".to_owned(),
                environment_variable: "JUST_COMPLETE".to_owned(),
            })
        );
        assert!(parse_completion_provider("just powershell script.ps1").is_none());
        assert!(parse_completion_provider("bad/name clap-env COMPLETE").is_none());
    }
}
