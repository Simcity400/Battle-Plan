//! Naval mission admission topology, 140E0DD70 / 140D51530 and the quantised
//! sea-distance matrix, 140A4E4B0 / 140E67CE0. Content is the shared native
//! province graph; port links are ProvinceRow.port_sea, never nearest-sea guesses.
use crate::{province_geometry::ProvinceGeometry, province_graph::ProvinceGraph};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Default)]
pub struct RegionEdge {
    pub to: u32,
    pub unrestricted: bool,
    /// None is a native sea-crossing record with a null rule, which admits.
    pub rules: Vec<Option<String>>,
}

/// 140E0DD70 builds region neighbours in province/neighbour order. Only
/// sea-to-sea edges set the unrestricted byte or append rule records.
pub fn region_edges(
    graph: &ProvinceGraph,
    geometry: &ProvinceGeometry,
    members: &BTreeMap<u32, Vec<u32>>,
    region_of: &BTreeMap<u32, u32>,
) -> BTreeMap<u32, Vec<RegionEdge>> {
    members
        .iter()
        .map(|(&region, provinces)| {
            let mut edges: Vec<RegionEdge> = Vec::new();
            for province in provinces {
                for &(to, _) in graph.edges.get(province).into_iter().flatten() {
                    let Some(&next) = region_of.get(&to).filter(|&&next| next != region) else {
                        continue;
                    };
                    let at = edges
                        .iter()
                        .position(|edge| edge.to == next)
                        .unwrap_or_else(|| {
                            edges.push(RegionEdge {
                                to: next,
                                ..Default::default()
                            });
                            edges.len() - 1
                        });
                    let edge = &mut edges[at];
                    if geometry.kind_flags(*province) == Some(0)
                        && geometry.kind_flags(to) == Some(0)
                    {
                        let crossing = graph.crossing(*province, to);
                        if crossing.is_some_and(|crossing| {
                            crossing.kind == crate::adjacencies::SEA_CROSSING
                                || crossing.rule.is_some()
                        }) {
                            if !edge.unrestricted {
                                edge.rules
                                    .push(crossing.and_then(|crossing| crossing.rule.clone()));
                            }
                        } else {
                            edge.unrestricted = true;
                            edge.rules.clear();
                        }
                    }
                }
            }
            (region, edges)
        })
        .collect()
}

/// 140D51530 marks a neighbour visited before testing its rule, even when
/// it fails. An undecided admission cannot safely be retried from another edge.
pub fn region_reaches(
    edges: &BTreeMap<u32, Vec<RegionEdge>>,
    sea: &BTreeSet<u32>,
    from: u32,
    to: u32,
    mut admits: impl FnMut(&str) -> Result<bool, String>,
) -> Result<bool, String> {
    if from == to {
        return Ok(true);
    }
    let mut seen = BTreeSet::from([from]);
    let mut queue = VecDeque::from([from]);
    while let Some(at) = queue.pop_front() {
        for edge in edges.get(&at).into_iter().flatten() {
            if !seen.insert(edge.to) || !sea.contains(&edge.to) {
                continue;
            }
            let mut pass = edge.unrestricted;
            if !pass {
                for rule in &edge.rules {
                    if match rule {
                        None => true,
                        Some(rule) => admits(rule)?,
                    } {
                        pass = true;
                        break;
                    }
                }
            }
            if pass {
                if edge.to == to {
                    return Ok(true);
                }
                queue.push_back(edge.to);
            }
        }
    }
    Ok(false)
}

