
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
use viewer_runtime::debug_fields::sorted_set;

const OWNED: &str = ".mod-corpus-owned";
const OWNED_HEADER: &str = "mod-corpus values";
const NONE_COLLECTED: &str = "mod-corpus values: none collected";

pub(super) struct Output {
    root: PathBuf,
    error: Mutex<Option<io::Error>>,
    owned: Mutex<Owned>,
}

struct Owned {
    manifest: fs::File,
    written: BTreeSet<PathBuf>,
    open: BTreeMap<PathBuf, fs::File>,
}

impl Output {
    pub fn prepare(root: &Path) -> io::Result<Arc<Self>> {
        use std::os::windows::fs::OpenOptionsExt;
        const SHARE_READ: u32 = 0x1;
        fs::create_dir_all(root)?;
        unlinked(root)?;
        let path = root.join(OWNED);
        unlinked(&path)?;
        if !path.exists()
            && let Some(report) = unowned_reports(root)?.into_iter().next()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is from an earlier run that this drive cannot confirm; remove it or choose another --out", report.display()),
            ));
        }
        let mut manifest = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(SHARE_READ)
            .open(&path)
            .map_err(|error| match error.raw_os_error() {
                Some(32) => io::Error::new(error.kind(), format!("another mod-corpus run is writing {}", root.display())),
                _ => error,
            })?;
        let mut text = String::new();
        manifest.read_to_string(&mut text)?;
        if text.lines().next() == Some(NONE_COLLECTED)
            && let Some(report) = unowned_reports(root)?.into_iter().next()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is from an earlier run that this drive cannot confirm; remove it or choose another --out", report.display()),
            ));
        }
        let mut owned = Vec::new();
        for (relative, identity) in owned_files(&path, &text)? {
            if let Some(stack) = relative.parent().filter(|stack| !stack.as_os_str().is_empty())
                && !unlinked(&root.join(stack))?
            {
                continue;
            }
            let report = root.join(&relative);
            match (open_owned(&report)?, identity) {
                (None, _) => {}
                (Some(file), Some(identity)) if file_identity(&file)? == identity => owned.push((relative, file)),
                (Some(_), _) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{} is from an earlier run that this drive cannot confirm; remove it or choose another --out", report.display()),
                    ));
                }
            }
        }
        for (_, file) in owned {
            delete_held(&file)?;
        }
        manifest.set_len(0)?;
        manifest.seek(io::SeekFrom::Start(0))?;
        writeln!(manifest, "{OWNED_HEADER}")?;
        Ok(Arc::new(Self {
            root: root.to_owned(),
            error: Mutex::default(),
            owned: Mutex::new(Owned { manifest, written: BTreeSet::new(), open: BTreeMap::new() }),
        }))
    }

    pub fn clear(root: &Path) -> io::Result<()> {
        if root.join(OWNED).is_file() {
            match Self::prepare(root) {
                Ok(output) => drop(output),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        } else if !unlinked(root).unwrap_or(false) {
            return Ok(());
        }
        let _ = fs::write(root.join(OWNED), format!("{NONE_COLLECTED}\n"));
        Ok(())
    }

    pub fn take_error(&self) -> Option<io::Error> {
        self.error.lock().unwrap().take()
    }

    fn open(&self, path: &Path) -> io::Result<fs::File> {
        let mut owned = self.owned.lock().unwrap();
        if owned.written.contains(path) {
            return owned.open.get(path).ok_or_else(|| io::Error::other(format!("{} was already finished", path.display())))?.try_clone();
        }
        let name = path
            .strip_prefix(&self.root)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .ok()
            .filter(|name| owned_path(name).is_some())
            .ok_or_else(|| io::Error::other(format!("{} is outside the values mod-corpus writes", path.display())))?;
        if let Some(stack) = path.parent().filter(|stack| *stack != self.root)
            && !unlinked(stack)?
        {
            return Err(io::Error::other(format!("{} is a link; mod-corpus writes only into its own folders", stack.display())));
        }
        let file = fs::OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(path)
            .map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => io::Error::new(
                    error.kind(),
                    format!("{} was not written by mod-corpus; move it or choose another --out", path.display()),
                ),
                _ => error,
            })?;
        let claimed = stable_identity(&file)
            .and_then(|identity| match identity {
                Some(identity) => writeln!(owned.manifest, "{name}\t{identity}"),
                None => writeln!(owned.manifest, "{name}"),
            })
            .and_then(|()| owned.manifest.flush());
        if let Err(error) = claimed {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(error);
        }
        owned.written.insert(path.to_owned());
        owned.open.insert(path.to_owned(), file.try_clone()?);
        Ok(file)
    }

    fn close(&self, stack: &Path) {
        let mut owned = self.owned.lock().unwrap();
        owned.open.retain(|path, _| path.parent() != Some(stack) && path.with_extension("") != stack);
    }
}

