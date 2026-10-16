//! Transient reads for S families beyond their committed prepared prefix.
use crate::{
    Source,
    metadata::check_cancel,
    prepared::{coverage::Coverage, series::Run},
    products::country_groups::HourSpan,
};
use replay::{
    container::SeekResult,
    frame::{DecodedFrame, FrameDecoder},
    rows::TableId,
};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// One frame's values: `(tag, field)` -> (native holder identity, raw value).
pub type Samples = BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>;

/// A family's stateless per-frame extraction, shared with its observer. Given
/// requested keys it may add the keys they expand to (a country's parts); a
/// family whose extraction needs recorded content captures it in the closure.
pub type Sample<'a> = &'a dyn Fn(&DecodedFrame, Option<&[(String, String)]>) -> io::Result<Samples>;

/// Adapter inputs are transient samples plus the branch's real bounds.
pub type AdapterTail<'a> = (
    (i32, i32),
    &'a BTreeMap<(String, String), Vec<Run>>,
    &'a [HourSpan],
    &'a BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
    bool,
);

/// Join accepted identity history and transient native-holder spans without a recording walk.
/// Exact state evidence replaces sampled evidence at its supplying hours, including aliases.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn histories(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    coverage: Option<&Coverage>,
    identities: &BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
    exact: &BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
    replay: &replay::container::Replay,
    requested: &[String],
    cancelled: &AtomicBool,
) -> io::Result<crate::products::country_history::RecordedHistories> {
    use crate::products::country_history::{CountryIdentityRun, HistoryScan, RecordedHistories};
    use std::{collections::BTreeSet, sync::Arc};

    check_cancel(cancelled)?;
    let mut history = if coverage.is_some() {
        store
            .read(key, &format!("country-identities-{branch}"), 0)?
            .map(|file| serde_json::from_reader::<_, HistoryScan>(io::BufReader::new(file)))
            .transpose()?
            .map_or_else(RecordedHistories::default, HistoryScan::finish)
    } else {
        RecordedHistories::default()
    };
    // Fields share native holders; discard repeated evidence before joining by tag.
    let by_tag = |evidence: &BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>| {
        let mut tags = BTreeMap::<String, BTreeSet<(i32, i32, u32, u32)>>::new();
        for ((tag, _), spans) in evidence {
            for &span in spans {
                if span.0 > span.1 {
                    return Err(crate::invalid("Invalid progressive country identity span"));
                }
                tags.entry(tag.clone()).or_default().insert(span);
            }
        }
        Ok(tags)
    };
    let exact = by_tag(exact)?;
    // A native slot cannot have two exact holders or aliases at the same hour.
    let mut slots = BTreeMap::<u32, BTreeSet<(i32, i32, &str, u32)>>::new();
    for (tag, spans) in &exact {
        check_cancel(cancelled)?;
        let mut previous: Option<(i32, u32, u32)> = None;
        for &(first, last, slot, id) in spans {
            if let Some((end, before_slot, before_id)) = previous {
                if end >= first && (before_slot, before_id) != (slot, id) {
                    return Err(crate::invalid(
                        "Conflicting exact progressive country identities",
                    ));
                }
                previous = Some((last.max(end), slot, id));
            } else {
                previous = Some((last, slot, id));
            }
            slots
                .entry(slot)
                .or_default()
                .insert((first, last, tag, id));
        }
    }
    for spans in slots.values() {
        let mut previous: Option<(i32, &str, u32)> = None;
        for &(first, last, tag, id) in spans {
            if let Some((end, before_tag, before_id)) = previous {
                if end >= first && (before_tag, before_id) != (tag, id) {
                    return Err(crate::invalid(
                        "Conflicting exact progressive country identities",
                    ));
                }
                previous = Some((last.max(end), tag, id));
            } else {
                previous = Some((last, tag, id));
            }
        }
    }
    let mut tags = BTreeMap::<String, BTreeSet<(i32, i32, u32, u32)>>::new();
    for (tag, spans) in by_tag(identities)? {
        check_cancel(cancelled)?;
        let joined = tags.entry(tag.clone()).or_default();
        for (first, last, slot, id) in spans {
            let mut cuts = exact
                .get(&tag)
                .into_iter()
                .flatten()
                .map(|span| (span.0, span.1))
                .chain(
                    slots
                        .get(&slot)
                        .into_iter()
                        .flatten()
                        .map(|span| (span.0, span.1)),
                )
                .filter(|&(start, end)| start <= last && end >= first)
                .collect::<Vec<_>>();
            cuts.sort_unstable();
            let mut start = i64::from(first);
            for (cut_first, cut_last) in cuts {
                if start < i64::from(cut_first) {
                    joined.insert((start as i32, cut_first - 1, slot, id));
                }
                start = start.max(i64::from(cut_last) + 1);
            }
            if start <= i64::from(last) {
                joined.insert((start as i32, last, slot, id));
            }
        }
    }
    for (tag, spans) in exact {
        tags.entry(tag).or_default().extend(spans);
    }
    for (tag, spans) in tags {
        check_cancel(cancelled)?;
        let country = history.countries.entry(tag.to_owned()).or_default();
        let runs = Arc::make_mut(&mut country.identities);
        let segments = Arc::make_mut(&mut country.segments);
        for (first, last, slot, country_id) in spans {
            if first > last {
                return Err(crate::invalid("Invalid progressive country identity span"));
            }
            let first = coverage.map_or(i64::from(first), |coverage| {
                i64::from(first).max(i64::from(coverage.last_hour) + 1)
            });
            if first > i64::from(last) {
                continue;
            }
            let first = first as i32;
            if let Some(previous) = runs.last_mut() {
                let same = (previous.slot, previous.country_id) == (slot, country_id);
                if !same && previous.last_hour >= first {
                    return Err(crate::invalid("Conflicting progressive country identities"));
                }
                if same && i64::from(previous.last_hour) + 1 >= i64::from(first) {
                    previous.last_hour = previous.last_hour.max(last);
                    segments
                        .last_mut()
                        .ok_or_else(|| crate::invalid("Missing prepared country identity segment"))?
                        .last_hour = previous.last_hour;
                    continue;
                }
            }
            runs.push(CountryIdentityRun {
                first_hour: first,
                last_hour: last,
                slot,
                country_id,
            });
            segments.push(HourSpan {
                first_hour: first,
                last_hour: last,
            });
        }
    }
    // A manual tag may have been renamed before the sampled window. Seed only
    // tags lacking any holder evidence, from one branch-start keyframe. This
    // observation links aliases by native holder without inventing intervening
    // identity coverage or proving that any other tag is absent.
    let missing: BTreeSet<_> = requested
        .iter()
        .filter(|tag| {
            history
                .countries
                .get(*tag)
                .is_none_or(|country| country.identities.is_empty())
        })
        .collect();
    if !missing.is_empty()
        && let Some(first) = replay.index.iter().find(|block| block.branch == branch)
    {
        use crate::metadata::{TaggedCountry, tagged_countries};
        use replay::rows::tables;
        check_cancel(cancelled)?;
        let selected = [TableId::Countries, TableId::CountryLinks];
        let keyframe = replay.read_keyframe_selected(first, &selected)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frame = decoder.advance(&keyframe)?;
        check_cancel(cancelled)?;
        if let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) {
            let tagged = tagged_countries(countries, links);
            for tag in missing {
                check_cancel(cancelled)?;
                if let Some(TaggedCountry::Found(row, slot)) = tagged.get(tag.as_bytes()) {
                    let country = history.countries.entry(tag.clone()).or_default();
                    Arc::make_mut(&mut country.identities).push(CountryIdentityRun {
                        first_hour: frame.hour,
                        last_hour: frame.hour,
                        slot: *slot,
                        country_id: row.id,
                    });
                    Arc::make_mut(&mut country.segments).push(HourSpan {
                        first_hour: frame.hour,
                        last_hour: frame.hour,
                    });
                }
            }
        }
    }
    // A requested subset does not prove complete-country absence at any tail hour.
    Ok(history)
}

