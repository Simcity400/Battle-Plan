//! Transient reads for S families beyond their committed prepared prefix.
use crate::{
    Source,
    metadata::check_cancel,
    prepared::{coverage::Coverage, series::Run},
    products::country_groups::HourSpan,
};
use replay::container::SeekResult;
use replay_data::rows::TableId;
use replay_frame::{
    frame::DecodedFrame, frame::FrameDecoder, frame::RecordedFrame as Frame, frame::TableUpdate,
};
use std::{
    cell::Cell,
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// One frame's values: `(tag, field)` -> (native holder identity, raw value).
pub type Samples = BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>;

/// One stateless projection's last table views and selection within a reply.
/// Callers must depend only on these tables and the requested keys, never on
/// the frame hour or mutable presentation state. Weak identities keep an old
/// allocation distinct even after the decoder releases its rows. The reader
/// still visits every hour, applying ordinary gaps, refusals and break flags.
#[derive(Default)]
pub struct SamplesCache(std::cell::RefCell<Option<CachedSamples>>);
struct CachedSamples {
    branch: u64,
    views: Vec<(usize, replay_frame::frame::ViewIdentity)>,
    requested: Option<Vec<(String, String)>>,
    values: Samples,
}
impl SamplesCache {
    pub fn sample(
        &self,
        frame: &DecodedFrame,
        tables: &[TableId],
        requested: Option<&[(String, String)]>,
        sample: Sample<'_>,
    ) -> io::Result<Samples> {
        let views: Vec<_> = tables
            .iter()
            .map(|table| (table.index(), frame.tables[table.index()].identity()))
            .collect();
        let mut held = self.0.borrow_mut();
        if let Some(previous) = held.as_ref().filter(|previous| {
            previous.branch == frame.branch
                && previous.views == views
                && previous.requested.as_deref() == requested
        }) {
            return Ok(previous.values.clone());
        }
        let values = sample(frame, requested)?;
        *held = Some(CachedSamples {
            branch: frame.branch,
            views,
            requested: requested.map(<[_]>::to_vec),
            values: values.clone(),
        });
        Ok(values)
    }
}

/// A family's stateless per-frame extraction, shared with its observer. Given
/// requested keys it may add the keys they expand to (a country's parts); a
/// family whose extraction needs recorded content captures it in the closure.
pub type Sample<'a> = &'a dyn Fn(&DecodedFrame, Option<&[(String, String)]>) -> io::Result<Samples>;

/// A [`Sample`] also given the hour's recorded updates where the read decoded
/// the hour from them ([`tail_recorded`]); none at a frame read alone (a
/// keyframe, the hour before an exact window).
pub type SampleRecorded<'a> = &'a dyn Fn(
    &DecodedFrame,
    Option<&Recorded<'_>>,
    Option<&[(String, String)]>,
) -> io::Result<Samples>;

/// One hour's recorded updates, as the exact read decoded the hour from them:
/// each read of the block, with the requested tables it selected, at the hour.
pub struct Recorded<'a> {
    reads: Vec<(&'a [TableId], &'a Frame)>,
}

impl Recorded<'_> {
    /// `table`'s recorded update this hour; none when no read selected it.
    pub fn update(&self, table: TableId) -> Option<&TableUpdate> {
        self.reads
            .iter()
            .find(|(tables, _)| tables.contains(&table))
            .map(|(_, frame)| &frame.tables[table.index()])
    }
}

/// Adapter inputs are transient samples plus the branch's real bounds.
pub type AdapterTail<'a> = (
    (i32, i32),
    &'a BTreeMap<(String, String), Vec<Run>>,
    &'a [HourSpan],
    &'a BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
    bool,
);

thread_local! {
    /// While set, sampled reads refuse to decode recorded blocks.
    static WITHOUT_DECODES: Cell<bool> = const { Cell::new(false) };
    /// Whether a sampled read refused to while [`WITHOUT_DECODES`] was set.
    static DECODE_REFUSED: Cell<bool> = const { Cell::new(false) };
}

/// Runs `query` without decoding recorded blocks: a sampled read that would
/// ([`tail`] or [`state_tail`] with blocks to read, [`frame_at`] at an hour the
/// source's [frame cache](super::frames) does not hold) fails instead, and the
/// flag returned says one did. The data service answers a prepared query this
/// way first, so an answer from prepared pages alone never waits behind sampled
/// reads; a query whose flag is set answers nothing and runs again where block
/// decodes are allowed.
pub fn without_block_decodes<T>(query: impl FnOnce() -> T) -> (T, bool) {
    struct Restore(bool, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            WITHOUT_DECODES.set(self.0);
            DECODE_REFUSED.set(self.1 || DECODE_REFUSED.get());
        }
    }
    let _restore = Restore(WITHOUT_DECODES.replace(true), DECODE_REFUSED.replace(false));
    let answer = query();
    (answer, DECODE_REFUSED.get())
}

/// Refuses a block decode inside [`without_block_decodes`]: a product reading
/// recorded blocks itself calls it before it reads one.
pub fn decodes_allowed() -> io::Result<()> {
    if WITHOUT_DECODES.get() {
        DECODE_REFUSED.set(true);
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "This sampled read runs on a sampled-read loader",
        ));
    }
    Ok(())
}

pub(super) fn block_decodes_disabled() -> bool {
    WITHOUT_DECODES.get()
}

/// Numeric pages can answer ahead of their branch-wide identity history. Keep
/// mounted queries following the missing identity gate until both are exact.
pub fn identity_coverage(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    values: Option<&Coverage>,
    identities_complete: bool,
    cancelled: &AtomicBool,
) -> io::Result<Coverage> {
    if identities_complete {
        return crate::prepared::prefix::extent(source, branch, values.cloned());
    }
    let identities = crate::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "country-identities",
        cancelled,
    )?;
    crate::prepared::prefix::extent(
        source,
        branch,
        values
            .zip(identities.as_ref())
            .and_then(|(values, identities)| {
                let first_hour = values.first_hour.max(identities.first_hour);
                let last_hour = values.last_hour.min(identities.last_hour);
                (first_hour <= last_hour).then_some(Coverage {
                    schema: values.schema,
                    first_hour,
                    last_hour,
                    complete: false,
                })
            }),
    )
}

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
    use crate::products::country_history::{
        CountryIdentityRun, RecordedHistories, read_prepared_histories,
    };
    use std::{collections::BTreeSet, sync::Arc};

    check_cancel(cancelled)?;
    // The identities may be prepared on their own beyond this coverage; the
    // accepted prefix is their history through its last hour.
    let mut history = if let Some(coverage) = coverage {
        read_prepared_histories(store, key, branch, Some(coverage.last_hour))?
            .map_or_else(RecordedHistories::default, |history| (*history).clone())
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
        use replay_data::rows::tables;
        check_cancel(cancelled)?;
        let selected = [TableId::Countries, TableId::CountryLinks];
        let keyframe = replay.read_keyframe_recorded(first, &selected)?;
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

/// One reached hour's selected tables, decoded from its block (or held by the
/// source's [frame cache](super::frames)) for a whole-frame state answer; none at
/// an unrecorded hour or inside a recorded gap.
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
    let frame = match super::frames::seek(source, branch, hour, tables, cancelled)? {
        SeekResult::Frame(frame) => Some(frame),
        SeekResult::Gap { .. } => None,
    };
    check_cancel(cancelled)?;
    Ok(frame)
}

