//! Read-only access to the configured game's current settings.txt.
use crate::parser::{self, Document, Item};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub fn read(user: &Path) -> io::Result<Document> {
    parser::parse(&bounded_read(&user.join("settings.txt"))?)
}

fn bounded_read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 {
        return Err(io::Error::other("Settings file exceeds its size limit"));
    }
    Ok(bytes)
}

/// The viewer's saved folder selection, with the same explicit CLI override.
pub fn configured_user_folder() -> io::Result<PathBuf> {
    let mut arguments = std::env::args_os().skip(1);
    let mut selected = None;
    while let Some(argument) = arguments.next() {
        if argument == "--user-dir" {
            selected =
                Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    io::Error::other("Missing --user-dir folder")
                })?));
        }
    }
    if let Some(path) = selected {
        return Ok(path);
    }
    #[derive(serde::Deserialize)]
    struct Folders {
        user_dir: PathBuf,
    }
    let config = std::env::current_exe()?.with_file_name("viewer.json");
    let folders: Folders = serde_json::from_slice(&bounded_read(&config)?)?;
    if folders.user_dir.as_os_str().is_empty() {
        return Err(io::Error::other(
            "Choose the Paradox user folder in Settings",
        ));
    }
    Ok(folders.user_dir)
}

pub fn block<'a>(items: &'a [Item], key: &str) -> io::Result<&'a [Item]> {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key.as_bytes()))
        .and_then(|item| item.value.items())
        .ok_or_else(|| io::Error::other(format!("Missing {key} block in game settings")))
}
pub fn scalar<'a>(items: &'a [Item], key: &str) -> io::Result<&'a str> {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key.as_bytes()))
        .and_then(|item| item.value.scalar_str())
        .ok_or_else(|| io::Error::other(format!("Missing {key} in game settings")))
}
/// The game's interface language from `settings.txt` (`language="l_english"`),
/// as the bare name localisation files use after `l_`.
pub fn language(items: &[Item]) -> io::Result<String> {
    scalar(items, "language")?
        .strip_prefix("l_")
        .filter(|name| {
            !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("Unsupported language in game settings"))
}
pub fn toggle(items: &[Item], key: &str) -> io::Result<bool> {
    match scalar(items, key)? {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(io::Error::other(format!("Invalid {key} in game settings"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_is_the_bare_name_and_refuses_other_forms() {
        let language = |text: &str| language(&parser::parse(text.as_bytes()).unwrap().items);
        assert_eq!(language(r#"language="l_english""#).unwrap(), "english");
        assert_eq!(language("language=l_braz_por").unwrap(), "braz_por");
        for bad in [
            r#"language="english""#,
            r#"language="l_""#,
            r#"language="l_../x""#,
            "",
        ] {
            assert!(language(bad).is_err(), "{bad}");
        }
    }
}
