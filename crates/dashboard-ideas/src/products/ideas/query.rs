//! Ideas' replies, read from the family's series publication ([`super::emit`]): a country's idea
//! history through its gate's accepted prefix, and the event feed's view of the family.
//! Nothing reads the recording's rows at query time.

use super::{
    NAME,
    emit::{
        ADDITIONS, COUNTS, DEFINITIONS, Definition, FAMILY, HOLD, IDENTITY, KNOWN, Kept, REFUSALS,
        ROLES, SCHEMA, TEMPLATES, TOKENS, UNRESOLVED, WARNINGS, kind,
    },
};
use dashboard_base::{
    Source,
    prepared::{engine, prefix, published},
    products::{
        ContentOptions, CountryParams, Product,
        country_groups::HourSpan,
        idea_history::{
            self, Addition, Feed, IdeaCharacter, IdeaHistory, IdeaHistoryQuery, IdeaSpan,
            ScannedIdeas, mark, missing_template,
        },
        politics_content::PoliticsContent,
    },
};
use dashboard_series::{
    codec::Binary,
    Cell, EventOrder, EventQuery, KeyHandle, PageRead, Selection, Snapshot,
    family::{Directory, Scalar},
    query::EventView,
};
use dashboard_text::identity::NamingCountry;
use installed_content_core::characters::BuiltTemplate;
use replay_data::rows::ModifierDefinitionRow;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

fn malformed() -> io::Error {
    dashboard_core::invalid("A prepared ideas publication is malformed")
}

/// The JSON an event of `kind` holds.
fn json<P: PageRead, T: DeserializeOwned>(
    snapshot: &mut Snapshot<'_, P>,
    event: EventView,
    kind: u64,
) -> io::Result<T> {
    let EventView::Family(event) = event else {
        return Err(malformed());
    };
    if event.kind != kind || event.fields.len != Cell::SIZE as u64 {
        return Err(malformed());
    }
    let Cell::Text(blob) = snapshot.field(event.fields, 0)? else {
        return Err(malformed());
    };
    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Every event of `key` the publication accepts, in native order.
fn events<P: PageRead>(snapshot: &mut Snapshot<'_, P>, key: KeyHandle) -> io::Result<Vec<EventView>> {
    let accepted = snapshot.publication().accepted;
    let selection = [Selection {
        key,
        membership: None,
    }];
    let mut events = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.event_page(
            EventQuery {
                selection: &selection,
                native: accepted,
                as_of: None,
                order: EventOrder::Native,
            },
            cursor,
            1 << 20,
            |event| {
                events.push(event);
                Ok(())
            },
        )?;
        cursor = page.next;
        if cursor.is_none() {
            return Ok(events);
        }
    }
}

/// The JSON of `key`'s latest event of `kind` the publication accepts.
fn latest<P: PageRead, T: DeserializeOwned>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    kind: u64,
) -> io::Result<Option<T>> {
    let Some(key) = key else {
        return Ok(None);
    };
    let last = snapshot.publication().accepted.last;
    let selection = Selection {
        key,
        membership: None,
    };
    match snapshot.latest_event(selection, last, None)? {
        Some(event) => json(snapshot, event, kind).map(Some),
        None => Ok(None),
    }
}

/// `key`'s runs the publication accepts, each with its value, pieces of one run joined.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: KeyHandle,
) -> io::Result<Vec<(i32, i32, i128)>> {
    let accepted = snapshot.publication().accepted;
    let mut joined: Vec<(i32, i32, i128)> = Vec::new();
    for run in engine::runs(snapshot, key, accepted.first, accepted.last)? {
        match joined.last_mut() {
            Some(held)
                if !run.break_before
                    && held.2 == run.value
                    && held.1.checked_add(1) == Some(run.span.first) =>
            {
                held.1 = run.span.last;
            }
            _ => joined.push((run.span.first, run.span.last, run.value)),
        }
    }
    Ok(joined)
}

fn text(scalar: Option<&Scalar>) -> io::Result<String> {
    scalar
        .and_then(Scalar::text)
        .and_then(|text| std::str::from_utf8(text).ok())
        .map(str::to_owned)
        .ok_or_else(malformed)
}

