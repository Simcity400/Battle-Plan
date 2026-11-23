use std::{cell::OnceCell, collections::BTreeMap};

use content_reader_key_names::buildings::Buildings;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    BranchId, rows::MapEvent, rows::MapEventBuildingReason, rows::MapEventFamily,
    rows::MapEventFault, rows::MapEventPayload, rows::MapEventUnitIntel, rows::MapEventsRow,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::{perspective::{Resolution, Vision}, visibility};
use viewer_runtime::presentation::Continuity;
use dashboard_text::table_access::{Refusals, TokenKeys, TokenNames, refuse};

use super::{Popup, Style};
use viewer_models::strike_effects::{Effect, Impact};

pub const FAKE_DIVISION_REVEALED: &[u8] = b"INTEL_DIVISION_REVEALED";
pub const RAILWAY_GUN_OVERRUN: &[u8] = b"OVERRUN_POPUP";
pub const CRITICAL_BOMBARDMENT: &[u8] = b"CRITICAL_BOMBARDMENT_DAMAGE";
pub const GENERAL_CAPTURED: &[u8] = b"CAPTURED_GENERAL_MAP_TEXT";
pub const PARADROP_LANDED: &[u8] = b"PROV_TEXT_UNITS_PARACHUTED";
pub const PARADROP_LANDED_WITH_LOSSES: &[u8] = b"PROV_TEXT_UNITS_PARACHUTED_WITH_LOSSES";
pub const BUILDING_ARGUMENT: &str = "BUILDING";
const SHOWN_LEVEL: u16 = 150;

pub fn building_key(reason: MapEventBuildingReason) -> &'static [u8] {
    match reason {
        MapEventBuildingReason::Bombed => b"BUILDING_BOMBED",
        MapEventBuildingReason::Sabotaged => b"BUILDING_SABOTAGED",
        MapEventBuildingReason::Collateral => b"BUILDING_DAMAGE_REASON_COLLATERAL",
        MapEventBuildingReason::Repaired => b"BUILDING_REPAIRED",
        MapEventBuildingReason::Scorched => b"BUILDING_SCORCHED",
    }
}

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
        if ladder == 0 && province == Some(0) && shared.is_ok() {
            return Ok(false);
        }
        unit_admission(ladder, province, self.modifier()?, shared.is_ok()).ok_or_else(|| {
            shared
                .err()
                .unwrap_or_else(|| "map text province fog unknown".into())
        })
    }

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

    fn text(&self, key: &[u8], args: &[(&str, Argument<'_>)]) -> Result<Vec<u8>, String> {
        self.localisation
            .localise_bytes(key, args)
            .map_err(|error| error.to_string())
    }

    pub fn popup(&self, event: &MapEvent) -> Result<Option<Popup>, String> {
        let (province, style, text) = match &event.payload {
            MapEventPayload::FakeDivisionRevealed(unit) => {
                if !self.unit_visible(unit)? {
                    return Ok(None);
                }
                (
                    unit.anchor_province,
                    Style::CombatLoss,
                    self.text(FAKE_DIVISION_REVEALED, &[])?,
                )
            }
            MapEventPayload::RailwayGunOverrun(unit) => {
                if !self.unit_visible(unit)? {
                    return Ok(None);
                }
                (
                    unit.anchor_province,
                    Style::CombatLoss,
                    self.text(RAILWAY_GUN_OVERRUN, &[])?,
                )
            }
            MapEventPayload::CriticalShoreBombardment(critical) => (
                critical.province,
                Style::SiegeInfo,
                self.text(CRITICAL_BOMBARDMENT, &[])?,
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
                    self.text(GENERAL_CAPTURED, &[])?,
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
                (paradrop.province, Style::SiegeInfo, self.text(key, &[])?)
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
                let text = self.text(&activity.key, &[])?;
                if text.is_empty() {
                    return Ok(None);
                }
                (activity.province, Style::SiegeInfo, text)
            }
            MapEventPayload::ScriptedMapRefresh(_) => return Ok(None),
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
                    )?,
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
        return (sharing_known
            || (visibility::unit_byte(base.max(250), modifier) >= SHOWN_LEVEL) == answer)
            .then_some(answer);
    }
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
    in_flight: bool,
    coverage: [bool; MapEventFamily::COUNT],
    coverage_refusals: Vec<String>,
    events: Vec<PreparedEvent>,
}

#[derive(Clone, Debug)]
pub struct Prepared {
    view: u32,
    interval: Result<PreparedInterval, String>,
}

impl Prepared {
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
                coverage[family.index()] = table_complete && health.complete;
                if family == MapEventFamily::ScriptedMapRefresh {
                    continue;
                }
                in_flight |= health.faults.contains(MapEventFault::InFlight);
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

    pub fn read(
        frame: &DecodedFrame,
        vision: &Vision,
        content: &Content,
        localisation: &Localisation,
        locate: impl Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    ) -> Self {
        match dashboard_text::table_access::rows::<tables::MapEvents>(frame) {
            Ok(intervals) => Self::new(
                &Context::new(frame, vision, content, localisation),
                intervals.rows(),
                intervals.is_complete(),
                locate,
            ),
            Err(error) => Self::unavailable(
                vision.ours(),
                format!("map text table malformed: {error}"),
            ),
        }
    }

    pub fn unavailable(view: u32, reason: String) -> Self {
        Self {
            view,
            interval: Err(reason),
        }
    }
}

pub struct Batch {
    pub reset: bool,
    pub popups: Vec<Popup>,
    pub explosions: Vec<Impact>,
    pub coverage: [bool; MapEventFamily::COUNT],
    pub refusals: Refusals,
}

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

#[derive(Default)]
pub struct Consumer {
    boundary: Option<Boundary>,
    consumed: Consumed,
    view: Option<u32>,
}

impl Consumer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn interrupt(&mut self) {
        self.boundary = None;
    }

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
        if !interval.in_flight {
            self.consumed.fence = self.consumed.fence.max(next.next);
            let fence = self.consumed.fence;
            self.consumed.above.retain(|sequence| *sequence >= fence);
        }
        out
    }
}

