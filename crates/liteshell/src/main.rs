use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use liteshell_builtins::{dispatch, Context, NAMES};
use liteshell_core::{
    config::{history_path, Config},
    history::History,
    parser, AppMode, OutputEvent, OutputSink, ShellState,
};
use liteshell_fff::FffSearch;
use liteshell_tui::{draw, CompletionSource, EventBuffer, TerminalSession, TuiState};
use liteshell_windows::{launch, resolve, WindowsCommandResolver};
use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::PathBuf,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("liteshell: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let shell = ShellState::new(std::env::current_dir()?);
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        interactive(shell)
    } else {
        plain(shell)
    }
}

struct PlainOutput;

impl OutputSink for PlainOutput {
    fn emit(&mut self, event: OutputEvent) {
        match event {
            OutputEvent::Text(text) => {
                print!("{text}");
                let _ = io::stdout().flush();
            }
            OutputEvent::Error(text) => {
                eprint!("{text}");
                let _ = io::stderr().flush();
            }
            OutputEvent::Pager { lines, .. } => println!("{}", lines.join("\n")),
            OutputEvent::Clear | OutputEvent::Status(_) => {}
        }
    }
}

fn plain(mut shell: ShellState) -> Result<(), Box<dyn std::error::Error>> {
    let mut search = FffSearch::default();
    let resolver = WindowsCommandResolver;
    let mut output = PlainOutput;
    let config = Config::default();
    let mut history = History::new(history_path(), config.history_capacity);
    let _ = history.load();

    for line in io::stdin().lock().lines() {
        let line = line?;
        let arguments = match parser::parse(&line) {
            Ok(arguments) => arguments,
            Err(error) => {
                eprintln!("parse: {error}");
                continue;
            }
        };
        if arguments.is_empty() {
            continue;
        }

        history.add(&line);
        let result = {
            let mut context = Context {
                shell: &mut shell,
                output: &mut output,
                search: &mut search,
                resolver: &resolver,
                interactive: false,
            };
            dispatch(&arguments, &mut context)
        };

        let status = if result.handled {
            result.status
        } else if let Some(path) = resolve(&arguments[0], &shell.cwd) {
            launch(&path, &arguments[1..], &shell.cwd).unwrap_or_else(|error| {
                eprintln!("{}: cannot start: {error}", arguments[0]);
                126
            })
        } else {
            eprintln!("{}: command not found", arguments[0]);
            127
        };
        shell.last_status = status;
        if !shell.running {
            break;
        }
    }

    let _ = history.save();
    Ok(())
}