/// The appointment a holding runs with: its seat and character; none for 0.
fn role(directory: &Directory, value: i128) -> io::Result<Option<(String, IdeaCharacter)>> {
    if value == 0 {
        return Ok(None);
    }
    let id = u64::try_from(value).map_err(|_| malformed())?;
    let Some(scalar) = directory.scalar(ROLES.checked_add(id).ok_or_else(malformed)?) else {
        return Err(malformed());
    };
    let [seat, character, token, name] = scalar.list() else {
        return Err(malformed());
    };
    Ok(Some((
        text(Some(seat))?,
        IdeaCharacter {
            id: character
                .integer()
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(malformed)?,
            token: text(Some(token))?,
            name: text(Some(name))?,
        },
    )))
}

/// The hours the inputs were complete, and the last of them.
fn known<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<Vec<(i32, i32, i128)>> {
    match directory.key(FAMILY, KNOWN, None) {
        Some(key) => runs(snapshot, key),
        None => Ok(Vec::new()),
    }
}

/// `actor`'s templates, a waiting one dated by `last`, the last hour the inputs were complete.
fn templates<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    last: Option<i32>,
) -> io::Result<BTreeMap<u32, Result<BuiltTemplate, String>>> {
    let kept: Option<Vec<(u32, Kept)>> =
        latest(snapshot, directory.key(actor, TEMPLATES, None), kind::TEMPLATES)?;
    kept.into_iter()
        .flatten()
        .map(|(id, kept)| {
            Ok((
                id,
                match kept {
                    Kept::Built(built) => Ok(built),
                    Kept::Refused(reason) => Err(reason),
                    Kept::Missing { complete, hour } => Err(missing_template(
                        id,
                        complete,
                        hour.or(last).ok_or_else(malformed)?,
                    )),
                },
            ))
        })
        .collect()
}

/// The recorded modifier definitions the publication accepts last, by name.
pub fn definitions<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<Option<BTreeMap<String, ModifierDefinitionRow>>> {
    let listed: Option<Vec<(String, Definition)>> = latest(
        snapshot,
        directory.key(FAMILY, DEFINITIONS, None),
        kind::DEFINITIONS,
    )?;
    Ok(listed.map(|listed| {
        listed
            .into_iter()
            .map(|(name, definition)| (name, definition.row()))
            .collect()
    }))
}

/// `tag`'s idea history as the publication holds it, its unobserved hours within `bounds`, the
/// branch's first and last hours.
pub fn country<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    bounds: Option<(i32, i32)>,
) -> io::Result<ScannedIdeas> {
    let known = known(snapshot, directory)?;
    // Every hour of the branch the inputs were not complete at, or not walked.
    let mut blind = Vec::new();
    if let Some((first, last)) = bounds {
        let mut next = first;
        for &(from, to, _) in &known {
            if next < from {
                mark(&mut blind, next, from - 1);
            }
            next = next.max(to.saturating_add(1));
        }
        if next <= last {
            mark(&mut blind, next, last);
        }
    }
    let Some(actor) = directory.actor(tag.as_bytes()) else {
        return Ok(ScannedIdeas {
            unobserved: blind,
            ..ScannedIdeas::default()
        });
    };
    let mut unobserved: Vec<HourSpan> = Vec::new();
    if let Some(key) = directory.key(actor, UNRESOLVED, None) {
        for (first_hour, last_hour, _) in runs(snapshot, key)? {
            unobserved.push(HourSpan {
                first_hour,
                last_hour,
            });
        }
    }
    unobserved.extend(blind);
    unobserved.sort_by_key(|span| span.first_hour);
    let mut merged = Vec::new();
    for span in unobserved {
        mark(&mut merged, span.first_hour, span.last_hour);
    }
    let mut spans = Vec::new();
    for &(id, key) in directory.dimensions(actor, HOLD) {
        let token = text(directory.scalar(TOKENS.checked_add(id).ok_or_else(malformed)?))?;
        for (first_hour, last_hour, value) in runs(snapshot, key)? {
            let held = role(directory, value)?;
            spans.push(IdeaSpan {
                token: token.clone(),
                first_hour,
                last_hour,
                slot: held.as_ref().map(|(seat, _)| seat.clone()),
                character: held.map(|(_, character)| character),
            });
        }
    }
    // As the scan opens them: by first hour, then by token.
    spans.sort_by(|left, right| {
        left.first_hour
            .cmp(&right.first_hour)
            .then_with(|| left.token.cmp(&right.token))
    });
    let mut identity_starts = Vec::new();
    let mut identity_naming = Vec::new();
    if let Some(key) = directory.key(actor, IDENTITY, None) {
        for event in events(snapshot, key)? {
            identity_starts.push(event.native_hour());
            identity_naming.push(json::<_, NamingCountry>(snapshot, event, kind::IDENTITY)?);
        }
    }
    let last = known.last().map(|&(_, last, _)| last);
    Ok(ScannedIdeas {
        spans,
        unobserved: merged,
        identity_starts,
        identity_naming,
        templates: templates(snapshot, directory, actor, last)?,
        warnings: latest(snapshot, directory.key(actor, WARNINGS, None), kind::WARNINGS)?
            .unwrap_or_default(),
        span_naming: Vec::new(),
        gap_starts: Vec::new(),
    })
}

