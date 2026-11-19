//! What one painting of the overlay changed against the painting before it.
//!
//! The gate asks what a watcher sees between two publications: a window that appeared, left or
//! drew different content, and a mover placed where its hours put it or standing on its
//! publication's own point because the recording gives its rows no places over the hours. Each
//! painting is compared with the one before it by window ([`super::sprites::WindowKey`]), which is
//! the identity a window keeps across publications, so a change is named by its kind and the pool
//! it happened in.
//!
//! Counted only when asked (`BATTLE_PLAN_VIEWER_COUNT_OVERLAY`): it walks every window of every painting,
//! so a timing run never pays for it.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::sprites::{Batch, WindowKey, WindowPool, WindowStamp};

/// Whether the overlay's changes between paintings are counted.
pub fn counting() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_COUNT_OVERLAY").or_else(|| std::env::var_os("HOI4_VIEWER_COUNT_OVERLAY")).is_some())
}

/// Where a window stood in the painting before: what it drew, the world point its publication
/// placed it at, and whether it carries places over the hours (a track).
#[derive(Clone, Copy)]
struct Seen {
    stamp: WindowStamp,
    world: Option<[f32; 3]>,
    tracked: bool,
}

/// What changed in one pool between two paintings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub windows: u64,
    pub appeared: u64,
    pub left: u64,
    /// The same window drawing different content: a number, an icon or a label that changed.
    pub content: u64,
    /// The same content taken at another depth or opacity, which a watcher sees only where it
    /// overlaps another window.
    pub order: u64,
    /// A window its publication placed at another world point that the painting before it carried
    /// places over the hours for: the display frames between the two publications are the frames
    /// that placed it, each at its own hour, so they showed it moving.
    pub moved_shown: u64,
    /// The same with no places to move it on, drawing what it drew before: a plate the recording
    /// moved inside the interval with nothing for the frames of that interval to move it on.
    pub moved_untracked: u64,
    /// The same with no places, drawing something else: the stack itself changed (a member joined
    /// or left it, a number or an icon changed), which is a discrete change of the publication and
    /// not motion the frames could have shown.
    pub moved_changed: u64,
    /// Windows carrying places over the hours.
    pub tracked: u64,
}

impl Counts {
    /// Each count under the name the record carries it by.
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

    /// Whether anything a watcher could see changed here.
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

/// What one painting changed: the whole of it, each pool's own, and the anchored quads belonging to
/// no window, which this count cannot name.
#[derive(Clone, Debug, Default)]
pub struct Changed {
    pub total: Counts,
    pub by_pool: Vec<(&'static str, Counts)>,
    pub loose_anchored: u64,
}

impl Changed {
    /// Writes the counts into this frame's record, each pool's under its own name.
    pub fn record(&self) {
        for (name, value) in self.total.named() {
            crate::view_record::add("overlay count", name, value);
        }
        for (pool, counts) in &self.by_pool {
            for (name, value) in counts.named() {
                if value > 0 {
                    crate::view_record::add_named("overlay count", &format!("{pool}.{name}"), value);
                }
            }
        }
        crate::view_record::add("overlay count", "loose_anchored", self.loose_anchored);
        crate::view_record::add("overlay count", "paintings", 1);
    }
}

/// The painting before, by window.
#[derive(Default)]
pub struct Watch {
    held: HashMap<WindowKey, Seen>,
    seen: HashMap<WindowKey, Seen>,
}

impl Watch {
    /// Compares `batch`, just painted, with the painting before it. The first painting of a run
    /// reports every window as having appeared, which the reader drops with the warm-up frames.
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
                    // Whether a watcher saw this move is decided by the painting it was moved
                    // *from*: those are the places the display frames of the interval had to
                    // place it at their own hours. The places this painting carries are for the
                    // interval that starts here.
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

/// The name a pool's counts are recorded under.
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
    use crate::map::{
        free_icons::{IconAnchor, IconFamily},
        sprites::{Quad, WindowPool},
    };

    fn quad(world: [f32; 3], moves: Option<u32>) -> Quad {
        Quad {
            texture: 0,
            rect: [0.0; 4],
            uv: [0.0; 4],
            color: [1.0; 4],
            rotation: 0.0,
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

    /// Each kind of change one publication can bring, named: a plate that appeared, one that left,
    /// one drawing a new number, one the frames of the interval had places over the hours to move
    /// on, one they had none for, and one whose stack changed as it moved.
    ///
    /// A move is judged by the painting it was moved **from**: those are the places the display
    /// frames of the interval were given. A plate that arrives and stops carries no places in the
    /// painting that places it at its destination, and the frames that carried it there were still
    /// shown its motion.
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
        // 1 drew a new number, 2 is gone, 3 moved on the places it carried and now stands,
        // 4 moved with none, 5 is new, 6 moved and drew something else.
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

    /// A window drawing the same painting at another depth is not a change a watcher sees where
    /// nothing overlaps it, and is counted apart from one that drew something else.
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
