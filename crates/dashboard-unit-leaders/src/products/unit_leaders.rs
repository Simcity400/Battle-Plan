//! Military leader careers from hourly reads; no combat outcome is inferred.
//!
//! Identity uses the full leader type/id, character link and raw owner;
//! CountryLinks resolves that owner. Categories 0/1/2 map to
//! fm_leader/cc_leader/navy_leader, resolved in the recorded stack.
//! KIND is the recorder's leader-kind discriminator. Army skills are
//! attack/defence/planning/logistics; navy skills are
//! attack/defence/maneuvering/coordination. A null skill object is missing,
//! not zero. Skill tooltip and experience progression stop at a missing next
//! skill definition; the star ceiling follows that same content lookup.
//!
//! Visibility uses the shared evaluator in the owner's country scope. The roster
//! enables its captured-role gate, excluding captured army categories <2.
//! Unknown visibility is withheld, with a named warning. No foreign-country/AI
//! gate. That caller also sets the selected character without clearing the
//! country; the shared CountryLeader scope preserves both carriers.
//! Portraits ask the recorded holder for army/navy, large (portrait_entry's
//! fallback); PoliticsContent decodes the same entries as Ruling party.
//! Trait names use their recorded tokens. Trait icons are GFX_trait_<token>,
//! resolved through the catalogue. TraitExperience entries are learning
//! progress, never assigned traits.
//!
//! Commands: OrdersGroups.leader reads the leader's full handle; is_army_group
//! and OrdersGroupPlan.name are stored fields. Fleets.leader is the full leader
//! reference; Fleets.name and OrdersGroups.name hold hourly stored strings.
//! Older recordings can supply command names from daily OrdersGroupPlans and
//! naval commands from NavalMissionTaskForces.leader, its fleet's leader reference.
//! Join TaskForces.fleet, never the mission's distinct range_fleet. Coverage is
//! limited to the mission kinds this hourly table records (naval_mission_rows.rs).
//! Fleet names and idle commands are unavailable only in those older records.
//! LandCombatDetails.sides[].commander and
//! NavalCombatDetails.sides[].commander identify side command once per full
//! combat handle. Each leader's hours are observation spans, never inferred dates.
//! An already-active or post-gap combat has an unknown start. LandCombatResults
//! joins the full combat handle. NavalResultSources links retained NavalResults
//! through the exact creation transition, not a date, province or sunk-ship
//! heuristic. Transient unregistered combats never join.
//!
//! Requests name countries, one leader, or the recording's lightweight choice
//! list. Careers keep each participation's dates and reads; leaderboards get
//! only the hours their counts change, which give the same totals at every
//! hour. Equal consecutive reads compress; missing hours, hidden leaders and
//! incomplete inputs break runs. A gain after missing input says first observed,
//! while holdings at branch start say held before. The counting JSON writer
//! refuses beyond the shared transport reply budget before sending.

use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::products::{Product, country_groups::HourSpan, politics_content};
use dashboard_base::{catalogue, source::Source};
use dashboard_core::{config::LoadContext, protocol::validate_tag};
use dashboard_text::identity::character_script_token;
use dashboard_text::{
    catalogue::{Catalogue, View},
    metadata::{LinkTags, link_slot, slot_read, validate_campaign},
    scripted_text::ScriptedContent,
    triggers::{Eval, Evaluator, LeaderVisibility, World, core_gap},
};
use replay::container::Replay;
use replay_data::{
    format::FormatVersion, rows::CharacterPortraitRow, rows::CharacterRow,
    rows::LandCombatDetailsRow, rows::NavalCombatDetailsRow, rows::Recorded, rows::TableId,
    rows::UnitLeaderRow, rows::UnitLeaderTraitExperienceRow, rows::UnitLeaderTraitRow,
    rows::full_mask, rows::tables,
};
use replay_frame::{
    frame::DecodedFrame, frame::FrameDecoder, frame::SharedView, frame::ViewIdentity,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, OnceCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[cfg(any(test, feature = "test-support"))]
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::Duration,
};
use ts_rs::TS;

pub struct UnitLeaders;
pub mod block;
pub mod portraits;
pub mod prepared;
pub const MAX_REPLY_BYTES: usize = dashboard_base::products::MAX_PRODUCT_REPLY_BYTES;
// Portrait tokens map army 0x289d to 1 and navy 0x289e to 2.
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
                engine_native::cunit_leader::KIND_ARMY | engine_native::cunit_leader::KIND_NAVY
            ),
            Self::Land => kind == engine_native::cunit_leader::KIND_ARMY,
            Self::Navy => kind == engine_native::cunit_leader::KIND_NAVY,
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
pub struct LeaderTraining {
    pub token: Option<String>,
    pub name: Option<String>,
    pub experience_raw: String,
    pub required_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderTrainingFactor {
    pub token: Option<String>,
    pub name: Option<String>,
    pub factor_raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderProgress {
    pub experience_raw: String,
    pub next_required_raw: Option<String>,
    /// None is missing acquisition; an empty list is complete native absence.
    pub training: Option<Vec<LeaderTraining>>,
    pub factors: Option<Vec<LeaderTrainingFactor>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderValues {
    pub person: u32,
    pub role: u32,
    pub skill: Option<i32>,
    pub components: [i32; 4],
    pub progress: Option<LeaderProgress>,
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

mod owner_projection {
    use super::*;
    use dashboard_text::metadata::LinkProjection;

    pub fn serialize<S: serde::Serializer>(
        tags: &LinkTags,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        tags.projection().serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<LinkTags, D::Error> {
        Option::<LinkProjection>::deserialize(deserializer).map(|projection| {
            projection
                .map(LinkTags::from_projection)
                .unwrap_or_default()
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, TS)]
#[ts(
    type = "[number, number, number, number, number | null, [number, number, number, number], number[] | null, LeaderProgress | null]"
)]
pub struct UnitLeaderRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: LeaderValues,
}

type WireLeaderRun = (
    i32,
    i32,
    u32,
    u32,
    Option<i32>,
    [i32; 4],
    Option<Vec<u32>>,
    Option<LeaderProgress>,
);
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
            &v.progress,
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
        (first_hour, last_hour, person, role, skill, components, traits, progress): WireLeaderRun,
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
                progress,
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
            v.progress,
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
    /// Careers: first participation hours: first absolute, then deltas. Deduplicated by combat.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number[]")]
    pub battle_starts: Option<Vec<i64>>,
    /// Careers: first supplying read for each participation, independent of its native date.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub battle_start_read_hours: Option<Vec<i32>>,
    /// Careers: indices whose combat was not known absent in the preceding hour. This is
    /// the combat's start uncertainty, not the leader's participation date.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unknown_battle_starts: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub battle_won: Option<Vec<Option<bool>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub battle_result_hours: Option<Vec<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub battle_result_read_hours: Option<Vec<Option<i32>>>,
    /// Leaderboards: when each participation became visible, the later of its date
    /// and first supplying read. Sorted hours: first absolute, then deltas.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number[]")]
    pub fought_hours: Option<Vec<i64>>,
    /// Leaderboards: when each known win became visible, the latest of its
    /// participation's visible hour, result date and result read; coded as `fought_hours`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number[]")]
    pub won_hours: Option<Vec<i64>>,
    /// Leaderboards: the same for each known loss.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number[]")]
    pub lost_hours: Option<Vec<i64>>,
    /// First hour with incomplete battle observations; recorded totals remain visible.
    pub partial_from: Option<i32>,
    /// Leaderboards at an hour: the facts its other runs and battle hours gave there.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub at_hour: Option<LeaderAtHour>,
}

/// A leaderboard row at `hour`: where the career's first run started, and its
/// battles counted then.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderAtHour {
    pub hour: i32,
    pub first_seen_hour: i32,
    pub fought: u32,
    pub won: Option<u32>,
    pub lost: Option<u32>,
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub preparing: Vec<String>,
    pub warnings: Vec<String>,
}

/// The prepared leaderboard can ask for one hour beyond its history prefix.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitLeadersAtHourParams {
    pub branch_id: String,
    pub selection: LeaderSelection,
    pub view: LeaderView,
    #[serde(default)]
    pub leaders: LeaderKinds,
    pub hour: i32,
}

/// Per-response diagnostics can change without copying the cached history.
#[derive(Debug, TS)]
#[ts(type = "UnitLeadersReply")]
pub struct UnitLeadersResponse {
    history: Arc<UnitLeadersReply>,
    pub warnings: Vec<String>,
}

#[cfg(any(test, feature = "test-support"))]
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
            10 + usize::from(history.selection_tags.is_some())
                + usize::from(!history.preparing.is_empty()),
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
        if !history.preparing.is_empty() {
            reply.serialize_field("preparing", &history.preparing)?;
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

/// Role-title localisation interface keys, resolved through recorded content.
fn rank_key(category: i32) -> Option<&'static str> {
    match category {
        0 => Some("fm_leader"),
        1 => Some("cc_leader"),
        2 => Some("navy_leader"),
        _ => None,
    }
}

fn maximum_skill(
    skills: &installed_content_core::unit_leader::Skills,
    level: i32,
    category: i32,
) -> Option<i32> {
    if !skills.conditions().is_empty() || level < 0 {
        return None;
    }
    let mut ceiling = level;
    // Sparse skill tables stop at the first missing next level.
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
    name: Option<&str>,
    hour: i32,
) -> (usize, bool, usize) {
    let before = state.career.commands.len();
    let unknown_before = state
        .command_indices
        .get(&formation)
        .map_or(0, |&index| state.career.commands[index].name_unknown.len());
    let repeated = state
        .command_indices
        .get(&formation)
        .is_some_and(|&index| state.career.commands[index].last_hour == hour);
    let leader = &mut state.career;
    if let Some(last) = state
        .command_indices
        .get(&formation)
        .and_then(|&index| leader.commands.get_mut(index))
        && last.last_hour.checked_add(1) == Some(hour)
        && (name.is_none() || last.name.is_none() || last.name.as_deref() == name)
        && last.kind == kind
    {
        last.last_hour = hour;
        if name.is_none() {
            span_push(&mut last.name_unknown, hour);
        }
        if name.is_some() && last.name.is_none() {
            last.name = name.map(str::to_owned);
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
            name_read_hour: name.map(|_| hour),
            name_unknown: if name.is_none() {
                vec![HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                }]
            } else {
                Vec::new()
            },
            name: name.map(str::to_owned),
        });
    }
    let index = state.command_indices[&formation];
    let unknown = if state.career.commands.len() == before {
        state.career.commands[index]
            .name_unknown
            .len()
            .saturating_sub(unknown_before)
    } else {
        state.career.commands[index].name_unknown.len()
    };
    (
        index,
        repeated,
        (state.career.commands.len() - before) * std::mem::size_of::<LeaderCommand>()
            + unknown * std::mem::size_of::<HourSpan>(),
    )
}

/// The hour's trigger inputs, read when a leader first needs them: the World
/// and core-table check of `ScriptedText::world`, without its text context.
struct HourWorld<'a> {
    content: &'a ScriptedContent,
    frame: &'a DecodedFrame,
    world: OnceCell<Result<World<'a>, String>>,
    /// The core-table check, read without the World.
    gap: OnceCell<Option<String>>,
    /// How `frame`'s rows may differ from the hour's whole frame, when it is
    /// a frame kept from a block read by its changes; None for a whole frame.
    kept: Option<&'a Kept<'a>>,
}

/// A frame kept from a block read by its changes and observed in place of
/// the hour's whole frame: its rows differ from the whole frame's only as
/// `reduced` says (tables kept with the fields leaders and the declared
/// trigger reads read, or not kept). A trigger world built on it gives a read
/// the value it has on the whole frame exactly when the read's declared
/// dependence avoids those differences ([`Kept::reads_whole`]). An
/// evaluation that read anything else, or a read that could not be made
/// again on it, stops the hour's observation before it changes the scan
/// ([`Kept::blocked`]): the hour is then observed on its whole frame.
pub(crate) struct Kept<'a> {
    reduced: &'a dyn TableChanges,
    blocked: Cell<bool>,
}

/// The queries and table fields that defeat a visibility proof. Only a
/// measured rejection walks the keys or formats their names.
fn rejected_inputs(inputs: &Inputs, since: &Since<'_>) {
    if !replay_frame::cpu::enabled() {
        return;
    }
    let mut tables = BTreeSet::new();
    let mut declared = true;
    for key in inputs.keys() {
        match key.dependence() {
            Some(reads) => {
                for read in reads {
                    if since.changed(&read) {
                        tables.insert(read.table);
                        let _input = replay_frame::cpu::scope_named(
                            "leaders.rejected-input",
                            &rejected_input_label(key, Some(&read)),
                        );
                    }
                }
            }
            None => {
                declared = false;
                block::rejected("leaders.reject.undeclared-input");
                let _input = replay_frame::cpu::scope_named(
                    "leaders.undeclared-input",
                    &rejected_input_label(key, None),
                );
            }
        }
    }
    if declared && tables.is_empty() {
        block::rejected("leaders.reject.input-hour");
    }
    for table in tables {
        rejected_table(table);
    }
}

