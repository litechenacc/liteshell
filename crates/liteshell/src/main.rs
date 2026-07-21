use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use liteshell_builtins::{dispatch, overview_text, version_text, Context, NAMES};
use liteshell_core::{
    config::{directory_db_path, history_path, load_startup, Config, StartupConfig},
    directory_db::{frecency, now_epoch, DirectoryDb},
    history::History,
    parser::{self, Aliases},
    AppMode, OutputEvent, OutputSink, SearchCandidate, SearchKind, SearchProvider, ShellState,
};
use liteshell_fff::FffSearch;
use liteshell_tui::{
    draw, selected_pager_text, selected_transcript_text, CompletionSource, EventBuffer,
    TerminalSession, TuiState,
};
use liteshell_windows::{
    build_command, get_clipboard_text, is_supported_executable, launch, resolve,
    set_clipboard_text, spawn_captured, terminate_process_tree, translate, ProcessJob,
    WindowsCommandResolver, TRANSLATED_NAMES,
};
use std::{
    collections::{HashMap, HashSet},
    io::{self, BufRead, BufReader, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    time::{Duration, Instant},
};

mod executor;

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const DIRECTORY_DB_FLUSH_INTERVAL: Duration = Duration::from_secs(5 * 60);

enum SearchTaskEvent {
    Candidate(SearchCandidate),
    Error(String),
    Finished { status: i32 },
}

struct RunningSearch {
    command: String,
    receiver: Receiver<SearchTaskEvent>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
    results: usize,
}

enum ExternalTaskEvent {
    Output { text: String, error: bool },
    Error(String),
    Finished { status: i32 },
}

struct RunningExternal {
    command: String,
    receiver: Receiver<ExternalTaskEvent>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

enum DeepCompletionEvent {
    Finished(Result<Vec<SearchCandidate>, String>),
}

struct RunningDeepCompletion {
    token: String,
    receiver: Receiver<DeepCompletionEvent>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

enum NativeCompletionEvent {
    Finished(Result<Vec<(String, String)>, String>),
}

struct RunningNativeCompletion {
    request: String,
    receiver: Receiver<NativeCompletionEvent>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

#[derive(Default)]
struct RunningTasks {
    search: Option<RunningSearch>,
    external: Option<RunningExternal>,
    deep_completion: Option<RunningDeepCompletion>,
    native_completion: Option<RunningNativeCompletion>,
}

struct InteractiveCommands {
    resolver: WindowsCommandResolver,
    aliases: Aliases,
    path_commands: Vec<(String, String)>,
    completion_providers: HashMap<String, String>,
}

fn main() {
    match run() {
        Ok(0) => {}
        Ok(status) => std::process::exit(status),
        Err(error) => {
            eprintln!("liteshell: {error}");
            std::process::exit(1);
        }
    }
}

enum Invocation {
    Auto { statusline: bool },
    Command { line: String, pipefail: bool },
    Help,
    Version,
}

fn invocation() -> Result<Invocation, String> {
    let mut args = std::env::args().skip(1);
    let mut command = None;
    let mut pipefail = true;
    let mut statusline = true;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-c" | "--command" => {
                if command.is_some() {
                    return Err("command may only be supplied once".into());
                }
                command = Some(
                    args.next()
                        .ok_or_else(|| format!("{argument} requires a command string"))?,
                );
            }
            "--pipefail" => pipefail = true,
            "--no-pipefail" => pipefail = false,
            "--status-line=auto" | "--status-line=on" => statusline = true,
            "--status-line=off" | "--no-status-line" => statusline = false,
            "--status-line" => {
                statusline = match args.next().as_deref() {
                    Some("auto" | "on") => true,
                    Some("off") => false,
                    Some(value) => {
                        return Err(format!(
                            "invalid status-line value: {value}; expected auto, on, or off"
                        ))
                    }
                    None => return Err("--status-line requires auto, on, or off".into()),
                };
            }
            "-h" | "--help" => return Ok(Invocation::Help),
            "-V" | "--version" => return Ok(Invocation::Version),
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }
    Ok(match command {
        Some(line) => Invocation::Command { line, pipefail },
        None => Invocation::Auto { statusline },
    })
}

fn print_help() {
    print!("{}", overview_text());
}

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let invocation =
        invocation().map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if matches!(invocation, Invocation::Help) {
        print_help();
        return Ok(0);
    }
    if matches!(invocation, Invocation::Version) {
        print!("{}", version_text());
        return Ok(0);
    }
    let startup = load_startup()?;
    let mut shell = ShellState::new(std::env::current_dir()?);
    match invocation {
        Invocation::Command { line, pipefail } => {
            let command_line =
                match parser::parse_command_line_with_aliases(&line, &startup.aliases) {
                    Ok(command_line) => command_line,
                    Err(error) => {
                        eprintln!("parse: {error}");
                        return Ok(2);
                    }
                };
            Ok(executor::execute(&command_line, &mut shell, pipefail))
        }
        Invocation::Auto { statusline }
            if io::stdin().is_terminal() && io::stdout().is_terminal() =>
        {
            interactive(shell, startup, statusline)?;
            Ok(0)
        }
        Invocation::Auto { .. } => plain(shell, startup.aliases),
        Invocation::Help => unreachable!(),
        Invocation::Version => unreachable!(),
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
            OutputEvent::Styled(text) => {
                print!("{}", text.text());
                let _ = io::stdout().flush();
            }
            OutputEvent::Error(text) => {
                eprint!("{text}");
                let _ = io::stderr().flush();
            }
            OutputEvent::Pager { lines, .. } => println!(
                "{}",
                lines
                    .iter()
                    .map(|line| line.text())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            OutputEvent::Clear | OutputEvent::Status(_) => {}
        }
    }
}

fn plain(mut shell: ShellState, aliases: Aliases) -> Result<i32, Box<dyn std::error::Error>> {
    let resolver = WindowsCommandResolver;
    let mut output = PlainOutput;
    let config = Config::default();
    let mut search = FffSearch::new(config.deep_search_exclude_dirs.clone());
    let mut history = History::new(history_path(), config.history_capacity);
    let _ = history.load();
    let mut directory_db = DirectoryDb::new(directory_db_path());
    let _ = directory_db.load();
    directory_db.record(&shell.cwd);

    for line in io::stdin().lock().lines() {
        let line = line?;
        let arguments = match parser::parse_with_aliases(&line, &aliases) {
            Ok(arguments) => arguments,
            Err(error) => {
                eprintln!("parse: {error}");
                shell.last_status = 2;
                continue;
            }
        };
        if arguments.is_empty() {
            continue;
        }

        history.add(&line);
        let previous_cwd = shell.cwd.clone();
        let result = {
            let mut context = Context {
                shell: &mut shell,
                input: &mut io::empty(),
                output: &mut output,
                search: &mut search,
                resolver: &resolver,
                interactive: false,
            };
            dispatch(&arguments, &mut context)
        };

        let status = if result.handled {
            result.status
        } else {
            match external_invocation(&arguments, &shell.cwd) {
                Ok(Some((path, translated_args))) => launch(&path, &translated_args, &shell.cwd)
                    .unwrap_or_else(|error| {
                        eprintln!("{}: cannot start: {error}", arguments[0]);
                        126
                    }),
                Ok(None) => {
                    eprintln!("{}: command not found", arguments[0]);
                    127
                }
                Err(error) => {
                    eprintln!("{error}");
                    2
                }
            }
        };
        shell.last_status = status;
        if result.handled && result.status == 0 && shell.cwd != previous_cwd {
            directory_db.record(&shell.cwd);
        }
        let _ = directory_db.flush_if_due(DIRECTORY_DB_FLUSH_INTERVAL);
        if !shell.running {
            break;
        }
    }

    let _ = history.save();
    directory_db.flush()?;
    Ok(shell.last_status)
}

fn interactive(
    mut shell: ShellState,
    startup: StartupConfig,
    show_statusline: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();
    let mut history = History::new(history_path(), config.history_capacity);
    let _ = history.load();
    let mut history_index = history.entries().len();
    let mut history_scratch = String::new();
    let mut search = FffSearch::new(config.deep_search_exclude_dirs.clone());
    let mut directory_db = DirectoryDb::new(directory_db_path());
    let _ = directory_db.load();
    directory_db.record(&shell.cwd);
    let mut completion_providers = HashMap::from([("just".to_owned(), "JUST_COMPLETE".to_owned())]);
    for provider in startup.completions {
        completion_providers.insert(
            provider.command.to_ascii_lowercase(),
            provider.environment_variable,
        );
    }
    let commands = InteractiveCommands {
        resolver: WindowsCommandResolver,
        aliases: startup.aliases,
        path_commands: discover_path_commands(),
        completion_providers,
    };
    let mut state = TuiState::new(config.scrollback_lines, config.scrollback_bytes);
    let mut terminal = TerminalSession::enter()?;
    let mut running = RunningTasks::default();

    while shell.running {
        let _ = directory_db.flush_if_due(DIRECTORY_DB_FLUSH_INTERVAL);
        state.update_scroll_flash();
        update_running_search(&mut running.search, &mut state, &mut shell.last_status);
        update_running_external(&mut running.external, &mut state, &mut shell.last_status);
        update_deep_completion(&mut running.deep_completion, &mut state);
        update_native_completion(&mut running.native_completion, &mut state);
        if let Some(task) = running.search.as_ref() {
            let frame = (task.started.elapsed().as_millis() / 80) as usize % SPINNER.len();
            state.status = format!(
                "{} {} {}… {} result{}",
                SPINNER[frame],
                if task.cancelled.load(Ordering::Relaxed) {
                    "cancelling"
                } else {
                    "running"
                },
                task.command,
                task.results,
                if task.results == 1 { "" } else { "s" }
            );
        } else if let Some(task) = running.external.as_ref() {
            let frame = (task.started.elapsed().as_millis() / 80) as usize % SPINNER.len();
            state.status = format!(
                "{} {} {}…",
                SPINNER[frame],
                if task.cancelled.load(Ordering::Relaxed) {
                    "cancelling"
                } else {
                    "running"
                },
                task.command,
            );
        } else if let Some(task) = running.deep_completion.as_ref() {
            let frame = (task.started.elapsed().as_millis() / 80) as usize % SPINNER.len();
            state.status = format!("{} searching directories…", SPINNER[frame]);
        } else if let Some(task) = running.native_completion.as_ref() {
            let frame = (task.started.elapsed().as_millis() / 80) as usize % SPINNER.len();
            state.status = format!("{} asking command for completions…", SPINNER[frame]);
        }

        terminal.terminal().draw(|frame| {
            draw(
                frame,
                &state,
                &shell.prompt(),
                shell.last_status,
                show_statusline,
            )
        })?;

        let terminal_height = terminal.terminal().size()?.height as usize;
        let terminal_width = terminal.terminal().size()?.width as usize;
        let transcript_height = terminal_height.saturating_sub(usize::from(show_statusline));

        let wait = if running.search.is_some()
            || running.external.is_some()
            || running.deep_completion.is_some()
            || running.native_completion.is_some()
            || state.scroll_flash_active()
        {
            Duration::from_millis(80)
        } else {
            Duration::from_secs(60)
        };
        if !event::poll(wait)? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(task) = running.external.as_ref() {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        task.cancelled.store(true, Ordering::Relaxed);
                        state.status = format!("cancelling {}…", task.command);
                    }
                    continue;
                }
                if let Some(task) = running.search.as_ref() {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        task.cancelled.store(true, Ordering::Relaxed);
                        state.status = format!("cancelling {}…", task.command);
                    }
                    continue;
                }
                if state.mode == AppMode::Pager {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                        && state.has_pager_selection()
                    {
                        copy_pager_selection(&mut state, terminal_width);
                        continue;
                    }
                    if key.code == KeyCode::Esc && state.has_pager_selection() {
                        state.clear_pager_selection();
                        state.status.clear();
                        continue;
                    }
                    handle_pager_key(
                        &mut state,
                        key,
                        terminal_height.saturating_sub(1),
                        terminal_width,
                    );
                    continue;
                }

                let preserves_selection =
                    matches!(key.code, KeyCode::Esc | KeyCode::PageUp | KeyCode::PageDown)
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL));
                if !preserves_selection {
                    state.clear_transcript_selection();
                }

                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if state.has_transcript_selection() {
                            copy_transcript_selection(&mut state, &shell.prompt(), terminal_width);
                            continue;
                        }
                        cancel_deep_completion(&mut running.deep_completion);
                        cancel_native_completion(&mut running.native_completion);
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
                        state.clear_transcript_selection();
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
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    &commands,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &mut running.native_completion,
                                    &config.deep_search_exclude_dirs,
                                );
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        state.clear_transcript_selection();
                        if state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History
                        {
                            state.completion_query.pop();
                            refresh_history_completion(history.entries(), &mut state);
                        } else {
                            let completion_was_open = state.mode == AppMode::Completion;
                            state.editor.backspace();
                            if completion_was_open {
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    &commands,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &mut running.native_completion,
                                    &config.deep_search_exclude_dirs,
                                );
                            }
                        }
                    }
                    KeyCode::Delete => {
                        state.clear_transcript_selection();
                        if !(state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History)
                        {
                            let completion_was_open = state.mode == AppMode::Completion;
                            state.editor.delete();
                            if completion_was_open {
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    &commands,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &mut running.native_completion,
                                    &config.deep_search_exclude_dirs,
                                );
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
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    &commands,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &mut running.native_completion,
                                    &config.deep_search_exclude_dirs,
                                );
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
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    &commands,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &mut running.native_completion,
                                    &config.deep_search_exclude_dirs,
                                );
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
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    &commands,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &mut running.native_completion,
                                    &config.deep_search_exclude_dirs,
                                );
                            }
                        } else {
                            accept_completion(&shell, &mut state);
                        }
                    }
                    KeyCode::Esc => {
                        if state.has_transcript_selection() {
                            state.clear_transcript_selection();
                            state.status.clear();
                            continue;
                        }
                        cancel_deep_completion(&mut running.deep_completion);
                        cancel_native_completion(&mut running.native_completion);
                        state.completion.clear();
                        state.completion_query.clear();
                        state.completion_source = CompletionSource::Path;
                        state.mode = AppMode::Editing;
                    }
                    KeyCode::Enter => {
                        state.clear_transcript_selection();
                        cancel_native_completion(&mut running.native_completion);
                        if state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::History
                            && state.completion.is_empty()
                        {
                            state.status = "no matching history".into();
                            continue;
                        }
                        if state.mode == AppMode::Completion
                            && state.completion_source == CompletionSource::DeepPath
                            && state.completion.is_empty()
                        {
                            state.status = if running.deep_completion.is_some() {
                                "recursive search is still running".into()
                            } else {
                                "no matching directories".into()
                            };
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
                            let previous_cwd = shell.cwd.clone();
                            execute_interactive(
                                &line,
                                &mut shell,
                                &mut search,
                                &commands,
                                &mut state,
                                &mut terminal,
                                &mut running,
                            )?;
                            if shell.cwd != previous_cwd {
                                directory_db.record(&shell.cwd);
                            }
                        }
                    }
                    KeyCode::PageUp => {
                        scroll_transcript_up(
                            &mut state,
                            transcript_height.saturating_sub(1).max(1),
                            transcript_height,
                        );
                    }
                    KeyCode::PageDown => {
                        scroll_transcript_down(
                            &mut state,
                            transcript_height.saturating_sub(1).max(1),
                            transcript_height,
                        );
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if state.mode == AppMode::Pager {
                        if mouse.row < terminal_height.saturating_sub(1) as u16
                            && state.begin_pager_selection(
                                mouse.column,
                                mouse.row,
                                terminal_height.saturating_sub(1),
                                terminal_width,
                            )
                        {
                            state.status = "selecting text…".into();
                        } else {
                            state.clear_pager_selection();
                        }
                    } else if matches!(state.mode, AppMode::Editing | AppMode::Completion)
                        && mouse.row < transcript_height as u16
                        && state.begin_transcript_selection(
                            mouse.column,
                            mouse.row,
                            transcript_height,
                        )
                    {
                        cancel_deep_completion(&mut running.deep_completion);
                        cancel_native_completion(&mut running.native_completion);
                        state.completion.clear();
                        state.completion_query.clear();
                        state.completion_source = CompletionSource::Path;
                        state.mode = AppMode::Editing;
                        state.status = "selecting text…".into();
                    } else if state.mode != AppMode::Pager {
                        state.clear_transcript_selection();
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if state.mode == AppMode::Pager {
                        state.update_pager_selection(
                            mouse.column,
                            mouse.row.min(terminal_height.saturating_sub(2) as u16),
                            terminal_height.saturating_sub(1),
                            terminal_width,
                        );
                    } else if matches!(state.mode, AppMode::Editing | AppMode::Completion) {
                        state.update_transcript_selection(
                            mouse.column,
                            mouse.row.min(transcript_height.saturating_sub(1) as u16),
                            transcript_height,
                        );
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if state.mode == AppMode::Pager {
                        state.update_pager_selection(
                            mouse.column,
                            mouse.row.min(terminal_height.saturating_sub(2) as u16),
                            terminal_height.saturating_sub(1),
                            terminal_width,
                        );
                        if state.finish_pager_selection() {
                            copy_pager_selection(&mut state, terminal_width);
                        } else {
                            state.status.clear();
                        }
                    } else if matches!(state.mode, AppMode::Editing | AppMode::Completion) {
                        state.update_transcript_selection(
                            mouse.column,
                            mouse.row.min(transcript_height.saturating_sub(1) as u16),
                            transcript_height,
                        );
                        if state.finish_transcript_selection() {
                            copy_transcript_selection(&mut state, &shell.prompt(), terminal_width);
                        } else {
                            state.status.clear();
                        }
                    }
                }
                MouseEventKind::Down(MouseButton::Right) => {
                    if state.mode == AppMode::Pager && state.has_pager_selection() {
                        copy_pager_selection(&mut state, terminal_width);
                        state.clear_pager_selection();
                    } else if state.has_transcript_selection() {
                        copy_transcript_selection(&mut state, &shell.prompt(), terminal_width);
                        state.clear_transcript_selection();
                    } else if matches!(state.mode, AppMode::Editing | AppMode::Completion) {
                        cancel_deep_completion(&mut running.deep_completion);
                        cancel_native_completion(&mut running.native_completion);
                        state.completion.clear();
                        state.completion_query.clear();
                        state.completion_source = CompletionSource::Path;
                        state.mode = AppMode::Editing;
                        paste_clipboard(&mut state);
                    }
                }
                MouseEventKind::ScrollUp => {
                    if state.mode == AppMode::Pager {
                        scroll_pager_up(
                            &mut state,
                            3,
                            terminal_height.saturating_sub(1),
                            terminal_width,
                        );
                    } else {
                        scroll_transcript_up(&mut state, 3, transcript_height);
                    }
                }
                MouseEventKind::ScrollDown => {
                    if state.mode == AppMode::Pager {
                        scroll_pager_down(
                            &mut state,
                            3,
                            terminal_height.saturating_sub(1),
                            terminal_width,
                        );
                    } else {
                        scroll_transcript_down(&mut state, 3, transcript_height);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    let _ = history.save();
    directory_db.flush()?;
    Ok(())
}

fn execute_interactive(
    line: &str,
    shell: &mut ShellState,
    search: &mut FffSearch,
    commands: &InteractiveCommands,
    state: &mut TuiState,
    terminal: &mut TerminalSession,
    running: &mut RunningTasks,
) -> Result<(), Box<dyn std::error::Error>> {
    state
        .output
        .push_text(&format!("{}{}\n", shell.prompt(), line), false);
    let command_line = match parser::parse_command_line_with_aliases(line, &commands.aliases) {
        Ok(command_line) => command_line,
        Err(error) => {
            state.output.push_text(&format!("parse: {error}\n"), true);
            shell.last_status = 2;
            state.output.push_divider();
            return Ok(());
        }
    };
    let compound = command_line.pipelines.len() != 1
        || command_line.pipelines[0].1.commands.len() != 1
        || !command_line.pipelines[0].1.commands[0]
            .redirections
            .is_empty();
    if compound {
        state.mode = AppMode::RunningChild;
        let result = executor::execute_captured(&command_line, shell, true)?;
        shell.last_status = result.status;
        if !result.stdout.is_empty() {
            state
                .output
                .push_text(&String::from_utf8_lossy(&result.stdout), false);
        }
        if !result.stderr.is_empty() {
            state
                .output
                .push_text(&String::from_utf8_lossy(&result.stderr), true);
        }
        state.mode = AppMode::Editing;
        state.output.push_divider();
        return Ok(());
    }
    let arguments = command_line.pipelines[0].1.commands[0].args.clone();

    let detailed_help = arguments.len() == 2 && matches!(arguments[1].as_str(), "-h" | "--help");
    if !detailed_help && matches!(arguments[0].to_ascii_lowercase().as_str(), "find" | "rg") {
        if arguments[0].eq_ignore_ascii_case("rg") && arguments.len() < 2 {
            state
                .output
                .push_text("rg: expected a search query\n", true);
            shell.last_status = 2;
            state.output.push_divider();
            return Ok(());
        }
        running.search = Some(start_search(
            arguments[0].to_ascii_lowercase(),
            arguments[1..].join(" "),
            shell.cwd.clone(),
            search.excluded_directories().map(str::to_owned).collect(),
        ));
        state.mode = AppMode::RunningTask;
        return Ok(());
    }

    let mut events = EventBuffer::default();
    let result = {
        let mut context = Context {
            shell,
            input: &mut io::empty(),
            output: &mut events,
            search,
            resolver: &commands.resolver,
            interactive: true,
        };
        dispatch(&arguments, &mut context)
    };
    state.apply(events.0);

    if result.handled {
        shell.last_status = result.status;
        state.output.push_divider();
        return Ok(());
    }

    let status = match external_invocation(&arguments, &shell.cwd) {
        Ok(Some((path, translated_args))) if requires_terminal(&path, &arguments) => {
            state.mode = AppMode::RunningChild;
            terminal.suspend(|| launch(&path, &translated_args, &shell.cwd))??
        }
        Ok(Some((path, translated_args))) => {
            match start_external(
                arguments[0].clone(),
                path,
                translated_args,
                shell.cwd.clone(),
            ) {
                Ok(task) => {
                    running.external = Some(task);
                    state.mode = AppMode::RunningTask;
                    return Ok(());
                }
                Err(error) => {
                    state
                        .output
                        .push_text(&format!("{}: cannot start: {error}\n", arguments[0]), true);
                    126
                }
            }
        }
        Ok(None) => {
            state
                .output
                .push_text(&format!("{}: command not found\n", arguments[0]), true);
            127
        }
        Err(error) => {
            state.output.push_text(&format!("{error}\n"), true);
            2
        }
    };
    shell.last_status = status;
    state.mode = AppMode::Editing;
    state.output.push_divider();
    Ok(())
}

fn external_invocation(
    arguments: &[String],
    cwd: &Path,
) -> Result<Option<(PathBuf, Vec<String>)>, String> {
    if let Some(translated) = translate(&arguments[0], &arguments[1..], cwd) {
        return translated
            .map(|command| Some((command.path, command.args)))
            .map_err(|error| error.to_string());
    }
    Ok(resolve(&arguments[0], cwd).map(|path| (path, arguments[1..].to_vec())))
}

fn requires_terminal(path: &Path, arguments: &[String]) -> bool {
    let command = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| arguments.first().map(String::as_str).unwrap_or_default())
        .to_ascii_lowercase();
    matches!(
        command.as_str(),
        "cmd"
            | "powershell"
            | "pwsh"
            | "bash"
            | "zsh"
            | "fish"
            | "wsl"
            | "ssh"
            | "nvim"
            | "vim"
            | "vi"
            | "nano"
            | "emacs"
            | "hx"
            | "helix"
            | "less"
            | "more"
            | "man"
            | "codex"
    )
}

fn start_external(
    command: String,
    path: PathBuf,
    arguments: Vec<String>,
    cwd: PathBuf,
) -> io::Result<RunningExternal> {
    let mut child = spawn_captured(&path, &arguments, &cwd)?;
    let stdout = child.stdout.take().expect("captured child stdout");
    let stderr = child.stderr.take().expect("captured child stderr");
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);

    std::thread::spawn(move || {
        let stdout_sender = sender.clone();
        let stdout_reader =
            std::thread::spawn(move || stream_external_output(stdout, false, stdout_sender));
        let stderr_sender = sender.clone();
        let stderr_reader =
            std::thread::spawn(move || stream_external_output(stderr, true, stderr_sender));

        let mut was_cancelled = false;
        let status = loop {
            if worker_cancelled.load(Ordering::Relaxed) && !was_cancelled {
                was_cancelled = true;
                if let Err(error) = terminate_process_tree(&mut child) {
                    let _ = sender.send(ExternalTaskEvent::Error(format!(
                        "cannot stop child process: {error}\n"
                    )));
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    break if was_cancelled {
                        130
                    } else {
                        status.code().unwrap_or(1)
                    }
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(error) => {
                    let _ = sender.send(ExternalTaskEvent::Error(format!(
                        "cannot wait for child process: {error}\n"
                    )));
                    break 1;
                }
            }
        };

        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        let _ = sender.send(ExternalTaskEvent::Finished { status });
    });

    Ok(RunningExternal {
        command,
        receiver,
        cancelled,
        started: Instant::now(),
    })
}

fn stream_external_output(reader: impl Read, error: bool, sender: mpsc::Sender<ExternalTaskEvent>) {
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Ok(_) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if sender
                    .send(ExternalTaskEvent::Output { text, error })
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send(ExternalTaskEvent::Error(format!(
                    "cannot read child output: {error}\n"
                )));
                break;
            }
        }
    }
}

