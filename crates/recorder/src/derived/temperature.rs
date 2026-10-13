//! Province temperatures against the hourly pass ([`replay::temperature`]).
//!
//! A recording that stores the pass's inputs (format 102) holds every province's words at an
//! anchor and the seeds since: each hour's derivation must reproduce the checksum of the
//! temperatures the collector read, and an anchor whose own hour the collector proved ordinary
//! must equal, word for word, the pass from the hour before.
//!
//! A recording that holds every hour's temperature but not the seed or the drifts is checked
//! where the pass is determined without them: every change between two
//! consecutive hours must lie in the temperature update window, from a cursor
//! advancing by the content's `provinces_per_update`, at an active province with an active
//! neighbour, and nothing changes while the manager is suspended. A province the pass reaches
//! without a region or period smooths its last committed temperature (the staged word equals it
//! after every commit), so its new temperature is exact without the seed; any other changed
//! province's must be reachable by smoothing a stage within `max_temperature_change` of it.

use replay::{
    frame::DecodedFrame,
    rows::{Recorded, TemperatureRecord, WeatherProvinceRow, tables},
    temperature::{self, Content, Memo, Simulation},
};

/// Mismatches listed by name; all of them are counted.
const LISTED: usize = 40;

#[derive(Default)]
pub(super) struct Stats {
    /// Frames with a complete WeatherProvinces table.
    hours: u64,
    /// Consecutive pairs compared, by the later hour's gates.
    updated_pairs: u64,
    suspended_pairs: u64,
    gate_unknown_pairs: u64,
    /// Rows whose temperature changed from the hour before.
    changed: u64,
    /// Hours whose changes fit no window the cursor can have reached.
    window_mismatches: u64,
    /// Feasible cursors after the last hour, per branch run.
    cursor_candidates: Vec<usize>,
    inactive_changed: u64,
    isolated_changed: u64,
    suspended_changed: u64,
    /// Window provinces the pass updated (unique cursor) whose temperature stayed.
    updated_unchanged: u64,
    exact_compared: u64,
    exact_equal: u64,
    bounded_compared: u64,
    bounded_violations: u64,
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
        self.checksum_mismatches
            + self.anchor_rows_differing
            + self.anchor_cursor_differing
            + self.window_mismatches
            + self.inactive_changed
            + self.isolated_changed
            + self.suspended_changed
            + (self.exact_compared - self.exact_equal)
            + self.bounded_violations
    }

    fn list(&mut self, line: String) {
        if self.listed.len() < LISTED {
            self.listed.push(line);
        }
    }
}

struct Previous {
    branch: u64,
    hour: i32,
    temperatures: Vec<i64>,
}

pub(super) struct Checker {
    content: Result<Content, String>,
    /// The derivation of anchored frames, and the hour before's derived words.
    memo: Memo,
    derived: Option<(u64, Simulation)>,
    previous: Option<Previous>,
    /// The cursors the hour's update can have started from; `None` until changes show it.
    candidates: Option<Vec<i32>>,
    pub(super) stats: Stats,
}

impl Checker {
    pub(super) fn new(content: Result<Content, String>) -> Self {
        Self {
            content,
            memo: Memo::default(),
            derived: None,
            previous: None,
            candidates: None,
            stats: Stats::default(),
        }
    }

