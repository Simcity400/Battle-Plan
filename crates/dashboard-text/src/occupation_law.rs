//! A state's effective occupation law: its own override, else the law its occupier set for the
//! occupied country, else the occupier's default when the law's triggers admit it, else the
//! database fallback. Eligibility is evaluated over the recorded trigger world at the hour.

use installed_content_core::occupation_laws::Law;

/// The law the occupier set for the occupied country.
pub enum GroupLaw<'a> {
    /// The occupier keeps no group inventory: the database fallback, unvalidated.
    Missing,
    /// Its law, none for the occupier's default.
    Law(Option<&'a [u8]>),
    Unknown(String),
}

pub struct LawInputs<'a> {
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
pub fn law_for(input: LawInputs<'_>, valid: impl FnOnce(&str) -> Result<bool, String>) -> Result<String, String> {
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
pub fn admitted(world: &crate::triggers::World<'_>, law: &Law, controller: u32, occupied: u32) -> Result<bool, String> {
    use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
    let mut occupier_scope = EventScope::of(EventObject::Country(controller));
    occupier_scope.from = Link::Scope(1);
    let mut occupied_scope = EventScope::of(EventObject::Country(occupied));
    occupied_scope.root = Link::Scope(0);
    let graph = std::rc::Rc::new(std::cell::RefCell::new(EventScopes::new(vec![occupier_scope, occupied_scope])));
    let evaluator = crate::triggers::Evaluator::new(world, None).with_event_scopes(graph);
    let scope = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
    evaluator
        .evaluate(&law.visible, scope)
        .and(|| if controller == occupied { crate::triggers::Eval::True } else { evaluator.evaluate(&law.available, scope) })
        .decided()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(state: Option<&'a [u8]>, group: GroupLaw<'a>, default: Option<(bool, &'a [u8])>) -> LawInputs<'a> {
        LawInputs { state, key_resolved: true, overrides_complete: true, group, default, fallback: Ok("fallback".as_bytes()) }
    }

    fn never(_: &str) -> Result<bool, String> {
        panic!("only the occupier's default is validated")
    }

    #[test]
    fn a_state_override_wins_then_the_group_law_then_an_admitted_default_else_the_fallback() {
        assert_eq!(law_for(inputs(Some("state".as_bytes()), GroupLaw::Law(Some("group".as_bytes())), Some((true, "default".as_bytes()))), never), Ok("state".into()));
        assert_eq!(law_for(inputs(None, GroupLaw::Law(Some("group".as_bytes())), Some((true, "default".as_bytes()))), never), Ok("group".into()));
        assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))), |law| Ok(law == "default")), Ok("default".into()));
        assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))), |_| Ok(false)), Ok("fallback".into()));
        // An unset default validates the fallback itself.
        assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((false, "default".as_bytes()))), |law| Ok(law == "fallback")), Ok("fallback".into()));
        // No group inventory: the database fallback, before any override and unvalidated.
        assert_eq!(law_for(inputs(Some("state".as_bytes()), GroupLaw::Missing, Some((true, "default".as_bytes()))), never), Ok("fallback".into()));
        assert!(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))).reaches_default());
        assert!(!inputs(Some("state".as_bytes()), GroupLaw::Law(None), Some((true, "default".as_bytes()))).reaches_default());
    }

    #[test]
    fn a_law_not_established_says_why_and_is_never_a_guess() {
        let unresolved = LawInputs { key_resolved: false, ..inputs(Some("state".as_bytes()), GroupLaw::Law(None), None) };
        assert!(law_for(unresolved, never).unwrap_err().contains("unresolved"));
        let incomplete = LawInputs { overrides_complete: false, ..inputs(None, GroupLaw::Law(Some("group".as_bytes())), None) };
        assert!(law_for(incomplete, never).unwrap_err().contains("overrides are incomplete"));
        assert_eq!(law_for(inputs(None, GroupLaw::Unknown("not recorded".into()), None), never), Err("not recorded".into()));
        assert!(law_for(inputs(None, GroupLaw::Law(None), None), never).unwrap_err().contains("default law is missing"));
        let no_fallback = LawInputs { fallback: Err("no fallback".into()), ..inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))) };
        assert_eq!(law_for(no_fallback, |_| Ok(false)), Err("no fallback".into()));
        assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))), |_| Err("trigger undecided".into())), Err("trigger undecided".into()));
    }
}