/// Recorded hours in a family's bounded look-back, in order. A skipped hour
/// breaks continuity; callers use the same observer as forward preparation.
pub fn read_window(
    source: &Source,
    branch: u64,
    first: i32,
    last: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Vec<DecodedFrame>> {
    check_cancel(cancelled)?;
    super::frames::window(source, branch, first, last, tables, cancelled)
}

/// The latest complete supplying read within `cadence` hours of `hour`.
/// Recorded read slots locate the hour before its rows are reconstructed.
/// Missing hours and unavailable tables stop a carry. A non-read snapshot or
/// delta needs the decoder to validate the carry; unchanged updates do not.
/// Owner-sliced families additionally check their owners' SliceReads.
#[allow(clippy::too_many_arguments)]
pub fn latest_read(
    source: &Source,
    branch: u64,
    hour: i32,
    table: TableId,
    cadence: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Option<Box<DecodedFrame>>> {
    if cadence <= 0 {
        return Err(crate::invalid("A supplying read needs a positive cadence"));
    }
    check_cancel(cancelled)?;
    let Some(block) = source.replay_ref().index.iter().find(|block| {
        block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
    }) else {
        return Ok(None);
    };
    let cache = super::observations::scope(source, "supplying-reads", None);
    let supplying = cache.get(
        block,
        &(hour, table.index(), cadence),
        cancelled,
        |_: &Option<i32>| std::mem::size_of::<Option<i32>>(),
        || supplying_read(source, branch, hour, table, cadence, cancelled),
    )?;
    let Some(hour) = *supplying else {
        return Ok(None);
    };
    let mut wanted = tables.to_vec();
    wanted.push(table);
    wanted.sort_unstable();
    wanted.dedup();
    frame_at(source, branch, hour, &wanted, cancelled)
}

fn supplying_read(
    source: &Source,
    branch: u64,
    hour: i32,
    table: TableId,
    cadence: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<i32>> {
    check_cancel(cancelled)?;
    let replay = source.replay_ref();
    let first = hour.saturating_sub(cadence - 1);
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| {
            block.branch == branch && block.last_hour >= first && block.first_hour <= hour
        })
        .collect();
    blocks.sort_by_key(|block| std::cmp::Reverse(block.first_hour));
    let mut expected = hour;
    let mut validate_carry = table == TableId::Clock;
    let mut supplying = None;
    let consumer = replay.section_consumer(
        blocks
            .first()
            .and_then(|block| replay.block_position(block))
            .unwrap_or(0),
    );
    'search: for block in blocks {
        if let Some(position) = replay.block_position(block) {
            consumer.at(position);
        }
        check_cancel(cancelled)?;
        let Some(reads) =
            super::frames::block(source, block, &[TableId::Campaign, table], cancelled)?
        else {
            // A block the cache cannot join needs the decoder's full checks.
            validate_carry = true;
            supplying = Some(first);
            break;
        };
        let read = reads
            .iter()
            .find(|read| read.tables.contains(&table))
            .expect("selected supplying table");
        for frame in read
            .frames
            .iter()
            .rev()
            .filter(|frame| first <= frame.hour && frame.hour <= hour)
        {
            check_cancel(cancelled)?;
            if frame.hour != expected
                || replay.gaps.iter().any(|gap| {
                    gap.branch == branch
                        && gap.first_hour <= frame.hour
                        && frame.hour <= gap.last_hour
                })
            {
                return Ok(None);
            }
            match &frame.tables[table.index()] {
                TableUpdate::Unavailable { .. }
                | TableUpdate::Incompatible { .. }
                | TableUpdate::Truncated { .. } => return Ok(None),
                _ => {}
            }
            if crate::metadata::slot_read(replay.format, &frame.metrics, table, frame.hour) {
                supplying = Some(frame.hour);
                break 'search;
            }
            if let TableUpdate::Unchanged { baseline } | TableUpdate::Delta { baseline, .. } =
                &frame.tables[table.index()]
                && frame.hour.checked_sub(1) != Some(*baseline)
            {
                return Ok(None);
            }
            // Unchanged updates prove a carry without reconstructing its rows.
            // A snapshot or delta without a read witness needs row validation.
            validate_carry |= !matches!(frame.tables[table.index()], TableUpdate::Unchanged { .. });
            let Some(before) = expected.checked_sub(1) else {
                break 'search;
            };
            expected = before;
        }
    }
    let Some(supplying) = supplying else {
        return Ok(None);
    };
    let frames = read_window(
        source,
        branch,
        supplying,
        if validate_carry { hour } else { supplying },
        &[table],
        cancelled,
    )?;
    let mut expected = if validate_carry { hour } else { supplying };
    for frame in frames.into_iter().rev() {
        if frame.hour != expected
            || !matches!(
                &*frame.tables[table.index()],
                replay_frame::frame::TableView::Complete(_)
            )
        {
            return Ok(None);
        }
        if crate::metadata::slot_read(
            source.replay_ref().format,
            &frame.metrics,
            table,
            frame.hour,
        ) {
            return Ok(Some(frame.hour));
        }
        let Some(before) = expected.checked_sub(1) else {
            break;
        };
        expected = before;
    }
    Ok(None)
}

/// About one year: the owner's ~2 s exact-zoom target. This is not a country or reply cap.
const EXACT_ZOOM_BLOCKS: usize = 13;

/// Stateless series share a block's channels for `cached_requested`, then
/// select `requested`. Cache keys include that extraction selection, so related
/// measures can share a pass without sampling unrequested countries. The sampler
/// must have no side effects; `product` names it independently of the reply shape.
/// Exact windows keep whole block folds where requested, and bounded folds at
/// their edges; wider windows keep keyframe facts.
#[allow(clippy::too_many_arguments)]
pub fn tail_cached(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    cached_requested: &[(String, String)],
    window: (i32, i32),
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    tail_cached_selected(
        source,
        product,
        branch,
        coverage,
        tables,
        requested,
        cached_requested,
        window,
        sample,
        false,
        cancelled,
    )
}

/// Cache a stateless census whose full set of keys is discovered in each
/// frame. Countries first appearing in the tail need no prepared catalogue key.
/// Content-dependent callers bind recorded content before lookup and distinguish
/// non-recorded projection inputs in `product`, including verification choices
/// and input availability. These helpers retain the sampler's numeric facts.
#[allow(clippy::too_many_arguments)]
pub fn tail_all_cached(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    window: (i32, i32),
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    tail_cached_selected(
        source,
        product,
        branch,
        coverage,
        tables,
        &[],
        &[],
        window,
        sample,
        true,
        cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
fn tail_cached_selected(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    cached_requested: &[(String, String)],
    window: (i32, i32),
    sample: Sample<'_>,
    all: bool,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    check_cancel(cancelled)?;
    if coverage.is_some_and(|coverage| coverage.complete) || window.0 > window.1 {
        return Ok(TailData::default());
    }
    let first = coverage.map_or(i64::from(window.0), |coverage| {
        i64::from(window.0).max(i64::from(coverage.last_hour) + 1)
    });
    let blocks: Vec<_> = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| {
            block.branch == branch
                && i64::from(block.last_hour) >= first
                && block.first_hour <= window.1
        })
        .collect();
    if blocks.len() <= EXACT_ZOOM_BLOCKS {
        return cached_recorded_blocks(
            source,
            product,
            branch,
            tables,
            1,
            requested,
            cached_requested,
            (first, window.1),
            blocks,
            &|frame, _, keys| sample(frame, if all { None } else { keys }),
            None,
            true,
            all,
            cancelled,
        );
    }
    let cache = super::observations::scope(source, product, None);
    let mut selected: Vec<_> = tables.iter().map(|table| table.index()).collect();
    selected.sort_unstable();
    selected.dedup();
    let mut keys = cached_requested.to_vec();
    keys.sort();
    keys.dedup();
    let wanted: std::collections::BTreeSet<_> = requested.iter().collect();
    let mut result = TailData::default();
    let replay = source.replay_ref();
    let consumer = replay.section_consumer(
        blocks
            .first()
            .and_then(|block| replay.block_position(block))
            .unwrap_or(0),
    );
    for block in blocks {
        if let Some(position) = replay.block_position(block) {
            consumer.at(position);
        }
        check_cancel(cancelled)?;
        if source.replay_ref().gaps.iter().any(|gap| {
            gap.branch == branch
                && gap.first_hour <= block.first_hour
                && block.first_hour <= gap.last_hour
        }) {
            continue;
        }
        let observed = cache.get(
            block,
            &(&selected, &keys, all),
            cancelled,
            |samples: &Samples| {
                let mut bytes = crate::prepared::prepass::HeldBytes::default();
                bytes.value(samples);
                bytes.total() as usize
            },
            || {
                let keyframe = source.replay_ref().read_keyframe_recorded(block, tables)?;
                let mut decoder = FrameDecoder::for_tables(tables);
                sample(
                    decoder.advance(&keyframe)?,
                    if all { None } else { Some(cached_requested) },
                )
            },
        )?;
        let spans = crate::prepared::prefix::without_gaps(
            source,
            branch,
            i64::from(block.first_hour).max(first) as i32,
            block.last_hour.min(window.1),
        );
        sampled_spans(
            &mut result,
            &spans,
            observed
                .iter()
                .filter(|(key, _)| all || wanted.contains(key)),
        );
    }
    Ok(result)
}

/// State hours use the same immutable exact folds as short summary windows.
/// Samplers may reject request fields, but must have no frame-dependent refusals
/// outside their requested points; use a checked sparse or full state walk otherwise.
#[allow(clippy::too_many_arguments)]
pub fn state_tail_cached(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    cached_requested: &[(String, String)],
    hours: &[i32],
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    state_tail_recorded_bounded(
        source,
        product,
        branch,
        coverage,
        tables,
        1,
        requested,
        cached_requested,
        hours,
        &|frame, _, keys| sample(frame, keys),
        true,
        false,
        cancelled,
    )
}

/// Isolated state values need only their reached frames. Unlike an hourly
/// fold this sampler must be stateless; a held daily value still needs its
/// supplying-read observer. Observations are independent of moving coverage.
#[allow(clippy::too_many_arguments)]
pub fn points_cached(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    hours: &[i32],
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    let cache = super::observations::scope(source, product, None);
    let mut result = TailData::default();
    let mut selected: Vec<_> = tables.iter().map(|table| table.index()).collect();
    selected.sort_unstable();
    selected.dedup();
    for hour in hours
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
    {
        check_cancel(cancelled)?;
        if coverage.is_some_and(|c| c.complete || hour <= c.last_hour) {
            continue;
        }
        let Some(block) = source.replay_ref().index.iter().find(|block| {
            block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
        }) else {
            continue;
        };
        let observed = cache.get_query(
            block,
            &(&selected, requested, hour),
            cancelled,
            |values: &Samples| {
                let mut bytes = crate::prepared::prepass::HeldBytes::default();
                bytes.value(values);
                bytes.total() as usize
            },
            || {
                frame_at(source, branch, hour, tables, cancelled)?
                    .map(|frame| sample(&frame, Some(requested)))
                    .transpose()
                    .map(Option::unwrap_or_default)
            },
        )?;
        let span = HourSpan {
            first_hour: hour,
            last_hour: hour,
        };
        sampled_spans(&mut result, &[span], observed.iter());
    }
    result.1.clear();
    Ok(result)
}

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

/// A stateless sampler with presentation operands captured after its last
/// observation. Re-asks retain each block's fold and those operands together,
/// then replay the operands in block order even when the fold was already held.
/// `finish` takes and clears the sampler's presentation operands; `restore`
/// applies them to the reply. `params` names every input either reads.
#[allow(clippy::too_many_arguments)]
pub fn tail_projected<T: Send + Sync + 'static>(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    window: (i32, i32),
    params: &impl serde::Serialize,
    sample: Sample<'_>,
    finish: impl Fn() -> T,
    restore: impl Fn(&T),
    bytes: impl Fn(&T) -> usize,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    check_cancel(cancelled)?;
    if coverage.is_some_and(|coverage| coverage.complete) || window.0 > window.1 {
        return Ok(TailData::default());
    }
    let first = coverage.map_or(i64::from(window.0), |coverage| {
        i64::from(window.0).max(i64::from(coverage.last_hour) + 1)
    });
    let blocks: Vec<_> = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| {
            block.branch == branch
                && i64::from(block.last_hour) >= first
                && block.first_hour <= window.1
        })
        .collect();
    let exact = blocks.len() <= EXACT_ZOOM_BLOCKS;
    let cache = super::observations::scope(source, product, None).working_set(blocks.len());
    let selected: Vec<_> = tables.iter().map(|table| table.index()).collect();
    let mut result = TailData::default();
    for block in blocks {
        let extent = (
            i64::from(block.first_hour).max(first),
            block.last_hour.min(window.1),
        );
        let observed = cache.get(
            block,
            &(&selected, requested, extent, exact, params),
            cancelled,
            |observed: &(TailData, Option<T>)| {
                tail_bytes(&observed.0)
                    + observed.1.as_ref().map_or(0, &bytes)
                    + std::mem::size_of::<(TailData, Option<T>)>()
            },
            || {
                let previous = finish();
                let sampled = Cell::new(false);
                let tail = sample_blocks_recorded(
                    source,
                    branch,
                    tables,
                    requested,
                    extent,
                    vec![block],
                    &|frame, _, keys| {
                        sampled.set(true);
                        sample(frame, keys)
                    },
                    None,
                    exact,
                    cancelled,
                )?;
                let presentation = sampled.get().then(&finish);
                restore(&previous);
                Ok((tail, presentation))
            },
        )?;
        if let Some(metadata) = &observed.1 {
            restore(metadata);
        }
        for (key, runs) in &observed.0.0 {
            let rows = result.0.entry(key.clone()).or_default();
            for run in runs {
                if let Some(previous) = rows.last_mut().filter(|previous| {
                    exact
                        && !run.break_before
                        && previous.value == run.value
                        && previous.last_hour.checked_add(1) == Some(run.first_hour)
                }) {
                    previous.last_hour = run.last_hour;
                } else {
                    rows.push(*run);
                }
            }
        }
        for (key, spans) in &observed.0.2 {
            let rows = result.2.entry(key.clone()).or_default();
            for &(start, end, slot, id) in spans {
                if let Some(previous) = rows.last_mut().filter(|previous| {
                    previous.1.checked_add(1) == Some(start)
                        && (previous.2, previous.3) == (slot, id)
                }) {
                    previous.1 = end;
                } else {
                    rows.push((start, end, slot, id));
                }
            }
        }
        for span in &observed.0.1 {
            crate::products::country_groups::mark_unobserved(
                &mut result.1,
                span.first_hour,
                span.last_hour,
            );
        }
    }
    Ok(result)
}

/// Exact state hours for the same full census as `tail_all_cached`, with the
/// same request-only refusal contract as `state_tail_cached`.
#[allow(clippy::too_many_arguments)]
pub fn state_tail_all_cached(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    hours: &[i32],
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    state_tail_recorded_bounded(
        source,
        product,
        branch,
        coverage,
        tables,
        1,
        &[],
        &[],
        hours,
        &|frame, _, _| sample(frame, None),
        true,
        true,
        cancelled,
    )
}

/// [`tail`] with each exactly read hour's recorded updates handed to `sample`
/// beside its frame, so a sampler can carry its own state from hour to hour.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tail_recorded(
    source: &Source,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    window: (i32, i32),
    sample: SampleRecorded<'_>,
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
    let exact = blocks.len() <= EXACT_ZOOM_BLOCKS;
    sample_blocks_recorded(
        source,
        branch,
        tables,
        requested,
        (first, window.1),
        blocks,
        sample,
        None,
        exact,
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
        (first, hours.last().copied().unwrap_or(i32::MIN)),
        blocks,
        sample,
        Some(&hours),
        cancelled,
    )
}

