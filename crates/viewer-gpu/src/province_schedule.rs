
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use super::{camera::Camera, province_admission::Admission};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    InView,
    OffScreen,
}

const EMPTY: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct WordSlot {
    key: u32,
    pos: u32,
    bits: u64,
}

impl WordSlot {
    const EMPTY: Self = Self {
        key: EMPTY,
        pos: 0,
        bits: 0,
    };
}

fn word_hash(key: u32) -> usize {
    let mut x = key;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x as usize
}

#[derive(Clone, Debug, Default)]
pub struct ProvinceSet {
    slots: Vec<WordSlot>,
    occupied: Vec<u32>,
}

impl PartialEq for ProvinceSet {
    fn eq(&self, other: &Self) -> bool {
        self.occupied.len() == other.occupied.len()
            && self.occupied.iter().all(|&slot| {
                let entry = self.slots[slot as usize];
                other.word_bits(entry.key) == entry.bits
            })
    }
}

impl Eq for ProvinceSet {}

impl ProvinceSet {
    fn find_slot(&self, key: u32) -> Option<usize> {
        if self.occupied.is_empty() {
            return None;
        }
        let mask = self.slots.len() - 1;
        let mut slot = word_hash(key) & mask;
        loop {
            let entry = self.slots[slot];
            if entry.key == key {
                return Some(slot);
            }
            if entry.key == EMPTY {
                return None;
            }
            slot = (slot + 1) & mask;
        }
    }

    fn word_bits(&self, key: u32) -> u64 {
        self.find_slot(key)
            .map_or(0, |slot| self.slots[slot].bits)
    }

    fn grow(&mut self) {
        let next_len = (self.slots.len() * 2).max(8);
        let mask = next_len - 1;
        let mut next = vec![WordSlot::EMPTY; next_len];
        for (pos, old_slot) in self.occupied.iter_mut().enumerate() {
            let mut entry = self.slots[*old_slot as usize];
            let mut slot = word_hash(entry.key) & mask;
            while next[slot].key != EMPTY {
                slot = (slot + 1) & mask;
            }
            entry.pos = pos as u32;
            next[slot] = entry;
            *old_slot = slot as u32;
        }
        self.slots = next;
    }

    pub fn insert(&mut self, id: u32) -> bool {
        let key = id >> 6;
        let bit = 1u64 << (id & 63);
        if self.slots.is_empty() {
            self.grow();
        }
        let mask = self.slots.len() - 1;
        let mut slot = word_hash(key) & mask;
        loop {
            let entry = &mut self.slots[slot];
            if entry.key == key {
                let fresh = entry.bits & bit == 0;
                entry.bits |= bit;
                return fresh;
            }
            if entry.key == EMPTY {
                break;
            }
            slot = (slot + 1) & mask;
        }
        if (self.occupied.len() + 1) * 2 > self.slots.len() {
            self.grow();
            let mask = self.slots.len() - 1;
            slot = word_hash(key) & mask;
            while self.slots[slot].key != EMPTY {
                slot = (slot + 1) & mask;
            }
        }
        let pos = self.occupied.len() as u32;
        self.slots[slot] = WordSlot {
            key,
            pos,
            bits: bit,
        };
        self.occupied.push(slot as u32);
        true
    }

    fn remove_word_bits(&mut self, key: u32, clear_bits: u64) {
        let Some(slot) = self.find_slot(key) else {
            return;
        };
        self.slots[slot].bits &= !clear_bits;
        if self.slots[slot].bits != 0 {
            return;
        }
        let pos = self.slots[slot].pos as usize;
        self.occupied.swap_remove(pos);
        if pos < self.occupied.len() {
            let moved_slot = self.occupied[pos] as usize;
            self.slots[moved_slot].pos = pos as u32;
        }
        let mask = self.slots.len() - 1;
        let mut hole = slot;
        let mut next = (hole + 1) & mask;
        while self.slots[next].key != EMPTY {
            let ideal = word_hash(self.slots[next].key) & mask;
            if (hole.wrapping_sub(ideal) & mask) < (next.wrapping_sub(ideal) & mask) {
                let moved = self.slots[next];
                self.slots[hole] = moved;
                self.occupied[moved.pos as usize] = hole as u32;
                hole = next;
            }
            next = (next + 1) & mask;
        }
        self.slots[hole] = WordSlot::EMPTY;
    }

