use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchKind {
    Directories,
    Mixed,
    Files,
    Grep,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchCandidate {
    pub label: String,
    pub detail: String,
    pub value: String,
    pub directory: bool,
}

pub trait SearchProvider {
    fn search(
        &mut self,
        kind: SearchKind,
        query: &str,
        root: &Path,
        limit: usize,
    ) -> Result<Vec<SearchCandidate>, String>;

    fn search_stream(
        &mut self,
        kind: SearchKind,
        query: &str,
        root: &Path,
        limit: usize,
        emit: &mut dyn FnMut(SearchCandidate),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        for candidate in self.search(kind, query, root, limit)? {
            if cancelled() {
                break;
            }
            emit(candidate);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandResult {
    pub status: i32,
    pub handled: bool,
}
impl CommandResult {
    pub const fn ok() -> Self {
        Self {
            status: 0,
            handled: true,
        }
    }
    pub const fn status(status: i32) -> Self {
        Self {
            status,
            handled: true,
        }
    }
    pub const fn unhandled() -> Self {
        Self {
            status: 127,
            handled: false,
        }
    }
}
