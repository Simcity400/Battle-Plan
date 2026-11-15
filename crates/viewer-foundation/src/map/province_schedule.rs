//! One off-screen province schedule per map view: the frame's province update list and the
//! presentation-dirty bit it drives for each province.
//!
//! Advance phase modulo 100 per frame. Update every admitted province; otherwise update dirty
//! provinces in their phase or under a positive force counter. Selection alone does not add a clean
//! off-screen province. Forced updates pass the in-view flag, retaining dirtiness. Raise selected
//! counters to at least two before updates and decrement each reached update, retaining one forced
//! frame after deselection. After updating, set the dirty bit for in-view provinces or newly
//! populated unit slots, then clear the population bit. Ordinary off-screen updates clear show bits
//! first and thus clear dirtiness. Reset initializes bit two; selected forced updates preserve it.
//!
//! Units and icon owners share this schedule's update list. Dirty off-screen provinces retain history
//! until their ordinary phased update. Advance the schedule once per frame before units and icon
//! pools read its list and bits.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use super::{camera::Camera, province_admission::Admission};

/// How a province is updated this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    /// Native in-view update flag 1: camera admission or a positive selected force counter.
    InView,
    /// Out of view, in the list by its bit, at its phase: updated with the in-view flag 0, which
    /// clears its show bits, so its owners erase what they hold.
    OffScreen,
}

/// Province ids, one bit each, iterated in ascending order.
#[derive(Clone, Debug, Default)]
struct Ids(Vec<u64>);

impl PartialEq for Ids {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl Ids {
    fn insert(&mut self, id: u32) {
        let word = id as usize / 64;
        if word >= self.0.len() {
            self.0.resize(word + 1, 0);
        }
        self.0[word] |= 1 << (id % 64);
    }

    fn remove(&mut self, id: &u32) {
        if let Some(word) = self.0.get_mut(*id as usize / 64) {
            *word &= !(1 << (id % 64));
        }
    }

    fn contains(&self, id: &u32) -> bool {
        self.0
            .get(*id as usize / 64)
            .is_some_and(|word| word >> (id % 64) & 1 != 0)
    }

    fn clear(&mut self) {
        self.0.fill(0);
    }

    fn retain(&mut self, keep: impl Fn(&u32) -> bool) {
        for (word, bits) in self.0.iter_mut().enumerate() {
            let mut set = *bits;
            while set != 0 {
                let bit = set.trailing_zeros();
                set &= set - 1;
                if !keep(&(word as u32 * 64 + bit)) {
                    *bits &= !(1 << bit);
                }
            }
        }
    }

    fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(word, &bits)| {
            let mut bits = bits;
            std::iter::from_fn(move || {
                (bits != 0).then(|| {
                    let bit = bits.trailing_zeros();
                    bits &= bits - 1;
                    word as u32 * 64 + bit
                })
            })
        })
    }
}

/// One view's phase, dirty bits and the frame's update list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schedule {
    /// The retained off-screen update cursor.
    phase: u32,
    /// The provinces whose bit `0x2` is clear; every other is set, as the constructor leaves it.
    clean: Ids,
    /// This frame's provinces updated with the native in-view flag 1.
    in_view: Ids,
    /// This frame's provinces updated out of view.
    off_screen: Ids,
    /// The frame's advance was made and its unit walk has not yet read it.
    pending: bool,
    /// The presentation clock of the last advance the view's counters made themselves
    /// ([`Self::present`]).
    presented: Option<f32>,
    /// Provinces containing this viewer's selected units, admitted while their selection count is positive.
    selected: BTreeSet<u32>,
    /// Selection counts rise to 2 on selection and decrease only when an update reaches the province.
    forced: BTreeMap<u32, u8>,
}

/// The schedule a view's scene and counters share.
pub type Shared = Arc<Mutex<Schedule>>;

/// Province-ordered updates shared by this frame's unit walk and text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceFrame {
    updates: Vec<(u32, Update)>,
}

impl ProvinceFrame {
    /// The reached updates in province order.
    pub fn updates(&self) -> &[(u32, Update)] {
        &self.updates
    }

