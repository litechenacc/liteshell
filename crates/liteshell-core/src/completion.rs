#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Candidate {
    pub label: String,
    pub detail: String,
    pub replacement: String,
}
#[derive(Clone, Debug, Default)]
pub struct Completion {
    pub candidates: Vec<Candidate>,
    pub selected: usize,
    pub replacement_start: usize,
    pub replacement_end: usize,
}
impl Completion {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn next(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = (self.selected + 1) % self.candidates.len();
        }
    }
    pub fn previous(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = if self.selected == 0 {
                self.candidates.len() - 1
            } else {
                self.selected - 1
            };
        }
    }
}
