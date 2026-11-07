//! What a payment's item is at its own hour: the recorded operands it names there (its
//! character, token, organisation, cost modifier), the names of its country and target, and
//! the scripted text and icon conditions the game evaluates against that hour's world. Only
//! the hour's frame says these, so the segment operator reads them as the row is dated; the
//! reply names the item from them when it is asked ([`super::present`]).

use super::*;
use dashboard_base::products::{
    character_portraits::{RecordedPortrait, recorded_portrait_in},
    idea_history,
    politics_content::PoliticsContent,
    production_lines::RawOrganisation,
};
use dashboard_text::catalogue::{View, plain_text};
use dashboard_text::identity::{Look, NamingCountry};
use installed_content_country_names::country_names::CountryNames;
use replay_frame::frame::{ColumnFrame, HourFrame, Rowset, Tables};
use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CharacterFact {
    pub id: u32,
    pub script_token: Option<String>,
    pub saved_name: String,
    pub template: Result<installed_content_core::characters::BuiltTemplate, String>,
    pub leader_category: Option<i32>,
    pub portrait: RecordedPortrait,
}

/// The cost modifier a daily political power cost names: its title and argument as the
/// hour's ModifierDefinitions recorded them, never a vanilla key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CostModifier {
    pub name_key: Vec<u8>,
    pub argument: Vec<u8>,
}

/// A recorded modifier definition, as an idea's tooltip formats its effects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RecordedModifier {
    pub index: u32,
    pub token: u32,
    pub name_key: Vec<u8>,
    pub postfix_key: Vec<u8>,
    pub argument: Vec<u8>,
    pub flags: u32,
    pub category_mask: u32,
    pub feature_selector: u32,
    pub precision: i32,
    pub eligibility: u8,
}

impl From<&ModifierDefinitionRow> for RecordedModifier {
    fn from(row: &ModifierDefinitionRow) -> Self {
        Self {
            index: row.index,
            token: row.token,
            name_key: row.name_key.clone(),
            postfix_key: row.postfix_key.clone(),
            argument: row.argument.clone(),
            flags: row.flags,
            category_mask: row.category_mask,
            feature_selector: row.feature_selector,
            precision: row.precision,
            eligibility: row.eligibility,
        }
    }
}

impl RecordedModifier {
    pub(super) fn row(&self) -> ModifierDefinitionRow {
        ModifierDefinitionRow {
            index: self.index,
            token: self.token,
            name_key: self.name_key.clone(),
            postfix_key: self.postfix_key.clone(),
            argument: self.argument.clone(),
            flags: self.flags,
            category_mask: self.category_mask,
            feature_selector: self.feature_selector,
            precision: self.precision,
            eligibility: self.eligibility,
        }
    }
}

/// The operands a row's item names at its hour.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct EventFacts {
    pub token: Option<String>,
    pub character: Option<CharacterFact>,
    pub organisation: Option<RawOrganisation>,
    /// Missing lexical spelling must not discard explicit name/icon overrides.
    pub organisation_token_known: bool,
    pub modifier: Option<CostModifier>,
    /// The item's text, resolved at the event frame.
    pub text: Option<ItemText>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct ItemText {
    pub name: Option<String>,
    pub sprites: Option<SpriteChoice>,
    pub description: Option<String>,
    pub unknown: Option<String>,
}

/// A decision's or focus's icon as the hour leaves it: its dated conditions evaluated at the
/// event hour, the sprites it can show in the order the game tries them. Which of them the
/// presentation declares is decided when a reply is served.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SpriteChoice {
    /// Tried in order: the first the presentation declares is shown.
    pub(super) candidates: Vec<String>,
    /// Shown when none is declared: a sprite the game shows without asking whether it is
    /// declared, nothing, or why the choice cannot be made.
    pub(super) otherwise: Result<Option<String>, String>,
}

/// A country as an event's hour names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct NamedCountry {
    pub tag: String,
    pub name: Option<String>,
    pub naming: NamingCountry,
    pub ruling_group: Result<Option<String>, String>,
}