    /// How `province` is updated this frame, or `None` when it is not ([`Schedule::update`]).
    pub fn update(&self, province: u32) -> Option<Update> {
        self.updates
            .binary_search_by_key(&province, |&(id, _)| id)
            .ok()
            .map(|at| self.updates[at].1)
    }
}

/// Force counts to restore if the frame never reaches its walk.
#[derive(Debug)]
pub struct FrameRestore {
    /// Each spent force count, by province, before it was spent.
    spent: Vec<(u32, u8)>,
}

impl Schedule {
    /// Canonical carried state for cross-crate presentation oracles.
    #[cfg(any(test, feature = "test-support"))]
    pub fn presentation_state(&self) -> impl PartialEq + std::fmt::Debug {
        let sorted = |values: &Ids| values.iter().collect::<BTreeSet<_>>();
        (
            self.phase,
            sorted(&self.clean),
            sorted(&self.in_view),
            sorted(&self.off_screen),
            self.pending,
            self.presented.map(f32::to_bits),
            self.selected.clone(),
            self.forced.clone(),
        )
    }

    pub fn set_selected(&mut self, selected: BTreeSet<u32>) {
        let previous = std::mem::replace(&mut self.selected, selected);
        // A click after the overlay advanced, before its unit walk, applies its force to
        // the already-built list. Repeated offers must not extend the same frame twice.
        if self.pending {
            for &id in self.selected.difference(&previous) {
                self.forced.insert(id, 2);
                if self.listed(id) {
                    self.off_screen.remove(&id);
                    self.in_view.insert(id);
                    self.clean.remove(&id);
                }
            }
        }
    }
    /// Prepare one scene frame; its unit walk consumes the pending advance.
    pub fn advance_frame(
        &mut self,
        bounds: &BTreeMap<u32, [f32; 4]>,
        provinces: impl IntoIterator<Item = u32>,
        camera: Camera,
        viewport: [u32; 2],
        steady: bool,
    ) {
        if !self.pending() {
            self.advance_view(bounds, provinces, camera, viewport, steady);
        }
    }

    /// One frame of and over `provinces`: the phase advanced, then each
    /// camera-admitted province listed and dirty. An out-of-view dirty province updates with
    /// flag 1 while forced, otherwise with flag 0 at its phase and becomes clean.
    /// `steady` (an offline frame settling for its camera) updates every
    /// dirty province out of view.
    pub fn advance(
        &mut self,
        provinces: impl IntoIterator<Item = u32>,
        in_view: impl Fn(u32) -> bool,
        steady: bool,
    ) {
        self.advance_admitted(provinces.into_iter().map(|id| (id, in_view(id))), steady);
    }

    /// Advance over retained province boxes without looking each one up in a tree.
    /// The caller refreshes this list whenever its province roster or boxes change.
    pub fn advance_frame_boxes(
        &mut self,
        provinces: &[(u32, Option<[f32; 4]>)],
        camera: Camera,
        viewport: [u32; 2],
        steady: bool,
    ) {
        if self.pending {
            return;
        }
        let admission = Admission::new(camera, viewport);
        self.advance_admitted(
            provinces
                .iter()
                .map(|&(id, bounds)| (id, bounds.is_some_and(|bounds| admission.admits(bounds)))),
            steady,
        );
    }

    fn advance_admitted(&mut self, provinces: impl IntoIterator<Item = (u32, bool)>, steady: bool) {
        // Direct callers finish the previous list before asking for another frame.
        // A scene instead retains it through advance_frame until its unit walk.
        self.take_pending();
        self.phase = (self.phase + 1) % 100;
        self.in_view.clear();
        self.off_screen.clear();
        for &id in &self.selected {
            self.forced.insert(id, 2);
        }
        for (id, admitted) in provinces {
            if admitted {
                self.in_view.insert(id);
                self.clean.remove(&id);
            } else if self.dirty(id) {
                if self.forced.contains_key(&id) {
                    self.in_view.insert(id);
                } else if steady || id % 100 == self.phase {
                    self.off_screen.insert(id);
                    self.clean.insert(id);
                }
            }
        }
        self.pending = true;
    }

    /// [`Self::advance`] at a camera: a province is in view when the camera admits its box.
    pub fn advance_view(
        &mut self,
        bounds: &BTreeMap<u32, [f32; 4]>,
        provinces: impl IntoIterator<Item = u32>,
        camera: Camera,
        viewport: [u32; 2],
        steady: bool,
    ) {
        let admission = Admission::new(camera, viewport);
        self.advance(
            provinces,
            |id| admission.admits_province(bounds, id),
            steady,
        );
    }

