//! Event-dated operands retained after observing the exact frame. A changing
//! UnitLeaders experience row never retains its table or changes these facts.
use super::*;
use dashboard_base::products::{
    character_portraits::{RecordedPortrait, recorded_portrait},
    production_lines::RawOrganisation,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CharacterFact {
    pub id: u32,
    pub script_token: Option<String>,
    pub saved_name: String,
    pub template: Result<installed_content_core::characters::BuiltTemplate, String>,
    pub leader_category: Option<i32>,
    pub portrait: RecordedPortrait,
}

/// Kept with its row. A day's costs repeat an advisor, idea, organisation or
/// cost modifier, so equal operands and texts share one allocation (`Shared`).
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct EventFacts {
    #[serde(with = "native_cause")]
    pub native_cause: PowerCause,
    pub token: Option<Arc<str>>,
    pub character: Option<Arc<CharacterFact>>,
    pub organisation: Option<Arc<RawOrganisation>>,
    /// Missing lexical spelling must not discard explicit name/icon overrides.
    pub organisation_token_known: bool,
    #[serde(with = "recorded_modifier")]
    pub modifier: Option<Arc<ModifierDefinitionRow>>,
    /// The item's text, resolved at the event frame.
    pub text: Option<Arc<ItemText>>,
}

pub(super) mod native_cause {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &PowerCause,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        (*value as u16).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PowerCause, D::Error> {
        PowerCause::try_from(u16::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

mod recorded_modifier {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &Option<Arc<ModifierDefinitionRow>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .as_ref()
            .map(|row| {
                super::super::prepared::checkpoint_frame(
                    0,
                    vec![TableRows::ModifierDefinitions(vec![(**row).clone()])],
                )
            })
            .transpose()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Arc<ModifierDefinitionRow>>, D::Error> {
        Option::<Vec<u8>>::deserialize(deserializer)?
            .map(|bytes| {
                let frame = super::super::prepared::read_checkpoint(&bytes)?;
                let rows = frame.complete::<tables::ModifierDefinitions>()?;
                match rows {
                    [row] => Ok(Arc::new(row.clone())),
                    _ => Err(dashboard_core::invalid("Invalid open politics modifier")),
                }
            })
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Default)]
pub(super) struct Shared {
    tokens: BTreeSet<Arc<str>>,
    texts: BTreeSet<Arc<ItemText>>,
    /// The latest fact of each character and organisation reference.
    characters: BTreeMap<u32, Option<Arc<CharacterFact>>>,
    organisations: BTreeMap<u64, Option<Arc<RawOrganisation>>>,
    modifier: Option<Arc<ModifierDefinitionRow>>,
}
impl Shared {
    pub(super) fn retain_open(&mut self) {
        self.tokens.retain(|value| Arc::strong_count(value) > 1);
        self.texts.retain(|value| Arc::strong_count(value) > 1);
        self.characters.retain(|_, value| {
            value
                .as_ref()
                .is_some_and(|value| Arc::strong_count(value) > 1)
        });
        self.organisations.retain(|_, value| {
            value
                .as_ref()
                .is_some_and(|value| Arc::strong_count(value) > 1)
        });
        if self
            .modifier
            .as_ref()
            .is_some_and(|value| Arc::strong_count(value) == 1)
        {
            self.modifier = None;
        }
    }
    pub(super) fn text(&mut self, text: ItemText) -> Arc<ItemText> {
        intern(&mut self.texts, text)
    }
}
/// The kept allocation of an equal value, else a new one, kept.
fn intern<T, V>(kept: &mut BTreeSet<Arc<T>>, value: V) -> Arc<T>
where
    T: Ord + ?Sized,
    V: std::borrow::Borrow<T> + Into<Arc<T>>,
{
    if let Some(old) = kept.get(value.borrow()) {
        return Arc::clone(old);
    }
    let value = value.into();
    kept.insert(Arc::clone(&value));
    value
}
/// `kept` if it equals `value`, else `value`, kept in its place.
fn share<T: PartialEq>(kept: &mut Option<Arc<T>>, value: T) -> Arc<T> {
    if let Some(old) = kept.as_ref().filter(|old| ***old == value) {
        return Arc::clone(old);
    }
    Arc::clone(kept.insert(Arc::new(value)))
}
/// `share` for operands dated by the first event that recorded them: this
/// ledger never reads that hour.
fn share_organisation(
    kept: &mut Option<Arc<RawOrganisation>>,
    value: RawOrganisation,
) -> Arc<RawOrganisation> {
    let bare = |raw: &RawOrganisation| RawOrganisation {
        first_hour: 0,
        ..raw.clone()
    };
    let first_hour = match kept.as_deref() {
        Some(old) if bare(old) == bare(&value) => old.first_hour,
        _ => value.first_hour,
    };
    share(
        kept,
        RawOrganisation {
            first_hour,
            ..value
        },
    )
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

/// The character an item records, directly or through the unit leader it
/// references, and the script token and saved name that name it.
struct RecordedCharacter<'f> {
    row: &'f CharacterRow,
    leader: Option<&'f UnitLeaderRow>,
    script_token: Option<&'f str>,
    saved_name: &'f str,
}
fn recorded_character<'f>(
    frame: &'f DecodedFrame,
    item: &LedgerItem,
) -> Option<RecordedCharacter<'f>> {
    let leader = (item.object_kind == "Leader")
        .then(|| {
            let reference = item.object_reference.as_deref()?.parse::<u64>().ok()?;
            let leaders = frame.complete::<tables::UnitLeaders>().ok()?;
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
    let row = &rows[rows.binary_search_by_key(&id, |row| row.id).ok()?];
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    Some(RecordedCharacter {
        row,
        leader,
        script_token: lexical.and_then(|lexical| character_script_token(row, lexical)),
        saved_name: std::str::from_utf8(&row.token).ok()?,
    })
}
pub(super) fn character_fact(
    frame: &DecodedFrame,
    item: &LedgerItem,
    cause: PowerCause,
) -> Option<CharacterFact> {
    let recorded = recorded_character(frame, item)?;
    let id = recorded.row.id;
    let category = recorded
        .leader
        .or_else(|| {
            frame
                .complete::<tables::UnitLeaders>()
                .ok()?
                .iter()
                .find(|leader| leader.character_id == id)
        })
        .map(|leader| leader.category);
    // Role creation reads the category mask; upgrades request
    // small army/navy portraits.
    let (role, size) = match cause {
        PowerCause::Advisor | PowerCause::AdvisorRemoval | PowerCause::Idea => (
            replay_data::rows::PORTRAIT_CIVILIAN,
            replay_data::rows::PORTRAIT_LARGE,
        ),
        PowerCause::OfficerUpgrade => (1, 0),
        PowerCause::CaptainUpgrade => (2, 0),
        PowerCause::ScientistRecruitment | PowerCause::ScientistFaction => {
            (4, replay_data::rows::PORTRAIT_LARGE)
        }
        _ => (
            match category {
                Some(0 | 1) => 1,
                Some(2) => 2,
                Some(3) => 4,
                _ => u32::MAX,
            },
            replay_data::rows::PORTRAIT_LARGE,
        ),
    };
    Some(CharacterFact {
        id,
        script_token: recorded.script_token.map(str::to_owned),
        saved_name: recorded.saved_name.to_owned(),
        template: dashboard_text::identity::built_template(
            recorded.row,
            frame.complete::<tables::LexicalTokens>().ok(),
        ),
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
/// [`character_name`] of the character an item records, without building the
/// portrait, template and leader category its event fact keeps: a holder
/// names its character every hour.
pub(super) fn recorded_name(
    frame: &DecodedFrame,
    item: &LedgerItem,
    content: &PoliticsContent,
) -> Option<Option<String>> {
    let recorded = recorded_character(frame, item)?;
    Some(content.character_name(recorded.script_token, recorded.saved_name))
}
pub(super) fn event_facts(
    shared: &mut Shared,
    row: &LedgerMutation,
    frame: Option<&DecodedFrame>,
    cause: PowerCause,
    component: Option<PowerDailyCostKind>,
) -> EventFacts {
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
            .and_then(|links| dashboard_text::metadata::link_slot(links, organisation.owner))
            .and_then(|slot| {
                countries?
                    .iter()
                    .find(|country| country.canonical_slot == slot)
            })
            .and_then(|country| std::str::from_utf8(&country.tag).ok())
            .map(str::to_owned);
        let raw = RawOrganisation {
            first_hour: row.hour,
            // The witness above guards every token-dependent selector; this
            // placeholder is never used to select a fallback icon.
            token: token.unwrap_or_default(),
            name_key: std::str::from_utf8(&organisation.name).ok()?.to_owned(),
            icon: std::str::from_utf8(&organisation.icon).ok()?.to_owned(),
            owner_unknown: (organisation.owner as i32) > 0 && owner_tag.is_none(),
            owner_tag,
        };
        Some((reference, raw))
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
    EventFacts {
        native_cause: cause,
        token: token.map(|token| intern(&mut shared.tokens, token)),
        character: character.map(|fact| share(shared.characters.entry(fact.id).or_default(), fact)),
        organisation: organisation.map(|(reference, raw)| {
            share_organisation(shared.organisations.entry(reference).or_default(), raw)
        }),
        organisation_token_known,
        modifier: modifier.map(|definition| share(&mut shared.modifier, definition)),
        text: None,
    }
}

#[cfg(test)]
mod tests;
