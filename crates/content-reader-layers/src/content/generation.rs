use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};

thread_local! { static COLLECT: RefCell<Option<Arc<Witnesses>>> = const { RefCell::new(None) }; }
thread_local! { static CURRENT: RefCell<Option<Arc<super::graph::Graph>>> = const { RefCell::new(None) }; }

pub(super) fn current() -> Option<Arc<super::graph::Graph>> {
    CURRENT.with(|current| current.borrow().clone())
}

pub(super) type Snapshot = Arc<SettingsSnapshot>;

pub(super) struct SettingsSnapshot {
    stamp: Stamp,
    bytes: Mutex<Option<super::Bytes>>,
}

pub struct Generation {
    previous: Option<Arc<super::graph::Graph>>,
    loading: Option<Loading>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

pub struct Loading {
    graph: Arc<super::graph::Graph>,
    open: std::sync::atomic::AtomicBool,
}

impl Loading {
    pub(super) fn new(graph: Arc<super::graph::Graph>) -> Self {
        graph.acquire();
        Self {
            graph,
            open: std::sync::atomic::AtomicBool::new(true),
        }
    }

    pub fn finish(&self) -> bool {
        if self.open.swap(false, std::sync::atomic::Ordering::AcqRel) {
            self.graph.release();
            true
        } else {
            false
        }
    }

    pub fn counts(&self) -> super::graph::Counts {
        self.graph.snapshot()
    }

    pub fn release_sources(&self) {
        self.graph.release_sources();
    }

    pub fn release_parsers(&self, parsers: &[&str]) {
        self.graph.release_parsers(parsers);
    }

    pub fn enter_generation(&self) -> Generation {
        Generation::attach(Arc::clone(&self.graph))
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        self.finish();
    }
}

impl Generation {
    pub fn enter() -> Self {
        Self::resume(Some(Arc::new(super::graph::Graph::fresh())))
    }

    pub fn inherit() -> Self {
        Self::resume(Some(
            current().unwrap_or_else(|| Arc::new(super::graph::Graph::fresh())),
        ))
    }

    pub fn loading() -> Option<Loading> {
        current().map(Loading::new)
    }

    pub(super) fn attach(graph: Arc<super::graph::Graph>) -> Self {
        Self::resume(Some(graph))
    }

    pub fn derive<T: Clone + Send + Sync + 'static>(
        bytes: &super::Bytes,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        build: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        use sha2::{Digest as _, Sha256};
        let graph = bytes
            .owner::<super::graph::Graph>()
            .or_else(current)
            .unwrap_or_else(|| Arc::new(super::graph::Graph::fresh()));
        let mut hash = Sha256::new();
        hash.update(bytes.origin().unwrap_or_default());
        hash.update(bytes.digest());
        let key = super::graph::Key {
            context: hash.finalize().into(),
            parser,
            schema,
            parameters: parameters.to_vec(),
        };
        let (product, _) = graph.product(key, None, |node| {
            let (value, diagnostics) = super::diagnostics::capture(|| {
                if let Some(source) = bytes.origin() {
                    graph.depends_on(node, source)?;
                }
                build()
            });
            (value, Vec::new(), diagnostics)
        })?;
        product.diagnostics.credit();
        product.value::<T>().map(|value| value.as_ref().clone())
    }

