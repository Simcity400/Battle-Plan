
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{Debug, Write as _},
    fs,
    io::{self, BufRead, Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[cfg(test)]
use viewer_foundation::debug_fields::sorted_set;

pub(super) type Error = Arc<Mutex<Option<io::Error>>>;

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Digest,
    Text,
}

#[derive(Default)]
struct Fingerprint {
    hash: Sha256,
    bytes: u64,
}

impl Write for Fingerprint {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.hash.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Writer {
    dir: PathBuf,
    scratch: Vec<String>,
    error: Error,
    mode: Mode,
    digests: Arc<Mutex<BTreeMap<String, Fingerprint>>>,
}

impl Writer {
    pub fn new(dir: PathBuf, error: Error, mode: Mode) -> Self {
        let output = if matches!(mode, Mode::Text) {
            dir.as_path()
        } else {
            dir.parent().unwrap()
        };
        if let Err(why) = fs::create_dir_all(output) {
            error.lock().unwrap().get_or_insert(why);
        }
        let scratch = dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("scratch")
            .join(dir.file_name().unwrap());
        let mut prefixes = vec![scratch.to_string_lossy().into_owned()];
        if let Ok(absolute) = fs::canonicalize(&scratch) {
            let absolute = absolute.to_string_lossy();
            prefixes.push(absolute.into_owned());
        }
        let mut scratch: Vec<_> = prefixes
            .into_iter()
            .flat_map(|prefix| {
                let windows = prefix.replace('/', "\\");
                let short = windows.strip_prefix("\\\\?\\").unwrap_or(&windows);
                [
                    windows.replace('\\', "\\\\"),
                    short.replace('\\', "\\\\"),
                    short.replace('\\', "/"),
                    prefix.replace('\\', "\\\\"),
                ]
            })
            .collect();
        scratch.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        scratch.dedup();
        Self {
            dir,
            scratch,
            error,
            mode,
            digests: Arc::default(),
        }
    }

    pub fn write(&self, reader: &str, value: &impl Debug) {
        let result = (|| match self.mode {
            Mode::Text => {
                let file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.dir.join(format!("{reader}.txt")))?;
                self.write_to(io::BufWriter::new(file), value)
            }
            Mode::Digest => {
                let mut digests = self.digests.lock().unwrap();
                self.write_to(digests.entry(reader.into()).or_default(), value)
            }
        })();
        self.record(result);
    }

    fn write_to(&self, output: impl Write, value: &impl Debug) -> io::Result<()> {
        let mut text = Normalized::new(output, &self.scratch);
        writeln!(text, "{value:?}").map_err(|_| {
            text.error
                .take()
                .unwrap_or_else(|| io::Error::other("reader Debug failed"))
        })?;
        text.finish()
    }

    pub fn finish(&self) {
        if matches!(self.mode, Mode::Text) {
            return;
        }
        let result = (|| {
            let path = self.dir.parent().unwrap().join(format!(
                "{}.sha256",
                self.dir.file_name().unwrap().to_string_lossy()
            ));
            let mut file = io::BufWriter::new(fs::File::create(path)?);
            for (reader, digest) in self.digests.lock().unwrap().iter() {
                writeln!(
                    file,
                    "{reader} {:x} {}",
                    digest.hash.clone().finalize(),
                    digest.bytes
                )?;
            }
            file.flush()
        })();
        self.record(result);
    }

    fn record(&self, result: io::Result<()>) {
        if let Err(error) = result {
            self.error.lock().unwrap().get_or_insert(error);
        }
    }
}

struct Normalized<'a, W> {
    output: W,
    prefixes: &'a [String],
    pending: String,
    error: Option<io::Error>,
}

impl<'a, W: Write> Normalized<'a, W> {
    fn new(output: W, prefixes: &'a [String]) -> Self {
        Self {
            output,
            prefixes,
            pending: String::new(),
            error: None,
        }
    }

