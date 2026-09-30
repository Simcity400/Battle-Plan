//! Current country colour bits, cosmetic tag, name operands, the retained old
//! names and the ruling party's first leader's character and trait tokens,
//! sampled hourly.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
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
    traits: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 512,
            traits: 512,
        }
    }
}
impl Capacities {
    /// One row per canonical country slot.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
    }

    /// The trait arena from the live total of every ruling first leader's
    /// traits (`country_scale::observe_politics`), with the three-quarters
    /// growth rule's headroom. Never lowers.
    pub(crate) fn raise_for_leader_traits(&mut self, traits: usize) {
        crate::country_scale::raise(&mut self.traits, crate::country_scale::rows(traits, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::AppearanceRow>,
    traits: Arena<u32>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, traits: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            traits: Arena::new(capacity.traits),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.traits.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.traits.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.traits.grow(&mut capacity.traits);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        match ccountry::dynamic_boundary(memory, base) {
            Some(boundary) => {
                let walk = ccountry::walk_appearance(
                    memory,
                    base,
                    state,
                    boundary,
                    &mut self.rows.rows,
                    &mut self.traits.first,
                );
                self.rows.outcome = walk.outcome;
                self.traits.len = walk.traits_len;
                self.traits.required = walk.required_traits;
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

    pub fn tables(&self) -> [TableView; N] {
        let Some(traits) = self.traits.first.get(..self.traits.len) else {
            return [unavailable("leader trait arena length invalid")];
        };
        let span = |row: &ccountry::AppearanceRow| {
            row.leader_traits_start
                .checked_add(row.leader_traits_len)
                .and_then(|end| traits.get(row.leader_traits_start..end))
        };
        let rows = &self.rows.rows[..self.rows.outcome.len().min(self.rows.rows.len())];
        if rows.iter().any(|row| span(row).is_none()) {
            return [unavailable("leader traits outside captured arena")];
        }
        [table::<tables::CountryAppearance, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| CountryAppearanceRow {
                country: r.country,
                red: r.rgba[0],
                green: r.rgba[1],
                blue: r.rgba[2],
                alpha: r.rgba[3],
                cosmetic_tag: r.cosmetic_tag.as_bytes().to_vec(),
                name_tag: r.name_tag.as_bytes().to_vec(),
                leader_ideology: r.leader_ideology.as_bytes().to_vec(),
                overlord: r.overlord,
                graphical_culture: r.graphical_culture.as_bytes().to_vec(),
                leader_character: r.leader_character,
                ui_red: r.ui_rgba[0],
                ui_green: r.ui_rgba[1],
                ui_blue: r.ui_rgba[2],
                ui_alpha: r.ui_rgba[3],
                leader_traits: span(r).unwrap().to_vec(),
                old_name: r.old_names.map(|names| names[0].as_bytes().to_vec()),
                old_adjective: r.old_names.map(|names| names[1].as_bytes().to_vec()),
                old_definite_name: r.old_names.map(|names| names[2].as_bytes().to_vec()),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn unreadable_census_is_unavailable_and_raw_colors_preserve_primary_values() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1, traits: 4 });
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0].country = 17;
        capture.rows.rows[0].rgba = [0x7fc01234, 0x80000000, 0xffffffff, 0x3f800000];
        capture.rows.rows[0].ui_rgba = [0x3f000000, 0x00000001, 0x7f800000, 0xbf800000];
        capture.rows.rows[0].cosmetic_tag.bytes[..3].copy_from_slice(&[65, 0, 255]);
        capture.rows.rows[0].cosmetic_tag.len = 3;
        capture.rows.rows[0].leader_character = Some(0);
        capture.traits.first[..3].copy_from_slice(&[9, 0x2ff1, 7]);
        capture.traits.len = 3;
        capture.rows.rows[0].leader_traits_start = 1;
        capture.rows.rows[0].leader_traits_len = 2;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::CountryAppearance(rows))] =
            capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].red, 0x7fc01234);
        assert_eq!(rows[0].cosmetic_tag, [65, 0, 255]);
        assert_eq!(rows[0].green, 0x80000000);
        assert_eq!(rows[0].blue, 0xffffffff);
        assert_eq!(rows[0].alpha, 0x3f800000);
        assert_eq!(rows[0].leader_character, Some(0));
        assert_eq!(
            [rows[0].ui_red, rows[0].ui_green, rows[0].ui_blue, rows[0].ui_alpha],
            [0x3f000000, 0x00000001, 0x7f800000, 0xbf800000]
        );
        assert_eq!(rows[0].leader_traits, [0x2ff1, 7]);
        assert_eq!(rows[0].old_name, None);
        // A span past the captured traits withholds the table.
        capture.rows.rows[0].leader_traits_len = 3;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }

    #[test]
    fn trait_capacity_follows_the_live_trait_total() {
        let mut capacity = Capacities::default();
        capacity.raise_for_leader_traits(30_000);
        assert!(capacity.traits >= 40_000);
        let raised = capacity;
        capacity.raise_for_leader_traits(3);
        assert!(capacity == raised);
    }
}
