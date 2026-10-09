//! One cached branch scan for Ships sunk log and Naval losses. Equipment losses
//! can adopt this scan in a follow-up; its independent consumer is unchanged.
//!
//! P: fill_common@140C1E960 stamps the event hour. Prune@1401D7560 deletes
//! entries whose event hour + NNavy.NAVAL_COMBAT_RESULT_TIMEOUT_YEARS * 0x2238
//! is <= now, swap-erasing the last pointer into the removed position. Daily
//! update@140DC4F30 calls it. Survivors are immutable and order is immaterial:
//! compare whole-entry multiplicities at consecutive complete reads, not vector
//! positions, partial keys or lifetime maxima. Identical entries are legitimate.
//! After a break, surviving multiset increases can be recovered by
//! their event hour. Events born and pruned entirely during the break cannot be
//! counted from this vector. Runtime retention identifies possible loss spans.
//! A hit on branch A waits behind a scan of branch B under the cache lock.
//!
//! P: convoy writer@1401CAE90 merges (month, killer, owner) and adds a signed
//! i32 count (@140E9E680). Prune@1401D7480 removes months strictly older than
//! current month - NNavy.CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS, also by swap-erase.
//! There is no convoy event hour: deltas retain their bucket and supplying read.

use super::{
    country_groups::HourSpan,
    country_history::{Identity, observe_identity},
};
use crate::{
    Source,
    metadata::{
        IdentityTrack, LinkTags, TaggedCountry, branch_bounds, link_slot, slot_read,
        tagged_countries, validate_campaign,
    },
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, SharedView, TableView},
    rows::{SunkConvoyRow, SunkShipEntry, TableId, TableRows, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug)]
pub struct ShipEvent {
    pub entry: SunkShipEntry,
    pub read_hour: i32,
    pub victim: Option<String>,
    pub attacker: Option<String>,
    /// Supplying-read fallback for a reference with no history at the event hour.
    pub victim_as_of: Option<i32>,
    pub attacker_as_of: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct ConvoyEvent {
    pub month: i32,
    pub read_hour: i32,
    pub count: i64,
    pub owner: Option<String>,
    pub killer: Option<String>,
    pub baseline: bool,
    pub previous_read: Option<i32>,
    pub owner_raw: u32,
    pub killer_raw: u32,
}

/// One tag's native holder over consecutive recorded hours. Tag spelling is
/// presentation; the canonical slot and native country id own its tallies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryIdentitySpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub identity: (u32, u32),
}

#[derive(Clone, Debug, Default)]
pub struct Scanned {
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_ship_read: Option<i32>,
    pub first_convoy_read: Option<i32>,
    pub last_ship_read: Option<i32>,
    pub last_convoy_read: Option<i32>,
    pub ship_reads: Vec<i32>,
    pub convoy_reads: Vec<i32>,
    pub ships: Vec<ShipEvent>,
    pub convoys: Vec<ConvoyEvent>,
    pub ship_unread: Vec<HourSpan>,
    pub convoy_unread: Vec<HourSpan>,
    /// Potential unseen births start after the preceding read, before unread coverage.
    pub ship_breaks: Vec<HourSpan>,
    pub convoy_breaks: Vec<HourSpan>,
    pub country_candidates: BTreeMap<u32, BTreeSet<Identity>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub country_identities: BTreeMap<String, Vec<CountryIdentitySpan>>,
    pub tokens: BTreeMap<u32, String>,
    pub warnings: Vec<String>,
    pub gap_recoveries: usize,
    pub refused_entries: BTreeMap<&'static str, usize>,
    pub uncertain_victims: BTreeMap<Identity, Vec<(i32, i32)>>,
    pub uncertain_attackers: BTreeMap<Identity, Vec<(i32, i32)>>,
    pub uncertain_convoy_owners: BTreeMap<Identity, Vec<(i32, i32)>>,
    pub uncertain_convoy_killers: BTreeMap<Identity, Vec<(i32, i32)>>,
    pub unresolved: BTreeSet<u32>,
}

