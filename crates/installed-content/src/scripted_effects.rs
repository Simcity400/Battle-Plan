//! Scripted-effect names at their own load point in CBuildingDatabase::load
//! (14018B610, call 14018CF67 -> 140193190), before any effect body.

use crate::{
    ResolvedLayers,
    parser::{
        c_string,
        native::{self, Shape},
    },
};
use std::{collections::BTreeSet, io};

/// Name pass (140193190 -> 14013ECE0 -> 14012B430): directory's own
/// `.txt` files, values skipped without
/// defining variables. ReadEffect (1405345D0) finds the name by token hash
/// (FindTokenFromSsoString 1424A7060) and requires the template's +0x58 flag.
pub fn names(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut names = BTreeSet::new();
    for path in crate::script_files::paths(layers, "common/scripted_effects/", Some(".txt")) {
        names.extend(file_names(&layers.read(&path)?));
    }
    Ok(names)
}

/// The name pass over one file, using the same CReader as the body readers.
pub fn file_names(bytes: &[u8]) -> BTreeSet<Vec<u8>> {
    native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(()),
        None,
        &mut |(), _| Shape::SkipIfOpen,
    )
    .items
    .iter()
    .map(|item| c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_pass_skips_bodies_and_folds_tokens() {
        let names = file_names(
            b"@name = One One = { @hidden = Lost } Two = @hidden \"Three\" = {} One = {}",
        );
        assert_eq!(
            names,
            [b"one".to_vec(), b"two".to_vec(), b"three".to_vec()].into()
        );
    }

    #[test]
    fn names_resolve_recorded_stack_and_only_directory_files() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!("effect-names-{}", std::process::id()));
        let vanilla = root.join("base");
        let modded = root.join("mod");
        for folder in [&vanilla, &modded] {
            std::fs::create_dir_all(folder.join("common/scripted_effects/sub"))?;
        }
        std::fs::write(
            vanilla.join("common/scripted_effects/base.txt"),
            b"old = {}",
        )?;
        std::fs::write(modded.join("common/scripted_effects/mod.txt"), b"New = {}")?;
        std::fs::write(
            modded.join("common/scripted_effects/sub/nested.txt"),
            b"nested = {}",
        )?;
        let layers = ResolvedLayers::from_folders(&[
            (&vanilla, &[]),
            (&modded, &["common/scripted_effects"]),
        ])?;
        let loaded = names(&layers)?;
        assert_eq!(loaded, [b"new".to_vec()].into());
        // This test owns the process-specific scratch directory.
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
