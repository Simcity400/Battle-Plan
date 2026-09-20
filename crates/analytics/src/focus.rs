//! Compact focus history. Dates bound observed completions, never pre-recording events.
use crate::{
    FocusEntry, FocusHistory, FocusOrder, FocusSegment, QueryLimits, scan::validate_campaign,
};
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};

pub fn focus_order(
    replay: &Replay,
    branch: u64,
    hour: i32,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<FocusOrder> {
    focus_history(replay, branch, tag, cancelled)?.at(hour)
}

/// Shared branch projection: every country is collected in one replay pass.
pub struct FocusBranch {
    pub countries: BTreeMap<String, FocusHistory>,
    pub retained_bytes: usize,
}

/// Country requests reuse one branch scan. The budget is shared by all cached
/// branches; switching countries never evicts another country's projection.
#[derive(Default)]
pub struct FocusCache {
    branches: std::collections::VecDeque<(u64, FocusBranch)>,
}

impl FocusCache {
    pub fn get(
        &mut self,
        replay: &Replay,
        branch: u64,
        tag: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<FocusHistory> {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Focus request cancelled",
            ));
        }
        if let Some(index) = self.branches.iter().position(|(id, _)| *id == branch) {
            let entry = self.branches.remove(index).unwrap();
            self.branches.push_back(entry);
        } else {
            let result = focus_branch(replay, branch, cancelled)?;
            while !self.branches.is_empty()
                && (self.branches.len() >= 4
                    || self
                        .branches
                        .iter()
                        .map(|(_, b)| b.retained_bytes)
                        .sum::<usize>()
                        + result.retained_bytes
                        > 64 * 1024 * 1024)
            {
                self.branches.pop_front();
            }
            self.branches.push_back((branch, result));
        }
        self.branches
            .back()
            .unwrap()
            .1
            .countries
            .get(tag)
            .cloned()
            .ok_or_else(|| io::Error::other("No recorded focus history for this country"))
    }
}

impl FocusHistory {
    pub fn at(&self, hour: i32) -> io::Result<FocusOrder> {
        let index = self.segments.partition_point(|s| s.first_hour <= hour);
        self.segments
            .get(index.wrapping_sub(1))
            .filter(|s| hour <= s.last_hour)
            .map(|s| s.order.clone())
            .ok_or_else(|| {
                io::Error::other("No complete recorded focus data for this country at this hour")
            })
    }
}

pub fn focus_history(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<FocusHistory> {
    focus_branch(replay, branch, cancelled)?
        .countries
        .remove(tag)
        .ok_or_else(|| io::Error::other("No recorded focus history for this country"))
}

pub fn focus_branch(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<FocusBranch> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Focus request cancelled",
        ));
    }
    let selected = [
        replay::rows::TableId::Campaign,
        replay::rows::TableId::Countries,
        replay::rows::TableId::CountryLinks,
        replay::rows::TableId::Focus,
        replay::rows::TableId::CompletedFocuses,
    ];
    let mut result = FocusBranch {
        countries: BTreeMap::new(),
        retained_bytes: 0,
    };
    let mut identities = BTreeMap::new();
    let mut segment_count = 0;
    for block in replay.index.iter().filter(|b| b.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Focus request cancelled",
            ));
        }
        if block.raw_length > QueryLimits::default().max_block_raw_bytes as u64 {
            return Err(io::Error::other(
                "Focus data block exceeds its memory budget",
            ));
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frames = match replay.read_block_selected(block, &selected) {
            Ok(frames) => frames,
            Err(_) => continue,
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Focus request cancelled",
                ));
            }
            let decoded = match decoder.advance(&frame) {
                Ok(frame) => frame,
                Err(_) => break,
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let (Ok(countries), Ok(links), Ok(focuses), Ok(completed)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
                decoded.complete::<tables::Focus>(),
                decoded.complete::<tables::CompletedFocuses>(),
            ) else {
                continue;
            };
            let by_slot: BTreeMap<_, _> = focuses
                .iter()
                .filter_map(|focus| {
                    links
                        .binary_search_by_key(&focus.country, |l| l.id)
                        .ok()
                        .map(|i| (links[i].canonical_slot, focus))
                })
                .collect();
            let mut by_country: BTreeMap<_, Vec<_>> = BTreeMap::new();
            for row in completed {
                by_country.entry(row.country).or_default().push(row);
            }
            for rows in by_country.values_mut() {
                rows.sort_by_key(|r| r.ordinal);
            }
            for country in countries {
                let Ok(slot) = crate::scan::mapped_slot(country, links) else {
                    continue;
                };
                let Some(focus) = by_slot.get(&slot) else {
                    continue;
                };
                let rows = by_country
                    .get(&focus.country)
                    .map_or(&[][..], Vec::as_slice);
                if !valid_order(rows, focus.completed_count) {
                    continue;
                }
                let Ok(tag) = std::str::from_utf8(&country.tag) else {
                    continue;
                };
                if tag.is_empty() || tag.len() > 512 {
                    continue;
                }
                let identity = (slot, country.id);
                let same_identity = identities.get(tag) == Some(&identity);
                let history = result.countries.entry(tag.to_owned()).or_default();
                let current = focus.current.as_deref().filter(|s| !s.is_empty());
                let continuous = focus.continuous.as_deref().filter(|s| !s.is_empty());
                if let Some(previous) = history.segments.last_mut() {
                    let order = &previous.order;
                    // Compare borrowed rows first: unchanged hours allocate no focus strings.
                    if same_identity
                        && previous.last_hour.checked_add(1) == Some(frame.hour)
                        && order.country_name.as_bytes() == country.name
                        && order.current.as_ref().map(|s| s.as_bytes()) == current
                        && order.continuous.as_ref().map(|s| s.as_bytes()) == continuous
                        && order.paused == (focus.paused != 0)
                        && order.completed.len() == rows.len()
                        && order
                            .completed
                            .iter()
                            .zip(rows)
                            .all(|(a, b)| a.token.as_bytes() == b.token)
                    {
                        previous.last_hour = frame.hour;
                        continue;
                    }
                }
                let mut order = FocusOrder {
                    country_name: String::from_utf8_lossy(&country.name).into_owned(),
                    completed: ordered(rows, focus.completed_count)?,
                    current: current.map(|s| String::from_utf8_lossy(s).into_owned()),
                    continuous: continuous.map(|s| String::from_utf8_lossy(s).into_owned()),
                    paused: focus.paused != 0,
                };
                if same_identity && let Some(previous) = history.segments.last() {
                    date_completions(&mut order, &previous.order, frame.hour);
                }
                result.retained_bytes += 256
                    + tag.len()
                    + order.country_name.len()
                    + order.current.as_ref().map_or(0, String::len)
                    + order.continuous.as_ref().map_or(0, String::len)
                    + order
                        .completed
                        .iter()
                        .map(|r| 64 + r.token.len())
                        .sum::<usize>();
                segment_count += 1;
                if result.retained_bytes > 64 * 1024 * 1024 || segment_count > 100000 {
                    return Err(io::Error::other("Focus history exceeds its memory budget"));
                }
                history.segments.push(FocusSegment {
                    first_hour: frame.hour,
                    last_hour: frame.hour,
                    order,
                });
                identities.insert(tag.to_owned(), identity);
            }
        }
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Focus request cancelled",
        ));
    }
    Ok(result)
}

