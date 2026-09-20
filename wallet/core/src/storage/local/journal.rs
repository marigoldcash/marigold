//! The payments journal: what this wallet paid and received, as a person
//! would put it — a time, a direction, an amount, the stamp — one line per
//! event, appended by the commands that move money and read by 'history'.
//!
//! Plaintext TSV beside the vault (`<folder>/<name>.wallet/journal.tsv`).
//! Nothing in it is secret: amounts and directions, no keys, no serials.
//! It is a record for the owner, not a source of truth — the notes are.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const JOURNAL_FILE: &str = "journal.tsv";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// Seconds since the epoch.
    pub at: u64,
    /// "paid", "received", "minted", "exported", "imported", "moved".
    pub kind: String,
    pub petals: u64,
    /// The stamp and fee that went with it, if any.
    pub stamp_petals: u64,
    /// One short phrase: "code handed over", "request", "to marigoldtest:…".
    pub detail: String,
    pub tx: String,
}

impl JournalEntry {
    pub fn now(kind: &str, petals: u64, stamp_petals: u64, detail: impl Into<String>, tx: impl Into<String>) -> Self {
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Self { at, kind: kind.to_string(), petals, stamp_petals, detail: detail.into(), tx: tx.into() }
    }
}

pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// `<folder>/<name>.wallet/journal.tsv`, the folder expanded the way the
    /// vault expands it.
    pub fn new<P: AsRef<Path>>(folder: P, name: &str) -> Self {
        let base = workflow_store::fs::resolve_path(folder.as_ref().to_str().unwrap_or("."))
            .unwrap_or_else(|_| folder.as_ref().to_path_buf());
        Self { path: base.join(crate::storage::local::wallet_dir_name(name)).join(JOURNAL_FILE) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn clean(text: &str) -> String {
        text.replace(['\t', '\n', '\r'], " ")
    }

    pub fn append(&self, entry: &JournalEntry) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        // Amounts and dates are not secrets, but they are nobody else's
        // business on a shared machine either.
        #[cfg(all(unix, not(target_arch = "wasm32")))]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}\t{}",
            entry.at,
            Self::clean(&entry.kind),
            entry.petals,
            entry.stamp_petals,
            Self::clean(&entry.detail),
            Self::clean(&entry.tx)
        )
    }

    /// Oldest first. A line that does not parse is skipped rather than
    /// failing the whole history.
    pub fn read(&self) -> std::io::Result<Vec<JournalEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.path)?;
        Ok(text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split('\t');
                let at = parts.next()?.parse().ok()?;
                let kind = parts.next()?.to_string();
                let petals = parts.next()?.parse().ok()?;
                let stamp_petals = parts.next()?.parse().ok()?;
                let detail = parts.next().unwrap_or("").to_string();
                let tx = parts.next().unwrap_or("").to_string();
                Some(JournalEntry { at, kind, petals, stamp_petals, detail, tx })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_reads_back_in_order_and_survives_a_bad_line() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let journal = Journal::new(dir.path(), "test");
        assert!(journal.read()?.is_empty());
        let paid = JournalEntry {
            at: 10,
            kind: "paid".into(),
            petals: 111_000_000,
            stamp_petals: 2_000_000,
            detail: "code handed\tover".into(),
            tx: "abc".into(),
        };
        let received = JournalEntry {
            at: 20,
            kind: "received".into(),
            petals: 5_000_000,
            stamp_petals: 0,
            detail: "request".into(),
            tx: String::new(),
        };
        journal.append(&paid)?;
        journal.append(&received)?;
        std::fs::OpenOptions::new().append(true).open(journal.path())?.write_all(b"garbage line\n")?;
        let back = journal.read()?;
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].detail, "code handed over", "tabs are flattened");
        assert_eq!(back[1], received);
        Ok(())
    }
}
