
use replay::campaign::{Replay, is_older_layout};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub const EXTENSION: &str = "battleplan";
const LOCK: &str = "recording-names.lock";
const MAX_COMPONENT: usize = 255;
pub const MAX_STEM: usize = MAX_COMPONENT - ".battleplan".len() - " (18446744073709551615)".len();

pub fn is_recording(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION))
}

fn at(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

pub fn campaign_of(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match Replay::read_campaign_uuid(path) {
        Err(error) if is_older_layout(&error) => Ok(None),
        result => result.map_err(|error| at(path, error)),
    }
}

pub type Found = BTreeMap<Vec<u8>, Vec<PathBuf>>;

pub fn recordings(folder: &Path) -> io::Result<Found> {
    let mut found = BTreeMap::<_, Vec<_>>::new();
    let entries = match fs::read_dir(folder) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        entries => entries.map_err(|error| at(folder, error))?,
    };
    for entry in entries {
        let entry = entry.map_err(|error| at(folder, error))?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| at(&path, error))?
            .is_file()
            && is_recording(&path)
            && let Some(campaign) = campaign_of(&path)?
        {
            found
                .entry(campaign.to_ascii_lowercase())
                .or_default()
                .push(path);
        }
    }
    found.values_mut().for_each(|paths| paths.sort());
    Ok(found)
}

pub fn listing(folder: &Path) -> io::Result<Vec<(PathBuf, u64, std::time::SystemTime)>> {
    let mut listed = Vec::new();
    let entries = match fs::read_dir(folder) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(listed),
        entries => entries.map_err(|error| at(folder, error))?,
    };
    for entry in entries {
        let entry = entry.map_err(|error| at(folder, error))?;
        let metadata = entry.metadata().map_err(|error| at(&entry.path(), error))?;
        if metadata.is_file() && is_recording(&entry.path()) {
            listed.push((entry.path(), metadata.len(), metadata.modified()?));
        }
    }
    listed.sort();
    Ok(listed)
}

pub fn recordings_of(folder: &Path, campaign: &[u8]) -> io::Result<Vec<PathBuf>> {
    Ok(recordings(folder)?
        .remove(&campaign.to_ascii_lowercase())
        .unwrap_or_default())
}

pub fn one_recording(
    folder: &Path,
    campaign: &[u8],
    found: Vec<PathBuf>,
) -> io::Result<Option<PathBuf>> {
    if found.len() > 1 {
        let names: Vec<_> = found
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy())
            .collect();
        return Err(io::Error::other(format!(
            "{} holds {} recordings of campaign {} ({}); keep one there and move the others",
            folder.display(),
            found.len(),
            String::from_utf8_lossy(campaign),
            names.join(", ")
        )));
    }
    Ok(found.into_iter().next())
}

pub fn recording_of(folder: &Path, campaign: &[u8]) -> io::Result<Option<PathBuf>> {
    one_recording(folder, campaign, recordings_of(folder, campaign)?)
}

pub fn uncommitted(path: &Path) -> io::Result<bool> {
    use replay_columnar::envelope::{FOOTER_BYTES, HEADER_BYTES, check_header};
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    if length == 0 {
        return Ok(true);
    }
    if length < HEADER_BYTES as u64 {
        return Ok(false);
    }
    let mut header = [0; HEADER_BYTES];
    io::Read::read_exact(&mut &file, &mut header)?;
    if check_header(&header).is_err() {
        return Ok(false);
    }
    if length < (HEADER_BYTES + FOOTER_BYTES) as u64 {
        return Ok(true);
    }
    Ok(replay_columnar::container::Footers::new(&file)?.next()?.is_none())
}

