//! One compact, campaign-wide ownership catalogue; no per-hour history retained.
use crate::{
    CountryMetadata, LandOwners, QueryLimits,
    reduce::short_text,
    scan::{mapped_slot, validate_campaign},
};
use replay::{
    container::Replay,
    frame::{FrameDecoder, TableView},
    rows::{TableId, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_COUNTRIES: usize = 4096;

/// Includes positive ownership observations from every recorded branch. Control
/// and cores do not establish ownership; unavailable intervals add no countries.
pub fn land_owners(replay: &Replay, cancelled: &AtomicBool) -> io::Result<LandOwners> {
    let check = || {
        if cancelled.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Country discovery cancelled",
            ))
        } else {
            Ok(())
        }
    };
    let mut countries = BTreeMap::<(String, u64, u32), CountryMetadata>::new();
    let mut incomplete = !replay.gaps.is_empty();
    let selected = [
        TableId::Campaign,
        TableId::States,
        TableId::CountryLinks,
        TableId::Countries,
    ];
    for block in &replay.index {
        check()?;
        if block.raw_length > QueryLimits::default().max_block_raw_bytes as u64 {
            return Err(io::Error::other(
                "Country discovery block exceeds its memory budget",
            ));
        }
        let frames = match replay.read_block_selected(block, &selected) {
            Ok(frames) => frames,
            Err(_) => {
                incomplete = true;
                continue;
            }
        };
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            check()?;
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) if validate_campaign(decoded, replay).is_ok() => decoded,
                _ => {
                    incomplete = true;
                    break;
                }
            };
            let states = match &decoded.tables[replay::rows::TableId::States.index()] {
                TableView::Complete(rows) => rows.rows::<tables::States>(),
                TableView::Truncated { rows, .. } => {
                    incomplete = true;
                    rows.rows::<tables::States>()
                }
                TableView::Unavailable { .. } => None,
            };
            let (Some(states), Ok(links), Ok(rows)) = (
                states,
                decoded.complete::<tables::CountryLinks>(),
                decoded.complete::<tables::Countries>(),
            ) else {
                incomplete = true;
                continue;
            };
            let mut owners: BTreeSet<_> = states
                .iter()
                .filter_map(|state| {
                    match links.binary_search_by_key(&state.owner, |link| link.id) {
                        Ok(index) => Some(links[index].canonical_slot),
                        Err(_) => {
                            incomplete |= state.owner != 0;
                            None
                        }
                    }
                })
                .collect();
            for row in rows {
                let Ok(slot) = mapped_slot(row, links) else {
                    incomplete = true;
                    continue;
                };
                if row.tag.is_empty() || !owners.remove(&slot) {
                    continue;
                }
                let tag = short_text(&String::from_utf8_lossy(&row.tag), 512);
                let key = (tag.clone(), frame.branch, slot);
                if countries.contains_key(&key) {
                    continue;
                }
                if countries.len() >= MAX_COUNTRIES {
                    return Err(io::Error::other(
                        "Country discovery exceeds its output budget",
                    ));
                }
                countries.insert(
                    key,
                    CountryMetadata {
                        id: slot.to_string(),
                        tag,
                        name: short_text(&String::from_utf8_lossy(&row.name), 512),
                        observed_branch_id: frame.branch.to_string(),
                        observed_hour: frame.hour,
                    },
                );
            }
            incomplete |= !owners.is_empty();
        }
    }
    check()?;
    Ok(LandOwners {
        countries: countries.into_values().collect(),
        warnings: if incomplete {
            vec!["Some ownership history is unavailable. Only countries with recorded land ownership are listed.".into()]
        } else {
            vec![]
        },
    })
}