/// Stateless state values whose projection has no observable effects at
/// unrequested hours. The preceding hour still supplies each value's break flag.
/// `check` performs every per-hour refusal of the full sampler before any
/// projection is skipped. Projection-dependent refusals need the full state walk.
#[allow(clippy::too_many_arguments)]
pub fn state_tail_sparse(
    source: &Source,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    requested: &[(String, String)],
    hours: &[i32],
    check: &dyn Fn(&DecodedFrame) -> io::Result<()>,
    sample: Sample<'_>,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    state_tail(
        source,
        branch,
        coverage,
        tables,
        requested,
        hours,
        &|frame, keys| {
            check(frame)?;
            if state_point(hours, frame.hour) {
                sample(frame, keys)
            } else {
                Ok(Samples::new())
            }
        },
        cancelled,
    )
}

fn state_point(hours: &[i32], hour: i32) -> bool {
    hours.contains(&hour)
        || hour
            .checked_add(1)
            .is_some_and(|next| hours.contains(&next))
}

/// [`state_tail`] with a sampler that carries its own state from hour to hour
/// ([`SampleRecorded`]): each target block, after its cadence look-back, is
/// decoded in order, so a family read at its own cadence answers a state hour
/// from its last read at or before it, looking back within its read cadence.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn state_tail_recorded(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    cadence: i32,
    requested: &[(String, String)],
    cached_requested: &[(String, String)],
    hours: &[i32],
    sample: SampleRecorded<'_>,
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
)> {
    state_tail_recorded_bounded(
        source,
        product,
        branch,
        coverage,
        tables,
        cadence,
        requested,
        cached_requested,
        hours,
        sample,
        false,
        false,
        cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
fn state_tail_recorded_bounded(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    cadence: i32,
    requested: &[(String, String)],
    cached_requested: &[(String, String)],
    hours: &[i32],
    sample: SampleRecorded<'_>,
    bounded: bool,
    all: bool,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
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
    cached_recorded_blocks(
        source,
        product,
        branch,
        tables,
        cadence,
        requested,
        cached_requested,
        (first, hours.last().copied().unwrap_or(i32::MIN)),
        blocks,
        sample,
        Some(&hours),
        bounded,
        all,
        cancelled,
    )
}

/// Blocks intersecting the supplying-read cadence before this block's first
/// hour. Short recording blocks may split one read period many times.
fn with_predecessors<'a>(
    source: &'a Source,
    branch: u64,
    block: &'a replay::container::BlockIndex,
    cadence: i32,
) -> Vec<&'a replay::container::BlockIndex> {
    let first = block.first_hour.saturating_sub(cadence.saturating_sub(1));
    source
        .replay_ref()
        .index
        .iter()
        .filter(|other| {
            other.branch == branch
                && other.last_hour >= first
                && other.first_hour <= block.first_hour
        })
        .collect()
}

/// [`tail_recorded`] for a family read at its own cadence (a daily read),
/// whose sampler answers only from a read it has seen. A short tail is read
/// exactly, hour by hour, after its cadence look-back, so the read before the
/// tail carries into its first hours. A longer one samples each block after the prefix
/// at its first hour, from its keyframe on, whose sample answers a requested
/// key: the family's first fresh supplying read in the block, exact at its
/// hour, which stands for the block's hours after the prefix, approximate.
/// The look-back is bounded by the family's read cadence, never by a chosen
/// count. A block with no such read, or whose hours are all a recorded gap,
/// is not sampled. `first_sample` must use the observation cache (or enforce
/// `decodes_allowed` itself) so a cache-only request never reads a missing block.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tail_first_read(
    source: &Source,
    product: &'static str,
    branch: u64,
    coverage: Option<&Coverage>,
    tables: &[TableId],
    cadence: i32,
    requested: &[(String, String)],
    cached_requested: &[(String, String)],
    window: (i32, i32),
    sample: SampleRecorded<'_>,
    first_sample: &(dyn Fn(&replay::container::BlockIndex) -> io::Result<Samples> + Sync),
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
    // The supplying cadence look-back keeps each short tail exact.
    if blocks.len() < EXACT_ZOOM_BLOCKS {
        return cached_recorded_blocks(
            source,
            product,
            branch,
            tables,
            cadence,
            requested,
            cached_requested,
            (first, window.1),
            blocks,
            sample,
            None,
            false,
            false,
            cancelled,
        );
    }
    let mut result = TailData::default();
    let consumer = replay.section_consumer(
        blocks
            .first()
            .and_then(|block| replay.block_position(block))
            .unwrap_or(0),
    );
    let first_sample_read = |block: &replay::container::BlockIndex| {
        if let Some(position) = replay.block_position(block) {
            consumer.reading(position);
        }
        first_sample(block)
    };
    let read_ahead =
        std::thread::available_parallelism().map_or(1, |threads| (threads.get() / 2).max(1));
    let mut accept = |block: &replay::container::BlockIndex, observed: Samples| -> io::Result<()> {
        if let Some(position) = replay.block_position(block) {
            consumer.at(position);
        }
        check_cancel(cancelled)?;
        if !observed.values().any(|(_, value)| value.is_some()) {
            return Ok(());
        }
        let start = i64::from(block.first_hour).max(first) as i32;
        let end = block.last_hour.min(window.1);
        let spans = crate::prepared::prefix::without_gaps(source, branch, start, end);
        sampled_spans(&mut result, &spans, observed.iter());
        Ok(())
    };
    if block_decodes_disabled() {
        // The extractor shares the observation cache's miss/busy refusal,
        // so held first reads can answer here without scheduling workers.
        for block in blocks {
            accept(block, first_sample(block)?)?;
        }
    } else {
        dashboard_core::blocks::scan_blocks_by(
            blocks,
            read_ahead,
            cancelled,
            &first_sample_read,
            |reads| {
                for (block, read) in reads {
                    accept(block, read?)?;
                }
                Ok(())
            },
        )?;
    }
    Ok(result)
}

