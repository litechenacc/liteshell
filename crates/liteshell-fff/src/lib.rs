mod ffi;
use liteshell_core::{SearchCandidate, SearchKind, SearchProvider};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

/// Safe search facade. The pinned DLL is probed lazily; the filesystem fallback
/// keeps the shell useful when the optional distribution DLL is absent.
#[derive(Default)]
pub struct FffSearch {
    dll: Option<Result<ffi::Library, String>>,
    root: Option<PathBuf>,
}
impl FffSearch {
    fn ensure(&mut self, root: &Path) {
        if self.root.as_deref() != Some(root) {
            self.root = Some(root.to_owned());
        }
        if self.dll.is_none() {
            self.dll = Some(ffi::Library::load());
        }
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
        walk(
            root, root, kind, &query, limit, &mut count, emit, cancelled,
        )
    }
}
fn walk(
    root: &Path,
    dir: &Path,
    kind: SearchKind,
    q: &str,
    limit: usize,
    count: &mut usize,
    emit: &mut dyn FnMut(SearchCandidate),
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    if *count >= limit || cancelled() {
        return Ok(());
    }
    let entries = match fs::read_dir(dir) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    for e in entries.flatten() {
        if *count >= limit || cancelled() {
            break;
        }
        let p = e.path();
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('/', "\\");
        let is_dir = p.is_dir();
        if is_dir {
            if matches!(kind, SearchKind::Directories | SearchKind::Mixed)
                && rel.to_lowercase().contains(q)
            {
                emit(SearchCandidate {
                    label: rel.clone(),
                    detail: "directory".into(),
                    value: format!("{rel}\\"),
                    directory: true,
                });
                *count += 1;
            }
            if !matches!(e.file_name().to_str(), Some(".git" | "target" | "build")) {
                walk(root, &p, kind, q, limit, count, emit, cancelled)?;
            }
        } else if matches!(kind, SearchKind::Files | SearchKind::Mixed)
            && rel.to_lowercase().contains(q)
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
        } else if kind == SearchKind::Grep && !q.is_empty() {
            if let Ok(file) = fs::File::open(&p) {
                for (number, line) in BufReader::new(file).lines().take(100_000).enumerate() {
                    let Ok(line) = line else { break };
                    if cancelled() {
                        break;
                    }
                    if line.to_lowercase().contains(q) {
                        emit(SearchCandidate {
                            label: format!("{rel}:{}:1", number + 1),
                            detail: line,
                            value: rel.clone(),
                            directory: false,
                        });
                        *count += 1;
                        if *count >= limit {
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
