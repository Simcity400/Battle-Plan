//! Military leader careers from hourly reads; no combat outcome is inferred.
//!
//! Identity: CUnitLeader +0x08/+0x0c (type/id), +0xf4c (character),
//! +0x120 (raw owner); CountryLinks resolves that owner. CATEGORY +0xe7c:
//! GetLocalizedRoleTitle@140c0a7a0 maps 0/1/2 to fm_leader/cc_leader/navy_leader.
//! These are executable localisation keys, resolved in the recorded stack.
//! KIND is the recorder's vtable discriminator (engine/cunit_leader.rs:83).
//! CArmyLeader +0xf58/f68/f78/f88 are attack/defence/planning/logistics
//! (Serialize@140c18ca0); CNavyLeader +0xf68/f78/f88/f98 are
//! attack/defence/maneuvering/coordination (Serialize@140c18e90).
//! Skill is +0xe60 -> CUnitLeaderSkill +0x1b8; a null object is missing, not zero.
//! BuildSkillLevelTooltip@1416c07e0 and AddExperience@140c05400 stop at a missing
//! next skill definition; the star ceiling follows that same content lookup.
//!
//! Visibility uses the shared IsVisible@140c0e360 evaluator in the owner's
//! country scope. The roster calls IsVisible with its captured-role gate:
//! 1410da238/1410da327 set R8B=1 before calls 1410da243/1410da332.
//! +0x1059 captured excludes army categories <2 (vtable +0xe8). Unknown
//! visibility is withheld, with a named warning. No foreign-country/AI gate.
//! That caller also sets the selected character without clearing the country;
//! the shared CountryLeader scope preserves both native carriers.
//! Portraits ask the recorded holder for army/navy, large (portrait_entry's
//! native fallback); PoliticsContent decodes the same entries as Ruling party.
//! Trait names use their recorded tokens. Trait icons are GFX_trait_<token>,
//! the native construction in 141c61c40/141d011a0, resolved through the catalogue.
//! TraitExperience entries are learning progress, never assigned traits.
//!
//! Commands: OrdersGroups.leader reads COrdersGroup +0x88's leader pointer,
//! then the full handle at leader +0x08 (Serialize@140be8100).
//! is_army_group is +0x39; OrdersGroupPlan.name is the stored +0x160 name.
//! Fleets.leader is the full native CUnitLeader reference; Fleets.name and
//! OrdersGroups.name hold the hourly stored +E0/+160 strings. Older recordings
//! can supply command names from daily OrdersGroupPlans and naval commands from
//! NavalMissionTaskForces.leader, its fleet's CUnitLeader reference
//! (CFleet +0xb0, ResolveFleetLeader@140d4fce0; Serialize@140d4a390).
//! Join TaskForces.fleet, never the mission's distinct range_fleet. Coverage is
//! limited to the mission kinds this hourly table records (naval_mission_rows.rs).
//! Fleet names and idle commands are unavailable only in those older records.
//! LandCombatDetails.sides[].commander and
//! NavalCombatDetails.sides[].commander identify side command, once per full
//! combat handle (replay/combat_rows.rs:161 and :468).
//! Each leader's hours are observation spans, never inferred dates.
//! An already-active or post-gap combat has an unknown start. LandCombatResults
//! joins the full combat handle. NavalResultSources links retained NavalResults
//! through the exact creation transition (1415B29C0 -> 140CD6600), not a date,
//! province or sunk-ship heuristic. Transient unregistered combats never join.
//!
//! Requests name countries, one leader, or the recording's lightweight choice
//! list. Equal consecutive reads compress; missing hours, hidden leaders and
//! incomplete inputs break runs. A gain after missing input says first observed,
//! while holdings at branch start say held before. The counting JSON writer
//! refuses beyond the shared transport reply budget before sending.

