//! Organisation modifier sums and the policy attach quote. Organisation state,
//! modifier metadata and country values are recorded; trait, policy and
//! modifier definitions resolve through the recorded content stack.
use crate::products::equipment::designs::queries::spelling;
use replay_data::rows::{self, tables};
use replay_frame::frame::DecodedFrame;

/// Fixed5 one.
pub const ONE: i64 = 100_000;

/// Whether the modifier named `name` takes effect under the recording's features.
pub(crate) fn modifier_eligible(frame: &DecodedFrame, name: &str) -> Result<bool, String> {
    let definition = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| {
            spelling(frame, row.token)
                .is_ok_and(|token| token.eq_ignore_ascii_case(name.as_bytes()))
        })
        .ok_or_else(|| format!("organisation modifier {name} definition unavailable (count: 1)"))?;
    if definition.feature_selector == 0 {
        return Ok(true);
    }
    if definition.eligibility == 0 {
        return Ok(false);
    }
    let [features] = frame
        .complete::<tables::SimulationFeatures>()
        .map_err(|e| e.to_string())?
    else {
        return Err("organisation modifier feature mask is not unique (count: 1)".into());
    };
    Ok(features.dlc_mask & definition.feature_selector != 0)
}

/// One source's effective `key` entry: zero when absent or ineligible, and
/// clamped to [0, ONE] when its definition says so.
pub(crate) fn organisation_modifier_value(
    frame: &DecodedFrame,
    modifiers: &installed_content_core::modifiers::Modifiers,
    modifier_name: &str,
    key: u32,
) -> Result<i64, String> {
    if let Some(reason) = modifiers.unheld.cause() {
        return Err(format!("organisation modifier: {reason} (count: 1)"));
    }
    let Some(entry) = modifiers
        .values
        .iter()
        .find(|row| row.name.eq_ignore_ascii_case(modifier_name))
    else {
        return Ok(0);
    };
    if !modifier_eligible(frame, modifier_name)? {
        return Ok(0);
    }
    let definition = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.index == key)
        .ok_or_else(|| {
            format!("organisation modifier {modifier_name} definition unavailable (count: 1)")
        })?;
    // The native getter selects the object's own numeric entry. Added
    // modifier objects affect effective emptiness, not this value lookup.
    Ok(
        if definition.flags & engine_native::modifier_definition::CLAMP_MAXIMUM != 0 {
            entry.value.clamp(0, ONE)
        } else {
            entry.value
        },
    )
}

/// The organisation's own sum of the modifier `key`, in signed fixed5: its
/// completed traits while the organisation is visible and available, its
/// initial trait, and its currently attached policy's organisation modifier.
/// Each source's entry is taken effectively before the sum.
pub fn local_modifier(
    frame: &DecodedFrame,
    organisation: &rows::IndustrialOrganisationRow,
    details: &rows::IndustrialOrganisationDetailsRow,
    key: u32,
    traits: &installed_content_core::mio_traits::Catalog,
    policies: &installed_content_core::industrial_organisations::policies::Catalog,
    world: &dashboard_text::triggers::World<'_>,
) -> Result<i64, String> {
    let reference = organisation.organisation;
    let name =
        std::str::from_utf8(spelling(frame, organisation.token)?).map_err(|e| e.to_string())?;
    let tree = traits
        .tree(name)?
        .ok_or_else(|| format!("organisation {name} traits are absent"))?;
    let modifier_name = engine_executable::modifier_keys::name(key)
        .ok_or("organisation modifier metadata unavailable")?;
    let value = |modifiers: &installed_content_core::modifiers::Modifiers| {
        organisation_modifier_value(frame, modifiers, modifier_name, key)
    };
    let mut local = 0_i64;
    // The organisation's conditions read the game's two-scope context: an
    // owner the recording cannot resolve leaves FROM unknown, so only a
    // condition that reads it is undecided.
    let owner = frame
        .complete::<tables::CountryLinks>()
        .ok()
        .and_then(|links| dashboard_text::metadata::link_slot(&links, organisation.owner));
    let evaluator = dashboard_text::triggers::Evaluator::new(world, None).with_event_scopes(
        std::rc::Rc::new(std::cell::RefCell::new(
            dashboard_text::scripted_text::EventScopes::organisation(reference, owner),
        )),
    );
    let scope = evaluator.event_scopes(0).map_err(|e| e.to_string())?;
    let ordinary = both(evaluator.evaluate(tree.visible, scope).decided(), || {
        evaluator.evaluate(tree.available, scope).decided()
    })?;
    if ordinary {
        for token in &details.unlocked {
            let token = std::str::from_utf8(spelling(frame, *token)?).map_err(|e| e.to_string())?;
            // An unresolved completed-trait token contributes nothing in the
            // native first-match template lookup.
            if let Some(definition) = traits.trait_definition(name, token)? {
                let modifiers = definition
                    .resolved_organisation_modifiers(|name| modifier_eligible(frame, name))?;
                local = local.wrapping_add(value(&modifiers)?);
            }
        }
    }
    if let Some(initial) = tree.initial {
        let modifiers =
            initial.resolved_organisation_modifiers(|name| modifier_eligible(frame, name))?;
        local = local.wrapping_add(value(&modifiers)?);
    }
    if let Some(selected) = owner_roster(frame, organisation)?
        .policies
        .iter()
        .find(|policy| policy.token == details.state.policy)
    {
        local = local.wrapping_add(value(
            &policies
                .policy(spelling(frame, selected.token)?)?
                .organisation_modifiers,
        )?);
    }
    Ok(local)
}