/// Load-static source rows, as built by 140A4E4B0. One native relaxation per base,
/// shared by every task force using it; no task-force x province path searches.
pub struct Distances {
    ports: BTreeMap<u32, u32>,
    indexed: Vec<u32>,
    rows: BTreeMap<u32, Result<BTreeMap<u32, i64>, String>>,
}
impl Distances {
    pub fn new(geometry: &ProvinceGeometry, ports: &BTreeMap<u32, u32>) -> Self {
        // 140A54280 indexes valid definitions in province-id order when
        // +0xC8 is nonzero or the definition's land/lake flags are both clear.
        let indexed = geometry
            .centroids
            .keys()
            .copied()
            .filter(|&id| {
                id != 0
                    && (geometry.kind_flags(id) == Some(0)
                        || ports.get(&id).is_some_and(|&port| port != 0))
            })
            .collect();
        Self {
            ports: ports.clone(),
            indexed,
            rows: BTreeMap::new(),
        }
    }
    /// 140E9A9D0 substitutes land/lake port seas first; a missing port
    /// takes 140E67820's wrapped centroid distance over the original pair.
    pub fn distance(
        &mut self,
        graph: &ProvinceGraph,
        geometry: &ProvinceGeometry,
        from: u32,
        to: u32,
    ) -> Result<i64, String> {
        let sea = |id| match geometry.kind_flags(id) {
            Some(0) => Some(id),
            Some(_) => self.ports.get(&id).copied().filter(|&id| id != 0),
            None => None,
        };
        let (Some(source), Some(target)) = (sea(from), sea(to)) else {
            return geometry
                .wrapped_distance(from, to)
                .ok_or_else(|| format!("naval distance province {from}/{to} is undefined"));
        };
        if source == target {
            return Ok(0);
        }
        // 140A4E4B0 narrows the map's lookup to signed i16. Negative slots
        // take 140E67CE0's unmapped sentinel; a wrapped positive slot aliases
        // that earlier matrix row. This is a native limit, not a chosen cap.
        let index = |id| {
            self.indexed
                .binary_search(&id)
                .ok()
                .map(|at| at as i16)
                .filter(|&at| at >= 0)
                .map(|at| self.indexed[at as usize])
        };
        let (Some(source), Some(target)) = (index(source), index(target)) else {
            return Ok(6_553_500_000);
        };
        if !self.rows.contains_key(&source) {
            self.rows
                .insert(source, self.relax(graph, geometry, source));
        }
        Ok(*self.rows[&source]
            .as_ref()
            .map_err(Clone::clone)?
            .get(&target)
            .unwrap_or(&6_553_500_000))
    }
    fn relax(
        &self,
        graph: &ProvinceGraph,
        geometry: &ProvinceGeometry,
        source: u32,
    ) -> Result<BTreeMap<u32, i64>, String> {
        let mut distances = BTreeMap::from([(source, 0_i64)]);
        let mut queue = VecDeque::from([source]);
        while let Some(from) = queue.pop_front() {
            let distance = distances[&from];
            let neighbours = graph.edges.get(&from).map(Vec::as_slice).unwrap_or(&[]);
            // 140A4E4B0 narrows the neighbour count to signed i16 before its
            // loop: negative counts visit nothing; wrapped positive counts
            // visit only the corresponding prefix.
            let count = (neighbours.len() as i16).max(0) as usize;
            for &(to, raw) in &neighbours[..count] {
                // Unindexed lakes and land without sea access are not vertices
                // of the native relaxation, even if the province graph links them.
                if self.indexed.binary_search(&to).is_err() {
                    continue;
                }
                let land = |id| geometry.kind_flags(id).is_some_and(|flags| flags & 1 != 0);
                if (land(from) && self.ports.get(&from) != Some(&to))
                    || (land(to) && self.ports.get(&to) != Some(&from))
                {
                    continue;
                }
                if geometry.kind_flags(to).is_none() {
                    return Err(format!("naval distance neighbour {to} is undefined"));
                }
                let raw = raw.ok_or_else(|| {
                    format!(
                        "naval distance edge {from}/{to}: {}",
                        graph.unavailable_reason(from, to)
                    )
                })? as i32;
                // A negative narrowed edge can make the native relaxation unbounded.
                if raw < 0 {
                    return Err(format!(
                        "naval distance edge {from}/{to} has negative narrowed distance {raw}"
                    ));
                }
                let next = distance.wrapping_add(i64::from(raw));
                if distances.get(&to).is_none_or(|&old| old < 0 || next < old) {
                    distances.insert(to, next);
                    queue.push_back(to);
                }
            }
        }
        // The game stores ushort(distance / 100000), not an unquantised path.
        Ok(distances
            .into_iter()
            .map(|(id, value)| (id, i64::from((value / 100_000) as u16) * 100_000))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_distance_quantises_and_port_land_cannot_bridge_two_seas() {
        let graph = ProvinceGraph {
            edges: BTreeMap::from([
                (1, vec![(2, Some(175_000)), (4, Some(100_000))]),
                (2, vec![(1, Some(175_000)), (3, Some(175_000))]),
                (3, vec![(2, Some(175_000)), (4, Some(100_000))]),
                (4, vec![(1, Some(100_000)), (3, Some(100_000))]),
            ]),
            ..Default::default()
        };
        let geometry = ProvinceGeometry {
            centroids: [(1, [0, 0]), (2, [1, 0]), (3, [2, 0]), (4, [0, 1])].into(),
            land: [4].into(),
            width: 10,
            ..Default::default()
        };
        let ports = [(4, 1)].into();
        let mut distances = Distances::new(&geometry, &ports);
        assert_eq!(distances.distance(&graph, &geometry, 1, 3), Ok(300_000));
        assert_eq!(distances.distance(&graph, &geometry, 4, 3), Ok(300_000));
    }
    #[test]
    fn unmapped_lakes_do_not_bridge_seas_and_negative_narrowed_edges_are_named() {
        let mut graph = ProvinceGraph {
            edges: [(1, vec![(2, Some(100_000))]), (2, vec![(3, Some(100_000))])].into(),
            ..Default::default()
        };
        let geometry = ProvinceGeometry {
            centroids: [(1, [0, 0]), (2, [1, 0]), (3, [2, 0])].into(),
            lake: [2].into(),
            width: 10,
            ..Default::default()
        };
        let ports = BTreeMap::new();
        assert_eq!(
            Distances::new(&geometry, &ports).distance(&graph, &geometry, 1, 3),
            Ok(6_553_500_000)
        );
        graph
            .edges
            .insert(1, vec![(3, Some(i64::from(i32::MAX) + 1))]);
        assert!(
            Distances::new(&geometry, &ports)
                .distance(&graph, &geometry, 1, 3)
                .unwrap_err()
                .contains("negative narrowed distance -2147483648")
        );
    }

    #[test]
    fn native_signed_i16_matrix_slots_keep_their_negative_sentinel_and_positive_alias() {
        let graph = ProvinceGraph {
            edges: [(1, vec![(2, Some(200_000))])].into(),
            ..Default::default()
        };
        let geometry = ProvinceGeometry {
            centroids: (1..=65_537).map(|id| (id, [0, 0])).collect(),
            ..Default::default()
        };
        let ports = BTreeMap::new();
        let mut distances = Distances::new(&geometry, &ports);
        assert_eq!(
            distances.distance(&graph, &geometry, 32_769, 2),
            Ok(6_553_500_000)
        );
        assert_eq!(
            distances.distance(&graph, &geometry, 65_537, 2),
            Ok(200_000)
        );
    }

    #[test]
    fn native_signed_i16_neighbour_count_keeps_its_empty_and_prefix_cases() {
        let mut graph = ProvinceGraph::default();
        let geometry = ProvinceGeometry {
            centroids: [(1, [0, 0]), (2, [1, 0])].into(),
            ..Default::default()
        };
        let ports = BTreeMap::new();
        for (count, expected) in [
            (32_767, 100_000),
            (32_768, 6_553_500_000),
            (65_536, 6_553_500_000),
            (65_537, 100_000),
        ] {
            graph.edges.insert(1, vec![(2, Some(100_000)); count]);
            assert_eq!(
                Distances::new(&geometry, &ports).distance(&graph, &geometry, 1, 2),
                Ok(expected)
            );
        }
    }

    #[test]
    fn native_region_visit_precedes_rule_admission() {
        let restricted = |to, rule: &str| RegionEdge {
            to,
            rules: vec![Some(rule.into())],
            ..Default::default()
        };
        let edges = [
            (1, vec![restricted(2, "no"), restricted(3, "yes")]),
            (3, vec![restricted(2, "yes")]),
        ]
        .into();
        assert_eq!(
            region_reaches(&edges, &[1, 2, 3].into(), 1, 2, |rule| Ok(rule == "yes")),
            Ok(false)
        );
    }
}