pub fn unsaved_recordings(folder: &Path) -> io::Result<BTreeMap<Vec<u8>, Result<PathBuf, String>>> {
    let unsaved = replay_settings::publication::unsaved_folder(folder);
    let entries = match fs::read_dir(&unsaved) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        entries => entries.map_err(|error| at(&unsaved, error))?,
    };
    let mut claimed = BTreeMap::<Vec<u8>, Vec<(PathBuf, bool)>>::new();
    for entry in entries {
        let entry = entry.map_err(|error| at(&unsaved, error))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| at(&path, error))?
            .is_file()
            || !is_recording(&path)
        {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.as_bytes().to_ascii_lowercase())
            .filter(|stem| replay_data::identity::is_campaign_uuid(stem));
        let header = match campaign_of(&path) {
            Ok(header) => header.map(|campaign| campaign.to_ascii_lowercase()),
            Err(error) => match uncommitted(&path) {
                Ok(true) => name.clone(),
                _ => return Err(error),
            },
        };
        let own = header.is_some() && header == name;
        for campaign in [header, name.filter(|_| !own)].into_iter().flatten() {
            claimed
                .entry(campaign)
                .or_default()
                .push((path.clone(), own));
        }
    }
    Ok(claimed
        .into_iter()
        .map(|(campaign, files)| {
            let found = match files.as_slice() {
                [(path, true)] => Ok(path.clone()),
                files => {
                    let mut names: Vec<_> = files
                        .iter()
                        .filter_map(|(path, _)| path.file_name())
                        .map(|name| name.to_string_lossy())
                        .collect();
                    names.sort();
                    let campaign = String::from_utf8_lossy(&campaign);
                    Err(format!(
                        "{}: the unsaved recording of campaign {campaign} is ambiguous ({}); only {campaign}.{EXTENSION}, its header naming that campaign, may be there",
                        unsaved.display(),
                        names.join(", "),
                    ))
                }
            };
            (campaign, found)
        })
        .collect())
}

pub fn unsaved_recording(folder: &Path, campaign: &[u8]) -> io::Result<Option<PathBuf>> {
    unsaved_recordings(folder)?
        .remove(&campaign.to_ascii_lowercase())
        .transpose()
        .map_err(io::Error::other)
}

pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

pub fn truncate(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (at, character) in text.char_indices() {
        used += character.len_utf16();
        if used > units {
            return &text[..at];
        }
    }
    text
}

const DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

fn device_name(base: &str) -> bool {
    let base = base.trim_end_matches(' ');
    if DEVICES
        .iter()
        .any(|device| base.eq_ignore_ascii_case(device))
    {
        return true;
    }
    let mut characters = base.chars();
    let prefix: String = characters.by_ref().take(3).collect();
    let rest: String = characters.collect();
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(
            rest.as_str(),
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
}

pub fn file_stem(text: &str) -> Option<String> {
    let spaced: String = text
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                ' '
            } else {
                character
            }
        })
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut stem = truncate(&joined, MAX_STEM)
        .trim_end_matches([' ', '.'])
        .to_owned();
    if stem.is_empty() {
        return None;
    }
    let base = stem.split('.').next().unwrap_or_default().len();
    if device_name(&stem[..base]) {
        stem.insert(base, '_');
    }
    Some(stem)
}

fn numbered(folder: &Path, stem: &str, number: u64) -> PathBuf {
    folder.join(if number == 1 {
        format!("{stem}.{EXTENSION}")
    } else {
        format!("{stem} ({number}).{EXTENSION}")
    })
}

pub fn unique_path(folder: &Path, stem: &str, own: Option<&Path>) -> io::Result<PathBuf> {
    let own = own.map(fs::canonicalize).transpose()?;
    for number in 1_u64.. {
        let path = numbered(folder, stem, number);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(path),
            Err(error) => return Err(at(&path, error)),
            Ok(_) if own.is_some() && own == Some(fs::canonicalize(&path)?) => return Ok(path),
            Ok(_) => {}
        }
    }
    Err(io::Error::other("no free recording name"))
}

pub fn stem_is(stem: &str, wanted: &str) -> bool {
    stem == wanted
        || stem
            .strip_prefix(wanted)
            .and_then(|rest| rest.strip_prefix(" ("))
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|number| {
                number.parse::<u64>().is_ok_and(|number| number >= 2) && !number.starts_with('0')
            })
}