fn update_running_external(
    running: &mut Option<RunningExternal>,
    state: &mut TuiState,
    last_status: &mut i32,
) {
    let mut finished = None;
    if let Some(task) = running.as_mut() {
        loop {
            match task.receiver.try_recv() {
                Ok(ExternalTaskEvent::Output { text, error }) => {
                    state.output.push_text(&text, error)
                }
                Ok(ExternalTaskEvent::Error(error)) => state.output.push_text(&error, true),
                Ok(ExternalTaskEvent::Finished { status }) => {
                    finished = Some(status);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    state
                        .output
                        .push_text("external command worker stopped unexpectedly\n", true);
                    finished = Some(1);
                    break;
                }
            }
        }
    }

    if let Some(status) = finished {
        let task = running.take().expect("running external command");
        *last_status = status;
        state.output.push_divider();
        state.mode = AppMode::Editing;
        state.status = format!(
            "{} {} · {:.1}s",
            task.command,
            if status == 0 {
                "finished"
            } else if status == 130 {
                "cancelled"
            } else {
                "failed"
            },
            task.started.elapsed().as_secs_f32(),
        );
    }
}

fn start_search(
    command: String,
    query: String,
    cwd: PathBuf,
    excluded_directories: Vec<String>,
) -> RunningSearch {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let worker_command = command.clone();
    std::thread::spawn(move || {
        let mut search = FffSearch::new(excluded_directories);
        let kind = if worker_command == "rg" {
            SearchKind::Grep
        } else {
            SearchKind::Files
        };
        let mut emit = |candidate| {
            let _ = sender.send(SearchTaskEvent::Candidate(candidate));
        };
        let result = search.search_stream(kind, &query, &cwd, 100, &mut emit, &|| {
            worker_cancelled.load(Ordering::Relaxed)
        });
        let status = if worker_cancelled.load(Ordering::Relaxed) {
            130
        } else if let Err(error) = result {
            let _ = sender.send(SearchTaskEvent::Error(format!(
                "{worker_command}: {error}\n"
            )));
            1
        } else {
            0
        };
        let _ = sender.send(SearchTaskEvent::Finished { status });
    });
    RunningSearch {
        command,
        receiver,
        cancelled,
        started: Instant::now(),
        results: 0,
    }
}

