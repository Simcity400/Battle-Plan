//! Playback's division movement speed against the game's own: the hourly movement step
//! adds the speed it computed for a division's current edge to the division's progress
//! numerator and stores the speed nowhere else, so a division that keeps its location,
//! first route province and route between two consecutive hours advanced its numerator
//! by the game's speed for that edge. Each such pair compares that step with
//! [`eta_derive::edge_speed`] over the earlier hour's recorded terms
//! ([`FrameTerms`]). The step runs inside the next hour's update, after that update's
//! own writers of some operands (supply, organisation, fuel, weather), so a difference
//! names the terms the division's edge took that could have changed in between.

use std::collections::BTreeMap;

use installed_content_core::movement::MovementMap;
use replay_data::{rows::DivisionRow, rows::Recorded, rows::tables};
use replay_derive::{
    eta_derive, eta_derive::Defines, eta_derive::OrganisationDefines, eta_derive::RailDefines,
    eta_derive::frame_terms::FrameTerms, eta_derive::frame_terms::Geography,
    eta_derive::frame_terms::SubUnitContent,
};
use replay_frame::frame::DecodedFrame;

/// The content the movement speed reads.
pub struct Content {
    pub map: MovementMap,
    pub defines: Defines,
    /// Each terrain category's own adjuster movement, in database order.
    pub terrain_movement: Vec<i64>,
    /// Each province's strategic region id.
    pub regions: BTreeMap<u32, u32>,
    /// The sub-unit database: special forces and type masks.
    pub sub_units: Result<installed_content_core::unit_leader::SubUnits, String>,
}

impl Content {
    pub fn read(layers: &installed_content_core::ResolvedLayers) -> std::io::Result<Self> {
        let defines = installed_content_core::defines::read(layers)?;
        let movement = installed_content_core::movement::MovementDefines::read(&defines)?;
        let terrain = installed_content_core::terrain::read_definitions(layers)?;
        let regions = installed_content_regions::regions::read(layers)?;
        Ok(Self {
            map: MovementMap::read(layers, &defines)?,
            defines: Defines {
                embark_distance: movement.embark_distance,
                transport_scale: movement.transport_scale,
                invasion_speed: movement.invasion_speed,
                outer_scale: movement.outer_scale,
                river_crossing_two: movement.river_crossing_two,
                river_crossing_three: movement.river_crossing_three,
                organisation: movement
                    .organisation
                    .ok()
                    .map(|(threshold, penalty)| OrganisationDefines { threshold, penalty }),
                rail: movement.rail.as_ref().ok().map(|rail| RailDefines {
                    speed: rail.speed,
                    maximum_level: rail.maximum_level,
                }),
                noncore_supply: movement
                    .noncore_supply
                    .each_ref()
                    .map(|value| value.as_ref().ok().copied()),
                combat_movement: movement.combat_movement.as_ref().ok().copied(),
            },
            terrain_movement: terrain
                .categories
                .iter()
                .map(|category| category.common_adjuster.movement)
                .collect(),
            regions: regions
                .regions
                .iter()
                .flat_map(|(&id, region)| region.provinces.iter().map(move |&p| (p, id)))
                .collect(),
            sub_units: installed_content_core::unit_leader::sub_units(layers)
                .map_err(|error| error.to_string()),
        })
    }
}

impl SubUnitContent for Content {
    fn special_forces(&self, name: &[u8]) -> Result<bool, String> {
        self.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .special_forces(name)
    }

    fn sub_unit_types(&self, name: &[u8]) -> Result<u64, String> {
        self.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .category(name)
    }
}

impl Geography for Content {
    fn terrain_index(&self, province: u32) -> Option<i32> {
        self.map.terrain(province).map(|terrain| terrain.index)
    }

    fn terrain_flags(&self, province: u32) -> Option<u8> {
        self.map.terrain(province).map(|terrain| terrain.flags)
    }

    fn terrain_movement(&self, index: i32) -> Option<i64> {
        usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|at| self.terrain_movement.get(at))
            .copied()
    }

    fn crossing_kind(&self, from: u32, to: u32) -> Option<u8> {
        self.map.crossing_kind(from, to)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.regions.get(&province).copied()
    }
}

