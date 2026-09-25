//! Refuse unverified recorded content unless the user explicitly allows it.

use crate::{Source, invalid, products::ContentOptions};
use installed_content::{ResolveOptions, localisation::Localisation};
use replay::identity::Verification;
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

/// The recording's localisation in the user's game language, read once per
/// source and folders, or the warning that says why names show tags.
pub fn name_localisation(
    source: &Source,
    content: Option<&ContentOptions>,
) -> Result<Arc<Localisation>, String> {
    type Cached = Option<(Option<ContentKey>, Result<Arc<Localisation>, String>)>;
    let key = content_key(content);
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if state.as_ref().is_none_or(|(cached, _)| *cached != key) {
        let result = match content {
            None => Err("Country names show tags until the game folders are chosen.".to_owned()),
            Some(content) => (|| {
                let mut warnings = Vec::new();
                check_replay(&source.replay(), content.allow_unverified, &mut warnings)?;
                let layers = source.content(&content.options)?;
                installed_content::localisation::read(&layers.layers, &language(&content.options)?)
            })()
            .map(Arc::new)
            .map_err(|error| format!("Country names show tags: {error}")),
        };
        *state = Some((key, result));
    }
    state.as_ref().expect("cached above").1.clone()
}

/// The user's game language from the configured user folder's `settings.txt`.
pub fn language(options: &ResolveOptions) -> io::Result<String> {
    installed_content::settings::language(&installed_content::settings::read(&options.user_dir)?.items)
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
