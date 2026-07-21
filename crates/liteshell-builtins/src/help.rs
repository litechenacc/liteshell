pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct CommandHelp {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub description: &'static str,
    pub arguments: &'static [(&'static str, &'static str)],
    pub options: &'static [(&'static str, &'static str)],
    pub examples: &'static [&'static str],
}

const HELP_OPTION: &[(&str, &str)] = &[("-h, --help", "Show detailed help")];

pub const COMMANDS: &[CommandHelp] = &[
    CommandHelp {
        name: "cd",
        usage: "cd [DIRECTORY]",
        summary: "Change the current directory",
        description: "Changes LiteShell's working directory. With no directory, cd uses USERPROFILE.",
        arguments: &[("DIRECTORY", "Destination path; defaults to the home directory")],
        options: HELP_OPTION,
        examples: &["cd docs", "cd ..", "cd"],
    },
    CommandHelp {
        name: "pwd",
        usage: "pwd",
        summary: "Print the current directory",
        description: "Prints the absolute working directory used to resolve relative paths.",
        arguments: &[],
        options: HELP_OPTION,
        examples: &["pwd"],
    },
    CommandHelp {
        name: "ls",
        usage: "ls [OPTIONS] [PATH]",
        summary: "List directory contents",
        description: "Lists a directory or one file. Directories are sorted before files.",
        arguments: &[("PATH", "File or directory to list; defaults to the current directory")],
        options: &[
            ("-a, --all", "Include names beginning with a dot"),
            ("-l, --long", "Show modified time, kind, and size"),
            ("-h, --help", "Show detailed help"),
            ("--", "Treat following values as paths"),
        ],
        examples: &["ls", "ls -la .", "ls README.md"],
    },
    CommandHelp {
        name: "mkdir",
        usage: "mkdir [OPTIONS] DIRECTORY...",
        summary: "Create directories",
        description: "Creates one or more directories relative to the current directory.",
        arguments: &[("DIRECTORY...", "One or more directories to create")],
        options: &[
            ("-p, --parents", "Create missing parents and accept existing directories"),
            ("-h, --help", "Show detailed help"),
            ("--", "Treat following values as directory names"),
        ],
        examples: &["mkdir output", "mkdir -p target/release/assets"],
    },
    CommandHelp {
        name: "rm",
        usage: "rm [OPTIONS] PATH...",
        summary: "Remove files or directories",
        description: "Removes files. Directory removal requires --recursive; dangerous recursive roots are rejected.",
        arguments: &[("PATH...", "One or more files or directories to remove")],
        options: &[
            ("-f, --force", "Ignore paths that do not exist"),
            ("-r, -R, --recursive", "Remove directories and their contents"),
            ("-h, --help", "Show detailed help"),
            ("--", "Treat following values as paths"),
        ],
        examples: &["rm build.log", "rm -rf target/tmp"],
    },
    CommandHelp {
        name: "touch",
        usage: "touch FILE...",
        summary: "Create files or update timestamps",
        description: "Creates missing files and updates the modified time of existing files.",
        arguments: &[("FILE...", "One or more files to create or update")],
        options: &[("-h, --help", "Show detailed help"), ("--", "Treat following values as file names")],
        examples: &["touch notes.txt", "touch one.txt two.txt"],
    },
    CommandHelp {
        name: "cat",
        usage: "cat [FILE...]",
        summary: "Print UTF-8 text",
        description: "Prints UTF-8 files with syntax highlighting. With no files, copies standard input.",
        arguments: &[("FILE...", "Text files to print; omit to read standard input")],
        options: HELP_OPTION,
        examples: &["cat README.md", "rg query | cat"],
    },
    CommandHelp {
        name: "tail",
        usage: "tail [OPTIONS] FILE",
        summary: "Print the last lines of a file",
        description: "Prints the final lines of one UTF-8 text file, preserving syntax highlighting.",
        arguments: &[("FILE", "Text file to read")],
        options: &[
            ("-n COUNT", "Number of lines to print; defaults to 10"),
            ("-h, --help", "Show detailed help"),
        ],
        examples: &["tail app.log", "tail -n 50 app.log"],
    },
    CommandHelp {
        name: "less",
        usage: "less FILE",
        summary: "Open text in the built-in pager",
        description: "Opens one UTF-8 text file in the interactive pager; redirected mode prints it.",
        arguments: &[("FILE", "Text file to open")],
        options: HELP_OPTION,
        examples: &["less README.md"],
    },
    CommandHelp {
        name: "clear",
        usage: "clear",
        summary: "Clear the terminal",
        description: "Clears the visible LiteShell output area.",
        arguments: &[],
        options: HELP_OPTION,
        examples: &["clear"],
    },
    CommandHelp {
        name: "which",
        usage: "which COMMAND...",
        summary: "Resolve command locations",
        description: "Identifies builtins, translated commands, and executables resolved from the current directory or PATH.",
        arguments: &[("COMMAND...", "One or more command names to resolve")],
        options: HELP_OPTION,
        examples: &["which rg", "which ps cargo"],
    },
    CommandHelp {
        name: "find",
        usage: "find [QUERY...]",
        summary: "Fuzzy-search file names",
        description: "Searches the indexed working tree for files and directories matching a fuzzy query.",
        arguments: &[("QUERY...", "Optional fuzzy query; words are joined with spaces")],
        options: HELP_OPTION,
        examples: &["find parser", "find rust migration"],
    },
    CommandHelp {
        name: "rg",
        usage: "rg QUERY...",
        summary: "Search indexed file contents",
        description: "Searches text in the working tree and prints matching file, line, and preview information.",
        arguments: &[("QUERY...", "Required text query; words are joined with spaces")],
        options: HELP_OPTION,
        examples: &["rg TODO", "rg completion candidate"],
    },
    CommandHelp {
        name: "ps",
        usage: "ps [OPTIONS] [NAME|PID]",
        summary: "List or filter Windows processes",
        description: "Translates Unix-like ps arguments to tasklist.exe. A name matches an image-name prefix.",
        arguments: &[("NAME|PID", "Optional image-name prefix or numeric PID")],
        options: &[
            ("aux, -aux", "Show verbose process information"),
            ("-h, --help", "Show detailed help"),
            ("--", "Treat following value as the filter")],
        examples: &["ps", "ps aux", "ps cargo", "ps 1234"],
    },
    CommandHelp {
        name: "kill",
        usage: "kill [OPTIONS] PID...",
        summary: "End Windows processes",
        description: "Translates Unix-like process termination arguments to taskkill.exe.",
        arguments: &[("PID...", "One or more numeric process IDs")],
        options: &[
            ("-s, --signal TERM|KILL", "TERM ends normally; KILL requests forced termination"),
            ("--signal=TERM|KILL", "Select the signal using assignment syntax"),
            ("--tree", "End each process and its descendants"),
            ("-h, --help", "Show detailed help"),
            ("--", "Treat following values as process IDs"),
        ],
        examples: &["kill 1234", "kill -s KILL 1234", "kill --tree 1234 5678"],
    },
    CommandHelp {
        name: "help",
        usage: "help [COMMAND]",
        summary: "Show LiteShell or command help",
        description: "Shows the command overview, or detailed help for one builtin or translated command.",
        arguments: &[("COMMAND", "Builtin or translated command to explain")],
        options: HELP_OPTION,
        examples: &["help", "help ls", "ls --help"],
    },
    CommandHelp {
        name: "version",
        usage: "version",
        summary: "Show LiteShell version information",
        description: "Prints the LiteShell package version used to build this executable.",
        arguments: &[],
        options: HELP_OPTION,
        examples: &["version", "liteshell --version"],
    },
    CommandHelp {
        name: "exit",
        usage: "exit",
        summary: "Leave LiteShell",
        description: "Ends the current interactive LiteShell session.",
        arguments: &[],
        options: HELP_OPTION,
        examples: &["exit"],
    },
    CommandHelp {
        name: "quit",
        usage: "quit",
        summary: "Leave LiteShell (alias of exit)",
        description: "Ends the current interactive LiteShell session. This is an alias of exit.",
        arguments: &[],
        options: HELP_OPTION,
        examples: &["quit"],
    },
];