    fn drain(&mut self, final_chunk: bool) -> io::Result<()> {
        let keep = self
            .prefixes
            .iter()
            .map(String::len)
            .max()
            .unwrap_or(0)
            .saturating_sub(1);
        let mut end = if final_chunk {
            self.pending.len()
        } else {
            self.pending.len().saturating_sub(keep)
        };
        while !self.pending.is_char_boundary(end) {
            end -= 1;
        }
        let mut at = 0;
        while at < end {
            let next = self
                .prefixes
                .iter()
                .filter_map(|prefix| self.pending[at..].find(prefix).map(|i| (at + i, prefix)))
                .min_by_key(|(i, prefix)| (*i, std::cmp::Reverse(prefix.len())));
            let Some((i, prefix)) = next.filter(|(i, _)| *i < end) else {
                break;
            };
            self.output.write_all(&self.pending.as_bytes()[at..i])?;
            self.output.write_all(b"<stack-user>")?;
            at = i + prefix.len();
        }
        if at < end {
            self.output.write_all(&self.pending.as_bytes()[at..end])?;
            at = end;
        }
        self.pending.drain(..at);
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        self.drain(true)?;
        self.output.flush()
    }
}

impl<W: Write> std::fmt::Write for Normalized<'_, W> {
    fn write_str(&mut self, mut text: &str) -> std::fmt::Result {
        while !text.is_empty() {
            let mut end = text.len().min(65536);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.pending.push_str(&text[..end]);
            text = &text[end..];
            if self.pending.len() >= 65536 {
                self.drain(false).map_err(|error| {
                    self.error = Some(error);
                    std::fmt::Error
                })?;
            }
        }
        Ok(())
    }
}

fn changed(base: &Path, head: &Path) -> io::Result<Option<(usize, String, String)>> {
    let mut a = io::BufReader::with_capacity(1 << 20, fs::File::open(base)?);
    let mut b = io::BufReader::with_capacity(1 << 20, fs::File::open(head)?);
    let (mut at, mut line, mut start) = (0u64, 1, 0);
    loop {
        let (left, right) = (a.fill_buf()?, b.fill_buf()?);
        if left.is_empty() && right.is_empty() {
            return Ok(None);
        }
        let n = left.len().min(right.len());
        let same = if left[..n] == right[..n] {
            n
        } else {
            left[..n]
                .iter()
                .zip(right)
                .position(|(a, b)| a != b)
                .unwrap()
        };
        line += left[..same].iter().filter(|&&byte| byte == b'\n').count();
        if let Some(i) = left[..same].iter().rposition(|&byte| byte == b'\n') {
            start = at + i as u64 + 1;
        }
        at += same as u64;
        if same < n || n == 0 {
            break;
        }
        a.consume(same);
        b.consume(same);
    }
    let excerpt = |path: &Path| -> io::Result<String> {
        let mut file = fs::File::open(path)?;
        let eof = at == file.metadata()?.len();
        file.seek(io::SeekFrom::Start(start.max(at.saturating_sub(80))))?;
        let mut bytes = Vec::new();
        file.take(240).read_to_end(&mut bytes)?;
        if let Some(i) = bytes.iter().position(|&byte| byte == b'\n') {
            bytes.truncate(i + 1);
        }
        Ok(format!(
            "{:?}{}",
            String::from_utf8_lossy(&bytes),
            if eof { " <end of file>" } else { "" }
        ))
    };
    Ok(Some((line, excerpt(base)?, excerpt(head)?)))
}

fn files(dir: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    for stack in fs::read_dir(dir)? {
        let stack = stack?;
        if stack.file_type()?.is_dir() {
            for file in fs::read_dir(stack.path())? {
                let file = file?;
                files.insert(PathBuf::from(stack.file_name()).join(file.file_name()));
            }
        } else {
            files.insert(PathBuf::from(stack.file_name()));
        }
    }
    Ok(files)
}