fn interactive(mut shell: ShellState) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();
    let mut history = History::new(history_path(), config.history_capacity);
    let _ = history.load();
    let mut history_index = history.entries().len();
    let mut history_scratch = String::new();
    let mut search = FffSearch::default();
    let resolver = WindowsCommandResolver;
    let mut state = TuiState::new(config.scrollback_lines, config.scrollback_bytes);
    let mut terminal = TerminalSession::enter()?;

    while shell.running {
        terminal
            .terminal()
            .draw(|frame| draw(frame, &state, &shell.prompt(), shell.last_status))?;

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if state.mode == AppMode::Pager {
                    handle_pager_key(&mut state, key);
                    continue;
                }

                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if !state.editor.text.is_empty() {
                            state.output.push_text(
                                &format!("{}{}^C\n", shell.prompt(), state.editor.text),
                                false,
                            );
                            state.output.push_divider();
                        }
                        state.editor.clear();
                        state.completion.clear();
                        state.completion_query.clear();
                        state.completion_source = CompletionSource::Path;
                        state.mode = AppMode::Editing;
                        state.status = "cancelled".into();
                    }
                    KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        state.output.clear();
                    }
                    KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        state.mode = AppMode::Completion;
                        state.completion_source = CompletionSource::History;
                        state.completion_query.clear();
                        refresh_history_completion(history.entries(), &mut state);
                    }
                    KeyCode::Char(character) => {
                        state.status.clear();
                        if state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History
                        {
                            state.completion_query.push(character);
                            refresh_history_completion(history.entries(), &mut state);
                        } else {
                            let completion_was_open = state.mode == AppMode::Completion;
                            state.editor.insert(character);
                            if completion_was_open || should_open_path_completion(&state) {
                                complete(&shell, &mut state);
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        if state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History
                        {
                            state.completion_query.pop();
                            refresh_history_completion(history.entries(), &mut state);
                        } else {
                            let completion_was_open = state.mode == AppMode::Completion;
                            state.editor.backspace();
                            if completion_was_open {
                                complete(&shell, &mut state);
                            }
                        }
                    }
                    KeyCode::Delete => {
                        if !(state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History)
                        {
                            let completion_was_open = state.mode == AppMode::Completion;
                            state.editor.delete();
                            if completion_was_open {
                                complete(&shell, &mut state);
                            }
                        }
                    }
                    KeyCode::Left => {
                        if !(state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History)
                        {
                            let completion_was_open = state.mode == AppMode::Completion;
                            state.editor.left();
                            if completion_was_open {
                                complete(&shell, &mut state);
                            }
                        }
                    }
                    KeyCode::Right => {
                        if !(state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History)
                        {
                            let completion_was_open = state.mode == AppMode::Completion;
                            state.editor.right();
                            if completion_was_open {
                                complete(&shell, &mut state);
                            }
                        }
                    }
                    KeyCode::Home => state.editor.cursor = 0,
                    KeyCode::End => state.editor.cursor = state.editor.text.len(),
                    KeyCode::Up => {
                        if state.mode == AppMode::Completion {
                            if !state.completion.is_empty() {
                                state.selected = state
                                    .selected
                                    .checked_sub(1)
                                    .unwrap_or(state.completion.len() - 1);
                            }
                        } else if history_index > 0 {
                            if history_index == history.entries().len() {
                                history_scratch = state.editor.text.clone();
                            }
                            history_index -= 1;
                            state.editor.set(history.entries()[history_index].clone());
                        }
                    }
                    KeyCode::Down => {
                        if state.mode == AppMode::Completion {
                            if !state.completion.is_empty() {
                                state.selected = (state.selected + 1) % state.completion.len();
                            }
                        } else if history_index < history.entries().len() {
                            history_index += 1;
                            let value = if history_index == history.entries().len() {
                                history_scratch.clone()
                            } else {
                                history.entries()[history_index].clone()
                            };
                            state.editor.set(value);
                        }
                    }
                    KeyCode::Tab => {
                        if state.completion.is_empty() {
                            if state.completion_source == CompletionSource::History {
                                refresh_history_completion(history.entries(), &mut state);
                            } else {
                                complete(&shell, &mut state);
                            }
                        } else {
                            accept_completion(&shell, &mut state);
                        }
                    }
                    KeyCode::Esc => {
                        state.completion.clear();
                        state.completion_query.clear();
                        state.completion_source = CompletionSource::Path;
                        state.mode = AppMode::Editing;
                    }
                    KeyCode::Enter => {
                        if state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History
                            && state.completion.is_empty()
                        {
                            state.status = "no matching history".into();
                            continue;
                        }
                        if !state.completion.is_empty() {
                            accept_completion(&shell, &mut state);
                        }
                        let line = state.editor.text.trim_end().to_owned();
                        state.editor.clear();
                        state.completion.clear();
                        state.completion_query.clear();
                        state.completion_source = CompletionSource::Path;
                        state.mode = AppMode::Editing;
                        if !line.trim().is_empty() {
                            history.add(&line);
                            history_index = history.entries().len();
                            execute_interactive(
                                &line,
                                &mut shell,
                                &mut search,
                                &resolver,
                                &mut state,
                                &mut terminal,
                            )?;
                        }
                    }
                    KeyCode::PageUp => state.output.scroll_up(10),
                    KeyCode::PageDown => state.output.scroll_down(10),
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => state.output.scroll_up(3),
                MouseEventKind::ScrollDown => state.output.scroll_down(3),
                _ => {}
            },
            _ => {}
        }
    }

    let _ = history.save();
    Ok(())
}

