//! Reading a session that sits in one text file which only grows (JSON lines): the shape of Claude
//! Code's transcripts. The marker of such a read is the byte offset after the last complete line.
//! An adapter for a file like that needs only these two helpers to read on from the marker.

use super::{HistoryPrompt, HistoryRead, LastRead, Source, Stamp};
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// The byte offset to continue a file from, taken from the marker of the last read. None means the
/// file cannot be continued and must be read again from the start: it was not read before, or it
/// is smaller or older than at the last read (it was replaced), or the marker is not an offset.
pub fn resume_offset(since: Option<LastRead>, now: Stamp) -> Option<u64> {
    let last = since?;
    let offset: i64 = last.marker.parse().ok().filter(|o| *o >= 0)?;
    let replaced = now.size < last.stamp.size || now.mtime < last.stamp.mtime || now.size < offset;
    (!replaced).then_some(offset as u64)
}

/// Pass every complete line of the file, from byte offset `from`, to `each` (without its line
/// break). Returns the offset after the last complete line: a line that is still being written is
/// left for the next read.
pub fn for_each_line(path: &Path, from: u64, mut each: impl FnMut(&[u8])) -> Result<u64> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    f.seek(SeekFrom::Start(from))?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut offset = from;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            return Ok(offset);
        }
        offset += n as u64;
        each(&buf[..n - 1]);
    }
}

/// Read an agent's prompt history that is a file of JSON lines, from the marker of the last read.
/// `parse` turns one line into a prompt (None: not a prompt). This is the default
/// `Agent::read_history`.
pub fn read_history(
    source: &Source,
    since: Option<LastRead>,
    mut parse: impl FnMut(&Value) -> Option<HistoryPrompt>,
) -> Result<HistoryRead> {
    let from = resume_offset(since, source.stamp);
    let mut prompts = vec![];
    let end = for_each_line(&source.path, from.unwrap_or(0), |line| {
        if let Some(p) = serde_json::from_slice::<Value>(line)
            .ok()
            .and_then(|v| parse(&v))
        {
            prompts.push(p);
        }
    })?;
    Ok(HistoryRead {
        prompts,
        marker: end.to_string(),
        replaces: from.is_none(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last(marker: &str, size: i64, mtime: i64) -> Option<LastRead<'_>> {
        Some(LastRead {
            marker,
            stamp: Stamp { size, mtime },
        })
    }

    #[test]
    fn a_file_that_only_grew_continues_from_the_offset() {
        let now = |size, mtime| Stamp { size, mtime };
        assert_eq!(resume_offset(last("100", 100, 5), now(160, 9)), Some(100));
        // a marker that stopped before the end of the file (a half-written last line)
        assert_eq!(resume_offset(last("90", 100, 5), now(100, 6)), Some(90));
    }

    #[test]
    fn a_file_that_shrank_or_got_older_is_read_again() {
        let now = |size, mtime| Stamp { size, mtime };
        assert_eq!(resume_offset(None, now(10, 1)), None);
        assert_eq!(
            resume_offset(last("100", 100, 5), now(60, 9)),
            None,
            "shrank"
        );
        assert_eq!(
            resume_offset(last("100", 100, 5), now(160, 4)),
            None,
            "older"
        );
        assert_eq!(
            resume_offset(last("100", 100, 5), now(99, 9)),
            None,
            "before the offset"
        );
        assert_eq!(resume_offset(last("not a number", 1, 1), now(9, 9)), None);
        assert_eq!(resume_offset(last("-3", 1, 1), now(9, 9)), None);
    }

    #[test]
    fn lines_are_passed_until_a_half_written_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jsonl");
        std::fs::write(&p, "one\ntwo\nthr").unwrap();
        let mut seen = vec![];
        let end = for_each_line(&p, 0, |l| {
            seen.push(String::from_utf8_lossy(l).into_owned())
        })
        .unwrap();
        assert_eq!((seen, end), (vec!["one".to_string(), "two".into()], 8));
        std::fs::write(&p, "one\ntwo\nthree\n").unwrap();
        let mut seen = vec![];
        let end = for_each_line(&p, 8, |l| {
            seen.push(String::from_utf8_lossy(l).into_owned())
        })
        .unwrap();
        assert_eq!((seen, end), (vec!["three".to_string()], 14));
    }
}