fn unlinked(path: &Path) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_attributes() & REPARSE_POINT != 0 => Err(io::Error::other(format!(
            "{} is a link; mod-corpus does not follow links out of its output",
            path.display()
        ))),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn file_identity(file: &fs::File) -> io::Result<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx};
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let size = size_of::<FILE_ID_INFO>() as u32;
    if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo, (&raw mut info).cast(), size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut identity = format!("{:016x}", info.VolumeSerialNumber);
    for byte in info.FileId.Identifier {
        let _ = write!(identity, "{byte:02x}");
    }
    Ok(identity)
}

fn identity_text(text: &str) -> Option<String> {
    (text.len() == 48 && text.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| text.to_ascii_lowercase())
}

fn stable_identity(file: &fs::File) -> io::Result<Option<String>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
    const SUPPORTS_OPEN_BY_FILE_ID: u32 = 0x0100_0000;
    let mut flags = 0_u32;
    let read = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut flags,
            std::ptr::null_mut(),
            0,
        )
    };
    if read == 0 || flags & SUPPORTS_OPEN_BY_FILE_ID == 0 {
        return Ok(None);
    }
    file_identity(file).map(Some)
}

fn open_owned(path: &Path) -> io::Result<Option<fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};
    match fs::OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn delete_held(file: &fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FileDispositionInfo, FileDispositionInfoEx, SetFileInformationByHandle,
    };
    let info = FILE_DISPOSITION_INFO_EX { Flags: 0x1 | 0x2 };
    let size = size_of::<FILE_DISPOSITION_INFO_EX>() as u32;
    if unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfoEx, (&raw const info).cast(), size) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(1 | 50 | 87)) {
        return Err(error);
    }
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    let size = size_of::<FILE_DISPOSITION_INFO>() as u32;
    if unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfo, (&raw const info).cast(), size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn owned_files(manifest: &Path, text: &str) -> io::Result<Vec<(PathBuf, Option<String>)>> {
    let mut lines = text.lines();
    if !text.is_empty() && !matches!(lines.next(), Some(OWNED_HEADER | NONE_COLLECTED)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} was not written by mod-corpus; choose another --out", manifest.display()),
        ));
    }
    lines
        .map(|line| {
            let (name, identity) = line.split_once('\t').map_or((line, None), |(name, identity)| (name, Some(identity)));
            let identity = identity.map(identity_text);
            owned_path(name)
                .filter(|_| identity != Some(None))
                .map(|file| (file, identity.flatten()))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{} names {line:?}, which mod-corpus never writes", manifest.display()),
                    )
                })
        })
        .collect()
}