use super::{
    LoadContext, Product, country_groups::HourSpan, identity::character_script_token,
    politics_content, validate_tag,
};
use crate::{
    Source,
    catalogue::{self, Catalogue, View},
    metadata::{LinkTags, link_slot, slot_read, validate_campaign},
    scripted_text::ScriptedText,
    triggers::Evaluator,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{
        CharacterPortraitRow, CharacterRow, Recorded, TableId, UnitLeaderRow, UnitLeaderTraitRow,
        tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use ts_rs::TS;

pub struct UnitLeaders;
pub mod portraits;
pub const MAX_REPLY_BYTES: usize = super::MAX_PRODUCT_REPLY_BYTES;
// TokenToPortraitType@1414d4b80: army token 0x289d -> 1, navy 0x289e -> 2.
const PORTRAIT_ARMY: u32 = 1;
const PORTRAIT_NAVY: u32 = 2;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub enum LeaderSelection {
    All,
    Countries { tags: Vec<String> },
    Leader { reference: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LeaderView {
    Leaderboard,
    Career,
    Choices,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LeaderKinds {
    #[default]
    All,
    Land,
    Navy,
}

impl LeaderKinds {
    fn includes(self, kind: u8) -> bool {
        match self {
            Self::All => matches!(
                kind,
                engine::cunit_leader::KIND_ARMY | engine::cunit_leader::KIND_NAVY
            ),
            Self::Land => kind == engine::cunit_leader::KIND_ARMY,
            Self::Navy => kind == engine::cunit_leader::KIND_NAVY,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitLeadersParams {
    pub branch_id: String,
    pub selection: LeaderSelection,
    pub view: LeaderView,
    #[serde(default)]
    pub leaders: LeaderKinds,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderPerson {
    pub character_id: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    /// Stable recorded read used by the lazy artwork request.
    pub portrait_hour: i32,
    /// The leader present at this identity read (a character can back more than one).
    pub portrait_reference: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderRole {
    pub owner: Option<String>,
    pub category: i32,
    pub rank: Option<String>,
    pub kind: u8,
    pub maximum_skill: Option<i32>,
    /// Native skill labels localised through the recorded content stack.
    pub attribute_names: [Option<String>; 4],
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderValues {
    pub person: u32,
    pub role: u32,
    pub skill: Option<i32>,
    pub components: [i32; 4],
    #[serde(default, with = "trait_indices")]
    #[ts(as = "Option<Vec<u32>>")]
    pub traits: Option<Arc<[u32]>>,
}

mod trait_indices {
    use super::*;

    pub fn serialize<S: serde::Serializer>(
        traits: &Option<Arc<[u32]>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        traits.as_deref().serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Arc<[u32]>>, D::Error> {
        Option::<Vec<u32>>::deserialize(deserializer).map(|traits| traits.map(Arc::from))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, TS)]
#[ts(
    type = "[number, number, number, number, number | null, [number, number, number, number], number[] | null]"
)]
pub struct UnitLeaderRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: LeaderValues,
}

type WireLeaderRun = (i32, i32, u32, u32, Option<i32>, [i32; 4], Option<Vec<u32>>);
impl Serialize for UnitLeaderRun {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let v = &self.value;
        (
            self.first_hour,
            self.last_hour,
            v.person,
            v.role,
            v.skill,
            v.components,
            v.traits.as_deref(),
        )
            .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for UnitLeaderRun {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        WireLeaderRun::deserialize(deserializer).map(Self::from)
    }
}
impl From<WireLeaderRun> for UnitLeaderRun {
    fn from(
        (first_hour, last_hour, person, role, skill, components, traits): WireLeaderRun,
    ) -> Self {
        Self {
            first_hour,
            last_hour,
            value: LeaderValues {
                person,
                role,
                skill,
                components,
                traits: traits.map(Arc::from),
            },
        }
    }
}
impl From<UnitLeaderRun> for WireLeaderRun {
    fn from(run: UnitLeaderRun) -> Self {
        let v = run.value;
        (
            run.first_hour,
            run.last_hour,
            v.person,
            v.role,
            v.skill,
            v.components,
            v.traits.as_deref().map(<[_]>::to_vec),
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LeaderChangeDate {
    HeldBefore,
    Gained,
    FirstObserved,
}

#[derive(Clone, Debug, PartialEq, Eq, TS)]
#[ts(type = "[number, number, number]")]
pub struct LeaderTraitGain {
    pub hour: i32,
    pub token: u32,
    pub date: LeaderChangeDate,
}
impl Serialize for LeaderTraitGain {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let date = match self.date {
            LeaderChangeDate::HeldBefore => 0_u8,
            LeaderChangeDate::Gained => 1,
            LeaderChangeDate::FirstObserved => 2,
        };
        (self.hour, self.token, date).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for LeaderTraitGain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (hour, token, date) = <(i32, u32, u8)>::deserialize(deserializer)?;
        let date = match date {
            0 => LeaderChangeDate::HeldBefore,
            1 => LeaderChangeDate::Gained,
            2 => LeaderChangeDate::FirstObserved,
            _ => return Err(serde::de::Error::custom("Invalid trait change date")),
        };
        Ok(Self { hour, token, date })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LeaderFormationKind {
    Army,
    ArmyGroup,
    Fleet,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderCommand {
    pub first_hour: i32,
    pub last_hour: i32,
    pub formation: String,
    pub kind: LeaderFormationKind,
    pub name: Option<String>,
    pub name_read_hour: Option<i32>,
    pub name_unknown: Vec<HourSpan>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderCareer {
    pub reference: String,
    pub runs: Vec<UnitLeaderRun>,
    pub trait_gains: Vec<LeaderTraitGain>,
    pub commands: Vec<LeaderCommand>,
    /// First participation hours: first absolute, then deltas. Deduplicated by combat.
    #[ts(type = "number[]")]
    pub battle_starts: Vec<i64>,
    /// First supplying read for each participation, independent of its native date.
    pub battle_start_read_hours: Vec<i32>,
    /// Indices whose combat was not known absent in the preceding hour. This is
    /// the combat's start uncertainty, not the leader's participation date.
    pub unknown_battle_starts: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub battle_won: Option<Vec<Option<bool>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub battle_result_hours: Option<Vec<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub battle_result_read_hours: Option<Vec<Option<i32>>>,
    /// First hour with incomplete battle observations; recorded totals remain visible.
    pub partial_from: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderTraitInfo {
    pub token: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderObservation {
    pub first_hour: i32,
    pub last_hour: i32,
    pub leaders: bool,
    pub land_battles: bool,
    pub naval_battles: bool,
    pub land_commands: bool,
    pub naval_commands: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnitLeadersReply {
    pub branch_id: String,
    pub first_hour: i32,
    pub leaders: Vec<LeaderCareer>,
    pub people: Vec<LeaderPerson>,
    /// Owner/rank facts shared across runs and leaders; a transfer selects another role.
    pub roles: Vec<LeaderRole>,

    pub traits: Vec<LeaderTraitInfo>,
    pub observed: Vec<LeaderObservation>,
    pub results_missing: String,
    pub navy_commands_missing: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub selection_tags: Option<Vec<String>>,
    pub warnings: Vec<String>,
}

/// Per-response diagnostics can change without copying the cached history.
#[derive(Debug, TS)]
#[ts(type = "UnitLeadersReply")]
pub struct UnitLeadersResponse {
    history: Arc<UnitLeadersReply>,
    pub warnings: Vec<String>,
}

impl From<Arc<UnitLeadersReply>> for UnitLeadersResponse {
    fn from(history: Arc<UnitLeadersReply>) -> Self {
        Self {
            warnings: history.warnings.clone(),
            history,
        }
    }
}

impl Serialize for UnitLeadersResponse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let history = &self.history;
        let mut reply = serializer.serialize_struct(
            "UnitLeadersReply",
            10 + usize::from(history.selection_tags.is_some()),
        )?;
        reply.serialize_field("branchId", &history.branch_id)?;
        reply.serialize_field("firstHour", &history.first_hour)?;
        reply.serialize_field("leaders", &history.leaders)?;
        reply.serialize_field("people", &history.people)?;
        reply.serialize_field("roles", &history.roles)?;
        reply.serialize_field("traits", &history.traits)?;
        reply.serialize_field("observed", &history.observed)?;
        reply.serialize_field("resultsMissing", &history.results_missing)?;
        reply.serialize_field("navyCommandsMissing", &history.navy_commands_missing)?;
        if let Some(tags) = &history.selection_tags {
            reply.serialize_field("selectionTags", tags)?;
        }
        reply.serialize_field("warnings", &self.warnings)?;
        reply.end()
    }
}

const RESULTS_MISSING: &str = "Outcomes require a recorded native result linked to the observed combat. Unfinished combats and missing result coverage remain unknown.";
const NAVY_COMMANDS_MISSING: &str = "Fleet commands cover only hours observed through recorded mission task forces. Fleet names and commands outside those observations are not recorded.";

fn handle(row: &UnitLeaderRow) -> u64 {
    (u64::from(row.id) << 32) | u64::from(row.reference_type)
}

/// Executable keys, not a vanilla content list: GetLocalizedRoleTitle@140c0a7a0.
fn rank_key(category: i32) -> Option<&'static str> {
    match category {
        0 => Some("fm_leader"),
        1 => Some("cc_leader"),
        2 => Some("navy_leader"),
        _ => None,
    }
}

fn maximum_skill(
    skills: &installed_content::unit_leader::Skills,
    level: i32,
    category: i32,
) -> Option<i32> {
    if !skills.conditions().is_empty() || level < 0 {
        return None;
    }
    let mut ceiling = level;
    // Sparse tables stop as the native XP loop does.
    loop {
        match skills.next_required_experience(ceiling, category)? {
            None => return Some(ceiling),
            Some(_) => ceiling = ceiling.checked_add(1)?,
        }
    }
}

fn span_push(spans: &mut Vec<HourSpan>, hour: i32) {
    if let Some(last) = spans.last_mut()
        && last.last_hour.checked_add(1) == Some(hour)
    {
        last.last_hour = hour;
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

fn observe(
    out: &mut Vec<LeaderObservation>,
    hour: i32,
    leaders: bool,
    land: bool,
    navy: bool,
    land_commands: bool,
    naval_commands: bool,
) {
    if let Some(last) = out.last_mut()
        && last.last_hour.checked_add(1) == Some(hour)
        && (last.leaders, last.land_battles, last.naval_battles) == (leaders, land, navy)
        && (last.land_commands, last.naval_commands) == (land_commands, naval_commands)
    {
        last.last_hour = hour;
    } else {
        out.push(LeaderObservation {
            first_hour: hour,
            last_hour: hour,
            leaders,
            land_battles: land,
            naval_battles: navy,
            land_commands,
            naval_commands,
        });
    }
}

fn change_date(previous: Option<&UnitLeaderRun>, hour: i32, start: i32) -> LeaderChangeDate {
    let consecutive = previous.is_some_and(|run| run.last_hour.checked_add(1) == Some(hour));
    if hour == start {
        LeaderChangeDate::HeldBefore
    } else if consecutive && previous.is_some_and(|run| run.value.traits.is_some()) {
        LeaderChangeDate::Gained
    } else {
        LeaderChangeDate::FirstObserved
    }
}

fn record_trait_gains(
    gains: &mut Vec<LeaderTraitGain>,
    tokens: Option<&[u32]>,
    last_known: Option<&[u32]>,
    hour: i32,
    date: LeaderChangeDate,
) {
    if let Some(tokens) = tokens {
        for token in tokens {
            if !last_known.is_some_and(|held| held.contains(token)) {
                gains.push(LeaderTraitGain {
                    hour,
                    token: *token,
                    date,
                });
            }
        }
    }
}

fn append_command(
    state: &mut LeaderState,
    formation: u64,
    kind: LeaderFormationKind,
    name: Option<String>,
    hour: i32,
) {
    let leader = &mut state.career;
    if let Some(last) = state
        .command_indices
        .get(&formation)
        .and_then(|&index| leader.commands.get_mut(index))
        && last.last_hour.checked_add(1) == Some(hour)
        && (name.is_none() || last.name.is_none() || last.name == name)
        && last.kind == kind
    {
        last.last_hour = hour;
        if name.is_none() {
            span_push(&mut last.name_unknown, hour);
        }
        if name.is_some() && last.name.is_none() {
            last.name = name;
            last.name_read_hour = Some(hour);
        }
    } else {
        state
            .command_indices
            .insert(formation, leader.commands.len());
        leader.commands.push(LeaderCommand {
            first_hour: hour,
            last_hour: hour,
            formation: formation.to_string(),
            kind,
            name_read_hour: name.as_ref().map(|_| hour),
            name_unknown: if name.is_none() {
                vec![HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                }]
            } else {
                Vec::new()
            },
            name,
        });
    }
}

fn visible(row: &UnitLeaderRow, scripted: Option<&ScriptedText<'_>>) -> Result<bool, String> {
    // IsVisible@140c0e360's optional role gate; Capture@140c02810 sets +0x1059.
    if row.kind == engine::cunit_leader::KIND_ARMY && (row.category as u32) < 2 && row.captured != 0
    {
        return Ok(false);
    }
    if row.visibility_choice == 0 && row.template_visibility == 0 {
        return Ok(true);
    }
    let world = scripted
        .ok_or("Content is unavailable for leader visibility")?
        .world()?;
    Evaluator::new(world, None)
        .leader_roster_visible(row)
        .decided()
}

fn reply_bytes(reply: &UnitLeadersReply) -> io::Result<usize> {
    crate::transport::json_len(reply, MAX_REPLY_BYTES).ok_or_else(|| {
        io::Error::other(
            "Leader reply exceeds its transport budget; select fewer countries or one career",
        )
    })
}

struct Content {
    catalogue: Arc<Catalogue>,
    politics: Arc<politics_content::PoliticsContent>,
    leaders: installed_content::unit_leader::Leaders,
}

impl Content {
    fn load(
        catalogue: Arc<Catalogue>,
        politics: Arc<politics_content::PoliticsContent>,
    ) -> io::Result<Self> {
        let leaders = installed_content::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)?;
        Ok(Self {
            catalogue,
            politics,
            leaders,
        })
    }
}

pub(crate) fn check_content(
    catalogue: Arc<Catalogue>,
    politics: Arc<politics_content::PoliticsContent>,
) -> io::Result<Vec<crate::content_check::Finding>> {
    use crate::content_check::{Finding, Kind};
    let content = Content::load(catalogue, politics)?;
    let mut findings: Vec<_> = content
        .leaders
        .misreads
        .iter()
        .map(|message| Finding::new(Kind::Misread, "unit leaders", message))
        .collect();
    findings.extend(
        content
            .leaders
            .traits
            .unsupported
            .iter()
            .map(|reason| Finding::new(Kind::Refused, "leader trait inventory", reason)),
    );
    findings.extend(
        content
            .leaders
            .skills
            .warnings()
            .iter()
            .map(|message| Finding::new(Kind::Refused, "leader skill definitions", message)),
    );
    findings.extend(content.leaders.skills.conditions().iter().map(|condition| {
        Finding::new(
            Kind::Offline,
            "leader skill definitions",
            format!("Maximum skill needs recorded modifier definitions: {condition}"),
        )
    }));
    for category in [0, 1, 2] {
        if let Some(key) = rank_key(category) {
            content.catalogue.localised(key, View::Plain);
        }
    }
    for kind in [
        engine::cunit_leader::KIND_ARMY,
        engine::cunit_leader::KIND_NAVY,
    ] {
        attribute_names(kind, Some(&content.catalogue));
    }
    for effects in content.leaders.traits.iter() {
        let info = trait_info(&effects.name, Some(&content.catalogue));
        if let Some(reason) = info.missing {
            findings.push(Finding::new(Kind::Refused, &effects.name, reason));
        }
        findings.extend(
            effects
                .unsupported
                .iter()
                .map(|reason| Finding::new(Kind::Refused, &effects.name, reason)),
        );
        findings.extend(effects.conditions.iter().map(|condition| {
            Finding::new(
                Kind::Offline,
                &effects.name,
                format!("Trait conditions need recorded modifier definitions: {condition}"),
            )
        }));
    }
    Ok(findings)
}

struct PersonRead {
    epoch: u64,
    character: Option<CharacterRow>,
    holder: Option<CharacterPortraitRow>,
    person: u32,
}

fn person(
    frame: &DecodedFrame,
    row: &UnitLeaderRow,
    character: Option<&CharacterRow>,
    token: Option<&str>,
    content: Option<&Content>,
    reply: &mut UnitLeadersReply,
) -> LeaderPerson {
    let token = token.map(str::to_owned);
    let name = character.and_then(|c| match std::str::from_utf8(&c.token) {
        Ok(saved) => content.and_then(|c| c.politics.character_name(token.as_deref(), saved)),
        Err(_) => {
            reply
                .warnings
                .push(format!("Character {} name is not UTF-8", row.character_id));
            None
        }
    });
    LeaderPerson {
        character_id: row.character_id,
        token,
        name,
        portrait_hour: frame.hour,
        portrait_reference: handle(row).to_string(),
    }
}

struct LastRead {
    row: UnitLeaderRow,
    traits: Option<Vec<UnitLeaderTraitRow>>,
    role: u32,
    person: u32,
    person_epoch: u64,
    owner_slot: Option<u32>,
    owner_epoch: u64,
    trait_epoch: u64,
}

struct LeaderState {
    reference: u64,
    career: LeaderCareer,
    command_indices: HashMap<u64, usize>,
    last_read: Option<LastRead>,
    visible_hour: Option<i32>,
    admitted: bool,
    last_run: Option<usize>,
    last_known_run: Option<usize>,
}

impl LeaderState {
    fn new(reference: u64) -> Self {
        Self {
            reference,
            career: LeaderCareer {
                reference: reference.to_string(),
                runs: Vec::new(),
                trait_gains: Vec::new(),
                commands: Vec::new(),
                battle_starts: Vec::new(),
                battle_start_read_hours: Vec::new(),
                unknown_battle_starts: Vec::new(),
                battle_won: None,
                battle_result_hours: None,
                battle_result_read_hours: None,
                partial_from: None,
            },
            command_indices: HashMap::new(),
            last_read: None,
            visible_hour: None,
            admitted: false,
            last_run: None,
            last_known_run: None,
        }
    }
}

// Hourly writes share one allocation and adjacent cache lines. Materialize only
// admitted careers after the scan, retaining the complete transfer history.
struct StoredRun {
    run: UnitLeaderRun,
    previous: Option<usize>,
}

fn append_stored_run(
    state: &mut LeaderState,
    runs: &mut Vec<StoredRun>,
    value: LeaderValues,
    hour: i32,
    start: i32,
    choices: bool,
) {
    let previous = state.last_run.map(|i| &runs[i].run);
    let last_known = state
        .last_known_run
        .and_then(|i| runs[i].run.value.traits.as_deref());
    record_trait_gains(
        &mut state.career.trait_gains,
        value.traits.as_deref(),
        last_known,
        hour,
        change_date(previous, hour, start),
    );
    if let Some(i) = state.last_run {
        let last = &mut runs[i].run;
        if choices {
            *last = UnitLeaderRun {
                first_hour: hour,
                last_hour: hour,
                value,
            };
            return;
        }
        if last.last_hour.checked_add(1) == Some(hour) && last.value == value {
            last.last_hour = hour;
            return;
        }
    }
    let known = value.traits.is_some();
    let index = runs.len();
    runs.push(StoredRun {
        run: UnitLeaderRun {
            first_hour: hour,
            last_hour: hour,
            value,
        },
        previous: state.last_run,
    });
    state.last_run = Some(index);
    if known {
        state.last_known_run = Some(index);
    }
}

fn trait_slice<'a>(
    rows: &'a [UnitLeaderTraitRow],
    reference: u64,
    cursor: &mut usize,
) -> &'a [UnitLeaderTraitRow] {
    while *cursor < rows.len() && rows[*cursor].leader_reference < reference {
        *cursor += 1;
    }
    let first = *cursor;
    while *cursor < rows.len() && rows[*cursor].leader_reference == reference {
        *cursor += 1;
    }
    &rows[first..*cursor]
}

fn scan(
    replay: &Replay,
    params: &UnitLeadersParams,
    content: Option<&Content>,
    cancelled: &AtomicBool,
) -> io::Result<UnitLeadersReply> {
    let branch = params.branch_id.parse::<u64>().expect("validated branch");
    let start = replay
        .index
        .iter()
        .filter(|b| b.branch == branch)
        .map(|b| b.first_hour)
        .min()
        .ok_or_else(|| crate::invalid("Branch has no recorded frames"))?;
    let career = params.view == LeaderView::Career;
    let choices = params.view == LeaderView::Choices;
    let requested: Option<BTreeSet<&str>> = match &params.selection {
        LeaderSelection::Countries { tags } => Some(tags.iter().map(String::as_str).collect()),
        _ => None,
    };
    let mut selected = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::UnitLeaders,
        TableId::Characters,
        TableId::LexicalTokens,
        TableId::CharacterPortraits,
        TableId::UnitLeaderTraits,
        TableId::LandCombatDetails,
        TableId::NavalCombatDetails,
        TableId::States,
        TableId::Provinces,
        TableId::ScriptFlags,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::CountryWars,
        TableId::CharacterRoleTraits,
        TableId::CharacterOperatives,
        TableId::CountryRetiredCharacters,
        TableId::LandCombatResults,
        TableId::NavalResultSources,
        TableId::NavalResults,
        TableId::NavalCombatResultDetails,
    ];
    if career {
        selected.extend([
            TableId::OrdersGroups,
            TableId::OrdersGroupPlans,
            TableId::NavalMissionTaskForces,
            TableId::TaskForces,
            TableId::Fleets,
        ]);
    }
    if choices {
        selected.retain(|table| {
            !matches!(
                table,
                TableId::CharacterPortraits
                    | TableId::UnitLeaderTraits
                    | TableId::LandCombatDetails
                    | TableId::NavalCombatDetails
            )
        });
    }
    let mut reply = UnitLeadersReply {
        branch_id: params.branch_id.clone(),
        first_hour: start,
        leaders: Vec::new(),
        people: Vec::new(),
        roles: Vec::new(),
        traits: Vec::new(),
        observed: Vec::new(),
        selection_tags: None,
        results_missing: RESULTS_MISSING.into(),
        navy_commands_missing: if replay.format.field_gaps(TableId::Fleets).next().is_none() {
            String::new()
        } else {
            NAVY_COMMANDS_MISSING.into()
        },
        warnings: Vec::new(),
    };
    let mut leader_indices = HashMap::<u64, usize>::new();
    let mut leaders = Vec::<LeaderState>::new();
    let mut stored_runs = Vec::<StoredRun>::new();
    let mut row_indices = Vec::<Option<(u64, usize)>>::new();
    let mut people = HashMap::<String, u32>::new();
    let mut last_people = HashMap::<(u32, u8), PersonRead>::new();
    let mut identity_epoch = 0_u64;
    let mut identity_tables: Option<[replay::frame::SharedView; 3]> = None;
    let mut owner_tags = LinkTags::default();
    let mut owner_epoch = 0_u64;
    let mut owner_tables: Option<[replay::frame::SharedView; 2]> = None;
    let mut trait_table: Option<(replay::frame::SharedView, bool)> = None;
    let mut trait_epoch = 0_u64;
    let mut roles = HashMap::<(Option<u32>, i32, u8, Option<i32>), u32>::new();
    let mut rank_names = HashMap::<i32, Option<String>>::new();
    let mut skill_ceilings = HashMap::<(i32, i32), Option<i32>>::new();
    let mut refusals = catalogue::Refusals::default();
    let mut trait_tokens = HashMap::<String, u32>::new();
    let mut trait_sets = HashMap::<Vec<u32>, Arc<[u32]>>::new();
    let mut token_list = Vec::new();
    let mut battle_indices = HashMap::<(u64, u8, u64), usize>::new();
    let mut battle_hours = HashMap::<u64, Vec<(i32, i32, bool, u8, u64, u8)>>::new();
    let mut outcomes = BattleOutcomes::new();
    let mut naval_sources = BTreeMap::<u64, replay::rows::NavalResultSourceRow>::new();
    let mut results_recorded = false;
    let mut missing_battles: [Option<i32>; 2] = [None, None];
    let mut previous_combats: [Option<(i32, BTreeSet<u64>)>; 2] = [None, None];
    let only = match &params.selection {
        LeaderSelection::Leader { reference } => {
            Some(reference.parse::<u64>().expect("validated leader"))
        }
        _ => None,
    };
    for block in replay.index.iter().filter(|b| b.branch == branch) {
        let frames = match replay.read_block_selected(block, &selected) {
            Ok(frames) => frames,
            Err(e) => {
                refusals.note(e.to_string(), format!("block {}", block.first_hour));
                continue;
            }
        };
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Leader request cancelled",
                ));
            }
            let decoded = match decoder.advance(&frame) {
                Ok(f) => f,
                Err(e) => {
                    refusals.note(e.to_string(), frame.hour.to_string());
                    break;
                }
            };
            validate_campaign(decoded, replay)?;
            let hour = decoded.hour;
            if slot_read(&decoded.metrics, TableId::LandCombatResults, hour)
                && let Ok(results) = decoded.complete::<tables::LandCombatResults>()
            {
                results_recorded = true;
                for result in results {
                    if record_outcome(
                        &mut outcomes,
                        (1, result.combat),
                        (result.end_hour, hour, result.winners),
                    ) {
                        refusals.note(
                            "Conflicting native land battle outcomes",
                            result.combat.to_string(),
                        );
                    }
                }
            }
            if slot_read(&decoded.metrics, TableId::NavalResultSources, hour)
                && let Ok(sources) = decoded.complete::<tables::NavalResultSources>()
            {
                results_recorded = true;
                for source in sources {
                    if !source.transient && !source.merged {
                        naval_sources.insert(source.result, source.clone());
                    }
                }
            }
            if let Ok(results) = decoded.complete::<tables::NavalResults>() {
                for result in results {
                    if let Some(source) = naval_sources.get(&result.id) {
                        // 140CD6600 copies the combat's first and second sides into
                        // the result's +18/+88; +F8 selects those exact embedded sides.
                        let winners = match result.winning_side {
                            1 => 1,
                            2 => 2,
                            _ => 0,
                        };
                        if result.winning_side > 2 {
                            refusals.note(
                                "Unsupported native naval winner selector",
                                result.id.to_string(),
                            );
                        }
                        if record_outcome(
                            &mut outcomes,
                            (2, source.combat),
                            (source.game_hour, hour, winners),
                        ) {
                            refusals.note(
                                "Conflicting native naval battle outcomes",
                                source.combat.to_string(),
                            );
                        }
                    }
                }
            }
            // Failed table reads allocate errors: resolve each once, never per leader.
            let characters = decoded.complete::<tables::Characters>().ok();
            let holders = if choices {
                None
            } else {
                decoded.complete::<tables::CharacterPortraits>().ok()
            };
            let lexical = decoded.complete::<tables::LexicalTokens>().ok();
            let now_identity = [
                TableId::Characters,
                TableId::CharacterPortraits,
                TableId::LexicalTokens,
            ]
            .map(|t| decoded.tables[t.index()].clone());
            if identity_tables.as_ref().is_none_or(|last| {
                last.iter()
                    .zip(&now_identity)
                    .any(|(a, b)| !replay::frame::SharedView::ptr_eq(a, b) && a != b)
            }) {
                identity_epoch += 1;
                identity_tables = Some(now_identity);
            }
            let land = decoded
                .complete::<tables::LandCombatDetails>()
                .ok()
                .filter(|_| slot_read(&decoded.metrics, TableId::LandCombatDetails, hour));
            let navy = decoded
                .complete::<tables::NavalCombatDetails>()
                .ok()
                .filter(|_| slot_read(&decoded.metrics, TableId::NavalCombatDetails, hour));
            for (i, known) in [land.is_some(), navy.is_some()].into_iter().enumerate() {
                if !known {
                    missing_battles[i] = Some(hour);
                }
            }
            let land_commands = career
                && slot_read(&decoded.metrics, TableId::OrdersGroups, hour)
                && decoded.complete::<tables::OrdersGroups>().is_ok();
            let fleet_rows = career
                .then(|| decoded.complete::<tables::Fleets>().ok())
                .flatten()
                .filter(|_| slot_read(&decoded.metrics, TableId::Fleets, hour));
            let fleet_commands = fleet_rows.filter(|rows| {
                rows.iter()
                    .all(|row| !matches!(row.leader, Recorded::NotRecorded))
            });
            let mut naval_commands = fleet_commands.is_some()
                || career
                    && slot_read(&decoded.metrics, TableId::NavalMissionTaskForces, hour)
                    && slot_read(&decoded.metrics, TableId::TaskForces, hour)
                    && decoded.complete::<tables::NavalMissionTaskForces>().is_ok()
                    && decoded.complete::<tables::TaskForces>().is_ok();
            if !slot_read(&decoded.metrics, TableId::UnitLeaders, hour) {
                observe(
                    &mut reply.observed,
                    hour,
                    false,
                    land.is_some(),
                    navy.is_some(),
                    land_commands,
                    naval_commands,
                );
                continue;
            }
            let (Ok(rows), Ok(_), Ok(links)) = (
                decoded.complete::<tables::UnitLeaders>(),
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            ) else {
                observe(
                    &mut reply.observed,
                    hour,
                    false,
                    land.is_some(),
                    navy.is_some(),
                    land_commands,
                    naval_commands,
                );
                continue;
            };
            if stored_runs.capacity() == 0 && !rows.is_empty() {
                let count = if only.is_some() { 1 } else { rows.len() };
                stored_runs.reserve(count);
            }
            let now_owners = [TableId::Countries, TableId::CountryLinks]
                .map(|t| decoded.tables[t.index()].clone());
            if owner_tables.as_ref().is_none_or(|last| {
                last.iter()
                    .zip(&now_owners)
                    .any(|(a, b)| !replay::frame::SharedView::ptr_eq(a, b) && a != b)
            }) {
                owner_tags.update(decoded)?;
                owner_epoch += 1;
                owner_tables = Some(now_owners);
            }
            let needs_visibility = rows
                .iter()
                .any(|row| row.visibility_choice != 0 || row.template_visibility != 0);
            let scripted = content.filter(|_| needs_visibility).and_then(|c| {
                match c.catalogue.scripted_content() {
                    Ok(s) => Some(ScriptedText::new(decoded, &c.catalogue.localisation, s, 0)),
                    Err(reason) => {
                        refusals.note(reason, "leader visibility content");
                        None
                    }
                }
            });
            let traits = decoded
                .complete::<tables::UnitLeaderTraits>()
                .ok()
                .filter(|_| slot_read(&decoded.metrics, TableId::UnitLeaderTraits, hour));
            let mut trait_cursor = 0;
            let mut complete = true;
            let now_traits = &decoded.tables[TableId::UnitLeaderTraits.index()];
            if trait_table.as_ref().is_none_or(|(last, known)| {
                *known != traits.is_some()
                    || !replay::frame::SharedView::ptr_eq(last, now_traits) && last != now_traits
            }) {
                trait_epoch += 1;
                trait_table = Some((now_traits.clone(), traits.is_some()));
            }
            for (row_index, row) in rows.iter().enumerate() {
                if !params.leaders.includes(row.kind) {
                    continue;
                }
                let reference = handle(row);
                if only.is_some_and(|wanted| wanted != reference) {
                    continue;
                }
                // Most rosters keep their recorded order. Avoid a hash lookup
                // for every leader while still accepting insertions/transfers.
                let index = if let Some((_, index)) = row_indices
                    .get(row_index)
                    .and_then(Option::as_ref)
                    .filter(|(last, _)| *last == reference)
                {
                    *index
                } else {
                    let index = *leader_indices.entry(reference).or_insert_with(|| {
                        leaders.push(LeaderState::new(reference));
                        leaders.len() - 1
                    });
                    if row_index >= row_indices.len() {
                        row_indices.resize(row_index + 1, None);
                    }
                    row_indices[row_index] = Some((reference, index));
                    index
                };
                let state = &mut leaders[index];
                let owner_unchanged = state.last_read.as_ref().is_some_and(|last| {
                    last.owner_epoch == owner_epoch && last.row.owner == row.owner
                });
                let owner_slot = state
                    .last_read
                    .as_ref()
                    .filter(|last| last.owner_epoch == owner_epoch && last.row.owner == row.owner)
                    .map(|last| last.owner_slot)
                    .unwrap_or_else(|| link_slot(links, row.owner));
                let owner_known = if owner_unchanged {
                    reply.roles[state.last_read.as_ref().expect("cached owner").role as usize]
                        .owner
                        .is_some()
                } else {
                    owner_tags.get(row.owner).is_some()
                };
                if !owner_known {
                    complete = false;
                    refusals.note(
                        "Leader owner has no resolved recorded country tag",
                        reference.to_string(),
                    );
                }
                if let Some(tags) = &requested
                    && !state.admitted
                    && !owner_unchanged
                    && owner_tags
                        .get(row.owner)
                        .is_some_and(|tag| tags.contains(tag.as_str()))
                {
                    state.admitted = true;
                }
                match visible(row, scripted.as_ref()) {
                    Ok(true) => {}
                    Ok(false) => {
                        continue;
                    }
                    Err(reason) => {
                        complete = false;
                        refusals.note(reason, reference.to_string());
                        continue;
                    }
                }
                state.visible_hour = Some(hour);
                let person_id = if let Some(last) = state.last_read.as_ref().filter(|last| {
                    last.person_epoch == identity_epoch
                        && last.row.character_id == row.character_id
                        && last.row.kind == row.kind
                }) {
                    last.person
                } else if let Some(cached) = last_people
                    .get(&(row.character_id, row.kind))
                    .filter(|p| p.epoch == identity_epoch)
                {
                    cached.person
                } else {
                    let character = characters.and_then(|cs| {
                        cs.binary_search_by_key(&row.character_id, |c| c.id)
                            .ok()
                            .map(|i| &cs[i])
                    });
                    let holder = holders.and_then(|ps| {
                        ps.binary_search_by_key(&row.character_id, |p| p.character)
                            .ok()
                            .map(|i| &ps[i])
                    });
                    let token = character
                        .zip(lexical)
                        .and_then(|(c, lex)| character_script_token(c, lex));
                    // Stable character/holder reads reuse identity without formatting or decoding artwork each hour.
                    let cached = last_people
                        .get(&(row.character_id, row.kind))
                        .filter(|cached| {
                            cached.character.as_ref() == character
                                && cached.holder.as_ref() == holder
                                && reply.people[cached.person as usize].token.as_deref() == token
                        })
                        .map(|cached| cached.person);
                    if let Some(index) = cached {
                        last_people
                            .get_mut(&(row.character_id, row.kind))
                            .expect("cached person")
                            .epoch = identity_epoch;
                        index
                    } else {
                        let identity = format!(
                            "{}:{}:{:?}:{:?}:{:?}",
                            row.character_id,
                            row.kind,
                            token,
                            character.map(|c| &c.token),
                            holder
                        );
                        let index = if let Some(&index) = people.get(&identity) {
                            index
                        } else {
                            let person =
                                person(decoded, row, character, token, content, &mut reply);
                            let index = reply.people.len() as u32;
                            reply.people.push(person);
                            people.insert(identity, index);
                            index
                        };
                        last_people.insert(
                            (row.character_id, row.kind),
                            PersonRead {
                                epoch: identity_epoch,
                                character: character.cloned(),
                                holder: holder.cloned(),
                                person: index,
                            },
                        );
                        index
                    }
                };
                let owner = if owner_unchanged {
                    reply.roles[state.last_read.as_ref().expect("cached owner").role as usize]
                        .owner
                        .as_deref()
                } else {
                    owner_tags.get(row.owner).map(String::as_str)
                };
                let raw_traits = (!choices)
                    .then_some(traits)
                    .flatten()
                    .map(|ts| trait_slice(ts, reference, &mut trait_cursor));
                let leader = &mut state.career;
                let previous_run = state.last_run.map(|i| &stored_runs[i].run);
                let previous_hour = previous_run.map(|r| r.last_hour);
                let gap = previous_hour.is_some_and(|h| h.checked_add(1) != Some(hour));
                let battle_gap = previous_hour.is_some_and(|h| {
                    missing_battles[usize::from(row.kind - 1)].is_some_and(|m| m >= h)
                });
                let current_battles_missing = if row.kind == engine::cunit_leader::KIND_ARMY {
                    land.is_none()
                } else {
                    navy.is_none()
                };
                if leader.partial_from.is_none() && (gap || battle_gap || current_battles_missing) {
                    leader.partial_from = Some(hour);
                }
                let traits_unchanged = state.last_read.as_ref().is_some_and(|last| {
                    last.trait_epoch == trait_epoch || last.traits.as_deref() == raw_traits
                });
                let unchanged = state.last_read.as_ref().is_some_and(|last| {
                    last.row.category == row.category
                        && last.row.kind == row.kind
                        && last.row.skill_status == row.skill_status
                        && last.row.skill_level == row.skill_level
                        && last.row.components == row.components
                        && traits_unchanged
                        && (owner_unchanged
                            || reply.roles[last.role as usize].owner.as_deref() == owner)
                        && last.person == person_id
                });
                if unchanged
                    && !gap
                    && let Some(i) = state.last_run
                {
                    stored_runs[i].run.last_hour = hour;
                    let read = state.last_read.as_mut().expect("unchanged read");
                    // Experience and other unshown fields can change without
                    // creating new values, roles or trait holdings.
                    if read.row != *row {
                        read.row.clone_from(row);
                    }
                    read.person_epoch = identity_epoch;
                    read.owner_slot = owner_slot;
                    read.owner_epoch = owner_epoch;
                    read.trait_epoch = trait_epoch;
                    continue;
                }
                let held = if traits_unchanged {
                    previous_run.expect("last trait read").value.traits.clone()
                } else {
                    raw_traits.and_then(|ts| {
                        let mut held = Vec::with_capacity(ts.len());
                        for t in ts {
                            let token = match std::str::from_utf8(&t.token) {
                                Ok(token) => token,
                                Err(_) => {
                                    refusals
                                        .note("Trait token is not UTF-8", reference.to_string());
                                    return None;
                                }
                            };
                            let index = if let Some(&index) = trait_tokens.get(token) {
                                index
                            } else {
                                let index = token_list.len() as u32;
                                token_list.push(token.to_owned());
                                trait_tokens.insert(token.to_owned(), index);
                                index
                            };
                            held.push(index);
                        }
                        held.sort_unstable();
                        held.dedup();
                        if let Some(shared) = trait_sets.get(&held) {
                            Some(Arc::clone(shared))
                        } else {
                            let shared = Arc::from(held.as_slice());
                            trait_sets.insert(held, Arc::clone(&shared));
                            Some(shared)
                        }
                    })
                };
                let maximum_skill = state
                    .last_read
                    .as_ref()
                    .filter(|last| {
                        last.row.skill_level == row.skill_level && last.row.category == row.category
                    })
                    .map(|last| reply.roles[last.role as usize].maximum_skill)
                    .unwrap_or_else(|| {
                        *skill_ceilings
                            .entry((row.skill_level, row.category))
                            .or_insert_with(|| {
                                content.and_then(|c| {
                                    maximum_skill(&c.leaders.skills, row.skill_level, row.category)
                                })
                            })
                    });
                let key = (owner_slot, row.category, row.kind, maximum_skill);
                let cached_role = state
                    .last_read
                    .as_ref()
                    .filter(|last| {
                        last.row.category == row.category
                            && last.row.kind == row.kind
                            && reply.roles[last.role as usize].maximum_skill == maximum_skill
                            && (owner_unchanged
                                || reply.roles[last.role as usize].owner.as_deref() == owner)
                    })
                    .map(|last| last.role)
                    .or_else(|| roles.get(&key).copied())
                    .filter(|&i| reply.roles[i as usize].owner.as_deref() == owner);
                let role = if let Some(role) = cached_role {
                    role
                } else {
                    let owner = owner.map(str::to_owned);
                    let rank = rank_names
                        .entry(row.category)
                        .or_insert_with(|| {
                            content.and_then(|c| {
                                rank_key(row.category)
                                    .and_then(|key| c.catalogue.localised(key, View::Plain))
                            })
                        })
                        .clone();
                    if row.skill_status != 0 && maximum_skill.is_none() {
                        refusals.note("Maximum skill is unavailable from recorded content or its skill definitions", row.category.to_string());
                    }
                    if rank_key(row.category).is_none() {
                        refusals.note("Unsupported leader rank category", row.category.to_string());
                    }
                    let index = reply.roles.len() as u32;
                    reply.roles.push(LeaderRole {
                        owner,
                        category: row.category,
                        rank,
                        kind: row.kind,
                        maximum_skill,
                        attribute_names: attribute_names(
                            row.kind,
                            content.map(|content| content.catalogue.as_ref()),
                        ),
                    });
                    roles.insert(key, index);
                    index
                };
                if let Some(last) = &mut state.last_read {
                    last.row.clone_from(row);
                    if !traits_unchanged {
                        last.traits = raw_traits.map(<[_]>::to_vec);
                    }
                    last.role = role;
                    last.person = person_id;
                    last.person_epoch = identity_epoch;
                    last.owner_slot = owner_slot;
                    last.owner_epoch = owner_epoch;
                    last.trait_epoch = trait_epoch;
                } else {
                    state.last_read = Some(LastRead {
                        row: row.clone(),
                        traits: raw_traits.map(<[_]>::to_vec),
                        role,
                        person: person_id,
                        person_epoch: identity_epoch,
                        owner_slot,
                        owner_epoch,
                        trait_epoch,
                    });
                }
                let value = LeaderValues {
                    person: person_id,
                    role,
                    skill: (row.skill_status != 0).then_some(row.skill_level),
                    components: row.components,
                    traits: held,
                };
                append_stored_run(state, &mut stored_runs, value, hour, start, choices);
            }
            if land_commands {
                let names: HashMap<_, _> = decoded
                    .complete::<tables::OrdersGroupPlans>()
                    .ok()
                    .into_iter()
                    .flatten()
                    .map(|p| (p.group, &p.name))
                    .collect();
                if let Ok(groups) = decoded.complete::<tables::OrdersGroups>() {
                    for group in groups {
                        let Some(&index) = leader_indices.get(&group.leader) else {
                            continue;
                        };
                        let state = &mut leaders[index];
                        if state.visible_hour != Some(hour) {
                            continue;
                        }
                        let raw_name = match &group.name {
                            Recorded::Value(Some(bytes)) => Some(bytes.as_slice()),
                            Recorded::NotRecorded => {
                                names.get(&group.handle).map(|bytes| bytes.as_slice())
                            }
                            Recorded::Value(None) => {
                                refusals.note(
                                    "Stored army command name capture refused",
                                    group.handle.to_string(),
                                );
                                None
                            }
                        };
                        let name = formation_name(raw_name, content)
                            .map_err(|reason| refusals.note(reason, group.handle.to_string()))
                            .ok()
                            .flatten();
                        let kind = if group.is_army_group != 0 {
                            LeaderFormationKind::ArmyGroup
                        } else {
                            LeaderFormationKind::Army
                        };
                        append_command(state, group.handle, kind, name, hour);
                    }
                }
            }
            if let Some(fleets) = fleet_commands {
                for fleet in fleets {
                    let commander = match fleet.leader {
                        Recorded::Value(Some(reference)) => reference,
                        _ => {
                            naval_commands = false;
                            refusals.note(
                                "Fleet commander reference capture refused",
                                fleet.id.to_string(),
                            );
                            continue;
                        }
                    };
                    let Some(&index) = leader_indices.get(&commander) else {
                        continue;
                    };
                    let state = &mut leaders[index];
                    if state.visible_hour != Some(hour)
                        || state
                            .last_read
                            .as_ref()
                            .is_none_or(|last| last.row.kind != engine::cunit_leader::KIND_NAVY)
                    {
                        continue;
                    }
                    let raw_name = match &fleet.name {
                        Recorded::Value(Some(bytes)) => Some(bytes.as_slice()),
                        _ => {
                            refusals.note("Stored fleet name unavailable", fleet.id.to_string());
                            None
                        }
                    };
                    let name = formation_name(raw_name, content)
                        .map_err(|reason| refusals.note(reason, fleet.id.to_string()))
                        .ok()
                        .flatten();
                    append_command(state, fleet.id, LeaderFormationKind::Fleet, name, hour);
                }
            } else if naval_commands
                && let (Ok(missions), Ok(tasks)) = (
                    decoded.complete::<tables::NavalMissionTaskForces>(),
                    decoded.complete::<tables::TaskForces>(),
                )
            {
                let mut fleets = BTreeMap::<u64, Option<u64>>::new();
                for mission in missions {
                    let Some(task) = tasks
                        .binary_search_by_key(&mission.task_force, |task| task.id)
                        .ok()
                        .map(|index| &tasks[index])
                    else {
                        naval_commands = false;
                        continue;
                    };
                    if task.fleet == 0 {
                        naval_commands = false;
                        continue;
                    }
                    let commander = fleets.entry(task.fleet).or_insert(Some(mission.leader));
                    if *commander != Some(mission.leader) {
                        *commander = None;
                        naval_commands = false;
                        refusals.note(
                            "Conflicting recorded fleet commander references",
                            task.fleet.to_string(),
                        );
                    }
                }
                for (fleet, commander) in fleets {
                    let Some(index) = commander.and_then(|c| leader_indices.get(&c).copied())
                    else {
                        continue;
                    };
                    let state = &mut leaders[index];
                    if state.visible_hour == Some(hour)
                        && state
                            .last_read
                            .as_ref()
                            .is_some_and(|last| last.row.kind == engine::cunit_leader::KIND_NAVY)
                    {
                        append_command(state, fleet, LeaderFormationKind::Fleet, None, hour);
                    }
                }
            }
            observe(
                &mut reply.observed,
                hour,
                complete,
                land.is_some(),
                navy.is_some(),
                land_commands,
                naval_commands,
            );
            if !choices {
                let combat_sets = [
                    land.map(|rows| rows.iter().map(|c| c.combat).collect::<BTreeSet<_>>()),
                    navy.map(|rows| rows.iter().map(|c| c.combat).collect::<BTreeSet<_>>()),
                ];
                let sides = land
                    .into_iter()
                    .flatten()
                    .flat_map(|c| {
                        c.sides.iter().enumerate().map(move |(side, s)| {
                            (1u8, c.combat, s.commander, 1u8 << side, hour, true)
                        })
                    })
                    .chain(navy.into_iter().flatten().flat_map(|c| {
                        c.sides.iter().enumerate().map(move |(side, s)| {
                            (2u8, c.combat, s.commander, 1u8 << side, hour, true)
                        })
                    }))
                    .chain(
                        decoded
                            .complete::<tables::LandCombatResults>()
                            .ok()
                            .filter(|_| {
                                slot_read(&decoded.metrics, TableId::LandCombatResults, hour)
                            })
                            .into_iter()
                            .flatten()
                            .flat_map(|result| {
                                result
                                    .sides
                                    .iter()
                                    .enumerate()
                                    .map(move |(side, operands)| {
                                        (
                                            1u8,
                                            result.combat,
                                            operands.commander,
                                            1u8 << side,
                                            result.end_hour,
                                            false,
                                        )
                                    })
                            }),
                    )
                    .chain(
                        decoded
                            .complete::<tables::NavalCombatResultDetails>()
                            .ok()
                            .into_iter()
                            .flatten()
                            .filter_map(|result| {
                                naval_sources
                                    .get(&result.result)
                                    .map(|source| (result, source))
                            })
                            .flat_map(|(result, source)| {
                                result
                                    .sides
                                    .iter()
                                    .enumerate()
                                    .map(move |(side, operands)| {
                                        (
                                            2u8,
                                            source.combat,
                                            operands.leader,
                                            1u8 << side,
                                            source.game_hour,
                                            false,
                                        )
                                    })
                            }),
                    );
                for (kind, combat, commander, side, participation_hour, active) in sides {
                    let Some(&index) = leader_indices.get(&commander) else {
                        continue;
                    };
                    if leaders[index].visible_hour != Some(hour) {
                        continue;
                    }
                    let reads = battle_hours.entry(commander).or_default();
                    if let Some(&index) = battle_indices.get(&(commander, kind, combat)) {
                        reads[index].5 |= side;
                    } else {
                        let combat_new_this_hour = active
                            && previous_combats[usize::from(kind - 1)]
                                .as_ref()
                                .is_some_and(|(h, set)| {
                                    h.checked_add(1) == Some(hour) && !set.contains(&combat)
                                });
                        battle_indices.insert((commander, kind, combat), reads.len());
                        reads.push((
                            participation_hour,
                            hour,
                            !combat_new_this_hour,
                            kind,
                            combat,
                            side,
                        ));
                    }
                }
                previous_combats = combat_sets.map(|set| set.map(|set| (hour, set)));
            }
        }
    }
    reply.leaders = leaders
        .into_iter()
        .filter_map(|state| {
            if state.last_run.is_none() || requested.is_some() && !state.admitted {
                return None;
            }
            let mut leader = state.career;
            let mut index = state.last_run;
            while let Some(i) = index {
                let stored = &stored_runs[i];
                leader.runs.push(stored.run.clone());
                index = stored.previous;
            }
            leader.runs.reverse();
            if let Some(mut hours) = battle_hours.remove(&state.reference) {
                hours.sort_by_key(|read| (read.0, read.3, read.4));
                let mut previous = 0_i64;
                let mut won = Vec::with_capacity(hours.len());
                let mut result_hours = Vec::with_capacity(hours.len());
                let mut result_read_hours = Vec::with_capacity(hours.len());
                for (index, (hour, read_hour, unknown, kind, combat, side)) in
                    hours.into_iter().enumerate()
                {
                    leader.battle_starts.push(i64::from(hour) - previous);
                    leader.battle_start_read_hours.push(read_hour);
                    previous = i64::from(hour);
                    let result = outcomes.get(&(kind, combat)).copied().flatten().filter(
                        |(result_hour, _, winners)| {
                            *result_hour >= hour && *winners != 0 && side.count_ones() == 1
                        },
                    );
                    won.push(result.map(|(_, _, winners)| winners & side != 0));
                    result_hours.push(result.map(|(hour, _, _)| hour));
                    result_read_hours.push(result.map(|(_, read_hour, _)| read_hour));
                    if unknown {
                        leader.unknown_battle_starts.push(index as u32);
                    }
                }
                if results_recorded {
                    leader.battle_won = Some(won);
                    leader.battle_result_hours = Some(result_hours);
                    leader.battle_result_read_hours = Some(result_read_hours);
                }
            }
            if results_recorded && leader.battle_won.is_none() {
                leader.battle_won = Some(Vec::new());
                leader.battle_result_hours = Some(Vec::new());
                leader.battle_result_read_hours = Some(Vec::new());
            }
            Some(leader)
        })
        .collect();
    reply
        .leaders
        .sort_unstable_by_key(|l| l.reference.parse::<u64>().expect("leader reference"));
    {
        let used: BTreeSet<_> = reply
            .leaders
            .iter()
            .flat_map(|leader| leader.runs.iter().map(|run| run.value.person))
            .collect();
        let remap: BTreeMap<_, _> = used
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new as u32))
            .collect();
        reply.people = used
            .into_iter()
            .map(|index| reply.people[index as usize].clone())
            .collect();
        let used_roles: BTreeSet<_> = reply
            .leaders
            .iter()
            .flat_map(|leader| leader.runs.iter().map(|run| run.value.role))
            .collect();
        let role_remap: BTreeMap<_, _> = used_roles
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new as u32))
            .collect();
        reply.roles = used_roles
            .into_iter()
            .map(|index| reply.roles[index as usize].clone())
            .collect();
        for leader in &mut reply.leaders {
            for run in &mut leader.runs {
                run.value.person = remap[&run.value.person];
                run.value.role = role_remap[&run.value.role];
            }
        }
        let used_traits: BTreeSet<_> =
            reply
                .leaders
                .iter()
                .flat_map(|leader| {
                    leader.trait_gains.iter().map(|gain| gain.token).chain(
                        leader.runs.iter().flat_map(|run| {
                            run.value
                                .traits
                                .iter()
                                .flat_map(|held| held.iter().copied())
                        }),
                    )
                })
                .collect();
        if used_traits.len() != token_list.len() {
            let mut remap = vec![0; token_list.len()];
            for (new, old) in used_traits.iter().enumerate() {
                remap[*old as usize] = new as u32;
            }
            let mut sets = HashMap::<Vec<u32>, Arc<[u32]>>::new();
            for leader in &mut reply.leaders {
                for gain in &mut leader.trait_gains {
                    gain.token = remap[gain.token as usize];
                }
                for run in &mut leader.runs {
                    if let Some(held) = &mut run.value.traits {
                        let mapped = if let Some(mapped) = sets.get(held.as_ref()) {
                            Arc::clone(mapped)
                        } else {
                            let mapped: Arc<[u32]> =
                                held.iter().map(|old| remap[*old as usize]).collect();
                            sets.insert(held.to_vec(), Arc::clone(&mapped));
                            mapped
                        };
                        *held = mapped;
                    }
                }
            }
            token_list = token_list
                .into_iter()
                .enumerate()
                .filter_map(|(i, token)| used_traits.contains(&(i as u32)).then_some(token))
                .collect();
        }
        if choices {
            reply.observed.clear();
        }
    }
    for token in token_list {
        let info = trait_info(&token, content.map(|content| content.catalogue.as_ref()));
        if let Some(reason) = &info.missing {
            refusals.note(reason.clone(), token);
        }
        reply.traits.push(info);
    }
    reply
        .warnings
        .extend(refusals.lines("Leader data withheld"));
    if let Some(c) = content {
        reply.warnings.extend(c.catalogue.verification.clone());
        reply.warnings.extend(c.catalogue.warnings.iter().cloned());
        reply.warnings.extend(c.politics.warnings.iter().cloned());
        reply
            .warnings
            .extend(c.leaders.skills.warnings().iter().cloned());
        reply.warnings.extend(c.leaders.misreads.iter().cloned());
        reply.warnings.extend(
            c.leaders
                .skills
                .conditions()
                .iter()
                .map(|condition| format!("Maximum skill withheld: {condition}")),
        );
        reply.warnings.extend(c.catalogue.refusal_lines());
    }
    reply.warnings.sort();
    reply.warnings.dedup();
    Ok(reply)
}

// Retention limits cached snapshots, never the inventory a request may ask for.
const CACHED_BRANCHES: usize = 2;
const CACHED_REPLIES: usize = 4;
type CachedReply = Result<Arc<UnitLeadersReply>, (io::ErrorKind, String)>;

fn remembered(result: io::Result<Arc<UnitLeadersReply>>) -> CachedReply {
    result.map_err(|error| (error.kind(), error.to_string()))
}
fn recalled(result: CachedReply) -> io::Result<Arc<UnitLeadersReply>> {
    result.map_err(|(kind, message)| io::Error::new(kind, message))
}

#[derive(Default)]
struct BranchState {
    scan: Option<CachedReply>,
    scanning: bool,
    replies: VecDeque<(UnitLeadersParams, CachedReply)>,
}
#[derive(Default)]
struct BranchSlot {
    state: Mutex<BranchState>,
    ready: Condvar,
}
impl BranchSlot {
    fn scanned(
        &self,
        cancelled: &AtomicBool,
        build: impl FnOnce() -> io::Result<UnitLeadersReply>,
    ) -> io::Result<Arc<UnitLeadersReply>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Leader request cancelled",
                ));
            }
            if let Some(reply) = &state.scan {
                return recalled(reply.clone());
            }
            if !state.scanning {
                state.scanning = true;
                break;
            }
            // A waiting career/picker can cancel independently of the scanning request.
            state = self
                .ready
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
        drop(state);
        let result = remembered(build().map(Arc::new));
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.scanning = false;
        if !matches!(&result, Err((io::ErrorKind::Interrupted, _))) {
            state.scan = Some(result.clone());
        }
        self.ready.notify_all();
        recalled(result)
    }

    fn reply(&self, params: &UnitLeadersParams) -> Option<CachedReply> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let index = state.replies.iter().position(|(key, _)| key == params)?;
        let entry = state.replies.remove(index)?;
        let reply = entry.1.clone();
        state.replies.push_back(entry);
        Some(reply)
    }
    fn remember(&self, params: &UnitLeadersParams, reply: CachedReply) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.replies.retain(|(key, _)| key != params);
        state.replies.push_back((params.clone(), reply));
        while state.replies.len() > CACHED_REPLIES {
            state.replies.pop_front();
        }
    }
}