    /// One frame: its derivation from its anchor, or its recorded temperatures compared with
    /// the hour before.
    pub(super) fn observe(&mut self, frame: &DecodedFrame, gates: &Result<[u8; 2], String>) {
        if let Ok(rows) = frame.complete::<tables::WeatherProvinces>()
            && matches!(
                rows.first().map(|row| row.temperature.record()),
                Some(Ok(TemperatureRecord::Anchored(_)))
            )
        {
            self.observe_anchored(frame, rows);
            return;
        }
        let temperatures = (|| {
            let rows = frame
                .complete::<tables::WeatherProvinces>()
                .map_err(|error| error.to_string())?;
            rows.iter()
                .enumerate()
                .map(|(slot, row)| {
                    if row.id as usize != slot {
                        return Err("WeatherProvinces slots not dense".to_owned());
                    }
                    match row.temperature.record() {
                        Ok(TemperatureRecord::AtHour(value)) => Ok(value),
                        Ok(_) => Err("temperature not recorded hourly".to_owned()),
                        Err(reason) => Err(reason.to_owned()),
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        })();
        let temperatures = match temperatures {
            Ok(temperatures) => temperatures,
            Err(reason) => {
                *self.stats.refused.entry(reason).or_default() += 1;
                self.reset();
                return;
            }
        };
        self.stats.hours += 1;
        if let Some(previous) = self.previous.take()
            && previous.branch == frame.branch
            && previous.hour.checked_add(1) == Some(frame.hour)
            && previous.temperatures.len() == temperatures.len()
        {
            self.compare(frame, &previous.temperatures, &temperatures, gates);
        } else {
            self.end_run();
        }
        self.previous = Some(Previous {
            branch: frame.branch,
            hour: frame.hour,
            temperatures,
        });
    }

    fn reset(&mut self) {
        self.previous = None;
        self.end_run();
    }

    fn end_run(&mut self) {
        if let Some(candidates) = self.candidates.take() {
            self.stats.cursor_candidates.push(candidates.len());
        }
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

    fn compare(
        &mut self,
        frame: &DecodedFrame,
        before: &[i64],
        after: &[i64],
        gates: &Result<[u8; 2], String>,
    ) {
        let Self {
            content,
            candidates,
            stats,
            ..
        } = self;
        let content = match content {
            Ok(content) => &*content,
            Err(reason) => {
                *stats.refused.entry(reason.clone()).or_default() += 1;
                return;
            }
        };
        if content.provinces.len() != after.len() {
            *stats
                .refused
                .entry(format!(
                    "content has {} weather slots, the recording {}",
                    content.provinces.len(),
                    after.len()
                ))
                .or_default() += 1;
            return;
        }
        let changed: Vec<u32> = (0..after.len() as u32)
            .filter(|&slot| before[slot as usize] != after[slot as usize])
            .collect();
        stats.changed += changed.len() as u64;
        match gates {
            Ok([0, 0]) => {}
            Ok(flags) => {
                stats.suspended_pairs += 1;
                if !changed.is_empty() {
                    stats.suspended_changed += changed.len() as u64;
                    stats.list(format!(
                        "branch {} hour {}: {} temperatures changed while the manager was suspended {flags:?}",
                        frame.branch,
                        frame.hour,
                        changed.len()
                    ));
                }
                return;
            }
            Err(_) => {
                // Whether the cursor advanced is unknown.
                stats.gate_unknown_pairs += 1;
                if let Some(run) = candidates.take() {
                    stats.cursor_candidates.push(run.len());
                }
                return;
            }
        }
        stats.updated_pairs += 1;
        let count = after.len() as i32;
        let width = content.settings.provinces_per_update;
        let fits = |cursor: i32| {
            changed
                .iter()
                .all(|&slot| inside(cursor, width, count, slot))
        };
        let feasible = match candidates.take() {
            Some(previous) => {
                let kept: Vec<i32> = previous.into_iter().filter(|&c| fits(c)).collect();
                if kept.is_empty() {
                    stats.window_mismatches += 1;
                    stats.list(format!(
                        "branch {} hour {}: {} changes fit no window of {width} from the cursors the earlier hours allow",
                        frame.branch,
                        frame.hour,
                        changed.len()
                    ));
                    infer(&changed, width, count)
                } else {
                    kept
                }
            }
            None => infer(&changed, width, count),
        };
        let states: Vec<temperature::State> = before
            .iter()
            .map(|&temperature| temperature::State {
                temperature,
                ..Default::default()
            })
            .collect();
        if let [cursor] = feasible[..]
            && let Ok((slots, _)) = temperature::window(cursor, width, count)
        {
            for slot in slots {
                let province = &content.provinces[slot as usize];
                if province.active
                    && before[slot as usize] == after[slot as usize]
                    && content
                        .neighbour_sums(slot, &states)
                        .is_ok_and(|(_, _, active)| active > 0)
                {
                    stats.updated_unchanged += 1;
                }
            }
        }
        for &slot in &changed {
            check_changed(stats, frame, content, &states, after, slot);
        }
        *candidates = (!feasible.is_empty()).then(|| {
            feasible
                .into_iter()
                .filter_map(|cursor| temperature::window(cursor, width, count).ok())
                .map(|(_, next)| next)
                .collect()
        });
    }

    pub(super) fn print(&mut self) {
        self.end_run();
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
        println!(
            "WeatherProvinces temperatures recorded hourly: {} hours; pairs updated {}, suspended {}, gates unknown {}; changed rows {}; mismatches {}",
            stats.hours,
            stats.updated_pairs,
            stats.suspended_pairs,
            stats.gate_unknown_pairs,
            stats.changed,
            stats.mismatches()
        );
        println!(
            "  window: mismatching hours {}, feasible cursors per run {:?}; window provinces updated but unchanged {}",
            stats.window_mismatches, stats.cursor_candidates, stats.updated_unchanged
        );
        println!(
            "  changed while inactive {}, without an active neighbour {}, while suspended {}",
            stats.inactive_changed, stats.isolated_changed, stats.suspended_changed
        );
        println!(
            "  without a period (exact): compared {}, equal {}; with a period (reachable stage): compared {}, violations {}",
            stats.exact_compared,
            stats.exact_equal,
            stats.bounded_compared,
            stats.bounded_violations
        );
        for (reason, count) in &stats.refused {
            println!("  refused ({reason}): {count}");
        }
        for line in &stats.listed {
            println!("  mismatch: {line}");
        }
    }
}

/// Whether the window from `cursor` visits `slot`: the `min(max(width, 1), count)` slots from
/// the cursor, wrapping ([`temperature::window`]); a cursor at or past the count visits none.
fn inside(cursor: i32, width: i32, count: i32, slot: u32) -> bool {
    (0..count).contains(&cursor)
        && (slot as i32 - cursor).rem_euclid(count) < width.max(1).min(count)
}

/// The cursors whose window holds every changed slot; none for an hour without changes.
fn infer(changed: &[u32], width: i32, count: i32) -> Vec<i32> {
    let Some(&first) = changed.first() else {
        return Vec::new();
    };
    // Only cursors less than a window before the first change can reach it.
    (0..width.max(1).min(count))
        .map(|back| (first as i32 - back).rem_euclid(count))
        .filter(|&cursor| {
            changed
                .iter()
                .all(|&slot| inside(cursor, width, count, slot))
        })
        .collect()
}

fn check_changed(
    stats: &mut Stats,
    frame: &DecodedFrame,
    content: &Content,
    states: &[temperature::State],
    after: &[i64],
    slot: u32,
) {
    let province = &content.provinces[slot as usize];
    let old = states[slot as usize].temperature;
    let new = after[slot as usize];
    if !province.active {
        stats.inactive_changed += 1;
        stats.list(format!(
            "branch {} hour {} province {slot}: inactive province changed {old} -> {new}",
            frame.branch, frame.hour
        ));
        return;
    }
    let (sum, _, active) = match content.neighbour_sums(slot, states) {
        Ok(sums) => sums,
        Err(reason) => {
            *stats.refused.entry(reason).or_default() += 1;
            return;
        }
    };
    if active <= 0 {
        stats.isolated_changed += 1;
        stats.list(format!(
            "branch {} hour {} province {slot}: no active neighbour, yet changed {old} -> {new}",
            frame.branch, frame.hour
        ));
        return;
    }
    let average = temperature::average(sum, active);
    let smoothing = content.settings.smoothing;
    match content.period(slot, frame.hour) {
        Err(reason) => {
            *stats.refused.entry(reason).or_default() += 1;
        }
        Ok(None) => {
            stats.exact_compared += 1;
            let derived = temperature::smooth(old, average, smoothing);
            if derived == new {
                stats.exact_equal += 1;
            } else {
                stats.list(format!(
                    "branch {} hour {} province {slot}: without a period {old} smooths to {derived}, recorded {new}",
                    frame.branch, frame.hour
                ));
            }
        }
        Ok(Some(_)) => {
            stats.bounded_compared += 1;
            // smooth(S) is nondecreasing for factors in 0..=100000: find the least stage
            // within the drift limit that reaches the recorded value.
            let limit = content.settings.max_change;
            let (mut low, mut high) = (old.saturating_sub(limit), old.saturating_add(limit));
            let reachable = if (0..=temperature::SCALE).contains(&smoothing) {
                while low < high {
                    let middle = low + (high - low) / 2;
                    if temperature::smooth(middle, average, smoothing) < new {
                        low = middle + 1;
                    } else {
                        high = middle;
                    }
                }
                temperature::smooth(low, average, smoothing) == new
            } else {
                true
            };
            if !reachable {
                stats.bounded_violations += 1;
                stats.list(format!(
                    "branch {} hour {} province {slot}: {new} is not a smoothed stage within {limit} of {old} (neighbour average {average})",
                    frame.branch, frame.hour
                ));
            }
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