fn update_running_search(
    running: &mut Option<RunningSearch>,
    state: &mut TuiState,
    last_status: &mut i32,
) {
    let mut finished = None;
    if let Some(task) = running.as_mut() {
        loop {
            match task.receiver.try_recv() {
                Ok(SearchTaskEvent::Candidate(candidate)) => {
                    task.results += 1;
                    let text = if task.command == "rg" {
                        format!("{}: {}\n", candidate.label, candidate.detail)
                    } else {
                        format!("{}\n", candidate.label)
                    };
                    state.output.push_text(&text, false);
                }
                Ok(SearchTaskEvent::Error(error)) => state.output.push_text(&error, true),
                Ok(SearchTaskEvent::Finished { status }) => {
                    finished = Some(status);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    state
                        .output
                        .push_text("search worker stopped unexpectedly\n", true);
                    finished = Some(1);
                    break;
                }
            }
        }
    }

    if let Some(status) = finished {
        let task = running.take().expect("running search");
        let elapsed = task.started.elapsed();
        *last_status = status;
        state.mode = AppMode::Editing;
        let outcome = match status {
            0 => "finished",
            130 => "cancelled",
            _ => "failed",
        };
        state.status = format!(
            "{} {} · {} results · {:.1}s",
            task.command,
            outcome,
            task.results,
            elapsed.as_secs_f32()
        );
        state.output.push_divider();
    }
}