fn execute_interactive(
    line: &str,
    shell: &mut ShellState,
    search: &mut FffSearch,
    resolver: &WindowsCommandResolver,
    state: &mut TuiState,
    terminal: &mut TerminalSession,
) -> Result<(), Box<dyn std::error::Error>> {
    state
        .output
        .push_text(&format!("{}{}\n", shell.prompt(), line), false);
    let arguments = match parser::parse(line) {
        Ok(arguments) => arguments,
        Err(error) => {
            state.output.push_text(&format!("parse: {error}\n"), true);
            state.output.push_divider();
            return Ok(());
        }
    };

    let mut events = EventBuffer::default();
    let result = {
        let mut context = Context {
            shell,
            output: &mut events,
            search,
            resolver,
            interactive: true,
        };
        dispatch(&arguments, &mut context)
    };
    state.apply(events.0);

    let status = if result.handled {
        result.status
    } else if let Some(path) = resolve(&arguments[0], &shell.cwd) {
        state.mode = AppMode::RunningChild;
        terminal.suspend(|| launch(&path, &arguments[1..], &shell.cwd))??
    } else {
        state
            .output
            .push_text(&format!("{}: command not found\n", arguments[0]), true);
        127
    };
    shell.last_status = status;

    if !result.handled {
        state.output.push_text(
            &format!("[child exited with status {status}]\n"),
            status != 0,
        );
        state.mode = AppMode::Editing;
    }
    state.output.push_divider();
    Ok(())
}

fn complete(shell: &ShellState, state: &mut TuiState) {
    state.mode = AppMode::Completion;
    state.completion_source = CompletionSource::Path;
    state.completion_query.clear();
    let before_cursor = &state.editor.text[..state.editor.cursor];
    let token_start = before_cursor
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0);
    let token = &before_cursor[token_start..];
    let command = before_cursor
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let directories_only = command == "cd";
    let accepts_current_directory = matches!(command.as_str(), "cd" | "ls");
    let mut values: Vec<(i32, String, String)> = Vec::new();

    if token_start == 0 {
        values.extend(NAMES.iter().filter_map(|name| {
            fuzzy_score(name, token).map(|score| (score, (*name).to_owned(), "builtin".to_owned()))
        }));
    } else {
        let expandable_root = matches!(token, "~" | "." | "..")
            || environment_prefix(token).is_some_and(|(_, consumed)| consumed == token.len());
        let separator = token.rfind(['\\', '/']);
        let (directory_part, query, explicit_current) = if expandable_root {
            (format!("{token}\\"), "", Some(token.to_owned()))
        } else {
            match separator {
                Some(index) => (token[..=index].to_owned(), &token[index + 1..], None),
                None => (String::new(), token, None),
            }
        };
        let directory = expand_completion_directory(shell, &directory_part);

        if accepts_current_directory && query.is_empty() {
            let current = explicit_current.unwrap_or_else(|| {
                if directory_part.is_empty() {
                    ".".to_owned()
                } else {
                    current_directory_candidate(&directory_part)
                }
            });
            values.push((i32::MAX, current, "current directory".to_owned()));
        }

        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let is_directory = entry.path().is_dir();
                if directories_only && !is_directory {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(score) = fuzzy_score(&name, query) else {
                    continue;
                };
                let mut value = format!("{directory_part}{name}");
                let detail = if is_directory {
                    value.push('\\');
                    "directory"
                } else {
                    "file"
                };
                values.push((score, value, detail.to_owned()));
            }
        }
    }

    values.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.to_lowercase().cmp(&right.1.to_lowercase()))
    });
    state.completion = values
        .into_iter()
        .map(|(_, value, detail)| (value, detail))
        .collect();
    state.selected = 0;
}

fn expand_completion_directory(shell: &ShellState, raw: &str) -> PathBuf {
    let raw = raw.trim_end_matches(['\\', '/']);
    if raw == "~" || raw.starts_with("~\\") || raw.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let rest = raw[1..].trim_start_matches(['\\', '/']);
            return PathBuf::from(home).join(rest);
        }
    }

    if let Some((value, consumed)) = environment_prefix(raw) {
        let rest = raw[consumed..].trim_start_matches(['\\', '/']);
        return PathBuf::from(value).join(rest);
    }

    if raw.is_empty() {
        shell.cwd.clone()
    } else {
        shell.resolve(PathBuf::from(raw))
    }
}