/// Every recorded field participates, including attribution and presence flags.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ShipKey<'a> {
    text: [&'a [u8]; 4],
    fields: [u64; 14],
}
impl<'a> From<&'a SunkShipEntry> for ShipKey<'a> {
    fn from(e: &'a SunkShipEntry) -> Self {
        Self {
            text: [
                &e.ship_name,
                &e.attribution_name,
                &e.class_name,
                &e.attribution_class_name,
            ],
            fields: [
                u64::from(e.victim_country),
                u64::from(e.attacker_country),
                e.sink_hour as u32 as u64,
                u64::from(e.class_token),
                u64::from(e.attribution_class_token),
                u64::from(e.attribution_class_present),
                e.model_level as u32 as u64,
                e.victim_equipment_variant,
                e.damage_source_reference,
                u64::from(e.province),
                u64::from(e.province_present),
                e.battle,
                u64::from(e.cause_record),
                u64::from(e.assisted_kill),
            ],
        }
    }
}

type RefusedShipKey = ([Vec<u8>; 4], [u64; 14]);

#[derive(Default)]
struct Tracker {
    ships: Option<SharedView>,
    ship_open: bool,
    ship_future: Option<i32>,
    refused_ships: HashMap<RefusedShipKey, usize>,
    convoys: BTreeMap<(i32, u32, u32), i32>,
    convoy_open: bool,
}

fn append_span(spans: &mut Vec<HourSpan>, first: i32, last: i32) {
    if first > last {
        return;
    }
    let index = spans.partition_point(|s| i64::from(s.last_hour) + 1 < i64::from(first));
    let end = spans.partition_point(|s| i64::from(s.first_hour) <= i64::from(last) + 1);
    let first_hour = spans
        .get(index)
        .filter(|_| index < end)
        .map_or(first, |s| first.min(s.first_hour));
    let last_hour = end
        .checked_sub(1)
        .filter(|_| index < end)
        .map_or(last, |i| last.max(spans[i].last_hour));
    spans.splice(
        index..end,
        [HourSpan {
            first_hour,
            last_hour,
        }],
    );
}

impl Scanned {
    pub(crate) fn identity(&self, tag: &str, hour: i32) -> Option<Identity> {
        let spans = self.country_identities.get(tag)?;
        let end = spans.partition_point(|span| span.first_hour <= hour);
        spans
            .get(end.checked_sub(1)?)
            .filter(|span| {
                hour <= span.last_hour
                    || spans
                        .get(end)
                        .is_some_and(|next| next.identity == span.identity)
            })
            .map(|span| span.identity)
    }

    fn missing(&mut self, track: &mut Tracker, first: i32, last: i32) {
        append_span(&mut self.ship_unread, first, last);
        append_span(&mut self.convoy_unread, first, last);
        append_span(
            &mut self.ship_breaks,
            self.last_ship_read.map_or(first, |h| h.saturating_add(1)),
            last,
        );
        append_span(
            &mut self.convoy_breaks,
            self.last_convoy_read.map_or(first, |h| h.saturating_add(1)),
            last,
        );
        track.ship_open = false;
        track.convoy_open = false;
    }
}

