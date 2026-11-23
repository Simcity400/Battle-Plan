use super::*;
use std::sync::mpsc;

const HELD_FILE: &str = "outer/layer/gfx/models/a.dds";
const HELD_ARCHIVE: &str = "outer/layer/gfx/models/pack.zip";
const LISTED: &str = "outer/layer/gfx/models";
const ABOVE: [&str; 4] = [
    "outer/layer/gfx/models",
    "outer/layer/gfx",
    "outer/layer",
    "outer",
];

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hoi4_sharing_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let models = root.join(LISTED);
    fs::create_dir_all(&models).unwrap();
    fs::create_dir_all(root.join("outer/layer/map")).unwrap();
    fs::create_dir_all(root.join("elsewhere")).unwrap();
    fs::write(root.join(HELD_FILE), b"held bytes").unwrap();
    fs::write(models.join("b.dds"), b"b").unwrap();
    fs::write(
        root.join(HELD_ARCHIVE),
        tests::zip_records(
            &[],
            &[tests::Record {
                name: b"gfx/models/zipped.dds",
                host: 0,
                flags: 0,
                deflate: false,
                data: b"zipped",
            }],
        ),
    )
    .unwrap();
    fs::write(root.join("outer/layer/map/covered.txt"), b"covered").unwrap();
    fs::canonicalize(root).unwrap()
}

#[derive(Clone, Copy, Debug)]
enum Change {
    Rename,
    Move,
    Delete,
}

const CHANGES: [Change; 3] = [Change::Rename, Change::Move, Change::Delete];

fn apply(root: &Path, object: &str, change: Change) -> io::Result<()> {
    let path = root.join(object);
    let name = path.file_name().unwrap().to_owned();
    match change {
        Change::Rename => {
            let mut renamed = name;
            renamed.push("_renamed");
            fs::rename(&path, path.with_file_name(renamed))
        }
        Change::Move => fs::rename(&path, root.join("elsewhere").join(name)),
        Change::Delete if path.is_dir() => fs::remove_dir_all(&path),
        Change::Delete => fs::remove_file(&path),
    }
}

type Held = Result<(io::Result<()>, io::Result<String>), String>;

fn while_held<H>(
    hold: impl FnOnce() -> io::Result<H> + Send,
    use_held: impl FnOnce(H) -> io::Result<String> + Send,
    change: impl FnOnce() -> io::Result<()>,
) -> Held {
    let (ready, opened) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    std::thread::scope(|scope| {
        let holder = scope.spawn(move || {
            let held = match hold() {
                Ok(held) => held,
                Err(error) => {
                    let _ = ready.send(Err(error.to_string()));
                    return Err(error);
                }
            };
            let _ = ready.send(Ok(()));
            let _ = released.recv();
            use_held(held)
        });
        let changed = opened
            .recv()
            .unwrap_or_else(|_| Err("the holder stopped before opening".to_owned()))
            .map(|()| change());
        drop(release);
        let used = holder
            .join()
            .map_err(|_| "the holder panicked".to_owned())?;
        changed.map(|changed| (changed, used))
    })
}

#[test]
fn a_failed_holder_reports_startup_failure_without_attempting_the_change() {
    let changed = std::cell::Cell::new(false);
    let held = while_held::<()>(
        || {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "synthetic open failure",
            ))
        },
        |_| panic!("a failed open has no held handle"),
        || {
            changed.set(true);
            Ok(())
        },
    );
    assert!(matches!(held, Err(why) if why == "synthetic open failure"));
    assert!(!changed.get());
}

fn held_matrix<H>(
    name: &str,
    objects: &[&str],
    hold: impl Fn(&Path) -> io::Result<H> + Sync,
    use_held: impl Fn(H) -> io::Result<String> + Sync,
    expected: Option<&str>,
) {
    let mut obstacles = Vec::new();
    for (at, object) in objects.iter().enumerate() {
        for change in CHANGES {
            if at > 0 && matches!(change, Change::Rename | Change::Move) {
                continue;
            }
            let root = scratch(&format!("{name}{at}{change:?}"));
            let held = while_held(|| hold(&root), &use_held, || apply(&root, object, change));
            match &held {
                Ok((Ok(()), Ok(used))) if expected.is_none_or(|expected| used == expected) => {}
                _ => obstacles.push(format!("{change:?} {object}: {held:?}")),
            }
            let _ = fs::remove_dir_all(&root);
        }
    }
    assert!(
        obstacles.is_empty(),
        "while a {name} handle was held:\n{}",
        obstacles.join("\n")
    );
}

#[test]
fn a_held_folder_read_shares_direct_changes_and_ancestor_deletion() {
    let objects: Vec<&str> = std::iter::once(HELD_FILE).chain(ABOVE).collect();
    held_matrix(
        "file",
        &objects,
        |root| folder_file(&root.join("outer/layer"), "gfx/models/a.dds").map(|(file, _)| file),
        |mut file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        },
        Some("held bytes"),
    );
}

#[test]
fn a_held_archive_read_shares_direct_changes_and_ancestor_deletion() {
    let objects: Vec<&str> = std::iter::once(HELD_ARCHIVE).chain(ABOVE).collect();
    held_matrix(
        "archive",
        &objects,
        |root| physfs_zip::Archive::open(&root.join(HELD_ARCHIVE)),
        |mut archive| {
            archive
                .mount()
                .map(|mounted| format!("{} records", mounted.entries.len()))
        },
        Some("1 records"),
    );
}

#[test]
fn a_held_listing_shares_direct_changes_and_ancestor_deletion() {
    held_matrix(
        "listing",
        &ABOVE,
        |root| {
            let mut entries = fs::read_dir(root.join(LISTED))?;
            entries.next().transpose()?;
            Ok(entries)
        },
        |entries| Ok(format!("{} more", entries.count())),
        None,
    );
}

#[test]
fn finished_reads_leave_nothing_that_blocks_a_change() {
    let objects: Vec<&str> = std::iter::once(HELD_FILE).chain(ABOVE).collect();
    let mut obstacles = Vec::new();
    for (at, object) in objects.iter().enumerate() {
        for change in CHANGES {
            let root = scratch(&format!("after{at}{change:?}"));
            let layer = root.join("outer/layer");
            let layers = ResolvedLayers::from_folders(&[(layer.as_path(), &[][..])]).unwrap();
            let _ = layers.report_uncovered_reads();
            assert_eq!(
                layers.read_uncovered_path(b"gfx/models/a.dds").unwrap(),
                b"held bytes"
            );
            assert_eq!(layers.read("map/covered.txt").unwrap(), b"covered");
            assert_eq!(
                layers
                    .read_uncovered_path(b"gfx/models/missing.dds")
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::NotFound
            );
            fs::write(layer.join("map/covered.txt"), b"changed").unwrap();
            assert!(layers.read("map/covered.txt").is_err());
            if let Err(error) = apply(&root, object, change) {
                obstacles.push(format!("{change:?} {object}: {error}"));
            }
            drop(layers);
            let _ = fs::remove_dir_all(&root);
        }
    }
    assert!(
        obstacles.is_empty(),
        "after finished reads:\n{}",
        obstacles.join("\n")
    );
}