fn rejected_input_label(
    key: &dashboard_text::triggers::inputs::Key,
    read: Option<&TableRead>,
) -> String {
    let Some(read) = read else {
        return format!("{key:?}: no declared table/fields");
    };
    let fields = (0..read.table.schema().len())
        .filter(|&field| read.fields & (1_u64 << field) != 0)
        .filter_map(|field| read.table.field_name(field))
        .collect::<Vec<_>>()
        .join(",");
    format!("{key:?}: {:?}[keys,status,{fields}]", read.table)
}

fn rejected_table(table: TableId) {
    if replay_frame::cpu::enabled() {
        let _input =
            replay_frame::cpu::scope_named("leaders.rejected-table", &format!("{table:?}"));
    }
}

impl<'a> Kept<'a> {
    pub(crate) fn new(reduced: &'a dyn TableChanges) -> Self {
        Self {
            reduced,
            blocked: Cell::new(false),
        }
    }

    /// Whether the hour needs its whole frame.
    pub(crate) fn blocked(&self) -> bool {
        self.blocked.get()
    }

    fn block(&self, reason: &'static str) {
        if !self.blocked.replace(true) {
            block::rejected(reason);
        }
    }

    /// The first evaluation that requires the whole frame, with its
    /// differing input tables named separately in measured CPU labels.
    fn block_inputs(&self, inputs: &Inputs, frame: &DecodedFrame) {
        if self.blocked() {
            return;
        }
        self.block("leaders.reject.kept-inputs");
        let since = Since::span(
            frame.branch,
            frame.hour,
            frame.hour,
            Some(frame),
            self.reduced,
        );
        rejected_inputs(inputs, &since);
    }

    /// Whether every input of `inputs`, which last read its value at `from`
    /// on `frame`'s branch, reads on `frame` what it reads on the hour's
    /// whole frame: each declares its dependence and none reads a field,
    /// row key or table this frame does not keep as the whole frame holds it.
    fn reads_whole(&self, inputs: &Inputs, frame: &DecodedFrame, from: i32) -> bool {
        inputs.unchanged(&Since::span(
            frame.branch,
            from,
            frame.hour,
            Some(frame),
            self.reduced,
        ))
    }

    /// Whether the tables a leader's template visibility reads
    /// (`World::decoded_leader_visibility`: its character and the tokens
    /// spelling its template tag) are kept whole.
    fn identity_whole(&self) -> bool {
        [TableId::Characters, TableId::LexicalTokens]
            .into_iter()
            .all(|table| matches!(self.reduced.table(table), TableChange::Same))
    }
}

impl<'a> HourWorld<'a> {
    fn get(&self) -> Result<&World<'a>, String> {
        let world = self
            .world
            .get_or_init(|| self.content.recorded_world(self.frame))
            .as_ref()
            .map_err(Clone::clone)?;
        self.gap()?;
        Ok(world)
    }

    /// Why the hour's core tables are not complete ([`World::core_gap`]).
    fn gap(&self) -> Result<(), String> {
        self.gap
            .get_or_init(|| core_gap(self.frame))
            .clone()
            .map_or(Ok(()), Err)
    }
}

/// A leader's [`LeaderVisibility`], valid while its key holds: the scan's
/// identity epoch of the Characters and LexicalTokens tables it reads, and
/// the leader's character and visibility inputs.
struct VisibilityRead {
    key: (Option<u64>, u32, u8, u8),
    visibility: LeaderVisibility,
    /// Whether its decided verdict was given: refusals are a set, so an
    /// unknown one is noted once.
    noted: bool,
}

/// A leader's last visibility evaluation in its owner's scope, reused while
/// every input it read reads the same value ([`dashboard_text::triggers::inputs`]):
/// the verdict is a function of the owner, the role it decides and those
/// reads.
struct Evaluation {
    owner: u32,
    visibility: LeaderVisibility,
    value: Eval,
    inputs: dashboard_text::triggers::inputs::Inputs,
    /// The last hour whose verdict this evaluation gave.
    hour: i32,
}

use dashboard_text::triggers::inputs::{
    Inputs, ReuseCounts, Since, TableChange, TableChanges, TableRead, check_reuse,
};

/// The hour's rereads of recorded inputs, each key read once however many
/// leaders' evaluations read it.
type Rereads = HashMap<dashboard_text::triggers::inputs::Key, Option<u64>>;

/// The digest `key` reads in the hour's `world`. An evaluation in an
/// owner's scope reads the World and its recorded scopes; a key it cannot
/// read again here is evaluated again.
fn reread(
    rereads: &mut Rereads,
    world: &World<'_>,
    frame: &DecodedFrame,
    key: &dashboard_text::triggers::inputs::Key,
) -> Option<u64> {
    use dashboard_text::triggers::inputs::{Key, digest};
    if let Some(&read) = rereads.get(key) {
        return read;
    }
    let read = match key {
        Key::World(query) => Some(world.reread(query)),
        Key::Scopes(query) => Some(world.recorded().reread(query)),
        Key::ClockSeed => Some(digest(&dashboard_text::scripted_text::clock_seed(frame))),
        Key::Table(_) | Key::Fields(..) | Key::Names(_) | Key::Text(_) | Key::FocusTag(_) => None,
    };
    rereads.insert(key.clone(), read);
    read
}

/// What deciding a leader's visibility reuses: its last evaluation, the
/// hour's rereads and the counts.
struct Reuse<'r> {
    evaluation: &'r mut Option<Evaluation>,
    rereads: &'r mut Rereads,
    counts: &'r mut ReuseCounts,
    /// What changed since the hour before, when it is known.
    since: Option<&'r Since<'r>>,
}

impl Reuse<'_> {
    /// `visibility` decided in the owner's scope: the last evaluation's
    /// verdict while every input it read reads the same, else evaluated.
    /// The hour's World is built only to read an input again or evaluate:
    /// an input whose declared dependence did not change since the hour
    /// before reads the same without it.
    fn decide(
        self,
        hour: &HourWorld<'_>,
        row: &UnitLeaderRow,
        visibility: &LeaderVisibility,
    ) -> Result<Eval, String> {
        let Self {
            evaluation,
            rereads,
            counts,
            since,
        } = self;
        let frame = hour.frame;
        let confirmed = evaluation.as_mut().is_some_and(|last| {
            if last.owner != row.owner || last.visibility != *visibility {
                return false;
            }
            // On a kept frame an input is read again only where it reads
            // what it reads on the whole frame.
            let whole = hour.kept.is_none_or(|kept| {
                kept.reads_whole(&last.inputs, frame, frame.hour.saturating_sub(1))
            });
            last.inputs.confirm_since(frame, since, |key| {
                if !whole {
                    return None;
                }
                let world = hour.get().ok()?;
                reread(rereads, world, frame, key)
            })
        });
        if confirmed {
            counts.reused += 1;
            if !check_reuse() {
                let last = evaluation.as_mut().expect("confirmed evaluation");
                last.hour = frame.hour;
                return Ok(last.value.clone());
            }
        }
        let world = hour.get()?;
        let (value, inputs) = dashboard_text::triggers::inputs::capture(frame, || {
            Evaluator::new(world, None).decide_leader_visibility(row, visibility.clone(), true)
        });
        // An evaluation on a kept frame gives the whole frame's verdict when
        // every value it read is the whole frame's: it then took the same
        // steps and read the same values there. Otherwise the hour is
        // observed on its whole frame.
        if let Some(kept) = hour.kept {
            if check_reuse() {
                kept.block("leaders.reject.check-mode");
            } else if !kept.reads_whole(&inputs, frame, frame.hour) {
                kept.block_inputs(&inputs, frame);
            }
        }
        if confirmed {
            // Check mode: the reused evaluation, run anyway, must not change.
            counts.checked += 1;
            if evaluation.as_ref().is_some_and(|last| last.value != value) {
                counts.mismatched += 1;
                #[cfg(test)]
                panic!(
                    "leader {}: a reused visibility evaluation changed at hour {}",
                    row.id, frame.hour
                );
            }
        } else {
            counts.evaluated += 1;
        }
        *evaluation = Some(Evaluation {
            owner: row.owner,
            visibility: visibility.clone(),
            value: value.clone(),
            inputs,
            hour: frame.hour,
        });
        Ok(value)
    }
}

/// [`visible_reusing`] evaluating in the owner's scope every time.
#[cfg(test)]
fn visible(
    row: &UnitLeaderRow,
    hour: Option<&HourWorld<'_>>,
    read: &mut Option<VisibilityRead>,
    epoch: Option<u64>,
    linked: bool,
) -> Result<bool, Option<String>> {
    visible_reusing(row, hour, read, None, epoch, linked)
}

/// Whether the roster shows a leader; `Err(None)` repeats an unknown already
/// noted for it. `epoch` is `None` when Characters or LexicalTokens is
/// incomplete, whose refusal names the hour. In the owner's scope it decides
/// through `reuse` when given, else evaluates.
fn visible_reusing(
    row: &UnitLeaderRow,
    hour: Option<&HourWorld<'_>>,
    read: &mut Option<VisibilityRead>,
    reuse: Option<Reuse<'_>>,
    epoch: Option<u64>,
    linked: bool,
) -> Result<bool, Option<String>> {
    // The optional role gate checks recorded capture state.
    if row.kind == engine_native::cunit_leader::KIND_ARMY
        && (row.category as u32) < 2
        && row.captured != 0
    {
        return Ok(false);
    }
    if row.visibility_choice == 0 && row.template_visibility == 0 {
        return Ok(true);
    }
    let Some(hour) = hour else {
        return Err(Some("Content is unavailable for leader visibility".into()));
    };
    let key = (
        epoch,
        row.character_id,
        row.visibility_choice,
        row.template_visibility,
    );
    if key.0.is_some() && read.as_ref().is_some_and(|read| read.key == key) {
        // Content and its environment are read once per scan, so once an
        // earlier hour's World read this, only the core tables can refuse.
        hour.gap()?;
    } else {
        // The template's visibility reads the character and its tag's
        // tokens, which a kept frame keeps whole.
        if let Some(kept) = hour.kept
            && !kept.identity_whole()
        {
            kept.block("leaders.reject.identity");
            return Err(None);
        }
        *read = Some(VisibilityRead {
            key,
            visibility: hour.get()?.decoded_leader_visibility(row),
            noted: false,
        });
    }
    let read = read.as_mut().expect("visibility read");
    let value = match &read.visibility {
        LeaderVisibility::Decided(Eval::Unknown(_)) if read.noted => return Err(None),
        LeaderVisibility::Decided(value) => {
            read.noted = true;
            value.clone()
        }
        // A fresh evaluation enters no frame before the owner's scope, where
        // the empty conjunction holds.
        LeaderVisibility::Unconditioned if linked => Eval::True,
        visibility => match reuse {
            Some(reuse) => reuse.decide(hour, row, visibility)?,
            // Evaluated every hour, never on a kept frame.
            None if hour.kept.is_some() => {
                hour.kept
                    .expect("kept frame")
                    .block("leaders.reject.no-reuse");
                return Err(None);
            }
            None => Evaluator::new(hour.get()?, None).decide_leader_visibility(
                row,
                visibility.clone(),
                true,
            ),
        },
    };
    value.decided().map_err(Some)
}

fn reply_bytes_within(reply: &UnitLeadersReply, budget: usize) -> io::Result<usize> {
    dashboard_core::transport::json_len(reply, budget).ok_or_else(|| {
        io::Error::other(
            "Leader reply exceeds its transport budget; select fewer countries or one career",
        )
    })
}

pub struct Content {
    catalogue: Arc<Catalogue>,
    politics: Arc<politics_content::PoliticsContent>,
    leaders: installed_content_core::unit_leader::Leaders,
}