impl Tracker {
    fn ships(&mut self, out: &mut Scanned, view: &SharedView, hour: i32) {
        let last_read = out.last_ship_read;
        if !self.ship_open && last_read.is_some() {
            out.gap_recoveries += 1;
        }
        self.ship_open = true;
        out.first_ship_read.get_or_insert(hour);
        out.last_ship_read = Some(hour);
        out.ship_reads.push(hour);
        if self
            .ships
            .as_ref()
            .is_some_and(|last| SharedView::ptr_eq(last, view))
            && self.ship_future.is_none_or(|future| hour < future)
        {
            return;
        }
        let mut previous = HashMap::<ShipKey<'_>, usize>::new();
        if let Some(view) = &self.ships {
            for entry in ship_entries(view) {
                if last_read.is_some_and(|hour| ship_refusal(entry, hour).is_none()) {
                    *previous.entry(entry.into()).or_default() += 1;
                }
            }
        }
        let mut now = HashMap::<ShipKey<'_>, (usize, &SunkShipEntry)>::new();
        self.ship_future = None;
        let mut refused = HashMap::<RefusedShipKey, usize>::new();
        for entry in ship_entries(view) {
            if let Some(reason) = ship_refusal(entry, hour) {
                let key = ShipKey::from(entry);
                let key = (key.text.map(|bytes| bytes.to_vec()), key.fields);
                let count = refused.entry(key.clone()).or_default();
                *count += 1;
                if *count > self.refused_ships.get(&key).copied().unwrap_or(0) {
                    *out.refused_entries.entry(reason).or_default() += 1;
                }
                if entry.sink_hour > hour {
                    self.ship_future = Some(
                        self.ship_future
                            .map_or(entry.sink_hour, |future| future.min(entry.sink_hour)),
                    );
                }
                continue;
            }
            now.entry(entry.into()).or_insert((0, entry)).0 += 1;
        }
        for entry in ship_entries(view) {
            let key = ShipKey::from(entry);
            let Some((count, entry)) = now.remove(&key) else {
                continue;
            };
            let added = count.saturating_sub(previous.get(&key).copied().unwrap_or(0));
            for _ in 0..added {
                out.ships.push(ShipEvent {
                    entry: entry.clone(),
                    read_hour: hour,
                    victim: None,
                    attacker: None,
                    victim_as_of: None,
                    attacker_as_of: None,
                });
            }
        }
        drop(now);
        drop(previous);
        self.refused_ships = refused;
        self.ships = Some(view.clone());
    }

    fn convoys(
        &mut self,
        out: &mut Scanned,
        rows: &[SunkConvoyRow],
        hour: i32,
        tags: &HashMap<u32, String>,
    ) {
        let baseline = out.first_convoy_read.is_none();
        for row in rows {
            let last = self
                .convoys
                .entry((row.month, row.killer_country, row.owner_country))
                .or_default();
            // ADD dword@140E9E680 wraps the native bucket. Widen both signed
            // endpoints so summing deltas reproduces each captured i32 value.
            let count = i64::from(row.convoys) - i64::from(*last);
            *last = row.convoys;
            if count != 0 {
                out.convoys.push(ConvoyEvent {
                    month: row.month,
                    read_hour: hour,
                    count,
                    owner: tags.get(&row.owner_country).cloned(),
                    killer: tags.get(&row.killer_country).cloned(),
                    baseline,
                    previous_read: out.last_convoy_read,
                    owner_raw: row.owner_country,
                    killer_raw: row.killer_country,
                });
            }
        }
        self.convoy_open = true;
        out.first_convoy_read.get_or_insert(hour);
        out.last_convoy_read = Some(hour);
        out.convoy_reads.push(hour);
    }
}

fn ship_entries(view: &SharedView) -> &[SunkShipEntry] {
    match &**view {
        TableView::Complete(TableRows::SunkShips(rows)) if rows.len() == 1 => &rows[0].entries,
        _ => &[],
    }
}

fn ship_refusal(entry: &SunkShipEntry, hour: i32) -> Option<&'static str> {
    if entry.sink_hour > hour {
        Some("SunkShips event hour is after its read")
    } else if entry.cause_record > 1
        || entry.assisted_kill > 1
        || entry.province_present > 1
        || entry.attribution_class_present > 1
    {
        Some("SunkShips boolean flag is above 1")
    } else {
        None
    }
}

#[derive(Clone)]
struct TagRun {
    first: i32,
    last: i32,
    tag: String,
    identity: Identity,
}

#[derive(Default)]
struct CountryCache {
    links: LinkTags,
    holders: Vec<(String, Identity)>,
    tags: HashMap<u32, String>,
    empty: HashMap<u32, String>,
}

pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scanned> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Naval sinkings cancelled",
        ));
    }
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::SunkShips,
        TableId::SunkConvoys,
        TableId::LexicalTokens,
    ];
    let start = replay
        .branches
        .iter()
        .find(|b| b.id == branch)
        .ok_or_else(|| io::Error::other("Naval sinkings branch not found"))?;
    let (first_hour, last_hour) = branch_bounds(replay, start);
    let mut out = Scanned {
        first_hour,
        last_hour,
        ..Default::default()
    };
    let mut track = Tracker::default();
    let mut raw_history = HashMap::<u32, Vec<TagRun>>::new();
    let mut identities = HashMap::<String, IdentityTrack>::new();
    let mut country_cache = CountryCache::default();
    let mut dictionary: Option<SharedView> = None;
    let mut blocks: Vec<_> = replay.index.iter().filter(|b| b.branch == branch).collect();
    blocks.sort_unstable_by_key(|b| b.first_hour);
    let mut next = i64::from(first_hour);
    for block in blocks {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Naval sinkings cancelled",
            ));
        }
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Naval sinkings cancelled",
                ));
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let hour = decoded.hour;
            if i64::from(hour) < next
                || replay
                    .gaps
                    .iter()
                    .any(|g| g.branch == branch && g.first_hour <= hour && hour <= g.last_hour)
            {
                continue;
            }
            if next < i64::from(hour) {
                out.missing(&mut track, next as i32, hour - 1);
            }
            let tags = countries(
                decoded,
                &mut out,
                &mut raw_history,
                &mut identities,
                &mut country_cache,
            );
            let view = &decoded.tables[TableId::LexicalTokens.index()];
            if !dictionary
                .as_ref()
                .is_some_and(|last| SharedView::ptr_eq(last, view))
                && let Ok(rows) = decoded.complete::<tables::LexicalTokens>()
            {
                dictionary = Some(view.clone());
                for row in rows {
                    if let Ok(text) = std::str::from_utf8(&row.literal) {
                        out.tokens
                            .entry(row.token)
                            .or_insert_with(|| text.to_owned());
                    }
                }
            }
            let ships = decoded.complete::<tables::SunkShips>();
            if let Ok([_]) = ships
                && slot_read(&decoded.metrics, TableId::SunkShips, hour)
            {
                track.ships(&mut out, &decoded.tables[TableId::SunkShips.index()], hour);
            } else if !matches!(ships, Ok([_])) || !track.ship_open {
                append_span(&mut out.ship_unread, hour, hour);
                append_span(
                    &mut out.ship_breaks,
                    out.last_ship_read.map_or(hour, |h| h.saturating_add(1)),
                    hour,
                );
                track.ship_open = false;
            }
            let convoys = decoded.complete::<tables::SunkConvoys>();
            if let Ok(rows) = convoys
                && slot_read(&decoded.metrics, TableId::SunkConvoys, hour)
            {
                track.convoys(&mut out, rows, hour, tags);
            } else if convoys.is_err() || !track.convoy_open {
                append_span(&mut out.convoy_unread, hour, hour);
                append_span(
                    &mut out.convoy_breaks,
                    out.last_convoy_read.map_or(hour, |h| h.saturating_add(1)),
                    hour,
                );
                track.convoy_open = false;
            }
            next = i64::from(hour) + 1;
        }
    }
    if next <= i64::from(last_hour) {
        out.missing(&mut track, next as i32, last_hour);
    }
    for event in &mut out.ships {
        for (raw, tag, as_of, uncertain, primary) in [
            (
                event.entry.victim_country,
                &mut event.victim,
                &mut event.victim_as_of,
                &mut out.uncertain_victims,
                true,
            ),
            (
                event.entry.attacker_country,
                &mut event.attacker,
                &mut event.attacker_as_of,
                &mut out.uncertain_attackers,
                event.entry.cause_record == 0 && event.entry.assisted_kill == 0,
            ),
        ] {
            let (resolved, dated, possible) = ship_attribution(
                raw,
                event.entry.sink_hour,
                event.read_hour,
                &raw_history,
                &out.country_candidates,
            );
            *tag = resolved;
            *as_of = dated;
            if raw != 0 && tag.is_none() {
                out.unresolved.insert(raw);
                if !primary {
                    continue;
                }
                for identity in possible {
                    uncertain
                        .entry(identity)
                        .or_default()
                        .push((event.entry.sink_hour, event.read_hour));
                }
            }
        }
    }
    for event in &mut out.convoys {
        for (raw, tag, uncertain) in [
            (
                event.owner_raw,
                &mut event.owner,
                &mut out.uncertain_convoy_owners,
            ),
            (
                event.killer_raw,
                &mut event.killer,
                &mut out.uncertain_convoy_killers,
            ),
        ] {
            let (resolved, possible) =
                attribution(raw, event.read_hour, &raw_history, &out.country_candidates);
            *tag = resolved;
            if raw != 0 && tag.is_none() {
                out.unresolved.insert(raw);
                for identity in possible {
                    uncertain
                        .entry(identity)
                        .or_default()
                        .push((event.month, event.read_hour));
                }
            }
        }
    }
    if out.gap_recoveries > 0 {
        out.warnings.push(format!("{} ship-history stretches resumed after missing coverage. Ships born and pruned wholly between reads cannot be counted when the break reaches retention.", out.gap_recoveries));
    }
    for (reason, count) in &out.refused_entries {
        out.warnings
            .push(format!("{count} naval entries refused: {reason}."));
    }
    if !out.unresolved.is_empty() {
        out.warnings.push(format!(
            "{} naval country references are unresolved.",
            out.unresolved.len()
        ));
    }
    Ok(out)
}

