
use crate::{Source, catalogue::kept::Keeping, invalid, products::ContentOptions};
use content_reader_localisation::localisation::Localisation;
use replay_data::identity::Verification;
use std::{io, path::PathBuf, sync::Arc};

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

pub type LocalisationKey = (ContentKey, PathBuf);

pub fn localisation_key(content: &ContentOptions) -> LocalisationKey {
    (
        content_key(Some(content)).expect("content options given"),
        content.options.cache_dir.clone(),
    )
}

pub fn localisation(source: &Source, content: &ContentOptions) -> io::Result<Arc<Localisation>> {
    localisation_with_key(source, content, localisation_key(content))
}

pub fn localisation_for(
    source: &Source,
    options: &content_reader_layers::ResolveOptions,
    allow_unverified: bool,
    resolved: &Arc<content_reader_layers::content::ResolvedContent>,
) -> io::Result<Arc<Localisation>> {
    let key = (
        (
            options.game_dir.clone(),
            options.user_dir.clone(),
            allow_unverified,
        ),
        options.cache_dir.clone(),
    );
    localisation_for_resolved(
        source,
        options,
        allow_unverified,
        key,
        Some(resolved),
        || Ok(Arc::clone(resolved)),
        Keeping::On,
    )
}

pub(crate) fn localisation_with_key(
    source: &Source,
    content: &ContentOptions,
    key: LocalisationKey,
) -> io::Result<Arc<Localisation>> {
    localisation_in(
        source,
        content,
        key,
        || source.content(&content.options),
        Keeping::On,
    )
}

pub(crate) fn localisation_in(
    source: &Source,
    content: &ContentOptions,
    key: LocalisationKey,
    resolved: impl FnOnce() -> io::Result<Arc<content_reader_layers::content::ResolvedContent>>,
    keeping: Keeping,
) -> io::Result<Arc<Localisation>> {
    localisation_for_resolved(
        source,
        &content.options,
        content.allow_unverified,
        key,
        None,
        resolved,
        keeping,
    )
}

fn localisation_for_resolved(
    source: &Source,
    options: &content_reader_layers::ResolveOptions,
    allow_unverified: bool,
    key: LocalisationKey,
    expected_resolved: Option<&Arc<content_reader_layers::content::ResolvedContent>>,
    resolved: impl FnOnce() -> io::Result<Arc<content_reader_layers::content::ResolvedContent>>,
    keeping: Keeping,
) -> io::Result<Arc<Localisation>> {
    type Cached = Option<(
        LocalisationKey,
        Option<std::sync::Weak<content_reader_layers::content::ResolvedContent>>,
        Arc<Localisation>,
        content_reader_layers::content::diagnostics::Diagnostics,
    )>;
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if keeping == Keeping::On
        && let Some((cached, cached_resolved, localisation, diagnostics)) = state.as_ref()
        && *cached == key
        && expected_resolved.is_none_or(|expected| {
            cached_resolved
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                .is_some_and(|old| Arc::ptr_eq(&old, expected))
        })
    {
        diagnostics.credit();
        return Ok(Arc::clone(localisation));
    }
    let mut warnings = Vec::new();
    check_replay(&source.replay(), allow_unverified, &mut warnings)?;
    let mut built_resolved = expected_resolved.map(Arc::downgrade);
    let build = || {
        content_reader_layers::content::diagnostics::capture(|| {
            let layers = resolved()?;
            built_resolved = Some(Arc::downgrade(&layers));
            let language = recorded_language(source.replay_ref());
            content_reader_localisation::localisation::read_cached(
                &layers.layers,
                &language,
                &options.cache_dir,
            )
        })
    };
    let (localisation, diagnostics) = match keeping {
        Keeping::On => crate::catalogue::kept::kept(
            crate::catalogue::kept::place::<Localisation>(source, &options.cache_dir, &[]),
            build,
        ),
        Keeping::Off => build(),
    };
    diagnostics.credit();
    let localisation = Arc::new(localisation?);
    if keeping == Keeping::On {
        if built_resolved.is_none() {
            built_resolved = source
                .bound_content(options)
                .as_ref()
                .map(Arc::downgrade);
        }
        *state = Some((key, built_resolved, Arc::clone(&localisation), diagnostics));
    }
    Ok(localisation)
}

pub fn warm(source: &Source, content: &ContentOptions) {
    let _warm = content_reader_layers::content::Generation::inherit();
    let resolved = source.content(&content.options).ok();
    let _generation = resolved.as_ref().map(|content| content.layers.enter_generation());
    let _ = localisation(source, content);
    let _ = crate::catalogue::catalogue(source, content);
}

pub fn warm_checked(source: &Source, content: &ContentOptions) -> io::Result<()> {
    let _warm = content_reader_layers::content::Generation::inherit();
    let resolved = source.content(&content.options)?;
    let _generation = resolved.layers.enter_generation();
    localisation(source, content)?;
    crate::catalogue::catalogue(source, content)?;
    Ok(())
}

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

pub fn recorded_language(replay: &replay::campaign::Replay) -> String {
    let language = &replay.identity.language;
    language.strip_prefix("l_").unwrap_or(language).to_owned()
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

pub fn strategic_regions(
    layers: &content_reader_layers::ResolvedLayers,
) -> io::Result<Arc<content_reader_regions::regions::StrategicRegions>> {
    layers
        .derived::<crate::catalogue::CachedResult<content_reader_regions::regions::StrategicRegions>>()
        .get_or_init(|| content_reader_regions::regions::read(layers))
}

pub fn check_replay(
    replay: &replay::campaign::Replay,
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

    #[test]
    fn warm_checked_surfaces_content_load_failures() {
        let root = std::env::temp_dir().join(format!("warm-checked-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (source, context) = crate::test_fixtures::source_with_content(
            vec![crate::test_fixtures::source(1, 0, Some(1))],
            &root,
        );
        let mut bad = context.content.unwrap();
        bad.allow_unverified = false;
        let err = source
            .in_scope(|| super::warm_checked(&source, &bad))
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("unverified content"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn localisation_for_shares_cached_arc_with_localisation() {
        let root = std::env::temp_dir().join(format!("shared-loc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (source, context) = crate::test_fixtures::source_with_content(
            vec![crate::test_fixtures::source(1, 0, Some(1))],
            &root,
        );
        let content = context.content.unwrap();
        let resolved = source.content(&content.options).unwrap();
        let first = super::localisation(&source, &content).unwrap();
        let second = super::localisation_for(
            &source,
            &content.options,
            content.allow_unverified,
            &resolved,
        )
        .unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        let _ = std::fs::remove_dir_all(&root);
    }
}
