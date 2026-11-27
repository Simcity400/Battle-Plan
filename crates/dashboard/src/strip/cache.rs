use super::{RAW_LIMIT, StripReply, VERSION, compute, request};
use dashboard_base::{Source, catalogue::{self, kept::{self, Place}}, products::{LoadContext, MAX_PRODUCT_REPLY_BYTES}};
use replay::campaign::timelines::Timelines;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap, fs::{self, File, OpenOptions}, io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::AtomicBool, mpsc::{SyncSender, sync_channel}},
    thread::{self, JoinHandle}, time::SystemTime,
};

const PAGE_STRIP_BYTES: u64 = 64 << 20;
const QUEUED_WRITES: usize = 4;
type Hash = blake3::Hash;
type Library = dyn Fn() -> io::Result<PathBuf> + Send + Sync;

pub(crate) struct Cache {
    root: Option<PathBuf>,
    library: Arc<Library>,
    busy: Arc<dyn Fn() -> bool + Send + Sync>,
    writer: Mutex<Option<(SyncSender<Pending>, JoinHandle<()>)>>,
}

#[derive(PartialEq, Serialize, Deserialize)]
struct Recording {
    path: PathBuf,
    revision: String,
}

pub(crate) struct Pending {
    recording: Recording,
    campaign: Hash,
    key: Hash,
    pub(super) action: Action,
}

pub(super) enum Action {
    Save(Vec<u8>),
    Touch(PathBuf),
    Remove(PathBuf),
}

impl Cache {
    pub(crate) fn new(
        root: Option<PathBuf>,
        library: impl Fn() -> io::Result<PathBuf> + Send + Sync + 'static,
        busy: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self { root, library: Arc::new(library), busy: Arc::new(busy), writer: Mutex::default() }
    }

    pub(crate) fn answer(
        &self, source: &Arc<Source>, params: &Value, load: &LoadContext, cancelled: &AtomicBool,
    ) -> io::Result<(StripReply, Option<Pending>)> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let ask = request(params)?;
        let cacheable = self.root.as_ref().and_then(|root| Some((root, key(source, params, load)?)));
        if let Some((root, (revision, key))) = cacheable {
            let campaign = blake3::hash(&source.replay_ref().campaign_id);
            let pending = |action| Pending {
                recording: Recording { path: source.path().into(), revision: revision.to_hex().to_string() },
                campaign, key, action,
            };
            for folder in [root.join(campaign.to_hex().as_str()), root.join("external")] {
                let path = group(&folder, key);
                match read(&path, key) {
                    Ok(bytes) => {
                        return Ok((StripReply::from_bytes(&bytes), Some(pending(Action::Touch(path)))));
                    }
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        self.store(pending(Action::Remove(path)));
                    }
                    Err(_) => {}
                }
            }
            let (bytes, complete) = compute(source, &ask, load, cancelled, &*self.busy)?;
            Ok((StripReply::from_bytes(&bytes), complete.then(|| pending(Action::Save(bytes)))))
        } else {
            let (bytes, _) = compute(source, &ask, load, cancelled, &*self.busy)?;
            Ok((StripReply::from_bytes(&bytes), None))
        }
    }

    pub(crate) fn store(&self, pending: Pending) -> bool {
        if (self.busy)() { return false; }
        let Some(root) = &self.root else { return false };
        let Ok(mut writer) = self.writer.lock() else { return false };
        if writer.is_none() {
            let (root, library, busy) = (root.clone(), Arc::clone(&self.library), Arc::clone(&self.busy));
            let (send, receive) = sync_channel::<Pending>(QUEUED_WRITES);
            let Ok(thread) = thread::Builder::new().name("dashboard strip cache".into()).spawn(move || {
                job_priority::background_thread();
                let ready = || if busy() { Err(io::ErrorKind::WouldBlock.into()) } else { Ok(()) };
                let mut disk = None;
                for pending in receive {
                    let _ = Disk::keep(&mut disk, &root, &*library, pending, &ready);
                }
            }) else { return false };
            *writer = Some((send, thread));
        }
        writer.as_ref().is_some_and(|(send, _)| send.try_send(pending).is_ok())
    }

    pub(crate) fn shutdown(&self) {
        if let Some((send, thread)) = self.writer.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            drop(send);
            let _ = thread.join();
        }
    }
}

pub(super) fn key(source: &Source, params: &Value, load: &LoadContext) -> Option<(Hash, Hash)> {
    let replay = source.replay_ref();
    if replay.cursors.last().is_some_and(|cursor| !cursor.finished) { return None; }
    let revision = replay.revision();
    let content = if let Some(content) = &load.content {
        let presented = source.presented_for(content).ok()?;
        Some((presented.content.layers.art_stack_key(), catalogue::binding(content)))
    } else {
        None
    };
    StripKey {
        file: FileKey {
            path: source.path().as_os_str().as_encoded_bytes(),
            stamp: &source.key().revision,
            root: revision.root,
            number: revision.number,
            length: revision.length,
        },
        version: VERSION,
        campaign: &replay.campaign_id,
        timelines: replay.timelines(),
        build: kept::build()?,
        artwork: dashboard_text::artwork::base(),
        content: content.as_ref(),
        params,
    }.hashes()
}