fn countries<'a>(
    decoded: &DecodedFrame,
    out: &mut Scanned,
    history: &mut HashMap<u32, Vec<TagRun>>,
    identities: &mut HashMap<String, IdentityTrack>,
    cache: &'a mut CountryCache,
) -> &'a HashMap<u32, String> {
    let Ok(changed) = cache.links.update(decoded) else {
        return &cache.empty;
    };
    if changed {
        let rows = decoded
            .complete::<tables::Countries>()
            .expect("LinkTags complete countries");
        let links = decoded
            .complete::<tables::CountryLinks>()
            .expect("LinkTags complete links");
        cache.holders.clear();
        let mut possible = HashMap::<u32, BTreeSet<Identity>>::new();
        for row in rows {
            possible
                .entry(row.canonical_slot)
                .or_default()
                .insert((row.canonical_slot, row.id));
        }
        for link in links {
            if let Some(holders) = possible.get(&link.canonical_slot) {
                out.country_candidates
                    .entry(link.id)
                    .or_default()
                    .extend(holders);
            }
        }
        let mut slots = HashMap::new();
        for (tag, country) in tagged_countries(rows, links) {
            if let TaggedCountry::Found(country, slot) = country
                && let Ok(tag) = std::str::from_utf8(tag)
            {
                let tag = tag.to_owned();
                out.segments.entry(tag.clone()).or_default();
                out.country_identities.entry(tag.clone()).or_default();
                identities.entry(tag.clone()).or_default();
                cache.holders.push((tag.clone(), (slot, country.id)));
                slots.insert(slot, tag);
            }
        }
        cache.tags = links
            .iter()
            .filter_map(|link| {
                let tag = link_slot(links, link.id).and_then(|slot| slots.get(&slot))?;
                // Preserve tagged_countries' ambiguity semantics rather than
                // accepting a different row LinkTags could name under the slot.
                if cache.links.get(link.id) == Some(tag) {
                    Some((link.id, tag.clone()))
                } else {
                    None
                }
            })
            .collect();
    }
    for (tag, identity) in &cache.holders {
        let contiguous = observe_identity(
            out.segments.get_mut(tag).expect("country segments"),
            identities.get_mut(tag).expect("country identity"),
            *identity,
            decoded.hour,
        );
        let spans = out
            .country_identities
            .get_mut(tag)
            .expect("country identity spans");
        if contiguous {
            spans.last_mut().expect("country identity span").last_hour = decoded.hour;
        } else {
            spans.push(CountryIdentitySpan {
                first_hour: decoded.hour,
                last_hour: decoded.hour,
                identity: *identity,
            });
        }
    }
    for (raw, tag) in &cache.tags {
        let Some(identity) = out.identity(tag, decoded.hour) else {
            continue;
        };
        let runs = history.entry(*raw).or_default();
        if let Some(run) = runs.last_mut()
            && run.tag == *tag
            && run.identity == identity
            && run.last.checked_add(1) == Some(decoded.hour)
        {
            run.last = decoded.hour;
        } else {
            runs.push(TagRun {
                first: decoded.hour,
                last: decoded.hour,
                tag: tag.clone(),
                identity,
            });
        }
    }
    &cache.tags
}

