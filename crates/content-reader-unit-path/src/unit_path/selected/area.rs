use super::super::Fact;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AreaNode {
    pub label: u32,
    pub first_land_province: Fact<u32>,
    pub nonsea_index: Fact<i32>,
    pub land_count: Fact<i32>,
    pub neighbors: Fact<Vec<u32>>,
}

pub fn area_cost(
    land_count: Fact<i32>,
    owner_raw: i32,
    mut first_land_state_owner: impl FnMut() -> Fact<i32>,
    mut access: impl FnMut() -> Fact<bool>,
    mut same_country: impl FnMut(i32, i32) -> Fact<bool>,
) -> Fact<i64> {
    if land_count? == 0 {
        return Ok(-100_000);
    }
    let state_owner = first_land_state_owner()?;
    if state_owner <= 0 {
        return Ok(-100_000);
    }
    if access()? {
        return Ok(100_000);
    }
    if owner_raw == state_owner {
        return Ok(0);
    }
    if owner_raw != 0 && same_country(owner_raw, state_owner)? {
        return Ok(0);
    }
    Ok(200_000)
}

fn valid_index(index: i32, count: u32) -> Fact<i32> {
    if index < 0 || index as u32 >= count {
        Err("area nonsea index outside snapshot domain".into())
    } else {
        Ok(index)
    }
}

#[derive(Clone, Copy)]
struct QueueEntry {
    cost: i64,
    province: u32,
    label: u32,
}
fn before_or_equal(a: QueueEntry, b: QueueEntry) -> bool {
    (a.cost, a.province) <= (b.cost, b.province)
}