pub fn version_text() -> String {
    format!("LiteShell {VERSION}\n")
}

pub fn command(name: &str) -> Option<&'static CommandHelp> {
    COMMANDS
        .iter()
        .find(|command| command.name.eq_ignore_ascii_case(name))
}

fn table<'a>(rows: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let rows: Vec<_> = rows.into_iter().collect();
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    let mut output = String::new();
    for (label, description) in rows {
        output.push_str(&format!("  {label:<width$}  {description}\n"));
    }
    output
}

pub fn overview_text() -> String {
    let mut output = format!(
        "LiteShell {VERSION}\nA fast, Windows-native interactive shell.\n\nUsage:\n  liteshell [OPTIONS]\n  liteshell -c <COMMAND> [OPTIONS]\n  <COMMAND> --help\n\nOptions:\n"
    );
    output.push_str(&table([
        (
            "-c, --command <COMMAND>",
            "Execute a command without starting the TUI",
        ),
        (
            "--pipefail",
            "Fail a pipeline when any stage fails (default)",
        ),
        (
            "--no-pipefail",
            "Use only the final pipeline stage's status",
        ),
        (
            "--status-line <MODE>",
            "Set status line to auto, on, or off",
        ),
        ("--no-status-line", "Hide the interactive status line"),
        ("-h, --help", "Show this help"),
        ("-V, --version", "Show version information"),
    ]));
    output.push_str("\nCommands:\n");
    output.push_str(&table(
        COMMANDS
            .iter()
            .filter(|command| command.name != "quit")
            .map(|command| (command.usage, command.summary)),
    ));
    output.push_str("\nRun '<COMMAND> --help' or 'help <COMMAND>' for details.\n");
    output
}

pub fn command_text(name: &str) -> Option<String> {
    let command = command(name)?;
    let mut output = format!(
        "{} - {}\n\n{}\n\nUsage:\n  {}\n",
        command.name, command.summary, command.description, command.usage
    );
    if !command.arguments.is_empty() {
        output.push_str("\nArguments:\n");
        output.push_str(&table(command.arguments.iter().copied()));
    }
    if !command.options.is_empty() {
        output.push_str("\nOptions:\n");
        output.push_str(&table(command.options.iter().copied()));
    }
    if !command.examples.is_empty() {
        output.push_str("\nExamples:\n");
        for example in command.examples {
            output.push_str(&format!("  {example}\n"));
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_is_tabular_and_mentions_version() {
        let help = overview_text();
        assert!(help.starts_with(&format!("LiteShell {VERSION}\n")));
        assert!(help.contains("-V, --version"));
        assert!(help.contains("ls [OPTIONS] [PATH]"));
    }

    #[test]
    fn every_registered_command_has_detailed_help() {
        for name in crate::NAMES.iter().chain(["ps", "kill"].iter()) {
            let help = command_text(name).unwrap_or_else(|| panic!("missing help for {name}"));
            assert!(help.contains("Usage:"), "incomplete help for {name}");
            assert!(help.contains("--help"), "missing help option for {name}");
        }
    }
}
