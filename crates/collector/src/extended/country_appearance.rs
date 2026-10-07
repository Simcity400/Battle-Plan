//! Current country colour bits, cosmetic tag, name operands, the retained old
//! names and the ruling party's first leader's character, sampled hourly.
//! The reserved-country naming singleton has independent availability and
//! live-sized operand bytes; it never enters the positive country census.
use crate::{
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{Memory, ccountry, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{CountryAppearanceRow, ReservedCountryNamingRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CountryAppearance, TableId::ReservedCountryNaming];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static ORIGINAL_TAG_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static POSITIVE_ORIGINAL_TAG_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static ORIGINAL_TAG_BUCKET_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static COUNTRY_ARRAY_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static RESERVED_OWNED_STATE_COUNT_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(crate) fn original_tag_data() -> impl serde::Serialize {
    let refused = ORIGINAL_TAG_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let positive = POSITIVE_ORIGINAL_TAG_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let buckets = ORIGINAL_TAG_BUCKET_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let array = COUNTRY_ARRAY_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let owned_count =
        RESERVED_OWNED_STATE_COUNT_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    crate::status::status!({
        "refused": refused.saturating_add(positive).saturating_add(buckets).saturating_add(array).saturating_add(owned_count),
        "positive_refused": positive,
        "reserved_refused": refused,
        "bucket_refused": buckets,
        "country_array_refused": array,
        "reserved_owned_state_count_refused": owned_count,
        "refusals": crate::status::map([
            ("positive country original tag guarded read or arena refusal", positive),
            ("reserved country original tag guarded read or arena refusal", refused),
            ("native original-tag buckets guarded read or arena refusal", buckets),
            ("physical country array guarded read or arena refusal", array),
            ("reserved owned state count guarded read refusal", owned_count),
        ].into_iter().filter(|(_, count)| *count != 0)),
        "scope": "cumulative independent original-tag byte, bucket, physical-array and reserved owned-state count refusals; signed references and naming operands remain available",
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    subjects: usize,
    naming_bytes: usize,
    original_tag_buckets: usize,
    original_tag_members: usize,
    country_array: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 512,
            subjects: 1024,
            naming_bytes: 0,
            original_tag_buckets: 0,
            original_tag_members: 0,
            country_array: 0,
        }
    }
}
impl Capacities {
    /// One row per canonical country slot.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
        crate::country_scale::raise(&mut self.subjects, crate::country_scale::rows(slots, 1));
    }

    pub(crate) fn raise_reserved_naming(&mut self, bytes: usize) {
        crate::country_scale::raise(&mut self.naming_bytes, bytes);
    }

    pub(crate) fn raise_original_tag_lists(&mut self, demand: engine::original_tag_lists::Demand) {
        crate::country_scale::raise(&mut self.original_tag_buckets, demand.buckets);
        crate::country_scale::raise(&mut self.original_tag_members, demand.members);
    }

    pub(crate) fn raise_country_array(&mut self, count: usize) {
        crate::country_scale::raise(&mut self.country_array, count);
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::AppearanceRow>,
    subject_starts: Vec<usize>,
    subjects: Arena<u32>,
    reserved: ccountry::ReservedNamingWalk,
    naming: Arena<u8>,
    original_tag_buckets: Arena<engine::original_tag_lists::Bucket>,
    original_tag_members: Arena<u32>,
    original_tag_bucket_len: Result<usize, &'static str>,
    country_array: Arena<u32>,
    country_array_len: Result<usize, &'static str>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            subjects: 0,
            naming_bytes: 0,
            original_tag_buckets: 0,
            original_tag_members: 0,
            country_array: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            subject_starts: owned_vec(engine::allocation::default_vec(capacity.rows)),
            subjects: Arena::new(capacity.subjects),
            reserved: ccountry::ReservedNamingWalk {
                row: ccountry::ReservedNamingRow::default(),
                outcome: Outcome::Unavailable {
                    reason: "clock unavailable",
                },
                bytes_len: 0,
                required_bytes: 0,
            },
            naming: Arena::new(capacity.naming_bytes),
            original_tag_buckets: Arena::new(capacity.original_tag_buckets),
            original_tag_members: Arena::new(capacity.original_tag_members),
            original_tag_bucket_len: Err("clock unavailable"),
            country_array: Arena::new(capacity.country_array),
            country_array_len: Err("clock unavailable"),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.subjects.reset();
        self.reserved.outcome = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        self.naming.reset();
        self.original_tag_buckets.reset();
        self.original_tag_members.reset();
        self.original_tag_bucket_len = Err("clock unavailable");
        self.country_array.reset();
        self.country_array_len = Err("clock unavailable");
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.subjects.allocated_bytes()
            + self.subject_starts.capacity() * std::mem::size_of::<usize>()
            + self.naming.allocated_bytes()
            + self.original_tag_buckets.allocated_bytes()
            + self.original_tag_members.allocated_bytes()
            + self.country_array.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        crate::table_capture::grow_vec(&mut self.subject_starts, capacity.rows);
        self.subjects.grow(&mut capacity.subjects);
        self.naming.grow(&mut capacity.naming_bytes);
        self.original_tag_buckets
            .grow(&mut capacity.original_tag_buckets);
        self.original_tag_members
            .grow(&mut capacity.original_tag_members);
        self.country_array.grow(&mut capacity.country_array);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.subjects.reset();
        self.naming.reset();
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        match ccountry::dynamic_boundary(memory, base) {
            Some(boundary) => {
                self.rows.outcome =
                    ccountry::walk_appearance(memory, base, state, boundary, &mut self.rows.rows);
                if !matches!(self.rows.outcome, Outcome::Unavailable { .. }) {
                    self.capture_subjects(memory);
                }
            }
            None => {
                self.rows.outcome = Outcome::Unavailable {
                    reason: "country tag database unreadable",
                }
            }
        }
        let appearance_ticks = qpc().saturating_sub(begin).max(0) as u64;
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        self.reserved = ccountry::read_reserved_naming(memory, base, state, &mut self.naming.first);
        self.naming.len = self.reserved.bytes_len;
        self.naming.required = self.reserved.required_bytes;
        if matches!(self.reserved.outcome, Outcome::Rows { len: 1 })
            && self.reserved.row.original_tag.is_none()
        {
            ORIGINAL_TAG_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if matches!(self.reserved.outcome, Outcome::Rows { len: 1 })
            && self.reserved.row.owned_state_count.is_none()
        {
            RESERVED_OWNED_STATE_COUNT_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let reserved_ticks = qpc().saturating_sub(begin).max(0) as u64;
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        ccountry::capture_appearance_original_tags(
            memory,
            state,
            &mut self.rows.rows[..self.rows.outcome.len()],
            &mut self.naming.first,
            &mut self.naming.len,
            &mut self.naming.required,
        );
        let refused = self.rows.rows[..self.rows.outcome.len()]
            .iter()
            .filter(|row| row.original_tag.is_none())
            .count();
        POSITIVE_ORIGINAL_TAG_REFUSALS
            .fetch_add(refused as u64, std::sync::atomic::Ordering::Relaxed);
        let appearance_ticks =
            appearance_ticks.saturating_add(qpc().saturating_sub(begin).max(0) as u64);
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        let buckets = engine::original_tag_lists::capture(
            memory,
            state,
            &mut self.original_tag_buckets.first,
            &mut self.original_tag_members.first,
        );
        self.original_tag_buckets.required = buckets.demand.buckets;
        self.original_tag_members.required = buckets.demand.members;
        self.original_tag_bucket_len = buckets.len;
        self.original_tag_buckets.len = buckets.len.unwrap_or(0);
        self.original_tag_members.len = if buckets.len.is_ok() {
            buckets.demand.members
        } else {
            0
        };
        if buckets.len.is_err() {
            ORIGINAL_TAG_BUCKET_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let array =
            engine::country_roster::capture(memory, base, state, &mut self.country_array.first);
        self.country_array.required = array.required;
        self.country_array_len = array.len;
        self.country_array.len = array.len.unwrap_or(0);
        if array.len.is_err() {
            COUNTRY_ARRAY_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        [
            appearance_ticks,
            reserved_ticks.saturating_add(qpc().saturating_sub(begin).max(0) as u64),
        ]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.reserved.outcome]
    }

    fn capture_subjects(&mut self, memory: &impl Memory) {
        let result = (|| {
            let initial = self.rows.outcome;
            for row in &self.rows.rows[..initial.len()] {
                self.subjects.required = self
                    .subjects
                    .required
                    .checked_add(row.subjects.count as usize)
                    .ok_or("country subject arena demand overflow")?;
            }
            let mut len = 0;
            let mut unread_rows = match initial {
                Outcome::Truncated { unread_rows, .. } => unread_rows,
                _ => 0,
            };
            for index in 0..initial.len() {
                let subjects = self.rows.rows[index].subjects;
                let end = self
                    .subjects
                    .len
                    .checked_add(subjects.count as usize)
                    .ok_or("country subject arena demand overflow")?;
                // Native faction previews name the stored roster in its order
                // (1404AC450). Read no payload for countries without subjects.
                let copied = self
                    .subjects
                    .first
                    .get_mut(self.subjects.len..end)
                    .is_some_and(|destination| {
                        engine::cpdx_array::copy_u32s(
                            memory,
                            subjects,
                            destination,
                            "country subject roster payload unreadable",
                        )
                        .is_ok()
                    });
                if !copied {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("country appearance unread row count overflow")?;
                    continue;
                }
                if len != index {
                    self.rows.rows[len] = self.rows.rows[index];
                }
                self.subject_starts[len] = self.subjects.len;
                len += 1;
                self.subjects.len = end;
            }
            self.rows.outcome = if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            };
            Ok(())
        })();
        if let Err(reason) = result {
            self.rows.fail(Outcome::Unavailable { reason });
        }
    }

    pub fn tables(&self) -> [TableView; N] {
        [self.appearance_table(), self.reserved_table()]
    }

    fn appearance_table(&self) -> TableView {
        let Some(subjects) = self.subjects.first.get(..self.subjects.len) else {
            return unavailable("country subject arena invalid");
        };
        if self.rows.rows[..self.rows.outcome.len()]
            .iter()
            .zip(&self.subject_starts)
            .any(|(row, &start)| {
                start
                    .checked_add(row.subjects.count as usize)
                    .and_then(|end| subjects.get(start..end))
                    .is_none()
            })
        {
            return unavailable("country subject span outside captured arena");
        }
        crate::spooling::table_iter::<tables::CountryAppearance, _>(
            self.outcomes()[0],
            self.rows.rows[..self.rows.outcome.len()]
                .iter()
                .zip(&self.subject_starts),
            |(r, &start)| CountryAppearanceRow {
                country: r.country,
                subjects: replay::rows::Recorded::Value(owned_vec(
                    engine::allocation::collect_vec(
                        subjects[start..start + r.subjects.count as usize]
                            .iter()
                            .map(|&value| value as i32),
                    ),
                )),
                red: r.rgba[0],
                green: r.rgba[1],
                blue: r.rgba[2],
                alpha: r.rgba[3],
                cosmetic_tag: owned_vec(copy_vec(r.cosmetic_tag.as_bytes())),
                name_tag: owned_vec(copy_vec(r.name_tag.as_bytes())),
                leader_ideology: owned_vec(copy_vec(r.leader_ideology.as_bytes())),
                overlord: r.overlord,
                graphical_culture: owned_vec(copy_vec(r.graphical_culture.as_bytes())),
                graphical_culture_2d: owned_vec(copy_vec(r.graphical_culture_2d.as_bytes())),
                leader_character: r.leader_character,
                ui_red: r.ui_rgba[0],
                ui_green: r.ui_rgba[1],
                ui_blue: r.ui_rgba[2],
                ui_alpha: r.ui_rgba[3],
                old_name: r
                    .old_names
                    .map(|names| owned_vec(copy_vec(names[0].as_bytes()))),
                old_adjective: r
                    .old_names
                    .map(|names| owned_vec(copy_vec(names[1].as_bytes()))),
                old_definite_name: r
                    .old_names
                    .map(|names| owned_vec(copy_vec(names[2].as_bytes()))),
                original_tag: replay::rows::Recorded::Value(
                    r.original_tag
                        .and_then(|name| {
                            name.start.checked_add(name.len).and_then(|end| {
                                self.naming
                                    .first
                                    .get(..self.naming.len)?
                                    .get(name.start..end)
                            })
                        })
                        .map(|bytes| owned_vec(copy_vec(bytes))),
                ),
                original_tag_reference: replay::rows::Recorded::Value(Some(
                    r.original_tag_reference,
                )),
            },
        )
    }

    fn reserved_table(&self) -> TableView {
        let r = &self.reserved.row;
        let Some(bytes) = self.naming.first.get(..self.naming.len) else {
            return unavailable("reserved naming arena invalid");
        };
        let span = |name: engine::csubject_relation::Name| {
            name.start
                .checked_add(name.len)
                .and_then(|end| bytes.get(name.start..end))
        };
        let Some(tag) = span(r.tag) else {
            return unavailable("reserved naming tag span invalid");
        };
        let Some(name_tag) = span(r.name_tag) else {
            return unavailable("reserved naming original tag span invalid");
        };
        let Some(cosmetic_tag) = span(r.cosmetic_tag) else {
            return unavailable("reserved naming cosmetic tag span invalid");
        };
        let Some(leader_ideology) = span(r.leader_ideology) else {
            return unavailable("reserved naming ideology span invalid");
        };
        let group = match r.group {
            Some(name) => match span(name) {
                Some(bytes) => Some(bytes),
                None => return unavailable("reserved naming group span invalid"),
            },
            None => None,
        };
        let autonomy = match r.autonomy {
            Some(name) => match span(name) {
                Some(bytes) => Some(bytes),
                None => return unavailable("reserved naming autonomy span invalid"),
            },
            None => None,
        };
        // Malformed optional spans refuse only their field, never the prefix.
        let original_tag = r.original_tag.and_then(span);
        crate::spooling::table::<tables::ReservedCountryNaming, _>(
            self.reserved.outcome,
            std::slice::from_ref(r),
            |_| ReservedCountryNamingRow {
                tag: owned_vec(copy_vec(tag)),
                name_tag: owned_vec(copy_vec(name_tag)),
                cosmetic_tag: owned_vec(copy_vec(cosmetic_tag)),
                leader_ideology: owned_vec(copy_vec(leader_ideology)),
                group: group.map(|bytes| owned_vec(copy_vec(bytes))),
                overlord: r.overlord,
                autonomy: autonomy.map(|bytes| owned_vec(copy_vec(bytes))),
                built: r.built,
                original_tag: replay::rows::Recorded::Value(
                    original_tag.map(|bytes| owned_vec(copy_vec(bytes))),
                ),
                original_tag_reference: replay::rows::Recorded::Value(Some(
                    r.original_tag_reference,
                )),
                original_tag_buckets: replay::rows::Recorded::Value(self.original_tag_lists()),
                country_array: replay::rows::Recorded::Value(
                    self.country_array_len
                        .ok()
                        .and_then(|len| {
                            self.country_array
                                .first
                                .get(..self.country_array.len)?
                                .get(..len)
                        })
                        .map(|values| owned_vec(copy_vec(values))),
                ),
                owned_state_count: replay::rows::Recorded::Value(r.owned_state_count),
            },
        )
    }

    fn original_tag_lists(&self) -> Option<Vec<Vec<u32>>> {
        let len = self.original_tag_bucket_len.ok()?;
        let buckets = self
            .original_tag_buckets
            .first
            .get(..self.original_tag_buckets.len)?
            .get(..len)?;
        let members = self
            .original_tag_members
            .first
            .get(..self.original_tag_members.len)?;
        if buckets.iter().any(|bucket| {
            bucket
                .start
                .checked_add(bucket.len)
                .and_then(|end| members.get(bucket.start..end))
                .is_none()
        }) {
            return None;
        }
        Some(owned_vec(engine::allocation::collect_vec(
            buckets.iter().map(|bucket| {
                owned_vec(copy_vec(&members[bucket.start..bucket.start + bucket.len]))
            }),
        )))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "country_appearance/bench/tests.rs"]
mod bench;
