
use content_reader_defines::defines::Defines;
use crate::{counter_groups, land_windows};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::collections::{BTreeMap, BTreeSet};
use super::{CounterFrame, CounterInputs, invalid};
use viewer_foundation::perspective::{SharingUnknown, Vision};
use viewer_foundation::tables::{Available, rows};
use viewer_foundation::unit_rules::{DEFENDING, STANDSTILL};
use viewer_gpu::icon_tracks::IconFamily;
use viewer_engagements::engagement_history::Provenance;
use viewer_engagements::icon_frames::{self, Kept};
use viewer_native_gui::gui_resources::viewport;
use viewer_engagements::{
    engaged::Engaged,
    engagement_progress::{self as progress, FIXED},
    land_combat_math::{ScoreDefines, ScoreGap, conditions, side_scores},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandCombatDefines {
    pub colors: [[f32; 4]; 3],
    pub scores: ScoreDefines,
}

impl LandCombatDefines {
    pub fn read(defines: &Defines) -> io::Result<Self> {
        let color = |name: &str| -> io::Result<[f32; 4]> {
            match defines.numbers("NGraphics", name)?[..] {
                [red, green, blue, alpha] => {
                    Ok([red as f32, green as f32, blue as f32, alpha as f32])
                }
                _ => Err(invalid(format!(
                    "NDefines.NGraphics.{name} is not an RGBA colour"
                ))),
            }
        };
        Ok(Self {
            colors: [
                color("LAND_COMBAT_WINNING_COLOR")?,
                color("LAND_COMBAT_LOSING_COLOR")?,
                color("LAND_COMBAT_BALANCED_COLOR")?,
            ],
            scores: ScoreDefines::read(defines)?,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const VANILLA: Self = Self {
        colors: [
            [0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
        ],
        scores: ScoreDefines::VANILLA,
    };
}

pub fn combat_tint(
    mut scores: [i64; 2],
    defending: bool,
    colors: &[[f32; 4]; 3],
) -> [f32; 4] {
    if defending {
        scores.swap(0, 1);
    }
    if scores[0] > scores[1].wrapping_mul(110_000) / FIXED {
        colors[0]
    } else if scores[1] > scores[0].wrapping_mul(110_000) / FIXED {
        colors[1]
    } else {
        colors[2]
    }
}

fn progress_text(
    localisation: &content_reader_localisation::localisation::Localisation,
    value: i64,
) -> Result<String, String> {
    const KEY: &str = "COMBAT_ICON_PROGRESS";
    localisation
        .localise_with(
            KEY.as_bytes(),
            &[(
                "VAL",
                content_reader_localisation::localisation::Argument::Fixed5(value),
            )],
        )
        .map_err(|refusal| refusal.to_string())
}

const NO_SIDE_ROW: &str =
    "icon omitted (it exists only while both sides' country lists are non-empty)";

const BOMBARDMENT: u32 = (1 << 11) | (1 << 12);

#[derive(Clone, Debug, PartialEq)]
pub struct LandMarker {
    pub id: u64,
    pub province: u32,
    pub attacker: Option<(u32, u32)>,
    pub seen: bool,
    pub progress: Option<i64>,
    pub tint: Option<[f32; 4]>,
    pub frame: Option<u32>,
    pub bombardment: bool,
}

impl LandMarker {
    pub fn province(&self) -> u32 {
        self.province
    }
}

pub type Gaps = BTreeMap<String, BTreeSet<u64>>;

fn gap(gaps: &mut Gaps, cause: String, combat: u64) {
    gaps.entry(cause).or_default().insert(combat);
}

#[derive(Clone, Copy, Debug)]
pub enum Held<'a> {
    Complete,
    Truncated(u32),
    Unavailable(&'a str),
    NotRecorded(replay_data::scope::CaptureMode),
}

impl<'a> Held<'a> {
    fn of<T: replay_data::rows::TableSchema>(table: &Available<'a, T>) -> Self {
        match table {
            Available::Complete(_) => Self::Complete,
            Available::Truncated { missing, .. } => Self::Truncated(*missing),
            Available::Unavailable { reason } => Self::Unavailable(reason),
            Available::NotRecorded { mode } => Self::NotRecorded(*mode),
        }
    }
}

pub fn missing_of(held: Held<'_>, name: &str, key: impl std::fmt::Display) -> String {
    match held {
        Held::Complete => format!("the {name} table has no row for {key}"),
        Held::Truncated(_) => {
            format!("the truncated {name} table did not reach {key}")
        }
        Held::Unavailable(reason) => {
            format!("the {name} table is unavailable ({reason}), so {key} is unknown")
        }
        Held::NotRecorded(mode) => {
            format!("the {name} table is not recorded in a {mode} recording, so {key} is unknown")
        }
    }
}

pub fn whole_of(held: Held<'_>, name: &str) -> Option<String> {
    match held {
        Held::Complete => None,
        Held::Truncated(missing) => Some(format!("the {name} table is truncated ({missing} rows unread)")),
        Held::Unavailable(reason) => Some(format!("the {name} table is unavailable ({reason})")),
        Held::NotRecorded(mode) => Some(format!("the {name} table is not recorded in a {mode} recording")),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CombatFields<'a> {
    pub id: u64,
    pub province: u32,
    pub attackers: &'a [u64],
    pub defenders: &'a [u64],
}

#[derive(Clone, Debug, Default)]
pub struct SideFields {
    pub indicator: u8,
    pub flags: [u32; 2],
    pub attacker_primary: Vec<u32>,
    pub defender_primary: Vec<u32>,
    pub defender_secondary: Vec<u32>,
}

impl SideFields {
    fn of(row: &replay_data::rows::LandCombatSideRow) -> Self {
        Self {
            indicator: row.indicator,
            flags: row.flags,
            attacker_primary: row.attacker_primary.clone(),
            defender_primary: row.defender_primary.clone(),
            defender_secondary: row.defender_secondary.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TintHeld<'a> {
    pub details: Held<'a>,
    pub divisions: Held<'a>,
    pub panels: Held<'a>,
    pub members: Held<'a>,
}

pub fn marker_tint(
    scores: Option<Result<[i64; 2], ScoreGap>>,
    defending: bool,
    defines: &LandCombatDefines,
    held: TintHeld<'_>,
) -> (Option<[f32; 4]>, Option<String>) {
    match scores {
        None => (None, Some(missing_of(held.details, "LandCombatDetails", "the combat"))),
        Some(Ok(scores)) => (Some(combat_tint(scores, defending, &defines.colors)), None),
        Some(Err(ScoreGap::Division(id))) => (
            None,
            Some(missing_of(held.divisions, "Divisions", format!("division {id}"))),
        ),
        Some(Err(ScoreGap::MaximumStrength(id))) => (
            None,
            Some(missing_of(
                held.panels,
                "DivisionPanelStats",
                format!(
                    "committed division {id}, whose maximum hit points the early damage estimate reads"
                ),
            )),
        ),
        Some(Err(ScoreGap::Member { side, division })) => (
            None,
            Some(missing_of(
                held.members,
                "LandCombatDivisions",
                format!("committed division {division} on side {side}"),
            )),
        ),
        Some(Err(gap @ ScoreGap::Numbers { .. })) => (None, Some(gap.reason())),
    }
}

pub trait MarkerJoins {
    fn sides(&self, combat: u64) -> Result<SideFields, String>;
    fn first_attacker(&self, attackers: &[u64]) -> Result<Option<(u32, u32)>, &'static str>;
    fn flag(&self, combat: u64) -> Engaged;
    fn kept(&self, combat: u64) -> Kept;
    fn defending(&self, sides: &SideFields) -> Result<bool, SharingUnknown>;
    fn tint(&self, combat: &CombatFields<'_>, defending: bool) -> (Option<[f32; 4]>, Option<String>);
    fn progress(&self, combat: &CombatFields<'_>) -> Result<i64, u64>;
    fn missing_division(&self, division: u64) -> String;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkerOutcome {
    pub marker: Option<LandMarker>,
    pub gaps: Vec<String>,
    pub limits: Vec<String>,
}

pub fn land_marker(combat: CombatFields<'_>, joins: &impl MarkerJoins) -> MarkerOutcome {
    let marker = (|| {
        let sides = joins
            .sides(combat.id)
            .map_err(|missing| format!("{NO_SIDE_ROW}: {missing}"))?;
        if sides.attacker_primary.is_empty() || sides.defender_primary.is_empty() {
            return Ok(None);
        }
        let attacker = joins
            .first_attacker(combat.attackers)
            .map_err(|reason| format!("icon omitted: {reason}"))?;
        let seen = match joins.flag(combat.id) {
            Engaged::Yes => true,
            Engaged::No => false,
            Engaged::Ambiguous(cause) => {
                return Err(format!("icon omitted: {}", cause.reason()));
            }
            Engaged::Unknown => {
                return Err(
                    "icon omitted: engaged flag unknown (no refresh seen since the branch joined a running game or skipped hours, or an input the test reads is unrecorded)"
                        .to_owned(),
                );
            }
        };
        if !seen {
            return Ok(Some((
                LandMarker {
                    id: combat.id,
                    province: combat.province,
                    attacker,
                    seen,
                    progress: None,
                    tint: Some([1.0; 4]),
                    frame: match joins.kept(combat.id) {
                        Kept::Frame(frame) => Some(frame),
                        _ => None,
                    },
                    bombardment: false,
                },
                None,
            )));
        }
        let defending = joins
            .defending(&sides)
            .map_err(|cause| format!("icon omitted: {}", progress::undefended(cause)))?;
        let (tint, tint_gap) = joins.tint(&combat, defending);
        Ok(Some((
            LandMarker {
                id: combat.id,
                province: combat.province,
                attacker,
                seen,
                progress: joins.progress(&combat).ok().map(|attacker| {
                    let side = if defending {
                        FIXED - attacker
                    } else {
                        attacker
                    };
                    side.wrapping_mul(10_000_000) / FIXED
                }),
                tint,
                frame: Some(u32::from(sides.indicator != 0)),
                bombardment: (sides.flags[0] | sides.flags[1]) & BOMBARDMENT != 0,
            },
            tint_gap,
        )))
    })();
    let mut out = MarkerOutcome::default();
    match marker {
        Ok(None) => {}
        Ok(Some((marker, tint_gap))) => {
            if marker.seen {
                if let Err(division) = joins.progress(&combat) {
                    out.gaps.push(format!(
                        "percentage shows '?' (progress): {}",
                        joins.missing_division(division)
                    ));
                }
                if let Some(cause) = tint_gap {
                    out.gaps
                        .push(format!("plate and wedge omitted (survival estimate): {cause}"));
                }
            } else {
                out.limits
                    .push(joins.kept(combat.id).limitation(icon_frames::LAND_NO_HISTORY));
            }
            out.marker = Some(marker);
        }
        Err(cause) => out.gaps.push(cause),
    }
    out
}

struct HourJoins<'a, R> {
    vision: &'a Vision,
    resolve: &'a R,
    engaged: &'a BTreeMap<u64, Engaged>,
    kept: &'a BTreeMap<u64, Kept>,
    sides: &'a BTreeMap<u64, &'a replay_data::rows::LandCombatSideRow>,
    sides_held: Held<'a>,
    provinces: &'a BTreeMap<u64, u32>,
    kinds: &'a BTreeMap<u64, (Option<u32>, bool)>,
    details: &'a BTreeMap<u64, &'a replay_data::rows::LandCombatDetailsRow>,
    members: &'a BTreeMap<u64, Vec<replay_data::rows::LandCombatMember<'a>>>,
    conditions: &'a viewer_engagements::land_combat_math::Conditions,
    progresses: &'a progress::Progresses,
    defines: &'a LandCombatDefines,
    held: TintHeld<'a>,
}

impl<R: Fn(u32) -> Option<u32>> MarkerJoins for HourJoins<'_, R> {
    fn sides(&self, combat: u64) -> Result<SideFields, String> {
        self.sides
            .get(&combat)
            .map(|row| SideFields::of(row))
            .ok_or_else(|| missing_of(self.sides_held, "LandCombatSides", "the combat"))
    }

    fn first_attacker(&self, attackers: &[u64]) -> Result<Option<(u32, u32)>, &'static str> {
        first_attacker(attackers, self.provinces, self.kinds)
    }

    fn flag(&self, combat: u64) -> Engaged {
        self.engaged.get(&combat).copied().unwrap_or(Engaged::Unknown)
    }

    fn kept(&self, combat: u64) -> Kept {
        self.kept.get(&combat).copied().unwrap_or(Kept::Undecided)
    }

    fn defending(&self, sides: &SideFields) -> Result<bool, SharingUnknown> {
        progress::defending_of(&sides.defender_primary, &sides.defender_secondary, self.vision, self.resolve)
    }

    fn tint(&self, combat: &CombatFields<'_>, defending: bool) -> (Option<[f32; 4]>, Option<String>) {
        let scores = self.details.get(&combat.id).map(|detail| {
            side_scores(
                detail,
                self.members.get(&combat.id).into_iter().flatten().copied(),
                self.conditions,
                [combat.attackers, combat.defenders],
                self.defines.scores,
            )
        });
        marker_tint(scores, defending, self.defines, self.held)
    }

    fn progress(&self, combat: &CombatFields<'_>) -> Result<i64, u64> {
        match self.progresses.attacker(combat.id) {
            Some(attacker) => Ok(attacker),
            None => Err(self.progresses.unrecorded(combat.id).unwrap_or(0)),
        }
    }

    fn missing_division(&self, division: u64) -> String {
        missing_of(self.held.divisions, "Divisions", format!("division {division}"))
    }
}

#[derive(Debug, Default)]
pub struct LandMarkers {
    pub markers: Vec<LandMarker>,
    pub gaps: Gaps,
    pub limits: Gaps,
}

impl LandMarkers {
    pub fn read(
        frame: &DecodedFrame,
        vision: &Vision,
        links: &BTreeMap<u32, u32>,
        (engaged, provenance): &(BTreeMap<u64, Engaged>, Provenance),
        kept: &BTreeMap<u64, Kept>,
        movement: &crate::Movements,
        defines: &LandCombatDefines,
    ) -> io::Result<Self> {
        let divisions = rows::<tables::Divisions>(frame)?;
        let conditions = conditions(frame)?;
        let provinces: BTreeMap<_, _> = divisions
            .rows()
            .iter()
            .map(|row| (row.id, row.province))
            .collect();
        let kinds: BTreeMap<_, _> = rows::<tables::DivisionMovement>(frame)?
            .rows()
            .iter()
            .map(|row| {
                let kind = movement
                    .get(row.division)
                    .filter(|route| route.kind_resolved)
                    .map(|route| route.stack_kind);
                (row.division, (kind, row.map_entity != 0))
            })
            .collect();
        let panels = rows::<tables::DivisionPanelStats>(frame)?;
        let progresses = progress::Progresses::read(frame)?;
        let sides_table = rows::<tables::LandCombatSides>(frame)?;
        let sides = progress::sides(frame)?;
        let details_table = rows::<tables::LandCombatDetails>(frame)?;
        let details: BTreeMap<_, _> = details_table.rows().iter().map(|r| (r.combat, r)).collect();
        let members = rows::<tables::LandCombatDivisions>(frame)?;
        let member_values =
            viewer_engagements::land_combat_math::member_values(frame, members.rows())?;
        let mut members_by_combat: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for row in replay_data::rows::land_combat_members(members.rows(), &member_values) {
            members_by_combat.entry(row.combat).or_default().push(row);
        }
        let resolve = |raw: u32| links.get(&raw).copied();
        let mut out = Self::default();
        let combats = rows::<tables::LandCombats>(frame)?.rows();
        if let Some(limit) = provenance.limitation()
            && !combats.is_empty()
        {
            out.limits
                .insert(limit.to_owned(), combats.iter().map(|c| c.id).collect());
        }
        let joins = HourJoins {
            vision,
            resolve: &resolve,
            engaged,
            kept,
            sides: &sides,
            sides_held: Held::of(&sides_table),
            provinces: &provinces,
            kinds: &kinds,
            details: &details,
            members: &members_by_combat,
            conditions: &conditions,
            progresses: &progresses,
            defines,
            held: TintHeld {
                details: Held::of(&details_table),
                divisions: Held::of(&divisions),
                panels: Held::of(&panels),
                members: Held::of(&members),
            },
        };
        for combat in combats {
            let fields = CombatFields {
                id: combat.id,
                province: combat.province,
                attackers: &combat.attackers,
                defenders: &combat.defenders,
            };
            let outcome = land_marker(fields, &joins);
            for cause in outcome.gaps {
                gap(&mut out.gaps, cause, combat.id);
            }
            for limit in outcome.limits {
                gap(&mut out.limits, limit, combat.id);
            }
            out.markers.extend(outcome.marker);
        }
        Ok(out)
    }
}

pub fn first_attacker(
    attackers: &[u64],
    provinces: &BTreeMap<u64, u32>,
    kinds: &BTreeMap<u64, (Option<u32>, bool)>,
) -> Result<Option<(u32, u32)>, &'static str> {
    for id in attackers {
        match kinds.get(id) {
            Some(&(_, false)) => {}
            Some(&(kind, true)) => {
                let province = *provinces.get(id).ok_or(
                    "the first mapped attacker's province is unrecorded (it has no Divisions row)",
                )?;
                let kind = kind.ok_or(
                    "the first mapped attacker's stack kind is unknown (content cannot place its route)",
                )?;
                return Ok(Some((province, kind)));
            }
            None => {
                return Err(
                    "an attacker's map entity is unrecorded (it has no DivisionMovement row), so the icon's first mapped attacker is unknown",
                );
            }
        }
    }
    Ok(None)
}

pub fn combat_direction(
    inputs: &CounterInputs<'_>,
    province: u32,
    attacker_anchor: Option<(u32, u32)>,
) -> Result<([f32; 3], f32), String> {
    let (defender, attacker) = combat_anchors(
        |province, kind| inputs.content.anchor(province, kind),
        province,
        attacker_anchor,
    )?;
    Ok(combat_midpoint(inputs.camera.map.x(), defender, attacker))
}

pub fn combat_anchors(
    anchor: impl Fn(u32, u32) -> Option<[f32; 3]>,
    province: u32,
    attacker: Option<(u32, u32)>,
) -> Result<([f32; 3], [f32; 3]), String> {
    let defender = anchor(province, DEFENDING)
        .or_else(|| anchor(province, STANDSTILL))
        .ok_or_else(|| {
            format!(
                "the content places neither a Defending nor a Standstill unit-stack anchor for combat province {province}"
            )
        })?;
    let Some((at, kind)) = attacker else {
        return Ok((defender, defender));
    };
    let attacker = anchor(at, kind)
        .or_else(|| anchor(at, STANDSTILL))
        .ok_or_else(|| {
            format!(
                "the content places neither a unit-stack anchor of kind {kind} nor a Standstill one for the first mapped attacker's province {at}"
            )
        })?;
    Ok((defender, attacker))
}

pub fn combat_midpoint(
    width: f32,
    defender: [f32; 3],
    attacker: [f32; 3],
) -> ([f32; 3], f32) {
    let attacker = [
        counter_groups::unwrap_x(width, defender[0], attacker[0]),
        attacker[1],
        attacker[2],
    ];
    let midpoint = [
        (attacker[0] + defender[0]) * 0.5,
        (attacker[1] + defender[1]) * 0.5,
        (attacker[2] + defender[2]) * 0.5,
    ];
    let angle = (attacker[0] - defender[0]).atan2(-(attacker[2] - defender[2]));
    (midpoint, angle)
}

pub fn land_combats(
    inputs: &CounterInputs<'_>,
    markers: &LandMarkers,
    out: &mut CounterFrame<impl Sized, impl Sized, impl Sized, impl Sized>,
) -> io::Result<()> {
    for (cause, combats) in &markers.gaps {
        out.report
            .combat_gaps
            .entry(("Land", cause.clone()))
            .or_default()
            .extend(combats);
    }
    for (limit, combats) in &markers.limits {
        out.report
            .combat_limits
            .entry(("Land", limit.clone()))
            .or_default()
            .extend(combats);
    }
    if !inputs.visibility.province_icons || !inputs.shows(inputs.content.defines.land_combat_cutoff)
    {
        return Ok(());
    }
    for marker in &markers.markers {
        let (anchor, angle) = match combat_direction(inputs, marker.province, marker.attacker) {
            Ok(direction) => direction,
            Err(cause) => {
                out.report
                    .combat_gaps
                    .entry(("Land", format!("icon omitted: {cause}")))
                    .or_default()
                    .insert(marker.id);
                continue;
            }
        };
        let Some(projection) = inputs.camera.project_map_icon(
            anchor,
            inputs.viewport,
            viewport(inputs.viewport, inputs.ui_scale),
        ) else {
            continue;
        };
        let text = match marker.progress {
            Some(value) => progress_text(inputs.localisation, value),
            None => Ok("?".to_owned()),
        }
        .unwrap_or_else(|cause| {
            out.report
                .combat_gaps
                .entry(("Land", format!("progress text refused: {cause}")))
                .or_default()
                .insert(marker.id);
            String::new()
        });
        out.native_land_combats.placed.push(land_windows::Placed {
            marker: marker.clone(),
            screen: projection.position,
            depth: projection.depth,
            angle,
            terrain: inputs
                .content
                .terrain
                .combat_terrain
                .get(&marker.province)
                .cloned(),
            text,
            anchor: inputs.icon_anchor(
                IconFamily::Combats,
                anchor,
                inputs.content.defines.land_combat_cutoff,
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BTreeMap, DEFENDING, DecodedFrame, Engaged, Gaps, LandCombatDefines, LandMarkers, Provenance, STANDSTILL, Vision, combat_anchors, progress_text};
    use replay_data::{
        rows::DivisionMovementRow, rows::DivisionRow, rows::DivisionStatusRow,
        rows::LandCombatDetailsRow, rows::LandCombatDivisionRow, rows::LandCombatRow,
        rows::LandCombatSideRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 0,
            hour: 3,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not recorded".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        }
    }

    fn land_frame(divisions: &[u64]) -> DecodedFrame {
        let mut frame = frame();
        *frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                id: 62,
                province: 9,
                attacker_classes: vec![0; 2],
                attackers: vec![1, 2],
                defenders: vec![3],
                ..Default::default()
            }]));
        *frame.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 62,
                attacker_primary: vec![5],
                defender_primary: vec![6],
                ..Default::default()
            }]));
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
            divisions
                .iter()
                .map(|&id| DivisionRow {
                    id,
                    organisation: 10,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(
                divisions
                    .iter()
                    .map(|&division| DivisionStatusRow {
                        division,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::DivisionPanelStats.index()] =
            TableView::Complete(TableRows::DivisionPanelStats(
                divisions
                    .iter()
                    .map(|&division| {
                        let mut row = replay_data::rows::DivisionPanelStatsRow {
                            division,
                            ..Default::default()
                        };
                        row.panel_stats[0] = Some(1_000_000);
                        row.panel_stats[1] = Some(1_000_000);
                        row
                    })
                    .collect(),
            ));
        *frame.tables[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(
                [1, 2]
                    .map(|division| DivisionMovementRow {
                        division,
                        ..Default::default()
                    })
                    .to_vec(),
            ));
        frame
    }

    fn land_gaps(frame: &DecodedFrame) -> Gaps {
        let links = BTreeMap::from([(5, 5), (6, 6)]);
        let engaged = (BTreeMap::from([(62, Engaged::Yes)]), Provenance::Indexed);
        let markers = LandMarkers::read(
            frame,
            &Vision::sharing(5, []),
            &links,
            &engaged,
            &BTreeMap::new(),
            &crate::Movements::default(),
            &LandCombatDefines::VANILLA,
        )
        .unwrap();
        assert!(markers.limits.is_empty(), "{:?}", markers.limits);
        markers.gaps
    }

    fn causes(gaps: &Gaps) -> Vec<(&str, Vec<u64>)> {
        gaps.iter()
            .map(|(cause, combats)| (cause.as_str(), combats.iter().copied().collect()))
            .collect()
    }

    #[test]
    fn the_progress_box_localises_its_fixed_point_value() {
        let localisation: content_reader_localisation::localisation::Localisation =
            [("COMBAT_ICON_PROGRESS", "§Y$VAL|1$§! pct")]
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();
        let value = 4_567_000;
        assert_eq!(
            progress_text(&localisation, value).as_deref(),
            Ok("\u{11}Y45.6\u{11}! pct")
        );
        let empty = content_reader_localisation::localisation::Localisation::default();
        assert_eq!(
            progress_text(&empty, value).as_deref(),
            Ok("COMBAT_ICON_PROGRESS")
        );
    }

    #[test]
    fn land_gaps_name_the_combat_and_each_part_s_own_cause() {
        let frame = land_frame(&[1, 3]);
        assert_eq!(
            causes(&land_gaps(&frame)),
            [
                (
                    "percentage shows '?' (progress): the Divisions table has no row for division 2",
                    vec![62]
                ),
                (
                    "plate and wedge omitted (survival estimate): the LandCombatDetails table is unavailable (not recorded), so the combat is unknown",
                    vec![62]
                ),
            ]
        );
    }

    #[test]
    fn a_missing_survival_estimate_names_its_operand() {
        let mut frame = land_frame(&[1, 2, 3]);
        let mut detail = LandCombatDetailsRow {
            combat: 62,
            ..Default::default()
        };
        detail.sides[0].committed = vec![1];
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(vec![detail]));
        let TableView::Complete(TableRows::Divisions(divisions)) =
            &mut *frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        let second = divisions.remove(1);
        assert_eq!(
            causes(&land_gaps(&frame)),
            [
                (
                    "percentage shows '?' (progress): the Divisions table has no row for division 2",
                    vec![62]
                ),
                (
                    "plate and wedge omitted (survival estimate): the Divisions table has no row for division 2",
                    vec![62]
                ),
            ]
        );
        let TableView::Complete(TableRows::Divisions(divisions)) =
            &mut *frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        divisions.insert(1, second);
        *frame.tables[TableId::LandCombatDivisions.index()] = TableView::Truncated {
            rows: TableRows::LandCombatDivisions(Vec::new()),
            missing: 4,
        };
        assert_eq!(
            causes(&land_gaps(&frame)),
            [(
                "plate and wedge omitted (survival estimate): the truncated LandCombatDivisions table did not reach committed division 1 on side 0",
                vec![62]
            )]
        );
        *frame.tables[TableId::LandCombatDivisions.index()] =
            TableView::Complete(TableRows::LandCombatDivisions(vec![
                LandCombatDivisionRow {
                    combat: 62,
                    side: 0,
                    division: 1,
                    own: Some(replay_data::rows::LandCombatMemberValues {
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ]));
        let TableView::Complete(TableRows::DivisionPanelStats(panels)) =
            &mut *frame.tables[TableId::DivisionPanelStats.index()]
        else {
            unreachable!()
        };
        panels.retain(|row| row.division != 2);
        assert_eq!(causes(&land_gaps(&frame)), []);
        let TableView::Complete(TableRows::DivisionPanelStats(panels)) =
            &mut *frame.tables[TableId::DivisionPanelStats.index()]
        else {
            unreachable!()
        };
        panels.retain(|row| row.division != 1);
        assert_eq!(
            causes(&land_gaps(&frame)),
            [(
                "plate and wedge omitted (survival estimate): the DivisionPanelStats table has no row for committed division 1, whose maximum hit points the early damage estimate reads",
                vec![62]
            )]
        );
    }

    #[test]
    fn an_omitted_land_icon_names_its_combat() {
        let mut frame = land_frame(&[1, 2, 3]);
        *frame.tables[TableId::LandCombatSides.index()] = TableView::Truncated {
            rows: TableRows::LandCombatSides(Vec::new()),
            missing: 1,
        };
        assert_eq!(
            causes(&land_gaps(&frame)),
            [(
                "icon omitted (it exists only while both sides' country lists are non-empty): the truncated LandCombatSides table did not reach the combat",
                vec![62]
            )]
        );
    }

    #[test]
    fn combat_anchors_refuse_by_province_and_slot() {
        let anchors = BTreeMap::from([
            ((9, DEFENDING), [1.0, 0.0, 1.0]),
            ((4, STANDSTILL), [2.0, 0.0, 2.0]),
        ]);
        let anchor = |province, kind| anchors.get(&(province, kind)).copied();
        assert_eq!(
            combat_anchors(anchor, 9, None),
            Ok(([1.0, 0.0, 1.0], [1.0, 0.0, 1.0]))
        );
        assert_eq!(
            combat_anchors(anchor, 9, Some((4, 7))),
            Ok(([1.0, 0.0, 1.0], [2.0, 0.0, 2.0]))
        );
        assert_eq!(
            combat_anchors(anchor, 9, Some((5, 7))),
            Err("the content places neither a unit-stack anchor of kind 7 nor a Standstill one for the first mapped attacker's province 5".to_owned())
        );
        assert_eq!(
            combat_anchors(anchor, 8, None),
            Err("the content places neither a Defending nor a Standstill unit-stack anchor for combat province 8".to_owned())
        );
    }
}