/// The tables the speed check decodes: each mover's Divisions and DivisionMovement
/// rows and every table the speed terms read ([`replay_derive::eta_derive::frame_terms::TABLES`]).
pub fn tables() -> impl Iterator<Item = replay_data::rows::TableId> {
    use replay_data::{rows::TableId::DivisionMovement, rows::TableId::Divisions};
    [Divisions, DivisionMovement]
        .into_iter()
        .chain(replay_derive::eta_derive::frame_terms::TABLES)
}

/// One division's edge at an hour: its location, first route province, route,
/// numerator and the speed playback derives for the edge.
struct Edge {
    location: u32,
    first: u32,
    path: Vec<u32>,
    numerator: i64,
    speed: Result<i64, String>,
    distance: Option<i64>,
    key: MovementRowKey,
}

#[derive(Default)]
pub struct Checker {
    content: Option<Result<Content, String>>,
    previous: Option<(replay_data::BranchId, i32, BTreeMap<u64, Edge>)>,
    /// Pairs whose step equals the derived speed, and those that differ.
    pub matched: u64,
    /// Pairs whose step differs from the earlier hour's derived speed but equals the
    /// later hour's, over the same edge.
    pub matched_later: u64,
    /// Equal progress steps clipped at the native first-edge distance.
    pub matched_clipped: u64,
    pub differed: u64,
    /// Pairs whose numerator did not advance (a stopped or blocked move).
    pub stalled: u64,
    /// Pairs whose derived speed was refused, by reason.
    pub refused: BTreeMap<String, u64>,
    listed: Vec<String>,
    /// Differences by the terms the division's edge took, for diagnosis.
    differences: BTreeMap<String, u64>,
}

impl Checker {
    pub fn new(content: Result<Content, String>) -> Self {
        Self {
            content: Some(content),
            ..Self::default()
        }
    }

    pub fn observe(&mut self, frame: &DecodedFrame) {
        let Some(Ok(content)) = self.content.as_ref() else {
            return;
        };
        let current = edges(frame, content);
        if let Some((branch, hour, previous)) = self.previous.take()
            && branch == frame.branch
            && hour.checked_add(1) == Some(frame.hour)
        {
            for (division, before) in &previous {
                let Some(after) = current.get(division) else {
                    continue;
                };
                if after.location != before.location
                    || after.first != before.first
                    || after.path != before.path
                    || after.numerator < before.numerator
                {
                    continue;
                }
                let step = after.numerator.wrapping_sub(before.numerator);
                match &before.speed {
                    Err(reason) => *self.refused.entry(reason.clone()).or_default() += 1,
                    Ok(_) if step == 0 => self.stalled += 1,
                    Ok(speed)
                        if progress_matches(
                            before.numerator,
                            after.numerator,
                            *speed,
                            before.distance,
                        ) =>
                    {
                        self.matched += 1;
                        self.matched_clipped += u64::from(*speed != step);
                    }
                    // The step runs after the next update's own writers of some operands:
                    // the later hour's terms, over the same edge, can be the ones it read.
                    Ok(_)
                        if after.speed.as_ref().is_ok_and(|speed| {
                            progress_matches(
                                before.numerator,
                                after.numerator,
                                *speed,
                                before.distance,
                            )
                        }) =>
                    {
                        self.matched_later += 1;
                        self.matched_clipped +=
                            u64::from(after.speed.as_ref().is_ok_and(|speed| *speed != step));
                    }
                    Ok(speed) => {
                        self.differed += 1;
                        let kind = classify(frame, *division, &before.key);
                        *self.differences.entry(kind.clone()).or_default() += 1;
                        if self.listed.len() < super::LISTED {
                            self.listed.push(format!(
                                "division {division:#x} hour {}: derived {speed}, stepped {step} ({kind})",
                                frame.hour - 1
                            ));
                        }
                    }
                }
            }
        }
        self.previous = Some((frame.branch, frame.hour, current));
    }

    pub fn print(&self) {
        match &self.content {
            Some(Err(reason)) => println!("division movement speed: content refused: {reason}"),
            _ => {
                println!(
                    "division movement speed against the next hour's numerator step: {} equal, {} equal with the next hour's terms only, {} differing, {} stalled, {} refused",
                    self.matched,
                    self.matched_later,
                    self.differed,
                    self.stalled,
                    self.refused.values().sum::<u64>()
                );
                for (kind, count) in &self.differences {
                    println!("  differing with {kind}: {count}");
                }
                println!(
                    "  equal steps clipped at the first-edge distance: {}",
                    self.matched_clipped
                );
                for (reason, count) in &self.refused {
                    println!("  refused ({reason}): {count}");
                }
                for line in &self.listed {
                    println!("  difference: {line}");
                }
            }
        }
    }

