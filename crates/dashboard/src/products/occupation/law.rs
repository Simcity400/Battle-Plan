//! A state's effective occupation law: its own override, else the law its occupier set for the
//! occupied country, else the occupier's default when the law's triggers admit it, else the
//! database fallback. Eligibility is evaluated over the recorded trigger world at the reply's hour.

use crate::catalogue::{self, Catalogue, View};
use installed_content_core::occupation_laws::{Law, Laws};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

/// A law the reply's states hold: its name and icon from the recorded content.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupationLawInfo {
    pub token: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub missing: Option<String>,
}

/// The law the occupier set for the occupied country.
pub(super) enum GroupLaw<'a> {
    /// The occupier keeps no group inventory: the database fallback, unvalidated.
    Missing,
    /// Its law, none for the occupier's default.
    Law(Option<&'a [u8]>),
    Unknown(String),
}

pub(super) struct LawInputs<'a> {
    pub state: Option<&'a [u8]>,
    pub key_resolved: bool,
    pub overrides_complete: bool,
    pub group: GroupLaw<'a>,
    /// The occupier's default: whether it is set, and its token.
    pub default: Option<(bool, &'a [u8])>,
    pub fallback: Result<&'a [u8], String>,
}

impl LawInputs<'_> {
    /// Whether the law falls to the occupier's default, which its triggers must admit.
    pub fn reaches_default(&self) -> bool {
        self.key_resolved && self.state.is_none() && self.overrides_complete && matches!(self.group, GroupLaw::Law(None)) && self.default.is_some()
    }
}

/// The effective law token, or why it cannot be established. `valid` decides whether the
/// occupier's default is admitted.
pub(super) fn law_for(input: LawInputs<'_>, valid: impl FnOnce(&str) -> Result<bool, String>) -> Result<String, String> {
    let token = |bytes: &[u8]| std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| "The recorded occupation-law key is not UTF-8".to_owned());
    if !input.key_resolved {
        return Err("The controller or occupied-country link is unresolved; the state's occupation-law key cannot be established".into());
    }
    if matches!(input.group, GroupLaw::Missing) {
        return token(input.fallback?);
    }
    if let Some(bytes) = input.state {
        return token(bytes);
    }
    if !input.overrides_complete {
        return Err("The occupation-law overrides are incomplete; this state's override may not have been recorded".into());
    }
    match input.group {
        GroupLaw::Law(Some(bytes)) => return token(bytes),
        GroupLaw::Unknown(reason) => return Err(reason),
        GroupLaw::Law(None) | GroupLaw::Missing => {}
    }
    let candidate = match input.default {
        Some((true, bytes)) => token(bytes)?,
        Some((false, _)) => token(input.fallback.clone()?)?,
        None => return Err("The occupier's default law is missing".into()),
    };
    if valid(&candidate)? { Ok(candidate) } else { token(input.fallback?) }
}

/// Whether `law`'s visible and available triggers admit it for occupier `controller` over
/// `occupied` (canonical slots): the occupier's FROM is the occupied country, the occupied's ROOT
/// the occupier; availability is skipped where both are one country.
pub(super) fn admitted(world: &dashboard_text::triggers::World<'_>, law: &Law, controller: u32, occupied: u32) -> Result<bool, String> {
    use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
    let mut occupier_scope = EventScope::of(EventObject::Country(controller));
    occupier_scope.from = Link::Scope(1);
    let mut occupied_scope = EventScope::of(EventObject::Country(occupied));
    occupied_scope.root = Link::Scope(0);
    let graph = std::rc::Rc::new(std::cell::RefCell::new(EventScopes::new(vec![occupier_scope, occupied_scope])));
    let evaluator = dashboard_text::triggers::Evaluator::new(world, None).with_event_scopes(graph);
    let scope = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
    evaluator
        .evaluate(&law.visible, scope)
        .and(|| if controller == occupied { dashboard_text::triggers::Eval::True } else { evaluator.evaluate(&law.available, scope) })
        .decided()
}

/// A law's icon drawn through `cat`: its declaration's sprite at its authored frame.
fn icon(cat: &Catalogue, laws: &Laws, token: &str) -> Result<String, String> {
    let law = laws.find(token)?.ok_or_else(|| "Occupation law has no content declaration".to_owned())?;
    let sprite = cat.sprites.find(&law.sprite)?.ok_or_else(|| "Occupation-law sprite not in the recorded content stack".to_owned())?;
    let frame = u32::try_from(law.frame).ok().and_then(|frame| frame.checked_sub(1)).ok_or_else(|| "Occupation-law icon frame is not positive".to_owned())?;
    let decode = cat.gui_decode.clone()?;
    catalogue::sprite_frame_png(&cat.content.layers, sprite, decode, frame, 32).map_err(|error| error.to_string())
}

/// A law's name (the token localised) and icon; an icon not drawn says why.
pub(super) fn info(cat: Result<&Catalogue, &str>, laws: Result<&Laws, &str>, token: &str) -> OccupationLawInfo {
    let name = cat.ok().and_then(|cat| cat.localised(token, View::Plain));
    let icon = match (cat, laws) {
        (Ok(cat), Ok(laws)) => icon(cat, laws, token),
        (Err(why), _) | (_, Err(why)) => Err(why.to_owned()),
    };
    OccupationLawInfo { token: token.to_owned(), name, missing: icon.as_ref().err().cloned(), icon: icon.ok() }
}
