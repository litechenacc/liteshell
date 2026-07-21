mod ffi;
use liteshell_core::{SearchCandidate, SearchKind, SearchProvider};
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

/// Safe search facade. The pinned DLL is probed lazily; the filesystem fallback
/// keeps the shell useful when the optional distribution DLL is absent.
pub struct FffSearch {
    dll: Option<Result<ffi::Library, String>>,
    root: Option<PathBuf>,
    excluded_directories: HashSet<String>,
}
impl FffSearch {
    pub fn new(excluded_directories: impl IntoIterator<Item = String>) -> Self {
        Self {
            dll: None,
            root: None,
            excluded_directories: excluded_directories
                .into_iter()
                .map(|name| name.to_lowercase())
                .collect(),
        }
    }

    pub fn excluded_directories(&self) -> impl Iterator<Item = &str> {
        self.excluded_directories.iter().map(String::as_str)
    }

    fn ensure(&mut self, root: &Path) {
        if self.root.as_deref() != Some(root) {
            self.root = Some(root.to_owned());
        }
        if self.dll.is_none() {
            self.dll = Some(ffi::Library::load());
        }
    }
}
impl Default for FffSearch {
    fn default() -> Self {
        Self::new([".git", "node_modules", "__pycache__"].map(str::to_owned))
    }
}
impl SearchProvider for FffSearch {
    fn search(
        &mut self,
        kind: SearchKind,
        query: &str,
        root: &Path,
        limit: usize,
    ) -> Result<Vec<SearchCandidate>, String> {
        let mut out = Vec::new();
        self.search_stream(
            kind,
            query,
            root,
            limit,
            &mut |candidate| out.push(candidate),
            &|| false,
        )?;
        Ok(out)
    }

    fn search_stream(
        &mut self,
        kind: SearchKind,
        query: &str,
        root: &Path,
        limit: usize,
        emit: &mut dyn FnMut(SearchCandidate),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        self.ensure(root);
        let query = query.to_lowercase();
        let mut count = 0;
        let options = WalkOptions {
            root,
            kind,
            query: &query,
            limit,
            excluded_directories: &self.excluded_directories,
            cancelled,
        };
        walk(&options, root, &mut count, emit)
    }
}

struct WalkOptions<'a> {
    root: &'a Path,
    kind: SearchKind,
    query: &'a str,
    limit: usize,
    excluded_directories: &'a HashSet<String>,
    cancelled: &'a dyn Fn() -> bool,
}

fn walk(
    options: &WalkOptions<'_>,
    dir: &Path,
    count: &mut usize,
    emit: &mut dyn FnMut(SearchCandidate),
) -> Result<(), String> {
    if *count >= options.limit || (options.cancelled)() {
        return Ok(());
    }
    let entries = match fs::read_dir(dir) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    for e in entries.flatten() {
        if *count >= options.limit || (options.cancelled)() {
            break;
        }
        let p = e.path();
        let rel = p
            .strip_prefix(options.root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('/', "\\");
        let is_dir = p.is_dir();
        if is_dir {
            let excluded = options
                .excluded_directories
                .contains(&e.file_name().to_string_lossy().to_lowercase());
            if excluded {
                continue;
            }
            if matches!(options.kind, SearchKind::Directories | SearchKind::Mixed)
                && path_matches(options.kind, &rel, options.query)
            {
                emit(SearchCandidate {
                    label: rel.clone(),
                    detail: "directory".into(),
                    value: format!("{rel}\\"),
                    directory: true,
                });
                *count += 1;
            }
            let follows_reparse_point = e
                .file_type()
                .map(|file_type| file_type.is_symlink())
                .unwrap_or(true);
            if !follows_reparse_point {
                walk(options, &p, count, emit)?;
            }
        } else if matches!(options.kind, SearchKind::Files | SearchKind::Mixed)
            && rel.to_lowercase().contains(options.query)
        {
            emit(SearchCandidate {
                label: rel.clone(),
                detail: e
                    .metadata()
                    .map(|m| format!("{} B", m.len()))
                    .unwrap_or_default(),
                value: rel,
                directory: false,
            });
            *count += 1;
        } else if options.kind == SearchKind::Grep && !options.query.is_empty() {
            if let Ok(file) = fs::File::open(&p) {
                for (number, line) in BufReader::new(file).lines().take(100_000).enumerate() {
                    let Ok(line) = line else { break };
                    if (options.cancelled)() {
                        break;
                    }
                    if line.to_lowercase().contains(options.query) {
                        emit(SearchCandidate {
                            label: format!("{rel}:{}:1", number + 1),
                            detail: line,
                            value: rel.clone(),
                            directory: false,
                        });
                        *count += 1;
                        if *count >= options.limit {
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn path_matches(kind: SearchKind, candidate: &str, query: &str) -> bool {
    if kind == SearchKind::Directories {
        fuzzy_match(candidate, query)
    } else {
        candidate.to_lowercase().contains(query)
    }
}

fn fuzzy_match(candidate: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut query = query.chars();
    let mut expected = query.next();
    for character in candidate.to_lowercase().chars() {
        if Some(character) == expected {
            expected = query.next();
            if expected.is_none() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_directories_are_pruned_but_build_is_searchable() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("node_modules").join("hidden-match")).unwrap();
        fs::create_dir_all(root.path().join("build").join("visible-match")).unwrap();
        let mut search = FffSearch::new(["node_modules".to_owned()]);

        let candidates = search
            .search(SearchKind::Directories, "match", root.path(), 100)
            .unwrap();

        assert!(candidates
            .iter()
            .any(|candidate| candidate.label.contains("visible-match")));
        assert!(!candidates
            .iter()
            .any(|candidate| candidate.label.contains("hidden-match")));
    }
}
