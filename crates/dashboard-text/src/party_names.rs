
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PartyNaming {
    pub token: String,
    pub tag: Option<String>,
    pub original_tag: Option<String>,
    pub key: Option<String>,
}

pub fn party_key(naming: &PartyNaming, defines: impl Fn(&str) -> bool) -> Option<String> {
    party_key_checked(naming, defines).map(|(key, _)| key)
}

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