/// Everything a row's item reads at its hour, interned by the emit and read by the reply.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Dated {
    pub facts: EventFacts,
    pub own: Option<NamedCountry>,
    pub target: Option<NamedCountry>,
    /// The recorded definitions of the modifiers its idea's tooltip formats: None where the
    /// hour recorded none, empty where it formats none.
    pub modifiers: Option<Vec<(String, RecordedModifier)>>,
}

/// The fields an item's dating reads of each table it reads by row.
pub(super) const LITERAL: u64 = mask::<LexicalTokenRow>(&["literal"]);
const LEADER: u64 = mask::<UnitLeaderRow>(&["character_id", "category"]);
const ROLE: u64 = mask::<AdvisorRoleRow>(&["idea", "slot"]);
const ORGANISATION: u64 = mask::<IndustrialOrganisationRow>(&["owner", "token", "name", "icon"]);
const PORTRAIT: u64 = mask::<CharacterPortraitRow>(&["portraits"]);
pub(super) const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
pub(super) const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const CHARACTER: u64 = dashboard_text::identity::CHARACTER_IDENTITY;
const COST_MODIFIER: u64 = mask::<ModifierDefinitionRow>(&["name_key", "argument"]);
const LITERAL_COLUMN: usize = column::<LexicalTokenRow>("literal");

/// The tables an hour's scripted text and trigger conditions read: the walk's views at its own
/// hour, with its lexer folds, or the copies carried from the hour before.
#[derive(Clone, Copy)]
pub(super) enum TextFrame<'h> {
    Walked {
        views: ColumnFrame<'h>,
        lexicon: Option<&'h dashboard_world::Lexicon>,
    },
    /// The hour before, as the copies carried from it hold it.
    Carried(&'h DecodedFrame),
}

impl<'h> TextFrame<'h> {
    pub(super) fn walked(
        world: &'h dashboard_world::World,
        state: &'h replay::segments::State,
    ) -> Self {
        Self::Walked {
            views: world.columns(state),
            lexicon: world.lexicon(),
        }
    }

    /// The hour's tables: the walk's views, or the copies.
    fn tables(&self) -> HourFrame<'h> {
        match *self {
            Self::Walked { views, .. } => HourFrame::Walked(views),
            Self::Carried(frame) => HourFrame::from(frame),
        }
    }

    /// The walk's lexer folds; none for the copies.
    fn lexicon(&self) -> Option<&'h dashboard_world::Lexicon> {
        match *self {
            Self::Walked { lexicon, .. } => lexicon,
            Self::Carried(_) => None,
        }
    }
}

/// `table` of `frame` when complete, each row holding the fields `mask` names.
fn complete<'f, F: Tables + ?Sized, T: TableSchema>(
    frame: &'f F,
    mask: u64,
) -> Option<Rowset<'f, T::Row>> {
    frame.complete_rows::<T>(mask).ok()
}

/// The row keyed `key`; none where it is not held or cannot be built.
fn row<'f, R: RowCodec>(rows: Option<Rowset<'f, R>>, key: u128) -> Option<Cow<'f, R>> {
    rows?.get(key).ok().flatten()
}

pub(super) fn lexical_token<F: Tables + ?Sized>(frame: &F, token: u32) -> Option<String> {
    let rows = complete::<F, tables::LexicalTokens>(frame, LITERAL)?;
    spelling(rows, token)
}

fn spelling(rows: Rowset<'_, LexicalTokenRow>, token: u32) -> Option<String> {
    let literal = rows
        .text(u128::from(token), |row| &row.literal, LITERAL_COLUMN)
        .ok()
        .flatten()?;
    std::str::from_utf8(literal).ok().map(str::to_owned)
}

pub(super) fn token_identity<F: Tables + ?Sized>(
    frame: &F,
    identity: &PowerIdentity,
) -> Option<String> {
    if !identity.item_key.is_empty() {
        return std::str::from_utf8(&identity.item_key)
            .ok()
            .map(str::to_owned);
    }
    lexical_token(frame, identity.item_token?)
}

/// The character an item records, directly or through the unit leader it references.
struct RecordedCharacter<'f> {
    row: Cow<'f, CharacterRow>,
    leader: Option<Cow<'f, UnitLeaderRow>>,
    script_token: Option<&'f str>,
}