    /// The advance for a view whose scene does not advance the schedule (a headless frame, a
    /// window before its scene exists): made by its counters over `bounds`, once per presented
    /// frame of the view's presentation clock (`time`); nothing when the scene's advance of this
    /// frame is pending. The counters' own advance is not left pending for a unit walk.
    pub fn present(
        &mut self,
        bounds: &BTreeMap<u32, [f32; 4]>,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
    ) {
        if self.pending || self.presented == Some(time) {
            return;
        }
        let admission = Admission::new(camera, viewport);
        self.advance_admitted(
            bounds
                .iter()
                .map(|(&id, &bounds)| (id, admission.admits(bounds))),
            false,
        );
        self.take_pending();
        self.presented = Some(time);
    }

    /// How `province` is updated this frame, or `None` when it is not.
    pub fn update(&self, province: u32) -> Option<Update> {
        if self.in_view.contains(&province) {
            Some(Update::InView)
        } else if self.off_screen.contains(&province) {
            Some(Update::OffScreen)
        } else {
            None
        }
    }

    /// This frame's reached updates. Owners that share native province order sort by ID
    /// before walking; dirty provinces waiting for their phase are not reached updates.
    pub fn updates(&self) -> impl Iterator<Item = (u32, Update)> + '_ {
        self.in_view
            .iter()
            .map(|id| (id, Update::InView))
            .chain(self.off_screen.iter().map(|id| (id, Update::OffScreen)))
    }

    /// Whether `province` is in this frame's update list: in view, or out of view with its dirty bit
    /// set when the list was built, regardless of whether this phase updates it ([`Self::update`]).
    pub fn listed(&self, province: u32) -> bool {
        self.update(province).is_some() || self.dirty(province)
    }

    /// Whether this frame's advance was made and its unit walk has not yet taken it.
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// Consume the frame's work and spend one force count for each reached update.
    /// Preparing a list alone cannot spend the final frame after deselection: a
    /// refused scene may leave that list unread until another hour replaces it.
    pub fn take_pending(&mut self) -> bool {
        let pending = std::mem::take(&mut self.pending);
        if pending {
            for id in self.in_view.iter().chain(self.off_screen.iter()) {
                if let Some(remaining) = self.forced.get_mut(&id) {
                    *remaining -= 1;
                    if *remaining == 0 {
                        self.forced.remove(&id);
                    }
                }
            }
        }
        pending
    }

    /// Takes pending updates in province order, spending force counts once.
    pub fn take_frame(&mut self) -> Option<(ProvinceFrame, FrameRestore)> {
        if !self.pending {
            return None;
        }
        let spent = self
            .in_view
            .iter()
            .chain(self.off_screen.iter())
            .filter_map(|id| self.forced.get(&id).map(|&count| (id, count)))
            .collect();
        self.take_pending();
        let mut updates: Vec<_> = self.updates().collect();
        updates.sort_unstable_by_key(|&(id, _)| id);
        Some((ProvinceFrame { updates }, FrameRestore { spent }))
    }

    /// Puts a taken frame back as if never taken: its walk did not run.
    pub fn untake_frame(&mut self, restore: FrameRestore) {
        self.pending = true;
        for (id, count) in restore.spent {
            self.forced.insert(id, count);
        }
    }

    /// A successfully applied hour supersedes a scene frame whose unit walk
    /// never ran (for example, a viewport too small for the native post seed).
    /// Restore its unconsumed dirty work and retain the force counts, selection,
    /// phase and the map-icon owners' pools; the next frame builds a fresh list.
    pub fn cancel_pending(&mut self) {
        if !std::mem::take(&mut self.pending) {
            return;
        }
        self.clean.retain(|id| !self.off_screen.contains(id));
        self.in_view.clear();
        self.off_screen.clear();
        self.presented = None;
    }

    /// Whether `province`'s bit `0x2` is set.
    pub fn dirty(&self, province: u32) -> bool {
        !self.clean.contains(&province)
    }

    /// Whether `province` is dirty out of view: a coming frame's phase cleans it, and its owners'
    /// icons kept off screen are erased then. A dirty province in view stays dirty while it is.
    pub fn cleans(&self, province: u32) -> bool {
        self.dirty(province) && self.update(province) != Some(Update::InView)
    }

    /// Reset listed provinces, or every province, to dirty bit two.
    pub fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        match provinces {
            None => self.clean.clear(),
            Some(provinces) => self.clean.retain(|id| !provinces.contains(id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::camera::Pose;

    /// Restoring a cut must undo its spent force counts, not just its pending bit.
    #[test]
    fn taken_frames_match_the_walk_and_restore_exactly() {
        // An owned snapshot: the carried state borrows its schedule.
        let state = |schedule: &Schedule| format!("{:?}", schedule.presentation_state());
        let mut schedules = [Schedule::default(), Schedule::default()];
        for schedule in &mut schedules {
            schedule.set_selected(BTreeSet::from([4, 7]));
            schedule.advance([1, 2, 4, 7, 103, 205], |id| id.is_multiple_of(2), false);
            schedule.set_selected(BTreeSet::from([4, 7, 103]));
        }
        let before = state(&schedules[1]);
        let walked: Vec<_> = {
            let walk = &mut schedules[0];
            assert!(walk.take_pending());
            let mut updates: Vec<_> = walk.updates().collect();
            updates.sort_unstable_by_key(|&(id, _)| id);
            updates
        };
        let (frame, restore) = schedules[1].take_frame().expect("an advance is pending");
        assert_eq!(frame.updates(), walked.as_slice());
        assert_eq!(state(&schedules[1]), state(&schedules[0]));
        for &(id, update) in &walked {
            assert_eq!(frame.update(id), Some(update));
        }
        assert_eq!(frame.update(3), None);
        schedules[1].untake_frame(restore);
        assert_eq!(state(&schedules[1]), before);
        // Nothing pending: nothing taken, nothing changed.
        let mut idle = Schedule::default();
        let idle_state = state(&idle);
        assert!(idle.take_frame().is_none());
        assert_eq!(state(&idle), idle_state);
    }

    #[test]
    fn retained_boxes_match_tree_admission_across_selection_reset_and_pending_frames() {
        use crate::map::camera::{MapDefines, MapSize, Placement};
        let bounds = super::super::province_admission::edge_boxes();
        let mut boxes: Vec<_> = bounds
            .iter()
            .map(|(&id, &bounds)| (id, Some(bounds)))
            .collect();
        boxes.extend([(0, None), (u32::MAX, None)]);
        let mut retained = Schedule::default();
        let mut tree = Schedule::default();
        for frame in 0..350 {
            let camera = Placement {
                center: [3000. + (frame % 30) as f32 * 3., 1000.],
                height: if frame % 37 == 0 { 900. } else { 50. },
                pose: Pose::Map,
            }
            .on(MapSize::TEST, MapDefines::test());
            let selected = if frame % 13 < 7 {
                BTreeSet::from([3, u32::MAX])
            } else {
                BTreeSet::new()
            };
            for schedule in [&mut retained, &mut tree] {
                schedule.set_selected(selected.clone());
                if frame % 31 == 0 {
                    schedule.reset(Some(&BTreeSet::from([0, 1, 3, u32::MAX])));
                }
                if frame % 19 == 0 {
                    schedule.cancel_pending();
                }
            }
            let steady = frame % 29 == 0;
            retained.advance_frame_boxes(&boxes, camera, [1024, 768], steady);
            tree.advance_frame(
                &bounds,
                boxes.iter().map(|&(id, _)| id),
                camera,
                [1024, 768],
                steady,
            );
            for &(id, _) in &boxes {
                assert_eq!(
                    retained.update(id),
                    tree.update(id),
                    "frame {frame}, province {id}"
                );
                assert_eq!(retained.dirty(id), tree.dirty(id));
                assert_eq!(retained.listed(id), tree.listed(id));
            }
            assert_eq!(retained.phase, tree.phase);
            assert_eq!(retained.forced, tree.forced);
            if frame % 7 != 0 {
                assert_eq!(retained.take_pending(), tree.take_pending());
            }
        }
    }

    #[test]
    fn reached_updates_exclude_dirty_waiters_and_follow_pending_selection() {
        let mut schedule = Schedule::default();
        schedule.advance([103, 101, 102], |id| id == 103, false);
        let updates = |schedule: &Schedule| schedule.updates().collect::<BTreeMap<_, _>>();
        assert_eq!(
            updates(&schedule),
            BTreeMap::from([(101, Update::OffScreen), (103, Update::InView),])
        );
        assert!(schedule.listed(102));
        schedule.set_selected(BTreeSet::from([101, 102]));
        assert_eq!(
            updates(&schedule),
            BTreeMap::from([
                (101, Update::InView),
                (102, Update::InView),
                (103, Update::InView),
            ])
        );
        schedule.take_pending();
        assert_eq!(updates(&schedule).len(), 3);
        schedule.cancel_pending();
        schedule.advance([103, 101, 102], |_| false, true);
        assert_eq!(updates(&schedule).len(), 3);
    }

    #[test]
    fn selected_province_bypasses_phase_even_after_a_pending_advance() {
        let mut schedule = Schedule::default();
        schedule.advance([88, 99], |_| false, false);
        assert_eq!(schedule.update(88), None);
        schedule.set_selected(BTreeSet::from([88]));
        assert_eq!(schedule.update(88), Some(Update::InView));
        assert_eq!(schedule.update(99), None);
        assert!(schedule.dirty(88));
        schedule.reset(None);
        schedule.advance([88, 99], |_| false, false);
        assert_eq!(schedule.update(88), Some(Update::InView));
        assert_eq!(schedule.update(99), None);
    }

    /// A province is dirty from its constructor or its last update in view until the frame
    /// whose phase is its id modulo 100 updates it out of view; a clean province out of view is
    /// not in the list and is never updated.
    #[test]
    fn a_province_stays_dirty_until_its_phase_updates_it_out_of_view() {
        let mut schedule = Schedule::default();
        let provinces = [3, 103, 7];
        // The constructor leaves every bit set.
        assert!(provinces.iter().all(|&id| schedule.dirty(id)));
        // Frames 1 and 2 update 1 and 2 out of view: nothing here clears.
        schedule.advance(provinces, |id| id == 7, false);
        assert_eq!(schedule.update(7), Some(Update::InView));
        assert_eq!(schedule.update(3), None);
        schedule.advance(provinces, |id| id == 7, false);
        assert!(schedule.dirty(3) && schedule.dirty(103));
        // Frame 3: both provinces of phase 3 are updated out of view.
        schedule.advance(provinces, |_| false, false);
        assert!(!schedule.dirty(3) && !schedule.dirty(103));
        assert_eq!(schedule.update(3), Some(Update::OffScreen));
        assert_eq!(schedule.update(103), Some(Update::OffScreen));
        // 7 was in view until frame 2 and stays dirty until frame 7.
        for _ in 4..7 {
            schedule.advance(provinces, |_| false, false);
            assert!(schedule.dirty(7));
            assert_eq!(schedule.update(7), None);
        }
        schedule.advance(provinces, |_| false, false);
        assert!(!schedule.dirty(7));
        assert_eq!(schedule.update(7), Some(Update::OffScreen));
        // A hundred frames on, 3 is at its phase again but clean: not in the list.
        for _ in 0..96 {
            schedule.advance(provinces, |_| false, false);
        }
        assert_eq!(schedule.update(3), None);
        // In view again: dirty at once; a reset sets the bit too.
        schedule.advance(provinces, |id| id == 3, false);
        assert!(schedule.dirty(3));
        schedule.reset(Some(&BTreeSet::from([103])));
        assert!(schedule.dirty(103) && !schedule.dirty(7));
        schedule.reset(None);
        assert!(schedule.dirty(7));
        // A settling frame updates every dirty province out of view.
        schedule.advance(provinces, |_| false, true);
        assert!(provinces.iter().all(|&id| !schedule.dirty(id)));
        assert!(schedule.pending() && schedule.take_pending());
        assert!(!schedule.pending() && !schedule.take_pending());
    }

    /// A province is in view while the camera admits its box: past the view's
    /// right edge within the side planes' ten-unit margin, and south of its bottom edge while the
    /// box's top, 70 up, shows.
    #[test]
    fn provinces_the_camera_admits_are_in_view() {
        let camera = super::super::camera::Placement {
            center: [3000.0, 1000.0],
            height: 50.0,
            pose: Pose::Map,
        }
        .on(
            super::super::camera::MapSize::TEST,
            super::super::camera::MapDefines::test(),
        );
        let bounds = super::super::province_admission::edge_boxes();
        assert_eq!(bounds[&5], [3044.0, 1000.0, 3046.0, 1002.0]);
        let mut schedule = Schedule::default();
        schedule.advance_frame(&bounds, bounds.keys().copied(), camera, [1024, 768], false);
        let phase = schedule.phase;
        schedule.advance_frame(&bounds, bounds.keys().copied(), camera, [1024, 768], false);
        assert_eq!(
            schedule.phase, phase,
            "a pending frame cannot advance twice"
        );
        assert!(schedule.take_pending());
        schedule.advance_frame(&bounds, bounds.keys().copied(), camera, [1024, 768], false);
        assert_eq!(schedule.phase, phase + 1);
        assert_eq!(schedule.update(1), Some(Update::InView));
        assert_eq!(schedule.update(2), Some(Update::InView));
        assert_eq!(schedule.update(3), None);
        assert_eq!(schedule.update(4), None);
        assert_eq!(schedule.update(5), Some(Update::InView));
        assert_eq!(schedule.update(6), Some(Update::InView));
    }

    /// A view whose scene does not advance the schedule advances it from its counters once per
    /// presented frame, and never over a pending advance of the scene.
    #[test]
    fn the_counters_advance_only_what_no_scene_advanced() {
        let mut schedule = Schedule::default();
        let bounds = BTreeMap::from([(1, [0.0, 0.0, 2.0, 2.0])]);
        let camera = Camera {
            center: [1.0, 1.0],
            height: 300.0,
            map: super::super::camera::MapSize::TEST,
            defines: super::super::camera::MapDefines::test(),
            pose: Pose::Map,
        };
        schedule.present(&bounds, camera, [64, 64], 1.0);
        let phase = schedule.phase;
        assert!(!schedule.pending());
        schedule.present(&bounds, camera, [64, 64], 1.0);
        assert_eq!(schedule.phase, phase);
        schedule.present(&bounds, camera, [64, 64], 2.0);
        assert_eq!(schedule.phase, phase + 1);
        schedule.advance([1], |_| true, false);
        schedule.present(&bounds, camera, [64, 64], 3.0);
        assert_eq!(schedule.phase, phase + 2);
        assert!(schedule.pending());
    }

    #[test]
    fn selected_offscreen_linger_keeps_native_flag_and_one_frame_after_deselection() {
        let mut schedule = Schedule::default();
        schedule.advance([88], |_| true, false);
        schedule.take_pending();
        schedule.set_selected(BTreeSet::from([88]));
        for _ in 0..101 {
            schedule.advance([88], |_| false, false);
            assert_eq!(schedule.update(88), Some(Update::InView));
            assert!(schedule.dirty(88) && schedule.listed(88));
            schedule.take_pending();
        }
        schedule.set_selected(BTreeSet::new());
        schedule.advance([88], |_| false, false);
        assert_eq!(schedule.update(88), Some(Update::InView));
        assert!(schedule.dirty(88));
        schedule.advance([88], |_| false, false);
        assert_eq!(schedule.update(88), None);
        assert!(schedule.dirty(88) && schedule.listed(88));
        while schedule.phase != 88 {
            schedule.advance([88], |_| false, false);
        }
        assert_eq!(schedule.update(88), Some(Update::OffScreen));
        assert!(schedule.listed(88) && !schedule.dirty(88));
        schedule.advance([88], |_| false, false);
        assert!(!schedule.listed(88));
    }

    #[test]
    fn selecting_clean_offscreen_province_does_not_add_it_to_native_list() {
        let mut schedule = Schedule::default();
        schedule.advance([1], |_| false, false);
        assert_eq!(schedule.update(1), Some(Update::OffScreen));
        schedule.advance([1], |_| false, false);
        assert!(!schedule.listed(1));
        schedule.set_selected(BTreeSet::from([1]));
        assert!(!schedule.listed(1));
        schedule.advance([1], |_| false, false);
        assert!(!schedule.listed(1));
        assert_eq!(schedule.update(1), None);
        // A reset puts the province back in the list; its unconsumed force then applies.
        schedule.reset(Some(&BTreeSet::from([1])));
        schedule.advance([1], |_| false, false);
        assert_eq!(schedule.update(1), Some(Update::InView));
        assert!(schedule.dirty(1));
    }

    #[test]
    fn repeated_pending_selection_offer_does_not_extend_deselection_grace() {
        let mut schedule = Schedule::default();
        schedule.advance([88], |_| true, false);
        schedule.set_selected(BTreeSet::from([88]));
        schedule.set_selected(BTreeSet::from([88]));
        schedule.take_pending();
        schedule.set_selected(BTreeSet::new());
        schedule.advance([88], |_| false, false);
        assert_eq!(schedule.update(88), Some(Update::InView));
        schedule.advance([88], |_| false, false);
        assert_eq!(schedule.update(88), None);
    }

    #[test]
    fn tiny_post_seed_abort_preserves_final_deselection_work_until_consumed() {
        let mut schedule = Schedule::default();
        schedule.set_selected(BTreeSet::from([88]));
        schedule.advance([2, 88, 99], |_| false, false);
        assert!(schedule.take_pending());
        assert_eq!(schedule.forced.get(&88), Some(&1));
        schedule.set_selected(BTreeSet::new());
        schedule.advance([2, 88, 99], |_| false, false);
        assert_eq!(schedule.update(2), Some(Update::OffScreen));
        assert_eq!(schedule.update(88), Some(Update::InView));

        // Scene::render refuses this native post seed before update_models can
        // take the list the overlay already advanced.
        let tiny_world = [1_u32, 64];
        assert!(
            super::super::gpu_contract::validate_size(tiny_world.map(|side| side / 2), 8192)
                .is_err()
        );
        assert!(schedule.pending());
        assert_eq!(schedule.forced.get(&88), Some(&1));
        // Successful hour, seek, branch or perspective applications all use
        // this boundary; an abandoned update cannot spend the force or clean 2.
        for _ in 0..4 {
            let phase = schedule.phase;
            schedule.cancel_pending();
            assert!(!schedule.pending());
            assert!(schedule.dirty(2) && schedule.dirty(88));
            assert_eq!(schedule.update(2), None);
            assert_eq!(schedule.update(88), None);
            assert_eq!(schedule.forced.get(&88), Some(&1));
            assert_eq!(schedule.phase, phase);
            assert!(schedule.selected.is_empty());
            // Build a replacement camera list, then refuse its render again.
            schedule.advance([2, 88, 99], |id| id == 99, false);
            assert_eq!(schedule.update(88), Some(Update::InView));
            assert_eq!(schedule.update(99), Some(Update::InView));
            assert_eq!(schedule.forced.get(&88), Some(&1));
        }
        // The real unit walk consumes the final force exactly once.
        assert!(schedule.take_pending());
        assert!(!schedule.take_pending());
        assert!(!schedule.forced.contains_key(&88));
        schedule.advance([2, 88, 99], |_| false, false);
        assert_eq!(schedule.update(88), None);
        assert!(schedule.dirty(88));
    }

    #[test]
    fn unconsumed_scene_list_holds_until_applied_hour_cancels_it() {
        let bounds = BTreeMap::from([
            (5, [2998.0, 998.0, 3002.0, 1002.0]),
            (88, [3998.0, 998.0, 4002.0, 1002.0]),
        ]);
        let camera = Camera {
            center: [3000.0, 1000.0],
            height: 50.0,
            map: super::super::camera::MapSize::TEST,
            defines: super::super::camera::MapDefines::test(),
            pose: Pose::Map,
        };
        let mut schedule = Schedule::default();
        schedule.advance_frame(&bounds, bounds.keys().copied(), camera, [1, 64], false);
        assert_eq!(schedule.update(5), Some(Update::InView));
        assert_eq!(schedule.update(88), None);
        let phase = schedule.phase;
        let moved = Camera {
            center: [4000.0, 1000.0],
            ..camera
        };
        // A failed hour apply never calls cancel_pending, so retries retain the
        // list and phase instead of consuming work or admitting a second list.
        schedule.advance_frame(&bounds, bounds.keys().copied(), moved, [1024, 768], false);
        assert_eq!(schedule.phase, phase);
        assert_eq!(schedule.update(5), Some(Update::InView));
        assert_eq!(schedule.update(88), None);
        schedule.cancel_pending();
        assert_eq!(schedule.phase, phase);
        schedule.advance_frame(&bounds, bounds.keys().copied(), moved, [1024, 768], false);
        assert_eq!(schedule.phase, phase + 1);
        assert_eq!(schedule.update(5), None);
        assert_eq!(schedule.update(88), Some(Update::InView));
        assert!(schedule.take_pending());
    }

    #[test]
    fn late_selection_cancel_retains_unconsumed_force_without_adding_clean_provinces() {
        let mut schedule = Schedule::default();
        schedule.advance([1, 88], |_| false, false);
        assert_eq!(schedule.update(1), Some(Update::OffScreen));
        schedule.set_selected(BTreeSet::from([1, 88]));
        assert_eq!(schedule.update(1), Some(Update::InView));
        assert_eq!(schedule.update(88), Some(Update::InView));
        schedule.set_selected(BTreeSet::from([1, 88]));
        schedule.set_selected(BTreeSet::new());
        schedule.cancel_pending();
        assert_eq!(schedule.forced.get(&88), Some(&2));
        for remaining in [1, 0] {
            schedule.advance([1, 88], |_| false, false);
            assert_eq!(schedule.update(88), Some(Update::InView));
            assert!(schedule.take_pending());
            assert_eq!(schedule.forced.get(&88).copied().unwrap_or(0), remaining);
        }
        schedule.advance([1, 88], |_| false, false);
        assert_eq!(schedule.update(88), None);

        schedule.advance([5], |_| false, true);
        schedule.take_pending();
        schedule.advance([5], |_| false, false);
        schedule.set_selected(BTreeSet::from([5]));
        assert!(!schedule.listed(5));
        schedule.cancel_pending();
        schedule.advance([5], |_| false, false);
        assert!(!schedule.listed(5));
        assert_eq!(schedule.forced.get(&5), Some(&2));
    }

    /// A province dirty in view stays dirty and does not clean; one dirty out of view cleans until
    /// its phase updates it out of view, which makes it clean.
    #[test]
    fn a_province_cleans_only_while_dirty_out_of_view() {
        let mut schedule = Schedule::default();
        schedule.advance([1, 2], |id| id == 1, false);
        assert!(schedule.dirty(1) && !schedule.cleans(1), "dirty in view");
        assert!(schedule.dirty(2) && schedule.cleans(2), "dirty out of view, before its phase");
        schedule.advance([1, 2], |id| id == 1, false);
        assert_eq!(schedule.update(2), Some(Update::OffScreen), "its phase");
        assert!(!schedule.dirty(2) && !schedule.cleans(2), "cleaned at its phase");
        assert!(!schedule.cleans(1));
    }

    #[test]
    fn counter_only_frames_consume_forces_once_and_cancellation_preserves_consumed_state() {
        let mut schedule = Schedule::default();
        let bounds = BTreeMap::from([(88, [10000.0, 10000.0, 10002.0, 10002.0])]);
        let camera = Camera {
            center: [1.0, 1.0],
            height: 50.0,
            map: super::super::camera::MapSize::TEST,
            defines: super::super::camera::MapDefines::test(),
            pose: Pose::Map,
        };
        schedule.set_selected(BTreeSet::from([88]));
        schedule.present(&bounds, camera, [64, 64], 1.0);
        assert_eq!(schedule.update(88), Some(Update::InView));
        assert_eq!(schedule.forced.get(&88), Some(&1));
        assert!(!schedule.pending());
        schedule.cancel_pending();
        assert_eq!(schedule.presented, Some(1.0));
        schedule.set_selected(BTreeSet::new());
        schedule.present(&bounds, camera, [64, 64], 1.0);
        assert_eq!(schedule.forced.get(&88), Some(&1));
        schedule.present(&bounds, camera, [64, 64], 2.0);
        assert_eq!(schedule.update(88), Some(Update::InView));
        assert!(!schedule.forced.contains_key(&88));
        schedule.present(&bounds, camera, [64, 64], 3.0);
        assert_eq!(schedule.update(88), None);
    }
}