    pub fn parse_file<T: Send + Sync + 'static>(
        path: &Path,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        build: impl FnOnce(&[u8]) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        use sha2::{Digest as _, Sha256};
        let graph = current().unwrap_or_else(|| Arc::new(super::graph::Graph::fresh()));
        let source = metadata_in(&graph, path, None, None)?;
        let bytes = source.value::<super::Bytes>()?;
        let mut hash = Sha256::new();
        hash.update(bytes.digest());
        hash.update(path.to_string_lossy().as_bytes());
        let key = super::graph::Key {
            context: hash.finalize().into(),
            parser,
            schema,
            parameters: parameters.to_vec(),
        };
        let (product, _) = graph.product(key, None, |node| {
            let (value, diagnostics) = super::diagnostics::capture(|| {
                graph.depends(node, &source)?;
                build(&bytes)
            });
            (value, Vec::new(), diagnostics)
        })?;
        product.diagnostics.credit();
        product.value()
    }

    pub fn parse_snapshot<T: Send + Sync + 'static>(
        path: &Path,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        read: impl FnOnce(&Path) -> io::Result<Vec<u8>>,
        build: impl FnOnce(&[u8]) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        use sha2::{Digest as _, Sha256};
        let Some(graph) = current() else {
            let bytes = read(path)?;
            super::io_counts::external(path, &bytes);
            return build(&bytes).map(Arc::new);
        };
        let canonical = path.canonicalize()?;
        let path = canonical.as_path();
        let stamp = Stamp::path(path)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.display().to_string()))?;
        let snapshot = {
            let mut snapshots = graph
                .snapshots
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let snapshot = snapshots.entry(path.to_owned()).or_insert_with(|| {
                Arc::new(SettingsSnapshot {
                    stamp: stamp.clone(),
                    bytes: Mutex::new(None),
                })
            });
            if snapshot.stamp != stamp {
                *snapshot = Arc::new(SettingsSnapshot {
                    stamp,
                    bytes: Mutex::new(None),
                });
            }
            Arc::clone(snapshot)
        };
        let bytes = {
            let mut cached = snapshot
                .bytes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if cached.is_none() {
                let bytes = super::Bytes::from(read(path)?);
                if Stamp::path(path)?.as_ref() != Some(&snapshot.stamp) {
                    return Err(changed(path));
                }
                super::io_counts::external_digest(path, bytes.digest(), bytes.len() as u64);
                *cached = Some(bytes);
            }
            cached.as_ref().expect("successful snapshot read").clone()
        };
        let mut hash = Sha256::new();
        hash.update(b"user-settings-snapshot/v1");
        hash.update(bytes.digest());
        hash.update(path.to_string_lossy().as_bytes());
        let key = super::graph::Key {
            context: hash.finalize().into(),
            parser,
            schema,
            parameters: parameters.to_vec(),
        };
        let (product, _) = graph.product(key, None, |_| {
            let (value, diagnostics) = super::diagnostics::capture(|| build(&bytes));
            (value, Vec::new(), diagnostics)
        })?;
        product.diagnostics.credit();
        product.value()
    }

    fn resume(generation: Option<Arc<super::graph::Graph>>) -> Self {
        let loading = generation
            .as_ref()
            .map(|graph| Loading::new(Arc::clone(graph)));
        Self {
            previous: CURRENT.with(|current| current.replace(generation)),
            loading,
            _thread: std::marker::PhantomData,
        }
    }
}

impl Drop for Generation {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
        drop(self.loading.take());
    }
}

pub fn carry<T>(work: impl FnOnce() -> T) -> impl FnOnce() -> T {
    let generation = current();
    let pending = generation
        .as_ref()
        .map(|graph| Loading::new(Arc::clone(graph)));
    move || {
        let _scope = Generation::resume(generation);
        drop(pending);
        work()
    }
}

pub(super) fn metadata(path: &Path) -> io::Result<super::Bytes> {
    let graph = current().unwrap_or_else(|| Arc::new(super::graph::Graph::fresh()));
    source(&graph, path, None, None)
}

pub(super) fn source(
    graph: &super::graph::Graph,
    path: &Path,
    file: Option<File>,
    parent: Option<&Arc<super::graph::Node>>,
) -> io::Result<super::Bytes> {
    metadata_in(graph, path, file, parent)
        .and_then(|product| product.value::<super::Bytes>())
        .map(|bytes| bytes.as_ref().clone())
}