#[cfg(test)]
thread_local! {
    static CONTENT_LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn content_loads() -> usize {
    CONTENT_LOADS.with(std::cell::Cell::get)
}

#[cfg(test)]
thread_local! {
    static PROGRESS_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The leader progress values this thread built rather than carried.
#[cfg(test)]
fn progress_builds() -> usize {
    PROGRESS_BUILDS.with(std::cell::Cell::get)
}

impl Content {
    pub fn for_catalogue(
        source: &Source,
        context: &LoadContext,
        catalogue: Option<Arc<Catalogue>>,
    ) -> io::Result<Option<Self>> {
        catalogue
            .map(|catalogue| {
                let politics =
                    politics_content::recorded_politics_content(source, context.content()?)?;
                Self::load(catalogue, politics)
            })
            .transpose()
    }
    fn load(
        catalogue: Arc<Catalogue>,
        politics: Arc<politics_content::PoliticsContent>,
    ) -> io::Result<Self> {
        #[cfg(test)]
        CONTENT_LOADS.with(|loads| loads.set(loads.get() + 1));
        let leaders =
            installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)?;
        Ok(Self {
            catalogue,
            politics,
            leaders,
        })
    }
}

pub fn check_content(
    catalogue: Arc<Catalogue>,
    politics: Arc<politics_content::PoliticsContent>,
) -> io::Result<Vec<dashboard_core::findings::Finding>> {
    use dashboard_core::findings::{Finding, Kind};
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
        engine_native::cunit_leader::KIND_ARMY,
        engine_native::cunit_leader::KIND_NAVY,
    ] {
        attribute_names(kind, Some(&content.catalogue));
    }
    for effects in content.leaders.traits.iter() {
        let info = trait_info(&effects.name, Some(&content));
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
    holder: Option<(u32, u32)>,
    /// The scan's progress epochs at this read, and the leader's training
    /// rows then (None while their table is incomplete).
    progress_epochs: [u64; 2],
    training: Option<Vec<UnitLeaderTraitExperienceRow>>,
}

/// The identity table rows an hour changed, each table's keys sorted, when
/// every identity table changed by exactly its recorded delta: a leader whose
/// own rows are not among them reads the identity it read the hour before.
struct IdentityChanges {
    characters: Vec<u128>,
    portraits: Vec<u128>,
    tokens: Vec<u128>,
}

impl IdentityChanges {
    fn read(changes: &FrameChanges) -> Option<Self> {
        let keys = |table: TableId| {
            changes.rows(table).map(|(changed, deleted)| {
                let mut keys: Vec<u128> = changed
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied())
                    .collect();
                keys.sort_unstable();
                keys
            })
        };
        Some(Self {
            characters: keys(TableId::Characters)?,
            portraits: keys(TableId::CharacterPortraits)?,
            tokens: keys(TableId::LexicalTokens)?,
        })
    }

    fn has(keys: &[u128], key: Option<u32>) -> bool {
        key.is_some_and(|key| keys.binary_search(&u128::from(key)).is_ok())
    }

    /// Carry `state`'s identity reads from epoch `before`, the hour before's
    /// tables, to this hour's when none of the rows they read changed: the
    /// leader's character, its portrait holder, and the tokens spelling its
    /// name and template tag. Returns how many reads it carried.
    fn renew(
        &self,
        state: &mut LeaderState,
        row: &UnitLeaderRow,
        characters: Option<&[CharacterRow]>,
        before: u64,
        epoch: u64,
        visibility_epoch: Option<u64>,
    ) -> usize {
        if Self::has(&self.characters, Some(row.character_id)) {
            return 0;
        }
        // Unchanged this hour, so the row the earlier reads read.
        let character = characters.and_then(|rows| {
            rows.binary_search_by_key(&row.character_id, |character| character.id)
                .ok()
                .map(|at| &rows[at])
        });
        let mut renewed = 0;
        if let Some(read) = &mut state.visibility
            && visibility_epoch.is_some()
            && read.key
                == (
                    Some(before),
                    row.character_id,
                    row.visibility_choice,
                    row.template_visibility,
                )
            && !Self::has(
                &self.tokens,
                character
                    .filter(|character| character.template_present != 0)
                    .map(|character| character.template_tag),
            )
        {
            read.key.0 = visibility_epoch;
            renewed += 1;
        }
        if let Some(last) = &mut state.last_read
            && last.person_epoch == before
            && last.row.character_id == row.character_id
            && last.row.kind == row.kind
            && !Self::has(&self.portraits, Some(row.character_id))
            && !Self::has(
                &self.tokens,
                character.and_then(|character| u32::try_from(character.name_token_id).ok()),
            )
        {
            last.person_epoch = epoch;
            renewed += 1;
        }
        renewed
    }
}

#[derive(Serialize, Deserialize)]
struct LeaderState {
    reference: u64,
    career: LeaderCareer,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    command_indices: HashMap<u64, usize>,
    #[serde(skip)]
    last_read: Option<LastRead>,
    #[serde(skip)]
    visibility: Option<VisibilityRead>,
    /// Runtime state: empty on resume, when the next read evaluates.
    #[serde(skip)]
    evaluation: Option<Evaluation>,
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
                battle_starts: None,
                battle_start_read_hours: None,
                unknown_battle_starts: None,
                battle_won: None,
                battle_result_hours: None,
                battle_result_read_hours: None,
                fought_hours: None,
                won_hours: None,
                lost_hours: None,
                partial_from: None,
                at_hour: None,
            },
            command_indices: HashMap::new(),
            last_read: None,
            visibility: None,
            evaluation: None,
            visible_hour: None,
            admitted: false,
            last_run: None,
            last_known_run: None,
        }
    }
}

// Hourly writes share one allocation and adjacent cache lines. Materialize only
// admitted careers after the scan, retaining the complete transfer history.
#[derive(Clone, Serialize, Deserialize)]
struct StoredRun {
    run: UnitLeaderRun,
    previous: Option<usize>,
    emitted: bool,
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
        emitted: false,
    });
    state.last_run = Some(index);
    if known {
        state.last_known_run = Some(index);
    }
}