fn recorded_character<'f, F: Tables + ?Sized>(
    frame: &'f F,
    identity: &PowerIdentity,
) -> Option<RecordedCharacter<'f>> {
    let leader = (identity.object_kind == PowerObjectKind::Leader)
        .then(|| {
            let reference = identity.object_reference?;
            row(
                complete::<F, tables::UnitLeaders>(frame, LEADER),
                u128::from(reference),
            )
        })
        .flatten();
    let id = identity
        .character
        .or_else(|| leader.as_ref().map(|row| row.character_id))?;
    let row = row(
        complete::<F, tables::Characters>(frame, CHARACTER),
        u128::from(id),
    )?;
    let lexical = complete::<F, tables::LexicalTokens>(frame, LITERAL);
    Some(RecordedCharacter {
        script_token: lexical.and_then(|lexical| {
            dashboard_text::identity::character_script_token_in(&row, lexical)
        }),
        row,
        leader,
    })
}

fn character_fact<F: Tables + ?Sized>(
    frame: &F,
    identity: &PowerIdentity,
    cause: PowerCause,
) -> Option<CharacterFact> {
    let recorded = recorded_character(frame, identity)?;
    let id = recorded.row.id;
    let category = match &recorded.leader {
        Some(leader) => Some(leader.category),
        None => complete::<F, tables::UnitLeaders>(frame, LEADER).and_then(|leaders| {
            leaders
                .iter()
                .filter_map(Result::ok)
                .find(|leader| leader.character_id == id)
                .map(|leader| leader.category)
        }),
    };
    // Role creation reads the category mask; upgrades request small army/navy portraits.
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
        saved_name: std::str::from_utf8(&recorded.row.token).ok()?.to_owned(),
        template: dashboard_text::identity::built_template_in(
            &recorded.row,
            complete::<F, tables::LexicalTokens>(frame, LITERAL),
        ),
        leader_category: category,
        portrait: if role == u32::MAX {
            RecordedPortrait::Unrecorded
        } else {
            recorded_portrait_in(
                complete::<F, tables::CharacterPortraits>(frame, PORTRAIT),
                id,
                role,
                size,
            )
        },
    })
}

/// The first Countries row at canonical `slot`, in key order.
fn first_at<F: Tables + ?Sized>(frame: &F, slot: u32) -> Option<CountryRow> {
    complete::<F, tables::Countries>(frame, COUNTRY)?
        .iter()
        .filter_map(Result::ok)
        .find(|country| country.canonical_slot == slot)
        .map(Cow::into_owned)
}

/// The operands `identity`, a row of `cause` dated at `frame`'s hour, names there.
pub(super) fn event_facts<F: Tables + ?Sized>(
    frame: Option<&F>,
    identity: &PowerIdentity,
    cause: PowerCause,
    component: Option<PowerDailyCostKind>,
) -> EventFacts {
    let character = frame.and_then(|frame| character_fact(frame, identity, cause));
    let mut token = if identity.item_key.is_empty() {
        frame.and_then(|frame| token_identity(frame, identity))
    } else {
        std::str::from_utf8(&identity.item_key)
            .ok()
            .map(str::to_owned)
    };
    if token.is_none() && matches!(cause, PowerCause::Advisor | PowerCause::AdvisorRemoval) {
        token = (|| {
            let frame = frame?;
            let id = character.as_ref()?.id;
            // A character's roles are keyed by it, then by ordinal.
            let first = u128::from(id) << 32;
            complete::<F, tables::AdvisorRoles>(frame, ROLE)?
                .range(first..=first | u128::from(u32::MAX))
                .filter_map(Result::ok)
                .find(|role| identity.slot_key.is_empty() || role.slot == identity.slot_key)
                .and_then(|role| {
                    std::str::from_utf8(&role.idea)
                        .ok()
                        .filter(|idea| !idea.is_empty())
                        .map(str::to_owned)
                })
        })();
    }
    let mut organisation_token_known = false;
    let organisation = (|| {
        let frame = frame?;
        if identity.object_kind != PowerObjectKind::Organisation {
            return None;
        }
        let reference = identity.object_reference?;
        let organisation = row(
            complete::<F, tables::IndustrialOrganisations>(frame, ORGANISATION),
            u128::from(reference),
        )?;
        let token = lexical_token(frame, organisation.token);
        organisation_token_known = token.is_some();
        let links = complete::<F, tables::CountryLinks>(frame, LINK);
        let owner_tag = links
            .and_then(|links| dashboard_text::metadata::link_slot_in(&links, organisation.owner))
            .and_then(|slot| first_at(frame, slot))
            .and_then(|country| String::from_utf8(country.tag).ok());
        Some(RawOrganisation {
            first_hour: frame.at_hour(),
            // The witness above guards every token-dependent selector; this placeholder is
            // never used to select a fallback icon.
            token: token.unwrap_or_default(),
            name_key: std::str::from_utf8(&organisation.name).ok()?.to_owned(),
            icon: std::str::from_utf8(&organisation.icon).ok()?.to_owned(),
            owner_unknown: (organisation.owner as i32) > 0 && owner_tag.is_none(),
            owner_tag,
            state: None,
        })
    })();
    // The daily getter reads modifier index 0x26.
    let modifier = (component == Some(PowerDailyCostKind::PoliticalPowerCostModifier))
        .then(|| {
            let row = row(
                complete::<F, tables::ModifierDefinitions>(frame?, COST_MODIFIER),
                0x26,
            )?;
            Some(CostModifier {
                name_key: row.name_key.clone(),
                argument: row.argument.clone(),
            })
        })
        .flatten();
    EventFacts {
        token,
        character,
        organisation,
        organisation_token_known,
        modifier,
        text: None,
    }
}