#[derive(Default)]
pub struct Presenter {
    consumer: Consumer,
    at: Option<(BranchId, i32)>,
}

impl Presenter {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

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

impl Prepared {
    pub fn display(&self) -> map_display_data::MapText {
        use map_display_data as d;
        let impact = |v: &Impact| d::Impact {
            id: v.id,
            province: v.province,
            effect: v.effect as u8,
            position: v.position,
            bounds: v.bounds,
        };
        let popup = |v: &Popup| d::Popup {
            sequence: v.sequence,
            family: v.family as u8,
            province: v.province,
            style: match v.style {
                Style::CombatLoss => 0,
                Style::SiegeInfo => 1,
            },
            text: v.text.clone(),
        };
        d::MapText {
            view: self.view,
            interval: match &self.interval {
                Err(reason) => d::MapTextResult::Unavailable {
                    reason: reason.clone(),
                },
                Ok(i) => d::MapTextResult::Value {
                    interval: d::Interval {
                        branch: i.boundary.branch,
                        epoch: i.boundary.epoch,
                        from_hour: i.boundary.from_hour,
                        to_hour: i.boundary.to_hour,
                        first: i.boundary.first,
                        next: i.boundary.next,
                        in_flight: i.in_flight,
                        coverage: i.coverage.to_vec(),
                        coverage_refusals: i.coverage_refusals.clone(),
                        events: i
                            .events
                            .iter()
                            .map(|e| d::MapEvent {
                                sequence: e.sequence,
                                family: e.family as u8,
                                popup: match &e.popup {
                                    Ok(v) => d::PopupResult::Value {
                                        popup: v.as_ref().map(popup),
                                    },
                                    Err(reason) => d::PopupResult::Refused {
                                        reason: reason.clone(),
                                    },
                                },
                                explosion: e.explosion.as_ref().map(|e| match e {
                                    Ok(v) => d::ImpactResult::Value { impact: impact(v) },
                                    Err(reason) => d::ImpactResult::Refused {
                                        reason: reason.clone(),
                                    },
                                }),
                            })
                            .collect(),
                    },
                },
            },
        }
    }