pub fn rename(from: &Path, stem: &str) -> io::Result<PathBuf> {
    let folder = from
        .parent()
        .ok_or_else(|| io::Error::other("recording has no folder"))?;
    loop {
        let to = unique_path(folder, stem, Some(from))?;
        if to.file_name() == from.file_name() {
            return Ok(to);
        }
        match replay_settings::publication::rename_new(from, &to) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            result => return result.map(|()| to),
        }
    }
}

pub fn place_new(source: &Path, folder: &Path, stem: &str) -> io::Result<PathBuf> {
    for number in 1_u64.. {
        let target = numbered(folder, stem, number);
        match replay_settings::publication::rename_new(source, &target) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            result => return result.map(|()| target),
        }
    }
    Err(io::Error::other("no free recording name"))
}

pub fn publish_new(
    folder: &Path,
    stem: &str,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let temporary = folder.join(format!("{stem}.{EXTENSION}.{}.tmp", std::process::id()));
    let placed = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            write(&mut file)?;
            file.sync_all()
        })
        .and_then(|()| place_new(&temporary, folder, stem));
    if placed.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    placed
}

pub struct NamesLock {
    _held: File,
}

impl NamesLock {
    fn open(folder: &Path) -> io::Result<File> {
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;
        fs::create_dir_all(folder)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .attributes(FILE_ATTRIBUTE_HIDDEN)
            .open(folder.join(LOCK))
    }

    pub fn wait(folder: &Path) -> io::Result<Self> {
        let lock = Self::open(folder)?;
        lock.lock()?;
        Ok(Self { _held: lock })
    }

    pub fn try_now(folder: &Path) -> io::Result<Option<Self>> {
        let lock = Self::open(folder)?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(Self { _held: lock })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hoi4-recording-files-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn header(path: &Path, campaign: &str) {
        let session = replay_data::identity::SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
            capture_mode: replay_data::identity::CaptureMode::Full,
            identity: replay_data::identity::Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "synthetic".into(),
                collector_hash: [3; 32],
                verification: replay_data::identity::Verification::Unchecked,
            },
        };
        let bytes = replay::campaign::fixtures::encode_records(
            recorder_derive::writers(),
            campaign.as_bytes().to_vec(),
            &[vec![replay_capture_spool::spool::Record::SessionStart(session)]],
        )
        .unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn names_become_safe_windows_file_names() {
        assert_eq!(file_stem("Germany 1936–1941").unwrap(), "Germany 1936–1941");
        assert_eq!(
            file_stem(" Kingdom: of <the> \"North\"/South|?* \t1936 ").unwrap(),
            "Kingdom of the North South 1936"
        );
        assert_eq!(
            file_stem("Name with\u{7}control").unwrap(),
            "Name with control"
        );
        assert_eq!(file_stem("Trailing dots... ").unwrap(), "Trailing dots");
        for device in ["CON", "nul", "Com1", "LPT9", "COM¹", "aux.backup", "PRN "] {
            let stem = file_stem(device).unwrap();
            let base = stem.split('.').next().unwrap();
            assert!(!device_name(base), "{device} became {stem}");
        }
        assert_eq!(file_stem("aux.backup").unwrap(), "aux_.backup");
        assert_eq!(file_stem("CONSOLE 1936").unwrap(), "CONSOLE 1936");
        assert_eq!(file_stem("COM10").unwrap(), "COM10");
        for empty in ["", "  ", "...", "?*:", "\u{0}"] {
            assert_eq!(file_stem(empty), None, "{empty:?}");
        }
        let long = "Ж".repeat(400) + "😀";
        let stem = file_stem(&long).unwrap();
        assert_eq!(utf16_len(&stem), MAX_STEM);
        let name = format!("{stem} (18446744073709551615).{EXTENSION}");
        assert!(utf16_len(&name) <= MAX_COMPONENT);
        assert_eq!(truncate("a😀b", 2), "a");
        assert_eq!(truncate("a😀b", 3), "a😀");
    }

