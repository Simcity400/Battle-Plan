//! Province temperatures against the hourly pass ([`replay::temperature`]).
//! Each hour must reproduce its read checksum. An anchor whose preceding
//! frame proves its pass must match the derived words and cursor.

use replay::{
    frame::DecodedFrame,
    rows::{Recorded, TemperatureRecord, WeatherProvinceRow, tables},
    temperature::{Content, Memo, Simulation},
};

/// Mismatches listed by name; all of them are counted.
const LISTED: usize = 40;

#[derive(Default)]
pub(super) struct Stats {
    refused: std::collections::BTreeMap<String, u64>,
    listed: Vec<String>,
    /// Frames holding anchored words and the pass's inputs.
    anchored_hours: u64,
    /// Hours whose derivation reproduced the read checksum.
    checksum_equal: u64,
    checksum_mismatches: u64,
    /// Anchors whose own hour the hour before proves an ordinary pass, compared word for word.
    anchors_compared: u64,
    anchor_rows_equal: u64,
    anchor_rows_differing: u64,
    anchor_cursor_differing: u64,
    /// Anchors whose own hour no earlier frame proves.
    anchors_unproven: u64,
}

impl Stats {
    fn mismatches(&self) -> u64 {
        self.checksum_mismatches + self.anchor_rows_differing + self.anchor_cursor_differing
    }

    fn list(&mut self, line: String) {
        if self.listed.len() < LISTED {
            self.listed.push(line);
        }
    }
}

pub(super) struct Checker {
    content: Result<Content, String>,
    /// The derivation of anchored frames, and the hour before's derived words.
    memo: Memo,
    derived: Option<(u64, Simulation)>,
    pub(super) stats: Stats,
}

impl Checker {
    pub(super) fn new(content: Result<Content, String>) -> Self {
        Self {
            content,
            memo: Memo::default(),
            derived: None,
            stats: Stats::default(),
        }
    }

    pub(super) fn observe(&mut self, frame: &DecodedFrame) {
        match frame.complete::<tables::WeatherProvinces>() {
            Ok(rows)
                if matches!(
                    rows.first().map(|row| row.temperature.record()),
                    Some(Ok(TemperatureRecord::Anchored(_)))
                ) =>
            {
                self.observe_anchored(frame, rows)
            }
            rows => {
                let reason = rows.err().map_or_else(
                    || "temperature anchor not recorded".to_owned(),
                    |error| error.to_string(),
                );
                *self.stats.refused.entry(reason).or_default() += 1;
                self.derived = None;
            }
        }
    }

    pub(super) fn failures(&self) -> [u64; 2] {
        [self.stats.mismatches(), self.stats.refused.values().sum()]
    }

    fn observe_anchored(&mut self, frame: &DecodedFrame, rows: &[WeatherProvinceRow]) {
        self.stats.anchored_hours += 1;
        let content = self.content.as_ref().map_err(String::as_str);
        let previous = self
            .derived
            .take()
            .filter(|(branch, simulation)| {
                *branch == frame.branch && simulation.hour.checked_add(1) == Some(frame.hour)
            })
            .map(|(_, simulation)| simulation);
        if let Some(Recorded::Value(Some(pass))) = rows.first().map(|row| &row.temperature_pass)
            && pass.anchor == frame.hour
        {
            match (pass.anchor_seed, previous, content) {
                (Some(seed), Some(mut derived), Ok(content)) => {
                    self.stats.anchors_compared += 1;
                    match derived.advance(content, Some(seed)) {
                        Ok(()) => {
                            compare_anchor(&mut self.stats, frame, rows, &derived, pass.cursor)
                        }
                        Err(reason) => *self.stats.refused.entry(reason).or_default() += 1,
                    }
                }
                _ => self.stats.anchors_unproven += 1,
            }
        }
        match self.memo.states(frame.branch, frame.hour, rows, content) {
            Ok(_) => {
                self.stats.checksum_equal += 1;
                self.derived = self
                    .memo
                    .simulation()
                    .map(|simulation| (frame.branch, simulation.clone()));
            }
            Err(reason) if reason.contains("checksum") => {
                self.stats.checksum_mismatches += 1;
                self.stats.list(format!(
                    "branch {} hour {}: {reason}",
                    frame.branch, frame.hour
                ));
            }
            Err(reason) => *self.stats.refused.entry(reason).or_default() += 1,
        }
    }

    pub(super) fn print(&self) {
        let stats = &self.stats;
        match &self.content {
            Ok(content) => println!(
                "temperature content: {} weather slots, {} active, window {}, smoothing {}, max change {}, {} terrain modifiers",
                content.provinces.len(),
                content.provinces.iter().filter(|p| p.active).count(),
                content.settings.provinces_per_update,
                content.settings.smoothing,
                content.settings.max_change,
                content.settings.terrain.len()
            ),
            Err(reason) => println!("temperature content refused: {reason}"),
        }
        println!(
            "WeatherProvinces anchored temperatures: {} hours; derivation equal to the read checksum {}, checksum mismatches {}; anchors after a proven pass {} (rows equal {}, rows differing {}, cursors differing {}), anchors without one {}",
            stats.anchored_hours,
            stats.checksum_equal,
            stats.checksum_mismatches,
            stats.anchors_compared,
            stats.anchor_rows_equal,
            stats.anchor_rows_differing,
            stats.anchor_cursor_differing,
            stats.anchors_unproven
        );
        for (reason, count) in &stats.refused {
            println!("  refused ({reason}): {count}");
        }
        for line in &stats.listed {
            println!("  mismatch: {line}");
        }
    }
}

/// The anchored words against the pass from the hour before.
fn compare_anchor(
    stats: &mut Stats,
    frame: &DecodedFrame,
    rows: &[WeatherProvinceRow],
    derived: &Simulation,
    cursor: i32,
) {
    if rows.len() != derived.states.len() {
        stats.anchor_rows_differing += rows.len().abs_diff(derived.states.len()) as u64;
        stats.list(format!(
            "branch {} hour {}: {} anchored rows, derived {}",
            frame.branch,
            frame.hour,
            rows.len(),
            derived.states.len()
        ));
    }
    if derived.cursor != cursor {
        stats.anchor_cursor_differing += 1;
        stats.list(format!(
            "branch {} hour {}: anchor cursor {cursor}, derived {}",
            frame.branch, frame.hour, derived.cursor
        ));
    }
    for (row, derived) in rows.iter().zip(&derived.states) {
        match row.temperature.record() {
            Ok(TemperatureRecord::Anchored(state)) if state == *derived => {
                stats.anchor_rows_equal += 1;
            }
            recorded => {
                stats.anchor_rows_differing += 1;
                stats.list(format!(
                    "branch {} hour {} province {}: anchored {recorded:?}, derived {derived:?}",
                    frame.branch, frame.hour, row.id
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;