/// The names at `frame`'s hour of the countries `slots` finds, raw country to census row: each
/// valid tag's, with its look and ruling group.
pub(super) fn named_countries<F: Tables + ?Sized>(
    frame: &F,
    content: Option<&PoliticsContent>,
    slots: &[(u32, CountryRow)],
) -> Vec<(u32, NamedCountry)> {
    let names = content.map(|content| {
        CountryNames::read_selected(
            frame,
            &content.catalogue.localisation,
            slots.iter().map(|(_, row)| row.canonical_slot),
        )
    });
    let mut named = Vec::with_capacity(slots.len());
    for (raw, country) in slots {
        let Ok(tag) = std::str::from_utf8(&country.tag) else {
            continue;
        };
        if dashboard_base::products::validate_tag(tag).is_err() {
            continue;
        }
        let look = Look::read(frame, country).map(|mut look| {
            look.civil_war = look
                .civil_war
                .map_err(|reason| rows::undated_reason(&reason));
            look
        });
        named.push((
            *raw,
            NamedCountry {
                tag: tag.to_owned(),
                name: names
                    .as_ref()
                    .and_then(|names| names.name(country.canonical_slot).ok())
                    .and_then(|name| plain_text(&name).ok()),
                naming: NamingCountry {
                    slot: country.canonical_slot,
                    look,
                },
                ruling_group: names.as_ref().map_or_else(
                    || Err("recorded content unresolved".into()),
                    |names| {
                        names
                            .ruling_group(country.canonical_slot)
                            .map(|group| group.map(str::to_owned))
                            .map_err(|reason| rows::undated_reason(&reason))
                    },
                ),
            },
        ));
    }
    named
}

/// One hour's item texts share naming and scope inputs across country roots.
#[derive(Default)]
pub(super) struct FrameTexts<'a> {
    world: std::sync::OnceLock<Result<dashboard_text::triggers::World<'a>, String>>,
    scripted: std::cell::RefCell<
        BTreeMap<u32, std::rc::Rc<dashboard_text::scripted_text::ScriptedText<'a>>>,
    >,
    /// Each organisation's name by reference and root: an organisation's research and
    /// production costs name it alike.
    organisations: std::cell::RefCell<BTreeMap<(u64, u32), Result<String, String>>>,
}

impl<'a> FrameTexts<'a> {
    /// The hour's scripted-text context for `root`, built once.
    fn scripted(
        &self,
        content: &'a PoliticsContent,
        frame: TextFrame<'a>,
        root: u32,
    ) -> Result<std::rc::Rc<dashboard_text::scripted_text::ScriptedText<'a>>, String> {
        if let Some(scripted) = self.scripted.borrow().get(&root) {
            return Ok(std::rc::Rc::clone(scripted));
        }
        let scripted = if let Some(existing) = self.scripted.borrow().values().next() {
            existing.for_root(root)
        } else {
            dashboard_text::scripted_text::ScriptedText::new_in(
                frame.tables(),
                &content.catalogue.localisation,
                content.catalogue.scripted_content()?,
                root,
            )
            .with_lexicon(frame.lexicon())
        };
        let scripted = std::rc::Rc::new(scripted);
        self.scripted
            .borrow_mut()
            .insert(root, std::rc::Rc::clone(&scripted));
        Ok(scripted)
    }
}

