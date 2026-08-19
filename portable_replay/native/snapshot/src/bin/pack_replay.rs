use portable_replay_format::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rc = match args.get(1).map(String::as_str) {
        Some("pack") => cmd_pack(&args),
        Some("verify") => cmd_verify(&args),
        _ => {
            eprintln!(
                "usage:\n  pack_replay pack   <store_root> <game_id> <out.battleplan>\n  \
                 pack_replay verify <in.battleplan> [<store_root> <game_id>]"
            );
            2
        }
    };
    exit(rc);
}

fn cmd_pack(args: &[String]) -> i32 {
    let (root, game, out) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(a), Some(b), Some(c)) => (PathBuf::from(a), b.clone(), PathBuf::from(c)),
        _ => {
            eprintln!("pack: need <store_root> <game_id> <out.battleplan>");
            return 2;
        }
    };
    let gdir = root.join(&game);
    let index_path = gdir.join("index.json");
    let index_bytes = match fs::read(&index_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("pack: cannot read {}: {e}", index_path.display());
            return 1;
        }
    };
    let index: Value = serde_json::from_slice(&index_bytes).expect("index json");
    let snaps = index
        .get("snapshots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut entries: Vec<PackEntry> = Vec::new();
    entries.push(PackEntry {
        path: format!("payload/{game}/index.json"),
        bytes: index_bytes,
        role: "index",
        hour_index: None,
    });

    let mut identity = PackIdentity {
        created_utc: Some(now_utc()),
        ..Default::default()
    };
    let mut took_identity = false;

    for s in &snaps {
        let file = match s.get("file").and_then(Value::as_str) {
            Some(f) => f,
            None => continue,
        };
        let hour = s.get("hour_index").and_then(Value::as_i64);
        let p = gdir.join(file);
        let bytes = match fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("pack: missing snapshot {}: {e}", p.display());
                return 1;
            }
        };
        if !took_identity {
            if let Ok(snap) = serde_json::from_slice::<Value>(&bytes) {
                identity.schema_version = snap
                    .get("schema_version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                identity.mod_identity = snap
                    .get("provenance")
                    .and_then(|pr| pr.get("mod"))
                    .cloned()
                    .filter(|v| !v.is_null());
                identity.build_exe_sha256 = snap
                    .get("provenance")
                    .and_then(|pr| pr.get("memory"))
                    .and_then(|m| m.get("exe_sha256"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            took_identity = true;
        }
        entries.push(PackEntry {
            path: format!("payload/{game}/{file}"),
            bytes,
            role: "snapshot",
            hour_index: hour,
        });
    }

    let tpath = gdir.join("timeline.json");
    if let Ok(tb) = fs::read(&tpath) {
        entries.push(PackEntry {
            path: format!("payload/{game}/timeline.json"),
            bytes: tb,
            role: "timeline",
            hour_index: None,
        });
    }

    let container = match pack(&game, &entries, &identity, true) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("pack: {e}");
            return 1;
        }
    };
    if let Err(e) = publish_verified_container(&out, &container) {
        eprintln!("pack: durable publication failed: {e}");
        return 1;
    }
    eprintln!(
        "packed {} snapshots -> {} ({} bytes)",
        entries.iter().filter(|e| e.role == "snapshot").count(),
        out.display(),
        container.len()
    );
    0
}

fn publish_verified_container(out: &Path, bytes: &[u8]) -> Result<(), String> {
    let in_memory = Container::from_bytes(bytes.to_vec(), &ExpectedIdentity::default())
        .map_err(|error| format!("packed bytes failed pre-publication open: {error}"))?;
    in_memory
        .verify_all()
        .map_err(|error| format!("packed bytes failed pre-publication verification: {error}"))?;

    let parent = out.parent().filter(|path| !path.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|error| format!("create output directory: {error}"))?;
    }
    let file_name = out
        .file_name()
        .ok_or_else(|| "output path has no file name".to_string())?;
    let canonical_parent = fs::canonicalize(parent.unwrap_or_else(|| Path::new(".")))
        .map_err(|error| format!("anchor output directory: {error}"))?;
    let anchored_out = canonical_parent.join(file_name);
    reject_reparse_destination(&anchored_out)?;
    let (temp, mut file) = create_unique_temp(&anchored_out)?;

    let write_result = (|| -> Result<(), String> {
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        let mut persisted = Vec::with_capacity(bytes.len());
        file.read_to_end(&mut persisted)
            .map_err(|error| error.to_string())?;
        if persisted != bytes {
            return Err("temporary container readback differs from packed bytes".into());
        }
        let reopened = Container::from_bytes(persisted, &ExpectedIdentity::default())
            .map_err(|error| format!("temporary container failed to re-open: {error}"))?;
        reopened
            .verify_all()
            .map_err(|error| format!("temporary container failed verification: {error}"))
    })();
    if let Err(error) = write_result {
        drop(file);
        let cleanup = fs::remove_file(&temp);
        return Err(cleanup_note(error, &temp, cleanup));
    }
    drop(file);

    if let Err(error) = atomic_replace(&temp, &anchored_out) {
        let cleanup = fs::remove_file(&temp);
        return Err(cleanup_note(
            format!("atomic replace failed: {error}"),
            &temp,
            cleanup,
        ));
    }
    sync_published_parent(&anchored_out)
        .map_err(|error| format!("sync output directory: {error}"))?;

    let published =
        fs::read(&anchored_out).map_err(|error| format!("read published container: {error}"))?;
    if published != bytes {
        return Err("published container differs from verified temporary bytes".into());
    }
    let reopened = Container::from_bytes(published, &ExpectedIdentity::default())
        .map_err(|error| format!("published container failed to re-open: {error}"))?;
    reopened
        .verify_all()
        .map_err(|error| format!("published container failed verification: {error}"))
}

