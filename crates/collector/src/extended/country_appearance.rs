//! Current country colour bits, cosmetic tag, name operands, the retained old
//! names and the ruling party's first leader's character,
//! sampled hourly.
use crate::{
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{Memory, ccountry, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{CountryAppearanceRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CountryAppearance];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    subjects: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 512,
            subjects: 1024,
        }
    }
}
impl Capacities {
    /// One row per canonical country slot.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
        crate::country_scale::raise(&mut self.subjects, crate::country_scale::rows(slots, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::AppearanceRow>,
    subject_starts: Vec<usize>,
    subjects: Arena<u32>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            subjects: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            subject_starts: owned_vec(engine::allocation::default_vec(capacity.rows)),
            subjects: Arena::new(capacity.subjects),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.subjects.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.subjects.allocated_bytes()
            + self.subject_starts.capacity() * std::mem::size_of::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        crate::table_capture::grow_vec(&mut self.subject_starts, capacity.rows);
        self.subjects.grow(&mut capacity.subjects);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.subjects.reset();
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
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
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
        let Some(subjects) = self.subjects.first.get(..self.subjects.len) else {
            return [unavailable("country subject arena invalid")];
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
            return [unavailable("country subject span outside captured arena")];
        }
        [crate::spooling::table_iter::<tables::CountryAppearance, _>(
            self.outcomes()[0],
            self.rows.rows[..self.rows.outcome.len()]
                .iter()
                .zip(&self.subject_starts),
            |(r, &start)| CountryAppearanceRow {
                country: r.country,
                subjects: owned_vec(engine::allocation::collect_vec(
                    subjects[start..start + r.subjects.count as usize]
                        .iter()
                        .map(|&value| value as i32),
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
            },
        )]
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "country_appearance/bench/tests.rs"]
mod bench;