fn metadata_in(
    graph: &super::graph::Graph,
    path: &Path,
    file: Option<File>,
    parent: Option<&Arc<super::graph::Node>>,
) -> io::Result<Arc<super::graph::Product>> {
    use sha2::{Digest as _, Sha256};
    let path = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            path.parent().ok_or(error)?.canonicalize()?.join(
                path.file_name()
                    .ok_or_else(|| super::invalid("content source has no filename"))?,
            )
        }
        Err(error) => return Err(error),
    };
    let context = Sha256::digest(if file.is_some() {
        b"physical-content-source/v1".as_slice()
    } else {
        b"standalone-content-source/v1".as_slice()
    })
    .into();
    if let Some(parent) = path.parent() {
        graph.watch_root(parent, true);
    }
    let key = super::graph::Key {
        context,
        parser: "source bytes",
        schema: 1,
        parameters: path.to_string_lossy().as_bytes().to_vec(),
    };
    let (product, _) = graph.product(key, parent, |_| {
        let (result, diagnostics) = super::diagnostics::capture(|| {
            let (mut file, before) = match file {
                Some(file) => {
                    let before = graph.witnesses.record_file_stamp(&path, &file)?;
                    (file, Some(before))
                }
                None => {
                    graph.witnesses.directory(&path)?;
                    let file = File::open(&path)?;
                    graph.witnesses.file(&path, &file)?;
                    (file, None)
                }
            };
            #[cfg(test)]
            super::tests::LOADS.with(|loads| loads.set(loads.get() + 1));
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut bytes)?;
            match before {
                Some(before) => {
                    if Stamp::file(&file)? != before {
                        return Err(changed(&path));
                    }
                }
                None => graph.witnesses.file(&path, &file)?,
            }
            let bytes = super::Bytes::from(bytes);
            super::io_counts::external_digest(&path, bytes.digest(), bytes.len() as u64);
            io::Result::Ok(bytes)
        });
        let reads = result
            .as_ref()
            .map(|bytes| {
                vec![super::ContentRead {
                    path: path.to_string_lossy().as_bytes().to_vec(),
                    purpose: String::new(),
                    layer: None,
                    outcome: super::ReadOutcome::Bytes {
                        length: bytes.len() as u64,
                        digest: bytes.digest(),
                    },
                }]
            })
            .unwrap_or_default();
        (result, reads, diagnostics)
    })?;
    Ok(product)
}

pub(super) fn document(path: &Path) -> io::Result<super::Document> {
    Generation::parse_file(path, "descriptor grammar", 1, &[], |bytes| {
        super::parser::parse(bytes)
            .map_err(|error| super::invalid(format!("{}: {error}", path.display())))
    })
    .map(|document| document.as_ref().clone())
}

pub(super) struct Collect(
    Option<Arc<Witnesses>>,
    std::marker::PhantomData<std::rc::Rc<()>>,
);

impl Collect {
    pub(super) fn enter(witnesses: Arc<Witnesses>) -> Self {
        Self(
            COLLECT.with(|current| current.replace(Some(witnesses))),
            std::marker::PhantomData,
        )
    }
}

impl Drop for Collect {
    fn drop(&mut self) {
        COLLECT.with(|current| current.replace(self.0.take()));
    }
}

