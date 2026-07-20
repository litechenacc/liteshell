use crate::{completion::Completion, editor::Editor, output::OutputEvent};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppMode {
    #[default]
    Editing,
    Completion,
    Pager,
    RunningTask,
    RunningChild,
    Exiting,
}
#[derive(Debug, Default)]
pub struct AppState {
    pub mode: AppMode,
    pub editor: Editor,
    pub completion: Completion,
    pub status: String,
    pub output: Vec<OutputEvent>,
}
impl AppState {
    pub fn transition(&mut self, mode: AppMode) {
        self.mode = mode;
    }
}
