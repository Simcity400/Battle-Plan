use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use crate::cfront_section;
use engine_native::corder_instance;
use engine_native_parts::creference_registry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_23b0);
pub const REFERENCE_TYPE: u32 = 66;
pub const HANDLE: usize = 8;
pub const SECTIONS: usize = 0x58;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub front: u64,
    pub section_start: usize,
    pub section_len: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct Walk {
    pub outcome: Outcome,
    pub sections: usize,
    pub pairs: usize,
    pub required_sections: usize,
    pub required_pairs: usize,
    pub required_handles: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub result: Result<usize, &'static str>,
    pub plans: usize,
}

impl Bound {
    pub const fn refused(reason: &'static str) -> Self {
        Self {
            result: Err(reason),
            plans: 0,
        }
    }
}

pub fn bound(plans: &[corder_instance::PlanRow], source: Outcome, handles: &mut Vec<u64>) -> Bound {
    let Outcome::Rows { len } = source else {
        return Bound::refused("fronts require a complete army order plan census");
    };
    if len > plans.len() {
        return Bound::refused("front handle scratch capacity exhausted");
    }
    if let Err(reason) = grow_payload(handles, len, "front handle allocation failed") {
        return Bound {
            result: Err(reason),
            plans: len,
        };
    }
    let mut count = 0;
    for plan in &plans[..len] {
        if plan.front_valid != 0 {
            handles[count] = plan.front;
            count += 1;
        }
    }
    handles[..count].sort_unstable();
    Bound {
        result: Ok(count),
        plans: len,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn walk(
    memory: &impl Memory,
    base: usize,
    plans: &[corder_instance::PlanRow],
    source: Outcome,
    handles: &mut Vec<u64>,
    output: &mut Vec<Row>,
    sections: &mut Vec<cfront_section::Row>,
    pairs: &mut Vec<u32>,
) -> Walk {
    let bound = bound(plans, source, handles);
    walk_bound(memory, base, bound, handles, output, sections, pairs)
}

pub fn walk_bound(
    memory: &impl Memory,
    base: usize,
    bound: Bound,
    handles: &[u64],
    output: &mut Vec<Row>,
    sections: &mut Vec<cfront_section::Row>,
    pairs: &mut Vec<u32>,
) -> Walk {
    let mut report = Walk {
        outcome: Outcome::Rows { len: 0 },
        sections: 0,
        pairs: 0,
        required_sections: 0,
        required_pairs: 0,
        required_handles: bound.plans,
    };
    let result = (|| {
        let count = bound.result?;
        let handles = handles
            .get(..count)
            .ok_or("front handle scratch capacity exhausted")?;
        grow_payload(output, bound.plans, "front row allocation failed")?;
        let mut stored = 0;
        let mut previous = None;
        for &handle in handles {
            if previous == Some(handle) {
                continue;
            }
            previous = Some(handle);
            if handle as u32 != REFERENCE_TYPE {
                return Err("bound front reference type mismatch");
            }
            let Some(pointer) = creference_registry::resolve(memory, base, handle)? else {
                continue;
            };
            let expected = engine_sites::at(base, VTABLE_RVA).ok_or("front identity mismatch")?;
            if read::<u64>(memory, pointer) != Some(expected as u64)
                || corder_instance::plan_field::<u64>(memory, pointer, HANDLE)? != handle
            {
                return Err("front identity mismatch");
            }
            let at = pointer
                .checked_add(SECTIONS)
                .ok_or("front address overflow")?;
            let roster = Array::read(memory, at, 8).ok_or("front section roster invalid")?;
            let first_section = report.sections;
            report.required_sections = report
                .required_sections
                .checked_add(roster.count as usize)
                .ok_or("front section length overflow")?;
            for index in 0..roster.count {
                let cell = roster
                    .element(index, 8)
                    .ok_or("front section address overflow")?;
                let section = read::<u64>(memory, cell)
                    .filter(|v| *v != 0)
                    .ok_or("front section pointer unreadable")?
                    as usize;
                let (id, array) = cfront_section::header(memory, base, section, pointer)?;
                let words = array.count as usize * 2;
                report.required_pairs = report
                    .required_pairs
                    .checked_add(words)
                    .ok_or("front pair length overflow")?;
                cfront_section::copy_pairs(memory, base, array, pairs, report.pairs)?;
                grow_payload(
                    sections,
                    report.sections + 1,
                    "front section allocation failed",
                )?;
                sections[report.sections] = cfront_section::Row {
                    id,
                    pair_start: report.pairs,
                    pair_len: words,
                };
                report.sections += 1;
                report.pairs += words;
            }
            output[stored] = Row {
                front: handle,
                section_start: first_section,
                section_len: roster.count as usize,
            };
            stored += 1;
        }
        Ok(Outcome::Rows { len: stored })
    })();
    report.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    if matches!(report.outcome, Outcome::Unavailable { .. }) {
        report.required_sections = report.sections;
        report.required_pairs = report.pairs;
    }
    report
}

fn grow_payload<T: Default>(
    output: &mut Vec<T>,
    required: usize,
    reason: &'static str,
) -> Result<(), &'static str> {
    if required > output.len() {
        crate::allocation::reserve(output, required - output.len()).map_err(|_| reason)?;
        output.resize_with(required, T::default);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cprovince;
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Image(BTreeMap<usize, u8>, std::cell::RefCell<Vec<(usize, usize)>>);
    impl Image {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, b)| (at + i, *b)));
        }
        fn u64(&mut self, at: usize, v: u64) {
            self.put(at, &v.to_le_bytes());
        }
        fn u32(&mut self, at: usize, v: u32) {
            self.put(at, &v.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }
    }
    impl Memory for Image {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            self.1.borrow_mut().push((at, out.len()));
            for (i, b) in out.iter_mut().enumerate() {
                let Some(v) = self.0.get(&(at + i)) else {
                    return false;
                };
                *b = *v;
            }
            true
        }
    }
    const FRONT: usize = 0x3000;
    const SECTION: usize = 0x4000;
    const HANDLE: u64 = (7 << 32) | 66;
    fn image() -> Image {
        let mut m = Image::default();
        m.u64(registry::LOW_ROOTS_RVA.0 + 66 * 8, 0x1000);
        m.put(0x1000, &[0; 32]);
        m.u64(0x1008, 0x2000);
        m.u32(0x1010, 1);
        m.u32(0x1014, 7);
        m.put(0x1018, &[5]);
        m.put(0x2000, &[0; 14 * bucket::STRIDE]);
        m.put(0x2000 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        let hash = HANDLE
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let at = 0x2000 + slot * bucket::STRIDE;
        m.put(at + bucket::DISTANCE, &[1]);
        m.u64(at + bucket::TYPE, HANDLE);
        m.u64(at + bucket::OBJECT, FRONT as u64);
        m.put(FRONT, &[0; 0x88]);
        m.u64(FRONT, VTABLE_RVA.0 as u64);
        m.u64(FRONT + 8, HANDLE);
        m.array(FRONT + SECTIONS, 0x3800, 2);
        m.u64(0x3800, SECTION as u64);
        m.u64(0x3808, (SECTION + 0x100) as u64);
        for (at, id, count) in [(SECTION, 9, 2), (SECTION + 0x100, 3, 0)] {
            m.put(at, &[0; 0x60]);
            m.u64(at, cfront_section::VTABLE_RVA.0 as u64);
            m.u32(at + 8, id);
            m.u64(at + 0x10, FRONT as u64);
            m.array(at + cfront_section::PAIRS, 0x5000, count);
        }
        for (at, id) in [(0x6000, 4), (0x7000, 8)] {
            m.put(at, &[0; 0xb0]);
            m.u64(at, cprovince::VTABLE_RVA.0 as u64);
            m.u32(at + cprovince::ID, id);
        }
        for (at, pointer) in [
            (0x5000, 0x6000),
            (0x5008, 0x7000),
            (0x5010, 0x7000),
            (0x5018, 0x6000),
        ] {
            m.u64(at, pointer);
        }
        m
    }
    #[test]
    fn native_section_and_pair_counts_do_not_allocate_unreadable_suffixes() {
        for pair_count in [false, true] {
            let mut memory = image();
            if pair_count {
                memory.array(SECTION + cfront_section::PAIRS, 0x5000, 32_768);
            } else {
                memory.array(FRONT + SECTIONS, 0x9000_0000, 32_768);
            }
            let plans = [corder_instance::PlanRow {
                front: HANDLE,
                front_valid: 1,
                ..Default::default()
            }];
            let mut sections = Vec::new();
            let mut pairs = Vec::new();
            let result = walk(
                &memory,
                0,
                &plans,
                Outcome::Rows { len: 1 },
                &mut Vec::new(),
                &mut Vec::new(),
                &mut sections,
                &mut pairs,
            );
            assert_eq!(
                result.outcome,
                Outcome::Unavailable {
                    reason: if pair_count {
                        "front pair unreadable"
                    } else {
                        "front section pointer unreadable"
                    }
                }
            );
            assert_eq!(sections.capacity(), 0);
            assert_eq!(pairs.len(), if pair_count { 4 } else { 0 });
            assert!(pairs.capacity() <= 4);
            assert_eq!(result.required_sections, 0);
            assert!(result.required_pairs <= pairs.len());
        }
    }

    #[test]
    fn bound_fronts_deduplicate_and_keep_sections_empty_arrays_and_pair_direction() {
        let m = image();
        let plans = [corder_instance::PlanRow {
            front: HANDLE,
            front_valid: 1,
            ..Default::default()
        }; 2];
        let mut rows = vec![Row::default(); 2];
        let mut sections = vec![cfront_section::Row::default(); 4];
        let mut pairs = vec![0; 8];
        let r = walk(
            &m,
            0,
            &plans,
            Outcome::Rows { len: 2 },
            &mut vec![0; 2],
            &mut rows,
            &mut sections,
            &mut pairs,
        );
        assert_eq!(r.outcome, Outcome::Rows { len: 1 });
        assert_eq!((r.sections, r.pairs), (2, 4));
        assert_eq!(&pairs[..4], &[4, 8, 8, 4]);
        assert_eq!(
            (sections[0].id, sections[1].id, sections[1].pair_len),
            (9, 3, 0)
        );
        for span in [
            (FRONT + SECTIONS, 16),
            (0x3800, 8),
            (0x3808, 8),
            (SECTION + cfront_section::PAIRS, 16),
            (SECTION + 0x100 + cfront_section::PAIRS, 16),
        ] {
            assert_eq!(m.1.borrow().iter().filter(|p| **p == span).count(), 1);
        }
        let reads = m.1.borrow().clone();
        m.1.borrow_mut().clear();
        let mut grown_rows = Vec::new();
        let mut grown_sections = Vec::new();
        let mut grown_pairs = Vec::new();
        let r = walk(
            &m,
            0,
            &plans,
            Outcome::Rows { len: 2 },
            &mut Vec::new(),
            &mut grown_rows,
            &mut grown_sections,
            &mut grown_pairs,
        );
        assert_eq!(r.outcome, Outcome::Rows { len: 1 });
        assert_eq!((r.required_sections, r.required_pairs), (2, 4));
        assert_eq!((r.sections, r.pairs), (2, 4));
        assert_eq!(grown_rows, rows);
        assert_eq!(grown_sections, sections[..2]);
        assert_eq!(grown_pairs, pairs[..4]);
        assert_eq!(*m.1.borrow(), reads);
        let r = walk(
            &m,
            0,
            &plans,
            Outcome::Rows { len: 2 },
            &mut Vec::new(),
            &mut rows,
            &mut sections,
            &mut pairs,
        );
        assert_eq!(r.outcome, Outcome::Rows { len: 1 });
        assert_eq!(r.required_handles, 2);
    }
    #[test]
    fn binding_reads_nothing_and_the_bound_walk_is_the_single_walk() {
        let m = image();
        let plans = [corder_instance::PlanRow {
            front: HANDLE,
            front_valid: 1,
            ..Default::default()
        }; 2];
        for (source, len) in [
            (Outcome::Rows { len: 2 }, 2),
            (Outcome::Rows { len: 3 }, 2),
            (Outcome::Unavailable { reason: "census" }, 2),
            (Outcome::Rows { len: 2 }, 0),
        ] {
            let single = {
                let (mut rows, mut sections, mut pairs) = (Vec::new(), Vec::new(), Vec::new());
                let walk = walk(
                    &m,
                    0,
                    &plans[..len],
                    source,
                    &mut Vec::new(),
                    &mut rows,
                    &mut sections,
                    &mut pairs,
                );
                (format!("{walk:?}"), rows, sections, pairs)
            };
            m.1.borrow_mut().clear();
            let mut handles = Vec::new();
            let bound = bound(&plans[..len], source, &mut handles);
            assert!(m.1.borrow().is_empty(), "binding read memory");
            let (mut rows, mut sections, mut pairs) = (Vec::new(), Vec::new(), Vec::new());
            let walk = walk_bound(&m, 0, bound, &handles, &mut rows, &mut sections, &mut pairs);
            assert_eq!(
                (format!("{walk:?}"), rows, sections, pairs),
                single,
                "{source:?}"
            );
        }
    }
    #[test]
    fn stale_binding_to_a_destroyed_front_has_no_row_without_hiding_live_fronts() {
        let m = image();
        let stale = (8_u64 << 32) | 66;
        let plans = [HANDLE, stale].map(|front| corder_instance::PlanRow {
            front,
            front_valid: 1,
            ..Default::default()
        });
        let mut rows = vec![Row::default(); 2];
        let r = walk(
            &m,
            0,
            &plans,
            Outcome::Rows { len: 2 },
            &mut vec![0; 2],
            &mut rows,
            &mut vec![cfront_section::Row::default(); 4],
            &mut vec![0; 8],
        );
        assert_eq!(r.outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].front, HANDLE);
        let mut broken = image();
        broken.u64(0x1008, 0x9000);
        assert!(matches!(
            walk(
                &broken,
                0,
                &plans,
                Outcome::Rows { len: 2 },
                &mut vec![0; 2],
                &mut rows,
                &mut vec![cfront_section::Row::default(); 4],
                &mut vec![0; 8],
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }
    #[test]
    fn invalid_front_section_and_province_identity_never_become_empty_geometry() {
        let plans = [corder_instance::PlanRow {
            front: HANDLE,
            front_valid: 1,
            ..Default::default()
        }];
        for at in [FRONT, FRONT + 8, SECTION, SECTION + 0x10, 0x6000] {
            let mut m = image();
            m.u64(at, 0);
            assert!(matches!(
                walk(
                    &m,
                    0,
                    &plans,
                    Outcome::Rows { len: 1 },
                    &mut Vec::new(),
                    &mut Vec::new(),
                    &mut Vec::new(),
                    &mut Vec::new()
                )
                .outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }
}