/// The text of the item `row` names, dated `frame`'s hour, with its country named `own`.
pub(super) fn item_text<'a>(
    own: Option<&NamedCountry>,
    row: &Item<'_>,
    facts: &EventFacts,
    content: &'a PoliticsContent,
    frame: Option<&'a TextFrame<'a>>,
    texts: &FrameTexts<'a>,
) -> Option<ItemText> {
    let mut text = event_text(own, row, facts, content, frame, texts)?;
    text.unknown = text.unknown.map(|reason| rows::undated_reason(&reason));
    Some(text)
}

/// What [`item_text`] reads of a power or cost row.
pub(super) struct Item<'r> {
    pub cause: PowerCause,
    pub identity: &'r PowerIdentity,
}

fn localise_at<'a>(
    content: &'a PoliticsContent,
    key: &str,
    frame: Option<&'a TextFrame<'a>>,
    texts: &FrameTexts<'a>,
    slot: Option<u32>,
    decision: Option<(&Item<'_>, bool)>,
    unknown_seed: bool,
) -> Result<Option<String>, String> {
    if !content.catalogue.has_presentation_key(key) {
        return Err(format!(
            "localisation {key} is missing; title is unavailable"
        ));
    }
    let raw = content
        .catalogue
        .localisation
        .localise(key.as_bytes())
        .map_err(|error| error.to_string())?;
    if !raw.contains('[') {
        return Ok(content.localised(key, View::Scripted));
    }
    let slot = slot.ok_or_else(|| {
        format!("localisation {key}: country identity is unavailable at action hour")
    })?;
    let frame = *frame
        .ok_or_else(|| format!("localisation {key}: its event-hour frame is unavailable"))?;
    let scripted = texts.scripted(content, frame, slot)?;
    if let Some((decision, selects_state)) = decision {
        use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
        // The recorded identity supplies the country target, but not the name seed or the
        // conditional state-target selector.
        let mut root = EventScope::of(EventObject::Country(slot));
        if selects_state {
            root.from = Link::Unknown;
        }
        let mut scopes = EventScopes::new(vec![root]);
        let mut unknown_seeds = BTreeSet::from([0]);
        if let Some(raw) = decision.identity.target_country {
            let target = row(
                complete::<_, tables::CountryLinks>(&frame.tables(), LINK),
                u128::from(raw),
            )
            .map(|link| link.canonical_slot);
            if let Some(target) = target {
                scopes.scopes[0].from = Link::Scope(1);
                let mut from = EventScope::of(EventObject::Country(target));
                if selects_state {
                    from.from = Link::Unknown;
                }
                scopes.scopes.push(from);
                unknown_seeds.insert(1);
            } else {
                scopes.scopes[0].from = Link::Unknown;
            }
        } else if let Some(state) = decision.identity.target_state {
            let present = state > 0
                && frame
                    .tables()
                    .rows::<tables::States>(0)
                    .is_some_and(|states| states.contains(u128::from(state)));
            if present {
                scopes.scopes[0].from = Link::Scope(1);
                let mut from = EventScope::of(EventObject::State(state));
                // An existing positive FROM state takes precedence over selector arrays and
                // triggers: a fresh nested state scope with default seed 1.
                if selects_state {
                    from.from = Link::Scope(2);
                }
                scopes.scopes.push(from);
                unknown_seeds.insert(1);
                if selects_state {
                    scopes
                        .scopes
                        .push(EventScope::of(EventObject::State(state)));
                }
            } else {
                scopes.scopes[0].from = Link::Unknown;
            }
        } else if decision.cause == PowerCause::TargetedDecision {
            scopes.scopes[0].from = Link::Unknown;
        }
        for index in unknown_seeds {
            scopes.scopes[index].random_seed_refusal =
                Some("decision caller random seed is not recorded");
        }
        return scripted
            .expand_in(&raw, &scopes)
            .and_then(|text| plain_text(&text))
            .map(Some);
    }
    if unknown_seed {
        use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes};
        let mut scope = EventScope::of(EventObject::Country(slot));
        scope.random_seed_refusal = Some("focus caller random seed is not recorded");
        let scopes = EventScopes::new(vec![scope]);
        return scripted
            .expand_in(&raw, &scopes)
            .and_then(|text| plain_text(&text))
            .map(Some);
    }
    Ok(content.catalogue.localise_in(key, &scripted))
}

