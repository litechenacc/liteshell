use liteshell_builtins::{dispatch, handles, Context};
use liteshell_core::{
    config::Config,
    parser::{Command, CommandLine, Connector, Pipeline, Redirection},
    OutputEvent, OutputSink, ShellState,
};
use liteshell_fff::FffSearch;
use liteshell_windows::{build_command, resolve, translate, ProcessJob, WindowsCommandResolver};
use os_pipe::{PipeReader, PipeWriter};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    process::{Child, Stdio},
    thread::{self, JoinHandle},
};

enum InputTarget {
    Inherit,
    Empty,
    Pipe(PipeReader),
    File(File),
}

enum OutputTarget {
    Stdout,
    Stderr,
    Pipe(PipeWriter),
    File(File),
}

impl OutputTarget {
    fn try_clone(&self) -> io::Result<Self> {
        match self {
            Self::Stdout => Ok(Self::Stdout),
            Self::Stderr => Ok(Self::Stderr),
            Self::Pipe(pipe) => Ok(Self::Pipe(pipe.try_clone()?)),
            Self::File(file) => Ok(Self::File(file.try_clone()?)),
        }
    }

    fn into_stdio(self) -> Stdio {
        match self {
            Self::Stdout | Self::Stderr => Stdio::inherit(),
            Self::Pipe(pipe) => Stdio::from(pipe),
            Self::File(file) => Stdio::from(file),
        }
    }

    fn into_writer(self) -> Box<dyn Write + Send> {
        match self {
            Self::Stdout => Box::new(io::stdout()),
            Self::Stderr => Box::new(io::stderr()),
            Self::Pipe(pipe) => Box::new(pipe),
            Self::File(file) => Box::new(file),
        }
    }

    fn message(&mut self, message: &str) {
        match self {
            Self::Stdout => {
                let _ = io::stdout().write_all(message.as_bytes());
            }
            Self::Stderr => {
                let _ = io::stderr().write_all(message.as_bytes());
            }
            Self::Pipe(pipe) => {
                let _ = pipe.write_all(message.as_bytes());
            }
            Self::File(file) => {
                let _ = file.write_all(message.as_bytes());
            }
        }
    }
}

impl InputTarget {
    fn into_stdio(self) -> Stdio {
        match self {
            Self::Inherit => Stdio::inherit(),
            Self::Empty => Stdio::null(),
            Self::Pipe(pipe) => Stdio::from(pipe),
            Self::File(file) => Stdio::from(file),
        }
    }

    fn into_reader(self) -> Box<dyn Read + Send> {
        match self {
            Self::Inherit => Box::new(io::stdin()),
            Self::Empty => Box::new(io::empty()),
            Self::Pipe(pipe) => Box::new(pipe),
            Self::File(file) => Box::new(file),
        }
    }
}

struct StreamOutput {
    stdout: Box<dyn Write + Send>,
    stderr: Box<dyn Write + Send>,
}

impl OutputSink for StreamOutput {
    fn emit(&mut self, event: OutputEvent) {
        match event {
            OutputEvent::Text(text) => write_all(&mut self.stdout, text.as_bytes()),
            OutputEvent::Styled(text) => write_all(&mut self.stdout, text.text().as_bytes()),
            OutputEvent::Error(text) => write_all(&mut self.stderr, text.as_bytes()),
            OutputEvent::Pager { lines, .. } => {
                let mut text = lines
                    .iter()
                    .map(|line| line.text())
                    .collect::<Vec<_>>()
                    .join("\n");
                text.push('\n');
                write_all(&mut self.stdout, text.as_bytes());
            }
            OutputEvent::Clear | OutputEvent::Status(_) => {}
        }
    }

    fn write_stdout(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stdout.write_all(bytes)?;
        self.stdout.flush()
    }
}

fn write_all(writer: &mut Box<dyn Write + Send>, bytes: &[u8]) {
    let _ = writer.write_all(bytes);
    let _ = writer.flush();
}

enum RunningStage {
    Child(Child),
    Builtin(JoinHandle<i32>),
    Immediate(i32),
}

impl RunningStage {
    fn wait(self) -> i32 {
        match self {
            Self::Child(mut child) => child
                .wait()
                .ok()
                .and_then(|status| status.code())
                .unwrap_or(1),
            Self::Builtin(thread) => thread.join().unwrap_or(1),
            Self::Immediate(status) => status,
        }
    }
}

pub fn execute(command_line: &CommandLine, shell: &mut ShellState, pipefail: bool) -> i32 {
    execute_with_stdio(
        command_line,
        shell,
        pipefail,
        true,
        OutputTarget::Stdout,
        OutputTarget::Stderr,
    )
}

