
use std::collections::HashMap;

use super::sprites::{Batch, WindowKey, WindowPool, WindowStamp};

#[derive(Clone, Copy)]
struct Seen {
    stamp: WindowStamp,
    world: Option<[f32; 3]>,
    tracked: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub windows: u64,
    pub appeared: u64,
    pub left: u64,
    pub content: u64,
    pub order: u64,
    pub moved_shown: u64,
    pub moved_untracked: u64,
    pub moved_changed: u64,
    pub tracked: u64,
}

impl Counts {
    pub fn named(&self) -> [(&'static str, u64); 9] {
        [
            ("windows", self.windows),
            ("appeared", self.appeared),
            ("left", self.left),
            ("content", self.content),
            ("order", self.order),
            ("moved_shown", self.moved_shown),
            ("moved_untracked", self.moved_untracked),
            ("moved_changed", self.moved_changed),
            ("tracked", self.tracked),
        ]
    }

    pub fn changed(&self) -> bool {
        self.appeared + self.left + self.content + self.moved_untracked > 0
    }

    fn add(&mut self, other: &Self) {
        for (axis, value) in [
            (&mut self.windows, other.windows),
            (&mut self.appeared, other.appeared),
            (&mut self.left, other.left),
            (&mut self.content, other.content),
            (&mut self.order, other.order),
            (&mut self.moved_shown, other.moved_shown),
            (&mut self.moved_untracked, other.moved_untracked),
            (&mut self.moved_changed, other.moved_changed),
            (&mut self.tracked, other.tracked),
        ] {
            *axis += value;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Changed {
    pub total: Counts,
    pub by_pool: Vec<(&'static str, Counts)>,
    pub loose_anchored: u64,
}

impl Changed {
    pub fn record(&self) {
        for (name, value) in self.total.named() {
            viewer_runtime::view_record::add("overlay count", name, value);
        }
        for (pool, counts) in &self.by_pool {
            for (name, value) in counts.named() {
                if value > 0 {
                    viewer_runtime::view_record::add_named("overlay count", &format!("{pool}.{name}"), value);
                }
            }
        }
        viewer_runtime::view_record::add("overlay count", "loose_anchored", self.loose_anchored);
        viewer_runtime::view_record::add("overlay count", "paintings", 1);
    }
}

#[derive(Default)]
pub struct Watch {
    held: HashMap<WindowKey, Seen>,
    seen: HashMap<WindowKey, Seen>,
}

impl Watch {
    pub fn painted(&mut self, batch: &Batch) -> Changed {
        self.seen.clear();
        let mut pools: Vec<(&'static str, Counts)> = Vec::new();
        let mut anchored_in_windows = 0;
        for window in batch.windows() {
            anchored_in_windows += batch.quads[window.quads()].iter().filter(|quad| quad.anchor.is_some()).count();
            let anchor = batch.quads[window.quads()]
                .iter()
                .find_map(|quad| quad.anchor);
            let now = Seen {
                stamp: window.stamp,
                world: anchor.map(|anchor| anchor.world),
                tracked: anchor.is_some_and(|anchor| anchor.moves.is_some()),
            };
            let mut counts = Counts { windows: 1, tracked: u64::from(now.tracked), ..Counts::default() };
            match self.held.remove(&window.key) {
                None => counts.appeared = 1,
                Some(before) => {
                    if before.stamp.painting != now.stamp.painting {
                        counts.content = 1;
                    } else if before.stamp.taken != now.stamp.taken {
                        counts.order = 1;
                    }
                    if before.world != now.world {
                        if before.tracked {
                            counts.moved_shown = 1;
                        } else if counts.content == 1 {
                            counts.moved_changed = 1;
                        } else {
                            counts.moved_untracked = 1;
                        }
                    }
                }
            }
            add_to(&mut pools, pool_name(window.key.pool), &counts);
            self.seen.insert(window.key, now);
        }
        for (key, _) in self.held.drain() {
            add_to(&mut pools, pool_name(key.pool), &Counts { left: 1, ..Counts::default() });
        }
        std::mem::swap(&mut self.held, &mut self.seen);
        let anchored = batch.quads.iter().filter(|quad| quad.anchor.is_some()).count();
        let loose = (anchored - anchored_in_windows.min(anchored)) as u64;
        let mut total = Counts::default();
        for (_, counts) in &pools {
            total.add(counts);
        }
        pools.sort_unstable_by_key(|(name, _)| *name);
        Changed { total, by_pool: pools, loose_anchored: loose }
    }
}

fn add_to(pools: &mut Vec<(&'static str, Counts)>, name: &'static str, counts: &Counts) {
    match pools.iter_mut().find(|(held, _)| *held == name) {
        Some((_, held)) => held.add(counts),
        None => pools.push((name, *counts)),
    }
}

fn pool_name(pool: WindowPool) -> &'static str {
    match pool {
        WindowPool::Base => "bases",
        WindowPool::FactionPing => "pings",
        WindowPool::LandCombat => "combats",
        WindowPool::LandCounter => "counters",
        WindowPool::Label => "labels",
        WindowPool::Raid => "raids",
        WindowPool::AdjacencyIcon => "adjacency",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_gpu::icon_tracks::{IconAnchor, IconFamily};
    use crate::sprites::{Quad, WindowPool};

    fn quad(world: [f32; 3], moves: Option<u32>) -> Quad {
        Quad {
            texture: 0,
            rect: [0.0; 4],
            uv: [0.0; 4],
            color: [1.0; 4],
            rotation: None,
            layer: 0,
            shade: 0.0,
            exact: None,
            anchor: Some(IconAnchor { world, cutoff: f32::INFINITY, family: IconFamily::Columns, moves }),
            present: None,
        }
    }

    fn paint(batch: &mut Batch, owner: u64, painting: u64, world: [f32; 3], moves: Option<u32>) {
        batch.at_window(
            WindowKey::new(WindowPool::LandCounter, 0, owner),
            WindowStamp { painting, taken: 1 },
        );
        batch.quads.push(quad(world, moves));
        batch.no_window();
    }

    #[test]
    fn a_painting_names_what_changed_against_the_painting_before_it() {
        let mut watch = Watch::default();
        let mut batch = Batch::default();
        for owner in [1, 2, 3, 4, 6] {
            paint(&mut batch, owner, 10 + owner, [owner as f32, 0.0, 0.0], (owner == 3).then_some(7));
        }
        let first = watch.painted(&batch);
        assert_eq!(first.total.appeared, 5);
        assert_eq!(first.total.tracked, 1);
        batch.clear();
        paint(&mut batch, 1, 99, [1.0, 0.0, 0.0], None);
        paint(&mut batch, 3, 13, [9.0, 0.0, 0.0], None);
        paint(&mut batch, 4, 14, [9.0, 0.0, 0.0], None);
        paint(&mut batch, 5, 15, [5.0, 0.0, 0.0], None);
        paint(&mut batch, 6, 98, [7.0, 0.0, 0.0], None);
        let then = watch.painted(&batch);
        assert_eq!(
            [then.total.appeared, then.total.left, then.total.content, then.total.moved_shown, then.total.moved_untracked, then.total.moved_changed],
            [1, 1, 2, 1, 1, 1]
        );
        assert_eq!(then.by_pool, vec![("counters", then.total)]);
    }

    #[test]
    fn the_same_painting_taken_at_another_depth_is_not_a_change_of_content() {
        let mut watch = Watch::default();
        let mut batch = Batch::default();
        paint(&mut batch, 1, 10, [1.0, 0.0, 0.0], None);
        watch.painted(&batch);
        batch.clear();
        batch.at_window(
            WindowKey::new(WindowPool::LandCounter, 0, 1_u64),
            WindowStamp { painting: 10, taken: 2 },
        );
        batch.quads.push(quad([1.0, 0.0, 0.0], None));
        batch.no_window();
        let then = watch.painted(&batch);
        assert_eq!([then.total.content, then.total.order, then.total.appeared], [0, 1, 0]);
        assert!(!then.total.changed());
    }
}
