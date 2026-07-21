mod scrollback;
mod terminal_session;
mod ui;

pub use scrollback::Scrollback;
pub use terminal_session::TerminalSession;
pub use ui::draw;

use liteshell_core::{AppMode, Editor, OutputEvent, OutputSink, StyledLine};

#[derive(Default)]
pub struct EventBuffer(pub Vec<OutputEvent>);

impl OutputSink for EventBuffer {
    fn emit(&mut self, event: OutputEvent) {
        self.0.push(event);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CompletionSource {
    #[default]
    Path,
    DeepPath,
    History,
}

pub struct TuiState {
    pub mode: AppMode,
    pub editor: Editor,
    pub output: Scrollback,
    pub status: String,
    pub completion: Vec<(String, String)>,
    pub completion_source: CompletionSource,
    pub completion_query: String,
    pub selected: usize,
    pub pager: Option<Pager>,
}

pub struct Pager {
    pub title: String,
    pub lines: Vec<StyledLine>,
    pub top: usize,
}

impl TuiState {
    pub fn new(lines: usize, bytes: usize) -> Self {
        Self {
            mode: AppMode::Editing,
            editor: Editor::default(),
            output: Scrollback::new(lines, bytes),
            status: String::new(),
            completion: Vec::new(),
            completion_source: CompletionSource::Path,
            completion_query: String::new(),
            selected: 0,
            pager: None,
        }
    }

    pub fn apply(&mut self, events: Vec<OutputEvent>) {
        for event in events {
            match event {
                OutputEvent::Text(text) => self.output.push_text(&text, false),
                OutputEvent::Styled(text) => self.output.push_styled(&text, false),
                OutputEvent::Error(text) => self.output.push_text(&text, true),
                OutputEvent::Clear => self.output.clear(),
                OutputEvent::Status(status) => self.status = status,
                OutputEvent::Pager { title, lines } => {
                    self.pager = Some(Pager {
                        title,
                        lines,
                        top: 0,
                    });
                    self.mode = AppMode::Pager;
                }
            }
        }
    }
}
