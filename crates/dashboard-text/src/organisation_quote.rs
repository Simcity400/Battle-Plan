use crate::triggers::{Evaluator, OrganisationCondition, Undecided, World};
use content_reader_military::{industrial_organisations::policies, mio_traits};
use content_reader_script::modifiers::Modifiers;
use replay_data::rows::{self, tables};
use replay_frame::frame::DecodedFrame;

pub const ONE: i64 = 100_000;

fn spelling(frame: &DecodedFrame, token: u32) -> Result<&[u8], String> {
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|error| error.to_string())?;
    let index = tokens
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| format!("lexical tokens lacks token {token}"))?;
    Ok(content_reader_parser::parser::c_string(&tokens[index].literal))
}

fn spelled(frame: &DecodedFrame, token: u32) -> Result<&str, String> {
    std::str::from_utf8(spelling(frame, token)?).map_err(|error| error.to_string())
}

pub fn modifier_eligible(frame: &DecodedFrame, name: &str) -> Result<bool, String> {
    let definition = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|error| error.to_string())?
        .iter()
        .find(|row| spelling(frame, row.token).is_ok_and(|token| token.eq_ignore_ascii_case(name.as_bytes())))
        .ok_or_else(|| format!("organisation modifier {name} definition unavailable (count: 1)"))?;
    if definition.feature_selector == 0 {
        return Ok(true);
    }
    if definition.eligibility == 0 {
        return Ok(false);
    }
    let [features] = frame
        .complete::<tables::SimulationFeatures>()
        .map_err(|error| error.to_string())?
    else {
        return Err("organisation modifier feature mask is not unique (count: 1)".into());
    };
    Ok(features.dlc_mask & definition.feature_selector != 0)
}

pub fn organisation_modifier_value(
    frame: &DecodedFrame,
    modifiers: &Modifiers,
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
        .map_err(|error| error.to_string())?
        .iter()
        .find(|row| row.index == key)
        .ok_or_else(|| format!("organisation modifier {modifier_name} definition unavailable (count: 1)"))?;
    Ok(
        if definition.flags & engine_native_parts::modifier_definition::CLAMP_MAXIMUM != 0 {
            entry.value.clamp(0, ONE)
        } else {
            entry.value
        },
    )
}

pub fn local_modifier(
    frame: &DecodedFrame,
    organisation: &rows::IndustrialOrganisationRow,
    details: &rows::IndustrialOrganisationDetailsRow,
    key: u32,
    traits: &mio_traits::Catalog,
    policies: &policies::Catalog,
    world: Result<&World<'_>, String>,
) -> Result<i64, Undecided> {
    let reference = organisation.organisation;
    let name = spelled(frame, organisation.token)?;
    let tree = traits
        .tree(name)?
        .ok_or_else(|| format!("organisation {name} traits are absent"))?;
    let modifier_name =
        engine_executable::modifier_keys::fixed_name(key).map_err(|error| error.to_string())?;
    let value = |modifiers: &Modifiers| organisation_modifier_value(frame, modifiers, modifier_name, key);
    let mut local = 0_i64;
    let ordinary = if tree.visible.is_empty() && tree.available.is_empty() {
        true
    } else {
        Evaluator::new(world?, None).organisation_conditions(
            reference,
            &[
                (OrganisationCondition::Visible, tree.visible),
                (OrganisationCondition::Available, tree.available),
            ],
        )?
    };
    if ordinary {
        for token in &details.unlocked {
            let token = spelled(frame, *token)?;
            if let Some(definition) = traits.trait_definition(name, token)? {
                let modifiers =
                    definition.resolved_organisation_modifiers(|name| modifier_eligible(frame, name))?;
                local = local.wrapping_add(value(&modifiers)?);
            }
        }
    }
    if let Some(initial) = tree.initial {
        let modifiers = initial.resolved_organisation_modifiers(|name| modifier_eligible(frame, name))?;
        local = local.wrapping_add(value(&modifiers)?);
    }
    if let Some(selected) = owner_roster(frame, organisation)?
        .policies
        .iter()
        .find(|policy| policy.token == details.state.policy)
    {
        local = local.wrapping_add(value(
            &policies.policy(spelling(frame, selected.token)?)?.organisation_modifiers,
        )?);
    }
    Ok(local)
}

pub fn owner_country<'a>(
    frame: &'a DecodedFrame,
    organisation: &rows::IndustrialOrganisationRow,
) -> Result<&'a rows::CountryRow, String> {
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|error| error.to_string())?;
    let slot = crate::metadata::link_slot(links, organisation.owner);
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
        .map_err(|error| error.to_string())?
        .iter()
        .find(|row| row.country == owner.id)
        .ok_or_else(unavailable)
}

pub fn attach_quote(base: i32, local: i64, country: i64) -> i32 {
    let sum = ONE.wrapping_add(local).wrapping_add(country);
    let factor = sum.wrapping_mul(ONE) / ONE;
    let raw = i64::from(base).wrapping_mul(factor).wrapping_mul(ONE) / ONE;
    (raw.wrapping_add(if raw < 0 { -ONE / 2 } else { ONE / 2 }) / ONE) as i32
}

pub fn shows_attach_cost(quote: i32, attached: bool) -> bool {
    quote > 0 && !attached
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyCost {
    pub local: i64,
    pub country: i64,
}

impl PolicyCost {
    pub fn quote(&self, base: i32) -> i32 {
        attach_quote(base, self.local, self.country)
    }
}

pub fn policy_cost(
    frame: &DecodedFrame,
    organisation: &rows::IndustrialOrganisationRow,
    details: &rows::IndustrialOrganisationDetailsRow,
    traits: &mio_traits::Catalog,
    policies: &policies::Catalog,
    world: Result<&World<'_>, String>,
) -> Result<PolicyCost, Undecided> {
    let rows::Recorded::Value(Some(country)) = owner_roster(frame, organisation)?.cost_modifier_raw else {
        return Err(format!(
            "organisation {} owner's policy cost modifier is not recorded",
            organisation.organisation
        )
        .into());
    };
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
