pub mod app;
pub mod command;
pub mod completion;
pub mod config;
pub mod editor;
pub mod history;
pub mod output;
pub mod parser;
pub mod shell;

pub use app::{AppMode, AppState};
pub use command::{CommandResult, SearchCandidate, SearchKind, SearchProvider};
pub use editor::Editor;
pub use output::{
    OutputEvent, OutputSink, SemanticColor, StyledLine, StyledSpan, StyledText, TextStyle,
};
pub use shell::ShellState;
