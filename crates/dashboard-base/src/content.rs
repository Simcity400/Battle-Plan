//! Refuse unverified recorded content unless the user explicitly allows it.

use crate::{Source, invalid, products::ContentOptions};
use installed_content_core::ResolveOptions;
use installed_content_localisation::localisation::Localisation;
use replay_data::identity::Verification;
use std::{io, path::PathBuf, sync::Arc};

/// What a resolved country name depends on besides the recording.
pub type ContentKey = (PathBuf, PathBuf, bool);

pub fn content_key(content: Option<&ContentOptions>) -> Option<ContentKey> {
    content.map(|content| {
        (
            content.options.game_dir.clone(),
            content.options.user_dir.clone(),
            content.allow_unverified,
        )
    })
}

/// Everything a source's localisation depends on besides the recording: the
/// folders, the cache folder and the user's game language.
pub type LocalisationKey = (ContentKey, PathBuf, String);

/// The key for these folders, reading the game language from the user folder.
pub fn localisation_key(content: &ContentOptions) -> io::Result<LocalisationKey> {
    Ok((
        content_key(Some(content)).expect("content options given"),
        content.options.cache_dir.clone(),
        language(&content.options)?,
    ))
}

/// The recording's localisation in the user's game language, read once per source
/// and key and shared by every product that names anything.
pub fn localisation(source: &Source, content: &ContentOptions) -> io::Result<Arc<Localisation>> {
    localisation_with_key(source, content, localisation_key(content)?)
}

pub(crate) fn localisation_with_key(
    source: &Source,
    content: &ContentOptions,
    key: LocalisationKey,
) -> io::Result<Arc<Localisation>> {
    localisation_in(source, content, key, || source.content(&content.options))
}

/// [`localisation_with_key`] reading, when it is not cached, the content `resolved` gives.
pub(crate) fn localisation_in(
    source: &Source,
    content: &ContentOptions,
    key: LocalisationKey,
    resolved: impl FnOnce() -> io::Result<Arc<installed_content_core::content::ResolvedContent>>,
) -> io::Result<Arc<Localisation>> {
    type Cached = Option<(LocalisationKey, Arc<Localisation>)>;
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, localisation)) = state.as_ref()
        && *cached == key
    {
        return Ok(Arc::clone(localisation));
    }
    let mut warnings = Vec::new();
    check_replay(&source.replay(), content.allow_unverified, &mut warnings)?;
    let layers = resolved()?;
    let localisation = Arc::new(installed_content_localisation::localisation::read(
        &layers.layers,
        &key.2,
    )?);
    *state = Some((key, Arc::clone(&localisation)));
    Ok(localisation)
}

/// [`localisation`] for naming countries, or the warning that says why names show tags.
pub fn name_localisation(
    source: &Source,
    content: Option<&ContentOptions>,
) -> Result<Arc<Localisation>, String> {
    match content {
        None => Err("Country names show tags until the game folders are chosen.".to_owned()),
        Some(content) => localisation(source, content)
            .map_err(|error| format!("Country names show tags: {error}")),
    }
}

/// The user's game language from the configured user folder's `settings.txt`.
pub fn language(options: &ResolveOptions) -> io::Result<String> {
    installed_content_settings::settings::read_keys(&options.user_dir)?.language()
}

pub fn check_verification(
    context: &str,
    state: &Verification,
    allow_unverified: bool,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let reason = match state {
        Verification::Verified { .. } => return Ok(()),
        Verification::Unverified { reason, .. } => reason.as_str(),
        Verification::Unchecked => "the collector did not verify the loaded content roster",
    };
    if !allow_unverified {
        return Err(invalid(format!(
            "{context} has unverified content: {reason}; --allow-unverified-content permits viewing with a persistent warning"
        )));
    }
    warnings.push(format!("UNVERIFIED CONTENT — {context}: {reason}"));
    Ok(())
}

/// Verifies the recording and every session; used by products that read game assets.
/// The strategic regions `layers` define ([`installed_content_regions::regions::read`]), read once for
/// the content stack and shared by every product reading them; each later use credits the
/// read's refusals as its own read would.
pub fn strategic_regions(
    layers: &installed_content_core::ResolvedLayers,
) -> io::Result<Arc<installed_content_regions::regions::StrategicRegions>> {
    layers
        .derived::<crate::catalogue::CachedResult<installed_content_regions::regions::StrategicRegions>>()
        .get_or_init(|| installed_content_regions::regions::read(layers))
}

pub fn check_replay(
    replay: &replay::container::Replay,
    allow_unverified: bool,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    check_verification(
        "recording",
        &replay.identity.verification,
        allow_unverified,
        warnings,
    )?;
    for session in &replay.sessions {
        check_verification(
            "session",
            &session.identity.verification,
            allow_unverified,
            warnings,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    #[test]
    fn a_content_stacks_strategic_regions_are_read_once_for_every_product() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[]);
        let layers = &catalogue.content.layers;
        let first = super::strategic_regions(layers).unwrap();
        let again = super::strategic_regions(layers).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
    }
}