#[derive(Default)]
struct Cache {
    catalogue: Option<Arc<Catalogue>>,
    branches: VecDeque<(String, Arc<BranchSlot>)>,
}
impl Cache {
    fn branch(&mut self, catalogue: Option<Arc<Catalogue>>, branch: &str) -> Arc<BranchSlot> {
        let same = match (&self.catalogue, &catalogue) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.branches.clear();
            self.catalogue = catalogue;
        }
        if let Some(index) = self.branches.iter().position(|(key, _)| key == branch) {
            let entry = self.branches.remove(index).expect("cached branch");
            let slot = Arc::clone(&entry.1);
            self.branches.push_back(entry);
            return slot;
        }
        let slot = Arc::new(BranchSlot::default());
        self.branches
            .push_back((branch.to_owned(), Arc::clone(&slot)));
        while self.branches.len() > CACHED_BRANCHES {
            self.branches.pop_front();
        }
        slot
    }
}

fn trait_info(token: &str, content: Option<&Catalogue>) -> LeaderTraitInfo {
    let (name, icon, missing) = content
        .map(|content| {
            let name = content.localised(token, View::Plain);
            let sprite_name = format!("GFX_trait_{token}");
            let asset = content
                .sprites
                .find(&sprite_name)
                .map_err(crate::invalid)
                .and_then(|sprite| {
                    sprite.ok_or_else(|| crate::invalid(format!("Missing sprite {sprite_name}")))
                })
                .and_then(|sprite| {
                    catalogue::sprite_frame_png(
                        &content.content.layers,
                        sprite,
                        *content
                            .gui_decode
                            .as_ref()
                            .map_err(|why| crate::invalid(why.clone()))?,
                        0,
                        32,
                    )
                });
            match asset {
                Ok(icon) => (name, Some(icon), None),
                Err(error) => (name, None, Some(error.to_string())),
            }
        })
        .unwrap_or((None, None, Some("Recorded content unavailable".into())));
    LeaderTraitInfo {
        token: token.into(),
        name,
        icon,
        missing,
    }
}