fn start_deep_completion(
    token: String,
    root: PathBuf,
    query: String,
    excluded_directories: Vec<String>,
) -> RunningDeepCompletion {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    std::thread::spawn(move || {
        let mut search = FffSearch::new(excluded_directories);
        let mut candidates: Vec<(i32, SearchCandidate)> = Vec::new();
        let result = search.search_stream(
            SearchKind::Directories,
            &query,
            &root,
            usize::MAX,
            &mut |mut candidate| {
                let score = fuzzy_score(&candidate.label, &query).unwrap_or_default();
                let relative = candidate.value.trim_end_matches(['\\', '/']);
                let mut value = display_completion_path(&root.join(relative));
                value.push('\\');
                candidate.label = value.clone();
                candidate.value = value;
                candidates.push((score, candidate));
                if candidates.len() > 512 {
                    candidates.sort_by(|left, right| {
                        right.0.cmp(&left.0).then_with(|| {
                            left.1
                                .label
                                .to_lowercase()
                                .cmp(&right.1.label.to_lowercase())
                        })
                    });
                    candidates.truncate(256);
                }
            },
            &|| worker_cancelled.load(Ordering::Relaxed),
        );
        let outcome = result.map(|()| {
            candidates.sort_by(|left, right| {
                right.0.cmp(&left.0).then_with(|| {
                    left.1
                        .label
                        .to_lowercase()
                        .cmp(&right.1.label.to_lowercase())
                })
            });
            candidates.truncate(256);
            candidates
                .into_iter()
                .map(|(_, candidate)| candidate)
                .collect()
        });
        if !worker_cancelled.load(Ordering::Relaxed) {
            let _ = sender.send(DeepCompletionEvent::Finished(outcome));
        }
    });
    RunningDeepCompletion {
        token,
        receiver,
        cancelled,
        started: Instant::now(),
    }
}