    /// Cross-hour progress categories are diagnostic: writers can change an
    /// operand between either endpoint and the movement step. Only content
    /// failure affects the result.
    pub fn failures(&self) -> [u64; 2] {
        match &self.content {
            Some(Err(_)) => [0, 1],
            _ => [0, 0],
        }
    }
}

/// Movement can stop at the first edge's distance while entry is blocked. The
/// observed numerator then witnesses the clipped step, rather than a smaller
/// speed. No tolerance is applied to either branch.
fn progress_matches(before: i64, after: i64, speed: i64, distance: Option<i64>) -> bool {
    let advanced = before.wrapping_add(speed).max(0);
    after == advanced || distance.is_some_and(|distance| after == distance && advanced >= distance)
}

/// The terms a differing division's edge took that the next hour's update can rewrite
/// first: days out of supply, partial supply, fuel, a retreat and a commander block
/// with entries, its armour speed entry apart.
fn classify(frame: &DecodedFrame, division: u64, movement: &MovementRowKey) -> String {
    let status = frame
        .complete::<tables::DivisionStatus>()
        .ok()
        .and_then(|rows| {
            rows.binary_search_by_key(&division, |row| row.division)
                .ok()
                .map(|at| &rows[at])
        });
    let Some(status) = status else {
        return "no status".into();
    };
    let mut terms = Vec::new();
    if matches!(status.out_of_supply_days, Recorded::Value(Some(days)) if days != 0) {
        terms.push("days out of supply");
    }
    if status.displayed_supply_ratio().is_some_and(|ratio| ratio < 100_000) {
        terms.push("partial supply");
    }
    if movement.fuel_limited {
        terms.push("fuel");
    }
    if movement.retreat {
        terms.push("retreat");
    }
    // A commander block with entries, its armour speed entry counted apart.
    match &status.commander_modifiers {
        Recorded::Value(Some(entries)) if entries[1].is_some() => {
            terms.push("commander armour speed entry");
        }
        Recorded::Value(Some(entries)) if entries.iter().any(Option::is_some) => {
            terms.push("commander");
        }
        _ => {}
    }
    if terms.is_empty() {
        "none of these".into()
    } else {
        terms.join(", ")
    }
}

/// What [`classify`] reads from a division's movement row.
struct MovementRowKey {
    fuel_limited: bool,
    retreat: bool,
}