/// The hour's trigger world: the scripted content's, else the content's own triggers on the
/// hour's tables.
fn event_world<'a>(
    content: &'a PoliticsContent,
    text: &'a TextFrame<'a>,
) -> Result<dashboard_text::triggers::World<'a>, String> {
    let world = match content
        .catalogue
        .scripted_content()
        .and_then(|scripted| scripted.recorded_world_in(text.tables()))
    {
        Ok(world) => world,
        Err(_) => {
            let power = content.power_content()?;
            dashboard_text::triggers::World::read_in(
                text.tables(),
                dashboard_text::triggers::ContentFacts::content_only(
                    &power.dlcs,
                    &power.triggers,
                    &power.aliases,
                ),
            )
        }
    };
    Ok(world.with_lexicon(text.lexicon()))
}

/// The token a row's item records: its fact, or its recorded key.
pub(super) fn recorded_token<'a>(
    facts: &'a EventFacts,
    identity: &'a PowerIdentity,
) -> Option<&'a str> {
    facts.token.as_deref().or_else(|| {
        (!identity.item_key.is_empty())
            .then(|| std::str::from_utf8(&identity.item_key).ok())
            .flatten()
    })
}

/// Text and sprite identity resolved while the exact event frame is present; images are drawn
/// later, for the selection a reply is served with. A late event outside those frames keeps its
/// recorded item and price but cannot borrow a later trigger state.
fn event_text<'a>(
    own: Option<&NamedCountry>,
    row: &Item<'_>,
    facts: &EventFacts,
    content: &'a PoliticsContent,
    frame: Option<&'a TextFrame<'a>>,
    texts: &FrameTexts<'a>,
) -> Option<ItemText> {
    let mut found = ItemText::default();
    if row.cause == PowerCause::DailyGain
        && row.identity.object_kind == PowerObjectKind::Organisation
    {
        let result = (|| -> Result<String, String> {
            let reference = row.identity.object_reference.ok_or_else(|| {
                "daily organisation cost: recorded reference is unavailable".to_owned()
            })?;
            let frame = *frame.ok_or_else(|| {
                format!("organisation {reference:#x}: its action-hour frame is unavailable")
            })?;
            let root = own.map_or(0, |own| own.naming.slot);
            if let Some(name) = texts.organisations.borrow().get(&(reference, root)) {
                return name.clone();
            }
            let name = (|| -> Result<String, String> {
                // The name never depends on the root unless its text is scripted, so one
                // context per hour serves every country's plain names.
                let any = texts
                    .scripted
                    .borrow()
                    .keys()
                    .next()
                    .copied()
                    .unwrap_or(root);
                let (template, scopes) = texts
                    .scripted(content, frame, any)?
                    .organisation_template(reference)?;
                let name = if template.contains('[') {
                    texts
                        .scripted(content, frame, root)?
                        .expand_in(&template, &scopes)?
                } else {
                    template
                };
                plain_text(&name)
            })();
            texts
                .organisations
                .borrow_mut()
                .insert((reference, root), name.clone());
            name
        })();
        match result {
            Ok(name) => found.name = Some(name),
            Err(reason) => found.unknown = Some(reason),
        }
        return Some(found);
    }
    let token = recorded_token(facts, row.identity)?;
    if matches!(
        row.cause,
        PowerCause::Decision | PowerCause::TargetedDecision | PowerCause::DecisionTimeout
    ) {
        let result = (|| -> Result<(), String> {
            let power = content.power_content()?;
            let decision = power
                .decisions
                .decision(token.as_bytes())?
                .ok_or_else(|| format!("decision {token} is not defined in recorded content"))?;
            let name = (|| -> Result<Option<String>, String> {
                let own = own.ok_or_else(|| {
                    format!("decision {token}: country identity is unavailable at action hour")
                })?;
                let look = own.naming.look.as_ref().ok_or_else(|| {
                    format!("decision {token}: CountryAppearance is unavailable at action hour")
                })?;
                let group = own.ruling_group.as_ref().map_err(Clone::clone)?;
                let key = decision.name_key(
                    &own.tag,
                    &look.cosmetic_tag,
                    group.as_deref().unwrap_or_default(),
                    |key| content.defines(key),
                )?;
                let selects_state = decision.name_selects_state(|body| {
                    let text = frame.ok_or_else(|| {
                        format!("decision {token} name selector: event-hour frame is unavailable")
                    })?;
                    texts
                        .world
                        .get_or_init(|| event_world(content, text))
                        .as_ref()
                        .map_err(Clone::clone)?
                        .has_children(body)
                })?;
                localise_at(
                    content,
                    &key,
                    frame,
                    texts,
                    Some(own.naming.slot),
                    Some((row, selects_state)),
                    false,
                )
            })();
            match name {
                Ok(name) => found.name = name,
                Err(reason) => found.unknown = Some(reason),
            };
            // The game shows the decision's own sprite when it is declared, else the first
            // declared sprite of the icon its dated conditions select.
            let own_sprite = decision.sprite_candidates(None)?.remove(0);
            let selected = decision.select_icon(|body| {
                if body.is_empty() {
                    return Ok(true);
                }
                let text = frame.ok_or_else(|| {
                    format!("decision {token} icon: event-hour frame is unavailable")
                })?;
                let own = own.ok_or_else(|| {
                    format!("decision {token} icon: country identity is unavailable at action hour")
                })?;
                let world = texts
                    .world
                    .get_or_init(|| event_world(content, text))
                    .as_ref()
                    .map_err(Clone::clone)?;
                let owner = dashboard_text::triggers::Scope::Country(own.naming.slot);
                let mut scopes = dashboard_text::triggers::Scopes::of(owner);
                scopes.from = Some(owner);
                scopes.prev = Some(owner);
                // Until a caller's recorded links establish FROM/PREV, a trigger reading one
                // refuses instead of guessing a target. Artwork FROM is the recorded target.
                if row.cause == PowerCause::TargetedDecision
                    || row.identity.target_country.is_some()
                    || row.identity.target_state.is_some()
                {
                    scopes.from = match (row.identity.target_country, row.identity.target_state) {
                        (Some(raw), _) => world
                            .slot(raw)
                            .map(dashboard_text::triggers::Scope::Country),
                        (None, Some(state)) => Some(dashboard_text::triggers::Scope::State(state)),
                        _ => None,
                    };
                    scopes.unknown.from = scopes.from.is_none();
                }
                world.evaluate(body, scopes).decided()
            });
            let mut candidates = vec![own_sprite];
            let otherwise = match selected {
                Ok(selected) => {
                    candidates.extend(decision.sprite_candidates(selected)?.into_iter().skip(1));
                    Ok(None)
                }
                Err(reason) => Err(format!("decision {token}: {reason}")),
            };
            found.sprites = Some(SpriteChoice {
                candidates,
                otherwise,
            });
            Ok(())
        })();
        if let Err(reason) = result {
            found.unknown = Some(format!("decision {token}: {reason}"));
        }
        return Some(found);
    }
    if row.cause == PowerCause::Focus {
        let result = (|| -> Result<(), String> {
            let power = content.power_content()?;
            let focus = power
                .national_focus(token)
                .map_err(|gap| format!("focus {token}: {gap}"))?
                .ok_or_else(|| format!("focus {token} is not defined in recorded content"))?;
            // The owner scope starts self-linked with a seed from caller inputs that are not
            // recorded; only a reached random read of this seed is unknown.
            match localise_at(
                content,
                &focus.localisation,
                frame,
                texts,
                own.map(|owner| owner.naming.slot),
                None,
                true,
            ) {
                Ok(name) => found.name = name,
                Err(reason) => found.unknown = Some(reason),
            };
            let art = (|| -> Result<SpriteChoice, String> {
                let own = own.ok_or_else(|| {
                    format!("focus {token} art: country identity unavailable at action hour")
                })?;
                // Country- then token-qualified overrides come before either icon selection.
                let candidates = focus
                    .sprite_overrides(token, Some(&own.tag))
                    .into_iter()
                    .collect();
                // The interface-window mode is not recorded.
                if focus.alternate_icon_present {
                    return Ok(SpriteChoice {
                        candidates,
                        otherwise: Err(format!(
                            "focus {token} art: alternate icon mode is not recorded"
                        )),
                    });
                }
                let otherwise = focus
                    .select_normal_icon(|body| {
                        if body.is_empty() {
                            return Ok(true);
                        }
                        let text = frame.ok_or_else(|| {
                            format!("focus {token} art: event-hour frame is unavailable")
                        })?;
                        let world = texts
                            .world
                            .get_or_init(|| event_world(content, text))
                            .as_ref()
                            .map_err(Clone::clone)?;
                        let owner = dashboard_text::triggers::Scope::Country(own.naming.slot);
                        let mut scopes = dashboard_text::triggers::Scopes::of(owner);
                        scopes.from = Some(owner);
                        scopes.prev = Some(owner);
                        world.evaluate(body, scopes).decided()
                    })
                    .map(|sprite| Some(sprite.to_owned()));
                Ok(SpriteChoice {
                    candidates,
                    otherwise,
                })
            })();
            match art {
                Ok(choice) => found.sprites = Some(choice),
                Err(reason) => {
                    found.unknown = Some(match found.unknown.take() {
                        Some(previous) => format!("{previous}; {reason}"),
                        None => reason,
                    })
                }
            }
            Ok(())
        })();
        if let Err(reason) = result {
            found.unknown = Some(reason);
        }
        return Some(found);
    }
    if matches!(
        row.cause,
        PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval
    ) {
        let own = own?;
        let slot = Some(own.naming.slot);
        if row.cause == PowerCause::Idea {
            match content.idea_name(token, own.naming.for_tag(&own.tag)) {
                Ok(dashboard_text::idea_names::IdeaName::Key(key)) => {
                    match localise_at(content, &key, frame, texts, slot, None, false) {
                        Ok(name) => found.name = name,
                        Err(reason) => found.unknown = Some(reason),
                    }
                }
                Ok(dashboard_text::idea_names::IdeaName::Generated(name)) => {
                    found.name = Some(name)
                }
                Err(reason) => found.unknown = Some(reason),
            }
        }
        found.description = content.idea_description_key(token).and_then(|key| {
            match localise_at(content, &key, frame, texts, slot, None, false) {
                Ok(description) => description,
                Err(reason) => {
                    rows::combine_reason(&mut found.unknown, Some(reason));
                    None
                }
            }
        });
        return Some(found);
    }
    None
}

