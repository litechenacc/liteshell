#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutputEvent {
    Text(String),
    Error(String),
    Clear,
    Pager { title: String, lines: Vec<String> },
    Status(String),
}

pub trait OutputSink {
    fn emit(&mut self, event: OutputEvent);
}

#[derive(Default)]
pub struct VecOutput(pub Vec<OutputEvent>);
impl OutputSink for VecOutput {
    fn emit(&mut self, event: OutputEvent) {
        self.0.push(event);
    }
}