/// Retained entries can predate country history. Name those only at the read
/// that supplied them; a later read must not fill an unresolved supplying read.
fn ship_attribution(
    raw: u32,
    hour: i32,
    read_hour: i32,
    history: &HashMap<u32, Vec<TagRun>>,
    candidates: &BTreeMap<u32, BTreeSet<Identity>>,
) -> (Option<String>, Option<i32>, BTreeSet<Identity>) {
    let (tag, possible) = attribution(raw, hour, history, candidates);
    if tag.is_none()
        && let Some(runs) = history.get(&raw)
        && runs.first().is_some_and(|run| hour < run.first)
    {
        let end = runs.partition_point(|run| run.first <= read_hour);
        if let Some(run) = end.checked_sub(1).and_then(|i| runs.get(i))
            && read_hour <= run.last
        {
            return (Some(run.tag.clone()), Some(read_hour), BTreeSet::new());
        }
    }
    (tag, None, possible)
}

fn attribution(
    raw: u32,
    hour: i32,
    history: &HashMap<u32, Vec<TagRun>>,
    candidates: &BTreeMap<u32, BTreeSet<Identity>>,
) -> (Option<String>, BTreeSet<Identity>) {
    if raw == 0 {
        return (None, BTreeSet::new());
    }
    let runs = history.get(&raw).map(Vec::as_slice).unwrap_or(&[]);
    let end = runs.partition_point(|r| r.first <= hour);
    let before = end.checked_sub(1).and_then(|i| runs.get(i));
    let after = runs.get(end);
    let resolved = before.filter(|r| hour <= r.last).or_else(|| {
        before.filter(|left| {
            after.is_some_and(|right| left.tag == right.tag && left.identity == right.identity)
        })
    });
    if let Some(run) = resolved {
        return (Some(run.tag.clone()), BTreeSet::new());
    }
    let mut possible: BTreeSet<_> = before
        .into_iter()
        .chain(after)
        .map(|run| run.identity)
        .collect();
    if possible.is_empty() {
        possible.extend(candidates.get(&raw).into_iter().flatten().copied());
    }
    (None, possible)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryLoss {
    pub first_hour: i32,
    pub last_hour: i32,
    pub known_at: i32,
}

#[derive(Default)]
struct Cache {
    entries: Vec<(u64, Arc<Scanned>)>,
}

pub(crate) struct Retention {
    pub ships: bool,
    pub convoys: bool,
    pub ship_loss: Vec<HistoryLoss>,
    pub convoy_loss: Vec<HistoryLoss>,
    pub warnings: Vec<String>,
}

/// Runtime defines used by daily prune@1401D7560/1401D7480. A missing numeric
/// definition is unknown, never the getter's default zero. Nonpositive windows
/// can delete changing histories before the next daily read.
pub(crate) fn retention(
    scan: &Scanned,
    catalogue: Option<&crate::catalogue::Catalogue>,
) -> Retention {
    let mut result = Retention {
        ships: false,
        convoys: false,
        ship_loss: Vec::new(),
        convoy_loss: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(catalogue) = catalogue else {
        result
            .warnings
            .push("Naval retention defines are unavailable; completeness is unknown.".into());
        return result;
    };
    let defines = installed_content::defines::read(&catalogue.content.layers);
    for (key, ship) in [
        ("NAVAL_COMBAT_RESULT_TIMEOUT_YEARS", true),
        ("CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS", false),
    ] {
        let value = defines.as_ref().map_err(ToString::to_string).and_then(|d| {
            d.number("NNavy", key).map_err(|e| e.to_string())?;
            d.int32("NNavy", key).map_err(|e| e.to_string())
        });
        match value {
            Ok(value) if value > 0 && (!ship || value.checked_mul(0x2238).is_some()) => {
                if ship {
                    result.ships = true;
                    let hours = i64::from(value) * 0x2238;
                    for span in &scan.ship_breaks {
                        let last = i64::from(span.last_hour) + 1 - hours;
                        if last >= i64::from(span.first_hour) {
                            result.ship_loss.push(HistoryLoss { first_hour: span.first_hour, last_hour: last as i32, known_at: span.last_hour.saturating_add(1) });
                        }
                    }
                    let long = result.ship_loss.len();
                    if long > 0 { result.warnings.push(format!("{long} unread ship-history spans reach NDefines.NNavy.{key} retention; sinkings born and pruned wholly between complete reads cannot be counted.")); }
                } else {
                    result.convoys = true;
                    for span in &scan.convoy_breaks {
                        if let (Some(first), Some(resume)) = (month(span.first_hour), month(span.last_hour.saturating_add(1))) {
                            // Prune@1401D7480 uses strict <; equality survives.
                            let last = resume - value - 1;
                            if last >= first {
                                result.convoy_loss.push(HistoryLoss {
                                    first_hour: month_start(first).unwrap_or(span.first_hour),
                                    last_hour: month_start(last + 1).unwrap_or(span.last_hour).saturating_sub(1),
                                    known_at: span.last_hour.saturating_add(1),
                                });
                            }
                        }
                    }
                    if !result.convoy_loss.is_empty() {
                        result.warnings.push(format!("{} unread convoy-history spans exceed NDefines.NNavy.{key} retention; unseen pruned month buckets cannot be counted.", result.convoy_loss.len()));
                    }
                }
            }
            Ok(_) => result.warnings.push(format!("NDefines.NNavy.{key} is nonpositive or outside supported retention arithmetic; completeness is unknown.")),
            Err(error) => result.warnings.push(format!("NDefines.NNavy.{key} is unavailable: {error}; completeness is unknown.")),
        }
    }
    result
}

/// Writer@1401CAE90 stores GetMonthZeroBased@140177140(date) +
/// ((hour - HOUR_EPOCH) / 0x2238) * 12, exactly this absolute month encoding.
pub(crate) fn month(hour: i32) -> Option<i32> {
    let elapsed = i64::from(hour) - i64::from(engine::cdate::HOUR_EPOCH);
    if elapsed < 0 {
        return None;
    }
    let mut day = (elapsed / 24) % 365;
    let mut m = 0;
    while day >= i64::from(engine::cdate::MONTH_LENGTHS[m]) {
        day -= i64::from(engine::cdate::MONTH_LENGTHS[m]);
        m += 1;
    }
    i32::try_from(elapsed / (365 * 24) * 12 + m as i64).ok()
}

pub(crate) fn month_start(bucket: i32) -> Option<i32> {
    if bucket < 0 {
        return None;
    }
    let days = i64::from(bucket / 12) * 365
        + engine::cdate::MONTH_LENGTHS[..(bucket % 12) as usize]
            .iter()
            .map(|n| i64::from(*n))
            .sum::<i64>();
    i32::try_from(i64::from(engine::cdate::HOUR_EPOCH) + days * 24).ok()
}

/// Serialize cache construction too: simultaneous widgets share one scan.
pub fn cached(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Scanned>> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Naval sinkings cancelled",
        ));
    }
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, scanned)) = cache.entries.iter().find(|(id, _)| *id == branch) {
        return Ok(Arc::clone(scanned));
    }
    let scanned = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
    cache.entries.push((branch, Arc::clone(&scanned)));
    // Retain the last two scanned branches, matching Country factories.
    if cache.entries.len() > 2 {
        cache.entries.remove(0);
    }
    Ok(scanned)
}

#[cfg(test)]
mod tests;