#[derive(Clone, Copy, Serialize)]
struct FileKey<'a> {
    path: &'a [u8],
    stamp: &'a str,
    root: [u8; 16],
    number: u64,
    length: u64,
}

#[derive(Clone, Copy, Serialize)]
struct StripKey<'a> {
    file: FileKey<'a>,
    version: u8,
    campaign: &'a [u8],
    #[serde(serialize_with = "serialize_timelines")]
    timelines: Option<&'a Timelines>,
    build: &'a (String, [u8; 24]),
    artwork: &'a str,
    #[serde(serialize_with = "serialize_content")]
    content: Option<&'a ([u8; 32], catalogue::Binding)>,
    params: &'a Value,
}

impl StripKey<'_> {
    fn hashes(&self) -> Option<(Hash, Hash)> {
        fn digest(value: &impl Serialize) -> Option<Hash> {
            let mut hash = blake3::Hasher::new();
            serde_json::to_writer(&mut hash, value).ok()?;
            Some(hash.finalize())
        }
        Some((digest(&self.file)?, digest(&("dashboard strip cache/v2", self))?))
    }
}

fn serialize_timelines<S: serde::Serializer>(timelines: &Option<&Timelines>, serializer: S) -> Result<S::Ok, S::Error> {
    timelines.map(|timelines| {
        let branches: Vec<_> = timelines.list.iter().map(|branch| {
            let pieces: Vec<_> = branch.pieces.iter().map(|piece| {
                (piece.segment, piece.first_hour, piece.last_hour, piece.declared)
            }).collect();
            let joins: Vec<_> = branch.joins.iter().map(|join| {
                (join.hour, join.segment, join.parent, join.evidence as u8, &join.candidates)
            }).collect();
            (branch.id, pieces, joins)
        }).collect();
        let links: Vec<_> = timelines.links.iter().map(|link| {
            (link.segment, link.parent, link.evidence as u8, &link.candidates, timelines.home(link.segment))
        }).collect();
        (timelines.main, branches, links)
    }).serialize(serializer)
}

fn serialize_content<S: serde::Serializer>(
    content: &Option<&([u8; 32], catalogue::Binding)>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    content.map(|(stack, ((game, user, allow_unverified), cache, presentation, settings))| {
        let game = game.as_os_str().as_encoded_bytes();
        let user = user.as_os_str().as_encoded_bytes();
        let cache = cache.as_os_str().as_encoded_bytes();
        (stack, ((game, user, allow_unverified), cache, presentation, settings))
    }).serialize(serializer)
}

fn marker(folder: &Path) -> Place {
    Place::new(folder.join("recording"), *blake3::hash(b"dashboard strip cache recording/v2").as_bytes())
}

fn group(folder: &Path, key: Hash) -> PathBuf {
    folder.join(format!("{key}.bpst"))
}

fn read(path: &Path, key: Hash) -> io::Result<Vec<u8>> {
    let payload = Place::new(path.to_owned(), *key.as_bytes()).load(MAX_PRODUCT_REPLY_BYTES)?;
    payload.split_first_chunk::<4>().and_then(|(length, compressed)| {
        let size = u32::from_le_bytes(*length) as usize;
        if !(8..=RAW_LIMIT).contains(&size) { return None; }
        zstd::bulk::decompress(compressed, size).ok().filter(|bytes| bytes.len() == size)
    }).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid cached strip"))
}

type Ready<'a> = &'a dyn Fn() -> io::Result<()>;

struct Disk {
    _owner: File,
    active: Option<Archive>,
}

