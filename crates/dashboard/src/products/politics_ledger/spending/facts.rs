//! Event-dated operands retained after observing the exact frame. A changing
//! UnitLeaders experience row never retains its table or changes these facts.
use super::*;
use crate::products::{
    character_portraits::{RecordedPortrait, recorded_portrait},
    production_lines::RawOrganisation,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CharacterFact {
    pub id: u32,
    pub script_token: Option<String>,
    pub saved_name: String,
    pub template: Result<installed_content::characters::BuiltTemplate, String>,
    pub leader_category: Option<i32>,
    pub portrait: RecordedPortrait,
}

pub(super) struct EventFacts {
    pub native_cause: PowerCause,
    pub token: Option<String>,
    pub character: Option<CharacterFact>,
    pub organisation: Option<RawOrganisation>,
    /// Missing lexical spelling must not discard explicit name/icon overrides.
    pub organisation_token_known: bool,
    pub modifier: Option<ModifierDefinitionRow>,
}

pub(super) fn token_identity(frame: &DecodedFrame, item: &LedgerItem) -> Option<String> {
    if !item.key.is_empty() {
        return std::str::from_utf8(&item.key).ok().map(str::to_owned);
    }
    lexical_token(frame, item.token?)
}
pub(super) fn lexical_token(frame: &DecodedFrame, token: u32) -> Option<String> {
    let rows = frame.complete::<tables::LexicalTokens>().ok()?;
    let index = rows.binary_search_by_key(&token, |row| row.token).ok()?;
    std::str::from_utf8(&rows[index].literal)
        .ok()
        .map(str::to_owned)
}

pub(super) fn character_fact(
    frame: &DecodedFrame,
    item: &LedgerItem,
    cause: PowerCause,
) -> Option<CharacterFact> {
    let leaders = frame.complete::<tables::UnitLeaders>().ok();
    let leader = (item.object_kind == "Leader")
        .then(|| {
            let reference = item.object_reference.as_deref()?.parse::<u64>().ok()?;
            let leaders = leaders?;
            let index = leaders
                .binary_search_by_key(&reference, |row| {
                    (u64::from(row.id) << 32) | u64::from(row.reference_type)
                })
                .ok()?;
            Some(&leaders[index])
        })
        .flatten();
    let id = item
        .character
        .or_else(|| leader.map(|row| row.character_id))?;
    let rows = frame.complete::<tables::Characters>().ok()?;
    let index = rows.binary_search_by_key(&id, |row| row.id).ok()?;
    let recorded = &rows[index];
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    let category = leader
        .or_else(|| {
            leaders.and_then(|leaders| leaders.iter().find(|leader| leader.character_id == id))
        })
        .map(|leader| leader.category);
    // Role creation reads the category mask; upgrades request
    // small army/navy portraits.
    let (role, size) = match cause {
        PowerCause::Advisor | PowerCause::AdvisorRemoval | PowerCause::Idea => (
            replay::rows::PORTRAIT_CIVILIAN,
            replay::rows::PORTRAIT_LARGE,
        ),
        PowerCause::OfficerUpgrade => (1, 0),
        PowerCause::CaptainUpgrade => (2, 0),
        PowerCause::ScientistRecruitment | PowerCause::ScientistFaction => {
            (4, replay::rows::PORTRAIT_LARGE)
        }
        _ => (
            match category {
                Some(0 | 1) => 1,
                Some(2) => 2,
                Some(3) => 4,
                _ => u32::MAX,
            },
            replay::rows::PORTRAIT_LARGE,
        ),
    };
    Some(CharacterFact {
        id,
        script_token: lexical
            .and_then(|lexical| character_script_token(recorded, lexical))
            .map(str::to_owned),
        saved_name: std::str::from_utf8(&recorded.token).ok()?.to_owned(),
        template: crate::products::identity::built_template(recorded, lexical),
        leader_category: category,
        portrait: if role == u32::MAX {
            RecordedPortrait::Unrecorded
        } else {
            recorded_portrait(
                frame.complete::<tables::CharacterPortraits>().ok(),
                id,
                role,
                size,
            )
        },
    })
}
pub(super) fn character_name(fact: &CharacterFact, content: &PoliticsContent) -> Option<String> {
    content.character_name(fact.script_token.as_deref(), &fact.saved_name)
}
pub(super) fn store_facts(
    scan: &mut Scanned,
    row: &LedgerMutation,
    frame: Option<&DecodedFrame>,
    cause: PowerCause,
    component: Option<PowerDailyCostKind>,
) {
    let key = item_text_id(row);
    if scan.event_facts.contains_key(&key) {
        return;
    }
    // Late publications must not borrow the current frame's identities.
    let frame = frame.filter(|frame| frame.hour == row.hour);
    let character = frame.and_then(|frame| character_fact(frame, &row.item, cause));
    let mut token = if row.item.key.is_empty() {
        frame.and_then(|frame| token_identity(frame, &row.item))
    } else {
        std::str::from_utf8(&row.item.key).ok().map(str::to_owned)
    };
    if token.is_none() && matches!(cause, PowerCause::Advisor | PowerCause::AdvisorRemoval) {
        token = frame.and_then(|frame| {
            let id = character.as_ref()?.id;
            frame
                .complete::<tables::AdvisorRoles>()
                .ok()?
                .iter()
                .find(|role| {
                    role.character == id
                        && (row.item.slot_key.is_empty() || role.slot == row.item.slot_key)
                })
                .and_then(|role| std::str::from_utf8(&role.idea).ok())
                .filter(|idea| !idea.is_empty())
                .map(str::to_owned)
        });
    }
    let mut organisation_token_known = false;
    let organisation = frame.and_then(|frame| {
        if row.item.object_kind != "Organisation" {
            return None;
        }
        let reference = row.item.object_reference.as_deref()?.parse::<u64>().ok()?;
        let rows = frame.complete::<tables::IndustrialOrganisations>().ok()?;
        let index = rows
            .binary_search_by_key(&reference, |row| row.organisation)
            .ok()?;
        let organisation = &rows[index];
        let token = lexical_token(frame, organisation.token);
        organisation_token_known = token.is_some();
        let links = frame.complete::<tables::CountryLinks>().ok();
        let countries = frame.complete::<tables::Countries>().ok();
        let owner_tag = links
            .and_then(|links| crate::metadata::link_slot(links, organisation.owner))
            .and_then(|slot| {
                countries?
                    .iter()
                    .find(|country| country.canonical_slot == slot)
            })
            .and_then(|country| std::str::from_utf8(&country.tag).ok())
            .map(str::to_owned);
        Some(RawOrganisation {
            first_hour: row.hour,
            // The witness above guards every token-dependent selector; this
            // placeholder is never used to select a fallback icon.
            token: token.unwrap_or_default(),
            name_key: std::str::from_utf8(&organisation.name).ok()?.to_owned(),
            icon: std::str::from_utf8(&organisation.icon).ok()?.to_owned(),
            owner_unknown: (organisation.owner as i32) > 0 && owner_tag.is_none(),
            owner_tag,
        })
    });
    // The daily getter reads modifier index 0x26. Its title and argument
    // come from that read's ModifierDefinitions, never a vanilla key.
    let modifier = (component == Some(PowerDailyCostKind::PoliticalPowerCostModifier))
        .then(|| {
            frame?
                .complete::<tables::ModifierDefinitions>()
                .ok()?
                .iter()
                .find(|row| row.index == 0x26)
                .cloned()
        })
        .flatten();
    let facts = EventFacts {
        native_cause: cause,
        token,
        character,
        organisation,
        organisation_token_known,
        modifier,
    };
    scan.event_facts.insert(key, facts);
}

#[cfg(test)]
mod tests;
