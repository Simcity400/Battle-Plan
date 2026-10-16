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

use installed_content::movement::MovementMap;
use replay::{
    eta_derive::{
        self, Defines, OrganisationDefines,
        frame_terms::{FrameTerms, Geography},
    },
    frame::DecodedFrame,
    rows::{DivisionRow, Recorded, tables},
};

/// The content the movement speed reads.
pub struct Content {
    pub map: MovementMap,
    pub defines: Defines,
    /// Each terrain category's own adjuster movement, in database order.
    pub terrain_movement: Vec<i64>,
    /// Each province's strategic region id.
    pub regions: BTreeMap<u32, u32>,
}

impl Content {
    pub fn read(layers: &installed_content::ResolvedLayers) -> std::io::Result<Self> {
        let defines = installed_content::defines::read(layers)?;
        let movement = installed_content::movement::MovementDefines::read(&defines)?;
        let terrain = installed_content::terrain::read_definitions(layers)?;
        let regions = installed_content::regions::read(layers)?;
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
        })
    }
}

impl Geography for Content {
    fn terrain_index(&self, province: u32) -> Option<i32> {
        self.map.terrain(province).map(|terrain| terrain.index)
    }

    fn land(&self, province: u32) -> Option<bool> {
        self.map
            .terrain(province)
            .map(|terrain| terrain.flags & 1 != 0)
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

/// The tables the speed terms read.
pub const TABLES: [replay::rows::TableId; 17] = {
    use replay::rows::TableId::*;
    [
        Divisions,
        DivisionMovement,
        DivisionStatus,
        Provinces,
        States,
        CountryLinks,
        Countries,
        CountryCounterInputs,
        OrdersGroupCounterInputs,
        WeatherProvinces,
        WeatherRegions,
        ModifierDefinitions,
        LexicalTokens,
        SimulationFeatures,
        WarRelations,
        Clock,
        RaidAirScores,
    ]
};

/// One division's edge at an hour: its location, first route province, route,
/// numerator and the speed playback derives for the edge.
struct Edge {
    location: u32,
    first: u32,
    path: Vec<u32>,
    numerator: i64,
    speed: Result<i64, String>,
    key: MovementRowKey,
}

#[derive(Default)]
pub struct Checker {
    content: Option<Result<Content, String>>,
    previous: Option<(i32, BTreeMap<u64, Edge>)>,
    /// Pairs whose step equals the derived speed, and those that differ.
    pub matched: u64,
    /// Pairs whose step differs from the earlier hour's derived speed but equals the
    /// later hour's, over the same edge.
    pub matched_later: u64,
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
        if let Some((hour, previous)) = self.previous.take()
            && hour + 1 == frame.hour
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
                    Ok(speed) if *speed == step => self.matched += 1,
                    // The step runs after the next update's own writers of some operands:
                    // the later hour's terms, over the same edge, can be the ones it read.
                    Ok(_) if after.speed.as_ref().is_ok_and(|speed| *speed == step) => {
                        self.matched_later += 1;
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
        self.previous = Some((frame.hour, current));
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
                for (reason, count) in &self.refused {
                    println!("  refused ({reason}): {count}");
                }
                for line in &self.listed {
                    println!("  difference: {line}");
                }
            }
        }
    }

    /// The witness reports differences without failing: the next hour's update can
    /// rewrite an operand before its movement step.
    pub fn failures(&self) -> [u64; 2] {
        match &self.content {
            Some(Err(_)) => [0, 1],
            _ => [0, 0],
        }
    }
}

/// The terms a differing division's edge took that the recording omits or that the
/// next hour's update can rewrite first: an unbypassed noncore-supply term, days out of
/// supply, partial supply, fuel, a retreat and a commander block with entries, its
/// armour speed entry apart.
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
    if status.move_flags & replay::rows::DivisionStatusRow::MOVE_NONCORE_SUPPLY_BYPASS == 0
        && status.move_flags & replay::rows::DivisionStatusRow::MOVE_EXILED == 0
    {
        terms.push("noncore unbypassed");
    }
    if matches!(status.out_of_supply_days, Recorded::Value(Some(days)) if days != 0) {
        terms.push("days out of supply");
    }
    if status.supply_ratio < 100_000 {
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
        let speed = match row.base_speed {
            Recorded::Value(Some(base)) => {
                terms.with_terms(division, row, content, &content.defines, |terms| {
                    eta_derive::edge_speed(base, division.province, first, terms, &content.defines)
                })
            }
            _ => Err("the division's base speed was refused".into()),
        };
        edges.insert(
            row.division,
            Edge {
                location: division.province,
                first,
                path: row.path.clone(),
                numerator: row.numerator,
                speed,
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
