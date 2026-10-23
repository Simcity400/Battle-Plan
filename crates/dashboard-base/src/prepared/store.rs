//! The prepared store of one recording path and the source key that binds
//! its files to the recorded content.

use crate::Source;
use std::{path::Path, sync::Arc};

/// The cache belongs to this recording path, even when another recording shares
/// its campaign identity. Every content input comes from the recorded stack.
pub fn files(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
) -> std::io::Result<(Arc<store::Store>, store::SourceKey)> {
    type Cached = Option<(
        Option<(crate::content::ContentKey, std::path::PathBuf)>,
        Arc<store::Store>,
        store::SourceKey,
    )>;
    let cache = source.state::<Cached>();
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    let options = content.map(|value| {
        (
            crate::content::content_key(Some(value)).unwrap(),
            value.options.cache_dir.clone(),
        )
    });
    if let Some((saved, store, key)) = cache.as_ref()
        && saved == &options
    {
        return Ok((Arc::clone(store), key.clone()));
    }
    if crate::protocol::replay_revision(source.path())? != source.key().revision {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Recording changed; reopen it before preparing the current content",
        ));
    }
    let path = std::fs::canonicalize(source.path())?;
    let store = recording_store(&path)?;
    let mut inputs = match content {
        Some(content) => source
            .content(&content.options)?
            .layers
            .recorded_source_files()?,
        None => Vec::new(),
    };
    if let Some(content) = content {
        let settings = content.options.user_dir.join("settings.txt");
        if settings.is_file() {
            inputs.push(settings);
        }
    }
    let key = match &source.cached_inputs {
        Some(saved) => saved.recapture_for(&path, &inputs)?,
        None => store::SourceKey::capture(&path, &inputs)?,
    };
    if crate::protocol::replay_revision(&path)? != source.key().revision {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Recording changed; reopen it before preparing the current content",
        ));
    }
    // Establish the manifest binding once; query reads retain it through this
    // source-local key rather than revisiting the resolved content inputs.
    store.is_reusable(&key)?;
    *cache = Some((options, Arc::clone(&store), key.clone()));
    Ok((store, key))
}

/// Shared host/native directory identity for every revision of one recording.
pub fn recording_store(path: &Path) -> std::io::Result<Arc<store::Store>> {
    Ok(Arc::new(store::Store::local(recording_identity(path)?)?))
}

/// Its folder and the campaign its header names, which a rename keeps; else its path.
pub fn recording_identity(path: &Path) -> std::io::Result<uuid::Uuid> {
    let path = std::fs::canonicalize(path)?;
    let mut hash = blake3::Hasher::new();
    match (
        path.parent(),
        replay::recording_files::campaign_of(&path).ok().flatten(),
    ) {
        (Some(folder), Some(campaign)) => {
            hash.update(folder.as_os_str().as_encoded_bytes());
            hash.update(&[0]);
            hash.update(&campaign.to_ascii_lowercase());
        }
        _ => {
            hash.update(path.as_os_str().as_encoded_bytes());
        }
    }
    let mut id = [0; 16];
    id.copy_from_slice(&hash.finalize().as_bytes()[..16]);
    Ok(uuid::Uuid::from_bytes(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with a recording's header naming `campaign`.
    fn header(path: &Path, campaign: &str) {
        let mut bytes = b"H4RP".to_vec();
        bytes.extend_from_slice(
            &replay_data::format::FormatVersion::CURRENT
                .version()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&720_u32.to_le_bytes());
        bytes.extend_from_slice(&(campaign.len() as u32).to_le_bytes());
        bytes.extend_from_slice(campaign.as_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn what_is_kept_about_a_recording_follows_it_through_renames() {
        let root =
            std::env::temp_dir().join(format!("hoi4-recording-identity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("elsewhere")).unwrap();
        let campaign = "00000000-0000-4000-8000-000000000000";
        let path = root.join("Germany 1936.battleplan");
        header(&path, campaign);
        let identity = recording_identity(&path).unwrap();
        let renamed = root.join("Germany 1936–1937.battleplan");
        std::fs::rename(&path, &renamed).unwrap();
        assert_eq!(recording_identity(&renamed).unwrap(), identity);
        // Another folder or campaign is another recording.
        let moved = root.join("elsewhere").join("Germany 1936.battleplan");
        header(&moved, campaign);
        assert_ne!(recording_identity(&moved).unwrap(), identity);
        let other = root.join("Italy 1936.battleplan");
        header(&other, "00000000-0000-4000-8000-000000000001");
        assert_ne!(recording_identity(&other).unwrap(), identity);
        // One naming no campaign is known by its path.
        let unnamed = root.join("synthetic.battleplan");
        header(&unnamed, "synthetic");
        let by_path = recording_identity(&unnamed).unwrap();
        std::fs::rename(&unnamed, root.join("renamed.battleplan")).unwrap();
        assert_ne!(
            recording_identity(&root.join("renamed.battleplan")).unwrap(),
            by_path
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