fn create_unique_temp(out: &Path) -> Result<(PathBuf, fs::File), String> {
    let name = out
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "output file name is not portable UTF-8".to_string())?;
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    let nonce = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    );
    for attempt in 0..128u32 {
        let temp = parent.join(format!(".{name}.pack-tmp-{nonce}.{attempt}"));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create unique output temp: {error}")),
        }
    }
    Err("could not allocate a unique output temp after 128 attempts".into())
}

fn reject_reparse_destination(out: &Path) -> Result<(), String> {
    match fs::symlink_metadata(out) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err("refusing a symbolic-link output destination".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x0000_0400 != 0 {
                    return Err("refusing a reparse-point output destination".into());
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect output destination: {error}")),
    }
}

fn cleanup_note(primary: String, temp: &Path, cleanup: std::io::Result<()>) -> String {
    match cleanup {
        Ok(()) => primary,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => primary,
        Err(error) => format!(
            "{primary}; failed to remove temporary artifact {}: {error}",
            temp.display()
        ),
    }
}

#[cfg(not(windows))]
fn sync_published_parent(out: &Path) -> std::io::Result<()> {
    fs::File::open(out.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(windows)]
fn sync_published_parent(_out: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(temp: &Path, out: &Path) -> std::io::Result<()> {
    fs::rename(temp, out)
}

#[cfg(windows)]
fn atomic_replace(temp: &Path, out: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let existing = temp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = out
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let ok = unsafe { MoveFileExW(existing.as_ptr(), replacement.as_ptr(), 0x1 | 0x8) };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn cmd_verify(args: &[String]) -> i32 {
    let infile = match args.get(2) {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("verify: need <in.battleplan>");
            return 2;
        }
    };
    let c = match Container::open(&infile, &ExpectedIdentity::default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("verify: open failed: {e}");
            return 1;
        }
    };
    if let Err(e) = c.verify_all() {
        eprintln!("verify: integrity FAILED: {e}");
        return 1;
    }
    eprintln!(
        "verify: OK — game={} snapshots={} signed={} unsigned={}",
        c.game_id(),
        c.index().snapshots.len(),
        c.signed,
        c.unsigned
    );

    if let (Some(root), Some(game)) = (args.get(3), args.get(4)) {
        let gdir = Path::new(root).join(game);
        for s in &c.index().snapshots {
            let orig = gdir.join(&s.file);
            let orig_bytes = match fs::read(&orig) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("verify: cannot read source {}: {e}", orig.display());
                    return 1;
                }
            };
            let want = portable_replay_format::sha256::Sha256::hex(&orig_bytes);
            let path = format!("payload/{game}/{}", s.file);
            let got = c
                .manifest()
                .get("entries")
                .and_then(Value::as_array)
                .and_then(|es| {
                    es.iter()
                        .find(|e| e.get("path").and_then(Value::as_str) == Some(&path))
                })
                .and_then(|e| e.get("sha256").and_then(Value::as_str))
                .unwrap_or("<none>");
            if got != want {
                eprintln!(
                    "verify: BYTE-IDENTITY MISMATCH for {}: source {} != container {}",
                    s.file, want, got
                );
                return 1;
            }
        }
        eprintln!(
            "verify: byte-identity OK for all {} snapshots",
            c.index().snapshots.len()
        );
    }
    0
}

fn now_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch:{secs}")
}