/// The training rows of the leader `reference`, which lead their key.
fn training_rows(
    rows: &[UnitLeaderTraitExperienceRow],
    reference: u64,
) -> &[UnitLeaderTraitExperienceRow] {
    let first = rows.partition_point(|p| p.leader_reference < reference);
    let last = rows.partition_point(|p| p.leader_reference <= reference);
    &rows[first..last]
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

/// The tables a scan decodes: its own, and the trigger kit leader visibility
/// reads (`triggers::RECORDED_TABLES`), so no recorded operand is discarded.
const LEADER_TABLES: &[TableId] = &[
    TableId::UnitLeaders,
    TableId::Characters,
    TableId::LexicalTokens,
    TableId::CharacterPortraits,
    TableId::UnitLeaderTraits,
    TableId::UnitLeaderTraitExperience,
    TableId::LandCombatDetails,
    TableId::NavalCombatDetails,
    TableId::LandCombatResults,
    TableId::NavalResultSources,
    TableId::NavalResults,
    TableId::NavalCombatResultDetails,
];
const COMMAND_TABLES: &[TableId] = &[
    TableId::OrdersGroups,
    TableId::OrdersGroupPlans,
    TableId::NavalMissionTaskForces,
    TableId::TaskForces,
    TableId::Fleets,
];

fn scan_tables(career: bool, choices: bool) -> Vec<TableId> {
    let mut selected = LEADER_TABLES.to_vec();
    selected.extend([
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::Provinces,
        TableId::ScriptFlags,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::CountryWars,
        TableId::CharacterRoleTraits,
        TableId::CharacterOperatives,
        TableId::CountryRetiredCharacters,
    ]);
    if career {
        selected.extend_from_slice(COMMAND_TABLES);
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
    selected.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
    selected.sort_unstable();
    selected.dedup();
    selected
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct NavalSource {
    combat: u64,
    game_hour: i32,
}

#[derive(Default, Serialize, Deserialize)]
#[allow(clippy::type_complexity)]
pub(crate) struct Scan {
    reply: UnitLeadersReply,
    frontier: Option<i32>,
    materialized: Option<i32>,
    #[serde(skip)]
    battle_frontier: Option<i32>,
    #[cfg(test)]
    #[serde(skip)]
    eager_battles: bool,
    #[serde(skip)]
    read_mask: Option<u32>,
    #[serde(skip)]
    scoped_visibility: bool,
    #[serde(skip)]
    active_commands: Vec<(usize, usize)>,
    #[serde(skip)]
    commands_continuous: bool,
    #[serde(skip)]
    holders: Option<(
        dashboard_text::metadata::CountryKeys,
        BTreeMap<String, (u32, u32)>,
    )>,
    #[serde(skip)]
    pending_bytes: usize,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    /// Whether visibility evaluations are reused while their inputs read the
    /// same; preparation sets it, the per-hour oracle evaluates every hour.
    #[serde(skip)]
    reuse_evaluations: bool,
    #[serde(skip)]
    reuse: ReuseCounts,
    /// The leaders whose last read's verdict an evaluation gave.
    #[serde(skip)]
    evaluated: Vec<usize>,
    /// The core-table refusal the last read's verdicts read, `None` when
    /// none read it.
    #[serde(skip)]
    read_gap: Option<Option<String>>,
    /// Identity reads carried across changed identity tables.
    #[cfg(test)]
    #[serde(skip)]
    renewed: usize,

    #[serde(skip)]
    leader_indices: HashMap<u64, usize>,
    leaders: Vec<LeaderState>,
    stored_runs: Vec<StoredRun>,
    #[serde(skip)]
    row_indices: Vec<Option<(u64, usize)>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    people: HashMap<String, u32>,
    #[serde(skip)]
    last_people: HashMap<(u32, u8), PersonRead>,
    /// How often the identity, owner and trait tables' views changed at the
    /// hours read: runtime only, as the caches they key are, since they
    /// count reads, which differ where hours are read differently but give
    /// the same careers.
    #[serde(skip)]
    identity_epoch: u64,
    #[serde(skip)]
    identity_tables: Option<[ViewIdentity; 3]>,
    /// Saved as the projection it was built from, so a restored scan keeps
    /// its cached owners exactly when a continued one would.
    #[serde(with = "owner_projection", default)]
    owner_tags: LinkTags,
    #[serde(skip)]
    owner_epoch: u64,
    #[serde(skip)]
    trait_table: Option<(ViewIdentity, bool)>,
    #[serde(skip)]
    trait_epoch: u64,
    /// The training rows' and the tokens' views at the last read, and how
    /// often each changed at the hours read, as `identity_epoch` counts.
    #[serde(skip)]
    progress_tables: Option<[ViewIdentity; 2]>,
    #[serde(skip)]
    progress_epochs: [u64; 2],
    #[serde(skip)]
    formation_names: HashMap<u64, (Option<Vec<u8>>, Option<String>)>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    roles: HashMap<String, u32>,
    #[serde(skip)]
    rank_names: HashMap<i32, Option<String>>,
    #[serde(skip)]
    skill_ceilings: HashMap<(i32, i32), Option<i32>>,
    refusals: catalogue::Refusals,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    trait_tokens: HashMap<String, u32>,
    #[serde(skip)]
    trait_sets: HashMap<Vec<u32>, Arc<[u32]>>,
    token_list: Vec<String>,
    #[serde(skip)]
    battle_indices: HashMap<(u64, u8, u64), usize>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    battle_hours: BattleHours,
    #[serde(skip)]
    outcomes: BattleOutcomes,
    #[serde(skip)]
    naval_sources: BTreeMap<u64, NavalSource>,
    results_recorded: bool,
    missing_battles: [Option<i32>; 2],
    /// Each kind's combats at the last read, sorted.
    previous_combats: [Option<(i32, Vec<u64>)>; 2],
    /// Each kind's combats with their sides' commanders at the last read,
    /// None when that kind's battles were not read: all the battle step
    /// reads of the combat details, whose other operands move every combat
    /// hour.
    #[serde(skip)]
    combats: [Option<Commanders>; 2],
    /// Whether the last read reached the leaders (their table was read and
    /// complete), and so the battles.
    #[serde(skip)]
    walked: bool,
}

/// One kind's combats at an hour, each with its sides' commanders.
pub(crate) type Commanders = Vec<(u64, [u64; 2])>;

/// The land combats' commanders.
pub(crate) fn land_commanders(rows: &[LandCombatDetailsRow]) -> Commanders {
    rows.iter()
        .map(|row| (row.combat, row.sides.each_ref().map(|side| side.commander)))
        .collect()
}

/// The naval combats' commanders.
pub(crate) fn naval_commanders(rows: &[NavalCombatDetailsRow]) -> Commanders {
    rows.iter()
        .map(|row| (row.combat, row.sides.each_ref().map(|side| side.commander)))
        .collect()
}

/// Whether `held` are the commanders of `rows` (None for battles not read).
fn same_commanders<R>(
    held: Option<&Commanders>,
    rows: Option<&[R]>,
    commanders: impl Fn(&R) -> (u64, [u64; 2]),
) -> bool {
    match (held, rows) {
        (None, None) => true,
        (Some(held), Some(rows)) => {
            held.len() == rows.len()
                && held
                    .iter()
                    .zip(rows)
                    .all(|(held, row)| *held == commanders(row))
        }
        _ => false,
    }
}

/// The battle tables an hour reads (complete and read at the hour): what the
/// battle step joins and what each leader's partial coverage depends on.
pub(crate) fn battle_tables(
    format: FormatVersion,
    decoded: &DecodedFrame,
) -> (
    Option<&[LandCombatDetailsRow]>,
    Option<&[NavalCombatDetailsRow]>,
) {
    let hour = decoded.hour;
    let land = decoded
        .complete::<tables::LandCombatDetails>()
        .ok()
        .filter(|_| slot_read(format, &decoded.metrics, TableId::LandCombatDetails, hour));
    let navy = decoded
        .complete::<tables::NavalCombatDetails>()
        .ok()
        .filter(|_| slot_read(format, &decoded.metrics, TableId::NavalCombatDetails, hour));
    (land, navy)
}

/// What the scan reads of each leader table, besides its row keys and
/// availability: an hour at which none of these changed reads what the
/// hour before read.
pub(crate) struct Reads {
    /// Every leader, character, portrait, token, assigned trait and training field.
    pub leaders: Vec<TableRead>,
    /// The leaders alone, whose visibility inputs decide whether any leader
    /// is evaluated in its owner's scope.
    pub unit_leaders: TableRead,
    /// The owners and holders: the countries' slots and tags and the links,
    /// which the holders' comparison reads ([`Scan::refresh_holders`]).
    pub holders: [TableRead; 2],
    /// Commands: the formation's leader, name and kind, every plan and fleet
    /// field, a mission's leader and a task force's fleet. Task forces'
    /// positions and routes and missions' fuel, ranges and replenishment
    /// move every hour while fleets sail; nothing reads them.
    pub commands: Vec<TableRead>,
    /// Results, read whole (event rows).
    pub results: Vec<TableRead>,
    /// The combat details' sides, of which the battle step reads the
    /// commanders ([`Commanders`]).
    pub combats: [TableRead; 2],
}

pub(crate) fn reads() -> &'static Reads {
    static READS: std::sync::OnceLock<Reads> = std::sync::OnceLock::new();
    READS.get_or_init(|| {
        let unit_leaders = TableRead {
            fields: full_mask(TableId::UnitLeaders.schema().len()),
            ..TableRead::keys(TableId::UnitLeaders)
        };
        let holders = [
            TableRead::fields(TableId::Countries, &["canonical_slot", "tag"]),
            TableRead::all(TableId::CountryLinks),
        ];
        let leaders = vec![
            unit_leaders,
            TableRead::all(TableId::Characters),
            TableRead::all(TableId::CharacterPortraits),
            TableRead::all(TableId::LexicalTokens),
            TableRead::all(TableId::UnitLeaderTraits),
            TableRead::all(TableId::UnitLeaderTraitExperience),
        ];
        Reads {
            leaders,
            unit_leaders,
            holders,
            commands: vec![
                TableRead::fields(TableId::OrdersGroups, &["leader", "name", "is_army_group"]),
                TableRead::all(TableId::OrdersGroupPlans),
                TableRead::all(TableId::Fleets),
                TableRead::fields(TableId::NavalMissionTaskForces, &["leader"]),
                TableRead::fields(TableId::TaskForces, &["fleet"]),
            ],
            results: [
                TableId::LandCombatResults,
                TableId::NavalResultSources,
                TableId::NavalResults,
                TableId::NavalCombatResultDetails,
            ]
            .map(TableRead::all)
            .to_vec(),
            combats: [
                TableRead::fields(TableId::LandCombatDetails, &["sides"]),
                TableRead::fields(TableId::NavalCombatDetails, &["sides"]),
            ],
        }
    })
}

/// The scan's read mask of an hour. Training is consumed from its held daily
/// sample on every hour; its availability and values have ordinary change proofs.
pub(crate) fn read_mask(
    format: FormatVersion,
    metrics: &replay_frame::frame::Metrics,
    hour: i32,
    career: bool,
) -> u32 {
    LEADER_TABLES
        .iter()
        .chain(COMMAND_TABLES.iter().filter(|_| career))
        .enumerate()
        .fold(0u32, |mask, (index, &table)| {
            mask | (u32::from(
                table == TableId::UnitLeaderTraitExperience
                    || slot_read(format, metrics, table, hour),
            ) << index)
        })
}

impl Scan {
    fn recount_pending(&mut self) {
        self.pending_bytes = self.stored_runs.len() * std::mem::size_of::<StoredRun>()
            + self
                .battle_hours
                .values()
                .map(|rows| rows.len() * std::mem::size_of::<(i32, i32, bool, u8, u64, u8)>())
                .sum::<usize>()
            + self
                .leaders
                .iter()
                .flat_map(|leader| &leader.career.commands)
                .map(|command| {
                    std::mem::size_of::<LeaderCommand>()
                        + command.name_unknown.len() * std::mem::size_of::<HourSpan>()
                })
                .sum::<usize>();
    }
    /// Read the admission holders again when the countries' slots and tags
    /// or the links may have changed (`moved`); whether they changed.
    fn refresh_holders(
        &mut self,
        frame: &DecodedFrame,
        moved: &dyn Fn(&TableRead) -> bool,
    ) -> bool {
        let [countries_read, links_read] = reads().holders;
        let links_moved = moved(&links_read);
        if self.holders.is_some() && !links_moved && !moved(&countries_read) {
            return false;
        }
        let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            return self.holders.take().is_some();
        };
        if !links_moved && let Some((keys, _)) = &self.holders {
            #[cfg(test)]
            {
                self.work.row_visits += countries.len();
            }
            if keys.same(countries) {
                return false;
            }
        }
        #[cfg(test)]
        {
            self.work.row_visits += countries.len() * 2;
        }
        let holders = dashboard_text::metadata::tagged_countries(countries, links)
            .into_iter()
            .filter_map(|(tag, country)| match country {
                dashboard_text::metadata::TaggedCountry::Found(country, slot) => {
                    std::str::from_utf8(tag)
                        .ok()
                        .map(|tag| (tag.to_owned(), (slot, country.id)))
                }
                _ => None,
            })
            .collect();
        let changed = links_moved
            || self
                .holders
                .as_ref()
                .is_none_or(|(_, before)| before != &holders);
        self.holders = Some((
            dashboard_text::metadata::CountryKeys::read(countries),
            holders,
        ));
        changed
    }
    /// Only authenticated consecutive observations can carry an open career.
    fn advance(&mut self, hour: i32) {
        debug_assert!(
            self.frontier
                .is_some_and(|last| last.checked_add(1) == Some(hour))
        );
        self.frontier = Some(hour);
    }

    /// Current bindings only; closed leaders and commands are never visited here.
    fn seal(&mut self) {
        let (Some(at), Some(through)) = (self.materialized, self.frontier) else {
            return;
        };
        if at == through {
            return;
        }
        if let Some(observed) = self.reply.observed.last_mut()
            && observed.last_hour == at
        {
            observed.last_hour = through;
        }
        for (_, index) in self.row_indices.iter().flatten() {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let state = &mut self.leaders[*index];
            if state.visible_hour == Some(at) {
                state.visible_hour = Some(through);
                if let Some(run) = state.last_run
                    && self.stored_runs[run].run.last_hour == at
                {
                    self.stored_runs[run].run.last_hour = through;
                }
            }
        }
        for &(leader, command) in &self.active_commands {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let command = &mut self.leaders[leader].career.commands[command];
            if command.last_hour == at {
                command.last_hour = through;
                if let Some(unknown) = command.name_unknown.last_mut()
                    && unknown.last_hour == at
                {
                    unknown.last_hour = through;
                }
            }
        }
        for missing in self.missing_battles.iter_mut().flatten() {
            if *missing == at {
                *missing = through;
            }
        }
        let battle_at = self.battle_frontier.take().unwrap_or(at);
        for (hour, _) in self.previous_combats.iter_mut().flatten() {
            if *hour == battle_at {
                *hour = through;
            }
        }
        self.materialized = Some(through);
    }
    pub(crate) fn new(replay: &Replay, branch: u64) -> io::Result<Self> {
        let start = replay
            .index
            .iter()
            .filter(|b| b.branch == branch)
            .map(|b| b.first_hour)
            .min()
            .ok_or_else(|| dashboard_core::invalid("Branch has no recorded frames"))?;
        let params = UnitLeadersParams {
            branch_id: branch.to_string(),
            selection: LeaderSelection::All,
            view: LeaderView::Career,
            leaders: LeaderKinds::All,
        };
        let reply = UnitLeadersReply {
            branch_id: params.branch_id.clone(),
            first_hour: start,
            leaders: Vec::new(),
            people: Vec::new(),
            roles: Vec::new(),
            traits: Vec::new(),
            observed: Vec::new(),
            selection_tags: None,
            preparing: Vec::new(),
            results_missing: RESULTS_MISSING.into(),
            navy_commands_missing: if replay.format.field_gaps(TableId::Fleets).next().is_none() {
                String::new()
            } else {
                NAVY_COMMANDS_MISSING.into()
            },
            warnings: Vec::new(),
        };

        Ok(Self {
            reply,
            ..Default::default()
        })
    }
    #[cfg(test)]
    fn observe(
        &mut self,
        decoded: &DecodedFrame,
        replay: &Replay,
        params: &UnitLeadersParams,
        content: Option<&Content>,
        changes: &FrameChanges,
        admit: &mut impl FnMut(u64, &LeaderRole, Option<(u32, u32)>),
    ) -> io::Result<()> {
        self.observe_changes(
            decoded, replay, params, content, changes, changes, None, admit,
        )
        .map(drop)
    }

    /// [`Self::observe`] with what changed since the hour before as `tables`
    /// tells it (`changes` keep the hour's reads and table-level facts), on
    /// a kept frame when `kept` says how it differs from the hour's whole
    /// frame. An hour at which nothing the scan reads changed carries its
    /// careers on; one at which only combats and results changed takes the
    /// battle step alone ([`Self::observe_battles`]); any other reads every
    /// leader. Returns false, having changed nothing the hour's observation
    /// on its whole frame would not, when a kept frame cannot give the
    /// hour's visibility verdicts: the hour must be observed on its whole
    /// frame.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_changes(
        &mut self,
        decoded: &DecodedFrame,
        replay: &Replay,
        params: &UnitLeadersParams,
        content: Option<&Content>,
        changes: &FrameChanges,
        tables: &dyn TableChanges,
        kept: Option<&Kept<'_>>,
        admit: &mut impl FnMut(u64, &LeaderRole, Option<(u32, u32)>),
    ) -> io::Result<bool> {
        // Check mode reads every reused input again, which needs the whole
        // frame.
        if kept.is_some() && content.is_some() && check_reuse() {
            block::rejected("leaders.reject.check-mode");
            return Ok(false);
        }
        let cpu = ::replay_frame::cpu::scope_phase("leaders:carry_proof");
        let since = Since::hour_before(decoded, tables);
        let moved = |read: &TableRead| since.as_ref().is_none_or(|since| since.changed(read));
        let holders_changed = self.refresh_holders(decoded, &moved);
        let career = params.view == LeaderView::Career;
        let inputs = LEADER_TABLES
            .iter()
            .chain(COMMAND_TABLES.iter().filter(|_| career));
        let mask = inputs.enumerate().fold(0u32, |mask, (index, table)| {
            mask | (u32::from(
                *table == TableId::UnitLeaderTraitExperience || changes.slot_read[table.index()],
            ) << index)
        });
        let reads = reads();
        if moved(&reads.unit_leaders) {
            self.scoped_visibility = decoded.complete::<tables::UnitLeaders>().is_ok_and(|rows| {
                #[cfg(test)]
                {
                    self.work.row_visits += rows.len();
                }
                rows.iter()
                    .any(|row| row.visibility_choice != 0 || row.template_visibility != 0)
            });
        }
        let steady = self.commands_continuous
            && !holders_changed
            && self
                .frontier
                .is_some_and(|last| last.checked_add(1) == Some(decoded.hour))
            && self.read_mask == Some(mask)
            && !reads.leaders.iter().any(&moved)
            && !(career && reads.commands.iter().any(&moved));
        if steady {
            let (land, navy) = battle_tables(replay.format, decoded);
            let [land_read, navy_read] = &reads.combats;
            // Only the commanders and the combats themselves are read of the
            // combat details.
            let combats = moved(land_read)
                && !same_commanders(self.combats[0].as_ref(), land, |row| {
                    (row.combat, row.sides.each_ref().map(|side| side.commander))
                })
                || moved(navy_read)
                    && !same_commanders(self.combats[1].as_ref(), navy, |row| {
                        (row.combat, row.sides.each_ref().map(|side| side.commander))
                    });
            let results = reads.results.iter().any(&moved);
            // Whether each kind's battles are read as at the last read: a
            // change of their availability changes the coverage, the
            // missing battle reads and partial careers, which only the
            // leader walk records.
            let available = self.combats[0].is_some() == land.is_some()
                && self.combats[1].is_some() == navy.is_some();
            if !self.scoped_visibility
                || content.is_none_or(|content| {
                    self.visibility_carries(decoded, content, since.as_ref(), kept)
                })
            {
                if !combats && !results {
                    self.advance(decoded.hour);
                    return Ok(true);
                }
                if career && available {
                    self.observe_battles(decoded, replay, land, navy);
                    return Ok(true);
                }
            }
        }
        drop(cpu);
        self.seal();
        self.frontier = Some(decoded.hour);
        self.materialized = Some(decoded.hour);
        self.read_mask = Some(mask);
        self.observe_rows(
            decoded,
            replay,
            params,
            content,
            Some(changes),
            since.as_ref(),
            kept,
            admit,
        )
    }

    /// The hour's step when only its combats and results changed since the
    /// hour before, whose leaders, commands, holders and visibility verdicts
    /// it reads as that hour did: the careers, commands, coverage and missing
    /// battle reads go on through the hour, as the leader walk would carry
    /// them, and the results and battles are read as it reads them, each
    /// side's commander joined while visible at the hour before (so at this
    /// one). After a read that did not reach the leaders, the walk would not
    /// reach them either: only the results are read.
    fn observe_battles(
        &mut self,
        decoded: &DecodedFrame,
        replay: &Replay,
        land: Option<&[LandCombatDetailsRow]>,
        navy: Option<&[NavalCombatDetailsRow]>,
    ) {
        let _cpu = ::replay_frame::cpu::scope_phase("leaders:battles");
        let hour = decoded.hour;
        #[cfg(not(test))]
        let eager = false;
        #[cfg(test)]
        let eager = self.eager_battles;
        if eager {
            self.seal();
        }
        let visible_at = self.materialized.unwrap_or(hour.saturating_sub(1));
        // Only battle continuity dates are read here. Career endpoints and
        // visibility witnesses still name the same unchanged leader walk.
        let battle_at = self.battle_frontier.unwrap_or(visible_at);
        for (at, _) in self.previous_combats.iter_mut().flatten() {
            if *at == battle_at {
                *at = hour.saturating_sub(1);
            }
        }
        let visits = record_results(
            replay.format,
            decoded,
            &mut self.outcomes,
            &mut self.naval_sources,
            &mut self.results_recorded,
            &mut self.refusals,
        );
        #[cfg(test)]
        {
            self.work.row_visits += visits;
        }
        #[cfg(not(test))]
        let _ = visits;
        if self.walked {
            let visits = record_battles(
                replay.format,
                decoded,
                land,
                navy,
                visible_at,
                &self.leaders,
                &self.leader_indices,
                &self.naval_sources,
                &mut self.battle_hours,
                &mut self.battle_indices,
                &mut self.previous_combats,
                &mut self.pending_bytes,
            );
            #[cfg(test)]
            {
                self.work.row_visits += visits;
            }
            #[cfg(not(test))]
            let _ = visits;
        }
        self.combats = [land.map(land_commanders), navy.map(naval_commanders)];
        // The careers, commands and coverage of the hour before go on
        // through this one; this hour's combat reads are already its own.
        self.frontier = Some(hour);
        if self.walked {
            self.battle_frontier = Some(hour);
        }
        if eager {
            self.seal();
        }
    }

    /// Carry the scan through the quiet hours `first..=last` that follow its
    /// last read, at which `tables` says what may have changed since the
    /// hour before `first` and `rows` holds the rows of every table they
    /// leave unchanged: each such hour would carry it as [`Self::observe`]
    /// does (no input the scan reads changed, its reads are the last read's,
    /// and every visibility verdict reads the same inputs). False, having
    /// changed nothing, when one would not.
    pub(crate) fn advance_quiet(
        &mut self,
        first: i32,
        last: i32,
        tables: &dyn TableChanges,
        rows: &DecodedFrame,
        content: Option<&Content>,
    ) -> bool {
        let Some(from) = first.checked_sub(1) else {
            block::rejected("leaders.reject.quiet-first-hour");
            return false;
        };
        if last < first {
            block::rejected("leaders.reject.quiet-order");
            return false;
        }
        if !self.commands_continuous {
            block::rejected("leaders.reject.quiet-commands");
            return false;
        }
        if self.frontier != Some(from) {
            block::rejected("leaders.reject.quiet-frontier");
            return false;
        }
        if self.read_mask.is_none() {
            block::rejected("leaders.reject.quiet-read-mask");
            return false;
        }
        let since = Since::span(rows.branch, from, last, Some(rows), tables);
        let reads = reads();
        if let Some(read) = reads
            .leaders
            .iter()
            .chain(&reads.holders)
            .chain(&reads.commands)
            .chain(&reads.results)
            .find(|read| since.changed(read))
        {
            block::rejected("leaders.reject.quiet-tables");
            rejected_table(read.table);
            return false;
        }
        if self.scoped_visibility && content.is_some() {
            // As `visibility_carries` decides at each hour.
            if !self.reuse_evaluations {
                block::rejected("leaders.reject.quiet-no-reuse");
                return false;
            }
            let gap = core_gap(rows);
            if self.read_gap.as_ref().is_some_and(|read| gap != *read) {
                block::rejected("leaders.reject.quiet-core-changed");
                return false;
            }
            if !self.evaluated.is_empty() {
                let Some(content) = content else {
                    block::rejected("leaders.reject.quiet-no-content");
                    return false;
                };
                if content.catalogue.scripted_content().is_err() {
                    block::rejected("leaders.reject.quiet-content");
                    return false;
                }
                if gap.is_some() {
                    block::rejected("leaders.reject.quiet-core-gap");
                    return false;
                }
                let leaders = &self.leaders;
                for &index in &self.evaluated {
                    let Some(evaluation) = &leaders[index].evaluation else {
                        block::rejected("leaders.reject.quiet-no-evaluation");
                        return false;
                    };
                    if !evaluation.inputs.unchanged(&since) {
                        block::rejected("leaders.reject.quiet-inputs");
                        rejected_inputs(&evaluation.inputs, &since);
                        return false;
                    }
                }
                for &index in &self.evaluated {
                    if let Some(evaluation) = &mut self.leaders[index].evaluation {
                        evaluation.inputs.advance(&since);
                    }
                }
                let hours = u64::try_from(i64::from(last) - i64::from(first) + 1).unwrap_or(0);
                self.reuse.reused += self.evaluated.len() as u64 * hours;
            }
        }
        self.frontier = Some(last);
        true
    }

    /// Whether every visibility verdict of the last read holds this hour,
    /// whose leader inputs are unchanged: the core tables refuse as they did,
    /// and every evaluation that gave a verdict reads the same inputs. On a
    /// kept frame an input is read again only where it reads what it reads
    /// on the whole frame.
    fn visibility_carries(
        &mut self,
        decoded: &DecodedFrame,
        content: &Content,
        since: Option<&Since<'_>>,
        kept: Option<&Kept<'_>>,
    ) -> bool {
        if !self.reuse_evaluations {
            return false;
        }
        if self
            .read_gap
            .as_ref()
            .is_some_and(|gap| core_gap(decoded) != *gap)
        {
            return false;
        }
        if self.evaluated.is_empty() {
            return true;
        }
        let Ok(scripted) = content.catalogue.scripted_content() else {
            return false;
        };
        let hour = HourWorld {
            content: scripted,
            frame: decoded,
            world: OnceCell::new(),
            gap: OnceCell::new(),
            kept,
        };
        if hour.gap().is_err() {
            return false;
        }
        // Inputs whose declared dependence did not change since the hour
        // before read the same without the hour's World.
        let mut rereads = Rereads::new();
        let leaders = &mut self.leaders;
        let carried = self.evaluated.iter().all(|&index| {
            leaders[index]
                .evaluation
                .as_mut()
                .is_some_and(|evaluation| {
                    let whole = kept.is_none_or(|kept| {
                        kept.reads_whole(
                            &evaluation.inputs,
                            decoded,
                            decoded.hour.saturating_sub(1),
                        )
                    });
                    evaluation.inputs.confirm_since(decoded, since, |key| {
                        if !whole {
                            return None;
                        }
                        let world = hour.get().ok()?;
                        reread(&mut rereads, world, decoded, key)
                    })
                })
        });
        if carried {
            self.reuse.reused += self.evaluated.len() as u64;
        }
        carried
    }

    /// The per-hour oracle's step: every hour read, every evaluation run.
    #[cfg(any(test, feature = "test-support"))]
    fn observe_read(
        &mut self,
        decoded: &DecodedFrame,
        replay: &Replay,
        params: &UnitLeadersParams,
        content: Option<&Content>,
        admit: &mut impl FnMut(u64, &LeaderRole, Option<(u32, u32)>),
    ) -> io::Result<()> {
        self.observe_rows(decoded, replay, params, content, None, None, None, admit)
            .map(drop)
    }

    /// The per-hour oracle and changed-frame preparation share this exact
    /// step; `changes` lets a leader whose own identity rows did not change
    /// keep its identity reads, and `since` (what changed since the hour
    /// before) an evaluation whose inputs' declared dependence did not change
    /// keep its verdict without the hour's World. Every leader's visibility
    /// is decided before any career changes: on a kept frame (`kept`) that
    /// cannot give a verdict, the step returns false there, having changed
    /// only what observing the hour on its whole frame changes the same way
    /// (caches, read hours, holders, refusal sets and new leaders' states).
    #[allow(clippy::too_many_arguments)]
    fn observe_rows(
        &mut self,
        decoded: &DecodedFrame,
        replay: &Replay,
        params: &UnitLeadersParams,
        content: Option<&Content>,
        changes: Option<&FrameChanges>,
        since: Option<&Since<'_>>,
        kept: Option<&Kept<'_>>,
        admit: &mut impl FnMut(u64, &LeaderRole, Option<(u32, u32)>),
    ) -> io::Result<bool> {
        let _cpu = ::replay_frame::cpu::scope_phase("leaders:rows");
        self.active_commands.clear();
        self.commands_continuous = true;
        self.evaluated.clear();
        self.read_gap = None;
        let reusing = self.reuse_evaluations;
        let start = self.reply.first_hour;
        let career = params.view == LeaderView::Career;
        let choices = params.view == LeaderView::Choices;
        let requested: Option<BTreeSet<&str>> = match &params.selection {
            LeaderSelection::Countries { tags } => Some(tags.iter().map(String::as_str).collect()),
            _ => None,
        };
        let only = match &params.selection {
            LeaderSelection::Leader { reference } => {
                Some(reference.parse::<u64>().expect("validated leader"))
            }
            _ => None,
        };
        let Self {
            reply,
            leader_indices,
            leaders,
            stored_runs,
            row_indices,
            people,
            last_people,
            identity_epoch,
            identity_tables,
            owner_tags,
            owner_epoch,
            trait_table,
            trait_epoch,
            progress_tables,
            progress_epochs,
            formation_names,
            roles,
            rank_names,
            skill_ceilings,
            refusals,
            trait_tokens,
            trait_sets,
            token_list,
            battle_indices,
            battle_hours,
            outcomes,
            naval_sources,
            results_recorded,
            missing_battles,
            previous_combats,
            combats,
            walked,
            active_commands,
            commands_continuous,
            holders: admission_holders,
            scoped_visibility,
            frontier,
            pending_bytes,
            reuse: reuse_counts,
            evaluated,
            read_gap,
            #[cfg(test)]
            work,
            #[cfg(test)]
                renewed: renewed_reads,
            ..
        } = self;
        let hour = decoded.hour;
        let visits = record_results(
            replay.format,
            decoded,
            outcomes,
            naval_sources,
            results_recorded,
            refusals,
        );
        #[cfg(test)]
        {
            work.row_visits += visits;
        }
        #[cfg(not(test))]
        let _ = visits;
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
        .map(|t| &decoded.tables[t.index()]);
        let identity_before = *identity_epoch;
        if identity_tables.as_ref().is_none_or(|last| {
            last.iter()
                .zip(now_identity)
                .any(|(last, now)| !last.is(now))
        }) {
            *identity_epoch += 1;
            *identity_tables = Some(now_identity.map(SharedView::identity));
        }
        let visibility_epoch =
            (characters.is_some() && lexical.is_some()).then_some(*identity_epoch);
        // A changed identity table changes few rows: a leader whose own rows
        // are not among them keeps the identity it read the hour before.
        let identity_changes = changes
            .filter(|_| *identity_epoch != identity_before)
            .and_then(IdentityChanges::read);
        // The hour's rereads of recorded inputs, shared by every evaluation.
        let mut rereads = Rereads::new();
        let (land, navy) = battle_tables(replay.format, decoded);
        *combats = [land.map(land_commanders), navy.map(naval_commanders)];
        for (i, known) in [land.is_some(), navy.is_some()].into_iter().enumerate() {
            if !known {
                missing_battles[i] = Some(hour);
            }
        }
        let land_commands = career
            && slot_read(replay.format, &decoded.metrics, TableId::OrdersGroups, hour)
            && decoded.complete::<tables::OrdersGroups>().is_ok();
        let fleet_rows = career
            .then(|| decoded.complete::<tables::Fleets>().ok())
            .flatten()
            .filter(|_| slot_read(replay.format, &decoded.metrics, TableId::Fleets, hour));
        let fleet_commands = fleet_rows.filter(|rows| {
            rows.iter()
                .all(|row| !matches!(row.leader, Recorded::NotRecorded))
        });
        let mut naval_commands = fleet_commands.is_some()
            || career
                && slot_read(
                    replay.format,
                    &decoded.metrics,
                    TableId::NavalMissionTaskForces,
                    hour,
                )
                && slot_read(replay.format, &decoded.metrics, TableId::TaskForces, hour)
                && decoded.complete::<tables::NavalMissionTaskForces>().is_ok()
                && decoded.complete::<tables::TaskForces>().is_ok();
        if !slot_read(replay.format, &decoded.metrics, TableId::UnitLeaders, hour) {
            observe(
                &mut reply.observed,
                hour,
                false,
                land.is_some(),
                navy.is_some(),
                land_commands,
                naval_commands,
            );
            *walked = false;
            return Ok(true);
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
            *walked = false;
            return Ok(true);
        };
        if stored_runs.capacity() == 0 && !rows.is_empty() {
            let count = if only.is_some() { 1 } else { rows.len() };
            stored_runs.reserve(count);
        }
        // Owner slots and tags read only the links and each country's
        // slot and tag, which LinkTags compares; balances never bump this.
        if owner_tags.update(decoded)? {
            *owner_epoch += 1;
        }
        let needs_visibility = if frontier.is_some() {
            *scoped_visibility
        } else {
            rows.iter()
                .any(|row| row.visibility_choice != 0 || row.template_visibility != 0)
        };
        let world = content.filter(|_| needs_visibility).and_then(|c| {
            match c.catalogue.scripted_content() {
                Ok(scripted) => Some(HourWorld {
                    content: scripted,
                    frame: decoded,
                    world: OnceCell::new(),
                    gap: OnceCell::new(),
                    kept,
                }),
                Err(reason) => {
                    refusals.note(reason, "leader visibility content");
                    None
                }
            }
        });
        let traits = decoded
            .complete::<tables::UnitLeaderTraits>()
            .ok()
            .filter(|_| {
                slot_read(
                    replay.format,
                    &decoded.metrics,
                    TableId::UnitLeaderTraits,
                    hour,
                )
            });
        let progress_rows = decoded.complete::<tables::UnitLeaderTraitExperience>().ok();
        let progress_tokens = decoded.complete::<tables::LexicalTokens>().ok();
        let now_progress = [TableId::UnitLeaderTraitExperience, TableId::LexicalTokens]
            .map(|t| &decoded.tables[t.index()]);
        for (at, now) in now_progress.iter().enumerate() {
            if progress_tables
                .as_ref()
                .is_none_or(|last| !last[at].is(now))
            {
                progress_epochs[at] += 1;
            }
        }
        *progress_tables = Some(now_progress.map(SharedView::identity));
        let mut trait_cursor = 0;
        let mut complete = true;
        let now_traits = &decoded.tables[TableId::UnitLeaderTraits.index()];
        if trait_table
            .as_ref()
            .is_none_or(|(last, known)| *known != traits.is_some() || !last.is(now_traits))
        {
            *trait_epoch += 1;
            *trait_table = Some((now_traits.identity(), traits.is_some()));
        }
        // Every leader's visibility first, so a kept frame that cannot give
        // a verdict stops the hour before any career changes.
        let mut planned = Vec::with_capacity(rows.len());
        for (row_index, row) in rows.iter().enumerate() {
            #[cfg(test)]
            {
                work.row_visits += 1;
            }
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
            if let Some(identity) = &identity_changes {
                let renewed = identity.renew(
                    state,
                    row,
                    characters,
                    identity_before,
                    *identity_epoch,
                    visibility_epoch,
                );
                #[cfg(test)]
                {
                    *renewed_reads += renewed;
                }
                #[cfg(not(test))]
                let _ = renewed;
            }
            let owner_unchanged = state.last_read.as_ref().is_some_and(|last| {
                last.owner_epoch == *owner_epoch && last.row.owner == row.owner
            });
            let owner_slot = state
                .last_read
                .as_ref()
                .filter(|last| last.owner_epoch == *owner_epoch && last.row.owner == row.owner)
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
            let decided = visible_reusing(
                row,
                world.as_ref(),
                &mut state.visibility,
                if reusing {
                    Some(Reuse {
                        evaluation: &mut state.evaluation,
                        rereads: &mut rereads,
                        counts: &mut *reuse_counts,
                        since,
                    })
                } else {
                    None
                },
                visibility_epoch,
                owner_slot.is_some(),
            );
            if state
                .evaluation
                .as_ref()
                .is_some_and(|evaluation| reusing && evaluation.hour == hour)
            {
                evaluated.push(index);
            }
            planned.push((row_index, index, owner_unchanged, owner_slot, decided));
        }
        if let Some(kept) = kept
            && kept.blocked()
        {
            // Verdicts read on this frame stand only where they read what
            // the whole frame holds: the hour evaluates again there. Its
            // visibility reads are read again too, so a refusal this pass
            // marked as noted, but never recorded, is recorded there.
            for &(_, index, ..) in &planned {
                let state = &mut leaders[index];
                if state
                    .evaluation
                    .as_ref()
                    .is_some_and(|evaluation| evaluation.hour == hour)
                {
                    state.evaluation = None;
                }
                state.visibility = None;
            }
            return Ok(false);
        }
        for (row_index, index, owner_unchanged, owner_slot, decided) in planned {
            let row = &rows[row_index];
            let reference = handle(row);
            let state = &mut leaders[index];
            match decided {
                Ok(true) => {}
                Ok(false) => {
                    continue;
                }
                Err(reason) => {
                    complete = false;
                    if let Some(reason) = reason {
                        refusals.note(reason, reference.to_string());
                    }
                    continue;
                }
            }
            state.visible_hour = Some(hour);
            let holder = owner_tags
                .get(row.owner)
                .and_then(|tag| admission_holders.as_ref()?.1.get(tag))
                .copied();
            // The prepared admission roster depends only on the current holder
            // and kind. Native hourly coverage still belongs to this scan.
            let admission_changed = state.last_read.as_ref().is_none_or(|last| {
                last.holder != holder
                    || last.row.kind != row.kind
                    || reply.roles[last.role as usize].owner.as_ref() != owner_tags.get(row.owner)
            });
            let person_id = if let Some(last) = state.last_read.as_ref().filter(|last| {
                last.person_epoch == *identity_epoch
                    && last.row.character_id == row.character_id
                    && last.row.kind == row.kind
            }) {
                last.person
            } else if let Some(cached) = last_people
                .get(&(row.character_id, row.kind))
                .filter(|p| p.epoch == *identity_epoch)
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
                        .epoch = *identity_epoch;
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
                    // Retain the operand identity, never the recorded rows themselves.
                    let identity = blake3::hash(identity.as_bytes()).to_hex().to_string();
                    let index = if let Some(&index) = people.get(&identity) {
                        index
                    } else {
                        let person = person(decoded, row, character, token, content, reply);
                        let index = reply.people.len() as u32;
                        reply.people.push(person);
                        people.insert(identity, index);
                        index
                    };
                    last_people.insert(
                        (row.character_id, row.kind),
                        PersonRead {
                            epoch: *identity_epoch,
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
            // Progress is a function of these fields, the leader's training
            // rows and the tokens naming them: equal to the last read's, it
            // is the last run's.
            let progress_same = state.last_run.is_some()
                && state.last_read.as_ref().is_some_and(|last| {
                    (
                        last.row.experience,
                        last.row.skill_status,
                        last.row.skill_level,
                        last.row.category,
                    ) == (
                        row.experience,
                        row.skill_status,
                        row.skill_level,
                        row.category,
                    ) && last.progress_epochs[1] == progress_epochs[1]
                        && (last.progress_epochs[0] == progress_epochs[0]
                            || last.training.as_deref()
                                == progress_rows.map(|rows| training_rows(rows, reference)))
                });
            let progress = (!progress_same).then(|| {
                #[cfg(test)]
                PROGRESS_BUILDS.with(|builds| builds.set(builds.get() + 1));
                let training = progress_rows.map(|rows| {
                    training_rows(rows, reference)
                        .iter()
                        .filter(|p| p.list == 0)
                        .map(|p| {
                            let token = progress_tokens
                                .and_then(|tokens| {
                                    tokens
                                        .binary_search_by_key(&(p.token_id as u32), |t| t.token)
                                        .ok()
                                        .map(|i| &tokens[i])
                                })
                                .and_then(|t| {
                                    std::str::from_utf8(installed_content_core::parser::c_string(
                                        &t.literal,
                                    ))
                                    .ok()
                                });
                            if token.is_none() {
                                refusals.note(
                                    "Training trait token is unavailable",
                                    format!("{reference}:{}", p.ordinal),
                                );
                            }
                            let required = token
                                .and_then(|token| content.and_then(|c| c.leaders.traits.get(token)))
                                .map(|t| i64::from(t.required_experience()) * 100_000);
                            LeaderTraining {
                                token: token.map(str::to_owned),
                                name: token.and_then(|token| {
                                    content.and_then(|c| c.catalogue.localised(token, View::Plain))
                                }),
                                experience_raw: p.experience.to_string(),
                                required_raw: required.map(|v| v.to_string()),
                            }
                        })
                        .collect()
                });
                let factors = progress_rows.map(|rows| {
                    training_rows(rows, reference)
                        .iter()
                        .filter(|p| p.list == 1)
                        .map(|p| {
                            let token = progress_tokens
                                .and_then(|tokens| {
                                    tokens
                                        .binary_search_by_key(&(p.token_id as u32), |t| t.token)
                                        .ok()
                                        .map(|i| &tokens[i])
                                })
                                .and_then(|t| {
                                    std::str::from_utf8(installed_content_core::parser::c_string(
                                        &t.literal,
                                    ))
                                    .ok()
                                });
                            if token.is_none() {
                                refusals.note(
                                    "Training trait token is unavailable",
                                    format!("{reference}:{}", p.ordinal),
                                );
                            }
                            LeaderTrainingFactor {
                                token: token.map(str::to_owned),
                                name: token.and_then(|token| {
                                    content.and_then(|c| c.catalogue.localised(token, View::Plain))
                                }),
                                factor_raw: p.experience.to_string(),
                            }
                        })
                        .collect()
                });
                Some(LeaderProgress {
                    experience_raw: row.experience.to_string(),
                    next_required_raw: (row.skill_status != 0)
                        .then_some(content)
                        .flatten()
                        .and_then(|c| {
                            c.leaders
                                .skills
                                .next_required_experience(row.skill_level, row.category)
                        })
                        .flatten()
                        .map(|v| (i64::from(v) * 100_000).to_string()),
                    training,
                    factors,
                })
            });
            let training = (!progress_same)
                .then(|| progress_rows.map(|rows| training_rows(rows, reference).to_vec()));
            let leader = &mut state.career;
            let previous_run = state.last_run.map(|i| &stored_runs[i].run);
            let previous_hour = previous_run.map(|r| r.last_hour);
            let gap = previous_hour.is_some_and(|h| h.checked_add(1) != Some(hour));
            let battle_gap = previous_hour.is_some_and(|h| {
                missing_battles[usize::from(row.kind - 1)].is_some_and(|m| m >= h)
            });
            let current_battles_missing = if row.kind == engine_native::cunit_leader::KIND_ARMY {
                land.is_none()
            } else {
                navy.is_none()
            };
            if leader.partial_from.is_none() && (gap || battle_gap || current_battles_missing) {
                leader.partial_from = Some(hour);
            }
            let traits_unchanged = state.last_read.as_ref().is_some_and(|last| {
                last.trait_epoch == *trait_epoch || last.traits.as_deref() == raw_traits
            });
            let unchanged = state.last_read.as_ref().is_some_and(|last| {
                last.row.category == row.category
                    && last.row.kind == row.kind
                    && last.row.skill_status == row.skill_status
                    && last.row.skill_level == row.skill_level
                    && last.row.components == row.components
                    && previous_run.is_some_and(|run| {
                        progress
                            .as_ref()
                            .is_none_or(|progress| run.value.progress == *progress)
                    })
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
                // Unshown identity fields can change without creating a new value.
                if read.row != *row {
                    read.row.clone_from(row);
                }
                read.person_epoch = *identity_epoch;
                read.owner_slot = owner_slot;
                read.owner_epoch = *owner_epoch;
                read.trait_epoch = *trait_epoch;
                read.holder = holder;
                read.progress_epochs = *progress_epochs;
                if let Some(training) = training {
                    read.training = training;
                }
                if admission_changed {
                    admit(reference, &reply.roles[read.role as usize], holder);
                }
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
                                refusals.note("Trait token is not UTF-8", reference.to_string());
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
            let key = serde_json::to_string(&(owner_slot, row.category, row.kind, maximum_skill))?;
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
                last.person_epoch = *identity_epoch;
                last.owner_slot = owner_slot;
                last.owner_epoch = *owner_epoch;
                last.trait_epoch = *trait_epoch;
                last.holder = holder;
                last.progress_epochs = *progress_epochs;
                if let Some(training) = training {
                    last.training = training;
                }
            } else {
                state.last_read = Some(LastRead {
                    row: row.clone(),
                    traits: raw_traits.map(<[_]>::to_vec),
                    role,
                    person: person_id,
                    person_epoch: *identity_epoch,
                    owner_slot,
                    owner_epoch: *owner_epoch,
                    trait_epoch: *trait_epoch,
                    holder,
                    progress_epochs: *progress_epochs,
                    training: training.unwrap_or_else(|| {
                        progress_rows.map(|rows| training_rows(rows, reference).to_vec())
                    }),
                });
            }
            let progress = progress.unwrap_or_else(|| {
                let last = state.last_run.expect("the progress's last run");
                stored_runs[last].run.value.progress.clone()
            });
            let value = LeaderValues {
                person: person_id,
                role,
                skill: (row.skill_status != 0).then_some(row.skill_level),
                components: row.components,
                progress,
                traits: held,
            };
            let before = stored_runs.len();
            append_stored_run(state, stored_runs, value, hour, start, choices);
            *pending_bytes = pending_bytes
                .saturating_add((stored_runs.len() - before) * std::mem::size_of::<StoredRun>());
            if admission_changed {
                admit(reference, &reply.roles[role as usize], holder);
            }
        }
        row_indices.truncate(rows.len());
        *read_gap = world.as_ref().and_then(|world| world.gap.get().cloned());
        if land_commands {
            let plans = decoded
                .complete::<tables::OrdersGroupPlans>()
                .unwrap_or_default();
            if let Ok(groups) = decoded.complete::<tables::OrdersGroups>() {
                for group in groups {
                    #[cfg(test)]
                    {
                        work.row_visits += 1;
                    }
                    let Some(&index) = leader_indices.get(&group.leader) else {
                        continue;
                    };
                    let state = &mut leaders[index];
                    if state.visible_hour != Some(hour) {
                        continue;
                    }
                    let raw_name = match &group.name {
                        Recorded::Value(Some(bytes)) => Some(bytes.as_slice()),
                        Recorded::NotRecorded => plans
                            .binary_search_by_key(&group.handle, |plan| plan.group)
                            .ok()
                            .map(|index| plans[index].name.as_slice()),
                        Recorded::Value(None) => {
                            refusals.note(
                                "Stored army command name capture refused",
                                group.handle.to_string(),
                            );
                            None
                        }
                    };
                    let name =
                        formation_name(formation_names, group.handle, raw_name, content, refusals);
                    let kind = if group.is_army_group != 0 {
                        LeaderFormationKind::ArmyGroup
                    } else {
                        LeaderFormationKind::Army
                    };
                    let (command, repeated, bytes) =
                        append_command(state, group.handle, kind, name, hour);
                    *pending_bytes = pending_bytes.saturating_add(bytes);
                    *commands_continuous &= !repeated;
                    active_commands.push((index, command));
                }
            }
        }
        if let Some(fleets) = fleet_commands {
            for fleet in fleets {
                #[cfg(test)]
                {
                    work.row_visits += 1;
                }
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
                        .is_none_or(|last| last.row.kind != engine_native::cunit_leader::KIND_NAVY)
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
                let name = formation_name(formation_names, fleet.id, raw_name, content, refusals);
                let (command, repeated, bytes) =
                    append_command(state, fleet.id, LeaderFormationKind::Fleet, name, hour);
                *pending_bytes = pending_bytes.saturating_add(bytes);
                *commands_continuous &= !repeated;
                active_commands.push((index, command));
            }
        } else if naval_commands
            && let (Ok(missions), Ok(tasks)) = (
                decoded.complete::<tables::NavalMissionTaskForces>(),
                decoded.complete::<tables::TaskForces>(),
            )
        {
            let mut fleets = BTreeMap::<u64, Option<u64>>::new();
            for mission in missions {
                #[cfg(test)]
                {
                    work.row_visits += 1;
                }
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
                let Some(index) = commander.and_then(|c| leader_indices.get(&c).copied()) else {
                    continue;
                };
                let state = &mut leaders[index];
                if state.visible_hour == Some(hour)
                    && state
                        .last_read
                        .as_ref()
                        .is_some_and(|last| last.row.kind == engine_native::cunit_leader::KIND_NAVY)
                {
                    let (command, repeated, bytes) =
                        append_command(state, fleet, LeaderFormationKind::Fleet, None, hour);
                    *pending_bytes = pending_bytes.saturating_add(bytes);
                    *commands_continuous &= !repeated;
                    active_commands.push((index, command));
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
            let visits = record_battles(
                replay.format,
                decoded,
                land,
                navy,
                hour,
                leaders,
                leader_indices,
                naval_sources,
                battle_hours,
                battle_indices,
                previous_combats,
                pending_bytes,
            );
            #[cfg(test)]
            {
                work.row_visits += visits;
            }
            #[cfg(not(test))]
            let _ = visits;
        }
        *walked = true;
        Ok(true)
    }
    fn finish(
        mut self,
        params: &UnitLeadersParams,
        content: Option<&Content>,
    ) -> io::Result<UnitLeadersReply> {
        self.seal();
        let choices = params.view == LeaderView::Choices;
        let requested: Option<BTreeSet<&str>> = match &params.selection {
            LeaderSelection::Countries { tags } => Some(tags.iter().map(String::as_str).collect()),
            _ => None,
        };
        let Self {
            mut reply,
            leaders,
            stored_runs,
            mut battle_hours,
            outcomes,
            results_recorded,
            mut token_list,
            mut refusals,
            ..
        } = self;
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
                let mut hours = battle_hours.remove(&state.reference).unwrap_or_default();
                hours.sort_by_key(|read| (read.0, read.3, read.4));
                let mut previous = 0_i64;
                let mut starts = Vec::with_capacity(hours.len());
                let mut start_reads = Vec::with_capacity(hours.len());
                let mut unknown_starts = Vec::new();
                let mut won = Vec::with_capacity(hours.len());
                let mut result_hours = Vec::with_capacity(hours.len());
                let mut result_read_hours = Vec::with_capacity(hours.len());
                for (index, (hour, read_hour, unknown, kind, combat, side)) in
                    hours.into_iter().enumerate()
                {
                    starts.push(i64::from(hour) - previous);
                    start_reads.push(read_hour);
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
                        unknown_starts.push(index as u32);
                    }
                }
                leader.battle_starts = Some(starts);
                leader.battle_start_read_hours = Some(start_reads);
                leader.unknown_battle_starts = Some(unknown_starts);
                if results_recorded {
                    leader.battle_won = Some(won);
                    leader.battle_result_hours = Some(result_hours);
                    leader.battle_result_read_hours = Some(result_read_hours);
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
            let used_traits: BTreeSet<_> = reply
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
        let prepared: BTreeMap<_, _> = std::mem::take(&mut reply.traits)
            .into_iter()
            .map(|info| (info.token.clone(), info))
            .collect();
        for token in token_list {
            let info = prepared
                .get(&token)
                .cloned()
                .unwrap_or_else(|| trait_info(&token, content));
            if let Some(reason) = &info.missing {
                refusals.note(reason.clone(), token);
            }
            reply.traits.push(info);
        }
        reply
            .warnings
            .extend(refusals.lines("Leader data withheld"));
        extend_content_warnings(&mut reply, content);
        reply.warnings.sort();
        reply.warnings.dedup();
        Ok(reply)
    }
}

fn extend_content_warnings(reply: &mut UnitLeadersReply, content: Option<&Content>) {
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
}

#[cfg(any(test, feature = "test-support"))]
fn scan(
    replay: &Replay,
    params: &UnitLeadersParams,
    content: Option<&Content>,
    cancelled: &AtomicBool,
) -> io::Result<UnitLeadersReply> {
    let branch = params.branch_id.parse::<u64>().expect("validated branch");
    let selected = scan_tables(
        params.view == LeaderView::Career,
        params.view == LeaderView::Choices,
    );
    let mut scan = Scan::new(replay, branch)?;
    scan.reply.branch_id.clone_from(&params.branch_id);
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|b| b.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(e) => {
                        scan.refusals
                            .note(e.to_string(), format!("block {}", block.first_hour));
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
                        Ok(frame) => frame,
                        Err(e) => {
                            scan.refusals.note(e.to_string(), frame.hour.to_string());
                            break;
                        }
                    };
                    validate_campaign(decoded, replay)?;
                    scan.observe_read(decoded, replay, params, content, &mut |_, _, _| {})?;
                }
            }
            Ok(())
        },
    )?;
    scan.finish(params, content)
}

// Retention limits cached snapshots, never the inventory a request may ask for.
#[cfg(any(test, feature = "test-support"))]
const CACHED_BRANCHES: usize = 2;
#[cfg(any(test, feature = "test-support"))]
const CACHED_REPLIES: usize = 4;
#[cfg(any(test, feature = "test-support"))]
type CachedReply = Result<Arc<UnitLeadersReply>, (io::ErrorKind, String)>;

#[cfg(any(test, feature = "test-support"))]
fn remembered(result: io::Result<Arc<UnitLeadersReply>>) -> CachedReply {
    result.map_err(|error| (error.kind(), error.to_string()))
}
#[cfg(any(test, feature = "test-support"))]
fn recalled(result: CachedReply) -> io::Result<Arc<UnitLeadersReply>> {
    result.map_err(|(kind, message)| io::Error::new(kind, message))
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct BranchState {
    scan: Option<CachedReply>,
    scanning: bool,
    replies: VecDeque<(UnitLeadersParams, CachedReply)>,
}
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct BranchSlot {
    state: Mutex<BranchState>,
    ready: Condvar,
}
#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Cache {
    catalogue: Option<Arc<Catalogue>>,
    branches: VecDeque<(String, Arc<BranchSlot>)>,
}
#[cfg(any(test, feature = "test-support"))]
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

/// The recorded trait registry and each trait's icon drawn for one selection; one per kind of
/// catalogue ([`catalogue::ByCatalogue`]).
#[derive(Default)]
struct TraitArt {
    traits: Option<(Arc<Catalogue>, installed_content_core::unit_leader::Traits)>,
    drawn: Option<catalogue::Redrawn<String, String>>,
}

/// Draws the trait icons a prepared payload stored (the recording's art, as preparation is) as
/// the request admitted under `content` shows them, when it shows visual mods: each by its token,
/// the whole choice (the registered sprite, or the GUI's fallback drawable when the presentation
/// does not declare it) made against the selection, its recorded name kept. When the selection
/// cannot draw it, the recording's own choice is drawn and named on every reply that shows it.
pub fn present_traits(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    traits: &mut [LeaderTraitInfo],
) -> io::Result<()> {
    let Some((admitted, recorded)) = catalogue::presenting(source, content)? else {
        return Ok(());
    };
    let state = source.state::<catalogue::ByCatalogue<TraitArt>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let TraitArt {
        traits: registry,
        drawn,
    } = state.of(&admitted);
    if !registry
        .as_ref()
        .is_some_and(|(held, _)| Arc::ptr_eq(held, &recorded))
    {
        let leaders =
            installed_content_core::unit_leader::Leaders::<String>::load(&recorded.content.layers, &|_| None)?;
        *registry = Some((Arc::clone(&recorded), leaders.traits));
    }
    let (_, registry) = registry.as_ref().expect("trait registry read");
    let drawn = catalogue::Redrawn::bind(drawn, &admitted, &recorded);
    for info in traits {
        let token = info.token.clone();
        let shown = drawn.get(
            token.clone(),
            || format!("trait {token} icon"),
            |catalogue| trait_icon(catalogue, registry, &token),
        );
        info.icon = shown.as_ref().ok().cloned();
        info.missing = shown.as_ref().err().cloned();
    }
    Ok(())
}

/// A trait's icon drawn through `catalogue`: the sprite its registration names, or the GUI's
/// fallback drawable when the catalogue does not declare that sprite.
fn trait_icon(
    catalogue: &Catalogue,
    traits: &installed_content_core::unit_leader::Traits,
    token: &str,
) -> Result<String, String> {
    let name = traits
        .assignment_sprite(token)
        .map_err(str::to_owned)?
        .ok_or_else(|| format!("Trait {token} is not registered"))?;
    let sprite =
        catalogue::created_gui_sprite(&catalogue.sprites, &name).map_err(|e| e.to_string())?;
    let decode = *catalogue.gui_decode.as_ref().map_err(Clone::clone)?;
    catalogue::sprite_frame_png(&catalogue.content.layers, sprite, decode, 0, 32)
        .map_err(|e| e.to_string())
}

fn trait_info(token: &str, content: Option<&Content>) -> LeaderTraitInfo {
    let (name, icon, missing) = content
        .map(|content| {
            let catalogue = &content.catalogue;
            let name = catalogue.localised(token, View::Plain);
            // The assignment tree admits every defined trait; icon construction
            // retries the stack's fallback drawable for an absent name. The whole choice is
            // made against the selection, else again as the recording's own, named.
            let icon = catalogue::draw(
                || format!("trait {token} icon"),
                catalogue,
                |catalogue| trait_icon(catalogue, &content.leaders.traits, token),
            );
            match icon.get().clone() {
                Ok(icon) => (name, Some(icon), None),
                Err(error) => (name, None, Some(error)),
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

// Army and navy trait-window localisation keys.
fn attribute_names(kind: u8, content: Option<&Catalogue>) -> [Option<String>; 4] {
    let keys = if kind == engine_native::cunit_leader::KIND_NAVY {
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

/// A formation's stored name. Names repeat every hour, so each formation's
/// bytes are formatted once; a refusal noted then names the formation, as a
/// repeat would.
fn formation_name<'n>(
    names: &'n mut HashMap<u64, (Option<Vec<u8>>, Option<String>)>,
    formation: u64,
    bytes: Option<&[u8]>,
    content: Option<&Content>,
    refusals: &mut catalogue::Refusals,
) -> Option<&'n str> {
    let bytes = bytes?;
    let (read, name) = names.entry(formation).or_default();
    if read.as_deref() != Some(bytes) {
        *name = content
            .ok_or_else(|| "Formation name content unavailable".to_owned())
            .and_then(|content| {
                dashboard_text::scripted_text::stored_text(
                    &content.catalogue.localisation,
                    &content.catalogue.recorded_language,
                    bytes,
                    "formation name",
                )
            })
            .and_then(|text| catalogue::display_text(text.as_bytes()))
            .map_err(|reason| refusals.note(reason, formation.to_string()))
            .ok();
        *read = Some(bytes.to_vec());
    }
    name.as_deref()
}

type BattleOutcomes = BTreeMap<(u8, u64), Option<(i32, i32, u8)>>;
type BattleHours = HashMap<u64, Vec<(i32, i32, bool, u8, u64, u8)>>;

/// An hour's combat handles, sorted once for its lookups.
fn sorted_combats(combats: impl Iterator<Item = u64>) -> Vec<u64> {
    let mut combats: Vec<u64> = combats.collect();
    combats.sort_unstable();
    combats.dedup();
    combats
}

/// Read the hour's results: land outcomes, naval result sources and the
/// naval outcomes they link. Returns the rows visited.
fn record_results(
    format: FormatVersion,
    decoded: &DecodedFrame,
    outcomes: &mut BattleOutcomes,
    naval_sources: &mut BTreeMap<u64, NavalSource>,
    results_recorded: &mut bool,
    refusals: &mut catalogue::Refusals,
) -> usize {
    let hour = decoded.hour;
    let mut visits = 0;
    if slot_read(format, &decoded.metrics, TableId::LandCombatResults, hour)
        && let Ok(results) = decoded.complete::<tables::LandCombatResults>()
    {
        *results_recorded = true;
        for result in results {
            visits += 1;
            if record_outcome(
                outcomes,
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
    if slot_read(format, &decoded.metrics, TableId::NavalResultSources, hour)
        && let Ok(sources) = decoded.complete::<tables::NavalResultSources>()
    {
        *results_recorded = true;
        for source in sources {
            visits += 1;
            if !source.transient && !source.merged {
                naval_sources.insert(
                    source.result,
                    NavalSource {
                        combat: source.combat,
                        game_hour: source.game_hour,
                    },
                );
            }
        }
    }
    if let Ok(results) = decoded.complete::<tables::NavalResults>() {
        for result in results {
            visits += 1;
            if let Some(source) = naval_sources.get(&result.id) {
                // Results copy the combat's first and second sides; the recorded
                // selector chooses those exact embedded sides.
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
                    outcomes,
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
    visits
}

/// Read the hour's battles: each side of each combat (`land`, `navy`, as the
/// hour reads them) and of each result, joined to its commander when that
/// leader is visible at `visible_at` (the hour itself after the leader walk;
/// the hour before when the leaders carry on unchanged). Ends with the
/// hour's combats as the last read's. Returns the sides visited.
#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::type_complexity,
    reason = "Battle intervals keep the scan's persisted tuple representation."
)]
fn record_battles(
    format: FormatVersion,
    decoded: &DecodedFrame,
    land: Option<&[LandCombatDetailsRow]>,
    navy: Option<&[NavalCombatDetailsRow]>,
    visible_at: i32,
    leaders: &[LeaderState],
    leader_indices: &HashMap<u64, usize>,
    naval_sources: &BTreeMap<u64, NavalSource>,
    battle_hours: &mut BattleHours,
    battle_indices: &mut HashMap<(u64, u8, u64), usize>,
    previous_combats: &mut [Option<(i32, Vec<u64>)>; 2],
    pending_bytes: &mut usize,
) -> usize {
    let hour = decoded.hour;
    let mut visits = 0;
    let combat_sets = [
        land.map(|rows| sorted_combats(rows.iter().map(|c| c.combat))),
        navy.map(|rows| sorted_combats(rows.iter().map(|c| c.combat))),
    ];
    let sides = land
        .into_iter()
        .flatten()
        .flat_map(|c| {
            c.sides
                .iter()
                .enumerate()
                .map(move |(side, s)| (1u8, c.combat, s.commander, 1u8 << side, hour, true))
        })
        .chain(navy.into_iter().flatten().flat_map(|c| {
            c.sides
                .iter()
                .enumerate()
                .map(move |(side, s)| (2u8, c.combat, s.commander, 1u8 << side, hour, true))
        }))
        .chain(
            decoded
                .complete::<tables::LandCombatResults>()
                .ok()
                .filter(|_| slot_read(format, &decoded.metrics, TableId::LandCombatResults, hour))
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
        visits += 1;
        let Some(&index) = leader_indices.get(&commander) else {
            continue;
        };
        if leaders[index].visible_hour != Some(visible_at) {
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
                        h.checked_add(1) == Some(hour) && set.binary_search(&combat).is_err()
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
            *pending_bytes =
                pending_bytes.saturating_add(std::mem::size_of::<(i32, i32, bool, u8, u64, u8)>());
        }
    }
    *previous_combats = combat_sets.map(|set| set.map(|set| (hour, set)));
    visits
}

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

/// A leaderboard's battle counts from a scanned career: the hours its
/// participations, known wins and known losses became visible, as the career
/// view counts them at every hour. A participation waits for its date and first
/// supplying read; a result also waits for its own date and read. Without
/// recorded results, wins and losses stay absent, as in the career.
fn visible_battle_hours(leader: &LeaderCareer) -> [Option<Vec<i64>>; 3] {
    let starts = leader
        .battle_starts
        .as_deref()
        .expect("scanned participations");
    let reads = leader
        .battle_start_read_hours
        .as_deref()
        .expect("scanned participation reads");
    let mut start = 0_i64;
    let fought: Vec<i64> = starts
        .iter()
        .zip(reads)
        .map(|(delta, read)| {
            start += delta;
            start.max(i64::from(*read))
        })
        .collect();
    let [won, lost] = match (
        &leader.battle_won,
        &leader.battle_result_hours,
        &leader.battle_result_read_hours,
    ) {
        (Some(won), Some(result_hours), Some(result_reads)) => {
            let mut results = [Vec::new(), Vec::new()];
            for (i, &visible) in fought.iter().enumerate() {
                if let (Some(outcome), Some(hour), Some(read)) =
                    (won[i], result_hours[i], result_reads[i])
                {
                    results[usize::from(!outcome)]
                        .push(visible.max(i64::from(hour)).max(i64::from(read)));
                }
            }
            results.map(|visible| Some(sorted_deltas(visible)))
        }
        _ => [None, None],
    };
    [Some(sorted_deltas(fought)), won, lost]
}

/// Sorts hours into the battle-start coding: first absolute, then deltas.
fn sorted_deltas(mut hours: Vec<i64>) -> Vec<i64> {
    hours.sort_unstable();
    let mut previous = 0_i64;
    for hour in &mut hours {
        let absolute = *hour;
        *hour -= previous;
        previous = absolute;
    }
    hours
}

fn project(
    scanned: &UnitLeadersReply,
    params: &UnitLeadersParams,
    history: Option<&dashboard_base::products::country_history::RecordedHistories>,
) -> UnitLeadersReply {
    let requested: BTreeSet<_> = match &params.selection {
        LeaderSelection::Countries { tags } => history
            .map_or_else(
                || tags.clone(),
                |history| {
                    dashboard_base::products::country_history::selected_aliases(history, tags)
                },
            )
            .into_iter()
            .collect(),
        _ => BTreeSet::new(),
    };
    let selected = history.and_then(|history| {
        let LeaderSelection::Countries { tags } = &params.selection else {
            return None;
        };
        let identities =
            dashboard_base::products::country_history::selected_identities(history, tags);
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
            let career = params.view == LeaderView::Career;
            let [fought_hours, won_hours, lost_hours] = if params.view == LeaderView::Leaderboard {
                visible_battle_hours(leader)
            } else {
                Default::default()
            };
            LeaderCareer {
                reference: leader.reference.clone(),
                runs,
                trait_gains: if choices {
                    Vec::new()
                } else {
                    leader.trait_gains.clone()
                },
                commands: if career {
                    leader.commands.clone()
                } else {
                    Vec::new()
                },
                battle_starts: career.then(|| leader.battle_starts.clone()).flatten(),
                battle_start_read_hours: career
                    .then(|| leader.battle_start_read_hours.clone())
                    .flatten(),
                unknown_battle_starts: career
                    .then(|| leader.unknown_battle_starts.clone())
                    .flatten(),
                battle_won: career.then(|| leader.battle_won.clone()).flatten(),
                battle_result_hours: career.then(|| leader.battle_result_hours.clone()).flatten(),
                battle_result_read_hours: career
                    .then(|| leader.battle_result_read_hours.clone())
                    .flatten(),
                fought_hours,
                won_hours,
                lost_hours,
                partial_from: leader.partial_from,
                at_hour: None,
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
        preparing: scanned.preparing.clone(),
        results_missing: scanned.results_missing.clone(),
        navy_commands_missing: scanned.navy_commands_missing.clone(),
        warnings: scanned.warnings.clone(),
    };
    compact(&mut reply, &scanned.people, &scanned.roles, &scanned.traits);
    reply
}

/// Keep only the people, roles and traits the reply's leaders still name.
fn compact(
    reply: &mut UnitLeadersReply,
    people: &[LeaderPerson],
    roles: &[LeaderRole],
    traits: &[LeaderTraitInfo],
) {
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
        .map(|index| people[*index as usize].clone())
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
        .iter()
        .map(|index| roles[*index as usize].clone())
        .collect();
    let used_traits: BTreeSet<_> = reply
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
    let trait_remap: BTreeMap<_, _> = used_traits
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new as u32))
        .collect();
    reply.traits = used_traits
        .iter()
        .map(|index| traits[*index as usize].clone())
        .collect();
    let mut trait_sets = HashMap::<Vec<u32>, Arc<[u32]>>::new();
    let remap_traits = reply.traits.len() != traits.len();
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
}

/// A leaderboard for one hour: each leader shown then keeps only the run
/// covering it, its trait gains through it and its battle counts then, and the
/// reply only the observation span covering it.
pub fn leaderboard_at(reply: &mut UnitLeadersReply, hour: i32) {
    let count = |hours: &Option<Vec<i64>>| {
        let mut at = 0_i64;
        hours.as_deref().map(|deltas| {
            let shown = deltas
                .iter()
                .take_while(|delta| {
                    at += **delta;
                    at <= i64::from(hour)
                })
                .count();
            u32::try_from(shown).expect("battle count fits the reply")
        })
    };
    reply.leaders.retain_mut(|leader| {
        let index = leader.runs.partition_point(|run| run.last_hour < hour);
        if leader
            .runs
            .get(index)
            .is_none_or(|run| run.first_hour > hour)
        {
            return false;
        }
        leader.at_hour = Some(LeaderAtHour {
            hour,
            first_seen_hour: leader.runs[0].first_hour,
            fought: count(&leader.fought_hours).expect("leaderboard participations"),
            won: count(&leader.won_hours),
            lost: count(&leader.lost_hours),
        });
        leader.runs = vec![leader.runs.swap_remove(index)];
        leader.trait_gains.retain(|gain| gain.hour <= hour);
        leader.fought_hours = None;
        leader.won_hours = None;
        leader.lost_hours = None;
        true
    });
    reply
        .observed
        .retain(|span| span.first_hour <= hour && hour <= span.last_hour);
    let people = std::mem::take(&mut reply.people);
    let roles = std::mem::take(&mut reply.roles);
    let traits = std::mem::take(&mut reply.traits);
    compact(reply, &people, &roles, &traits);
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
            UnitLeadersAtHourParams::decl(),
            LeaderPerson::decl(),
            LeaderRole::decl(),
            LeaderTraining::decl(),
            LeaderTrainingFactor::decl(),
            LeaderProgress::decl(),
            LeaderValues::decl(),
            UnitLeaderRun::decl(),
            LeaderChangeDate::decl(),
            LeaderTraitGain::decl(),
            LeaderFormationKind::decl(),
            LeaderCommand::decl(),
            LeaderCareer::decl(),
            LeaderAtHour::decl(),
            LeaderTraitInfo::decl(),
            LeaderObservation::decl(),
            UnitLeadersReply::decl(),
        ]
    }
    #[cfg(any(test, feature = "test-support"))]
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
            let content = Content::for_catalogue(source, context, cat);
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
            Some(dashboard_base::products::country_history::branch_histories(
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
        let result = remembered(reply_bytes_within(&reply, MAX_REPLY_BYTES).map(|_| Arc::new(reply)));
        slot.remember(params, result.clone());
        recalled(result).map(UnitLeadersResponse::from)
    }
}

#[cfg(test)]
mod tests;