/// Clip approximate provenance to the same viewport and membership as chart values.
pub fn clip_approximate(
    summary: &mut crate::prepared_chart::ChartSummary,
    approximate: &[HourSpan],
    within: Option<&[HourSpan]>,
) {
    let mut clipped = Vec::new();
    for span in approximate {
        let memberships = within.unwrap_or_else(|| std::slice::from_ref(span));
        for membership in memberships {
            let first_hour = span
                .first_hour
                .max(membership.first_hour)
                .max(summary.from.floor() as i32);
            let last_hour = span
                .last_hour
                .min(membership.last_hour)
                .min(summary.to.floor() as i32);
            if first_hour <= last_hour {
                clipped.push(HourSpan {
                    first_hour,
                    last_hour,
                });
            }
        }
    }
    clipped.sort_by_key(|span| span.first_hour);
    summary.approximate.clear();
    for span in clipped {
        crate::products::country_groups::mark_unobserved(
            &mut summary.approximate,
            span.first_hour,
            span.last_hour,
        );
    }
}

/// Combine prepared navigation with transient runs, preserving clipped membership edges.
#[allow(clippy::type_complexity)]
pub fn navigate(
    prepared: &BTreeMap<(String, String), (Option<f64>, Option<Run>)>,
    tail: &BTreeMap<(String, String), Vec<Run>>,
    requested: &[(String, String)],
    within: &BTreeMap<String, Vec<HourSpan>>,
    navigation: &crate::prepared::series::Navigation,
) -> io::Result<Option<f64>> {
    let origin = navigation.origin()?;
    let forward = matches!(
        navigation.key,
        crate::prepared::series::MeasureKey::Home | crate::prepared::series::MeasureKey::ArrowRight
    );
    let mut result: Option<f64> = None;
    for key in requested {
        let raw = tail.get(key).map_or(&[][..], Vec::as_slice);
        let (mut candidate, preceding) = prepared.get(key).copied().unwrap_or((None, None));
        candidate = candidate.filter(|hour| {
            if forward {
                *hour > origin
            } else {
                *hour < origin
            }
        });
        let mut runs = std::borrow::Cow::Borrowed(raw);
        if let (Some(first), Some(before)) = (raw.first(), preceding)
            && !first.break_before
            && before.last_hour.checked_add(1) == Some(first.first_hour)
            && before.value == first.value
        {
            if candidate == Some(f64::from(first.first_hour)) {
                candidate = None;
            }
            let mut joined = raw.to_vec();
            joined[0].first_hour = before.first_hour;
            runs = std::borrow::Cow::Owned(joined);
        }
        if let Some(hour) = candidate {
            result = Some(result.map_or(hour, |previous| {
                if forward {
                    previous.min(hour)
                } else {
                    previous.max(hour)
                }
            }));
        }
        let spans = within.get(&key.0).map_or_else(
            || vec![(navigation.first_hour, navigation.last_hour)],
            |spans| {
                spans
                    .iter()
                    .map(|span| {
                        (
                            f64::from(span.first_hour).max(navigation.first_hour),
                            f64::from(span.last_hour).min(navigation.last_hour),
                        )
                    })
                    .collect()
            },
        );
        for (first, last) in spans.into_iter().filter(|(first, last)| first <= last) {
            let at = if forward {
                origin.max(first - 1.0)
            } else {
                origin.min(last + 1.0)
            };
            let candidate = if forward {
                runs.iter()
                    .find(|run| f64::from(run.last_hour) >= at.floor())
                    .and_then(|run| {
                        if f64::from(run.first_hour) <= at.floor() {
                            run.last_hour.checked_add(1).map(f64::from)
                        } else {
                            Some(f64::from(run.first_hour))
                        }
                    })
            } else {
                runs.iter()
                    .rev()
                    .find(|run| f64::from(run.first_hour) < at)
                    .map(|run| {
                        run.last_hour
                            .checked_add(1)
                            .map(f64::from)
                            .filter(|hour| *hour < at)
                            .unwrap_or(f64::from(run.first_hour))
                    })
            }
            .filter(|hour| *hour >= first && *hour <= last);
            let carried = runs.iter().any(|run| {
                f64::from(run.first_hour) <= first && f64::from(run.last_hour) >= first.floor()
            });
            let edge = (carried && (forward && origin < first || !forward && origin > first))
                .then_some(first);
            for hour in [candidate, edge].into_iter().flatten() {
                result = Some(result.map_or(hour, |previous| {
                    if forward {
                        previous.min(hour)
                    } else {
                        previous.max(hour)
                    }
                }));
            }
        }
    }
    Ok(result.or_else(|| {
        navigation
            .from
            .map(|hour| hour.clamp(navigation.first_hour, navigation.last_hour))
    }))
}