/// An `idea-history` request answered from the family's publication through its gate's accepted
/// prefix, marked with it; before the family publishes the branch, the empty history.
pub fn reply(
    source: &Source,
    content: &PoliticsContent,
    options: &ContentOptions,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: IdeaHistoryQuery = serde_json::from_value(params.clone())?;
    let country = CountryParams {
        branch_id: params.branch_id,
        tag: params.tag,
    };
    country.validate().map_err(dashboard_core::invalid)?;
    let branch = country.branch();
    let through = params.through.or(params.hour);
    let prefix = prefix::read_or_empty(
        source,
        Some(options),
        IdeaHistory::KEY,
        branch,
        through,
        cancelled,
    )?;
    let replay = source.replay_ref();
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| dashboard_text::metadata::branch_bounds(replay, start));
    let opened = published::open_through(
        source,
        Some(options),
        NAME,
        SCHEMA,
        branch,
        (!prefix.coverage.complete).then_some(prefix.coverage.last_hour),
    )?;
    let (mut scanned, recorded) = match &opened {
        Some(opened) => opened.read(cancelled, |snapshot, directory| {
            Ok((
                self::country(snapshot, directory, &country.tag, bounds)?,
                definitions(snapshot, directory)?,
            ))
        })?,
        None => (ScannedIdeas::default(), None),
    };
    prefix.clip(&mut scanned.unobserved);
    for gap in replay
        .gaps
        .iter()
        .filter(|gap| gap.branch == branch && through.is_none_or(|hour| gap.first_hour <= hour))
    {
        scanned.unobserved.push(HourSpan {
            first_hour: gap.first_hour,
            last_hour: through.map_or(gap.last_hour, |hour| gap.last_hour.min(hour)),
        });
    }
    let mut gaps = std::mem::take(&mut scanned.unobserved);
    gaps.sort_by_key(|span| span.first_hour);
    for gap in gaps {
        mark(&mut scanned.unobserved, gap.first_hour, gap.last_hour);
    }
    let mut reply =
        idea_history::present(content, &country, &scanned, recorded.as_ref(), cancelled)?;
    source.append_reply_warnings(&mut reply.warnings, Some(options));
    let mut data = serde_json::to_value(reply)?;
    prefix.mark(&mut data)?;
    Ok(data)
}

/// The event feed's view of the publication: every addition, the advisors' templates of each
/// country appointing one, the refusals and the modifier definitions.
pub fn feed<P: PageRead>(snapshot: &mut Snapshot<'_, P>, directory: &Directory) -> io::Result<Feed> {
    let mut additions = Vec::new();
    if let Some(key) = directory.key(FAMILY, ADDITIONS, None) {
        for event in events(snapshot, key)? {
            additions.push(json::<_, Addition>(snapshot, event, kind::ADDITION)?);
        }
    }
    let known = known(snapshot, directory)?;
    let last = known.last().map(|&(_, last, _)| last);
    let mut templates = BTreeMap::new();
    for addition in &additions {
        if addition.span.character.is_none() || templates.contains_key(&addition.tag) {
            continue;
        }
        let actor = directory
            .actor(addition.tag.as_bytes())
            .ok_or_else(malformed)?;
        templates.insert(
            addition.tag.clone(),
            self::templates(snapshot, directory, actor, last)?,
        );
    }
    let count = |scalar: &Scalar| {
        scalar
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .ok_or_else(malformed)
    };
    let refusals = directory
        .scalar(REFUSALS)
        .map_or(&[][..], Scalar::list)
        .iter()
        .map(|pair| match pair.list() {
            [reason, rows] => Ok((text(Some(reason))?, count(rows)?)),
            _ => Err(malformed()),
        })
        .collect::<io::Result<_>>()?;
    let gap_additions = match directory.scalar(COUNTS).map(Scalar::list) {
        Some([gaps]) => count(gaps)?,
        Some(_) => return Err(malformed()),
        None => 0,
    };
    Ok(Feed {
        additions,
        observed: !known.is_empty(),
        templates,
        refusals,
        gap_additions,
        definitions: definitions(snapshot, directory)?,
    })
}