fn update_deep_completion(running: &mut Option<RunningDeepCompletion>, state: &mut TuiState) {
    let event = running
        .as_ref()
        .and_then(|task| task.receiver.try_recv().ok());
    let Some(DeepCompletionEvent::Finished(result)) = event else {
        return;
    };
    let task = running.take().expect("running deep completion");
    if state.completion_source != CompletionSource::DeepPath
        || current_completion_token(state) != task.token
    {
        return;
    }
    match result {
        Ok(candidates) => {
            state.completion = candidates
                .into_iter()
                .map(|candidate| (candidate.value, "recursive directory".to_owned()))
                .collect();
            state.selected = 0;
            state.status = if state.completion.is_empty() {
                "no matching directories".into()
            } else {
                format!("{} recursive matches", state.completion.len())
            };
        }
        Err(error) => {
            state.completion.clear();
            state.status = format!("recursive search failed: {error}");
        }
    }
}

fn cancel_deep_completion(running: &mut Option<RunningDeepCompletion>) {
    if let Some(task) = running.take() {
        task.cancelled.store(true, Ordering::Relaxed);
    }
}

fn current_completion_token(state: &TuiState) -> String {
    let before_cursor = &state.editor.text[..state.editor.cursor];
    let token_start = before_cursor
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0);
    before_cursor[token_start..].to_owned()
}

fn executable_command_name(path: &Path) -> Option<String> {
    if !is_supported_executable(path) {
        return None;
    }
    let file_name = path.file_name()?.to_string_lossy();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if matches!(
        extension.to_ascii_lowercase().as_str(),
        "exe" | "com" | "cmd" | "bat" | "ps1"
    ) {
        path.file_stem()
            .map(|name| name.to_string_lossy().into_owned())
    } else {
        Some(file_name.into_owned())
    }
}

fn command_candidates_in_directory(
    directory: &Path,
    source: &str,
    rank: i64,
    query: &str,
) -> Vec<(i64, String, String)> {
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = executable_command_name(&entry.path())?;
            let score = fuzzy_score(&name, query)?;
            Some((rank + score as i64, name, format!("{source} executable")))
        })
        .collect()
}

fn discover_path_commands() -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut commands = Vec::new();
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for (_, name, detail) in command_candidates_in_directory(&directory, "PATH", 0, "") {
            if seen.insert(name.to_ascii_lowercase()) {
                commands.push((name, detail));
            }
        }
    }
    commands.sort_by(|left, right| {
        left.0
            .to_ascii_lowercase()
            .cmp(&right.0.to_ascii_lowercase())
    });
    commands
}

struct NativeCompletionRequest {
    key: String,
    path: PathBuf,
    args: Vec<String>,
    environment_variable: String,
    cwd: PathBuf,
}

fn native_completion_request(
    shell: &ShellState,
    before_cursor: &str,
    aliases: &Aliases,
    providers: &HashMap<String, String>,
) -> Option<NativeCompletionRequest> {
    let mut args = parser::parse_with_aliases(before_cursor, aliases).ok()?;
    let command = args.first()?.to_ascii_lowercase();
    let environment_variable = providers.get(&command)?.clone();
    let path = resolve(&args[0], &shell.cwd)?;
    if before_cursor
        .chars()
        .last()
        .is_some_and(char::is_whitespace)
    {
        args.push(String::new());
    }
    Some(NativeCompletionRequest {
        key: before_cursor.to_owned(),
        path,
        args,
        environment_variable,
        cwd: shell.cwd.clone(),
    })
}

fn start_native_completion(request: NativeCompletionRequest) -> RunningNativeCompletion {
    const TIMEOUT: Duration = Duration::from_millis(500);
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let key = request.key.clone();
    std::thread::spawn(move || {
        let mut invocation_args = Vec::with_capacity(request.args.len() + 1);
        invocation_args.push("--".to_owned());
        invocation_args.extend(request.args);
        let result = (|| -> Result<Vec<(String, String)>, String> {
            let job = ProcessJob::kill_on_close().ok();
            let mut child = build_command(&request.path, &invocation_args, &request.cwd)
                .env(&request.environment_variable, "powershell")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())?;
            if let Some(job) = job.as_ref() {
                let _ = job.assign(&child);
            }
            let mut stdout = child.stdout.take().expect("piped completion stdout");
            let reader = std::thread::spawn(move || {
                let mut bytes = Vec::new();
                stdout.read_to_end(&mut bytes).map(|_| bytes)
            });
            let started = Instant::now();
            loop {
                if worker_cancelled.load(Ordering::Relaxed) || started.elapsed() >= TIMEOUT {
                    let _ = terminate_process_tree(&mut child);
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(if worker_cancelled.load(Ordering::Relaxed) {
                        "cancelled".to_owned()
                    } else {
                        "timed out after 500 ms".to_owned()
                    });
                }
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    Err(error) => return Err(error.to_string()),
                }
            }
            let status = child.wait().map_err(|error| error.to_string())?;
            let output = reader
                .join()
                .map_err(|_| "completion output reader panicked".to_owned())?
                .map_err(|error| error.to_string())?;
            if !status.success() {
                return Err(format!("provider exited with {status}"));
            }
            Ok(parse_native_completion_output(&output))
        })();
        if !worker_cancelled.load(Ordering::Relaxed) {
            let _ = sender.send(NativeCompletionEvent::Finished(result));
        }
    });
    RunningNativeCompletion {
        request: key,
        receiver,
        cancelled,
        started: Instant::now(),
    }
}