/// The recorded definitions of the modifiers the tooltip of `token`'s idea formats, as an
/// advisor's or idea's payment shows it with its country `own` and character `facts` name.
pub(super) fn tooltip_modifiers(
    content: &PoliticsContent,
    facts: &EventFacts,
    own: Option<&NamedCountry>,
    identity: &PowerIdentity,
    recorded: Option<&BTreeMap<String, ModifierDefinitionRow>>,
) -> Option<Vec<(String, RecordedModifier)>> {
    let recorded = recorded?;
    let token = recorded_token(facts, identity)?;
    own?;
    let localise = |key: &str, view: View| content.localised(key, view);
    let joined = idea_history::Joined::of(content, &localise);
    let built = facts.character.as_ref().map(|recorded| &recorded.template);
    let (info, _) = idea_history::prepare_info(
        &joined,
        token,
        std::str::from_utf8(&identity.slot_key)
            .ok()
            .filter(|slot| !slot.is_empty()),
        built
            .map(|built| built.as_ref().map_err(String::as_str))
            .unwrap_or(Err(
                "the recorded holder/template is unavailable at the action hour",
            )),
        false,
    );
    let (keys, _) = idea_history::effect_keys([&info]);
    Some(
        keys.into_iter()
            .filter_map(|key| {
                let row = recorded.get(&key)?;
                Some((key, RecordedModifier::from(row)))
            })
            .collect(),
    )
}
