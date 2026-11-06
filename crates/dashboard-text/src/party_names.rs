//! The party naming ladder: the localisation key a ruling party's name is read
//! from, shared by party histories and scripted text.

use serde::{Deserialize, Serialize};

/// What names a party: its tag and ideology group, or the keys `set_party_name` stored.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct PartyNaming {
    /// The ideology group token.
    pub token: String,
    /// The party's own tag; None when no recorded country has its slot.
    pub tag: Option<String>,
    /// The country's original tag, empty for none; None when not recorded.
    pub original_tag: Option<String>,
    /// The stored key, when the default flag is clear.
    pub key: Option<String>,
}

/// The party localisation key: the stored key, else
/// `<TAG>_<group>_party`, `<ORIGTAG>_<group>_party`, then the group token.
/// None when the tag or original tag is not recorded.
pub fn party_key(naming: &PartyNaming, defines: impl Fn(&str) -> bool) -> Option<String> {
    party_key_checked(naming, defines).map(|(key, _)| key)
}

/// The same party ladder, retaining its fallback reason for the offline check.
pub fn party_key_checked(
    naming: &PartyNaming,
    defines: impl Fn(&str) -> bool,
) -> Option<(String, Option<String>)> {
    let selected_tag = match naming.original_tag.as_deref() {
        Some("") => naming.tag.as_deref(),
        original => original,
    };
    party_key_for_tag_checked(naming, selected_tag, defines)
}

/// The same ladder with exact selected tag bytes. A positive
/// reference may spell empty text, which still probes `_<group>_party`.
pub(crate) fn party_key_with_selected_tag(
    naming: &PartyNaming,
    selected_tag: &str,
    defines: impl Fn(&str) -> bool,
) -> Option<String> {
    party_key_for_tag_checked(naming, Some(selected_tag), defines).map(|(key, _)| key)
}

pub fn party_key_for_tag_checked(
    naming: &PartyNaming,
    selected_tag: Option<&str>,
    defines: impl Fn(&str) -> bool,
) -> Option<(String, Option<String>)> {
    if let Some(key) = &naming.key {
        return Some((key.clone(), None));
    }
    let own = format!("{}_{}_party", naming.tag.as_deref()?, naming.token);
    let key = if defines(&own) {
        own
    } else {
        format!("{}_{}_party", selected_tag?, naming.token)
    };
    Some(if defines(&key) {
        (key, None)
    } else {
        (
            naming.token.clone(),
            Some(format!(
                "party name keys for {:?} / {} are missing; the ideology group name is shown",
                naming.tag.as_deref()?,
                naming.token
            )),
        )
    })
}

#[cfg(test)]
mod tests;
