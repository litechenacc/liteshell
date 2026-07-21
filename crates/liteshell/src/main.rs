use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use liteshell_builtins::{dispatch, Context, NAMES};
use liteshell_core::{
    config::{directory_db_path, history_path, load_startup_environment, Config},
    directory_db::{frecency, now_epoch, DirectoryDb},
    history::History,
    parser, AppMode, OutputEvent, OutputSink, SearchCandidate, SearchKind, SearchProvider,
    ShellState,
};
use liteshell_fff::FffSearch;
use liteshell_tui::{draw, CompletionSource, EventBuffer, TerminalSession, TuiState};
use liteshell_windows::{
    launch, resolve, spawn_captured, terminate_process_tree, WindowsCommandResolver,
};
use std::{
    io::{self, BufRead, BufReader, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    time::{Duration, Instant},
};

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

#[derive(Default)]
struct RunningTasks {
    search: Option<RunningSearch>,
    external: Option<RunningExternal>,
    deep_completion: Option<RunningDeepCompletion>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("liteshell: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    load_startup_environment()?;
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

fn plain(mut shell: ShellState) -> Result<(), Box<dyn std::error::Error>> {
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
        let previous_cwd = shell.cwd.clone();
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
    Ok(())
}

fn interactive(mut shell: ShellState) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();
    let mut history = History::new(history_path(), config.history_capacity);
    let _ = history.load();
    let mut history_index = history.entries().len();
    let mut history_scratch = String::new();
    let mut search = FffSearch::new(config.deep_search_exclude_dirs.clone());
    let mut directory_db = DirectoryDb::new(directory_db_path());
    let _ = directory_db.load();
    directory_db.record(&shell.cwd);
    let resolver = WindowsCommandResolver;
    let mut state = TuiState::new(config.scrollback_lines, config.scrollback_bytes);
    let mut terminal = TerminalSession::enter()?;
    let mut running = RunningTasks::default();

    while shell.running {
        let _ = directory_db.flush_if_due(DIRECTORY_DB_FLUSH_INTERVAL);
        update_running_search(&mut running.search, &mut state, &mut shell.last_status);
        update_running_external(&mut running.external, &mut state, &mut shell.last_status);
        update_deep_completion(&mut running.deep_completion, &mut state);
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
        }

        terminal
            .terminal()
            .draw(|frame| draw(frame, &state, &shell.prompt(), shell.last_status))?;

        let wait = if running.search.is_some()
            || running.external.is_some()
            || running.deep_completion.is_some()
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
                    handle_pager_key(&mut state, key);
                    continue;
                }

                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        cancel_deep_completion(&mut running.deep_completion);
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
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &config.deep_search_exclude_dirs,
                                );
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
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &config.deep_search_exclude_dirs,
                                );
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
                                complete_with_sources(
                                    &shell,
                                    &mut state,
                                    Some(&directory_db),
                                    &mut running.deep_completion,
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
                                    Some(&directory_db),
                                    &mut running.deep_completion,
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
                                    Some(&directory_db),
                                    &mut running.deep_completion,
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
                                    Some(&directory_db),
                                    &mut running.deep_completion,
                                    &config.deep_search_exclude_dirs,
                                );
                            }
                        } else {
                            accept_completion(&shell, &mut state);
                        }
                    }
                    KeyCode::Esc => {
                        cancel_deep_completion(&mut running.deep_completion);
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
                                &resolver,
                                &mut state,
                                &mut terminal,
                                &mut running,
                            )?;
                            if shell.cwd != previous_cwd {
                                directory_db.record(&shell.cwd);
                            }
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
    directory_db.flush()?;
    Ok(())
}

fn execute_interactive(
    line: &str,
    shell: &mut ShellState,
    search: &mut FffSearch,
    resolver: &WindowsCommandResolver,
    state: &mut TuiState,
    terminal: &mut TerminalSession,
    running: &mut RunningTasks,
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

    if matches!(arguments[0].to_ascii_lowercase().as_str(), "find" | "rg") {
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
            output: &mut events,
            search,
            resolver,
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

    let status = if let Some(path) = resolve(&arguments[0], &shell.cwd) {
        if requires_terminal(&path, &arguments) {
            state.mode = AppMode::RunningChild;
            terminal.suspend(|| launch(&path, &arguments[1..], &shell.cwd))??
        } else {
            match start_external(
                arguments[0].clone(),
                path,
                arguments[1..].to_vec(),
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
    } else {
        state
            .output
            .push_text(&format!("{}: command not found\n", arguments[0]), true);
        127
    };
    shell.last_status = status;

    state.output.push_text(
        &format!("[child exited with status {status}]\n"),
        status != 0,
    );
    state.mode = AppMode::Editing;
    state.output.push_divider();
    Ok(())
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
        state.output.push_text(
            &format!("[child exited with status {status}]\n"),
            status != 0,
        );
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

fn complete(shell: &ShellState, state: &mut TuiState) {
    let mut deep_completion = None;
    complete_with_sources(shell, state, None, &mut deep_completion, &[]);
}

fn complete_with_sources(
    shell: &ShellState,
    state: &mut TuiState,
    directory_db: Option<&DirectoryDb>,
    deep_completion: &mut Option<RunningDeepCompletion>,
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
    if directories_only && token.contains('*') {
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
    let mut values: Vec<(i64, String, String)> = Vec::new();

    if token_start == 0 {
        values.extend(NAMES.iter().filter_map(|name| {
            fuzzy_score(name, &token)
                .map(|score| (score as i64, (*name).to_owned(), "builtin".to_owned()))
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
    state.completion = values
        .into_iter()
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

        complete_with_sources(&shell, &mut state, Some(&db), &mut deep, &[]);

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