fn parse_native_completion_output(output: &[u8]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| {
            let (value, detail) = line.split_once('\t').unwrap_or((line, "command value"));
            (!value.is_empty() && seen.insert(value.to_owned()))
                .then(|| (value.to_owned(), detail.to_owned()))
        })
        .collect()
}

fn update_native_completion(running: &mut Option<RunningNativeCompletion>, state: &mut TuiState) {
    let event = match running.as_ref().map(|task| task.receiver.try_recv()) {
        Some(Ok(event)) => event,
        Some(Err(TryRecvError::Disconnected)) => {
            running.take();
            state.completion.clear();
            state.status = "command completion worker stopped unexpectedly".into();
            return;
        }
        Some(Err(TryRecvError::Empty)) | None => return,
    };
    let NativeCompletionEvent::Finished(result) = event;
    let task = running.take().expect("running native completion");
    let before_cursor = &state.editor.text[..state.editor.cursor];
    if state.completion_source != CompletionSource::Native || before_cursor != task.request {
        return;
    }
    match result {
        Ok(candidates) => {
            state.completion = candidates;
            state.selected = 0;
            state.status = if state.completion.is_empty() {
                "no command completions".into()
            } else {
                format!("{} command completions", state.completion.len())
            };
        }
        Err(error) if error == "cancelled" => {}
        Err(error) => {
            state.completion.clear();
            state.status = format!("command completion failed: {error}");
        }
    }
}

fn cancel_native_completion(running: &mut Option<RunningNativeCompletion>) {
    if let Some(task) = running.take() {
        task.cancelled.store(true, Ordering::Relaxed);
    }
}

fn complete(shell: &ShellState, state: &mut TuiState) {
    let mut deep_completion = None;
    let mut native_completion = None;
    let commands = InteractiveCommands {
        resolver: WindowsCommandResolver,
        aliases: Aliases::new(),
        path_commands: Vec::new(),
        completion_providers: HashMap::new(),
    };
    complete_with_sources(
        shell,
        state,
        &commands,
        None,
        &mut deep_completion,
        &mut native_completion,
        &[],
    );
}

fn complete_with_sources(
    shell: &ShellState,
    state: &mut TuiState,
    commands: &InteractiveCommands,
    directory_db: Option<&DirectoryDb>,
    deep_completion: &mut Option<RunningDeepCompletion>,
    native_completion: &mut Option<RunningNativeCompletion>,
    excluded_directories: &[String],
) {
    state.mode = AppMode::Completion;
    state.completion_source = CompletionSource::Path;
    state.completion_query.clear();
    let before_cursor = &state.editor.text[..state.editor.cursor];
    let token_start = before_cursor
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0);
    let token = before_cursor[token_start..].to_owned();
    let command = before_cursor
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let directories_only = command == "cd";
    let accepts_current_directory = matches!(command.as_str(), "cd" | "ls");
    let completing_command = token_start == 0;
    if directories_only && token.contains('*') {
        cancel_native_completion(native_completion);
        state.completion_source = CompletionSource::DeepPath;
        state.completion.clear();
        state.selected = 0;
        if deep_completion.as_ref().map(|task| task.token.as_str()) != Some(token.as_str()) {
            cancel_deep_completion(deep_completion);
            let (root, query) = deep_search_request(shell, &token);
            *deep_completion = Some(start_deep_completion(
                token,
                root,
                query,
                excluded_directories.to_vec(),
            ));
        }
        state.status = "searching directories…".into();
        return;
    }
    cancel_deep_completion(deep_completion);

    if !completing_command {
        if let Some(request) = native_completion_request(
            shell,
            before_cursor,
            &commands.aliases,
            &commands.completion_providers,
        ) {
            state.completion_source = CompletionSource::Native;
            state.completion.clear();
            state.selected = 0;
            if native_completion.as_ref().map(|task| task.request.as_str())
                != Some(request.key.as_str())
            {
                cancel_native_completion(native_completion);
                *native_completion = Some(start_native_completion(request));
            }
            state.status = "asking command for completions…".into();
            return;
        }
    }
    cancel_native_completion(native_completion);
    let mut values: Vec<(i64, String, String)> = Vec::new();

    let command_is_path = token.contains(['\\', '/']);
    if completing_command && !command_is_path {
        values.extend(commands.aliases.keys().filter_map(|name| {
            fuzzy_score(name, &token)
                .map(|score| (5_000_000 + score as i64, name.clone(), "alias".to_owned()))
        }));
        values.extend(NAMES.iter().filter_map(|name| {
            fuzzy_score(name, &token).map(|score| {
                (
                    4_000_000 + score as i64,
                    (*name).to_owned(),
                    "builtin".to_owned(),
                )
            })
        }));
        values.extend(TRANSLATED_NAMES.iter().filter_map(|name| {
            fuzzy_score(name, &token).map(|score| {
                (
                    3_000_000 + score as i64,
                    (*name).to_owned(),
                    "windows translation".to_owned(),
                )
            })
        }));
        values.extend(command_candidates_in_directory(
            &shell.cwd, "cwd", 2_000_000, &token,
        ));
        values.extend(commands.path_commands.iter().filter_map(|(name, detail)| {
            fuzzy_score(name, &token)
                .map(|score| (1_000_000 + score as i64, name.clone(), detail.clone()))
        }));
    } else {
        let expandable_root = matches!(token.as_str(), "~" | "." | "..")
            || environment_prefix(&token).is_some_and(|(_, consumed)| consumed == token.len());
        let separator = token.rfind(['\\', '/']);
        let (directory_part, query, explicit_current) = if expandable_root {
            (format!("{token}\\"), "", Some(token.to_owned()))
        } else {
            match separator {
                Some(index) => (token[..=index].to_owned(), &token[index + 1..], None),
                None => (String::new(), token.as_str(), None),
            }
        };
        #[cfg(windows)]
        let directory_part = directory_part.replace('/', "\\");
        let directory = expand_completion_directory(shell, &directory_part);

        if accepts_current_directory && query.is_empty() {
            let current = explicit_current.unwrap_or_else(|| {
                if directory_part.is_empty() {
                    ".".to_owned()
                } else {
                    current_directory_candidate(&directory_part)
                }
            });
            values.push((i64::MAX, current, "current directory".to_owned()));
        }

        let mut local_paths = std::collections::HashSet::new();
        if let Ok(entries) = std::fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let is_directory = entry.path().is_dir();
                if directories_only && !is_directory {
                    continue;
                }
                if completing_command && !is_directory && !is_supported_executable(&entry.path()) {
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
                } else if completing_command {
                    "executable"
                } else {
                    "file"
                };
                local_paths.insert(entry.path().to_string_lossy().to_lowercase());
                values.push((2_000_000 + score as i64, value, detail.to_owned()));
            }
        }

        if directories_only && !query.is_empty() {
            if let Some(directory_db) = directory_db {
                let now = now_epoch();
                for entry in directory_db.entries() {
                    let key = entry.path.to_string_lossy().to_lowercase();
                    if key == shell.cwd.to_string_lossy().to_lowercase()
                        || local_paths.contains(&key)
                    {
                        continue;
                    }
                    let basename = entry
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();
                    let (class, score) = if let Some(score) = fuzzy_score(basename, query) {
                        (1_000_000_i64, score)
                    } else if let Some(score) = fuzzy_score(&key, query) {
                        (500_000_i64, score)
                    } else {
                        continue;
                    };
                    let mut value = display_completion_path(&entry.path);
                    value.push('\\');
                    let rank = frecency(entry, now).min(10_000.0) as i64;
                    values.push((class + score as i64 * 100 + rank, value, "visited".into()));
                }
            }
        }
    }

    values.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.to_lowercase().cmp(&right.1.to_lowercase()))
    });
    let mut seen = HashSet::new();
    state.completion = values
        .into_iter()
        .filter(|(_, value, _)| seen.insert(value.to_ascii_lowercase()))
        .map(|(_, value, detail)| (value, detail))
        .collect();
    state.selected = 0;
}