impl Disk {
    fn open(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let owner = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join("writer.lock"))?;
        owner.try_lock().map_err(io::Error::from)?;
        Ok(Self { _owner: owner, active: None })
    }

    fn keep(disk: &mut Option<Self>, root: &Path, library: &Library, pending: Pending, ready: Ready<'_>) -> io::Result<()> {
        let Pending { recording, campaign, key, action } = pending;
        ready()?;
        let library_now = library()?;
        let library_entries = fs::read_dir(&library_now)?;
        ready()?;
        let disk = match disk {
            Some(disk) => disk,
            None => disk.insert(Self::open(root)?),
        };
        let bytes = match action {
            Action::Touch(path) => return touch(&path, ready),
            Action::Remove(path) => {
                return if read(&path, key).is_err_and(|error| error.kind() == io::ErrorKind::InvalidData) {
                    remove(&path, ready)
                } else { Ok(()) };
            }
            Action::Save(bytes) => bytes,
        };
        let folder = root.join(if recording.path.starts_with(&library_now) { campaign.to_hex().to_string() } else { "external".into() });
        if disk.active.as_ref().is_none_or(|old| old.folder != folder || old.recording != recording || old.library != library_now) {
            let gone = displaced(root, &library_now, library_entries, &folder, ready)?;
            if library()? != library_now { return Ok(()); }
            for path in gone {
                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    if entry.file_type()?.is_file() { remove(&entry.path(), ready)?; }
                }
                ready()?;
                fs::remove_dir(path)?;
            }
            disk.active = Some(Archive::open(folder, recording, library_now, ready)?);
        }
        let archive = disk.active.as_mut().unwrap();
        let path = group(&archive.folder, key);
        ready()?;
        let size = bytes.len() as u64;
        let mut payload = (size as u32).to_le_bytes().to_vec();
        payload.extend(zstd::bulk::compress(&bytes, 3)?);
        archive.reserve(&path, size, ready)?;
        Place::new(path.clone(), *key.as_bytes()).save_serial(&payload, ready)?;
        archive.bytes = archive.bytes + size - archive.entries.insert(path, size).unwrap_or_default();
        Ok(())
    }
}

fn previous(folder: &Path) -> io::Result<Option<Recording>> {
    match marker(folder).load(MAX_PRODUCT_REPLY_BYTES) {
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::InvalidData) => Ok(None),
        result => Ok(Some(serde_json::from_slice(&result?)?)),
    }
}

fn displaced(root: &Path, library: &Path, library_entries: fs::ReadDir, current: &Path, ready: Ready<'_>) -> io::Result<Vec<PathBuf>> {
    for entry in library_entries { ready()?; entry?; }
    let mut gone = Vec::new();
    for entry in fs::read_dir(root)? {
        ready()?;
        let entry = entry?;
        let path = entry.path();
        if path == current || !entry.file_type()?.is_dir() { continue; }
        let Ok(campaign) = Hash::from_hex(entry.file_name().as_encoded_bytes()) else { continue };
        let Some(recording) = previous(&path)? else { gone.push(path); continue };
        let valid = if recording.path.starts_with(library) {
            match replay::campaign::Replay::read_campaign_id(&recording.path) {
                Ok(found) => blake3::hash(&found) == campaign,
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(_) => continue,
            }
        } else { false };
        if !valid { gone.push(path); }
    }
    Ok(gone)
}

struct Archive {
    folder: PathBuf,
    recording: Recording,
    library: PathBuf,
    entries: HashMap<PathBuf, u64>,
    bytes: u64,
}

impl Archive {
    fn open(folder: PathBuf, recording: Recording, library: PathBuf, ready: Ready<'_>) -> io::Result<Self> {
        let matched = previous(&folder)?.as_ref() == Some(&recording);
        let mut entries = HashMap::new();
        ready()?;
        fs::create_dir_all(&folder)?;
        for entry in fs::read_dir(&folder)? {
            ready()?;
            let entry = entry?;
            if !entry.file_type()?.is_file() { continue; }
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "bpst") { continue; }
            if matched && let Ok(size) = raw_size(&path) { entries.insert(path, size); }
            else { remove(&path, ready)?; }
        }
        if !matched { marker(&folder).save_serial(&serde_json::to_vec(&recording)?, ready)?; }
        let bytes = entries.values().sum();
        Ok(Self { folder, recording, library, entries, bytes })
    }

    fn reserve(&mut self, path: &Path, size: u64, ready: Ready<'_>) -> io::Result<()> {
        let additional = size.saturating_sub(self.entries.get(path).copied().unwrap_or_default());
        if self.bytes + additional <= PAGE_STRIP_BYTES { return Ok(()); }
        let mut oldest: Vec<_> = self.entries.keys().filter(|kept| kept.as_path() != path).map(|path| {
            (path.metadata().and_then(|meta| meta.modified()).unwrap_or(SystemTime::UNIX_EPOCH), path.clone())
        }).collect();
        oldest.sort_unstable();
        for (_, entry) in oldest {
            remove(&entry, ready)?;
            self.bytes -= self.entries.remove(&entry).unwrap();
            if self.bytes + additional <= PAGE_STRIP_BYTES { break; }
        }
        Ok(())
    }
}

fn remove(path: &Path, ready: Ready<'_>) -> io::Result<()> {
    ready()?;
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn touch(path: &Path, ready: Ready<'_>) -> io::Result<()> {
    ready()?;
    OpenOptions::new().write(true).open(path)?.set_modified(SystemTime::now())
}

fn raw_size(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(Place::PAYLOAD_OFFSET))?;
    let mut bytes = [0; 4];
    file.read_exact(&mut bytes)?;
    let size = u32::from_le_bytes(bytes) as u64;
    if !(8..=RAW_LIMIT as u64).contains(&size) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid strip size"));
    }
    Ok(size)
}

#[cfg(test)]
mod tests;
