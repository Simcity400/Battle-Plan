

use content_reader_map::province_graph::ProvinceGraph;
use replay_data::{
    rows::DivisionMovementRow, rows::DivisionRow, rows::DivisionStatusRow, rows::FactionMemberRow,
    rows::NavalBaseRow, rows::RelationLinkRow, rows::WarRelationRow,
};

use super::*;

pub(super) const WIDTH: u32 = 120;
pub(super) const HEIGHT: u32 = 110;
const BASES: usize = 300;
pub(super) const MARINES: usize = 20;
const MEMBERS: u32 = 300;

pub(super) fn id(x: u32, y: u32) -> u32 {
    1 + y * WIDTH + x
}

pub(super) fn land(x: u32) -> bool {
    x % 30 < 26
}

pub(super) fn grid() -> (ProvinceGraph, ProvinceGeometry) {
    let mut graph = ProvinceGraph::default();
    let mut geometry = ProvinceGeometry {
        width: WIDTH as i32,
        height: HEIGHT as i32,
        ..Default::default()
    };
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let mut next = Vec::new();
            for (dx, dy) in [(1_i32, 0_i32), (0, 1), (-1, 0), (0, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if (0..WIDTH as i32).contains(&nx) && (0..HEIGHT as i32).contains(&ny) {
                    next.push((id(nx as u32, ny as u32), Some(50_000)));
                }
            }
            graph.edges.insert(id(x, y), next);
            geometry.centroids.insert(id(x, y), [x as i32, y as i32]);
            if land(x) {
                geometry.land.insert(id(x, y));
            }
        }
    }
    (graph, geometry)
}

fn owner(x: u32, y: u32, countries: u32) -> u32 {
    let continent = x / 30;
    let (first, count) = if continent == 0 {
        (0, MEMBERS)
    } else {
        (MEMBERS, countries - MEMBERS)
    };
    let cell = (continent.saturating_sub(1) * 26 + x % 30) * HEIGHT + y;
    let cells = if continent == 0 { 26 } else { 3 * 26 } * HEIGHT;
    first + cell * count / cells
}

pub(super) fn hour(countries: u32) -> DecodedFrame {
    let mut frame = frame();
    let raw = |slot: u32| 1_000 + slot;
    put::<tables::Countries>(
        &mut frame,
        (0..countries)
            .map(|slot| CountryRow {
                id: raw(slot),
                canonical_slot: slot,
                tag: format!("C{slot:04}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::CountryLinks>(
        &mut frame,
        (0..countries)
            .map(|slot| CountryLinkRow {
                id: raw(slot),
                canonical_slot: slot,
            })
            .collect(),
    );
    let mut provinces = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let slot = land(x).then(|| owner(x, y, countries));
            provinces.push(ProvinceRow {
                id: id(x, y),
                state: slot.map_or(0, |slot| 10_000 + slot),
                owner: slot.map_or(0, raw),
                controller: slot.map_or(0, raw),
                ..Default::default()
            });
        }
    }
    put::<tables::Provinces>(&mut frame, provinces);
    put::<tables::FactionMembers>(
        &mut frame,
        (0..MEMBERS)
            .map(|slot| FactionMemberRow {
                country: raw(slot),
                faction: 1,
                ordinal: slot,
            })
            .collect(),
    );
    super::reserved_faction(&mut frame, 1);
    put::<tables::WarRelations>(
        &mut frame,
        (MEMBERS..countries - 1)
            .step_by(2)
            .map(|slot| WarRelationRow {
                first_country: raw(slot),
                second_country: raw(slot + 1),
                start_hour: 1,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::RelationLinks>(
        &mut frame,
        (0..countries)
            .flat_map(|slot| {
                (1..=4).map(move |step| RelationLinkRow {
                    country: raw(slot),
                    target_index: (slot + step * 7) % countries,
                    links: RelationLinkRow::MILITARY_ACCESS,
                    transport_subject: Default::default(),
                })
            })
            .collect(),
    );
    let mut divisions = Vec::new();
    for index in 0..MARINES as u32 {
        let (x, y) = (index % 26, 5 + index * 5);
        divisions.push(DivisionRow {
            id: 1 + u64::from(index),
            roster_owner: raw(owner(x, y, countries)),
            logical_owner: raw(owner(x, y, countries)),
            province: id(x, y),
            hit_points: 100_000,
            organisation: 500_000,
            ..Default::default()
        });
    }
    for index in 0..countries / 2 {
        let cell = index * 7919 % (WIDTH * HEIGHT);
        let (x, y) = (cell % WIDTH, cell / WIDTH);
        if !land(x) {
            continue;
        }
        let owner = raw(owner(x, y, countries));
        divisions.push(DivisionRow {
            id: 10_000 + u64::from(index),
            roster_owner: owner,
            logical_owner: owner,
            province: id(x, y),
            hit_points: 100_000,
            organisation: 500_000,
            ..Default::default()
        });
    }
    put::<tables::DivisionMovement>(
        &mut frame,
        divisions
            .iter()
            .map(|division| DivisionMovementRow {
                division: division.id,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::DivisionStatus>(
        &mut frame,
        divisions
            .iter()
            .map(|division| DivisionStatusRow {
                division: division.id,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::Divisions>(&mut frame, divisions);
    put::<tables::NavalBases>(&mut frame, bases(countries));
    frame
}

pub(super) fn base_provinces() -> Vec<u32> {
    (0..BASES as u32)
        .map(|index| {
            let y = index % HEIGHT;
            match index % 3 {
                0 | 1 => id(25, y),
                _ => id(30 + index % 26, y),
            }
        })
        .collect()
}

fn bases(countries: u32) -> Vec<NavalBaseRow> {
    base_provinces()
        .into_iter()
        .map(|province| {
            let (x, y) = ((province - 1) % WIDTH, (province - 1) / WIDTH);
            NavalBaseRow {
                province,
                country: 1_000 + owner(x, y, countries),
                level: 1,
                healthy_level: 1,
                ..Default::default()
            }
        })
        .collect()
}