fn display_completion_path(path: &Path) -> String {
    let displayed = path.to_string_lossy();
    displayed
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| displayed.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| displayed.into_owned())
}

fn deep_search_request(shell: &ShellState, token: &str) -> (PathBuf, String) {
    let star = token.find('*').expect("deep search token");
    let before = &token[..star];
    let after = &token[star + 1..];
    let separator = before.rfind(['\\', '/']);
    let (root_text, query_prefix) = match separator {
        Some(index) => (&before[..=index], &before[index + 1..]),
        None => ("", before),
    };
    let root = expand_completion_directory(shell, root_text);
    let query = format!("{query_prefix}{after}")
        .trim_matches(['\\', '/'])
        .to_owned();
    (root, query)
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
        "cd" | "ls" | "mkdir" | "rm" | "touch" | "cat" | "tail" | "less" | "find" | "rg"
    )
}

fn copy_transcript_selection(state: &mut TuiState, prompt: &str, width: usize) {
    let Some(text) = selected_transcript_text(state, prompt, width) else {
        state.status = "nothing selected".into();
        return;
    };
    copy_selection_text(state, &text);
}

fn copy_pager_selection(state: &mut TuiState, width: usize) {
    let Some(text) = selected_pager_text(state, width) else {
        state.status = "nothing selected".into();
        return;
    };
    copy_selection_text(state, &text);
}

fn copy_selection_text(state: &mut TuiState, text: &str) {
    match set_clipboard_text(text) {
        Ok(()) => {
            let characters = text.chars().count();
            state.status = format!("copied {characters} characters");
        }
        Err(error) => state.status = format!("cannot copy selection: {error}"),
    }
}

fn paste_clipboard(state: &mut TuiState) {
    match get_clipboard_text() {
        Ok(Some(text)) => {
            let text = normalize_pasted_text(&text);
            if text.is_empty() {
                state.status = "clipboard has no pasteable text".into();
                return;
            }
            let characters = text.chars().count();
            state.editor.insert_str(&text);
            state.status = format!("pasted {characters} characters");
        }
        Ok(None) => state.status = "clipboard does not contain text".into(),
        Err(error) => state.status = format!("cannot paste clipboard: {error}"),
    }
}

fn normalize_pasted_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_was_carriage_return = false;
    for character in text.chars() {
        match character {
            '\r' => {
                normalized.push(' ');
                previous_was_carriage_return = true;
            }
            '\n' if previous_was_carriage_return => previous_was_carriage_return = false,
            '\n' | '\t' => {
                normalized.push(' ');
                previous_was_carriage_return = false;
            }
            character if character.is_control() => previous_was_carriage_return = false,
            character => {
                normalized.push(character);
                previous_was_carriage_return = false;
            }
        }
    }
    normalized
}

fn scroll_transcript_up(state: &mut TuiState, amount: usize, visible_lines: usize) {
    let trailing_lines = usize::from(state.mode != AppMode::RunningTask) * 2;
    if state
        .output
        .scroll_up(amount, visible_lines, trailing_lines)
    {
        state.flash_scroll_top();
    }
}

fn scroll_transcript_down(state: &mut TuiState, amount: usize, visible_lines: usize) {
    let trailing_lines = usize::from(state.mode != AppMode::RunningTask) * 2;
    if state
        .output
        .scroll_down(amount, visible_lines, trailing_lines)
    {
        state.flash_scroll_bottom();
    }
}

fn scroll_pager_up(state: &mut TuiState, amount: usize, visible_lines: usize, width: usize) {
    let maximum = state
        .pager_visual_line_count(width)
        .saturating_sub(visible_lines);
    let Some(pager) = state.pager.as_mut() else {
        return;
    };
    pager.top = pager.top.saturating_sub(amount).min(maximum);
    let at_top = pager.top == 0;
    if at_top {
        state.flash_scroll_top();
    }
}

fn scroll_pager_down(state: &mut TuiState, amount: usize, visible_lines: usize, width: usize) {
    let maximum = state
        .pager_visual_line_count(width)
        .saturating_sub(visible_lines);
    let Some(pager) = state.pager.as_mut() else {
        return;
    };
    pager.top = pager.top.saturating_add(amount).min(maximum);
    let at_bottom = pager.top == maximum;
    if at_bottom {
        state.flash_scroll_bottom();
    }
}

