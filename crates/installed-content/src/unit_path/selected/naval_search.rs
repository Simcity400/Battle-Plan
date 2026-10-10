//! FA1E90/F922B0/F91960 strategic-region search. The stored neighbour
//! order, centroid, native heap ties and repeated entries are preserved.
use super::super::Fact;
use crate::province_graph::{heap_pop, heap_push};
use replay::rows::{TransportNavalRegion, TransportNavalSearch};

#[derive(Default, Clone, Copy)]
struct Node {
    present: bool,
    f: i64,
    g: i64,
    parent: Option<u32>,
    depth: i32,
}

fn region(graph: &TransportNavalSearch, id: u32) -> Fact<&TransportNavalRegion> {
    graph
        .regions
        .as_ref()
        .ok_or("transport original strategic-region graph refused")?
        .get(id as usize)
        .and_then(Option::as_ref)
        .ok_or_else(|| {
            format!("transport naval search region {id} outside original nonnull domain")
        })
}

fn heuristic(from: [i32; 2], to: [i32; 2], width: i32) -> i64 {
    let mut x = to[0].wrapping_sub(from[0]);
    if x < -(width / 2) {
        x = x.wrapping_add(width);
    }
    if width / 2 < x {
        x = x.wrapping_sub(width);
    }
    let y = to[1].wrapping_sub(from[1]);
    let squared = x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y));
    let mut root = squared;
    let mut next = squared / 2;
    if squared > 1 {
        loop {
            root = next;
            next = (squared / root).wrapping_add(root) / 2;
            if next >= root {
                break;
            }
        }
    }
    i64::from(root).wrapping_mul(100_000)
}

/// F9E3A0 takes the FIRST adjacency whose target ID matches the neighbour,
/// then accepts a native unrestricted byte or the first nullable/rule OR.
fn edge(
    graph: &TransportNavalSearch,
    from: u32,
    to: u32,
    filter: bool,
    block_one: bool,
    mode: &mut impl FnMut(u32) -> Fact<u8>,
    rule: &mut impl FnMut(&[u8]) -> Fact<bool>,
) -> Fact<bool> {
    if filter {
        let value = mode(to)?;
        if value == 2 || (block_one && value == 1) {
            return Ok(false);
        }
    }
    let Some(edge) = region(graph, from)?.edges.iter().find(|edge| edge.to == to) else {
        return Ok(false);
    };
    if edge.flags & 2 == 0 {
        return Ok(false);
    }
    if edge.unrestricted != 0 {
        return Ok(true);
    }
    for &name in &edge.rules {
        let Some(name) = name else {
            return Ok(true);
        };
        let name = graph
            .rules
            .get(name as usize)
            .ok_or("transport naval rule name index outside original dictionary")?;
        if rule(name)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn search(
    graph: &TransportNavalSearch,
    from: u32,
    to: u32,
    filter: bool,
    block_one: bool,
    mode: &mut impl FnMut(u32) -> Fact<u8>,
    rule: &mut impl FnMut(&[u8]) -> Fact<bool>,
) -> Fact<Vec<u32>> {
    let width = graph
        .width
        .filter(|width| *width > 0)
        .ok_or("transport native search map width refused")?;
    let regions = graph
        .regions
        .as_ref()
        .ok_or("transport original strategic-region graph refused")?;
    region(graph, from)?;
    let destination = region(graph, to)?.position;
    let mut nodes = vec![Node::default(); regions.len()];
    let mut visited = vec![false; regions.len()];
    let mut heap = Vec::new();
    nodes[from as usize].present = true;
    visited[from as usize] = true;
    heap_push(&mut heap, (0, from));
    while !heap.is_empty() {
        let at = heap_pop(&mut heap);
        if at == to {
            let mut path = vec![at];
            while let Some(parent) = nodes[*path.last().unwrap() as usize].parent {
                path.push(parent);
            }
            path.reverse(); // C97F00 emits source through destination, inclusive.
            return Ok(path);
        }
        let current = nodes[at as usize];
        if current.depth == i32::MAX {
            continue;
        }
        visited[at as usize] = true;
        for neighbour in &region(graph, at)?.edges {
            let next = neighbour.to;
            let seen = visited
                .get(next as usize)
                .ok_or("transport naval neighbour outside original domain")?;
            if *seen || !edge(graph, at, next, filter, block_one, mode, rule)? {
                continue;
            }
            let node = &mut nodes[next as usize];
            if !node.present {
                node.g = 0x53e2_d620_fb40;
                node.f = node.g;
            }
            let cost = current.g.wrapping_add(100_000);
            if cost >= node.g {
                continue;
            }
            let estimate = if node.present {
                node.f.wrapping_sub(node.g)
            } else {
                heuristic(region(graph, next)?.position, destination, width)
            };
            *node = Node {
                present: true,
                g: cost,
                f: estimate.wrapping_add(cost),
                parent: Some(at),
                depth: current.depth.wrapping_add(1),
            };
            heap_push(&mut heap, (node.f, next));
        }
    }
    Ok(Vec::new())
}

/// FA1E90 first searches without access filtering. Only a mode1/2 region
/// on that exact path triggers the retry. Mode2 stays blocked; mode1 retries
/// once more with its block removed, only if its first retry was empty.
/// The native context's two region exemptions are null in this caller.
pub fn admit(
    graph: &TransportNavalSearch,
    from: u32,
    to: u32,
    retry_access: bool,
    mut mode: impl FnMut(u32) -> Fact<u8>,
    mut has_mode: impl FnMut(u8) -> Fact<bool>,
    mut rule: impl FnMut(&[u8]) -> Fact<bool>,
) -> Fact<bool> {
    let first = search(graph, from, to, false, false, &mut mode, &mut rule)?;
    if first.is_empty() || !retry_access {
        return Ok(!first.is_empty());
    }
    let mut previous = None;
    for region in first {
        if previous == Some(region) {
            continue;
        }
        previous = Some(region);
        if !matches!(mode(region)?, 1 | 2) {
            continue;
        }
        // Both calls run, including the second when the first is true.
        let one = has_mode(1)?;
        let two = has_mode(2)?;
        if !one && !two {
            break;
        }
        let retry = search(graph, from, to, true, one, &mut mode, &mut rule)?;
        if !retry.is_empty() {
            return Ok(true);
        }
        return if one {
            Ok(!search(graph, from, to, true, false, &mut mode, &mut rule)?.is_empty())
        } else {
            Ok(false)
        };
    }
    Ok(true)
}

#[cfg(test)]
#[path = "naval_search_tests.rs"]
mod tests;