/// One reached hour's selected tables, decoded directly from its block for a
/// whole-frame state answer; none at an unrecorded hour or inside a recorded gap.
pub fn frame_at(
    source: &Source,
    branch: u64,
    hour: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Option<Box<DecodedFrame>>> {
    check_cancel(cancelled)?;
    let replay = source.replay_ref();
    if replay
        .gaps
        .iter()
        .any(|gap| gap.branch == branch && gap.first_hour <= hour && gap.last_hour >= hour)
    {
        return Ok(None);
    }
    let frame =
        match replay.seek_selected_with(branch, hour, Some(tables), || check_cancel(cancelled))? {
            SeekResult::Frame(frame) => Some(frame),
            SeekResult::Gap { .. } => None,
        };
    check_cancel(cancelled)?;
    Ok(frame)
}

/// About one year: the owner's ~2 s exact-zoom target. This is not a country or reply cap.
const EXACT_ZOOM_BLOCKS: usize = 13;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tail(
    source: &Source,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    window: (i32, i32),
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
)> {
    check_cancel(cancelled)?;
    if coverage.is_some_and(|coverage| coverage.complete) || window.0 > window.1 {
        return Ok((BTreeMap::new(), Vec::new(), BTreeMap::new()));
    }
    let replay = source.replay_ref();
    let first = coverage.map_or(i64::from(window.0), |coverage| {
        i64::from(window.0).max(i64::from(coverage.last_hour) + 1)
    });
    let blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| {
            block.branch == branch
                && i64::from(block.last_hour) >= first
                && block.first_hour <= window.1
        })
        .collect();
    sample_blocks(
        source,
        branch,
        tables,
        requested,
        (first, window.1),
        blocks,
        sample,
        None,
        cancelled,
    )
}

