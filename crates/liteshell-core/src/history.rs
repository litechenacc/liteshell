use std::{fs, io, path::PathBuf};

#[derive(Debug)]
pub struct History {
    entries: Vec<String>,
    capacity: usize,
    path: PathBuf,
}
impl History {
    pub fn new(path: PathBuf, capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
            path,
        }
    }
    pub fn entries(&self) -> &[String] {
        &self.entries
    }
    pub fn add(&mut self, line: impl Into<String>) {
        let line = line.into();
        if line.is_empty() || self.entries.last() == Some(&line) {
            return;
        }
        self.entries.push(line);
        if self.entries.len() > self.capacity {
            self.entries.drain(..self.entries.len() - self.capacity);
        }
    }
    pub fn load(&mut self) -> io::Result<()> {
        let text = match fs::read_to_string(&self.path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        self.entries.clear();
        for line in text
            .trim_start_matches('\u{feff}')
            .lines()
            .map(str::trim_end)
        {
            self.add(line);
        }
        Ok(())
    }
    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut text = self.entries.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        fs::write(&self.path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_dedup() {
        let mut h = History::new("x".into(), 2);
        h.add("a");
        h.add("a");
        h.add("b");
        h.add("c");
        assert_eq!(h.entries(), ["b", "c"]);
    }
}