fn manifest(path: &Path) -> io::Result<BTreeMap<String, String>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error),
    };
    let mut entries = BTreeMap::new();
    for line in text.lines() {
        let Some((reader, digest)) = line.split_once(' ') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid value manifest",
            ));
        };
        if entries.insert(reader.into(), digest.into()).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate value reader",
            ));
        }
    }
    Ok(entries)
}

pub(super) fn diff(base: &Path, head: &Path) -> io::Result<String> {
    let (base, head) = (base.join("values"), head.join("values"));
    if !base.is_dir() || !head.is_dir() {
        return Ok(String::new());
    }
    let (before, after) = (files(&base)?, files(&head)?);
    let mut out = String::new();
    for file in before.union(&after) {
        if file.extension().is_some_and(|ext| ext == "sha256") {
            let (a, b) = (manifest(&base.join(file))?, manifest(&head.join(file))?);
            for reader in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                let name = format!("{}/{reader}", file.file_stem().unwrap().to_string_lossy());
                match (a.get(reader), b.get(reader)) {
                    (Some(a), Some(b)) if a != b => {
                        let _ =
                            writeln!(out, "- value {name}: differs\n  - base: {a}\n  - head: {b}");
                    }
                    (None, Some(_)) => {
                        let _ = writeln!(out, "- value {name}: only in head");
                    }
                    (Some(_), None) => {
                        let _ = writeln!(out, "- value {name}: only in base");
                    }
                    _ => {}
                }
            }
            continue;
        }
        let name = file.to_string_lossy().replace('\\', "/");
        if !before.contains(file) || !after.contains(file) {
            let _ = writeln!(
                out,
                "- value {name}: only in {}",
                if before.contains(file) {
                    "base"
                } else {
                    "head"
                }
            );
            continue;
        }
        let Some((line, a, b)) = changed(&base.join(file), &head.join(file))? else {
            continue;
        };
        let _ = writeln!(
            out,
            "- value {name}: differs at line {}\n  - base: {}\n  - head: {}",
            line, a, b
        );
    }
    if out.is_empty() {
        out.push_str("values identical\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn hash_sets_dump_identically_across_seeds_and_insertion_orders() {
        let expected = format!("{:?}", sorted_set(&(0..32).collect()));
        for _ in 0..16 {
            assert_eq!(
                format!("{:?}", sorted_set(&(0..32).rev().collect())),
                expected
            );
        }
    }

    #[test]
    fn digest_matches_normalized_text_and_sorts_repeated_readers() -> io::Result<()> {
        let dir = scratch("corpus-digests");
        let error = Error::default();
        let digest = Writer::new(
            dir.join("hash/values/stack.mod"),
            error.clone(),
            Mode::Digest,
        );
        let text = Writer::new(dir.join("text/values/stack.mod"), error.clone(), Mode::Text);
        for (writer, root) in [(&digest, "hash"), (&text, "text")] {
            writer.write(
                "z",
                &dir.join(root)
                    .join("scratch")
                    .join("stack.mod")
                    .join("file"),
            );
            writer
                .clone()
                .write("z", &vec!["é".repeat(65536), "again".into()]);
            writer.write("a", &());
            writer.finish();
        }
        assert!(error.lock().unwrap().is_none());
        let path = dir.join("hash/values/stack.mod.sha256");
        assert!(fs::read_to_string(&path)?.starts_with("a "));
        let entries = manifest(&path)?;
        for (reader, actual) in entries {
            let bytes = fs::read(dir.join(format!("text/values/stack.mod/{reader}.txt")))?;
            assert_eq!(
                actual,
                format!("{:x} {}", Sha256::digest(&bytes), bytes.len())
            );
        }
        fs::remove_dir_all(dir)
    }

    #[test]
    fn digest_diff_reports_changed_and_one_sided_readers_and_stacks() -> io::Result<()> {
        let dir = scratch("corpus-digest-diff");
        let (base, head) = (dir.join("base"), dir.join("head"));
        let a = Writer::new(base.join("values/stack"), Error::default(), Mode::Digest);
        let b = Writer::new(head.join("values/stack"), Error::default(), Mode::Digest);
        a.write("reader", &1);
        b.write("reader", &2);
        a.write("gone", &());
        b.write("new", &());
        a.finish();
        b.finish();
        let text = diff(&base, &head)?;
        assert!(text.contains("stack/reader: differs"));
        assert!(text.contains("stack/gone: only in base"));
        assert!(text.contains("stack/new: only in head"));
        fs::copy(
            base.join("values/stack.sha256"),
            head.join("values/stack.sha256"),
        )?;
        assert_eq!(diff(&base, &head)?, "values identical\n");
        fs::rename(
            base.join("values/stack.sha256"),
            base.join("values/other.sha256"),
        )?;
        assert!(diff(&base, &head)?.contains("other/reader: only in base"));
        fs::remove_dir_all(dir)
    }

    #[test]
    fn value_diff_reports_changed_and_one_sided_files() -> io::Result<()> {
        let dir = scratch("corpus-values");
        let (base, head) = (dir.join("base"), dir.join("head"));
        assert_eq!(diff(&base, &head)?, "");
        let (a, b) = (
            Writer::new(base.join("values/stack"), Arc::default(), Mode::Text),
            Writer::new(head.join("values/stack"), Arc::default(), Mode::Text),
        );
        a.write("reader", &vec![1, 2]);
        b.write("reader", &vec![1, 3]);
        a.write("gone", &());
        b.write("new", &());
        let text = diff(&base, &head)?;
        assert!(text.contains("stack/reader.txt: differs at line 1"));
        assert!(text.contains("stack/gone.txt: only in base"));
        assert!(text.contains("stack/new.txt: only in head"));
        let mut long = vec![b'x'; 1 << 21];
        long.extend_from_slice(b"\nlast\n");
        fs::write(base.join("values/stack/reader.txt"), &long)?;
        let mut different = long.clone();
        different[(1 << 21) + 2] = b'o';
        fs::write(head.join("values/stack/reader.txt"), &different)?;
        assert!(diff(&base, &head)?.contains("differs at line 2"));
        fs::write(
            head.join("values/stack/reader.txt"),
            &long[..long.len() - 1],
        )?;
        let newline = diff(&base, &head)?;
        assert!(newline.contains("differs at line 2"));
        assert!(newline.contains("<end of file>"));
        fs::write(
            head.join("values/stack/reader.txt"),
            fs::read(base.join("values/stack/reader.txt"))?,
        )?;
        fs::rename(
            head.join("values/stack/new.txt"),
            head.join("values/stack/gone.txt"),
        )?;
        assert_eq!(diff(&base, &head)?, "values identical\n");
        fs::remove_dir_all(dir)
    }

    #[test]
    fn path_normalization_preserves_debug_bytes_across_chunks() -> io::Result<()> {
        use std::fmt::Write as _;
        let prefixes = vec!["C:/scratch/用户/stack".into(), "C:/scratch".into()];
        let input = format!(
            "{} C:/scratch/用户/stack/file C:/scratch/other\n",
            "é".repeat(32763)
        );
        let expected = input
            .replace(&prefixes[0], "<stack-user>")
            .replace(&prefixes[1], "<stack-user>");
        for width in [4, 7, 65530, input.len()] {
            let mut output = Vec::new();
            let mut text = Normalized::new(&mut output, &prefixes);
            let mut rest = input.as_str();
            while !rest.is_empty() {
                let mut end = rest.len().min(width.max(4));
                while !rest.is_char_boundary(end) {
                    end -= 1;
                }
                text.write_str(&rest[..end]).unwrap();
                rest = &rest[end..];
            }
            text.finish()?;
            assert_eq!(output, expected.as_bytes());
        }
        Ok(())
    }
}
