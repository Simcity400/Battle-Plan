//! `CheckNavalInvasionRoute@1410277D0`'s mode-0 request to `140E174F0`:
//! +29/+2C set, all other flags clear. Keep the native heap, including land
//! port dead ends and duplicate entries: they can change equal-cost ties.

use super::*;
use crate::province_graph::{heap_pop, heap_push};

#[derive(Clone, Copy, Default)]
struct Node {
    seen: bool,
    parent: usize,
    f: i64,
    g: i64,
}

fn heuristic(squared: i32) -> i64 {
    // 140E18145..1816F: CVTTSD2SI, two wrapping IMULs, then signed /100000.
    // A negative square sum produces INT_MIN from sqrt(NaN), not zero.
    let root = if squared < 0 {
        i32::MIN
    } else {
        f64::from(squared).sqrt() as i32
    };
    i64::from(root).wrapping_mul(100_000).wrapping_mul(125_000) / 100_000
}

impl<W: MoveWorld + ?Sized> Mover<'_, W> {
    /// `14101F4B0`: use the stored port sea if it resolves, otherwise the
    /// first sea neighbour in native definition order (even an impassable edge).
    pub fn invasion_sea(&self, province: u32) -> Fact<Option<u32>> {
        let Some(at) = self.graph.dense(province).filter(|_| (province as i32) > 0) else {
            return Ok(None);
        };
        let port = self.world.port_sea(province)?;
        if port > 0 && self.graph.dense(port).is_some() {
            return Ok(Some(port));
        }
        for edge in self
            .graph
            .neighbours(at)
            .ok_or_else(|| format!("province {province} has no neighbours"))?
        {
            let next = self.graph.ids[edge.to as usize];
            if self.is_sea(next)? {
                return Ok(Some(next));
            }
        }
        Ok(None)
    }

    /// Source sea excluded, target sea included (`140E15090`); equal seas
    /// return one entry without a search (`1410277D0`). The post-search navy
    /// permission check changes only the native return flag, not this route.
    /// `stop_at_owner` is the resolved request country only when its raw
    /// reference is signed-positive (`140E17B31`); otherwise it is absent.
    pub fn invasion_route(
        &self,
        army: &Army,
        stop_at_owner: Option<Country>,
        from: u32,
        to: u32,
    ) -> Fact<Vec<u32>> {
        if from == to {
            return Ok(vec![from]);
        }
        // Preserve this caller's unsupported-content refusal before vf70;
        // counter requests own a different native precheck order.
        self.native_endpoints(from, to)?;
        // 140E18350 invokes vf70 with naval=true, no mode-2 node gate.
        if !self.province_access_for_move(army, to, true)? {
            return Ok(Vec::new());
        }
        Ok(self
            .native_route(army, from, to, stop_at_owner, |from, to| {
                self.invasion_edge(army, from, to)
            })?
            .unwrap_or_default())
    }

    /// Shared 140E174F0 native A* loop. None is search failure; Some(empty)
    /// is successful equal endpoints. Request callers own their prechecks.
    pub(super) fn native_route(
        &self,
        army: &Army,
        from: u32,
        to: u32,
        stop_at_owner: Option<Country>,
        mut admits: impl FnMut(u32, u32) -> Fact<bool>,
    ) -> Fact<Option<Vec<u32>>> {
        let graph = self.graph;
        let (source, target) = self.native_endpoints(from, to)?;
        let centroid = |at: usize| {
            graph.centroids[at]
                .ok_or_else(|| format!("unit path province {} has no centroid", graph.ids[at]))
        };
        let [tx, ty] = centroid(target)?;
        let mut nodes = vec![Node::default(); graph.ids.len()];
        let mut closed = vec![false; graph.ids.len()];
        let mut heap = Vec::new();
        nodes[source].seen = true;
        closed[source] = true;
        heap_push(&mut heap, (0, source as u32));
        while !heap.is_empty() {
            let at = heap_pop(&mut heap) as usize;
            if at == target {
                let mut route = Vec::new();
                let mut cursor = at;
                while cursor != source {
                    route.push(graph.ids[cursor]);
                    cursor = nodes[cursor].parent;
                    if route.len() > nodes.len() {
                        return Err("unit path predecessor cycle".into());
                    }
                }
                route.reverse();
                return Ok(Some(route));
            }
            closed[at] = true;
            let id = graph.ids[at];
            // 140E17B37 stops expansion at the request country's state owner.
            if let Some(country) = stop_at_owner
                && self.world.state_owner(id)? == country
            {
                continue;
            }
            for edge in graph
                .neighbours(at)
                .ok_or_else(|| format!("province {id} has no neighbours"))?
            {
                let next = edge.to as usize;
                if closed[next] || edge.cost == Cost::Zero {
                    continue;
                }
                let other = graph.ids[next];
                if !admits(id, other)? {
                    continue;
                }
                let mut cost = edge.distance.ok_or_else(|| {
                    format!(
                        "unit path edge {id}->{other} distance is unavailable: {}",
                        graph
                            .unavailable
                            .get(&(id.min(other), id.max(other)))
                            .map_or("its terrain cost is unavailable", String::as_str)
                    )
                })?;
                let owner = self.world.state_owner(other)?;
                if owner != army.owner && !self.world.path_friendly(army.owner, owner)? {
                    cost = cost.wrapping_mul(2);
                }
                let g = nodes[at].g.wrapping_add(cost);
                let node = &mut nodes[next];
                if node.seen && node.g <= g {
                    continue;
                }
                let h = if node.seen {
                    node.f.wrapping_sub(node.g)
                } else {
                    let [x, y] = centroid(next)?;
                    let mut dx = tx.wrapping_sub(x);
                    if dx < -(graph.width / 2) {
                        dx = dx.wrapping_add(graph.width);
                    } else if dx > graph.width / 2 {
                        dx = dx.wrapping_sub(graph.width);
                    }
                    let dy = ty.wrapping_sub(y);
                    let squared = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
                    heuristic(squared)
                };
                *node = Node {
                    seen: true,
                    parent: at,
                    f: g.wrapping_add(h),
                    g,
                };
                heap_push(&mut heap, (node.f, next as u32));
            }
        }
        Ok(None)
    }

    fn native_endpoints(&self, from: u32, to: u32) -> Fact<(usize, usize)> {
        // MoveGraph::new sorts definition IDs, so its last ID proves the bound.
        if self
            .graph
            .ids
            .last()
            .is_some_and(|&id| id > u16::MAX as u32)
        {
            return Err("unit path province id exceeds the native u16 node representation".into());
        }
        Ok((
            self.graph
                .dense(from)
                .ok_or_else(|| format!("unit path source {from} is undefined"))?,
            self.graph
                .dense(to)
                .ok_or_else(|| format!("unit path target {to} is undefined"))?,
        ))
    }

    fn invasion_edge(&self, army: &Army, from: u32, to: u32) -> Fact<bool> {
        let crossing = self.graph.crossing(from, to);
        if from == to || crossing.is_some_and(|c| c.impassable) {
            return Ok(false);
        }
        if let Some(rule) = crossing.and_then(|c| c.rule.as_ref())
            && !self.rule_admits_army(self.rule(rule)?, army.owner)?
        {
            return Ok(false);
        }
        // 140E17080's forced-transfer branch reads class byte 6, not the
        // mode-2 embark/base-level rules. No land-to-land hop is admitted.
        match [self.is_land(from)?, self.is_land(to)?] {
            [true, true] => return Ok(false),
            [true, false] if self.world.port_sea(from)? != to => return Ok(false),
            [false, true] if self.world.port_sea(to)? != from => return Ok(false),
            _ => {}
        }
        if let Some(crossing) = crossing {
            match &crossing.strait {
                Strait::Through(through)
                    if self.world.strait_fleet(*through, army.owner)? == StraitFleet::Visible =>
                {
                    return Ok(false);
                }
                Strait::Invalid(reason) => return Err(reason.clone()),
                _ => {}
            }
        }
        and(
            or(Ok(army.exiled), || self.move_access(army.owner, to)),
            || self.province_access_for_move(army, to, true),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::heuristic;

    #[test]
    fn native_heuristic_preserves_overflow_before_fixed_point_division() {
        assert_eq!(heuristic(24), 500_000);
        assert_eq!(heuristic(i32::MAX), 5_792_500_000);
        assert_eq!(heuristic(-1), -83_968_015_262_904);
    }
}