fn edges(frame: &DecodedFrame, content: &Content) -> BTreeMap<u64, Edge> {
    let mut edges = BTreeMap::new();
    let (Ok(movement), Ok(divisions)) = (
        frame.complete::<tables::DivisionMovement>(),
        frame.complete::<tables::Divisions>(),
    ) else {
        return edges;
    };
    let divisions: BTreeMap<u64, &DivisionRow> =
        divisions.iter().map(|row| (row.id, row)).collect();
    let terms = FrameTerms::new(frame);
    for row in movement {
        let (Some(&first), Some(division)) = (row.path.first(), divisions.get(&row.division))
        else {
            continue;
        };
        if row.path_total == 0 || division.province == 0 || first == 0 {
            continue;
        }
        let speed = terms.with_terms(division, row, content, &content.defines, |terms| {
            eta_derive::edge_speed_with(
                || eta_derive::base_speed(terms),
                division.province,
                first,
                terms,
                &content.defines,
            )
        });
        edges.insert(
            row.division,
            Edge {
                location: division.province,
                first,
                path: row.path.clone(),
                numerator: row.numerator,
                speed,
                distance: content
                    .map
                    .terrain(division.province)
                    .zip(content.map.terrain(first))
                    .and_then(|(from, to)| {
                        if from.flags & 3 == 0 && to.flags & 1 != 0 {
                            Some(content.defines.embark_distance)
                        } else {
                            content.map.distance(division.province, first)
                        }
                    }),
                key: MovementRowKey {
                    // A fuel word below a whole tank is decided by the speed term itself;
                    // here any positive word under the largest recorded fuel is flagged.
                    fuel_limited: matches!(row.fuel, Recorded::Value(Some(fuel)) if fuel > 0),
                    retreat: row.retreat != 0,
                },
            },
        );
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_hour_progress_matches_native_clipping_without_a_tolerance() {
        assert!(progress_matches(
            1_016_240,
            1_016_666,
            5_000,
            Some(1_016_666)
        ));
        assert!(!progress_matches(
            1_016_240,
            1_016_665,
            5_000,
            Some(1_016_666)
        ));
        assert!(!progress_matches(
            1_010_000,
            1_016_666,
            5_000,
            Some(1_016_666)
        ));
        assert!(progress_matches(
            1_010_000,
            1_015_000,
            5_000,
            Some(1_016_666)
        ));
        assert!(!progress_matches(1_016_240, 1_016_666, 5_000, None));
    }
    fn checker() -> Checker {
        Checker::new(Ok(Content {
            map: MovementMap::default(),
            defines: Defines::default(),
            terrain_movement: Vec::new(),
            regions: BTreeMap::new(),
            sub_units: Err("unused".into()),
        }))
    }
    fn moving(branch: u64, hour: i32) -> DecodedFrame {
        use replay_data::rows::{DivisionMovementRow, TableId, TableRows};
        use replay_frame::frame::{Metrics, TableView};
        let mut frame = DecodedFrame {
            branch,
            hour,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                id: 7,
                province: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: 7,
                path: vec![2],
                path_total: 1,
                numerator: 10,
                ..Default::default()
            }]));
        frame
    }
    #[test]
    fn a_step_pairs_only_consecutive_hours_of_one_branch() {
        let mut checker = checker();
        checker.observe(&moving(1, 10));
        checker.observe(&moving(2, 11));
        assert_eq!(checker.refused.values().sum::<u64>(), 0);
        checker.observe(&moving(2, 12));
        assert_eq!(checker.refused.values().sum::<u64>(), 1);
        checker.observe(&moving(2, i32::MAX));
        checker.observe(&moving(2, i32::MIN));
        assert_eq!(checker.refused.values().sum::<u64>(), 1);
    }
    #[test]
    fn a_naval_transfer_edge_is_derived_without_the_base_speed() {
        use replay_data::rows::{DivisionMovementRow, OwnerTransportSpeedRow, TableId, TableRows};
        use replay_frame::frame::TableView;
        let mut frame = moving(1, 10);
        // No panel row: the transfer edge needs no base speed.
        {
            *frame.tables[TableId::DivisionMovement.index()] =
                TableView::Complete(TableRows::DivisionMovement(vec![DivisionMovementRow {
                    division: 7,
                    path: vec![2],
                    path_total: 1,
                    naval_transfer: 1,
                    ..Default::default()
                }]));
            *frame.tables[TableId::DivisionTransportSpeeds.index()] = TableView::Complete(
                TableRows::DivisionTransportSpeeds(vec![OwnerTransportSpeedRow {
                    owner: 0,
                    region: 70,
                    speed: Some(800_000),
                }]),
            );
            let content = Content {
                map: MovementMap::default(),
                defines: Defines {
                    transport_scale: 50_000,
                    ..Defines::default()
                },
                terrain_movement: Vec::new(),
                regions: BTreeMap::from([(1, 70)]),
                sub_units: Err("unused".into()),
            };
            assert_eq!(edges(&frame, &content)[&7].speed, Ok(400_000));
        }
    }
    #[test]
    fn cross_hour_progress_reports_without_failing_or_requiring_observations() {
        assert_eq!(Checker::default().failures(), [0, 0]);
        let mut checker = checker();
        assert_eq!(checker.failures(), [0, 0]);
        checker.matched = 1;
        assert_eq!(checker.failures(), [0, 0]);
        checker.matched_later = 2;
        checker.differed = 3;
        assert_eq!(checker.failures(), [0, 0]);
        checker.refused.insert("missing state terms".into(), 4);
        checker.stalled = 5;
        assert_eq!(checker.failures(), [0, 0]);
        checker.content = Some(Err("movement content unavailable".into()));
        assert_eq!(checker.failures(), [0, 1]);
    }
}