/// One sampled observation stands for each clipped, gap-free span. An unchanged
/// holder stays one identity row across adjacent blocks, while values retain
/// their sampled block boundaries.
#[allow(clippy::type_complexity)]
/// Extend a tail with a keyframe's observations over visibly approximate spans.
pub fn sampled_spans<'a>(
    result: &mut TailData,
    spans: &[HourSpan],
    observed: impl Iterator<Item = (&'a (String, String), &'a (Option<(u32, u32)>, Option<i128>))>,
) {
    for (key, &(identity, value)) in observed {
        if let Some((slot, country_id)) = identity {
            let rows = result.2.entry(key.clone()).or_default();
            for span in spans {
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
            for span in spans {
                result.0.entry(key.clone()).or_default().push(Run::new(
                    span.first_hour,
                    span.last_hour,
                    value,
                    true,
                ));
            }
        }
    }
    for span in spans {
        crate::products::country_groups::mark_unobserved(
            &mut result.1,
            span.first_hour,
            span.last_hour,
        );
    }
}

/// Runs, approximation spans and holder identities extracted from a tail.
pub type TailData = (
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
);
struct BlockSamples(TailData);

fn block_sample_bytes(block: &BlockSamples) -> usize {
    tail_bytes(&block.0)
}

/// Inline and retained allocation capacities of compact tail observations.
pub fn tail_bytes(tail: &TailData) -> usize {
    let (runs, approximate, identities) = tail;
    std::mem::size_of::<TailData>()
        + approximate.capacity() * std::mem::size_of::<HourSpan>()
        + runs
            .iter()
            .map(|((tag, field), values)| {
                2 * std::mem::size_of::<((String, String), Vec<Run>)>()
                    + tag.capacity()
                    + field.capacity()
                    + values.capacity() * std::mem::size_of::<Run>()
            })
            .sum::<usize>()
        + identities
            .iter()
            .map(|((tag, field), values)| {
                2 * std::mem::size_of::<((String, String), Vec<(i32, i32, u32, u32)>)>()
                    + tag.capacity()
                    + field.capacity()
                    + values.capacity() * std::mem::size_of::<(i32, i32, u32, u32)>()
            })
            .sum::<usize>()
}

/// Cache exact block observations, including their supplying-read predecessor,
/// then clip them for each ask. Stateless reads bound a missing edge fold to the
/// request's read window; a held whole-block fold can answer it directly. Daily
/// samplers retain their whole supplying-read folds.
#[allow(clippy::too_many_arguments)]
fn cached_recorded_blocks(
    source: &Source,
    product: &'static str,
    branch: u64,
    tables: &[TableId],
    cadence: i32,
    requested: &[(String, String)],
    cached_requested: &[(String, String)],
    window: (i64, i32),
    blocks: Vec<&replay::container::BlockIndex>,
    sample: SampleRecorded<'_>,
    state_hours: Option<&[i32]>,
    bounded: bool,
    all: bool,
    cancelled: &AtomicBool,
) -> io::Result<TailData> {
    let cache = super::observations::scope(source, product, None).working_set(blocks.len());
    let mut result = TailData::default();
    let mut selected: Vec<_> = tables.iter().map(|table| table.index()).collect();
    selected.sort_unstable();
    selected.dedup();
    let wanted: std::collections::BTreeSet<_> = requested.iter().cloned().collect();
    let mut keys = cached_requested.to_vec();
    keys.sort();
    keys.dedup();
    let replay = source.replay_ref();
    let consumer = replay.section_consumer(
        blocks
            .first()
            .and_then(|block| replay.block_position(block))
            .unwrap_or(0),
    );
    for block in blocks {
        if let Some(position) = replay.block_position(block) {
            consumer.at(position);
        }
        let whole = (&selected, &keys, cadence, all, None::<(i64, i32)>);
        let extent = bounded
            .then_some((
                i64::from(block.first_hour).max(window.0),
                block.last_hour.min(window.1),
            ))
            .filter(|&(first, last)| {
                first != i64::from(block.first_hour) || last != block.last_hour
            });
        let mut held = if extent.is_some() || (bounded && state_hours.is_some()) {
            cache.get_cached::<BlockSamples>(block, &whole, cancelled)?
        } else {
            None
        };
        if held.is_none() && bounded && state_hours.is_some() && extent.is_some() {
            held = cache.get_cached::<BlockSamples>(
                block,
                &(&selected, &keys, cadence, all, extent),
                cancelled,
            )?;
        }
        let observed = match held {
            Some(observed) => observed,
            None if bounded && state_hours.is_some() => {
                // Stateless state reads need only their points and the hour
                // before each point, which supplies the ordinary break flag.
                // A whole fold already held by a summary still takes precedence.
                let hours: Vec<_> = state_hours
                    .unwrap()
                    .iter()
                    .copied()
                    .filter(|&hour| block.first_hour <= hour && hour <= block.last_hour)
                    .collect();
                cache.get(
                    block,
                    &("points", &selected, &keys, all, &hours),
                    cancelled,
                    block_sample_bytes,
                    || {
                        sample_blocks(
                            source,
                            branch,
                            tables,
                            cached_requested,
                            (i64::from(block.first_hour), *hours.last().unwrap()),
                            vec![block],
                            &|frame, requested| {
                                if state_point(&hours, frame.hour) {
                                    sample(frame, None, requested)
                                } else {
                                    Ok(Samples::new())
                                }
                            },
                            Some(&hours),
                            cancelled,
                        )
                        .map(BlockSamples)
                    },
                )?
            }
            None => cache.get(
                block,
                &(&selected, &keys, cadence, all, extent),
                cancelled,
                block_sample_bytes,
                || {
                    // The cadence bounds the look-back; its block count cannot
                    // turn this exact cache entry into a keyframe sample.
                    sample_blocks_recorded(
                        source,
                        branch,
                        tables,
                        cached_requested,
                        extent.unwrap_or((i64::from(block.first_hour), block.last_hour)),
                        with_predecessors(source, branch, block, cadence),
                        sample,
                        None,
                        true,
                        cancelled,
                    )
                    .map(BlockSamples)
                },
            )?,
        };
        let spans: Vec<(i32, i32)> = match state_hours {
            Some(hours) => hours
                .iter()
                .copied()
                .filter(|&hour| {
                    i64::from(hour) >= window.0
                        && block.first_hour <= hour
                        && hour <= block.last_hour
                        && hour <= window.1
                })
                .map(|hour| (hour, hour))
                .collect(),
            None => {
                let first = i64::from(block.first_hour).max(window.0);
                let last = block.last_hour.min(window.1);
                if first > i64::from(last) {
                    Vec::new()
                } else {
                    vec![(first as i32, last)]
                }
            }
        };
        for (key, runs) in &observed.0.0 {
            if !all && !wanted.contains(key) {
                continue;
            }
            for &(first, last) in &spans {
                for run in runs {
                    let start = run.first_hour.max(first);
                    let end = run.last_hour.min(last);
                    if start <= end {
                        let clipped = Run {
                            first_hour: start,
                            last_hour: end,
                            break_before: run.break_before && start == run.first_hour,
                            ..*run
                        };
                        let rows = result.0.entry(key.clone()).or_default();
                        if let Some(previous) = rows.last_mut().filter(|previous| {
                            !clipped.break_before
                                && previous.value == clipped.value
                                && previous.last_hour.checked_add(1) == Some(clipped.first_hour)
                        }) {
                            previous.last_hour = clipped.last_hour;
                        } else {
                            rows.push(clipped);
                        }
                    }
                }
            }
        }
        for (key, identities) in &observed.0.2 {
            if !all && !wanted.contains(key) {
                continue;
            }
            for &(first, last) in &spans {
                for &(start, end, slot, id) in identities {
                    let start = start.max(first);
                    let end = end.min(last);
                    if start <= end {
                        let rows = result.2.entry(key.clone()).or_default();
                        if let Some(previous) = rows.last_mut().filter(|previous| {
                            previous.1.checked_add(1) == Some(start)
                                && (previous.2, previous.3) == (slot, id)
                        }) {
                            previous.1 = end;
                        } else {
                            rows.push((start, end, slot, id));
                        }
                    }
                }
            }
        }
    }
    Ok(result)
}

/// About three blocks of a window are read at once per query: ahead of the
/// decode, without holding many blocks' frames in memory.
const EXACT_READ_AHEAD: usize = 2;

/// [`sample_blocks_recorded`] with a sampler of frames alone.
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
    let exact = state_hours.is_some() || blocks.len() <= EXACT_ZOOM_BLOCKS;
    sample_blocks_recorded(
        source,
        branch,
        tables,
        requested,
        window,
        blocks,
        &|frame, _, requested| sample(frame, requested),
        state_hours,
        exact,
        cancelled,
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sample_blocks_recorded(
    source: &Source,
    branch: u64,
    tables: &[TableId],
    requested: &[(String, String)],
    window: (i64, i32),
    blocks: Vec<&replay::container::BlockIndex>,
    sample: SampleRecorded<'_>,
    state_hours: Option<&[i32]>,
    exact: bool,
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
)> {
    if !blocks.is_empty() {
        decodes_allowed()?;
    }
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
                super::frames::seek(source, branch, before, tables, cancelled)?
        {
            for (key, (identity, value)) in sample(&frame, None, Some(requested))? {
                previous.insert(key, (frame.hour, identity, value));
            }
        }
    }
    let consumer = replay.section_consumer(
        blocks
            .first()
            .and_then(|block| replay.block_position(block))
            .unwrap_or(0),
    );
    if exact {
        // Blocks are read ahead on spare cores (or found in the source's frame
        // cache) while their hours are decoded and sampled here, in block order.
        let read =
            |block: &replay::container::BlockIndex| -> io::Result<Vec<super::frames::BlockRead>> {
                if let Some(position) = replay.block_position(block) {
                    consumer.reading(position);
                }
                Ok(
                    match super::frames::block(source, block, tables, cancelled)? {
                        Some(reads) => reads,
                        None => {
                            let mut selected = tables.to_vec();
                            selected.sort_unstable();
                            selected.dedup();
                            vec![super::frames::BlockRead {
                                tables: selected,
                                frames: Arc::new(
                                    replay.read_block_recorded_selected(block, tables)?,
                                ),
                            }]
                        }
                    },
                )
            };
        dashboard_core::blocks::scan_blocks_by(
            blocks,
            EXACT_READ_AHEAD,
            cancelled,
            &read,
            |reads| {
                for (block, read) in reads {
                    if let Some(position) = replay.block_position(block) {
                        consumer.at(position);
                    }
                    check_cancel(cancelled)?;
                    let start = i64::from(block.first_hour).max(first) as i32;
                    let end = block.last_hour.min(window.1);
                    let reads = read?;
                    let mut decoders = reads
                        .iter()
                        .map(|read| {
                            let selected = tables
                                .iter()
                                .copied()
                                .filter(|table| read.tables.binary_search(table).is_ok())
                                .collect::<Vec<_>>();
                            (
                                FrameDecoder::for_tables(&selected),
                                selected,
                                read.frames.as_slice(),
                            )
                        })
                        .collect::<Vec<_>>();
                    let hours = decoders.first().map_or(0, |(_, _, frames)| frames.len());
                    for position in 0..hours {
                        check_cancel(cancelled)?;
                        let decoded = &advance(&mut decoders, position)?;
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
                        let recorded = Recorded {
                            reads: decoders
                                .iter()
                                .map(|(_, selected, frames)| {
                                    (selected.as_slice(), &frames[position])
                                })
                                .collect(),
                        };
                        let observed = sample(decoded, Some(&recorded), Some(requested))?;
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
                                let contiguous = previous.get(&key).is_some_and(
                                    |&(hour, before, previous_value)| {
                                        hour.checked_add(1) == Some(decoded.hour)
                                            && before == identity
                                            && previous_value.is_some()
                                    },
                                );
                                let rows = runs.entry(key.clone()).or_default();
                                if let Some(last) = rows.last_mut().filter(|run| {
                                    contiguous
                                        && run.value == value
                                        && run.last_hour.checked_add(1) == Some(decoded.hour)
                                }) {
                                    last.last_hour = decoded.hour;
                                } else {
                                    rows.push(Run::new(
                                        decoded.hour,
                                        decoded.hour,
                                        value,
                                        !contiguous,
                                    ));
                                }
                            }
                            previous.insert(key, (decoded.hour, identity, value));
                        }
                    }
                }
                Ok(())
            },
        )?;
        return Ok((runs, approximate, identities));
    }
    for block in blocks {
        if let Some(position) = replay.block_position(block) {
            consumer.at(position);
        }
        check_cancel(cancelled)?;
        let start = i64::from(block.first_hour).max(first) as i32;
        let end = block.last_hour.min(window.1);
        // Only the block's keyframe sections are read: no decode through the block.
        let keyframe = replay.read_keyframe_recorded(block, tables)?;
        let mut decoder = FrameDecoder::for_tables(tables);
        let frame = decoder.advance(&keyframe)?;
        let spans = crate::prepared::prefix::without_gaps(source, branch, start, end);
        if replay.gaps.iter().any(|gap| {
            gap.branch == branch && gap.first_hour <= frame.hour && gap.last_hour >= frame.hour
        }) {
            continue;
        }
        for (key, (identity, value)) in sample(frame, None, Some(requested))? {
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
            crate::products::country_groups::mark_unobserved(
                &mut approximate,
                span.first_hour,
                span.last_hour,
            );
        }
    }
    Ok((runs, approximate, identities))
}

/// The hour at `position` of a block held as several reads (each with its
/// decoder and the request's tables it holds), decoded by each read's decoder
/// and joined: each table from the read holding it, every other table
/// unselected as a decoder of the request's tables alone leaves it. Each
/// table's view depends only on its own updates, so the joined frame is that
/// one decoder's frame. The caller drops it before the next hour, so decoders
/// keep applying their deltas in place.
fn advance(
    decoders: &mut [(FrameDecoder, Vec<TableId>, &[Frame])],
    position: usize,
) -> io::Result<DecodedFrame> {
    let mut joined: Option<DecodedFrame> = None;
    for (decoder, selected, frames) in decoders.iter_mut() {
        let decoded = decoder.advance(&frames[position])?;
        match &mut joined {
            None => joined = Some(decoded.clone()),
            Some(joined) => {
                for table in selected.iter() {
                    joined.tables[table.index()] = decoded.tables[table.index()].clone();
                }
            }
        }
    }
    joined.ok_or_else(|| crate::invalid("A block read holds no frames"))
}