// Native localisation keys: army labels at CalculateFactionTheaterCommanderBonus
// @14116f770, navy at CNavyLeaderTraitWindow::vt28_vf0@14180c000.
fn attribute_names(kind: u8, content: Option<&Catalogue>) -> [Option<String>; 4] {
    let keys = if kind == engine::cunit_leader::KIND_NAVY {
        [
            "SKILL_NAVY_LEADER_ATTACK",
            "SKILL_NAVY_LEADER_DEFENSE",
            "SKILL_NAVY_LEADER_MANEUVERING",
            "SKILL_NAVY_LEADER_COORDINATION",
        ]
    } else {
        [
            "SKILL_ATTACK",
            "SKILL_DEFENSE",
            "SKILL_PLANNING",
            "SKILL_LOGISTICS",
        ]
    };
    keys.map(|key| content.and_then(|content| content.localised(key, View::Plain)))
}

fn formation_name(
    bytes: Option<&[u8]>,
    content: Option<&Content>,
) -> Result<Option<String>, String> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let content = content.ok_or("Formation name content unavailable")?;
    let text = crate::scripted_text::stored_text(
        &content.catalogue.localisation,
        &content.catalogue.recorded_language,
        bytes,
        "formation name",
    )?;
    catalogue::display_text(text.as_bytes()).map(Some)
}