fn date_completions(order: &mut FocusOrder, previous: &FocusOrder, hour: i32) {
    // Only an unchanged prefix establishes the identity of existing ordinals.
    // A reset/reorder starts another undated list rather than reusing old dates.
    if order.completed.len() < previous.completed.len()
        || !order
            .completed
            .iter()
            .zip(&previous.completed)
            .all(|(a, b)| a.token == b.token)
    {
        return;
    }
    for (index, entry) in order.completed.iter_mut().enumerate() {
        entry.completed_by_hour = previous
            .completed
            .get(index)
            .map_or(Some(hour), |old| old.completed_by_hour);
    }
}

fn valid_order(rows: &[&replay::rows::CompletedFocusRow], count: i32) -> bool {
    count >= 0
        && count as usize == rows.len()
        && rows.len() <= 512
        && rows.iter().enumerate().all(|(index, row)| {
            row.ordinal == index as u32 && !row.token.is_empty() && row.token.len() <= 512
        })
}

fn ordered(rows: &[&replay::rows::CompletedFocusRow], count: i32) -> io::Result<Vec<FocusEntry>> {
    if count < 0 || count as usize != rows.len() || rows.len() > 512 {
        return Err(io::Error::other(
            "Completed focus order is incomplete or exceeds its limit",
        ));
    }
    let mut rows = rows.to_vec();
    rows.sort_by_key(|row| row.ordinal);
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            if row.ordinal != index as u32 || row.token.is_empty() || row.token.len() > 512 {
                return Err(io::Error::other(
                    "Completed focus order has missing or invalid entries",
                ));
            }
            Ok(FocusEntry {
                ordinal: row.ordinal,
                token: String::from_utf8_lossy(&row.token).into_owned(),
                completed_by_hour: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::CompletedFocusRow;
    #[test]
    fn preserves_recorded_order_and_repeats() {
        let rows = [
            CompletedFocusRow {
                country: 42,
                ordinal: 2,
                token: b"a".to_vec(),
            },
            CompletedFocusRow {
                country: 42,
                ordinal: 0,
                token: b"a".to_vec(),
            },
            CompletedFocusRow {
                country: 42,
                ordinal: 1,
                token: b"z".to_vec(),
            },
        ];
        let result = ordered(&rows.iter().collect::<Vec<_>>(), 3).unwrap();
        assert_eq!(
            result.iter().map(|r| r.token.as_str()).collect::<Vec<_>>(),
            ["a", "z", "a"]
        );
        assert!(ordered(&[&rows[0]], 1).is_err());
        assert!(ordered(&[], 1).is_err());
        assert!(ordered(&[], -1).is_err());
        assert!(ordered(&[], 0).unwrap().is_empty());
    }
}