    pub fn into_display(self) -> map_display_data::MapText {
        use map_display_data as d;
        let impact = |v: Impact| d::Impact {
            id: v.id,
            province: v.province,
            effect: v.effect as u8,
            position: v.position,
            bounds: v.bounds,
        };
        let popup = |v: Popup| d::Popup {
            sequence: v.sequence,
            family: v.family as u8,
            province: v.province,
            style: match v.style {
                Style::CombatLoss => 0,
                Style::SiegeInfo => 1,
            },
            text: v.text,
        };
        d::MapText {
            view: self.view,
            interval: match self.interval {
                Err(reason) => d::MapTextResult::Unavailable {
                    reason,
                },
                Ok(i) => d::MapTextResult::Value {
                    interval: d::Interval {
                        branch: i.boundary.branch,
                        epoch: i.boundary.epoch,
                        from_hour: i.boundary.from_hour,
                        to_hour: i.boundary.to_hour,
                        first: i.boundary.first,
                        next: i.boundary.next,
                        in_flight: i.in_flight,
                        coverage: i.coverage.to_vec(),
                        coverage_refusals: i.coverage_refusals,
                        events: i
                            .events
                            .into_iter()
                            .map(|e| d::MapEvent {
                                sequence: e.sequence,
                                family: e.family as u8,
                                popup: match e.popup {
                                    Ok(v) => d::PopupResult::Value {
                                        popup: v.map(popup),
                                    },
                                    Err(reason) => d::PopupResult::Refused {
                                        reason,
                                    },
                                },
                                explosion: e.explosion.map(|e| match e {
                                    Ok(v) => d::ImpactResult::Value { impact: impact(v) },
                                    Err(reason) => d::ImpactResult::Refused {
                                        reason,
                                    },
                                }),
                            })
                            .collect(),
                    },
                },
            },
        }
    }

    pub fn from_display(value: map_display_data::MapText) -> std::io::Result<Self> {
        use map_display_data as d;
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid map text display operand",
            )
        };
        let family = |id| {
            MapEventFamily::ALL
                .into_iter()
                .find(|f| *f as u8 == id)
                .ok_or_else(invalid)
        };
        let impact = |v: d::Impact| -> std::io::Result<Impact> {
            Ok(Impact {
                id: v.id,
                province: v.province,
                effect: match v.effect {
                    0 => Effect::Nuclear,
                    1 => Effect::Thermonuclear,
                    2 => Effect::CriticalBombardment,
                    3 => Effect::MissileExplosion,
                    _ => return Err(invalid()),
                },
                position: v.position,
                bounds: v.bounds,
            })
        };
        let popup = |v: d::Popup| -> std::io::Result<Popup> {
            Ok(Popup {
                sequence: v.sequence,
                family: family(v.family)?,
                province: v.province,
                style: match v.style {
                    0 => Style::CombatLoss,
                    1 => Style::SiegeInfo,
                    _ => return Err(invalid()),
                },
                text: v.text,
            })
        };
        let interval = match value.interval {
            d::MapTextResult::Unavailable { reason } => Err(reason),
            d::MapTextResult::Value { interval: i } => Ok(PreparedInterval {
                boundary: Boundary {
                    branch: i.branch,
                    epoch: i.epoch,
                    from_hour: i.from_hour,
                    to_hour: i.to_hour,
                    first: i.first,
                    next: i.next,
                },
                in_flight: i.in_flight,
                coverage: i.coverage.try_into().map_err(|_| invalid())?,
                coverage_refusals: i.coverage_refusals,
                events: i
                    .events
                    .into_iter()
                    .map(|e| {
                        Ok(PreparedEvent {
                            sequence: e.sequence,
                            family: family(e.family)?,
                            popup: match e.popup {
                                d::PopupResult::Value { popup: v } => {
                                    Ok(v.map(&popup).transpose()?)
                                }
                                d::PopupResult::Refused { reason } => Err(reason),
                            },
                            explosion: e
                                .explosion
                                .map(|e| -> std::io::Result<_> {
                                    Ok(match e {
                                        d::ImpactResult::Value { impact: v } => Ok(impact(v)?),
                                        d::ImpactResult::Refused { reason } => Err(reason),
                                    })
                                })
                                .transpose()?,
                        })
                    })
                    .collect::<std::io::Result<_>>()?,
            }),
        };
        Ok(Self {
            view: value.view,
            interval,
        })
    }
}