type BattleOutcomes = BTreeMap<(u8, u64), Option<(i32, i32, u8)>>;

fn record_outcome(outcomes: &mut BattleOutcomes, key: (u8, u64), result: (i32, i32, u8)) -> bool {
    match outcomes.entry(key) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(Some(result));
            false
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            if entry
                .get()
                .is_some_and(|previous| (previous.0, previous.2) != (result.0, result.2))
            {
                entry.insert(None);
                true
            } else {
                false
            }
        }
    }
}

fn project(
    scanned: &UnitLeadersReply,
    params: &UnitLeadersParams,
    history: Option<&super::country_history::RecordedHistories>,
) -> UnitLeadersReply {
    let requested: BTreeSet<_> = match &params.selection {
        LeaderSelection::Countries { tags } => history
            .map_or_else(
                || tags.clone(),
                |history| super::country_history::selected_aliases(history, tags),
            )
            .into_iter()
            .collect(),
        _ => BTreeSet::new(),
    };
    let selected = history.and_then(|history| {
        let LeaderSelection::Countries { tags } = &params.selection else {
            return None;
        };
        let identities = super::country_history::selected_identities(history, tags);
        Some(
            requested
                .iter()
                .map(|tag| {
                    let runs = history.countries[tag]
                        .identities
                        .iter()
                        .filter(|identity| {
                            identities.contains(&(identity.slot, identity.country_id))
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    (tag.as_str(), runs)
                })
                .collect::<BTreeMap<_, _>>(),
        )
    });
    let choices = params.view == LeaderView::Choices;
    let leaders: Vec<_> = scanned
        .leaders
        .iter()
        .filter(|leader| {
            leader.runs.iter().any(|run| {
                params
                    .leaders
                    .includes(scanned.roles[run.value.role as usize].kind)
            }) && match &params.selection {
                LeaderSelection::All => true,
                LeaderSelection::Leader { reference } => &leader.reference == reference,
                LeaderSelection::Countries { .. } => leader.runs.iter().any(|run| {
                    scanned.roles[run.value.role as usize]
                        .owner
                        .as_deref()
                        .is_some_and(|owner| {
                            requested.contains(owner)
                                && selected.as_ref().is_none_or(|selected| {
                                    let identities = &selected[owner];
                                    let first = identities.partition_point(|identity| {
                                        identity.last_hour < run.first_hour
                                    });
                                    identities.get(first).is_some_and(|identity| {
                                        identity.first_hour <= run.last_hour
                                    })
                                })
                        })
                }),
            }
        })
        .map(|leader| {
            let runs = if choices {
                let mut last = leader
                    .runs
                    .last()
                    .expect("visible career has a run")
                    .clone();
                last.first_hour = last.last_hour;
                last.value.traits = None;
                vec![last]
            } else {
                leader.runs.clone()
            };
            LeaderCareer {
                reference: leader.reference.clone(),
                runs,
                trait_gains: if choices {
                    Vec::new()
                } else {
                    leader.trait_gains.clone()
                },
                commands: if params.view == LeaderView::Career {
                    leader.commands.clone()
                } else {
                    Vec::new()
                },
                battle_starts: if choices {
                    Vec::new()
                } else {
                    leader.battle_starts.clone()
                },
                battle_start_read_hours: if choices {
                    Vec::new()
                } else {
                    leader.battle_start_read_hours.clone()
                },
                unknown_battle_starts: if choices {
                    Vec::new()
                } else {
                    leader.unknown_battle_starts.clone()
                },
                battle_won: if choices {
                    None
                } else {
                    leader.battle_won.clone()
                },
                battle_result_hours: if choices {
                    None
                } else {
                    leader.battle_result_hours.clone()
                },
                battle_result_read_hours: if choices {
                    None
                } else {
                    leader.battle_result_read_hours.clone()
                },
                partial_from: leader.partial_from,
            }
        })
        .collect();
    let mut reply = UnitLeadersReply {
        branch_id: scanned.branch_id.clone(),
        first_hour: scanned.first_hour,
        leaders,
        people: Vec::new(),
        roles: Vec::new(),
        traits: Vec::new(),
        observed: if choices {
            Vec::new()
        } else {
            scanned.observed.clone()
        },
        selection_tags: match &params.selection {
            LeaderSelection::Countries { .. } => Some(requested.into_iter().collect()),
            _ => None,
        },
        results_missing: scanned.results_missing.clone(),
        navy_commands_missing: scanned.navy_commands_missing.clone(),
        warnings: scanned.warnings.clone(),
    };
    let used: BTreeSet<_> = reply
        .leaders
        .iter()
        .flat_map(|leader| leader.runs.iter().map(|run| run.value.person))
        .collect();
    let remap: BTreeMap<_, _> = used
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new as u32))
        .collect();
    reply.people = used
        .iter()
        .map(|index| scanned.people[*index as usize].clone())
        .collect();
    let roles: BTreeSet<_> = reply
        .leaders
        .iter()
        .flat_map(|leader| leader.runs.iter().map(|run| run.value.role))
        .collect();
    let role_remap: BTreeMap<_, _> = roles
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new as u32))
        .collect();
    reply.roles = roles
        .iter()
        .map(|index| scanned.roles[*index as usize].clone())
        .collect();
    let traits: BTreeSet<_> = reply
        .leaders
        .iter()
        .flat_map(|leader| {
            leader
                .trait_gains
                .iter()
                .map(|gain| gain.token)
                .chain(leader.runs.iter().flat_map(|run| {
                    run.value
                        .traits
                        .iter()
                        .flat_map(|held| held.iter().copied())
                }))
        })
        .collect();
    let trait_remap: BTreeMap<_, _> = traits
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new as u32))
        .collect();
    reply.traits = traits
        .iter()
        .map(|index| scanned.traits[*index as usize].clone())
        .collect();
    let mut trait_sets = HashMap::<Vec<u32>, Arc<[u32]>>::new();
    let remap_traits = reply.traits.len() != scanned.traits.len();
    for leader in &mut reply.leaders {
        for run in &mut leader.runs {
            run.value.person = remap[&run.value.person];
            run.value.role = role_remap[&run.value.role];
            if remap_traits && let Some(held) = &mut run.value.traits {
                let mapped = trait_sets
                    .entry(held.to_vec())
                    .or_insert_with(|| held.iter().map(|old| trait_remap[old]).collect());
                *held = Arc::clone(mapped);
            }
        }
        for gain in &mut leader.trait_gains {
            gain.token = trait_remap[&gain.token];
        }
    }
    reply
}