/// The conjunction of `first` and `second`, three-valued: a false part decides
/// it whatever the other, so an unknown first part still lets a false second
/// one decide. `second` is evaluated only when `first` does not already fail.
pub fn both(
    first: Result<bool, String>,
    second: impl FnOnce() -> Result<bool, String>,
) -> Result<bool, String> {
    match first {
        Ok(false) => Ok(false),
        Ok(true) => second(),
        Err(why) => match second() {
            Ok(false) => Ok(false),
            _ => Err(why),
        },
    }
}

/// The organisation's owner. The owner is a raw country reference, which can
/// be an alias: it resolves through CountryLinks to its country.
pub fn owner_country<'a>(
    frame: &'a DecodedFrame,
    organisation: &rows::IndustrialOrganisationRow,
) -> Result<&'a rows::CountryRow, String> {
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?;
    let slot = dashboard_text::metadata::link_slot(links, organisation.owner);
    slot.and_then(|slot| {
        frame
            .complete::<tables::Countries>()
            .ok()?
            .iter()
            .find(|country| country.canonical_slot == slot)
    })
    .ok_or_else(|| {
        format!(
            "organisation {}'s owner {} has no recorded country",
            organisation.organisation, organisation.owner
        )
    })
}

/// The policy roster of the organisation's owner, keyed by its country's id.
pub fn owner_roster<'a>(
    frame: &'a DecodedFrame,
    organisation: &rows::IndustrialOrganisationRow,
) -> Result<&'a rows::IndustrialOrganisationPoliciesRow, String> {
    let unavailable = || {
        format!(
            "organisation {} country policy roster is unavailable",
            organisation.organisation
        )
    };
    let owner = owner_country(frame, organisation).map_err(|_| unavailable())?;
    frame
        .complete::<tables::IndustrialOrganisationPolicies>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.country == owner.id)
        .ok_or_else(unavailable)
}

/// The whole political power an attach costs for a stored base cost `base`,
/// the organisation's local modifier sum and its owner's effective policy-cost
/// modifier, both signed fixed5: the factor is summed before it scales the
/// base, the result rounds half away from zero, and every step narrows as the
/// game's integer widths do. Nothing clamps a negative quote.
pub fn attach_quote(base: i32, local: i64, country: i64) -> i32 {
    let sum = ONE.wrapping_add(local).wrapping_add(country);
    // The game rescales twice; under overflow these steps do not cancel.
    let factor = sum.wrapping_mul(ONE) / ONE;
    let raw = i64::from(base).wrapping_mul(factor).wrapping_mul(ONE) / ONE;
    (raw.wrapping_add(if raw < 0 { -ONE / 2 } else { ONE / 2 }) / ONE) as i32
}

/// Whether the owner's political power `political_power_raw` (fixed5) affords `quote`.
pub fn affordable(political_power_raw: i64, quote: i32) -> bool {
    political_power_raw >= i64::from(quote) * ONE
}

/// Whether a policy row shows its attach cost: a positive quote on a policy not attached.
pub fn shows_attach_cost(quote: i32, attached: bool) -> bool {
    quote > 0 && !attached
}

/// What scales every policy's attach quote for one organisation: its local
/// modifier sum and its owner's effective policy-cost modifier, signed fixed5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyCost {
    pub local: i64,
    pub country: i64,
}
impl PolicyCost {
    /// The attach quote of a policy with stored base cost `base` ([`attach_quote`]).
    pub fn quote(&self, base: i32) -> i32 {
        attach_quote(base, self.local, self.country)
    }
}

/// The organisation's [`PolicyCost`]. The owner's modifier is the recorded
/// effective value; a failed or absent read is a named refusal, never zero.
pub fn policy_cost(
    frame: &DecodedFrame,
    organisation: &rows::IndustrialOrganisationRow,
    details: &rows::IndustrialOrganisationDetailsRow,
    traits: &installed_content_core::mio_traits::Catalog,
    policies: &installed_content_core::industrial_organisations::policies::Catalog,
    world: &dashboard_text::triggers::World<'_>,
) -> Result<PolicyCost, String> {
    let country = owner_roster(frame, organisation)?
        .cost_modifier_raw
        .ok_or_else(|| {
            format!(
                "organisation {} owner's policy cost modifier is not recorded",
                organisation.organisation
            )
        })?;
    let local = local_modifier(
        frame,
        organisation,
        details,
        engine_executable::modifier_keys::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST,
        traits,
        policies,
        world,
    )?;
    Ok(PolicyCost { local, country })
}

#[cfg(test)]
mod tests;