/// The exact and rough reads as they were before shared block reads and
/// read-ahead: every block read here, one at a time. Parity tests compare
/// [`sample_blocks`] with it.
#[cfg(test)]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sample_blocks_serial(
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
    let exact = state_hours.is_some() || blocks.len() <= EXACT_ZOOM_BLOCKS;
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
            let keyframe = replay.read_keyframe_recorded(block, tables)?;
            let mut decoder = FrameDecoder::for_tables(tables);
            let frame = decoder.advance(&keyframe)?;
            let spans = crate::prepared::prefix::without_gaps(source, branch, start, end);
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
                crate::products::country_groups::mark_unobserved(
                    &mut approximate,
                    span.first_hour,
                    span.last_hour,
                );
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

    #[test]
    fn latest_reads_are_bounded_shared_and_stop_at_missing_hours_or_tables() {
        use crate::prepared::query::frames;
        use crate::test_fixtures::file_source;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let flag = AtomicBool::new(false);
        let table = TableId::CountryIntel;
        for blocks in [2, 12] {
            let make = |hour| {
                let mut frame = frame(1, hour, None);
                *frame.tables[table.index()] =
                    TableView::Complete(TableRows::CountryIntel(Vec::new()));
                frame.metrics.table_ns[table.index()] = u64::from(hour % 24 == 0);
                frame
            };
            let mut recording = crate::test_fixtures::replay((0..24).map(make).collect());
            for block in 1..blocks {
                recording = recording
                    .append(&[continuation_records(
                        &recording,
                        1,
                        (block * 24..block * 24 + 24).map(make).collect(),
                    )])
                    .unwrap();
            }
            let (source, root) = file_source(recording);
            let at = (blocks - 1) * 24 + 5;
            let read = latest_read(&source, 1, at, table, 24, &[table], &flag)
                .unwrap()
                .unwrap();
            assert_eq!(read.hour, at - 5);
            assert_eq!(
                frames::reads(&source),
                (1, 0),
                "read slots find the supplier in the newest block"
            );
            assert_eq!(
                frames::entries(&source),
                2,
                "one raw block and only its supplying hour"
            );
            let (target, refused) =
                without_block_decodes(|| frame_at(&source, 1, at, &[table], &flag));
            assert!(refused, "the carried target was not reconstructed");
            assert_eq!(target.unwrap_err().kind(), io::ErrorKind::WouldBlock);
            let (cached, refused) =
                without_block_decodes(|| latest_read(&source, 1, at, table, 24, &[table], &flag));
            assert!(!refused);
            assert_eq!(cached.unwrap().unwrap().hour, read.hour);
            let (bytes, bound, computations) =
                super::super::observations::usage(&source, "supplying-reads");
            assert!(bytes > 0 && bytes <= bound);
            assert_eq!(computations, 1, "a cached supplying lookup is not repeated");
            let first = frames::reads(&source);
            assert_eq!(
                latest_read(&source, 1, at, table, 24, &[table], &flag)
                    .unwrap()
                    .unwrap()
                    .hour,
                read.hour
            );
            assert_eq!(
                frames::reads(&source),
                first,
                "the same hour is already reconstructed"
            );
            let SeekResult::Frame(expected) = source
                .replay_ref()
                .seek_selected(1, at, Some(&[table]))
                .unwrap()
            else {
                panic!("a recorded hour");
            };
            assert_eq!(
                *frame_at(&source, 1, at, &[table], &flag).unwrap().unwrap(),
                *expected
            );
            assert_eq!(frames::reads(&source), (first.0, first.1 + 1));
            assert!(frames::accounted(&source));
            drop(source);
            let _ = std::fs::remove_dir_all(root);
        }
        let make = |hour, known| {
            let mut frame = frame(1, hour, None);
            if known {
                *frame.tables[table.index()] =
                    TableView::Complete(TableRows::CountryIntel(Vec::new()));
            }
            frame.metrics.table_ns[table.index()] = u64::from(hour == 0);
            frame
        };
        for missing in [false, true] {
            let recording =
                crate::test_fixtures::replay(vec![make(0, true), make(1, !missing), make(3, true)]);
            let (source, root) = file_source(recording);
            assert!(
                latest_read(&source, 1, 3, table, 24, &[table], &flag)
                    .unwrap()
                    .is_none(),
                "a gap ends the carry"
            );
            if missing {
                assert!(
                    latest_read(&source, 1, 1, table, 24, &[table], &flag)
                        .unwrap()
                        .is_none()
                );
            }
            drop(source);
            let _ = std::fs::remove_dir_all(root);
        }
        let mut fresh = make(3, true);
        fresh.metrics.table_ns[table.index()] = 1;
        let (source, root) = file_source(crate::test_fixtures::replay(vec![make(0, true), fresh]));
        assert_eq!(
            latest_read(&source, 1, 3, table, 24, &[table], &flag)
                .unwrap()
                .unwrap()
                .hour,
            3,
            "a fresh supplying read does not carry across the preceding gap"
        );
        drop(source);
        let _ = std::fs::remove_dir_all(root);
        let (source, root) = file_source(crate::test_fixtures::replay(
            (0..=24).map(|hour| make(hour, true)).collect(),
        ));
        assert!(
            latest_read(&source, 1, 24, table, 24, &[table], &flag)
                .unwrap()
                .is_none(),
            "a missed read expires at the family's cadence"
        );
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn supplying_reads_across_keyframes_validate_the_carried_snapshots() {
        use crate::test_fixtures::file_source;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let table = TableId::CountryIntel;
        let make = |hour| {
            let mut frame = frame(1, hour, None);
            *frame.tables[table.index()] = TableView::Complete(TableRows::CountryIntel(Vec::new()));
            frame.metrics.table_ns[table.index()] = u64::from(hour == 20);
            frame
        };
        let recording = crate::test_fixtures::replay((0..24).map(make).collect());
        let recording = recording
            .append(&[continuation_records(
                &recording,
                1,
                (24..28).map(make).collect(),
            )])
            .unwrap();
        let (source, root) = file_source(recording);
        let flag = AtomicBool::new(false);
        let read = latest_read(&source, 1, 26, table, 24, &[table], &flag)
            .unwrap()
            .unwrap();
        let SeekResult::Frame(expected) = source
            .replay_ref()
            .seek_selected(1, 20, Some(&[table]))
            .unwrap()
        else {
            panic!("a recorded supplying hour");
        };
        assert_eq!(read, expected);
        assert_eq!(super::super::frames::reads(&source), (2, 0));
        let before = super::super::frames::reads(&source);
        assert_eq!(
            latest_read(&source, 1, 26, table, 24, &[table], &flag)
                .unwrap()
                .unwrap(),
            expected
        );
        assert_eq!(super::super::frames::reads(&source), before);
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn window_reads_join_seek_selections_and_keep_every_hour_under_eviction() {
        use super::super::frames;
        let source = varied_source();
        let flag = AtomicBool::new(false);
        frame_at(&source, 1, 5, &[TableId::Countries], &flag).unwrap();
        for tables in [
            crate::prepared_measure::TABLES,
            &TableId::ALL,
            &[TableId::Countries][..],
        ] {
            let actual = read_window(&source, 1, 4, 7, tables, &flag).unwrap();
            assert_eq!(actual.len(), 4);
            for frame in actual {
                let SeekResult::Frame(expected) = source
                    .replay_ref()
                    .seek_selected(1, frame.hour, Some(tables))
                    .unwrap()
                else {
                    panic!("a recorded hour");
                };
                assert_eq!(frame, *expected);
                assert!(
                    frame
                        .tables
                        .iter()
                        .zip(&expected.tables)
                        .all(|(a, b)| a.sampled_at() == b.sampled_at())
                );
            }
            assert!(frames::accounted(&source));
        }
        frames::set_budget(&source, 0);
        let actual = read_window(&source, 1, 4, 7, crate::prepared_measure::TABLES, &flag).unwrap();
        assert_eq!(
            actual.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
            [4, 5, 6, 7]
        );
        assert_eq!(frames::held_bytes(&source), 0);
        assert!(frames::accounted(&source));
    }

    #[test]
    fn a_window_waiting_for_one_hour_does_not_block_another_window() {
        use super::super::frames;
        use std::sync::mpsc::channel;
        use std::time::Duration;
        let source = varied_source();
        let flag = AtomicBool::new(false);
        let tables = crate::prepared_measure::TABLES;
        let (source, flag) = (&source, &flag);
        std::thread::scope(|scope| {
            let held = frames::hold_hour(source, 1, 5);
            let (ready, started) = channel();
            scope.spawn(move || {
                let releases = std::cell::Cell::new(0);
                frames::ON_RELEASE.set(Some(Box::new(move || {
                    releases.set(releases.get() + 1);
                    if releases.get() == 2 {
                        ready.send(()).unwrap();
                    }
                })));
                let result = read_window(source, 1, 4, 5, tables, flag);
                frames::ON_RELEASE.set(None);
                result.unwrap();
            });
            let started = started.recv_timeout(Duration::from_secs(10));
            let (sent, answers) = channel();
            scope.spawn(move || {
                sent.send(read_window(source, 1, 18, 19, tables, flag))
                    .unwrap();
            });
            let answer = answers.recv_timeout(Duration::from_secs(10));
            drop(held);
            started.expect("the first window did not publish its first hour");
            let answer = answer
                .expect("a waiting window serialized another block's window")
                .unwrap();
            assert_eq!(
                answer.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
                [18, 19]
            );
        });
        assert!(frames::accounted(source));
    }

    fn renamed_seed_recording() -> Replay {
        fn named(branch: u64, hour: i32, tag: &[u8]) -> DecodedFrame {
            let mut frame = frame(branch, hour, None);
            if let replay_frame::frame::TableView::Complete(
                replay_data::rows::TableRows::Countries(rows),
            ) = &mut *frame.tables[TableId::Countries.index()]
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
        let keyframe = replay.read_keyframe_recorded(first, &selected).unwrap();
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
            replay.read_keyframe_recorded(first, &selected).is_err(),
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

    #[test]
    fn stateless_state_projects_only_points_and_predecessors_and_retains_them() {
        let source = varied_source();
        let flag = AtomicBool::new(false);
        let tables = crate::prepared_measure::TABLES;
        let calls = Cell::new(0);
        let sample = |frame: &DecodedFrame, keys: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            crate::prepared_measure::sample(frame, keys)
        };
        let key = [("GER".to_owned(), "politicalPower".to_owned())];
        let hours = [3, 7, 14, 19];
        let expected = sample_blocks_serial(
            &source,
            1,
            tables,
            &key,
            (i64::from(i32::MIN), 19),
            source
                .replay_ref()
                .index
                .iter()
                .filter(|block| {
                    hours
                        .iter()
                        .any(|hour| block.first_hour <= *hour && *hour <= block.last_hour)
                })
                .collect(),
            &crate::prepared_measure::sample,
            Some(&hours),
            &flag,
        )
        .unwrap();
        let checked = Cell::new(0);
        let sparse = state_tail_sparse(
            &source,
            1,
            None,
            tables,
            &key,
            &hours,
            &|_| {
                checked.set(checked.get() + 1);
                Ok(())
            },
            &sample,
            &flag,
        )
        .unwrap();
        assert_eq!(checked.get(), 15, "all visited non-gap hours were checked");
        assert_eq!(sparse, expected);
        assert_eq!(
            calls.get(),
            7,
            "only requested points and non-gap predecessors"
        );
        calls.set(0);
        let actual = state_tail_cached(
            &source,
            "test-sparse-state",
            1,
            None,
            tables,
            &key,
            &key,
            &hours,
            &sample,
            &flag,
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(calls.get(), 7, "four points, three non-gap predecessors");
        let before = calls.get();
        let coverage = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        };
        let (again, refused) = without_block_decodes(|| {
            state_tail_cached(
                &source,
                "test-sparse-state",
                1,
                Some(&coverage),
                tables,
                &key,
                &key,
                &hours,
                &sample,
                &flag,
            )
        });
        assert!(!refused);
        assert_eq!(again.unwrap(), expected);
        assert_eq!(calls.get(), before);
        let (bytes, bound, _) = super::super::observations::usage(&source, "test-sparse-state");
        assert!(bytes > 0 && bytes <= bound);
    }

    #[test]
    fn sparse_state_returns_a_check_refusal_before_any_requested_projection() {
        let source = varied_source();
        let flag = AtomicBool::new(false);
        let calls = Cell::new(0);
        let sample = |frame: &DecodedFrame, keys: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            crate::prepared_measure::sample(frame, keys)
        };
        let error = state_tail_sparse(
            &source,
            1,
            None,
            crate::prepared_measure::TABLES,
            &[],
            &[3],
            &|frame| {
                if frame.hour == 1 {
                    Err(io::Error::other("intermediate sampler refusal"))
                } else {
                    Ok(())
                }
            },
            &sample,
            &flag,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "intermediate sampler refusal");
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn projected_tails_reuse_unchanged_blocks_and_restore_metadata_with_byte_parity() {
        let source = varied_source();
        let flag = AtomicBool::new(false);
        let tables = crate::prepared_measure::TABLES;
        let calls = Cell::new(0);
        let metadata = Cell::new(None);
        let sample = |frame: &DecodedFrame, keys: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            metadata.set(Some(frame.hour));
            crate::prepared_measure::sample(frame, keys)
        };
        let key = [("GER".to_owned(), "politicalPower".to_owned())];
        let ask = |coverage: Option<&Coverage>| {
            tail_projected(
                &source,
                "test-projected-tail",
                1,
                coverage,
                tables,
                &key,
                (0, 19),
                &(),
                &sample,
                || metadata.take(),
                |held| metadata.set(*held),
                |_| std::mem::size_of::<Option<i32>>(),
                &flag,
            )
            .unwrap()
        };
        for through in [None, Some(1), Some(3), Some(5), Some(12)] {
            let coverage = through.map(|last_hour| Coverage {
                schema: 0,
                first_hour: 0,
                last_hour,
                complete: false,
            });
            let actual = ask(coverage.as_ref());
            assert_eq!(metadata.get(), Some(19));
            let expected = tail(
                &source,
                1,
                coverage.as_ref(),
                tables,
                &key,
                (0, 19),
                &crate::prepared_measure::sample,
                &flag,
            )
            .unwrap();
            assert_eq!(actual, expected, "prefix {through:?}");
            let encode = |tail: &TailData| {
                serde_json::to_vec(&(
                    tail.0.iter().collect::<Vec<_>>(),
                    &tail.1,
                    tail.2.iter().collect::<Vec<_>>(),
                ))
                .unwrap()
            };
            assert_eq!(encode(&actual), encode(&expected));
            let before = calls.get();
            metadata.set(None);
            let (again, refused) = without_block_decodes(|| ask(coverage.as_ref()));
            assert!(!refused);
            assert_eq!(again, expected);
            assert_eq!(metadata.get(), Some(19));
            assert_eq!(calls.get(), before, "held folds were sampled again");
        }
        let (bytes, bound, computations) =
            super::super::observations::usage(&source, "test-projected-tail");
        assert!(bytes > 0 && bytes <= bound);
        assert_eq!(
            computations, 8,
            "five initial blocks and three clipped boundary blocks"
        );
    }

    #[test]
    fn projected_tails_keep_the_original_keyframe_to_exact_transition() {
        let source = source(14);
        let flag = AtomicBool::new(false);
        let tables = crate::prepared_measure::TABLES;
        let key = [("GER".to_owned(), "politicalPower".to_owned())];
        for through in [None, Some(1)] {
            let coverage = through.map(|last_hour| Coverage {
                schema: 0,
                first_hour: 0,
                last_hour,
                complete: false,
            });
            let actual = tail_projected(
                &source,
                "test-projected-transition",
                1,
                coverage.as_ref(),
                tables,
                &key,
                (0, 27),
                &(),
                &crate::prepared_measure::sample,
                || (),
                |_: &()| {},
                |_| 0,
                &flag,
            )
            .unwrap();
            assert_eq!(
                actual,
                tail(
                    &source,
                    1,
                    coverage.as_ref(),
                    tables,
                    &key,
                    (0, 27),
                    &crate::prepared_measure::sample,
                    &flag,
                )
                .unwrap()
            );
            assert_eq!(actual.1.is_empty(), through.is_some());
        }
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
    fn full_census_keyframes_keep_their_marks_until_the_exact_zoom_transition() {
        let source = source(15);
        let flag = AtomicBool::new(false);
        let calls = Cell::new(0);
        let sample = |frame: &DecodedFrame, _: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            crate::prepared_measure::sample(frame, None)
        };
        let oracle = |frame: &DecodedFrame, _: Option<&[(String, String)]>| {
            crate::prepared_measure::sample(frame, None)
        };
        let tables = crate::prepared_measure::TABLES;
        for through in [-1, 0, 1, 3] {
            let prefix = (through >= 0).then_some(Coverage {
                schema: 0,
                first_hour: 0,
                last_hour: through,
                complete: false,
            });
            let actual = tail_all_cached(
                &source,
                "test-census-zoom",
                1,
                prefix.as_ref(),
                tables,
                (0, 29),
                &sample,
                &flag,
            )
            .unwrap();
            assert_eq!(
                actual,
                tail(
                    &source,
                    1,
                    prefix.as_ref(),
                    tables,
                    &[],
                    (0, 29),
                    &oracle,
                    &flag
                )
                .unwrap()
            );
            assert_eq!(actual.1.is_empty(), through == 3);
            if through < 3 {
                assert_eq!(calls.get(), 15);
            }
        }
    }

    #[test]
    fn full_census_folds_reuse_discovered_keys_for_states_and_moving_prefixes() {
        let source = source(2);
        let flag = AtomicBool::new(false);
        let calls = Cell::new(0);
        let census = |frame: &DecodedFrame, _: Option<&[(String, String)]>| {
            let mut values = crate::prepared_measure::sample(frame, None)?;
            // A tail-only alias is discovered by the census, not supplied by
            // a prepared catalogue or by the request's country selection.
            if frame.hour >= 2 {
                values.insert(
                    ("NEW".into(), "politicalPower".into()),
                    (Some((77, 88)), Some(123)),
                );
            }
            Ok(values)
        };
        let sample = |frame: &DecodedFrame, keys: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            census(frame, keys)
        };
        let tables = crate::prepared_measure::TABLES;
        let cold = tail_all_cached(
            &source,
            "test-census",
            1,
            None,
            tables,
            (0, 3),
            &sample,
            &flag,
        )
        .unwrap();
        assert!(
            cold.0
                .contains_key(&("NEW".into(), "politicalPower".into()))
        );
        assert_eq!(
            cold,
            tail(&source, 1, None, tables, &[], (0, 3), &census, &flag).unwrap()
        );
        let computed = calls.get();
        for through in [0, 1, 2] {
            let prefix = Coverage {
                schema: 0,
                first_hour: 0,
                last_hour: through,
                complete: false,
            };
            let (actual, refused) = without_block_decodes(|| {
                tail_all_cached(
                    &source,
                    "test-census",
                    1,
                    Some(&prefix),
                    tables,
                    (0, 3),
                    &sample,
                    &flag,
                )
            });
            assert!(!refused);
            assert_eq!(
                actual.unwrap(),
                tail(
                    &source,
                    1,
                    Some(&prefix),
                    tables,
                    &[],
                    (0, 3),
                    &census,
                    &flag
                )
                .unwrap()
            );
        }
        for hours in [[0, 2], [1, 3]] {
            let actual = state_tail_all_cached(
                &source,
                "test-census",
                1,
                None,
                tables,
                &hours,
                &sample,
                &flag,
            )
            .unwrap();
            assert_eq!(
                actual,
                state_tail(&source, 1, None, tables, &[], &hours, &census, &flag).unwrap()
            );
        }
        assert_eq!(calls.get(), computed);
    }

    #[test]
    fn stateless_exact_folds_are_shared_by_moving_prefixes_channels_and_state_hours() {
        let source = source(2);
        let flag = AtomicBool::new(false);
        let calls = Cell::new(0);
        let sample = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            crate::prepared_measure::sample(frame, requested)
        };
        let tables = crate::prepared_measure::TABLES;
        let key = ("GER".to_owned(), "politicalPower".to_owned());
        let cached = [key.clone(), ("GER".to_owned(), "commandPower".to_owned())];
        let ask = |coverage: Option<&Coverage>, window| {
            tail_cached(
                &source,
                "test-stateless",
                1,
                coverage,
                tables,
                std::slice::from_ref(&key),
                &cached,
                window,
                &sample,
                &flag,
            )
            .unwrap()
        };
        let cold = ask(None, (0, 3));
        assert_eq!(
            cold,
            tail(
                &source,
                1,
                None,
                tables,
                std::slice::from_ref(&key),
                (0, 3),
                &crate::prepared_measure::sample,
                &flag,
            )
            .unwrap()
        );
        let computed = calls.get();
        for through in [0, 1, 2] {
            let prefix = Coverage {
                schema: 0,
                first_hour: 0,
                last_hour: through,
                complete: false,
            };
            assert_eq!(
                ask(Some(&prefix), (0, 3)),
                tail(
                    &source,
                    1,
                    Some(&prefix),
                    tables,
                    std::slice::from_ref(&key),
                    (0, 3),
                    &crate::prepared_measure::sample,
                    &flag,
                )
                .unwrap()
            );
            assert_eq!(calls.get(), computed, "only the clipping boundary moved");
        }
        let other = [("GER".to_owned(), "commandPower".to_owned())];
        for hours in [[0, 2], [1, 3]] {
            let (actual, refused) = without_block_decodes(|| {
                state_tail_cached(
                    &source,
                    "test-stateless",
                    1,
                    None,
                    tables,
                    &other,
                    &cached,
                    &hours,
                    &sample,
                    &flag,
                )
            });
            assert!(!refused, "the same blocks already hold every channel");
            assert_eq!(
                actual.unwrap(),
                state_tail(
                    &source,
                    1,
                    None,
                    tables,
                    &other,
                    &hours,
                    &crate::prepared_measure::sample,
                    &flag,
                )
                .unwrap()
            );
        }
        assert_eq!(calls.get(), computed);
        let (bytes, bound, entries) = super::super::observations::usage(&source, "test-stateless");
        assert!(bytes > 0 && bytes <= bound);
        assert_eq!(entries, 2);
    }

    #[test]
    fn cached_folds_do_not_refuse_an_hour_because_a_later_sample_fails() {
        let source = source(1);
        let flag = AtomicBool::new(false);
        let keys = [("GER".to_owned(), "politicalPower".to_owned())];
        let calls = Cell::new(0);
        let sample = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            if frame.hour == 1 {
                return Err(io::Error::other("later sample"));
            }
            crate::prepared_measure::sample(frame, requested)
        };
        let tables = crate::prepared_measure::TABLES;
        let expected = tail(&source, 1, None, tables, &keys, (0, 0), &sample, &flag).unwrap();
        calls.set(0);
        assert_eq!(
            tail_cached(
                &source,
                "test-narrow-error",
                1,
                None,
                tables,
                &keys,
                &keys,
                (0, 0),
                &sample,
                &flag,
            )
            .unwrap(),
            expected
        );
        assert_eq!(calls.get(), 1, "the later sample was never evaluated");
        assert_eq!(
            state_tail_cached(
                &source,
                "test-narrow-error",
                1,
                None,
                tables,
                &keys,
                &keys,
                &[0],
                &sample,
                &flag,
            )
            .unwrap(),
            expected
        );
        assert_eq!(calls.get(), 1, "state reuses the successful bounded fold");
        calls.set(0);
        assert!(
            tail_cached(
                &source,
                "test-narrow-error",
                1,
                None,
                tables,
                &keys,
                &keys,
                (0, 1),
                &sample,
                &flag,
            )
            .is_err()
        );
        assert_eq!(
            calls.get(),
            2,
            "an in-window failure is returned without retrying"
        );
    }

    #[test]
    fn cached_failures_in_requested_blocks_are_propagated_once() {
        let source = source(2);
        let flag = AtomicBool::new(false);
        let keys = [("GER".to_owned(), "politicalPower".to_owned())];
        for kind in [
            io::ErrorKind::Other,
            io::ErrorKind::InvalidData,
            io::ErrorKind::UnexpectedEof,
        ] {
            let failures = Cell::new(0);
            let sample = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
                if frame.hour == 2 {
                    failures.set(failures.get() + 1);
                    return Err(io::Error::new(kind, "requested sample"));
                }
                crate::prepared_measure::sample(frame, requested)
            };
            let tables = crate::prepared_measure::TABLES;
            let error = tail_cached(
                &source,
                "test-failure-tail",
                1,
                None,
                tables,
                &keys,
                &keys,
                (0, 3),
                &sample,
                &flag,
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), "requested sample");
            assert_eq!(failures.get(), 1);
            failures.set(0);
            let error = state_tail_cached(
                &source,
                "test-failure-state",
                1,
                None,
                tables,
                &keys,
                &keys,
                &[0, 3],
                &sample,
                &flag,
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), "requested sample");
            assert_eq!(failures.get(), 1);
        }
    }

    #[test]
    fn stateless_keyframes_are_shared_until_exact_zoom_replaces_them() {
        let source = source(15);
        let flag = AtomicBool::new(false);
        let calls = Cell::new(0);
        let sample = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            crate::prepared_measure::sample(frame, requested)
        };
        let key = [("GER".to_owned(), "politicalPower".to_owned())];
        let tables = crate::prepared_measure::TABLES;
        let cached = [
            key[0].clone(),
            ("GER".to_owned(), "commandPower".to_owned()),
        ];
        let ask = |prefix: Option<&Coverage>, keys: &[(String, String)]| {
            tail_cached(
                &source,
                "test-stateless-rough",
                1,
                prefix,
                tables,
                keys,
                &cached,
                (0, 29),
                &sample,
                &flag,
            )
            .unwrap()
        };
        let cold = ask(None, &key);
        assert_eq!(
            cold,
            tail(
                &source,
                1,
                None,
                tables,
                &key,
                (0, 29),
                &crate::prepared_measure::sample,
                &flag,
            )
            .unwrap()
        );
        assert!(!cold.1.is_empty());
        assert_eq!(calls.get(), 15);
        let other = [("GER".to_owned(), "commandPower".to_owned())];
        let mut prefix = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 0,
            complete: false,
        };
        for keys in [&key, &other] {
            assert_eq!(
                ask(Some(&prefix), keys),
                tail(
                    &source,
                    1,
                    Some(&prefix),
                    tables,
                    keys,
                    (0, 29),
                    &crate::prepared_measure::sample,
                    &flag,
                )
                .unwrap()
            );
            assert_eq!(calls.get(), 15);
        }
        prefix.last_hour = 3;
        let exact = ask(Some(&prefix), &key);
        assert!(exact.1.is_empty());
        assert_eq!(
            exact,
            tail(
                &source,
                1,
                Some(&prefix),
                tables,
                &key,
                (0, 29),
                &crate::prepared_measure::sample,
                &flag,
            )
            .unwrap()
        );
        let computed = calls.get();
        assert_eq!(ask(Some(&prefix), &key), exact);
        assert_eq!(calls.get(), computed);
    }

    #[test]
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
        if let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Countries(
            rows,
        )) = &mut *frames[1].tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = None;
        }
        for decoded in &mut frames[3..] {
            if let replay_frame::frame::TableView::Complete(
                replay_data::rows::TableRows::Countries(rows),
            ) = &mut *decoded.tables[TableId::Countries.index()]
            {
                rows[0].id = 9002;
            }
            if let replay_frame::frame::TableView::Complete(
                replay_data::rows::TableRows::CountryLinks(rows),
            ) = &mut *decoded.tables[TableId::CountryLinks.index()]
            {
                rows.push(replay_data::rows::CountryLinkRow {
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
        replay.gaps.push(replay_frame::branches::Gap {
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

    /// `tail` and `state_tail` as they read before shared block reads and
    /// read-ahead: the same blocks, read and decoded one at a time.
    #[allow(clippy::type_complexity)]
    fn serial(
        source: &Source,
        tables: &[TableId],
        requested: &[(String, String)],
        window: Option<(i32, i32)>,
        hours: &[i32],
    ) -> (
        BTreeMap<(String, String), Vec<Run>>,
        Vec<HourSpan>,
        BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
    ) {
        let replay = source.replay_ref();
        let wanted = hours
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let (window, blocks) = match window {
            Some((first, last)) => (
                (i64::from(first), last),
                replay
                    .index
                    .iter()
                    .filter(|block| {
                        block.branch == 1 && block.last_hour >= first && block.first_hour <= last
                    })
                    .collect::<Vec<_>>(),
            ),
            None => (
                (i64::from(i32::MIN), i32::MAX),
                replay
                    .index
                    .iter()
                    .filter(|block| {
                        block.branch == 1
                            && wanted
                                .range(block.first_hour..=block.last_hour)
                                .next()
                                .is_some()
                    })
                    .collect(),
            ),
        };
        sample_blocks_serial(
            source,
            1,
            tables,
            requested,
            window,
            blocks,
            &crate::prepared_measure::sample,
            window.1.eq(&i32::MAX).then_some(hours),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    /// Five blocks of GER's political power: runs crossing blocks, a missing
    /// value, a replaced native holder inside a block, and a recorded gap.
    fn varied_source() -> Source {
        let holder = |frame: &mut DecodedFrame| {
            if let replay_frame::frame::TableView::Complete(
                replay_data::rows::TableRows::Countries(rows),
            ) = &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].id = 9002;
            }
            if let replay_frame::frame::TableView::Complete(
                replay_data::rows::TableRows::CountryLinks(rows),
            ) = &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.push(replay_data::rows::CountryLinkRow {
                    canonical_slot: 7,
                    id: 9002,
                });
            }
        };
        let values = [
            Some(5),
            Some(5),
            None,
            Some(5),
            Some(5),
            Some(5),
            Some(6),
            Some(6),
            Some(6),
            Some(6),
            Some(6),
            Some(7),
            Some(7),
            Some(7),
            Some(7),
            Some(7),
            Some(7),
            Some(8),
            Some(8),
            Some(8),
        ];
        let block = |first: usize| {
            (first..first + 4)
                .map(|hour| {
                    let mut decoded = frame(1, hour as i32, values[hour]);
                    if hour >= 6 {
                        holder(&mut decoded);
                    }
                    decoded
                })
                .collect::<Vec<_>>()
        };
        let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, block(0))]).unwrap();
        for first in [4, 8, 12, 16] {
            replay = replay
                .append(&[continuation_records(&replay, 1, block(first))])
                .unwrap();
        }
        assert_eq!(replay.index.len(), 5);
        replay.gaps.push(replay_frame::branches::Gap {
            branch: 1,
            first_hour: 13,
            last_hour: 13,
            reason: "synthetic gap".into(),
        });
        Source::from_replay(
            crate::SourceKey {
                source_id: "varied".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn shared_read_ahead_exact_reads_equal_the_serial_reads_and_read_each_block_once() {
        let source = varied_source();
        let key: (String, String) = ("GER".into(), "politicalPower".into());
        let requested = std::slice::from_ref(&key);
        let flag = AtomicBool::new(false);
        let measure = crate::prepared_measure::TABLES;
        // A narrower request first, so the measure's reads join a held read of
        // the same block with the tables it lacks.
        let narrow = [TableId::Campaign, TableId::Countries];
        for window in [(0, 19), (2, 9)] {
            assert_eq!(
                tail(
                    &source,
                    1,
                    None,
                    &narrow,
                    requested,
                    window,
                    &crate::prepared_measure::sample,
                    &flag,
                )
                .unwrap(),
                serial(&source, &narrow, requested, Some(window), &[]),
                "{window:?}"
            );
        }
        assert_eq!(super::super::frames::reads(&source).0, 5);
        for round in 0..2 {
            for window in [(0, 19), (2, 9), (6, 6), (4, 4), (13, 13), (12, 15), (3, 17)] {
                let shared = tail(
                    &source,
                    1,
                    None,
                    measure,
                    requested,
                    window,
                    &crate::prepared_measure::sample,
                    &flag,
                )
                .unwrap();
                assert_eq!(
                    shared,
                    serial(&source, measure, requested, Some(window), &[]),
                    "round {round}, {window:?}"
                );
            }
            for hours in [&[3, 6, 13, 19][..], &[8], &[4, 5, 6, 7]] {
                assert_eq!(
                    state_tail(
                        &source,
                        1,
                        None,
                        measure,
                        requested,
                        hours,
                        &crate::prepared_measure::sample,
                        &flag,
                    )
                    .unwrap(),
                    serial(&source, measure, requested, None, hours),
                    "round {round}, {hours:?}"
                );
            }
        }
        // The fixture exercises what the reads must keep exact.
        let (runs, _, identities) = serial(&source, measure, requested, Some((0, 19)), &[]);
        assert!(
            runs[&key]
                .iter()
                .any(|run| run.break_before && run.first_hour > 0)
        );
        assert!(
            runs[&key]
                .iter()
                .any(|run| run.first_hour < 4 && run.last_hour >= 4)
        );
        assert_eq!(identities[&key].len(), 3, "a holder change and a gap");
        // Each block was read once for the narrow tables and once for the
        // country links it lacked; the second round read nothing.
        assert_eq!(super::super::frames::reads(&source).0, 10);
    }

    #[test]
    fn concurrent_same_hour_reads_decode_each_hour_and_block_once() {
        let source = varied_source();
        let key: (String, String) = ("GER".into(), "politicalPower".into());
        let measure = crate::prepared_measure::TABLES;
        let expected = match source
            .replay_ref()
            .seek_selected(1, 19, Some(measure))
            .unwrap()
        {
            SeekResult::Frame(frame) => frame,
            SeekResult::Gap { .. } => unreachable!(),
        };
        let states = serial(&source, measure, std::slice::from_ref(&key), None, &[19]);
        std::thread::scope(|scope| {
            let readers = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let flag = AtomicBool::new(false);
                        let frame = frame_at(&source, 1, 19, measure, &flag).unwrap().unwrap();
                        let state = state_tail(
                            &source,
                            1,
                            None,
                            measure,
                            std::slice::from_ref(&key),
                            &[19],
                            &crate::prepared_measure::sample,
                            &flag,
                        )
                        .unwrap();
                        (frame, state)
                    })
                })
                .collect::<Vec<_>>();
            for reader in readers {
                let (frame, state) = reader.join().unwrap();
                assert_eq!(frame, expected);
                assert!(
                    frame
                        .tables
                        .iter()
                        .zip(&expected.tables)
                        .all(|(read, seek)| read.sampled_at() == seek.sampled_at())
                );
                assert_eq!(state, states);
            }
        });
        // The state reads' left edge is the hour before the block, seeked once.
        assert_eq!(
            super::super::frames::reads(&source),
            (1, 2),
            "eight requests: one block read, and one seek of each hour"
        );
        // A narrower request is answered from the held hour, its other tables
        // unselected exactly as its own seek leaves them.
        let narrow = [TableId::Countries];
        let direct = source
            .replay_ref()
            .seek_selected(1, 19, Some(&narrow))
            .unwrap();
        let SeekResult::Frame(direct) = direct else {
            unreachable!()
        };
        let held = frame_at(&source, 1, 19, &narrow, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(held, direct);
        assert!(
            held.complete::<replay_data::rows::tables::CountryLinks>()
                .is_err()
        );
        assert_eq!(super::super::frames::reads(&source), (1, 2));
    }

    #[test]
    fn the_frame_cache_keeps_its_byte_budget_and_evicts_the_least_recently_used() {
        let source = varied_source();
        let measure = crate::prepared_measure::TABLES;
        let flag = AtomicBool::new(false);
        frame_at(&source, 1, 1, measure, &flag).unwrap().unwrap();
        let one = super::super::frames::held_bytes(&source);
        assert!(one > 0);
        super::super::frames::set_budget(&source, one * 3 / 2);
        frame_at(&source, 1, 2, measure, &flag).unwrap().unwrap();
        assert!(super::super::frames::held_bytes(&source) <= one * 3 / 2);
        assert_eq!(super::super::frames::reads(&source).1, 2);
        // Hour 2 is held; hour 1, used least recently, was evicted.
        frame_at(&source, 1, 2, measure, &flag).unwrap().unwrap();
        assert_eq!(super::super::frames::reads(&source).1, 2);
        frame_at(&source, 1, 1, measure, &flag).unwrap().unwrap();
        assert_eq!(super::super::frames::reads(&source).1, 3);
        // An hour outside the recording is a gap, never kept.
        assert!(frame_at(&source, 1, 99, measure, &flag).unwrap().is_none());
        assert!(frame_at(&source, 1, 13, measure, &flag).unwrap().is_none());
        assert!(super::super::frames::held_bytes(&source) <= one * 3 / 2);
    }

    #[test]
    fn reads_without_block_decodes_refuse_only_what_needs_a_decode() {
        let source = varied_source();
        let measure = crate::prepared_measure::TABLES;
        let key: (String, String) = ("GER".into(), "politicalPower".into());
        let flag = AtomicBool::new(false);
        let (read, refused) = without_block_decodes(|| frame_at(&source, 1, 5, measure, &flag));
        assert!(refused);
        assert_eq!(read.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        let (read, refused) = without_block_decodes(|| {
            tail(
                &source,
                1,
                None,
                measure,
                std::slice::from_ref(&key),
                (0, 3),
                &crate::prepared_measure::sample,
                &flag,
            )
        });
        assert!(refused);
        assert_eq!(read.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert_eq!(super::super::frames::reads(&source), (0, 0));
        // A held hour and a covered tail answer without a decode.
        let held = frame_at(&source, 1, 5, measure, &flag).unwrap();
        let (read, refused) = without_block_decodes(|| frame_at(&source, 1, 5, measure, &flag));
        assert!(!refused);
        assert_eq!(read.unwrap(), held);
        let complete = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 19,
            complete: true,
        };
        let (read, refused) = without_block_decodes(|| {
            tail(
                &source,
                1,
                Some(&complete),
                measure,
                std::slice::from_ref(&key),
                (0, 19),
                &crate::prepared_measure::sample,
                &flag,
            )
        });
        assert!(!refused);
        assert!(read.unwrap().0.is_empty());
        // Outside it, decodes are allowed again.
        assert!(frame_at(&source, 1, 6, measure, &flag).unwrap().is_some());
    }

    #[test]
    fn a_read_is_accounted_before_it_is_released() {
        use super::super::frames;
        let source = Arc::new(varied_source());
        let checks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let source = Arc::clone(&source);
            let checks = std::rc::Rc::clone(&checks);
            frames::ON_RELEASE.set(Some(Box::new(move || {
                checks.borrow_mut().push(frames::accounted(&source));
            })));
        }
        let key: (String, String) = ("GER".into(), "politicalPower".into());
        let flag = AtomicBool::new(false);
        // Narrower reads first, then wider ones joined to them, of an hour and a
        // block; a one-block window is read on this thread.
        frame_at(&source, 1, 5, &[TableId::Countries], &flag).unwrap();
        frame_at(&source, 1, 5, crate::prepared_measure::TABLES, &flag).unwrap();
        for tables in [
            &[TableId::Campaign, TableId::Countries][..],
            crate::prepared_measure::TABLES,
        ] {
            tail(
                &source,
                1,
                None,
                tables,
                std::slice::from_ref(&key),
                (0, 3),
                &crate::prepared_measure::sample,
                &flag,
            )
            .unwrap();
        }
        // A gap is released holding nothing.
        frame_at(&source, 1, 99, crate::prepared_measure::TABLES, &flag).unwrap();
        frames::ON_RELEASE.set(None);
        let checks = checks.borrow();
        assert_eq!(checks.len(), 5, "two hour reads, two block reads and a gap");
        assert!(
            checks.iter().all(|&accounted| accounted),
            "every release found its read accounted: {checks:?}"
        );
        assert!(frames::accounted(&source));
    }

    #[test]
    fn cached_reads_count_their_frames_whole_and_their_nested_rows() {
        use super::super::frames;
        let source = varied_source();
        let key: (String, String) = ("GER".into(), "politicalPower".into());
        let flag = AtomicBool::new(false);
        // One four-hour block, read for one table: every frame is kept whole.
        tail(
            &source,
            1,
            None,
            &[TableId::Countries],
            std::slice::from_ref(&key),
            (0, 3),
            &crate::prepared_measure::sample,
            &flag,
        )
        .unwrap();
        assert!(
            frames::held_bytes(&source) >= 4 * std::mem::size_of::<Frame>(),
            "{} bytes for four whole frames",
            frames::held_bytes(&source)
        );
        // A row's list is counted, not only the row.
        let mut long = frame(1, 0, Some(1));
        if let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Countries(
            rows,
        )) = &mut *long.tables[TableId::Countries.index()]
        {
            rows[0].tag = vec![b'x'; 256 << 10];
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![long])]).unwrap();
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "long".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &flag,
        )
        .unwrap();
        frame_at(&source, 1, 0, &[TableId::Countries], &flag)
            .unwrap()
            .unwrap();
        assert!(
            frames::held_bytes(&source) >= 256 << 10,
            "{} bytes for a 256 KiB tag",
            frames::held_bytes(&source)
        );
        assert!(frames::accounted(&source));
    }

    #[test]
    fn requests_without_block_decodes_never_wait_on_another_read() {
        use super::super::frames;
        let source = varied_source();
        let measure = crate::prepared_measure::TABLES;
        let flag = AtomicBool::new(false);
        let held = frame_at(&source, 1, 6, measure, &flag).unwrap();
        // Another request is reading hours 5 and 6.
        let _five = frames::hold_hour(&source, 1, 5);
        let _six = frames::hold_hour(&source, 1, 6);
        let cancelled = AtomicBool::new(false);
        let (source, cancelled) = (&source, &cancelled);
        std::thread::scope(|scope| {
            let (sent, answers) = std::sync::mpsc::channel();
            scope.spawn(move || {
                let refused = without_block_decodes(|| frame_at(source, 1, 5, measure, cancelled));
                let covered = without_block_decodes(|| frame_at(source, 1, 6, measure, cancelled));
                sent.send((refused, covered)).unwrap();
            });
            // A wait would last until the other read ends; cancel it rather than hang.
            let answer = answers.recv_timeout(std::time::Duration::from_secs(10));
            cancelled.store(true, std::sync::atomic::Ordering::Release);
            let ((refused, refused_flag), (covered, covered_flag)) =
                answer.expect("an uncovered request waited on another request's read");
            assert!(refused_flag);
            assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::WouldBlock);
            assert!(!covered_flag, "a held hour answers without a decode");
            assert_eq!(covered.unwrap(), held);
        });
    }

    #[test]
    fn requests_without_block_decodes_leave_no_entries_for_hours_they_miss() {
        use super::super::frames;
        let source = varied_source();
        let measure = crate::prepared_measure::TABLES;
        let flag = AtomicBool::new(false);
        for hour in 0..20 {
            let (read, refused) =
                without_block_decodes(|| frame_at(&source, 1, hour, measure, &flag));
            if hour != 13 {
                assert!(refused, "hour {hour}");
                assert_eq!(read.unwrap_err().kind(), io::ErrorKind::WouldBlock);
            }
        }
        assert_eq!(frames::entries(&source), 0, "a refused miss keeps no entry");
        // Nor does a read that found nothing to keep.
        assert!(frame_at(&source, 1, 99, measure, &flag).unwrap().is_none());
        assert_eq!(frames::entries(&source), 0);
        assert_eq!(frames::held_bytes(&source), 0);
    }

    #[test]
    fn held_rows_count_every_nested_allocation() {
        use super::super::frames;
        use replay_data::{rows::PowerBalanceModifierBreakdownRow, rows::TableRows};
        const SOURCES: usize = 10_000;
        let mut decoded = frame(1, 0, Some(1));
        *decoded.tables[TableId::PowerBalanceModifierBreakdowns.index()] =
            replay_frame::frame::TableView::Complete(TableRows::PowerBalanceModifierBreakdowns(
                vec![PowerBalanceModifierBreakdownRow {
                    record: 1,
                    // Each empty source encodes in four bytes but holds a list.
                    sources: vec![Vec::new(); SOURCES],
                    ..Default::default()
                }],
            ));
        let flag = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "nested".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded])]).unwrap()),
            &flag,
        )
        .unwrap();
        frame_at(
            &source,
            1,
            0,
            &[TableId::PowerBalanceModifierBreakdowns],
            &flag,
        )
        .unwrap()
        .unwrap();
        assert!(
            frames::held_bytes(&source) >= SOURCES * std::mem::size_of::<Vec<u8>>(),
            "{} bytes for {SOURCES} held lists",
            frames::held_bytes(&source)
        );
        assert!(frames::accounted(&source));
    }
    #[test]
    fn supplying_read_lookups_share_a_bounded_cache_across_hour_variants() {
        use crate::test_fixtures::file_source;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let table = TableId::CountryIntel;
        let recording = crate::test_fixtures::replay(
            (0..24)
                .map(|hour| {
                    let mut value = frame(1, hour, None);
                    *value.tables[table.index()] =
                        TableView::Complete(TableRows::CountryIntel(Vec::new()));
                    value.metrics.table_ns[table.index()] = u64::from(hour == 0);
                    value
                })
                .collect(),
        );
        let (source, root) = file_source(recording);
        let flag = AtomicBool::new(false);
        let ask = |hour| {
            latest_read(&source, 1, hour, table, 24, &[table], &flag)
                .unwrap()
                .unwrap()
                .hour
        };
        for hour in 0..24 {
            assert_eq!(ask(hour), 0);
            let (bytes, bound, _) = super::super::observations::usage(&source, "supplying-reads");
            assert!(bytes > 0 && bytes <= bound);
        }
        let computed = super::super::observations::usage(&source, "supplying-reads").2;
        assert_eq!(ask(23), 0);
        assert_eq!(
            super::super::observations::usage(&source, "supplying-reads").2,
            computed
        );
        assert_eq!(ask(0), 0);
        assert_eq!(
            super::super::observations::usage(&source, "supplying-reads").2,
            computed + 1,
            "the oldest hour was evicted under the single-block bound"
        );
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn latest_read_is_bounded_by_cadence_and_reuses_its_blocks() {
        use crate::test_fixtures::{
            continuation_records, file_source, replay as recording, source as frame,
        };
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let make = |hour| {
            let mut value = frame(1, hour, None);
            value.tables[TableId::AirAces.index()] =
                TableView::Complete(TableRows::AirAces(vec![])).into();
            value.metrics.table_ns[TableId::AirAces.index()] = u64::from(hour % 24 == 5);
            value
        };
        let run = |start| {
            let hours: Vec<i32> = (start..=110).collect();
            let mut chunks = hours.chunks(12);
            let mut replay = recording(chunks.next().unwrap().iter().copied().map(make).collect());
            for chunk in chunks {
                replay = replay
                    .append(&[continuation_records(
                        &replay,
                        1,
                        chunk.iter().copied().map(make).collect(),
                    )])
                    .unwrap();
            }
            let (source, root) = file_source(replay);
            let flag = AtomicBool::new(false);
            let ask = || {
                latest_read(
                    &source,
                    1,
                    110,
                    TableId::AirAces,
                    24,
                    &[TableId::AirAces],
                    &flag,
                )
                .unwrap()
                .unwrap()
                .hour
            };
            assert_eq!(ask(), 101);
            let reads = super::super::frames::reads(&source);
            assert_eq!(ask(), 101);
            assert_eq!(super::super::frames::reads(&source), reads);
            let _ = std::fs::remove_dir_all(root);
            reads.0
        };
        assert_eq!(run(0), run(72), "earlier blocks add no look-back reads");
    }
}

