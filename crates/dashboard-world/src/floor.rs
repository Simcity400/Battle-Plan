//! What a family's operators did against what changed, added up over a run while accounting is
//! on ([`replay_frame::cpu::enabled`]): the rows each table it reads changed, and its own counts
//! of what it touched, reported by name.

use replay::segments::{Manifest, State};
use replay_data::rows::{TABLE_COUNT, TableId};
use std::sync::{
    LazyLock,
    atomic::{AtomicU64, Ordering},
};

/// Room for a family's own counts.
const NAMED: usize = 16;

/// A run's floor measurement of one family.
pub struct Floor {
    names: &'static [&'static str],
    hours: AtomicU64,
    rows: [AtomicU64; TABLE_COUNT],
    tables: [AtomicU64; TABLE_COUNT],
    named: [AtomicU64; NAMED],
}

static ROWS: LazyLock<Vec<&'static str>> = LazyLock::new(|| names("rows changed"));
static TABLES: LazyLock<Vec<&'static str>> = LazyLock::new(|| names("hours changed"));

fn names(what: &str) -> Vec<&'static str> {
    TableId::ALL
        .iter()
        .map(|table| &*Box::leak(format!("{what} {table:?}").into_boxed_str()))
        .collect()
}

impl Floor {
    /// A floor whose own counts are `names`, by index.
    pub const fn new(names: &'static [&'static str]) -> Self {
        assert!(names.len() <= NAMED);
        Self {
            names,
            hours: AtomicU64::new(0),
            rows: [const { AtomicU64::new(0) }; TABLE_COUNT],
            tables: [const { AtomicU64::new(0) }; TABLE_COUNT],
            named: [const { AtomicU64::new(0) }; NAMED],
        }
    }

    pub fn on() -> bool {
        replay_frame::cpu::enabled()
    }

    /// One operator hour: the rows the hour changed of each of `reads` its manifest sections.
    pub fn hour(&self, state: &State, manifest: &Manifest<'_>, reads: &[TableId]) {
        if !Self::on() {
            return;
        }
        self.hours.fetch_add(1, Ordering::Relaxed);
        for section in &manifest.sections {
            let index = section.table;
            if index >= TABLE_COUNT || state.skips(index) {
                continue;
            }
            if TableId::ALL
                .get(index)
                .is_none_or(|table| reads.binary_search(table).is_err())
            {
                continue;
            }
            let Some(changes) = state.changes(index) else {
                continue;
            };
            let written = changes.masks.iter().filter(|mask| **mask != 0).count()
                + changes.fill_masks.iter().filter(|mask| **mask != 0).count();
            let rows = written + changes.inserted.len() + changes.deleted.len();
            if rows > 0 {
                self.rows[index].fetch_add(rows as u64, Ordering::Relaxed);
                self.tables[index].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Adds `count` to own count `at`.
    pub fn add(&self, at: usize, count: u64) {
        if Self::on() {
            self.named[at].fetch_add(count, Ordering::Relaxed);
        }
    }

    pub fn report(&self) -> Vec<(&'static str, u64)> {
        let mut report = vec![("floor hours", self.hours.load(Ordering::Relaxed))];
        for index in 0..TABLE_COUNT {
            let rows = self.rows[index].load(Ordering::Relaxed);
            if rows > 0 {
                report.push((ROWS[index], rows));
                report.push((TABLES[index], self.tables[index].load(Ordering::Relaxed)));
            }
        }
        for (at, name) in self.names.iter().enumerate() {
            report.push((name, self.named[at].load(Ordering::Relaxed)));
        }
        report
    }
}