pub struct CapturedExecution {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Execute through the same pipeline engine while draining both final streams
/// concurrently, so interactive TUI output can be restored to scrollback
/// without risking a full-pipe deadlock.
pub fn execute_captured(
    command_line: &CommandLine,
    shell: &mut ShellState,
    pipefail: bool,
) -> io::Result<CapturedExecution> {
    let (mut stdout_reader, stdout_writer) = os_pipe::pipe()?;
    let (mut stderr_reader, stderr_writer) = os_pipe::pipe()?;
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout_reader.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr_reader.read_to_end(&mut bytes);
        bytes
    });
    let status = execute_with_stdio(
        command_line,
        shell,
        pipefail,
        false,
        OutputTarget::Pipe(stdout_writer),
        OutputTarget::Pipe(stderr_writer),
    );
    Ok(CapturedExecution {
        status,
        stdout: stdout_thread.join().unwrap_or_default(),
        stderr: stderr_thread.join().unwrap_or_default(),
    })
}

fn execute_with_stdio(
    command_line: &CommandLine,
    shell: &mut ShellState,
    pipefail: bool,
    inherit_input: bool,
    stdout: OutputTarget,
    stderr: OutputTarget,
) -> i32 {
    // A terminal-tool timeout commonly kills only the shell process. Keeping
    // every external stage in one kill-on-close Job Object prevents orphaned
    // grandchildren when that happens.
    let job = ProcessJob::kill_on_close().ok();
    let mut status = 0;
    for (connector, pipeline) in &command_line.pipelines {
        let run = match connector {
            Connector::Always => true,
            Connector::And => status == 0,
            Connector::Or => status != 0,
        };
        if run {
            let pipeline_stdout = match stdout.try_clone() {
                Ok(output) => output,
                Err(error) => {
                    eprintln!("liteshell: cannot duplicate stdout: {error}");
                    return 126;
                }
            };
            let pipeline_stderr = match stderr.try_clone() {
                Ok(output) => output,
                Err(error) => {
                    eprintln!("liteshell: cannot duplicate stderr: {error}");
                    return 126;
                }
            };
            status = execute_pipeline(
                pipeline,
                shell,
                pipefail,
                job.as_ref(),
                if inherit_input {
                    InputTarget::Inherit
                } else {
                    InputTarget::Empty
                },
                pipeline_stdout,
                pipeline_stderr,
            );
            shell.last_status = status;
            if !shell.running {
                break;
            }
        }
    }
    status
}

fn execute_pipeline(
    pipeline: &Pipeline,
    shell: &mut ShellState,
    pipefail: bool,
    job: Option<&ProcessJob>,
    input: InputTarget,
    output: OutputTarget,
    error: OutputTarget,
) -> i32 {
    if pipeline.commands.len() == 1 {
        return execute_single(&pipeline.commands[0], shell, job, input, output, error);
    }

    let count = pipeline.commands.len();
    let mut inputs: Vec<Option<InputTarget>> = (0..count).map(|_| None).collect();
    let mut outputs: Vec<Option<OutputTarget>> = (0..count).map(|_| None).collect();
    inputs[0] = Some(input);
    outputs[count - 1] = Some(output);
    for index in 0..count - 1 {
        match os_pipe::pipe() {
            Ok((reader, writer)) => {
                outputs[index] = Some(OutputTarget::Pipe(writer));
                inputs[index + 1] = Some(InputTarget::Pipe(reader));
            }
            Err(error) => {
                eprintln!("liteshell: cannot create pipe: {error}");
                return 126;
            }
        }
    }

    let config = Config::default();
    let mut stages = Vec::with_capacity(count);
    for (index, command) in pipeline.commands.iter().enumerate() {
        let input = inputs[index].take().expect("pipeline input");
        let output = outputs[index].take().expect("pipeline output");
        let shell = ShellState {
            cwd: shell.cwd.clone(),
            running: true,
            last_status: shell.last_status,
        };
        let stage_error = match error.try_clone() {
            Ok(output) => output,
            Err(failure) => {
                eprintln!("liteshell: cannot duplicate stderr: {failure}");
                return 126;
            }
        };
        stages.push(spawn_stage(
            command.clone(),
            shell,
            input,
            output,
            stage_error,
            config.deep_search_exclude_dirs.clone(),
            job,
        ));
    }

    let statuses: Vec<i32> = stages.into_iter().map(RunningStage::wait).collect();
    if pipefail {
        statuses
            .iter()
            .rev()
            .copied()
            .find(|status| *status != 0)
            .unwrap_or(0)
    } else {
        statuses.last().copied().unwrap_or(0)
    }
}

fn execute_single(
    command: &Command,
    shell: &mut ShellState,
    job: Option<&ProcessJob>,
    input: InputTarget,
    output: OutputTarget,
    error: OutputTarget,
) -> i32 {
    let (input, output, mut error) = match endpoints(command, shell, input, output, error) {
        Ok(endpoints) => endpoints,
        Err(status) => return status,
    };

    if handles(&command.args) {
        let resolver = WindowsCommandResolver;
        let config = Config::default();
        let mut search = FffSearch::new(config.deep_search_exclude_dirs);
        let mut input = input.into_reader();
        let mut output = StreamOutput {
            stdout: output.into_writer(),
            stderr: error.into_writer(),
        };
        let mut context = Context {
            shell,
            input: &mut input,
            output: &mut output,
            search: &mut search,
            resolver: &resolver,
            interactive: false,
        };
        return dispatch(&command.args, &mut context).status;
    }

    let (path, args) = match external_invocation(&command.args, &shell.cwd) {
        Ok(invocation) => invocation,
        Err((status, message)) => {
            error.message(&format!("{message}\n"));
            return status;
        }
    };
    match build_command(&path, &args, &shell.cwd)
        .stdin(input.into_stdio())
        .stdout(output.into_stdio())
        .stderr(error.into_stdio())
        .spawn()
    {
        Ok(mut child) => {
            if let Some(job) = job {
                let _ = job.assign(&child);
            }
            child
                .wait()
                .ok()
                .and_then(|status| status.code())
                .unwrap_or(1)
        }
        Err(error) => {
            eprintln!("{}: cannot start: {error}", command.args[0]);
            126
        }
    }
}