/// State reads select distinct target blocks, not the intervening history.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn state_tail(
    source: &Source,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    hours: &[i32],
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
)> {
    check_cancel(cancelled)?;
    let first = coverage.map_or(i64::from(i32::MIN), |coverage| {
        i64::from(coverage.last_hour) + 1
    });
    let hours = hours
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let blocks = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| {
            block.branch == branch
                && !coverage.is_some_and(|coverage| coverage.complete)
                && i64::from(block.last_hour) >= first
                && hours
                    .range((i64::from(block.first_hour).max(first) as i32)..=block.last_hour)
                    .next()
                    .is_some()
        })
        .collect();
    let hours = hours.into_iter().collect::<Vec<_>>();
    sample_blocks(
        source,
        branch,
        tables,
        requested,
        (first, i32::MAX),
        blocks,
        sample,
        Some(&hours),
        cancelled,
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sample_blocks(
    source: &Source,
    branch: u64,
    tables: &[TableId],
    requested: &[(String, String)],
    window: (i64, i32),
    blocks: Vec<&replay::container::BlockIndex>,
    sample: Sample<'_>,
    state_hours: Option<&[i32]>,
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
)> {
    let mut runs: BTreeMap<(String, String), Vec<Run>> = BTreeMap::new();
    let mut approximate = Vec::new();
    let mut identities: BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>> = BTreeMap::new();
    let replay = source.replay_ref();
    let state_hours = state_hours.map(|hours| {
        hours
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
    });
    let first = window.0;
    let exact = blocks.len() <= EXACT_ZOOM_BLOCKS;
    let mut previous: BTreeMap<(String, String), (i32, Option<(u32, u32)>, Option<i128>)> =
        BTreeMap::new();
    // Read the preceding observation at an exact left edge, so a preparation
    // boundary cannot invent an identity or value break. The baseline is transient.
    if exact && let Some(block) = blocks.first() {
        let start = i64::from(block.first_hour).max(first) as i32;
        if start == block.first_hour
            && let Some(before) = start.checked_sub(1)
            && replay.index.iter().any(|index| {
                index.branch == branch && index.first_hour <= before && index.last_hour >= before
            })
            && !replay.gaps.iter().any(|gap| {
                gap.branch == branch && gap.first_hour <= before && gap.last_hour >= before
            })
            && let SeekResult::Frame(frame) =
                replay
                    .seek_selected_with(branch, before, Some(tables), || check_cancel(cancelled))?
        {
            for (key, (identity, value)) in sample(&frame, Some(requested))? {
                previous.insert(key, (frame.hour, identity, value));
            }
        }
    }
    for block in blocks {
        check_cancel(cancelled)?;
        let start = i64::from(block.first_hour).max(first) as i32;
        let end = block.last_hour.min(window.1);
        if exact {
            let frames = replay.read_block_selected(block, tables)?;
            let mut decoder = FrameDecoder::for_tables(tables);
            for frame in &frames {
                check_cancel(cancelled)?;
                let decoded = decoder.advance(frame)?;
                if decoded.hour > end {
                    break;
                }
                if replay.gaps.iter().any(|gap| {
                    gap.branch == branch
                        && gap.first_hour <= decoded.hour
                        && gap.last_hour >= decoded.hour
                }) {
                    previous.clear();
                    continue;
                }
                let observed = sample(decoded, Some(requested))?;
                if decoded.hour < start
                    || state_hours
                        .as_ref()
                        .is_some_and(|hours| !hours.contains(&decoded.hour))
                {
                    for (key, (identity, value)) in observed {
                        previous.insert(key, (decoded.hour, identity, value));
                    }
                    continue;
                }
                for (key, (identity, value)) in observed {
                    if let Some((slot, country_id)) = identity {
                        let rows = identities.entry(key.clone()).or_default();
                        if let Some(last) = rows.last_mut().filter(|last| {
                            last.1.checked_add(1) == Some(decoded.hour)
                                && (last.2, last.3) == (slot, country_id)
                        }) {
                            last.1 = decoded.hour;
                        } else {
                            rows.push((decoded.hour, decoded.hour, slot, country_id));
                        }
                    }
                    if let Some(value) = value {
                        let contiguous =
                            previous
                                .get(&key)
                                .is_some_and(|&(hour, before, previous_value)| {
                                    hour.checked_add(1) == Some(decoded.hour)
                                        && before == identity
                                        && previous_value.is_some()
                                });
                        let rows = runs.entry(key.clone()).or_default();
                        if let Some(last) = rows.last_mut().filter(|run| {
                            contiguous
                                && run.value == value
                                && run.last_hour.checked_add(1) == Some(decoded.hour)
                        }) {
                            last.last_hour = decoded.hour;
                        } else {
                            rows.push(Run::new(decoded.hour, decoded.hour, value, !contiguous));
                        }
                    }
                    previous.insert(key, (decoded.hour, identity, value));
                }
            }
        } else {
            // Only the block's keyframe sections are read: no decode through the block.
            let keyframe = replay.read_keyframe_selected(block, tables)?;
            let mut decoder = FrameDecoder::for_tables(tables);
            let frame = decoder.advance(&keyframe)?;
            let mut spans = vec![HourSpan {
                first_hour: start,
                last_hour: end,
            }];
            for gap in replay.gaps.iter().filter(|gap| gap.branch == branch) {
                spans = spans
                    .into_iter()
                    .flat_map(|span| {
                        if gap.last_hour < span.first_hour || gap.first_hour > span.last_hour {
                            return vec![span];
                        }
                        let mut parts = Vec::new();
                        if gap.first_hour > span.first_hour {
                            parts.push(HourSpan {
                                first_hour: span.first_hour,
                                last_hour: gap.first_hour - 1,
                            });
                        }
                        if gap.last_hour < span.last_hour {
                            parts.push(HourSpan {
                                first_hour: gap.last_hour + 1,
                                last_hour: span.last_hour,
                            });
                        }
                        parts
                    })
                    .collect();
            }
            if replay.gaps.iter().any(|gap| {
                gap.branch == branch && gap.first_hour <= frame.hour && gap.last_hour >= frame.hour
            }) {
                continue;
            }
            for (key, (identity, value)) in sample(frame, Some(requested))? {
                if let Some((slot, country_id)) = identity {
                    let rows = identities.entry(key.clone()).or_default();
                    for span in &spans {
                        // An unchanged holder across adjacent blocks stays one span,
                        // so the output grows with identity changes, not blocks.
                        if let Some(last) = rows.last_mut().filter(|last| {
                            last.1.checked_add(1) == Some(span.first_hour)
                                && (last.2, last.3) == (slot, country_id)
                        }) {
                            last.1 = span.last_hour;
                        } else {
                            rows.push((span.first_hour, span.last_hour, slot, country_id));
                        }
                    }
                }
                if let Some(value) = value {
                    for span in &spans {
                        runs.entry(key.clone()).or_default().push(Run::new(
                            span.first_hour,
                            span.last_hour,
                            value,
                            true,
                        ));
                    }
                }
            }
            for span in spans {
                if let Some(last) = approximate.last_mut().filter(|last: &&mut HourSpan| {
                    last.last_hour.checked_add(1) == Some(span.first_hour)
                }) {
                    last.last_hour = span.last_hour;
                } else {
                    approximate.push(span);
                }
            }
        }
    }
    Ok((runs, approximate, identities))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, continuation_records, records, source as frame};
    use replay::container::Replay;
    use std::sync::Arc;

    fn renamed_seed_recording() -> Replay {
        fn named(branch: u64, hour: i32, tag: &[u8]) -> DecodedFrame {
            let mut frame = frame(branch, hour, None);
            if let replay::frame::TableView::Complete(replay::rows::TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = tag.to_vec();
            }
            frame
        }
        let prefix = Replay::from_records(
            CAMPAIGN.into(),
            &[
                records(2, vec![named(2, 0, b"unrelated")]),
                records(1, vec![named(1, 10, b"mod/old"), named(1, 11, b"mod/old")]),
            ],
        )
        .unwrap();
        prefix
            .append(&[continuation_records(
                &prefix,
                1,
                vec![named(1, 20, b"mod/new"), named(1, 21, b"mod/new")],
            )])
            .unwrap()
    }

    #[test]
    fn progressive_histories_seed_resolves_a_rename_before_the_window_without_writing_or_walking() {
        use crate::products::country_history::{selected_aliases, selected_identities};
        let fixture = crate::prepared::series::tests::Fixture::new();
        let mut replay = renamed_seed_recording();
        // Only the requested branch's first block is decoded; no later payload is
        // needed to join supplied spans. Earlier blocks stay readable because the
        // reader admits blocks in recording order.
        let outside = replay.bytes().len() as u64;
        let keep = replay
            .index
            .iter()
            .position(|block| block.branch == 1)
            .unwrap();
        for (position, block) in replay.index.iter_mut().enumerate() {
            if position > keep {
                // Offsets stay ordered for block lookup; any read runs past the file.
                block.length = outside;
            }
        }
        let spans = BTreeMap::from([
            (("mod/new".into(), "fuel".into()), vec![(20, 21, 7, 9001)]),
            (
                ("other slot".into(), "fuel".into()),
                vec![(20, 21, 8, 9001)],
            ),
        ]);
        // Duplicate and absent picks still admit only the one seed keyframe.
        let requested = vec!["mod/old".into(), "not at start".into(), "mod/old".into()];
        let history = histories(
            &fixture.store,
            &fixture.key,
            1,
            None,
            &spans,
            &BTreeMap::new(),
            &replay,
            &requested,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            selected_identities(&history, &requested),
            std::collections::BTreeSet::from([(7, 9001)])
        );
        assert_eq!(
            selected_aliases(&history, &["mod/old".into()]),
            ["mod/new", "mod/old"]
        );
        let seed = &history.countries["mod/old"].identities;
        assert_eq!(seed.len(), 1);
        assert_eq!(
            (
                seed[0].first_hour,
                seed[0].last_hour,
                seed[0].slot,
                seed[0].country_id
            ),
            (10, 10, 7, 9001)
        );
        assert_eq!(
            history.countries["mod/old"].segments,
            Arc::new(vec![HourSpan {
                first_hour: 10,
                last_hour: 10
            }])
        );
        assert_eq!(history.countries["mod/new"].identities[0].first_hour, 20);
        assert!(
            history.identity_reads.is_empty(),
            "a seed cannot prove complete census or absence"
        );
        assert!(!history.countries.contains_key("not at start"));
        assert!(
            fixture.store.saved_key().unwrap().is_none(),
            "the identity seed is transient"
        );
    }

    #[test]
    fn progressive_histories_seed_keeps_tags_absent_at_branch_start_unknown_and_checks_cancellation()
     {
        use crate::products::country_history::selected_identities;
        let fixture = crate::prepared::series::tests::Fixture::new();
        let replay = renamed_seed_recording();
        let requested = vec!["not at start".into()];
        let history = histories(
            &fixture.store,
            &fixture.key,
            1,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &replay,
            &requested,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            history.countries.is_empty(),
            "do not seed unrequested countries"
        );
        assert!(selected_identities(&history, &requested).is_empty());
        assert!(history.identity_reads.is_empty());
        assert!(fixture.store.saved_key().unwrap().is_none());
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            histories(
                &fixture.store,
                &fixture.key,
                1,
                None,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &replay,
                &requested,
                &cancelled
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn progressive_histories_warm_and_sampled_identity_evidence_read_no_seed_keyframe() {
        use crate::products::country_history::{HistoryScan, selected_aliases};
        let fixture = crate::prepared::series::tests::Fixture::new();
        let mut replay = renamed_seed_recording();
        let selected = [TableId::Countries, TableId::CountryLinks];
        let first = replay.index.iter().find(|block| block.branch == 1).unwrap();
        let keyframe = replay.read_keyframe_selected(first, &selected).unwrap();
        let mut decoder = FrameDecoder::for_tables(&selected);
        let mut prefix = HistoryScan::new(false, &[]);
        prefix.observe(decoder.advance(&keyframe).unwrap());
        let mut writer = fixture.writer();
        writer
            .write_at(
                "country-identities-1",
                0,
                serde_json::to_vec(&prefix).unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let outside = replay.bytes().len() as u64;
        for block in &mut replay.index {
            block.offset = outside;
        }
        let first = replay.index.iter().find(|block| block.branch == 1).unwrap();
        assert!(
            replay.read_keyframe_selected(first, &selected).is_err(),
            "the seed read must fail if attempted"
        );
        let coverage = Coverage {
            schema: 0,
            first_hour: 10,
            last_hour: 10,
            complete: false,
        };
        let spans = BTreeMap::from([(("mod/new".into(), "fuel".into()), vec![(20, 21, 7, 9001)])]);
        let requested = vec!["mod/old".into(), "mod/new".into()];
        let history = histories(
            &fixture.store,
            &fixture.key,
            1,
            Some(&coverage),
            &spans,
            &BTreeMap::new(),
            &replay,
            &requested,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            selected_aliases(&history, &["mod/old".into()]),
            ["mod/new", "mod/old"]
        );
        assert_eq!(
            history.identity_reads,
            [HourSpan {
                first_hour: 10,
                last_hour: 10
            }]
        );
        let sampled = histories(
            &fixture.store,
            &fixture.key,
            1,
            None,
            &spans,
            &BTreeMap::new(),
            &replay,
            &["mod/new".into()],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            !sampled.countries.contains_key("mod/old"),
            "cold reads ignore stale accepted pages"
        );
        assert_eq!(sampled.countries["mod/new"].identities[0].first_hour, 20);
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "country-identities-1")
                .unwrap(),
            Some(1)
        );
    }

    #[test]
    fn progressive_histories_join_prefix_aliases_gaps_and_tail_only_holders_without_writing() {
        use crate::{prepared::series::tests::Fixture, products::country_history::HistoryScan};
        let fixture = Fixture::new();
        let mut prefix = HistoryScan::new(false, &[]);
        for hour in 0..=1 {
            prefix.observe(&crate::test_fixtures::scale_frame(1, hour, 1));
        }
        let mut writer = fixture.writer();
        writer
            .write_at(
                "country-identities-1",
                0,
                serde_json::to_vec(&prefix).unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let coverage = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        };
        let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
        let spans = BTreeMap::from([
            (
                (tag.clone(), "fuel".into()),
                vec![(0, 4, 0, 100_000), (7, 8, 0, 200_000)],
            ),
            ((tag.clone(), "capacity".into()), vec![(2, 4, 0, 100_000)]),
            (("mod/new".into(), "fuel".into()), vec![(5, 6, 0, 100_000)]),
            (("tail only".into(), "fuel".into()), vec![(2, 8, 1, 333)]),
        ]);
        let flag = AtomicBool::new(false);
        let history = histories(
            &fixture.store,
            &fixture.key,
            1,
            Some(&coverage),
            &spans,
            &BTreeMap::new(),
            source(1).replay_ref(),
            &[],
            &flag,
        )
        .unwrap();
        let runs = &history.countries[&tag].identities;
        assert_eq!(runs.len(), 2);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour, runs[0].country_id),
            (0, 4, 100_000)
        );
        assert_eq!(
            (runs[1].first_hour, runs[1].last_hour, runs[1].country_id),
            (7, 8, 200_000)
        );
        assert_eq!(
            history.countries["mod/new"].identities[0].country_id,
            100_000
        );
        assert_eq!(history.countries["tail only"].identities[0].country_id, 333);
        assert_eq!(
            history.identity_reads,
            vec![HourSpan {
                first_hour: 0,
                last_hour: 1
            }]
        );
        assert_eq!(history.countries[&tag].segments.len(), 2);
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "country-identities-1")
                .unwrap(),
            Some(1)
        );
        let cold = histories(
            &fixture.store,
            &fixture.key,
            1,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            source(1).replay_ref(),
            &[],
            &flag,
        )
        .unwrap();
        assert!(
            cold.countries.is_empty(),
            "no accepted coverage must ignore stale prepared identity pages"
        );
        let conflicting = BTreeMap::from([(
            (tag, "fuel".into()),
            vec![(2, 4, 0, 100_000), (3, 5, 0, 200_000)],
        )]);
        assert!(
            histories(
                &fixture.store,
                &fixture.key,
                1,
                Some(&coverage),
                &conflicting,
                &BTreeMap::new(),
                source(1).replay_ref(),
                &[],
                &flag
            )
            .is_err()
        );
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            histories(
                &fixture.store,
                &fixture.key,
                1,
                None,
                &spans,
                &BTreeMap::new(),
                source(1).replay_ref(),
                &[],
                &flag
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn progressive_histories_exact_state_replaces_rough_holder_at_its_hour() {
        let fixture = crate::prepared::series::tests::Fixture::new();
        let rough = BTreeMap::from([(("mod/a".into(), "fuel".into()), vec![(10, 11, 0, 100)])]);
        let exact = BTreeMap::from([
            (("mod/a".into(), "fuel".into()), vec![(11, 11, 0, 200)]),
            (("mod/a".into(), "capacity".into()), vec![(11, 11, 0, 200)]),
        ]);
        let history = histories(
            &fixture.store,
            &fixture.key,
            1,
            None,
            &rough,
            &exact,
            source(1).replay_ref(),
            &[],
            &AtomicBool::new(false),
        )
        .unwrap();
        let runs = &history.countries["mod/a"].identities;
        assert_eq!(
            runs.iter()
                .map(|run| (run.first_hour, run.last_hour, run.country_id))
                .collect::<Vec<_>>(),
            [(10, 10, 100), (11, 11, 200)]
        );
        assert_eq!(history.countries["mod/a"].segments.len(), 2);
        assert!(
            history.identity_reads.is_empty(),
            "partial evidence must not prove full-census absence"
        );
        assert_eq!(
            rough.values().next().unwrap(),
            &[(10, 11, 0, 100)],
            "rough provenance stays with the numeric tail"
        );
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "country-identities-1")
                .unwrap(),
            None
        );
    }

    #[test]
    fn progressive_histories_exact_alias_clips_both_sides_of_the_rough_span() {
        let fixture = crate::prepared::series::tests::Fixture::new();
        let rough = BTreeMap::from([(("mod/old".into(), "fuel".into()), vec![(10, 12, 0, 100)])]);
        let exact = BTreeMap::from([(("mod/new".into(), "fuel".into()), vec![(11, 11, 0, 100)])]);
        let history = histories(
            &fixture.store,
            &fixture.key,
            1,
            None,
            &rough,
            &exact,
            source(1).replay_ref(),
            &[],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            history.countries["mod/old"]
                .identities
                .iter()
                .map(|run| (run.first_hour, run.last_hour))
                .collect::<Vec<_>>(),
            [(10, 10), (12, 12)]
        );
        let renamed = &history.countries["mod/new"].identities[0];
        assert_eq!(
            (
                renamed.first_hour,
                renamed.last_hour,
                renamed.slot,
                renamed.country_id
            ),
            (11, 11, 0, 100)
        );
    }

    #[test]
    fn progressive_histories_conflicting_exact_holders_and_aliases_are_refused() {
        let fixture = crate::prepared::series::tests::Fixture::new();
        for (tag, id) in [("mod/a", 200), ("mod/alias", 100)] {
            let exact = BTreeMap::from([
                (("mod/a".into(), "fuel".into()), vec![(10, 11, 0, 100)]),
                ((tag.into(), "capacity".into()), vec![(11, 12, 0, id)]),
            ]);
            assert!(
                histories(
                    &fixture.store,
                    &fixture.key,
                    1,
                    None,
                    &BTreeMap::new(),
                    &exact,
                    source(1).replay_ref(),
                    &[],
                    &AtomicBool::new(false),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn progressive_approximate_clips_viewport_membership_and_preserves_missing_gaps() {
        let mut summary = crate::prepared_chart::ChartSummary {
            from: 3.5,
            to: 12.75,
            width: 10,
            buckets: Vec::new(),
            runs: Some(Vec::new()),
            approximate: Vec::new(),
        };
        let spans = [
            HourSpan {
                first_hour: 0,
                last_hour: 6,
            },
            HourSpan {
                first_hour: 8,
                last_hour: 20,
            },
        ];
        let within = [
            HourSpan {
                first_hour: 4,
                last_hour: 9,
            },
            HourSpan {
                first_hour: 12,
                last_hour: 14,
            },
        ];
        clip_approximate(&mut summary, &spans, Some(&within));
        assert_eq!(
            summary.approximate,
            vec![
                HourSpan {
                    first_hour: 4,
                    last_hour: 6
                },
                HourSpan {
                    first_hour: 8,
                    last_hour: 9
                },
                HourSpan {
                    first_hour: 12,
                    last_hour: 12
                }
            ]
        );
        clip_approximate(&mut summary, &spans, None);
        assert_eq!(
            summary.approximate,
            vec![
                HourSpan {
                    first_hour: 3,
                    last_hour: 6
                },
                HourSpan {
                    first_hour: 8,
                    last_hour: 12
                }
            ]
        );
        clip_approximate(&mut summary, &spans, Some(&[]));
        assert!(summary.approximate.is_empty());
        clip_approximate(&mut summary, &[], None);
        assert!(summary.approximate.is_empty());
    }

    fn source(blocks: i32) -> Source {
        let mut replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![frame(1, 0, Some(0)), frame(1, 1, Some(99))],
            )],
        )
        .unwrap();
        for block in 1..blocks {
            replay = replay
                .append(&[continuation_records(
                    &replay,
                    1,
                    vec![
                        frame(1, block * 2, Some(i64::from(block))),
                        frame(1, block * 2 + 1, Some(99)),
                    ],
                )])
                .unwrap();
        }
        assert_eq!(replay.index.len(), blocks as usize);
        Source::from_replay(
            crate::SourceKey {
                source_id: "progressive".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    #[allow(clippy::type_complexity)]
    fn read(
        source: &Source,
        window: (i32, i32),
        coverage: Option<&Coverage>,
    ) -> (BTreeMap<(String, String), Vec<Run>>, Vec<HourSpan>) {
        tail(
            source,
            1,
            coverage,
            &[TableId::Campaign, TableId::Countries, TableId::CountryLinks],
            &[("GER".into(), "politicalPower".into())],
            window,
            &crate::prepared_measure::sample,
            &AtomicBool::new(false),
        )
        .map(|(runs, approximate, _)| (runs, approximate))
        .unwrap()
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn rough_tails_keep_one_identity_span_while_the_holder_is_unchanged() {
        let source = source(15);
        let coverage = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        };
        let key: (String, String) = ("GER".into(), "politicalPower".into());
        let (_, approximate, identities) = tail(
            &source,
            1,
            Some(&coverage),
            &[TableId::Campaign, TableId::Countries, TableId::CountryLinks],
            std::slice::from_ref(&key),
            (0, 29),
            &crate::prepared_measure::sample,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(approximate.len(), 1);
        assert_eq!(
            identities[&key].len(),
            1,
            "fourteen rough blocks, one holder"
        );
        assert_eq!((identities[&key][0].0, identities[&key][0].1), (2, 29));
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_prefix_and_fourteen_block_tail_are_rough_but_thirteen_block_zoom_is_exact() {
        let source = source(15);
        let coverage = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        };
        let (runs, approximate) = read(&source, (0, 29), Some(&coverage));
        let key = ("GER".into(), "politicalPower".into());
        assert_eq!(runs[&key].len(), 14);
        assert_eq!(runs[&key][0], Run::new(2, 3, 1, true));
        assert_eq!(
            approximate,
            vec![HourSpan {
                first_hour: 2,
                last_hour: 29
            }]
        );
        let (runs, approximate) = read(&source, (4, 29), Some(&coverage));
        assert!(approximate.is_empty());
        assert_eq!(runs[&key][0].value, 2);
        assert_eq!(runs[&key][1].value, 99);
        assert_eq!(runs[&key].len(), 26);
        let complete = Coverage {
            last_hour: 29,
            complete: true,
            ..coverage
        };
        assert!(read(&source, (0, 29), Some(&complete)).0.is_empty());
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn absent_preparation_reads_keyframes_and_cancellation_reads_nothing() {
        let source = source(14);
        let (runs, approximate) = read(&source, (0, 27), None);
        assert_eq!(runs.values().next().unwrap().len(), 14);
        assert_eq!(
            approximate,
            vec![HourSpan {
                first_hour: 0,
                last_hour: 27
            }]
        );
        let error = tail(
            &source,
            1,
            None,
            &[TableId::Countries, TableId::CountryLinks],
            &[],
            (0, 27),
            &crate::prepared_measure::sample,
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
    #[test]
    fn navigation_does_not_invent_a_change_where_the_prepared_prefix_continues() {
        let key = ("opaque".into(), "politicalPower".into());
        let before = Run::new(0, 1, 5, true);
        let prepared = BTreeMap::from([(key.clone(), (Some(2.0), Some(before)))]);
        let tail = BTreeMap::from([(key.clone(), vec![Run::new(2, 5, 5, false)])]);
        let navigation = crate::prepared::series::Navigation {
            from: Some(0.0),
            first_hour: 0.0,
            last_hour: 5.0,
            key: crate::prepared::series::MeasureKey::ArrowRight,
        };
        assert_eq!(
            navigate(
                &prepared,
                &tail,
                std::slice::from_ref(&key),
                &BTreeMap::new(),
                &navigation
            )
            .unwrap(),
            Some(0.0)
        );
        let backwards = crate::prepared::series::Navigation {
            from: Some(4.0),
            key: crate::prepared::series::MeasureKey::ArrowLeft,
            ..navigation
        };
        assert_eq!(
            navigate(
                &prepared,
                &tail,
                std::slice::from_ref(&key),
                &BTreeMap::new(),
                &backwards
            )
            .unwrap(),
            Some(0.0)
        );
        let changed = BTreeMap::from([(key.clone(), vec![Run::new(2, 5, 7, false)])]);
        assert_eq!(
            navigate(
                &prepared,
                &changed,
                std::slice::from_ref(&key),
                &BTreeMap::new(),
                &navigation
            )
            .unwrap(),
            Some(2.0)
        );
    }

    #[test]
    fn exact_samples_preserve_missing_values_and_replacement_holder_boundaries() {
        let mut frames = (0..5)
            .map(|hour| frame(1, hour, Some(7)))
            .collect::<Vec<_>>();
        if let replay::frame::TableView::Complete(replay::rows::TableRows::Countries(rows)) =
            &mut *frames[1].tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = None;
        }
        for decoded in &mut frames[3..] {
            if let replay::frame::TableView::Complete(replay::rows::TableRows::Countries(rows)) =
                &mut *decoded.tables[TableId::Countries.index()]
            {
                rows[0].id = 9002;
            }
            if let replay::frame::TableView::Complete(replay::rows::TableRows::CountryLinks(rows)) =
                &mut *decoded.tables[TableId::CountryLinks.index()]
            {
                rows.push(replay::rows::CountryLinkRow {
                    canonical_slot: 7,
                    id: 9002,
                });
            }
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "gaps".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let (values, approximate, identities) = tail(
            &source,
            1,
            None,
            &[TableId::Campaign, TableId::Countries, TableId::CountryLinks],
            &[("GER".into(), "politicalPower".into())],
            (0, 4),
            &crate::prepared_measure::sample,
            &AtomicBool::new(false),
        )
        .unwrap();
        let key = ("GER".into(), "politicalPower".into());
        assert!(approximate.is_empty());
        assert_eq!(
            values[&key],
            vec![
                Run::new(0, 0, 7, true),
                Run::new(2, 2, 7, true),
                Run::new(3, 4, 7, true)
            ]
        );
        assert_eq!(identities[&key], vec![(0, 2, 7, 9001), (3, 4, 7, 9002)]);
    }

    #[test]
    fn sparse_state_reads_use_only_requested_hours_and_their_distinct_blocks() {
        let source = source(15);
        let hours = [1, 29];
        let key = ("GER".into(), "politicalPower".into());
        let (values, approximate, identities) = state_tail(
            &source,
            1,
            None,
            &[TableId::Campaign, TableId::Countries, TableId::CountryLinks],
            std::slice::from_ref(&key),
            &hours,
            &crate::prepared_measure::sample,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(approximate.is_empty());
        assert_eq!(
            values[&key]
                .iter()
                .map(|run| (run.first_hour, run.last_hour, run.value))
                .collect::<Vec<_>>(),
            vec![(1, 1, 99), (29, 29, 99)]
        );
        assert_eq!(identities[&key], vec![(1, 1, 7, 9001), (29, 29, 7, 9001)]);
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn rough_spans_exclude_recorded_gaps_and_interleaved_branches_do_not_count_toward_zoom() {
        let first = records(1, vec![frame(1, 0, Some(0)), frame(1, 1, Some(99))]);
        let context = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&first)).unwrap();
        let mut segments = vec![first];
        for block in 1..15 {
            if block == 7 {
                segments.push(records(
                    2,
                    vec![frame(2, 12, Some(777)), frame(2, 13, Some(777))],
                ));
            }
            segments.push(continuation_records(
                &context,
                1,
                vec![
                    frame(1, block * 2, Some(i64::from(block))),
                    frame(1, block * 2 + 1, Some(99)),
                ],
            ));
        }
        // Assemble every segment before opening: once an interleaved recording
        // has integrity warnings, the writer correctly refuses further appends.
        let mut replay = Replay::from_records(CAMPAIGN.into(), &segments).unwrap();
        assert_eq!(replay.index.len(), 16);
        replay.gaps.push(replay::branches::Gap {
            branch: 1,
            first_hour: 5,
            last_hour: 5,
            reason: "synthetic gap".into(),
        });
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "branches".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let (values, approximate) = read(&source, (0, 29), None);
        let key = ("GER".into(), "politicalPower".into());
        assert!(
            values[&key]
                .iter()
                .all(|run| !(run.first_hour <= 5 && run.last_hour >= 5))
        );
        assert!(values[&key].iter().all(|run| run.value != 777));
        assert_eq!(
            approximate,
            vec![
                HourSpan {
                    first_hour: 0,
                    last_hour: 4
                },
                HourSpan {
                    first_hour: 6,
                    last_hour: 29
                }
            ]
        );
        assert!(
            read(&source, (4, 29), None).1.is_empty(),
            "only the thirteen branch-one blocks make the exact-zoom bound"
        );
    }
}
