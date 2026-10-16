//! The prepared store of one recording path and the source key that binds
//! its files to the recorded content.

use crate::Source;
use std::sync::Arc;

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

/// Shared host/native directory identity for every revision at one canonical path.
pub fn recording_store(path: &std::path::Path) -> std::io::Result<Arc<store::Store>> {
    let path = std::fs::canonicalize(path)?;
    let hash = blake3::hash(path.as_os_str().as_encoded_bytes());
    let mut id = [0; 16];
    id.copy_from_slice(&hash.as_bytes()[..16]);
    Ok(Arc::new(store::Store::local(uuid::Uuid::from_bytes(id))?))
}