fn spawn_stage(
    command: Command,
    mut shell: ShellState,
    input: InputTarget,
    output: OutputTarget,
    error: OutputTarget,
    excluded_directories: Vec<String>,
    job: Option<&ProcessJob>,
) -> RunningStage {
    let (input, output, mut error) = match endpoints(&command, &shell, input, output, error) {
        Ok(endpoints) => endpoints,
        Err(status) => return RunningStage::Immediate(status),
    };

    if handles(&command.args) {
        return RunningStage::Builtin(thread::spawn(move || {
            let resolver = WindowsCommandResolver;
            let mut search = FffSearch::new(excluded_directories);
            let mut input = input.into_reader();
            let mut output = StreamOutput {
                stdout: output.into_writer(),
                stderr: error.into_writer(),
            };
            let mut context = Context {
                shell: &mut shell,
                input: &mut input,
                output: &mut output,
                search: &mut search,
                resolver: &resolver,
                interactive: false,
            };
            dispatch(&command.args, &mut context).status
        }));
    }

    let (path, args) = match external_invocation(&command.args, &shell.cwd) {
        Ok(invocation) => invocation,
        Err((status, message)) => {
            error.message(&format!("{message}\n"));
            return RunningStage::Immediate(status);
        }
    };
    match build_command(&path, &args, &shell.cwd)
        .stdin(input.into_stdio())
        .stdout(output.into_stdio())
        .stderr(error.into_stdio())
        .spawn()
    {
        Ok(child) => {
            if let Some(job) = job {
                let _ = job.assign(&child);
            }
            RunningStage::Child(child)
        }
        Err(error) => {
            eprintln!("{}: cannot start: {error}", command.args[0]);
            RunningStage::Immediate(126)
        }
    }
}

fn external_invocation(
    arguments: &[String],
    cwd: &std::path::Path,
) -> Result<(std::path::PathBuf, Vec<String>), (i32, String)> {
    if let Some(translated) = translate(&arguments[0], &arguments[1..], cwd) {
        return translated
            .map(|command| (command.path, command.args))
            .map_err(|error| (2, error.to_string()));
    }
    resolve(&arguments[0], cwd)
        .map(|path| (path, arguments[1..].to_vec()))
        .ok_or_else(|| (127, format!("{}: command not found", arguments[0])))
}

fn endpoints(
    command: &Command,
    shell: &ShellState,
    mut input: InputTarget,
    mut output: OutputTarget,
    mut error: OutputTarget,
) -> Result<(InputTarget, OutputTarget, OutputTarget), i32> {
    for redirection in &command.redirections {
        match redirection {
            Redirection::Input(path) => {
                input = InputTarget::File(open_input(shell, path)?);
            }
            Redirection::Output { path, append } => {
                output = OutputTarget::File(open_output(shell, path, *append)?);
            }
            Redirection::Error { path, append } => {
                error = OutputTarget::File(open_output(shell, path, *append)?);
            }
            Redirection::ErrorToOutput => {
                error = output.try_clone().map_err(|failure| {
                    eprintln!("liteshell: cannot duplicate output: {failure}");
                    1
                })?;
            }
        }
    }
    Ok((input, output, error))
}

fn open_input(shell: &ShellState, path: &str) -> Result<File, i32> {
    let path = shell.resolve(path);
    File::open(&path).map_err(|error| {
        eprintln!("liteshell: {}: {error}", path.display());
        1
    })
}

fn open_output(shell: &ShellState, path: &str, append: bool) -> Result<File, i32> {
    let path = shell.resolve(path);
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&path)
        .map_err(|error| {
            eprintln!("liteshell: {}: {error}", path.display());
            1
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use liteshell_core::parser::{parse_command_line_with_aliases, Aliases};

    #[test]
    fn captured_pipeline_drains_both_streams_into_tui_friendly_buffers() {
        let root = tempfile::tempdir().unwrap();
        let mut shell = ShellState::new(root.path().to_owned());
        let line =
            parse_command_line_with_aliases("pwd | cat; ls missing 2>&1 | cat", &Aliases::new())
                .unwrap();
        let result = execute_captured(&line, &mut shell, true).unwrap();
        assert_eq!(result.status, 1);
        let stdout = String::from_utf8(result.stdout).unwrap();
        assert!(stdout.contains(&root.path().display().to_string()));
        assert!(stdout.contains("ls: path not found"));
        assert!(result.stderr.is_empty());
    }
}