/// Expand a leading `$VAR`, `${VAR}`, or `%VAR%` and return the number of bytes
/// occupied by the reference. The command line keeps the reference itself so
/// execution still goes through the shell parser's normal expansion rules.
fn environment_prefix(value: &str) -> Option<(std::ffi::OsString, usize)> {
    let (name, consumed) = if let Some(rest) = value.strip_prefix("${") {
        let end = rest.find('}')?;
        (&rest[..end], end + 3)
    } else if let Some(rest) = value.strip_prefix('$') {
        let length = rest
            .char_indices()
            .take_while(|(_, character)| character.is_alphanumeric() || *character == '_')
            .map(|(index, character)| index + character.len_utf8())
            .last()?;
        (&rest[..length], length + 1)
    } else {
        let rest = value.strip_prefix('%')?;
        let end = rest.find('%')?;
        (&rest[..end], end + 2)
    };
    if name.is_empty() {
        return None;
    }
    std::env::var_os(name)
        .or_else(|| {
            name.eq_ignore_ascii_case("HOME")
                .then(|| std::env::var_os("USERPROFILE"))
                .flatten()
        })
        .map(|expanded| (expanded, consumed))
}

fn current_directory_candidate(directory_part: &str) -> String {
    let trimmed = directory_part.trim_end_matches(['\\', '/']);
    if trimmed.ends_with(':') {
        directory_part.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let query: Vec<char> = query.to_lowercase().chars().collect();
    let mut query_index = 0;
    let mut score = 0;
    let mut previous_match = None;

    for (index, character) in candidate.to_lowercase().chars().enumerate() {
        if character != query[query_index] {
            continue;
        }
        score += if previous_match == Some(index.saturating_sub(1)) {
            12
        } else {
            4
        };
        if index == query_index {
            score += 8;
        }
        previous_match = Some(index);
        query_index += 1;
        if query_index == query.len() {
            return Some(score - candidate.chars().count() as i32);
        }
    }
    None
}

fn refresh_history_completion(entries: &[String], state: &mut TuiState) {
    state.mode = AppMode::Completion;
    state.completion_source = CompletionSource::History;
    let query = state.completion_query.as_str();
    let mut seen = std::collections::HashSet::new();
    let mut matches: Vec<(i32, usize, String)> = entries
        .iter()
        .rev()
        .enumerate()
        .filter_map(|(order, command)| {
            if !seen.insert(command.clone()) {
                return None;
            }
            fuzzy_score(command, query).map(|score| (score, order, command.clone()))
        })
        .collect();
    if !query.is_empty() {
        matches.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    }
    state.completion = matches
        .into_iter()
        .map(|(_, _, command)| (command, "history".to_owned()))
        .collect();
    state.selected = 0;
}

fn accept_completion(shell: &ShellState, state: &mut TuiState) {
    if let Some((value, detail)) = state.completion.get(state.selected).cloned() {
        if state.completion_source == CompletionSource::History {
            state.editor.set(value);
            state.completion.clear();
            state.completion_query.clear();
            state.completion_source = CompletionSource::Path;
            state.mode = AppMode::Editing;
            return;
        }

        let token_start = state.editor.text[..state.editor.cursor]
            .rfind(char::is_whitespace)
            .map(|index| index + 1)
            .unwrap_or(0);
        state
            .editor
            .text
            .replace_range(token_start..state.editor.cursor, &value);
        state.editor.cursor = token_start + value.len();
        state.completion.clear();

        if detail == "current directory" {
            state.editor.text.insert(state.editor.cursor, ' ');
            state.editor.cursor += 1;
            state.mode = AppMode::Editing;
        } else if matches!(value.chars().last(), Some('\\' | '/')) {
            // A directory selection drills into that directory. The selected
            // directory itself remains the first candidate, followed by children.
            complete(shell, state);
        } else {
            state.editor.text.insert(state.editor.cursor, ' ');
            state.editor.cursor += 1;
            state.mode = AppMode::Editing;
        }
    }
}

fn should_open_path_completion(state: &TuiState) -> bool {
    let before_cursor = &state.editor.text[..state.editor.cursor];
    if !before_cursor
        .chars()
        .last()
        .is_some_and(char::is_whitespace)
    {
        return false;
    }
    let command = before_cursor.split_whitespace().next().unwrap_or_default();
    matches!(
        command.to_ascii_lowercase().as_str(),
        "cd" | "ls" | "cat" | "tail" | "less" | "find" | "rg"
    )
}

fn handle_pager_key(state: &mut TuiState, key: KeyEvent) {
    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
        state.pager = None;
        state.mode = AppMode::Editing;
        return;
    }

    let Some(pager) = state.pager.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            pager.top = pager
                .top
                .saturating_add(1)
                .min(pager.lines.len().saturating_sub(1));
        }
        KeyCode::Up | KeyCode::Char('k') => pager.top = pager.top.saturating_sub(1),
        KeyCode::PageDown | KeyCode::Char(' ') => {
            pager.top = pager
                .top
                .saturating_add(20)
                .min(pager.lines.len().saturating_sub(1));
        }
        KeyCode::PageUp => pager.top = pager.top.saturating_sub(20),
        KeyCode::Char('g') => pager.top = 0,
        KeyCode::Char('G') => pager.top = pager.lines.len().saturating_sub(1),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matching_accepts_non_contiguous_characters() {
        assert!(fuzzy_score("fuzzy-search", "fzs").is_some());
        assert!(fuzzy_score("fuzzy-search", "xyz").is_none());
    }

    #[test]
    fn history_completion_is_recent_first_then_fuzzy() {
        let entries = vec![
            "pwd".to_owned(),
            "ls src".to_owned(),
            "cd docs".to_owned(),
            "ls src".to_owned(),
        ];
        let mut state = TuiState::new(100, 4096);
        refresh_history_completion(&entries, &mut state);
        assert_eq!(state.completion[0].0, "ls src");
        assert_eq!(state.completion[1].0, "cd docs");

        state.completion_query = "pd".to_owned();
        refresh_history_completion(&entries, &mut state);
        assert_eq!(state.completion[0].0, "pwd");
    }

    #[test]
    fn cd_completion_excludes_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("directory")).unwrap();
        std::fs::write(root.path().join("file.txt"), "file").unwrap();
        let shell = ShellState::new(root.path().to_owned());
        let mut state = TuiState::new(100, 4096);
        state.editor.set("cd ".to_owned());

        complete(&shell, &mut state);

        assert!(state
            .completion
            .iter()
            .any(|(value, detail)| value.starts_with("directory") && detail == "directory"));
        assert!(!state
            .completion
            .iter()
            .any(|(value, _)| value.contains("file.txt")));
    }

    #[test]
    fn ls_completion_starts_with_current_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("directory")).unwrap();
        let shell = ShellState::new(root.path().to_owned());
        let mut state = TuiState::new(100, 4096);
        state.editor.set("ls ".to_owned());

        complete(&shell, &mut state);

        assert_eq!(
            state
                .completion
                .first()
                .map(|(value, detail)| (value.as_str(), detail.as_str())),
            Some((".", "current directory"))
        );
    }

    #[test]
    fn dot_and_environment_roots_are_expanded_for_completion() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("directory")).unwrap();
        let shell = ShellState::new(root.path().to_owned());
        let mut state = TuiState::new(100, 4096);

        state.editor.set("cd ./".to_owned());
        complete(&shell, &mut state);
        assert!(state
            .completion
            .iter()
            .any(|(value, _)| value == ".\\directory\\"));

        std::env::set_var("LITESHELL_COMPLETION_TEST", root.path());
        state
            .editor
            .set("cd $LITESHELL_COMPLETION_TEST\\".to_owned());
        complete(&shell, &mut state);
        std::env::remove_var("LITESHELL_COMPLETION_TEST");
        assert!(state
            .completion
            .iter()
            .any(|(value, _)| value == "$LITESHELL_COMPLETION_TEST\\directory\\"));
    }

    #[cfg(windows)]
    #[test]
    fn accepting_a_directory_lists_its_children() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("parent")).unwrap();
        std::fs::write(root.path().join("parent").join("child.txt"), "child").unwrap();
        let shell = ShellState::new(root.path().to_owned());
        let mut state = TuiState::new(100, 4096);
        state.editor.set("ls ".to_owned());

        complete(&shell, &mut state);
        state.selected = state
            .completion
            .iter()
            .position(|(value, _)| value == "parent\\")
            .unwrap();
        accept_completion(&shell, &mut state);

        assert_eq!(state.editor.text, "ls parent\\");
        assert!(state
            .completion
            .iter()
            .any(|(value, _)| value == "parent\\child.txt"));
    }
}
