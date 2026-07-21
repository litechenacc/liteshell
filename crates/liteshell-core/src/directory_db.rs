use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const RECORD_MAGIC: &[u8; 4] = b"LSD1";
const RECORD_HEADER_LEN: usize = 32;

#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub rank: f64,
    pub last_accessed: u64,
}

#[derive(Debug)]
pub struct DirectoryDb {
    path: PathBuf,
    entries: HashMap<String, DirectoryEntry>,
    pending: HashMap<String, DirectoryEntry>,
    last_flush: SystemTime,
    writer: Option<JoinHandle<(Vec<DirectoryEntry>, io::Result<()>)>>,
}

impl DirectoryDb {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            entries: HashMap::new(),
            pending: HashMap::new(),
            last_flush: SystemTime::now(),
            writer: None,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &DirectoryEntry> {
        self.entries.values()
    }

    pub fn load(&mut self) -> io::Result<()> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        self.entries.clear();
        let mut offset = 0;
        while bytes.len().saturating_sub(offset) >= RECORD_HEADER_LEN {
            if &bytes[offset..offset + 4] != RECORD_MAGIC {
                offset += 1;
                continue;
            }
            let path_len =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let rank = f64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
            let last_accessed =
                u64::from_le_bytes(bytes[offset + 16..offset + 24].try_into().unwrap());
            let expected_checksum =
                u64::from_le_bytes(bytes[offset + 24..offset + 32].try_into().unwrap());
            let end = offset
                .saturating_add(RECORD_HEADER_LEN)
                .saturating_add(path_len);
            if end > bytes.len() {
                break;
            }
            let path_bytes = &bytes[offset + RECORD_HEADER_LEN..end];
            if record_checksum(rank, last_accessed, path_bytes) != expected_checksum {
                offset += 1;
                continue;
            }
            let path = String::from_utf8_lossy(path_bytes).into_owned();
            self.apply(PathBuf::from(path), rank, last_accessed);
            offset = end;
        }
        self.pending.clear();
        self.last_flush = SystemTime::now();
        Ok(())
    }

    pub fn record(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref().to_owned();
        let now = now_epoch();
        self.apply(path.clone(), 1.0, now);
        let key = path_key(&path);
        let pending = self.pending.entry(key).or_insert(DirectoryEntry {
            path,
            rank: 0.0,
            last_accessed: now,
        });
        pending.rank += 1.0;
        pending.last_accessed = now;
    }

    pub fn flush_if_due(&mut self, interval: Duration) -> io::Result<()> {
        self.poll_writer()?;
        if !self.pending.is_empty() && self.last_flush.elapsed().unwrap_or(interval) >= interval {
            let mut records: Vec<_> = self.pending.drain().map(|(_, entry)| entry).collect();
            records.sort_by_key(|entry| path_key(&entry.path));
            let path = self.path.clone();
            self.writer = Some(thread::spawn(move || {
                let result = append_records(&path, &records);
                (records, result)
            }));
            self.last_flush = SystemTime::now();
        }
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.finish_writer()?;
        if self.pending.is_empty() {
            self.last_flush = SystemTime::now();
            return Ok(());
        }
        let mut records: Vec<_> = self.pending.drain().map(|(_, entry)| entry).collect();
        records.sort_by_key(|entry| path_key(&entry.path));
        if let Err(error) = append_records(&self.path, &records) {
            self.restore_pending(records);
            return Err(error);
        }
        self.last_flush = SystemTime::now();
        Ok(())
    }

    fn poll_writer(&mut self) -> io::Result<()> {
        if self
            .writer
            .as_ref()
            .is_some_and(|writer| writer.is_finished())
        {
            self.finish_writer()?;
        }
        Ok(())
    }

    fn finish_writer(&mut self) -> io::Result<()> {
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        let (records, result) = writer
            .join()
            .map_err(|_| io::Error::other("directory database writer panicked"))?;
        if let Err(error) = result {
            self.restore_pending(records);
            return Err(error);
        }
        Ok(())
    }

    fn restore_pending(&mut self, records: Vec<DirectoryEntry>) {
        for entry in records {
            let DirectoryEntry {
                path,
                rank,
                last_accessed,
            } = entry;
            let key = path_key(&path);
            let pending = self.pending.entry(key).or_insert(DirectoryEntry {
                path,
                rank: 0.0,
                last_accessed,
            });
            pending.rank += rank;
            pending.last_accessed = pending.last_accessed.max(last_accessed);
        }
    }

    fn apply(&mut self, path: PathBuf, rank: f64, last_accessed: u64) {
        let key = path_key(&path);
        let entry = self.entries.entry(key).or_insert(DirectoryEntry {
            path,
            rank: 0.0,
            last_accessed,
        });
        entry.rank += rank;
        entry.last_accessed = entry.last_accessed.max(last_accessed);
    }
}

impl Drop for DirectoryDb {
    fn drop(&mut self) {
        if self.flush().is_err() {
            let _ = self.flush();
        }
    }
}

pub fn frecency(entry: &DirectoryEntry, now: u64) -> f64 {
    let age = now.saturating_sub(entry.last_accessed);
    let multiplier = if age < 60 * 60 {
        4.0
    } else if age < 24 * 60 * 60 {
        2.0
    } else if age < 7 * 24 * 60 * 60 {
        0.5
    } else {
        0.25
    };
    entry.rank * multiplier
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn append_records(path: &Path, records: &[DirectoryEntry]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for entry in records {
        let path = entry.path.to_string_lossy();
        let path = path.as_bytes();
        let path_len = u32::try_from(path.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path is too long"))?;
        let mut record = Vec::with_capacity(RECORD_HEADER_LEN + path.len());
        record.extend_from_slice(RECORD_MAGIC);
        record.extend_from_slice(&path_len.to_le_bytes());
        record.extend_from_slice(&entry.rank.to_le_bytes());
        record.extend_from_slice(&entry.last_accessed.to_le_bytes());
        record.extend_from_slice(
            &record_checksum(entry.rank, entry.last_accessed, path).to_le_bytes(),
        );
        record.extend_from_slice(path);
        file.write_all(&record)?;
    }
    file.sync_data()
}

fn record_checksum(rank: f64, last_accessed: u64, path: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in rank
        .to_le_bytes()
        .into_iter()
        .chain(last_accessed.to_le_bytes())
        .chain(path.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_merged_and_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("directories.db");
        {
            let mut db = DirectoryDb::new(path.clone());
            db.record(r"C:\work\LiteShell");
            db.flush_if_due(Duration::ZERO).unwrap();
            db.record(r"c:\WORK\liteshell");
            db.flush().unwrap();
        }
        let mut loaded = DirectoryDb::new(path);
        loaded.load().unwrap();
        let entry = loaded.entries().next().unwrap();
        assert_eq!(entry.rank, 2.0);
        assert_eq!(loaded.entries().count(), 1);
    }

    #[test]
    fn truncated_last_record_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("directories.db");
        fs::write(&path, b"LSD1\x10\0\0").unwrap();
        let mut db = DirectoryDb::new(path);
        db.load().unwrap();
        assert_eq!(db.entries().count(), 0);
    }
}