#[cfg(test)]
mod sample_cache_tests {
    use super::*;
    use replay_data::{rows::TableRows, rows::tables};
    use replay_frame::frame::TableView;
    use std::cell::Cell;

    #[test]
    fn unchanged_views_reuse_samples_and_changed_inputs_refuse_or_project_again() {
        let mut frame = crate::test_fixtures::scale_frame(1, 10, 5000);
        let calls = Cell::new(0);
        let sample = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            calls.set(calls.get() + 1);
            let rows = frame.complete::<tables::Countries>()?;
            let mut result = Samples::new();
            for row in rows {
                let key = (String::from_utf8(row.tag.clone()).unwrap(), "value".into());
                if requested.is_none_or(|wanted| wanted.contains(&key)) {
                    result.insert(
                        key,
                        (Some((row.canonical_slot, row.id)), Some(i128::from(row.id))),
                    );
                }
            }
            Ok(result)
        };
        let cache = SamplesCache::default();
        let tables = [TableId::Countries];
        let first = cache.sample(&frame, &tables, None, &sample).unwrap();
        assert_eq!(first.len(), 5000);
        frame.hour += 7;
        assert_eq!(cache.sample(&frame, &tables, None, &sample).unwrap(), first);
        assert_eq!(calls.get(), 1);
        let selection = [first.keys().next().unwrap().clone()];
        assert_eq!(
            cache
                .sample(&frame, &tables, Some(&selection), &sample)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(calls.get(), 2);
        frame.branch += 1;
        cache
            .sample(&frame, &tables, Some(&selection), &sample)
            .unwrap();
        assert_eq!(calls.get(), 3);
        *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "synthetic refusal".into(),
        };
        assert!(
            cache
                .sample(&frame, &tables, Some(&selection), &sample)
                .is_err()
        );
        assert!(
            cache
                .sample(&frame, &tables, Some(&selection), &sample)
                .is_err()
        );
        assert_eq!(calls.get(), 5, "retained a failed extraction");
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![]));
        assert!(
            cache
                .sample(&frame, &tables, None, &sample)
                .unwrap()
                .is_empty()
        );
        assert_eq!(calls.get(), 6);
    }
}