    #[test]
    fn a_taken_name_gets_the_next_number_as_windows_names_copies() {
        let folder = scratch("unique");
        assert_eq!(
            unique_path(&folder, "Germany 1936", None).unwrap(),
            folder.join("Germany 1936.battleplan")
        );
        fs::write(folder.join("Germany 1936.battleplan"), b"").unwrap();
        fs::write(folder.join("germany 1936 (2).battleplan"), b"").unwrap();
        assert_eq!(
            unique_path(&folder, "Germany 1936", None).unwrap(),
            folder.join("Germany 1936 (3).battleplan")
        );
        let own = folder.join("germany 1936 (2).battleplan");
        assert_eq!(
            unique_path(&folder, "Germany 1936", Some(&own)).unwrap(),
            folder.join("Germany 1936 (2).battleplan")
        );
        let error = unique_path(&folder, "bad\0name", None).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(stem_is("Germany 1936", "Germany 1936"));
        assert!(stem_is("Germany 1936 (2)", "Germany 1936"));
        assert!(stem_is("Germany 1936 (17)", "Germany 1936"));
        for other in [
            "Germany 1936 (1)",
            "Germany 1936 (02)",
            "Germany 1936 (x)",
            "Germany 1937",
            "Germany 1936 (2) (2)",
        ] {
            assert!(!stem_is(other, "Germany 1936"), "{other}");
        }
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_rename_never_replaces_another_file() {
        let folder = scratch("rename");
        let from = folder.join("a.battleplan");
        fs::write(&from, b"recording").unwrap();
        fs::write(folder.join("Germany 1936.battleplan"), b"another").unwrap();
        let to = rename(&from, "Germany 1936").unwrap();
        assert_eq!(to, folder.join("Germany 1936 (2).battleplan"));
        assert_eq!(fs::read(&to).unwrap(), b"recording");
        assert_eq!(
            fs::read(folder.join("Germany 1936.battleplan")).unwrap(),
            b"another"
        );
        assert!(!from.exists());
        assert_eq!(rename(&to, "Germany 1936").unwrap(), to);
        let cased = rename(&to, "GERMANY 1936 (2)").unwrap();
        assert_eq!(cased.file_name().unwrap(), "GERMANY 1936 (2).battleplan");
        assert_eq!(fs::read(&cased).unwrap(), b"recording");
        let error = replay_settings::publication::rename_new(&cased, &folder.join("Germany 1936.battleplan"))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_new_recording_never_replaces_a_file_at_its_name() {
        let folder = scratch("publish");
        let published = publish_new(&folder, "Germany 1936", |file| {
            io::Write::write_all(file, b"recording")?;
            fs::write(folder.join("Germany 1936.battleplan"), b"another")
        })
        .unwrap();
        assert_eq!(published, folder.join("Germany 1936 (2).battleplan"));
        assert_eq!(fs::read(&published).unwrap(), b"recording");
        assert_eq!(
            fs::read(folder.join("Germany 1936.battleplan")).unwrap(),
            b"another"
        );
        let left: Vec<_> = fs::read_dir(&folder).unwrap().collect();
        assert_eq!(left.len(), 2, "no temporary file is left");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_held_recording_keeps_its_name() {
        let folder = scratch("held");
        let from = folder.join("a.battleplan");
        fs::write(&from, b"recording").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
            .open(&from)
            .unwrap();
        assert!(rename(&from, "Germany 1936").is_err());
        assert!(from.exists() && !folder.join("Germany 1936.battleplan").exists());
        drop(held);
        assert!(rename(&from, "Germany 1936").is_ok());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_campaigns_recording_is_found_by_its_header_not_its_name() {
        let folder = scratch("find");
        let campaign = "00000000-0000-4000-8000-000000000000";
        let other = "00000000-0000-4000-8000-000000000001";
        assert_eq!(recording_of(&folder, campaign.as_bytes()).unwrap(), None);
        assert_eq!(
            recording_of(&folder.join("missing"), campaign.as_bytes()).unwrap(),
            None
        );
        header(&folder.join("Germany 1936–1941.battleplan"), campaign);
        header(&folder.join(format!("{other}.battleplan")), other);
        header(&folder.join("same header.txt"), campaign);
        header(&folder.join("not a campaign.battleplan"), "a save name");
        fs::create_dir_all(folder.join("Germany (2)")).unwrap();
        header(
            &folder.join("Germany (2)").join("Germany 1940.battleplan"),
            campaign,
        );
        assert_eq!(
            recording_of(&folder, campaign.as_bytes()).unwrap(),
            Some(folder.join("Germany 1936–1941.battleplan"))
        );
        assert_eq!(
            recording_of(&folder, campaign.to_uppercase().as_bytes()).unwrap(),
            Some(folder.join("Germany 1936–1941.battleplan"))
        );
        header(
            &folder.join("Germany 1936–1941 - Copy.battleplan"),
            campaign,
        );
        let error = recording_of(&folder, campaign.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(error.contains("2 recordings"), "{error}");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_recording_that_cannot_be_read_is_an_error_not_an_absence() {
        let folder = scratch("unreadable");
        let campaign = b"00000000-0000-4000-8000-000000000000";
        fs::write(folder.join("damaged.battleplan"), b"junk").unwrap();
        let error = recording_of(&folder, campaign).unwrap_err().to_string();
        assert!(error.contains("damaged.battleplan"), "{error}");
        fs::remove_file(folder.join("damaged.battleplan")).unwrap();
        let held_path = folder.join("held.battleplan");
        header(&held_path, "00000000-0000-4000-8000-000000000000");
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&held_path)
            .unwrap();
        assert!(recording_of(&folder, campaign).is_err());
        drop(held);
        assert_eq!(recording_of(&folder, campaign).unwrap(), Some(held_path));
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_unsaved_recording_with_no_revision_yet_is_its_campaign_s_by_its_name_alone() {
        let folder = scratch("uncommitted");
        let saved = "00000000-0000-4000-8000-000000000000";
        let fresh = "00000000-0000-4000-8000-000000000001";
        let first = replay_settings::publication::unsaved_path(&folder, saved);
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        header(&first, saved);
        let second = replay_settings::publication::unsaved_path(&folder, fresh);
        header(&second, fresh);
        let bytes = fs::read(&second).unwrap();
        fs::write(
            &second,
            &bytes[..bytes.len() - replay_columnar::envelope::FOOTER_BYTES],
        )
        .unwrap();
        assert!(campaign_of(&second).is_err(), "it has no header to read");
        let found = unsaved_recordings(&folder).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            unsaved_recording(&folder, saved.as_bytes()).unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            unsaved_recording(&folder, fresh.as_bytes()).unwrap(),
            Some(second.clone())
        );
        fs::write(&second, b"").unwrap();
        assert_eq!(
            unsaved_recording(&folder, fresh.as_bytes()).unwrap(),
            Some(second.clone())
        );
        let unnamed = second.with_file_name("partial.battleplan");
        fs::rename(&second, &unnamed).unwrap();
        assert_eq!(unsaved_recording(&folder, fresh.as_bytes()).unwrap(), None);
        assert_eq!(
            unsaved_recording(&folder, saved.as_bytes()).unwrap(),
            Some(first)
        );
        fs::write(&unnamed, b"junk").unwrap();
        let error = unsaved_recordings(&folder).unwrap_err().to_string();
        assert!(error.contains("partial.battleplan"), "{error}");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn the_naming_lock_is_one_holder_at_a_time_and_hidden() {
        use std::os::windows::fs::MetadataExt;
        let folder = scratch("lock");
        let lock = NamesLock::wait(&folder).unwrap();
        assert!(NamesLock::try_now(&folder).unwrap().is_none());
        let attributes = fs::metadata(folder.join(LOCK)).unwrap().file_attributes();
        assert_ne!(
            attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
            0
        );
        drop(lock);
        assert!(NamesLock::try_now(&folder).unwrap().is_some());
        fs::remove_dir_all(folder).unwrap();
    }
}