fn listed(root: &Path) -> io::Result<Option<Vec<PathBuf>>> {
    let manifest = root.join(OWNED);
    match fs::read_to_string(&manifest) {
        Ok(text) => owned_files(&manifest, &text).map(|files| Some(files.into_iter().map(|(file, _)| file).filter(|file| root.join(file).is_file()).collect())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn unowned_reports(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() {
            for file in fs::read_dir(entry.path())? {
                let file = file?;
                if owned_path(&format!("{name}/{}", file.file_name().to_string_lossy())).is_some() {
                    found.push(file.path());
                }
            }
        } else if owned_path(&name).is_some() {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

fn owned_path(line: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = line.split('/').collect();
    let safe = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.contains(['\\', ':', '\0'])
    };
    let shaped = match parts.as_slice() {
        [file] => file.ends_with(".sha256"),
        [_, file] => file.ends_with(".txt"),
        _ => false,
    };
    (shaped && parts.iter().all(|part| safe(part))).then(|| parts.iter().collect())
}

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
    output: Arc<Output>,
    mode: Mode,
    digests: Arc<Mutex<BTreeMap<String, Fingerprint>>>,
}

impl Writer {
    pub fn new(output: &Arc<Output>, stack: &str, mode: Mode) -> Self {
        let dir = output.root.join(stack);
        if matches!(mode, Mode::Text)
            && let Err(why) = fs::create_dir_all(&dir)
        {
            output.error.lock().unwrap().get_or_insert(why);
        }
        let scratch = output
            .root
            .parent()
            .unwrap()
            .join("scratch")
            .join(stack);
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
            output: Arc::clone(output),
            mode,
            digests: Arc::default(),
        }
    }

    pub fn write(&self, reader: &str, value: &impl Debug) {
        let result = (|| match self.mode {
            Mode::Text => {
                let file = self.output.open(&self.dir.join(format!("{reader}.txt")))?;
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
            self.output.close(&self.dir);
            return;
        }
        let result = (|| {
            let path = self.dir.parent().unwrap().join(format!(
                "{}.sha256",
                self.dir.file_name().unwrap().to_string_lossy()
            ));
            let mut file = io::BufWriter::new(self.output.open(&path)?);
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
        self.output.close(&self.dir);
        self.record(result);
    }

    fn record(&self, result: io::Result<()>) {
        if let Err(error) = result {
            self.output.error.lock().unwrap().get_or_insert(error);
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
    if let Some(listed) = listed(dir)? {
        return Ok(listed.into_iter().collect());
    }
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
    if [&base, &head].into_iter().any(|side| fs::read_to_string(side.join(OWNED)).is_ok_and(|text| text.lines().next() == Some(NONE_COLLECTED))) {
        return Ok(String::new());
    }
    let (before, after) = (files(&base)?, files(&head)?);
    let mut out = String::new();
    for file in before.union(&after) {
        if file.extension().is_some_and(|ext| ext == "sha256") {
            let held = |side: &Path, files: &BTreeSet<PathBuf>| if files.contains(file) { manifest(&side.join(file)) } else { Ok(BTreeMap::new()) };
            let (a, b) = (held(&base, &before)?, held(&head, &after)?);
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
        let (hashed, texts) = (
            Output::prepare(&dir.join("hash/values"))?,
            Output::prepare(&dir.join("text/values"))?,
        );
        let digest = Writer::new(&hashed, "stack.mod", Mode::Digest);
        let text = Writer::new(&texts, "stack.mod", Mode::Text);
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
        assert!(hashed.take_error().is_none() && texts.take_error().is_none());
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
        drop((digest, text, hashed, texts));
        fs::remove_dir_all(dir)
    }

    #[test]
    fn digest_diff_reports_changed_and_one_sided_readers_and_stacks() -> io::Result<()> {
        let dir = scratch("corpus-digest-diff");
        let (base, head) = (dir.join("base"), dir.join("head"));
        let a = Writer::new(&Output::prepare(&base.join("values"))?, "stack", Mode::Digest);
        let b = Writer::new(&Output::prepare(&head.join("values"))?, "stack", Mode::Digest);
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
        fs::write(
            head.join("values/stack.sha256"),
            fs::read(base.join("values/stack.sha256"))?,
        )?;
        assert_eq!(diff(&base, &head)?, "values identical\n");
        drop(a);
        fs::rename(
            base.join("values/stack.sha256"),
            base.join("values/other.sha256"),
        )?;
        fs::write(base.join("values").join(OWNED), "mod-corpus values\nother.sha256\n")?;
        assert!(diff(&base, &head)?.contains("other/reader: only in base"));
        drop(b);
        fs::remove_dir_all(dir)
    }

    #[test]
    fn value_diff_reports_changed_and_one_sided_files() -> io::Result<()> {
        let dir = scratch("corpus-values");
        let (base, head) = (dir.join("base"), dir.join("head"));
        assert_eq!(diff(&base, &head)?, "");
        let (a, b) = (
            Writer::new(&Output::prepare(&base.join("values"))?, "stack", Mode::Text),
            Writer::new(&Output::prepare(&head.join("values"))?, "stack", Mode::Text),
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
        drop(b);
        fs::rename(
            head.join("values/stack/new.txt"),
            head.join("values/stack/gone.txt"),
        )?;
        fs::write(head.join("values").join(OWNED), "mod-corpus values\nstack/reader.txt\nstack/gone.txt\n")?;
        fs::write(base.join("values/notes.md"), "not a value")?;
        fs::write(head.join("values/stack/notes.txt"), "not a value either")?;
        assert_eq!(diff(&base, &head)?, "values identical\n", "files no run wrote are not values");
        drop(a);
        fs::remove_dir_all(dir)
    }

    #[test]
    fn a_reused_output_replaces_only_the_values_a_run_wrote() -> io::Result<()> {
        let dir = scratch("corpus-owned");
        let root = dir.join("values");
        fs::create_dir_all(root.join("stack"))?;
        fs::write(root.join("notes.md"), "kept")?;
        fs::write(root.join("stack/mine.txt"), "kept")?;
        let legacy = Output::prepare(&root).err().expect("reports no manifest owns are refused before anything is written");
        assert!(legacy.to_string().contains("cannot confirm"), "{legacy}");
        assert!(!root.join(OWNED).exists());
        Output::clear(&root)?;
        assert_eq!(fs::read_to_string(root.join(OWNED))?, format!("{NONE_COLLECTED}\n"));
        let marked = Output::prepare(&root).err().expect("a none-collected mark does not vouch for older reports");
        assert!(marked.to_string().contains("cannot confirm"), "{marked}");
        assert_eq!(fs::read_to_string(root.join(OWNED))?, format!("{NONE_COLLECTED}\n"), "a refused run leaves the mark as it was");
        fs::remove_file(root.join(OWNED))?;
        fs::remove_file(root.join("stack/mine.txt"))?;
        let output = Output::prepare(&root)?;
        let busy =Output::prepare(&root).err().expect("a second run into the same folder is refused");
        assert!(busy.to_string().contains("another mod-corpus run"), "{busy}");
        let text = Writer::new(&output, "stack", Mode::Text);
        text.write("reader", &1);
        text.write("reader", &2);
        Writer::new(&output, "other", Mode::Digest).finish();
        assert!(output.take_error().is_none());
        assert_eq!(fs::read_to_string(root.join("stack/reader.txt"))?, "1\n2\n");
        drop(text);
        drop(output);

        fs::write(root.join("stack/mine.txt"), "kept")?;
        let output = Output::prepare(&root)?;
        assert!(!root.join("stack/reader.txt").exists());
        assert!(!root.join("other.sha256").exists());
        assert_eq!(fs::read_to_string(root.join("notes.md"))?, "kept");
        assert_eq!(fs::read_to_string(root.join("stack/mine.txt"))?, "kept");
        let text = Writer::new(&output, "stack", Mode::Text);
        text.write("reader", &3);
        text.write("mine", &4);
        let refused = output.take_error().expect("an unowned report is refused");
        assert!(refused.to_string().contains("not written by mod-corpus"), "{refused}");
        assert_eq!(fs::read_to_string(root.join("stack/reader.txt"))?, "3\n");
        assert_eq!(fs::read_to_string(root.join("stack/mine.txt"))?, "kept");
        drop(text);
        drop(output);

        fs::rename(root.join("stack/reader.txt"), root.join("stack/moved.txt"))?;
        fs::write(root.join("stack/reader.txt"), "the user's")?;
        let unconfirmed = Output::prepare(&root).err().expect("a report that is not the one written is refused, not replaced");
        assert!(unconfirmed.to_string().contains("cannot confirm"), "{unconfirmed}");
        assert_eq!(fs::read_to_string(root.join("stack/reader.txt"))?, "the user's", "a file put where a report was is not the report");

        fs::write(root.join(OWNED), "mod-corpus values\nnotes.md\n")?;
        assert!(Output::prepare(&root).is_err());
        fs::write(root.join(OWNED), "someone else's\n")?;
        assert!(Output::prepare(&root).is_err());
        assert_eq!(fs::read_to_string(root.join("notes.md"))?, "kept");
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