impl Product for UnitLeaders {
    const KEY: &'static str = "unit-leaders";
    const CONTENT_DEPENDENT: bool = true;
    type Params = UnitLeadersParams;
    type Reply = UnitLeadersResponse;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        match &params.selection {
            LeaderSelection::All => {}
            LeaderSelection::Countries { tags } => {
                for tag in tags {
                    validate_tag(tag)?;
                }
            }
            LeaderSelection::Leader { reference } => {
                reference
                    .parse::<u64>()
                    .map_err(|_| "Invalid leader reference")?;
            }
        }
        if params.view == LeaderView::Career
            && !matches!(params.selection, LeaderSelection::Leader { .. })
        {
            return Err("A career request needs one leader");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            LeaderSelection::decl(),
            LeaderView::decl(),
            LeaderKinds::decl(),
            UnitLeadersParams::decl(),
            LeaderPerson::decl(),
            LeaderRole::decl(),
            LeaderValues::decl(),
            UnitLeaderRun::decl(),
            LeaderChangeDate::decl(),
            LeaderTraitGain::decl(),
            LeaderFormationKind::decl(),
            LeaderCommand::decl(),
            LeaderCareer::decl(),
            LeaderTraitInfo::decl(),
            LeaderObservation::decl(),
            UnitLeadersReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let content_error = cat
            .as_ref()
            .err()
            .map(|e| format!("Leader content unavailable: {e}"));
        let cat = cat.ok();
        let state = source.state::<Cache>();
        let slot = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .branch(cat.clone(), &params.branch_id);
        if let Some(reply) = slot.reply(params) {
            return recalled(reply).map(UnitLeadersResponse::from);
        }
        let scanned = slot.scanned(cancelled, || {
            let content = cat
                .map(|catalogue| -> io::Result<Content> {
                    let politics = politics_content::politics_content(source, context.content()?)?;
                    Content::load(catalogue, politics)
                })
                .transpose();
            let full = UnitLeadersParams {
                branch_id: params.branch_id.clone(),
                selection: LeaderSelection::All,
                view: LeaderView::Career,
                leaders: LeaderKinds::All,
            };
            let mut reply = scan(
                &source.replay(),
                &full,
                content.as_ref().ok().and_then(Option::as_ref),
                cancelled,
            )?;
            reply.warnings.extend(content_error);
            if let Err(error) = content {
                reply
                    .warnings
                    .push(format!("Leader content unavailable: {error}"));
            }
            reply.warnings.sort();
            reply.warnings.dedup();
            Ok(reply)
        })?;
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Leader request cancelled",
            ));
        }
        let history = if let LeaderSelection::Countries { .. } = &params.selection {
            let branch = params.branch_id.parse::<u64>().expect("validated branch");
            Some(super::country_history::branch_histories(
                source,
                branch,
                false,
                &[],
                cancelled,
            )?)
        } else {
            None
        };
        let reply = project(&scanned, params, history.as_deref());
        // Count only the projected wire reply. Remember an oversize refusal too.
        let result = remembered(reply_bytes(&reply).map(|_| Arc::new(reply)));
        slot.remember(params, result.clone());
        recalled(result).map(UnitLeadersResponse::from)
    }
}

#[cfg(test)]
mod tests;