#[derive(Default)]
struct AreaQueue(Vec<QueueEntry>);
impl AreaQueue {
    fn push(&mut self, entry: QueueEntry) {
        let mut hole = self.0.len();
        self.0.push(entry);
        while hole > 0 {
            let parent = (hole - 1) / 2;
            if before_or_equal(self.0[parent], entry) {
                break;
            }
            self.0[hole] = self.0[parent];
            hole = parent;
        }
        self.0[hole] = entry;
    }
    fn pop(&mut self) -> Option<QueueEntry> {
        let result = *self.0.first()?;
        let last = self.0.pop().unwrap();
        if self.0.is_empty() {
            return Some(result);
        }
        let mut hole = 0;
        while hole * 2 + 2 < self.0.len() {
            let left = hole * 2 + 1;
            let right = left + 1;
            let child = if (self.0[left].cost, self.0[left].province)
                < (self.0[right].cost, self.0[right].province)
            {
                left
            } else {
                right
            };
            self.0[hole] = self.0[child];
            hole = child;
        }
        let left = hole * 2 + 1;
        if left < self.0.len() {
            self.0[hole] = self.0[left];
            hole = left;
        }
        while hole > 0 {
            let parent = (hole - 1) / 2;
            if before_or_equal(self.0[parent], last) {
                break;
            }
            self.0[hole] = self.0[parent];
            hole = parent;
        }
        self.0[hole] = last;
        Some(result)
    }
}
pub fn areas_connect(
    start: u32,
    target: u32,
    nonsea_count: u32,
    mut node: impl FnMut(u32) -> Fact<AreaNode>,
    mut cost: impl FnMut(&AreaNode) -> Fact<i64>,
) -> Fact<bool> {
    if start == target {
        return Ok(true);
    }
    let start_node = node(start)?;
    let start_index = start_node.nonsea_index.clone()?;
    if start_index < 0 {
        return Ok(false);
    }
    let mut visited = BTreeSet::new();
    visited.insert(valid_index(start_index, nonsea_count)?);
    let mut queue = AreaQueue::default();
    for label in start_node.neighbors.clone()? {
        let neighbor = node(label)?;
        let value = cost(&neighbor)?;
        if value < 0 {
            continue;
        }
        let index = valid_index(neighbor.nonsea_index.clone()?, nonsea_count)?;
        let province = neighbor.first_land_province.clone()?;
        queue.push(QueueEntry {
            cost: value,
            province,
            label,
        });
        visited.insert(index);
    }
    while let Some(QueueEntry {
        cost: accumulated,
        label,
        ..
    }) = queue.pop()
    {
        if label == target {
            return Ok(true);
        }
        let current = node(label)?;
        for next in current.neighbors.clone()? {
            let neighbor = node(next)?;
            let index = valid_index(neighbor.nonsea_index.clone()?, nonsea_count)?;
            if visited.contains(&index) {
                continue;
            }
            let value = cost(&neighbor);
            let value = if next == target { 0 } else { value? };
            if value < 0 {
                continue;
            }
            let province = neighbor.first_land_province.clone()?;
            queue.push(QueueEntry {
                cost: accumulated.wrapping_add(value),
                province,
                label: next,
            });
            visited.insert(index);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(label: u32, neighbors: &[u32]) -> AreaNode {
        AreaNode {
            label,
            first_land_province: Ok(label + 10),
            nonsea_index: Ok(label as i32),
            land_count: Ok(1),
            neighbors: Ok(neighbors.to_vec()),
        }
    }
    #[test]
    fn initial_target_negative_is_blocked_but_later_target_cost_is_overridden() {
        let get = |label| {
            Ok(match label {
                1 => node(1, &[3]),
                _ => node(label, &[]),
            })
        };
        assert_eq!(areas_connect(1, 3, 8, get, |_| Ok(-100_000)), Ok(false));
        let get = |label| {
            Ok(match label {
                1 => node(1, &[2]),
                2 => node(2, &[3]),
                _ => node(label, &[]),
            })
        };
        let mut calls = Vec::new();
        assert_eq!(
            areas_connect(1, 3, 8, get, |n| {
                calls.push(n.label);
                if n.label == 3 {
                    Err("overwritten callback operand".into())
                } else {
                    Ok(100_000)
                }
            }),
            Ok(true)
        );
        assert_eq!(calls, [2, 3]);
    }
    #[test]
    fn same_area_succeeds_without_graph_and_initial_repeats_are_not_deduplicated() {
        assert_eq!(areas_connect(7, 7, 0, |_| panic!(), |_| panic!()), Ok(true));
        let get = |label| {
            Ok(match label {
                1 => node(1, &[2, 2]),
                _ => node(label, &[]),
            })
        };
        let mut calls = 0;
        assert_eq!(
            areas_connect(1, 3, 8, get, |_| {
                calls += 1;
                Ok(0)
            }),
            Ok(false)
        );
        assert_eq!(calls, 2);
    }
    #[test]
    fn callback_sign_raw_owner_and_alias_are_preserved() {
        assert_eq!(
            area_cost(Ok(0), 4, || panic!(), || panic!(), |_, _| panic!()),
            Ok(-100_000)
        );
        assert_eq!(
            area_cost(Ok(-1), 0, || Ok(0), || panic!(), |_, _| panic!()),
            Ok(-100_000)
        );
        assert_eq!(
            area_cost(Ok(1), 0, || Ok(5), || Ok(false), |_, _| panic!()),
            Ok(200_000)
        );
        assert_eq!(
            area_cost(Ok(1), 5, || Ok(5), || Ok(false), |_, _| panic!()),
            Ok(0)
        );
        assert_eq!(
            area_cost(Ok(1), 7, || Ok(5), || Ok(false), |_, _| Ok(true)),
            Ok(0)
        );
        assert_eq!(
            area_cost(Ok(1), 5, || Ok(5), || Ok(true), |_, _| panic!()),
            Ok(100_000)
        );
    }
    #[test]
    fn unreadable_graph_is_unknown_negative_start_is_false_and_reached_neighbor_is_refused() {
        assert!(
            areas_connect(
                1,
                2,
                8,
                |_| Err("area payload capture refused".into()),
                |_| panic!()
            )
            .is_err()
        );
        let mut negative_start = node(1, &[]);
        negative_start.nonsea_index = Ok(-1);
        negative_start.neighbors = Err("unreached neighbor list".into());
        assert_eq!(
            areas_connect(1, 2, 8, |_| Ok(negative_start.clone()), |_| panic!()),
            Ok(false)
        );
        let mut negative_neighbor = node(2, &[]);
        negative_neighbor.nonsea_index = Ok(-1);
        let get = |label| {
            Ok(if label == 1 {
                node(1, &[2])
            } else {
                negative_neighbor.clone()
            })
        };
        assert!(areas_connect(1, 2, 8, get, |_| Ok(0)).is_err());
        let mut outside_start = node(1, &[]);
        outside_start.nonsea_index = Ok(8);
        assert!(areas_connect(1, 2, 8, |_| Ok(outside_start.clone()), |_| panic!()).is_err());
    }
    #[test]
    fn visited_identity_is_nonsea_index_and_is_marked_before_heap_pop() {
        let get = |label| {
            let mut result = match label {
                1 => node(1, &[2, 3]),
                3 => node(3, &[4]),
                4 => node(4, &[5]),
                _ => node(label, &[]),
            };
            if label == 4 {
                result.nonsea_index = Ok(2);
            }
            Ok(result)
        };
        let mut calls = Vec::new();
        assert_eq!(
            areas_connect(1, 5, 8, get, |n| {
                calls.push(n.label);
                Ok(if n.label == 2 { 200_000 } else { 0 })
            }),
            Ok(false)
        );
        assert_eq!(calls, [2, 3]);
    }
    #[test]
    fn area_heap_cost_ties_compare_first_land_province_before_label() {
        let mut queue = AreaQueue::default();
        for (label, province, cost) in [(1, 40, 0), (9, 20, 0), (2, 10, 1)] {
            queue.push(QueueEntry {
                label,
                province,
                cost,
            });
        }
        let mut labels = Vec::new();
        while let Some(entry) = queue.pop() {
            labels.push(entry.label);
        }
        assert_eq!(labels, [9, 1, 2]);
    }
}