fn handle_pager_key(state: &mut TuiState, key: KeyEvent, visible_lines: usize, width: usize) {
    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
        state.clear_pager_selection();
        state.pager = None;
        state.mode = AppMode::Editing;
        return;
    }

    if state.pager.is_none() {
        return;
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => scroll_pager_down(state, 1, visible_lines, width),
        KeyCode::Up | KeyCode::Char('k') => scroll_pager_up(state, 1, visible_lines, width),
        KeyCode::PageDown | KeyCode::Char(' ') => scroll_pager_down(
            state,
            visible_lines.saturating_sub(1).max(1),
            visible_lines,
            width,
        ),
        KeyCode::PageUp => scroll_pager_up(
            state,
            visible_lines.saturating_sub(1).max(1),
            visible_lines,
            width,
        ),
        KeyCode::Char('g') => scroll_pager_up(state, usize::MAX, visible_lines, width),
        KeyCode::Char('G') => scroll_pager_down(state, usize::MAX, visible_lines, width),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_commands() -> InteractiveCommands {
        InteractiveCommands {
            resolver: WindowsCommandResolver,
            aliases: Aliases::new(),
            path_commands: Vec::new(),
            completion_providers: HashMap::new(),
        }
    }

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

    #[test]
    fn explicit_current_directory_completes_executable_commands() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("tools")).unwrap();
        std::fs::write(root.path().join("runner.exe"), "executable").unwrap();
        std::fs::write(root.path().join("notes.txt"), "not executable").unwrap();
        let shell = ShellState::new(root.path().to_owned());
        let mut state = TuiState::new(100, 4096);
        state.editor.set("./".to_owned());

        complete(&shell, &mut state);

        assert!(state
            .completion
            .iter()
            .any(|(value, detail)| value == ".\\runner.exe" && detail == "executable"));
        assert!(state
            .completion
            .iter()
            .any(|(value, detail)| value == ".\\tools\\" && detail == "directory"));
        assert!(!state
            .completion
            .iter()
            .any(|(value, _)| value.contains("notes.txt")));
    }

    #[test]
    fn terminal_programs_are_kept_out_of_captured_mode() {
        assert!(requires_terminal(
            Path::new(r"C:\tools\nvim.exe"),
            &["nvim".to_owned()]
        ));
        assert!(requires_terminal(
            Path::new(r"C:\tools\ssh.exe"),
            &["ssh".to_owned(), "host".to_owned()]
        ));
        assert!(!requires_terminal(
            Path::new(r"C:\tools\just.exe"),
            &["just".to_owned(), "release".to_owned()]
        ));
    }

    #[test]
    fn visited_directories_are_added_to_cd_completion_below_local_matches() {
        let root = tempfile::tempdir().unwrap();
        let local = root.path().join("liteshell-local");
        let visited = root.path().join("works").join("liteshell-visited");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::create_dir_all(&visited).unwrap();
        let shell = ShellState::new(root.path().to_owned());
        let mut db = DirectoryDb::new(root.path().join("directories.db"));
        db.record(&visited);
        let mut state = TuiState::new(100, 4096);
        state.editor.set("cd lite".to_owned());
        let mut deep = None;

        let mut native = None;
        complete_with_sources(
            &shell,
            &mut state,
            &test_commands(),
            Some(&db),
            &mut deep,
            &mut native,
            &[],
        );

        assert!(state.completion[0].0.starts_with("liteshell-local"));
        assert!(state
            .completion
            .iter()
            .any(|(value, detail)| value.contains("liteshell-visited") && detail == "visited"));
    }

    #[test]
    fn star_splits_recursive_root_from_fuzzy_query() {
        let shell = ShellState::new(PathBuf::from(r"C:\home"));
        let (root, query) = deep_search_request(&shell, r"works\*lite");
        assert_eq!(root, PathBuf::from(r"C:\home\works"));
        assert_eq!(query, "lite");

        let (root, query) = deep_search_request(&shell, "lite*");
        assert_eq!(root, shell.cwd);
        assert_eq!(query, "lite");
    }

    #[test]
    fn external_output_preserves_stream_kind() {
        let (sender, receiver) = mpsc::channel();
        stream_external_output("first\nsecond".as_bytes(), true, sender);

        let events: Vec<_> = receiver.into_iter().collect();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ExternalTaskEvent::Output { text, error: true } if text == "first\n"
        ));
        assert!(matches!(
            &events[1],
            ExternalTaskEvent::Output { text, error: true } if text == "second"
        ));
    }

    #[test]
    fn empty_command_completion_includes_aliases_cwd_and_path_commands() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("local-tool.exe"), "executable").unwrap();
        let shell = ShellState::new(root.path().to_owned());
        let mut state = TuiState::new(100, 4096);
        let mut commands = test_commands();
        commands
            .aliases
            .insert("work".to_owned(), "just".to_owned());
        commands
            .path_commands
            .push(("just".to_owned(), "PATH executable".to_owned()));
        let mut deep = None;
        let mut native = None;

        complete_with_sources(
            &shell,
            &mut state,
            &commands,
            None,
            &mut deep,
            &mut native,
            &[],
        );

        assert!(state
            .completion
            .iter()
            .any(|candidate| candidate == &("work".to_owned(), "alias".to_owned())));
        assert!(state
            .completion
            .iter()
            .any(|candidate| candidate == &("local-tool".to_owned(), "cwd executable".to_owned())));
        assert!(state
            .completion
            .iter()
            .any(|candidate| candidate == &("just".to_owned(), "PATH executable".to_owned())));
    }

    #[test]
    fn native_completion_output_supports_values_with_and_without_help() {
        let candidates = parse_native_completion_output(
            b"build\r\n--justfile\tUse a different justfile\r\nbuild\r\n",
        );
        assert_eq!(
            candidates,
            [
                ("build".to_owned(), "command value".to_owned()),
                (
                    "--justfile".to_owned(),
                    "Use a different justfile".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn pasted_text_is_normalized_to_one_safe_command_line() {
        assert_eq!(
            normalize_pasted_text("cargo test\r\njust check\n中文\tvalue\u{1b}"),
            "cargo test just check 中文 value"
        );
    }

    #[test]
    fn native_completion_request_expands_alias_and_preserves_empty_argument() {
        let cwd = std::env::current_dir().unwrap();
        let command_path = resolve("cmd", &cwd).unwrap();
        let shell = ShellState::new(cwd);
        let aliases = Aliases::from([("c".to_owned(), "cmd /d".to_owned())]);
        let providers = HashMap::from([("cmd".to_owned(), "CMD_COMPLETE".to_owned())]);

        let request = native_completion_request(&shell, "c ", &aliases, &providers).unwrap();

        assert_eq!(request.path, command_path);
        assert_eq!(request.args, ["cmd", "/d", ""]);
        assert_eq!(request.environment_variable, "CMD_COMPLETE");
    }

    #[test]
    fn native_completion_provider_is_called_as_a_bounded_tool() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("completion-provider.cmd");
        std::fs::write(
            &script,
            "@echo off\r\nif \"%TEST_COMPLETE%\"==\"powershell\" (\r\n  echo build\r\n  echo --flag\tFlag help\r\n)\r\n",
        )
        .unwrap();
        let running = start_native_completion(NativeCompletionRequest {
            key: "provider ".to_owned(),
            path: script,
            args: vec!["provider".to_owned(), String::new()],
            environment_variable: "TEST_COMPLETE".to_owned(),
            cwd: root.path().to_owned(),
        });

        let event = running
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let NativeCompletionEvent::Finished(result) = event;
        assert_eq!(
            result.unwrap(),
            [
                ("build".to_owned(), "command value".to_owned()),
                ("--flag".to_owned(), "Flag help".to_owned()),
            ]
        );
    }

    #[test]
    fn completed_external_status_stays_out_of_scrollback() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(ExternalTaskEvent::Finished { status: 7 })
            .unwrap();
        let mut running = Some(RunningExternal {
            command: "example".to_owned(),
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
        });
        let mut state = TuiState::new(100, 4096);
        state.mode = AppMode::RunningTask;
        let mut last_status = 0;

        update_running_external(&mut running, &mut state, &mut last_status);

        assert_eq!(last_status, 7);
        assert!(running.is_none());
        assert_eq!(state.mode, AppMode::Editing);
        let lines: Vec<_> = state.output.lines().collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].divider);
        assert!(!lines[0].text.contains("status 7"));
    }

    #[test]
    fn pager_boundaries_keep_a_full_page_and_trigger_feedback() {
        let mut state = TuiState::new(100, 4096);
        state.apply(vec![OutputEvent::Pager {
            title: "test".to_owned(),
            lines: (0..20)
                .map(|index| liteshell_core::StyledLine::plain(index.to_string()))
                .collect(),
        }]);

        scroll_pager_down(&mut state, usize::MAX, 5, 80);
        assert_eq!(state.pager.as_ref().unwrap().top, 15);
        assert!(state.scroll_flash_active());

        scroll_pager_up(&mut state, usize::MAX, 5, 80);
        assert_eq!(state.pager.as_ref().unwrap().top, 0);
        assert!(state.scroll_flash_active());
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