    pub fn remove(&mut self, id: u32) {
        self.remove_word_bits(id >> 6, 1u64 << (id & 63));
    }

    pub fn remove_all(&mut self, other: &Self) {
        if self.occupied.is_empty() {
            return;
        }
        for &slot in &other.occupied {
            let entry = other.slots[slot as usize];
            self.remove_word_bits(entry.key, entry.bits);
        }
    }

    pub fn contains(&self, id: u32) -> bool {
        let bit = 1u64 << (id & 63);
        self.word_bits(id >> 6) & bit != 0
    }

    pub fn is_empty(&self) -> bool {
        self.occupied.is_empty()
    }

    pub fn clear(&mut self) {
        if self.occupied.is_empty() {
            return;
        }
        if self.occupied.len() * 4 >= self.slots.len() {
            self.slots.fill(WordSlot::EMPTY);
        } else {
            for &slot in &self.occupied {
                self.slots[slot as usize] = WordSlot::EMPTY;
            }
        }
        self.occupied.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.occupied.iter().flat_map(|&slot| {
            let entry = self.slots[slot as usize];
            let base = entry.key << 6;
            let mut bits = entry.bits;
            std::iter::from_fn(move || {
                (bits != 0).then(|| {
                    let bit = bits.trailing_zeros();
                    bits &= bits - 1;
                    base | bit
                })
            })
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn slot_capacity(&self) -> usize {
        self.slots.capacity()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schedule {
    phase: u32,
    clean: ProvinceSet,
    in_view: ProvinceSet,
    off_screen: ProvinceSet,
    pending: bool,
    presented: Option<f32>,
    candidates: Vec<(u32, [f32; 4])>,
    admitted_against: super::camera::Hold,
    candidates_for: Option<(Camera, [u32; 2], u64, super::camera::Slack)>,
    admitted: ProvinceSet,
    admitted_for: Option<(Camera, [u32; 2], u64)>,
    selected: BTreeSet<u32>,
    forced: BTreeMap<u32, u8>,
}

pub type Shared = Arc<Mutex<Schedule>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceFrame {
    updates: Vec<(u32, Update)>,
}

impl ProvinceFrame {
    pub fn updates(&self) -> &[(u32, Update)] {
        &self.updates
    }

    pub fn update(&self, province: u32) -> Option<Update> {
        self.updates
            .binary_search_by_key(&province, |&(id, _)| id)
            .ok()
            .map(|at| self.updates[at].1)
    }
}

#[derive(Debug)]
pub struct FrameRestore {
    spent: Vec<(u32, u8)>,
}

impl Schedule {
    #[cfg(any(test, feature = "test-support"))]
    pub fn presentation_state(&self) -> impl PartialEq + std::fmt::Debug {
        let sorted = |values: &ProvinceSet| values.iter().collect::<BTreeSet<_>>();
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn storage_slots(&self) -> usize {
        self.clean.slot_capacity()
            + self.in_view.slot_capacity()
            + self.off_screen.slot_capacity()
            + self.admitted.slot_capacity()
            + self.candidates.capacity()
    }

    pub fn set_selected(&mut self, selected: BTreeSet<u32>) {
        let previous = std::mem::replace(&mut self.selected, selected);
        if self.pending {
            for &id in self.selected.difference(&previous) {
                self.forced.insert(id, 2);
                if self.listed(id) {
                    self.off_screen.remove(id);
                    self.in_view.insert(id);
                    self.clean.remove(id);
                }
            }
        }
    }
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

    pub fn advance(
        &mut self,
        provinces: impl IntoIterator<Item = u32>,
        in_view: impl Fn(u32) -> bool,
        steady: bool,
    ) {
        self.advance_admitted(provinces.into_iter().map(|id| (id, in_view(id))), steady);
    }

    pub fn advance_frame_boxes(
        &mut self,
        provinces: &[(u32, Option<[f32; 4]>)],
        camera: Camera,
        viewport: [u32; 2],
        steady: bool,
        boxes: u64,
    ) {
        if self.pending {
            return;
        }
        let (against, slack) = self.admitted_against.at(camera, viewport);
        if self.candidates_for != Some((against, viewport, boxes, slack)) {
            let admission = Admission::held(against, viewport, slack);
            self.candidates.clear();
            for &(id, bounds) in provinces {
                if let Some(bounds) = bounds.filter(|&bounds| admission.admits(bounds)) {
                    self.candidates.push((id, bounds));
                }
            }
            self.candidates_for = Some((against, viewport, boxes, slack));
            self.admitted_for = None;
        }
        if self.admitted_for != Some((camera, viewport, boxes)) {
            let admission = Admission::new(camera, viewport);
            self.admitted.clear();
            for &(id, bounds) in &self.candidates {
                if admission.admits(bounds) {
                    self.admitted.insert(id);
                }
            }
            self.admitted_for = Some((camera, viewport, boxes));
        }
        let admitted = std::mem::take(&mut self.admitted);
        self.advance_admitted(provinces.iter().map(|&(id, _)| (id, admitted.contains(id))), steady);
        self.admitted = admitted;
    }

    fn advance_admitted(&mut self, provinces: impl IntoIterator<Item = (u32, bool)>, steady: bool) {
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
                self.clean.remove(id);
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

    pub fn update(&self, province: u32) -> Option<Update> {
        if self.in_view.contains(province) {
            Some(Update::InView)
        } else if self.off_screen.contains(province) {
            Some(Update::OffScreen)
        } else {
            None
        }
    }

    pub fn updates(&self) -> impl Iterator<Item = (u32, Update)> + '_ {
        self.in_view
            .iter()
            .map(|id| (id, Update::InView))
            .chain(self.off_screen.iter().map(|id| (id, Update::OffScreen)))
    }

    pub fn listed(&self, province: u32) -> bool {
        self.update(province).is_some() || self.dirty(province)
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    pub fn take_pending(&mut self) -> bool {
        let pending = std::mem::take(&mut self.pending);
        if pending {
            self.forced.retain(|&id, remaining| {
                if self.in_view.contains(id) || self.off_screen.contains(id) {
                    *remaining -= 1;
                    *remaining != 0
                } else {
                    true
                }
            });
        }
        pending
    }

    pub fn take_frame(&mut self) -> Option<(ProvinceFrame, FrameRestore)> {
        if !self.pending {
            return None;
        }
        let spent = self
            .forced
            .iter()
            .filter_map(|(&id, &count)| {
                (self.in_view.contains(id) || self.off_screen.contains(id)).then_some((id, count))
            })
            .collect();
        self.take_pending();
        let mut updates: Vec<_> = self.updates().collect();
        updates.sort_unstable_by_key(|&(id, _)| id);
        Some((ProvinceFrame { updates }, FrameRestore { spent }))
    }

    pub fn untake_frame(&mut self, restore: FrameRestore) {
        self.pending = true;
        for (id, count) in restore.spent {
            self.forced.insert(id, count);
        }
    }

    pub fn cancel_pending(&mut self) {
        if !std::mem::take(&mut self.pending) {
            return;
        }
        self.clean.remove_all(&self.off_screen);
        self.in_view.clear();
        self.off_screen.clear();
        self.presented = None;
    }

    pub fn dirty(&self, province: u32) -> bool {
        !self.clean.contains(province)
    }

    pub fn cleans(&self, province: u32) -> bool {
        self.dirty(province) && self.update(province) != Some(Update::InView)
    }

    pub fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        match provinces {
            None => self.clean.clear(),
            Some(provinces) if !self.clean.is_empty() => {
                for &id in provinces {
                    self.clean.remove(id);
                }
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Pose;

    #[test]
    fn taken_frames_match_the_walk_and_restore_exactly() {
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
        let mut idle = Schedule::default();
        let idle_state = state(&idle);
        assert!(idle.take_frame().is_none());
        assert_eq!(state(&idle), idle_state);
    }

    #[test]
    fn retained_boxes_match_tree_admission_across_selection_reset_and_pending_frames_and_repeated_cameras() {
        use crate::camera::{MapDefines, MapSize, Placement};
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
            retained.advance_frame_boxes(&boxes, camera, [1024, 768], steady, 0);
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
            let bound = 18 * (boxes.len() + 1) + 36;
            assert!(retained.storage_slots() <= bound);
            assert!(tree.storage_slots() <= bound);
        }
    }

    #[test]
    fn a_resize_or_rebuilt_boxes_test_the_admission_again() {
        use crate::camera::{MapDefines, MapSize, Placement};
        let camera = Placement { center: [3000., 1000.], height: 50., pose: Pose::Map }.on(MapSize::TEST, MapDefines::test());
        let bounds = super::super::province_admission::edge_boxes();
        let wide: Vec<_> = bounds.iter().map(|(&id, &bounds)| (id, Some(bounds))).collect();
        let boxless: Vec<_> = wide.iter().map(|&(id, _)| (id, None)).collect();
        let admitted = |schedule: &Schedule| wide.iter().filter(|&&(id, _)| schedule.update(id) == Some(Update::InView)).count();
        let mut schedule = Schedule::default();
        schedule.advance_frame_boxes(&wide, camera, [1024, 768], false, 0);
        let (seen, narrow) = (admitted(&schedule), {
            schedule.take_pending();
            schedule.advance_frame_boxes(&wide, camera, [64, 64], false, 0);
            admitted(&schedule)
        });
        assert!(seen > 0 && narrow < seen, "a narrower viewport admits fewer of the {seen} provinces, not the same ones ({narrow})");
        schedule.take_pending();
        schedule.advance_frame_boxes(&boxless, camera, [64, 64], false, 1);
        assert_eq!(admitted(&schedule), 0, "boxes rebuilt without any box admit nothing");
        schedule.take_pending();
        schedule.advance_frame_boxes(&wide, camera, [1024, 768], false, 0);
        assert_eq!(admitted(&schedule), seen, "the first camera, viewport and boxes again admit what they did");
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

    #[test]
    fn a_province_stays_dirty_until_its_phase_updates_it_out_of_view() {
        let mut schedule = Schedule::default();
        let provinces = [3, 103, 7];
        assert!(provinces.iter().all(|&id| schedule.dirty(id)));
        schedule.advance(provinces, |id| id == 7, false);
        assert_eq!(schedule.update(7), Some(Update::InView));
        assert_eq!(schedule.update(3), None);
        schedule.advance(provinces, |id| id == 7, false);
        assert!(schedule.dirty(3) && schedule.dirty(103));
        schedule.advance(provinces, |_| false, false);
        assert!(!schedule.dirty(3) && !schedule.dirty(103));
        assert_eq!(schedule.update(3), Some(Update::OffScreen));
        assert_eq!(schedule.update(103), Some(Update::OffScreen));
        for _ in 4..7 {
            schedule.advance(provinces, |_| false, false);
            assert!(schedule.dirty(7));
            assert_eq!(schedule.update(7), None);
        }
        schedule.advance(provinces, |_| false, false);
        assert!(!schedule.dirty(7));
        assert_eq!(schedule.update(7), Some(Update::OffScreen));
        for _ in 0..96 {
            schedule.advance(provinces, |_| false, false);
        }
        assert_eq!(schedule.update(3), None);
        schedule.advance(provinces, |id| id == 3, false);
        assert!(schedule.dirty(3));
        schedule.reset(Some(&BTreeSet::from([103])));
        assert!(schedule.dirty(103) && !schedule.dirty(7));
        schedule.reset(None);
        assert!(schedule.dirty(7));
        schedule.advance(provinces, |_| false, true);
        assert!(provinces.iter().all(|&id| !schedule.dirty(id)));
        assert!(schedule.pending() && schedule.take_pending());
        assert!(!schedule.pending() && !schedule.take_pending());
    }

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

        let tiny_world = [1_u32, 64];
        assert!(
            super::super::gpu_contract::validate_size(tiny_world.map(|side| side / 2), 8192)
                .is_err()
        );
        assert!(schedule.pending());
        assert_eq!(schedule.forced.get(&88), Some(&1));
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
            schedule.advance([2, 88, 99], |id| id == 99, false);
            assert_eq!(schedule.update(88), Some(Update::InView));
            assert_eq!(schedule.update(99), Some(Update::InView));
            assert_eq!(schedule.forced.get(&88), Some(&1));
        }
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

    fn assert_province_set_invariants(set: &ProvinceSet) {
        if set.occupied.is_empty() {
            assert!(set.slots.iter().all(|slot| slot.key == EMPTY));
            return;
        }
        assert!(set.slots.len().is_power_of_two() && set.slots.len() >= 8);
        assert!(set.occupied.len() * 2 <= set.slots.len());
        let non_empty = set
            .slots
            .iter()
            .filter(|slot| slot.key != EMPTY)
            .count();
        assert_eq!(non_empty, set.occupied.len());
        for (pos, &slot) in set.occupied.iter().enumerate() {
            let entry = set.slots[slot as usize];
            assert_ne!(entry.key, EMPTY);
            assert_ne!(entry.bits, 0);
            assert_eq!(entry.pos as usize, pos);
            assert_eq!(set.find_slot(entry.key), Some(slot as usize));
        }
    }

    #[test]
    fn province_set_matches_btree_across_extremes_collisions_and_clears() {
        let mut state = 0x9e37_79b9u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let universe: Vec<u32> = [
            0,
            1,
            63,
            64,
            65,
            127,
            128,
            4_000_000_011,
            u32::MAX - 65,
            u32::MAX - 64,
            u32::MAX - 1,
            u32::MAX,
        ]
        .into_iter()
        .chain((0..96u32).map(|index| index.wrapping_mul(100_019) + 7))
        .chain((0..64u32).map(|index| index << 10))
        .collect();
        let mut set = ProvinceSet::default();
        let mut oracle = BTreeSet::new();
        for step in 0..1_500u32 {
            let roll = next();
            let id = universe[roll as usize % universe.len()];
            match (roll >> 8) % 8 {
                0..=3 => {
                    assert_eq!(set.insert(id), oracle.insert(id), "step {step} id {id}");
                }
                4..=5 => {
                    set.remove(id);
                    oracle.remove(&id);
                }
                6 => {
                    let mut removed = ProvinceSet::default();
                    for _ in 0..(roll >> 16) % 12 {
                        let victim = universe[next() as usize % universe.len()];
                        removed.insert(victim);
                        oracle.remove(&victim);
                    }
                    assert_province_set_invariants(&removed);
                    set.remove_all(&removed);
                }
                _ => {
                    set.clear();
                    oracle.clear();
                }
            }
            assert_province_set_invariants(&set);
            assert_eq!(set.is_empty(), oracle.is_empty());
            let mut reverse = ProvinceSet::default();
            for &member in oracle.iter().rev() {
                reverse.insert(member);
            }
            assert_eq!(set, reverse);
            assert_eq!(set.iter().collect::<BTreeSet<_>>(), oracle);
            for &probe in &[0, 63, 64, u32::MAX - 1, u32::MAX, id] {
                assert_eq!(set.contains(probe), oracle.contains(&probe));
            }
            assert!(
                set.slot_capacity() <= (4 * universe.len()).max(8),
                "step {step}: slot capacity {} exceeded sparse bound",
                set.slot_capacity()
            );
        }
    }

    #[test]
    fn sparse_high_ids_keep_schedule_storage_bounded_to_active_provinces() {
        let ids = [0, 1, 100_019, 4_000_000_011, u32::MAX - 1, u32::MAX];
        let mut schedule = Schedule::default();
        schedule.set_selected(BTreeSet::from([u32::MAX]));
        for step in 0..250u32 {
            let steady = step % 50 == 49;
            schedule.advance(ids, |id| (id ^ step).is_multiple_of(3), steady);
            if step % 17 == 0 {
                let (frame, restore) = schedule.take_frame().expect("pending advance");
                assert!(
                    frame.updates().iter().all(|&(id, _)| ids.contains(&id)),
                    "step {step}"
                );
                schedule.untake_frame(restore);
            }
            if step % 23 == 0 {
                schedule.cancel_pending();
            } else {
                assert!(schedule.take_pending());
            }
            if step % 31 == 0 {
                schedule.reset(Some(&BTreeSet::from([u32::MAX, 0])));
            }
            if step % 71 == 0 {
                schedule.reset(None);
            }
            let bound = 18 * ids.len() + 36;
            assert!(
                schedule.storage_slots() <= bound,
                "step {step}: schedule storage {} exceeded bound {bound}",
                schedule.storage_slots()
            );
        }
        assert!(schedule.storage_slots() <= 18 * ids.len() + 36);
    }
}
