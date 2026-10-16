//! The recorded map events, interpreted for this viewer at the presented hour.
//!
//! Each event's actors, intel, anchors and fog switch are recorded at the edge
//! where the game decides it, before the game's local filter. That filter is
//! applied here for the viewing country: country links, relations, modifiers
//! and province fog are the presented hour's reconstruction. Camera, selection,
//! animation clocks and queues are viewer state, never recording operands.
//! Unknown coverage never hides an event that was recorded.
use std::{cell::OnceCell, collections::BTreeMap};

use installed_content::{
    buildings::Buildings,
    localisation::{Argument, Localisation},
};
use replay::{
    BranchId,
    frame::DecodedFrame,
    rows::{
        MapEvent, MapEventBuildingReason, MapEventFamily, MapEventFault, MapEventPayload,
        MapEventUnitIntel, MapEventsRow, tables,
    },
};
use viewer_foundation::{
    map::{
        perspective::{Resolution, Vision},
        presentation::Continuity,
        visibility,
    },
    table_access::{Refusals, TokenKeys, TokenNames, refuse},
};

use super::{Popup, Style};
use crate::map::strike_effects::{Effect, Impact};

/// The localisation keys the game's producers show.
pub const FAKE_DIVISION_REVEALED: &[u8] = b"INTEL_DIVISION_REVEALED";
pub const RAILWAY_GUN_OVERRUN: &[u8] = b"OVERRUN_POPUP";
pub const CRITICAL_BOMBARDMENT: &[u8] = b"CRITICAL_BOMBARDMENT_DAMAGE";
pub const GENERAL_CAPTURED: &[u8] = b"CAPTURED_GENERAL_MAP_TEXT";
pub const PARADROP_LANDED: &[u8] = b"PROV_TEXT_UNITS_PARACHUTED";
pub const PARADROP_LANDED_WITH_LOSSES: &[u8] = b"PROV_TEXT_UNITS_PARACHUTED_WITH_LOSSES";
/// The building damage text's substitution for the building's name.
pub const BUILDING_ARGUMENT: &str = "BUILDING";
/// The intel level a revealed unit's text needs.
const SHOWN_LEVEL: u16 = 150;

/// Each building damage reason's localisation key.
pub fn building_key(reason: MapEventBuildingReason) -> &'static [u8] {
    match reason {
        MapEventBuildingReason::Bombed => b"BUILDING_BOMBED",
        MapEventBuildingReason::Sabotaged => b"BUILDING_SABOTAGED",
        MapEventBuildingReason::Collateral => b"BUILDING_DAMAGE_REASON_COLLATERAL",
        MapEventBuildingReason::Repaired => b"BUILDING_REPAIRED",
        MapEventBuildingReason::Scorched => b"BUILDING_SCORCHED",
    }
}

/// The building types the recorded content stack declares, for their name keys.
pub struct Content {
    buildings: TokenKeys,
}

impl Content {
    pub fn new(buildings: &Buildings) -> Self {
        Self {
            buildings: TokenKeys::new(buildings.types.keys()),
        }
    }
}

/// One presented hour's inputs. Relation and identity reads are shared by
/// the hour's events and made only by the families that need them.
pub struct Context<'a> {
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    content: &'a Content,
    localisation: &'a Localisation,
    links: OnceCell<Result<BTreeMap<u32, u32>, String>>,
    viewer: OnceCell<Result<u32, String>>,
    modifier: OnceCell<Result<i64, String>>,
    names: OnceCell<Result<TokenNames<'a>, String>>,
}

impl<'a> Context<'a> {
    pub fn new(
        frame: &'a DecodedFrame,
        vision: &'a Vision,
        content: &'a Content,
        localisation: &'a Localisation,
    ) -> Self {
        Self {
            frame,
            vision,
            content,
            localisation,
            links: OnceCell::new(),
            viewer: OnceCell::new(),
            modifier: OnceCell::new(),
            names: OnceCell::new(),
        }
    }

