//! Refuse unverified recorded content unless the user explicitly allows it.

use crate::invalid;
use replay::identity::Verification;
use std::io;

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