pub(super) fn listing(path: &Path) -> io::Result<()> {
    COLLECT.with(|current| {
        if let Some(witnesses) = current.borrow().as_ref() {
            witnesses.directory(path)
        } else if let Some(graph) = self::current() {
            graph.changes.watch_listing(path);
            graph.witnesses.directory(path)
        } else {
            Ok(())
        }
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    directory: bool,
    length: u64,
    modified: SystemTime,
    identity: Vec<u64>,
}

impl Stamp {
    fn file(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        #[cfg(windows)]
        let identity = {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FileBasicInfo,
                GetFileInformationByHandle, GetFileInformationByHandleEx,
            };
            let mut basic: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
            let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            let handle = file.as_raw_handle();
            if unsafe { GetFileInformationByHandle(handle, &raw mut information) } == 0
                || unsafe {
                    GetFileInformationByHandleEx(
                        handle,
                        FileBasicInfo,
                        (&raw mut basic).cast(),
                        size_of::<FILE_BASIC_INFO>() as u32,
                    )
                } == 0
            {
                return Err(io::Error::last_os_error());
            }
            vec![
                u64::from(information.dwVolumeSerialNumber),
                u64::from(information.nFileIndexHigh) << 32 | u64::from(information.nFileIndexLow),
                basic.LastWriteTime as u64,
                basic.ChangeTime as u64,
            ]
        };
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            vec![
                metadata.dev(),
                metadata.ino(),
                metadata.ctime() as u64,
                metadata.ctime_nsec() as u64,
            ]
        };
        #[cfg(not(any(windows, unix)))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "content generation cannot verify file identities on this platform",
        ));
        #[cfg(any(windows, unix))]
        Ok(Self {
            directory: metadata.is_dir(),
            length: metadata.len(),
            modified: metadata.modified()?,
            identity,
        })
    }

    fn path(path: &Path) -> io::Result<Option<Self>> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
            };
            options
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .access_mode(FILE_READ_ATTRIBUTES);
        }
        match options.open(path) {
            Ok(file) => {
                let mut stamp = Self::file(&file)?;
                #[cfg(windows)]
                if file.metadata()?.is_dir() && !directory_change_time(&file) {
                    stamp.identity.extend(
                        directory_digest(path)?
                            .chunks_exact(8)
                            .map(|word| u64::from_le_bytes(word.try_into().expect("digest word"))),
                    );
                }
                Ok(Some(stamp))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
fn directory_change_time(file: &File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
    let mut name = [0_u16; 32];
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    } == 0
    {
        return false;
    }
    let length = name
        .iter()
        .position(|&character| character == 0)
        .unwrap_or(name.len());
    let name = String::from_utf16_lossy(&name[..length]);
    name.eq_ignore_ascii_case("NTFS") || name.eq_ignore_ascii_case("ReFS")
}

#[cfg(windows)]
fn directory_digest(path: &Path) -> io::Result<[u8; 32]> {
    use sha2::{Digest as _, Sha256};
    let mut names = std::fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let kind = entry.file_type()?;
            Ok((
                entry.file_name(),
                [
                    u8::from(kind.is_dir()),
                    u8::from(kind.is_file()),
                    u8::from(kind.is_symlink()),
                ],
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    names.sort_unstable();
    let mut digest = Sha256::new();
    for (name, kind) in names {
        let name = name.as_encoded_bytes();
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name);
        digest.update(kind);
    }
    Ok(digest.finalize().into())
}

#[derive(Debug, PartialEq, Eq)]
struct RootProbe {
    canonical: Result<PathBuf, io::ErrorKind>,
    identity: Result<Option<([u64; 2], bool)>, io::ErrorKind>,
}

impl RootProbe {
    fn read(path: &Path) -> Self {
        Self {
            canonical: path.canonicalize().map_err(|error| error.kind()),
            identity: Stamp::path(path)
                .map(|stamp| stamp.map(|stamp| ([stamp.identity[0], stamp.identity[1]], stamp.directory)))
                .map_err(|error| error.kind()),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Witnesses(
    Mutex<BTreeMap<PathBuf, Option<Stamp>>>,
    Mutex<BTreeMap<PathBuf, RootProbe>>,
);

impl Witnesses {
    pub(super) fn root(&self, path: &Path) -> Option<PathBuf> {
        let probe = RootProbe::read(path);
        let canonical = probe.canonical.as_ref().ok().cloned();
        self.1.lock().unwrap_or_else(|error| error.into_inner())
            .entry(path.to_owned()).or_insert(probe);
        canonical
    }

    pub(super) fn inherit_listing(&self, root: &Path, prefix: &[u8], inventory: &Self) {
        let prefix = String::from_utf8_lossy(prefix)
            .trim_end_matches('/')
            .to_owned();
        let source = inventory
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut selected = Vec::new();
        let mut ancestor = None;
        for (path, stamp) in source.iter() {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let relative = relative
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if relative.starts_with(&prefix) {
                selected.push((path.clone(), stamp.clone()));
            } else if relative.is_empty()
                || prefix
                    .strip_prefix(&relative)
                    .is_some_and(|tail| tail.starts_with('/'))
            {
                if ancestor
                    .as_ref()
                    .is_none_or(|(length, _, _)| relative.len() > *length)
                {
                    ancestor = Some((relative.len(), path.clone(), stamp.clone()));
                }
            }
        }
        if selected.is_empty()
            && let Some((_, path, stamp)) = ancestor
        {
            selected.push((path, stamp));
        }
        drop(source);
        let mut own = self.0.lock().unwrap_or_else(|error| error.into_inner());
        for (path, stamp) in selected {
            own.entry(path).or_insert(stamp);
        }
    }

    pub(super) fn adopt(&self, path: &Path, inventory: &Self) -> io::Result<()> {
        if self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(path)
        {
            return Ok(());
        }
        let current = Stamp::path(path)?;
        let expected = {
            let inventory = inventory
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            inventory
                .get(path)
                .cloned()
                .or_else(|| {
                    let identity = &current.as_ref()?.identity[..2];
                    if identity[1] == 0 {
                        return None;
                    }
                    inventory
                        .values()
                        .find(|stamp| {
                            stamp
                                .as_ref()
                                .is_some_and(|stamp| &stamp.identity[..2] == identity)
                        })
                        .cloned()
                })
                .unwrap_or(current)
        };
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(path.to_owned())
            .or_insert(expected);
        Ok(())
    }

    pub(super) fn directory(&self, path: &Path) -> io::Result<()> {
        if self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(path)
        {
            return Ok(());
        }
        let stamp = Stamp::path(path)?;
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(path.to_owned())
            .or_insert(stamp);
        Ok(())
    }

    pub(super) fn record_file(&self, path: &Path, file: &File) -> io::Result<()> {
        self.record_file_stamp(path, file).map(|_| ())
    }

    fn record_file_stamp(&self, path: &Path, file: &File) -> io::Result<Stamp> {
        let stamp = Stamp::file(file)?;
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(path.to_path_buf())
            .or_insert_with(|| Some(stamp.clone()));
        Ok(stamp)
    }

    pub(super) fn file(&self, path: &Path, file: &File) -> io::Result<()> {
        let stamp = Some(Stamp::file(file)?);
        let mut witnesses = self.0.lock().unwrap_or_else(|error| error.into_inner());
        match witnesses.get(path) {
            Some(previous) if *previous != stamp => Err(changed(path)),
            Some(_) => Ok(()),
            None => {
                witnesses.insert(path.to_path_buf(), stamp);
                Ok(())
            }
        }
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        let roots = self.1.lock().unwrap_or_else(|error| error.into_inner());
        for (path, expected) in roots.iter() {
            if RootProbe::read(path) != *expected {
                return Err(changed(path));
            }
        }
        drop(roots);
        let witnesses = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        for (path, expected) in &witnesses {
            if Stamp::path(path)?.as_ref() != expected.as_ref() {
                return Err(changed(path));
            }
        }
        Ok(())
    }

    pub(super) fn validate_path(&self, path: &Path) -> io::Result<()> {
        let expected = self.0.lock().unwrap_or_else(|error| error.into_inner()).get(path).cloned();
        match expected {
            Some(expected) if Stamp::path(path)? == expected => Ok(()),
            _ => Err(changed(path)),
        }
    }
}

fn changed(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "content generation changed since resolution: {}",
            path.display()
        ),
    )
}