    fn links(&self) -> Result<&BTreeMap<u32, u32>, String> {
        self.links
            .get_or_init(|| {
                let rows = self
                    .frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|e| e.to_string())?;
                let mut links = BTreeMap::new();
                for row in rows {
                    if links.insert(row.id, row.canonical_slot).is_some() {
                        return Err("map text CountryLinks has duplicate raw references".into());
                    }
                }
                Ok(links)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The viewing country's raw reference: the recorded client's own word
    /// when the view is the recorded one, otherwise its country's id.
    fn viewer(&self) -> Result<u32, String> {
        self.viewer
            .get_or_init(|| {
                if self.vision.resolution() == &Resolution::Recorded
                    && let Ok([player]) = self.frame.complete::<tables::LocalPlayer>()
                    && let Some(raw) = player.viewing_country()
                    && self.links()?.get(&raw) == Some(&self.vision.ours())
                {
                    return Ok(raw);
                }
                let rows = self
                    .frame
                    .complete::<tables::Countries>()
                    .map_err(|e| e.to_string())?;
                let mut rows = rows
                    .iter()
                    .filter(|row| row.canonical_slot == self.vision.ours());
                let row = rows
                    .next()
                    .ok_or("map text viewing country has no Countries row")?;
                if rows.next().is_some() {
                    return Err("map text viewing country has duplicate Countries rows".into());
                }
                Ok(row.id)
            })
            .clone()
    }

    /// The game's country equality: equal raw words, or two nonzero words of
    /// one canonical country.
    fn same(&self, a: u32, b: u32) -> Result<bool, String> {
        if a == b {
            return Ok(true);
        }
        if a == 0 || b == 0 {
            return Ok(false);
        }
        let links = self.links()?;
        let a = links
            .get(&a)
            .ok_or("map text actor absent from CountryLinks")?;
        let b = links
            .get(&b)
            .ok_or("map text viewer absent from CountryLinks")?;
        Ok(a == b)
    }

    fn shared(&self, receiver: u32, target: u32, military: bool) -> Result<bool, String> {
        visibility::event_shared(self.frame, self.links()?, receiver, target, military)
            .map_err(str::to_owned)
    }

    /// The viewing country's unit-intel modifier.
    fn modifier(&self) -> Result<i64, String> {
        self.modifier
            .get_or_init(|| {
                if self.vision.ours() == 0 {
                    return Err("map text reserved viewer unit modifier not recorded".into());
                }
                let rows = self
                    .frame
                    .complete::<tables::CountryVisibility>()
                    .map_err(|e| e.to_string())?;
                let links = self.links()?;
                let mut modifier = None;
                for row in rows {
                    let slot = if row.country == 0 {
                        0
                    } else {
                        *links
                            .get(&row.country)
                            .ok_or("map text CountryVisibility country unresolved")?
                    };
                    if slot == self.vision.ours() && modifier.replace(row.unit_modifier).is_some() {
                        return Err("map text viewer CountryVisibility row duplicated".into());
                    }
                }
                if let Some(modifier) = modifier {
                    return Ok(modifier);
                }
                // The table omits zero modifiers, so a complete table proves
                // zero for a country it lists no row for.
                let countries = self
                    .frame
                    .complete::<tables::Countries>()
                    .map_err(|e| e.to_string())?;
                match countries
                    .iter()
                    .filter(|row| row.canonical_slot == self.vision.ours())
                    .count()
                {
                    1 => Ok(0),
                    0 => Err("map text viewing country has no Countries row".into()),
                    _ => Err("map text viewing country has duplicate Countries rows".into()),
                }
            })
            .clone()
    }

    fn names(&self) -> Result<&TokenNames<'a>, String> {
        self.names
            .get_or_init(|| TokenNames::new(self.frame).map_err(|e| e.to_string()))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Whether the viewing country's intel level of the unit reaches the
    /// text's threshold, as the game computes it: its own or a sharing
    /// country's unit is fully known; otherwise the intel array's level for
    /// the viewer gives a ladder value, the province's visibility byte raises
    /// it, and the viewer's modifier scales the result.
    fn unit_visible(&self, unit: &MapEventUnitIntel) -> Result<bool, String> {
        if !unit.fog || !self.vision.fog() {
            return Ok(true);
        }
        let viewer = self.viewer()?;
        if self.same(unit.owner, viewer)? {
            return Ok(true);
        }
        let shared = self.shared(unit.owner, viewer, false);
        let level = if shared == Ok(true) {
            3
        } else {
            unit.intel
                .iter()
                .find(|entry| entry.country == self.vision.ours())
                .map_or(0, |entry| entry.level)
        };
        let ladder = match level {
            1 => 150,
            2.. => 250,
            _ => 0,
        };
        let province = self.vision.province_byte(unit.visibility_province);
        // A zero level and an unseen province decide before the modifier.
        if ladder == 0 && province == Some(0) && shared.is_ok() {
            return Ok(false);
        }
        // An unknown relation can only raise the level to the top of the
        // ladder, and a failed fog rebuild leaves the province byte unknown:
        // only an answer every such value agrees on decides.
        unit_admission(ladder, province, self.modifier()?, shared.is_ok()).ok_or_else(|| {
            shared
                .err()
                .unwrap_or_else(|| "map text province fog unknown".into())
        })
    }

    /// A landing shows to the dropping country's friends and wherever the
    /// viewer sees the province.
    fn paradrop_visible(&self, fog: bool, actor: u32, province: u32) -> Result<bool, String> {
        if !fog || !self.vision.fog() {
            return Ok(true);
        }
        let seen = self.vision.province_byte(province).map(|byte| byte != 0);
        if seen == Some(true) {
            return Ok(true);
        }
        let friendly = self
            .viewer()
            .and_then(|viewer| self.shared(viewer, actor, true));
        match (friendly, seen) {
            (Ok(true), _) => Ok(true),
            (Ok(false), Some(_)) => Ok(false),
            (Err(reason), _) => Err(reason),
            (Ok(false), None) => Err("map text paradrop province fog unknown".into()),
        }
    }

    fn text(&self, key: &[u8], args: &[(&str, Argument<'_>)]) -> Vec<u8> {
        self.localisation.localise_bytes(key, args)
    }

    /// The event's text for this viewer, or `None` when the game would not
    /// show it to this country.
    pub fn popup(&self, event: &MapEvent) -> Result<Option<Popup>, String> {
        let (province, style, text) = match &event.payload {
            MapEventPayload::FakeDivisionRevealed(unit) => {
                if !self.unit_visible(unit)? {
                    return Ok(None);
                }
                (
                    unit.anchor_province,
                    Style::CombatLoss,
                    self.text(FAKE_DIVISION_REVEALED, &[]),
                )
            }
            MapEventPayload::RailwayGunOverrun(unit) => {
                if !self.unit_visible(unit)? {
                    return Ok(None);
                }
                (
                    unit.anchor_province,
                    Style::CombatLoss,
                    self.text(RAILWAY_GUN_OVERRUN, &[]),
                )
            }
            MapEventPayload::CriticalShoreBombardment(critical) => (
                critical.province,
                Style::SiegeInfo,
                self.text(CRITICAL_BOMBARDMENT, &[]),
            ),
            MapEventPayload::GeneralCaptured(capture) => {
                let viewer = self.viewer()?;
                if !either(
                    self.same(viewer, capture.owner),
                    self.same(viewer, capture.captor),
                )? {
                    return Ok(None);
                }
                (
                    capture.province,
                    Style::SiegeInfo,
                    self.text(GENERAL_CAPTURED, &[]),
                )
            }
            MapEventPayload::ParadropLanding(paradrop) => {
                if !self.paradrop_visible(paradrop.fog, paradrop.country, paradrop.province)? {
                    return Ok(None);
                }
                let key = if paradrop.lost {
                    PARADROP_LANDED_WITH_LOSSES
                } else {
                    PARADROP_LANDED
                };
                (paradrop.province, Style::SiegeInfo, self.text(key, &[]))
            }
            MapEventPayload::ResistanceActivity(activity) => {
                let viewer = self.viewer()?;
                if viewer as i32 <= 0
                    || !either(
                        self.same(viewer, activity.controller),
                        self.same(viewer, activity.overlord),
                    )?
                {
                    return Ok(None);
                }
                // The game shows nothing for an activity whose text is empty.
                let text = self.text(&activity.key, &[]);
                if text.is_empty() {
                    return Ok(None);
                }
                (activity.province, Style::SiegeInfo, text)
            }
            MapEventPayload::BuildingDamage(building) => {
                if !self.same(self.viewer()?, building.controller)? {
                    return Ok(None);
                }
                let spelling = self
                    .names()?
                    .name(building.building_type)
                    .map_err(|e| e.to_string())?;
                let key = self
                    .content
                    .buildings
                    .resolve(spelling)?
                    .ok_or("map text building token has no content declaration")?;
                let name = self
                    .localisation
                    .localise(key.as_bytes())
                    .map_err(|e| e.to_string())?;
                (
                    building.province,
                    Style::SiegeInfo,
                    self.text(
                        building_key(building.reason),
                        &[(BUILDING_ARGUMENT, Argument::Text(&name))],
                    ),
                )
            }
        };
        if province == 0 {
            return Err("map text anchor province is null".into());
        }
        Ok(Some(Popup {
            sequence: event.sequence,
            family: event.payload.family(),
            province,
            style,
            text,
        }))
    }
}

/// The native OR of two predicates either of which may be unknown: a proven
/// positive settles it.
fn either(a: Result<bool, String>, b: Result<bool, String>) -> Result<bool, String> {
    if a == Ok(true) || b == Ok(true) {
        Ok(true)
    } else {
        Ok(a? || b?)
    }
}

fn unit_admission(
    ladder: u16,
    province: Option<u8>,
    modifier: i64,
    sharing_known: bool,
) -> Option<bool> {
    let base = ladder.max(province.map_or(0, u16::from));
    if province.is_some() {
        let answer = visibility::unit_byte(base, modifier) >= SHOWN_LEVEL;
        // An unknown sharing predicate leaves the ladder or raises it to 250.
        return (sharing_known
            || (visibility::unit_byte(base.max(250), modifier) >= SHOWN_LEVEL) == answer)
            .then_some(answer);
    }
    // An unknown province byte ranges over every byte. The game's arithmetic
    // wraps, so every reachable base is checked rather than assuming order.
    let mut answers =
        (base..=255).map(|value| visibility::unit_byte(value, modifier) >= SHOWN_LEVEL);
    let answer = answers.next()?;
    answers.all(|next| next == answer).then_some(answer)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Boundary {
    branch: BranchId,
    epoch: u64,
    from_hour: i32,
    to_hour: i32,
    first: u64,
    next: u64,
}

/// One event of a prepared interval: its sequence, its popup for the
/// viewing country (or why it cannot be decided) and, for a critical shore
/// bombardment, its located explosion.
#[derive(Clone, Debug)]
pub struct PreparedEvent {
    pub sequence: u64,
    pub family: MapEventFamily,
    pub popup: Result<Option<Popup>, String>,
    pub explosion: Option<Result<Impact, String>>,
}

#[derive(Clone, Debug)]
struct PreparedInterval {
    boundary: Boundary,
    /// A number taken before a seal may be published in a later interval.
    in_flight: bool,
    coverage: [bool; MapEventFamily::COUNT],
    coverage_refusals: Vec<String>,
    events: Vec<PreparedEvent>,
}

/// One hour's map events interpreted for one viewing country on an hour
/// builder: localisation, country joins and visibility are decided there, so
/// presenting it only admits and queues.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// The viewing country; its fog switch only changes admission.
    view: u32,
    interval: Result<PreparedInterval, String>,
}

impl Prepared {
    /// The hour's interval for `context`'s viewer. `intervals` are the
    /// hour's `MapEvents` rows and `table_complete` whether the table was
    /// read whole. `locate` gives a province's standstill unit-stack anchor
    /// and admission box for an explosion.
    pub fn new(
        context: &Context<'_>,
        intervals: &[MapEventsRow],
        table_complete: bool,
        locate: impl Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    ) -> Self {
        let view = context.vision.ours();
        let interval = (|| {
            let [row] = intervals else {
                return Err(
                    "map text interval unavailable or not a singleton; coverage unknown".to_owned(),
                );
            };
            if row.to_hour != context.frame.hour {
                return Err("map text interval is not this hour's; coverage unknown".into());
            }
            row.validate()
                .map_err(|error| format!("map text malformed interval: {error}"))?;
            let mut coverage = [false; MapEventFamily::COUNT];
            let mut coverage_refusals = Vec::new();
            let mut in_flight = false;
            for family in MapEventFamily::ALL {
                let health = row.health(family);
                in_flight |= health.faults.contains(MapEventFault::InFlight);
                coverage[family.index()] = table_complete && health.complete;
                if !coverage[family.index()] {
                    coverage_refusals.push(format!(
                        "map text {} coverage unknown ({}; {} refused)",
                        family.name(),
                        health
                            .faults
                            .iter()
                            .map(|fault| fault.name())
                            .collect::<Vec<_>>()
                            .join(", "),
                        health.missed
                    ));
                }
            }
            let events = row
                .events
                .iter()
                .map(|event| PreparedEvent {
                    sequence: event.sequence,
                    family: event.payload.family(),
                    popup: context.popup(event),
                    explosion: match &event.payload {
                        MapEventPayload::CriticalShoreBombardment(critical) => Some(
                            locate(critical.province)
                                .map(|(position, bounds)| Impact {
                                    id: event.sequence,
                                    province: critical.province,
                                    effect: Effect::CriticalBombardment,
                                    position,
                                    bounds,
                                })
                                .ok_or_else(|| {
                                    "critical shore bombardment has no explosion: its province has no standstill anchor or painted box".to_owned()
                                }),
                        ),
                        _ => None,
                    },
                })
                .collect();
            Ok(PreparedInterval {
                boundary: Boundary {
                    branch: context.frame.branch,
                    epoch: row.epoch,
                    from_hour: row.from_hour,
                    to_hour: row.to_hour,
                    first: row.first_sequence,
                    next: row.next_sequence,
                },
                in_flight,
                coverage,
                coverage_refusals,
                events,
            })
        })();
        Self { view, interval }
    }

    /// The hour's table could not be read at all.
    pub fn unavailable(view: u32, reason: String) -> Self {
        Self {
            view,
            interval: Err(reason),
        }
    }
}

/// One presented interval's popups and explosions, and whether the queues
/// start again.
pub struct Batch {
    pub reset: bool,
    pub popups: Vec<Popup>,
    /// Critical shore bombardments presented for the first time.
    pub explosions: Vec<Impact>,
    pub coverage: [bool; MapEventFamily::COUNT],
    pub refusals: Refusals,
}

/// The events of one branch and epoch already presented: every sequence
/// below `fence`, and the ones in `above`. An interval sealed while a number
/// was in flight leaves the numbers it listed above the fence, so a later
/// interval that lists the late one still presents it.
#[derive(Default)]
struct Consumed {
    key: Option<(BranchId, u64)>,
    fence: u64,
    above: std::collections::BTreeSet<u64>,
}

impl Consumed {
    fn contains(&self, sequence: u64) -> bool {
        sequence < self.fence || self.above.contains(&sequence)
    }
}

/// Consumes each presented interval's events once, in order.
#[derive(Default)]
pub struct Consumer {
    boundary: Option<Boundary>,
    /// Gaps break queue continuity but never make a consumed event new again.
    consumed: Consumed,
    view: Option<u32>,
}

impl Consumer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A presented hour that failed breaks continuity without forgetting
    /// consumed events.
    pub fn interrupt(&mut self) {
        self.boundary = None;
    }

    /// Every presented hour, including an unavailable or empty interval. A
    /// repeated interval is consumed once.
    pub fn present(&mut self, prepared: &Prepared, continuity: Continuity) -> Batch {
        let mut out = Batch {
            reset: continuity == Continuity::Reset || self.view != Some(prepared.view),
            popups: Vec::new(),
            explosions: Vec::new(),
            coverage: [false; MapEventFamily::COUNT],
            refusals: BTreeMap::new(),
        };
        if out.reset {
            self.boundary = None;
            self.consumed = Consumed::default();
        }
        self.view = Some(prepared.view);
        let interval = match &prepared.interval {
            Ok(interval) => interval,
            Err(reason) => {
                self.boundary = None;
                out.reset = true;
                refuse(&mut out.refusals, reason.clone());
                return out;
            }
        };
        let next = interval.boundary;
        let repeated = self.boundary == Some(next);
        if !repeated
            && let Some(previous) = self.boundary
            && (previous.branch != next.branch
                || previous.epoch != next.epoch
                || previous.to_hour != next.from_hour
                || previous.next != next.first)
        {
            out.reset = true;
            refuse(
                &mut out.refusals,
                "map text interval discontinuity; intervening coverage unknown".into(),
            );
        }
        self.boundary = Some(next);
        out.coverage = interval.coverage;
        for reason in &interval.coverage_refusals {
            refuse(&mut out.refusals, reason.clone());
        }
        if repeated {
            return out;
        }
        if self.consumed.key != Some((next.branch, next.epoch)) {
            self.consumed = Consumed {
                key: Some((next.branch, next.epoch)),
                ..Default::default()
            };
        }
        for event in &interval.events {
            if self.consumed.contains(event.sequence) {
                continue;
            }
            self.consumed.above.insert(event.sequence);
            match &event.popup {
                Ok(Some(popup)) => out.popups.push(popup.clone()),
                Ok(None) => {}
                Err(reason) => refuse(
                    &mut out.refusals,
                    format!("map text {}: {reason}", event.family.name()),
                ),
            }
            match &event.explosion {
                Some(Ok(impact)) => out.explosions.push(impact.clone()),
                Some(Err(reason)) => refuse(&mut out.refusals, reason.clone()),
                None => {}
            }
        }
        // Every number below a seal that found nothing in flight was listed.
        if !interval.in_flight {
            self.consumed.fence = self.consumed.fence.max(next.next);
            let fence = self.consumed.fence;
            self.consumed.above.retain(|sequence| *sequence >= fence);
        }
        out
    }
}

/// The scene's map event presentation: the hour last presented, built or
/// failed, decides whether consumption starts again, so a failed build
/// retried at the same hour presents nothing twice.
#[derive(Default)]
pub struct Presenter {
    consumer: Consumer,
    at: Option<(BranchId, i32)>,
}

impl Presenter {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Before presenting hours ending at `at`: the first one's continuity.
    pub fn begin(&mut self, at: (BranchId, i32), seek: bool) -> Continuity {
        let reset = presentation_reset(self.at, at, seek);
        self.at = Some(at);
        if reset {
            Continuity::Reset
        } else {
            Continuity::Continue
        }
    }

    pub fn present(&mut self, prepared: &Prepared, continuity: Continuity) -> Batch {
        self.consumer.present(prepared, continuity)
    }

    /// A presented hour that failed: consumption starts again only where a
    /// built hour's would. Returns whether it did.
    pub fn fail(&mut self, at: (BranchId, i32), seek: bool) -> bool {
        let reset = presentation_reset(self.at, at, seek);
        self.at = Some(at);
        if reset {
            self.consumer.reset();
        } else {
            self.consumer.interrupt();
        }
        reset
    }
}

/// Whether presenting `next` after `previous` starts the queues again: a
/// seek, a branch change or a step back.
pub fn presentation_reset(
    previous: Option<(BranchId, i32)>,
    next: (BranchId, i32),
    seek: bool,
) -> bool {
    seek || previous.is_some_and(|previous| previous.0 != next.0 || next.1 < previous.1)
}

#[cfg(test)]
#[path = "events/tests.rs"]
mod tests;
