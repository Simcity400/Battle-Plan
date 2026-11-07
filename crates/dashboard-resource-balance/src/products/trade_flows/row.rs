//! The trade flows the row recording's frames hold, read whole hour by hour: the oracle the
//! family's columnar preparation and its replies are held to.

use super::{
    Flow, Identity, PartialSpan, Run, ScannedFlows, Terms, TradeCountryIdentity, TradeFlowParams,
    TradeFlowsReply, identity, sample,
};
use crate::products::resource_balance::mark;
use dashboard_base::products::{
    country_history::{self, RecordedHistories},
    resources::Learned,
    territory::CountryReads,
};
use dashboard_base::source::Source;
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::validate_campaign;
use replay::campaign::Replay;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// An exchange's seller and buyer links, its resource and its terms.
type Operand = (u32, u32, u32, Terms);

fn exchange_operand(row: &replay_data::rows::ResourceExchangeRow) -> Operand {
    (
        row.exporter,
        row.importer,
        row.resource,
        Terms {
            delivered: row.delivered,
            factories: row.assigned_factories,
            assigned: row.convoys_assigned,
            required: row.convoys_required,
            deals: 1,
        },
    )
}

/// Adds a known hour with `deals` left out, joining it to the last span when it
/// follows it with the same count.
fn mark_partial(spans: &mut Vec<PartialSpan>, hour: i32, deals: u32) {
    match spans.last_mut() {
        Some(span) if span.deals == deals && span.last_hour.checked_add(1) == Some(hour) => {
            span.last_hour = hour
        }
        Some(span) if span.last_hour >= hour => {}
        _ => spans.push(PartialSpan {
            first_hour: hour,
            last_hour: hour,
            deals,
        }),
    }
}

/// What a frame's flows are read from: its countries, its exchanges in recorded order and
/// the resources its definitions and tokens name, each none where its tables are not complete.
#[derive(Default)]
pub(super) struct Inputs {
    countries: Option<CountryReads>,
    exchanges: Option<Vec<Operand>>,
    pub(super) names: Option<Learned>,
}

impl Inputs {
    pub(super) fn read(frame: &DecodedFrame) -> Self {
        Self {
            countries: frame
                .complete::<tables::Countries>()
                .ok()
                .map(|rows| CountryReads::new(frame, rows)),
            exchanges: frame
                .complete::<tables::ResourceExchanges>()
                .ok()
                .map(|rows| rows.iter().map(exchange_operand).collect()),
            names: frame
                .complete::<tables::ResourceDefinitions>()
                .ok()
                .zip(frame.complete::<tables::LexicalTokens>().ok())
                .map(|(definitions, lexical)| {
                    let mut names = Learned::default();
                    names.learn(definitions, lexical);
                    names
                }),
        }
    }

    /// Each holder slot's first tag in tag order, with its country id.
    pub(super) fn identities(&self) -> HashMap<u32, (&str, u32)> {
        let mut ids = HashMap::new();
        if let Some(countries) = &self.countries {
            for (tag, identity) in countries.holders() {
                if let Some((slot, id)) = *identity {
                    ids.entry(slot).or_insert((tag.as_str(), id));
                }
            }
        }
        ids
    }
}

/// The read's flows summed in recorded order, and the deals it left out by seller and buyer;
/// none where its tables are not complete. Signed partial sums refuse as they overflow.
#[allow(clippy::type_complexity)]
pub(super) fn flow_rows<'a>(
    inputs: &'a Inputs,
    ids: &HashMap<u32, (&'a str, u32)>,
    wanted: Option<&BTreeSet<Identity>>,
    rejected: &mut Vec<(Option<(Identity, Identity)>, u32)>,
) -> io::Result<Option<BTreeMap<(Identity, Identity, u32), (&'a str, &'a str, Terms)>>> {
    let (Some(exchanges), Some(names), Some(countries)) =
        (&inputs.exchanges, &inputs.names, &inputs.countries)
    else {
        return Ok(None);
    };
    let slots = countries.slots();
    let mut read = BTreeMap::new();
    for &(exporter, importer, resource, operand) in exchanges {
        let side = |raw| {
            slots
                .get(&raw)
                .and_then(|slot| ids.get(slot).map(|(tag, id)| (*slot, *tag, *id)))
        };
        let (Some((seller_slot, seller, seller_id)), Some((buyer_slot, buyer, buyer_id))) =
            (side(exporter), side(importer))
        else {
            rejected.push((None, 1));
            continue;
        };
        let seller_identity = (seller_slot, seller_id);
        let buyer_identity = (buyer_slot, buyer_id);
        if wanted
            .is_some_and(|ids| !ids.contains(&seller_identity) && !ids.contains(&buyer_identity))
        {
            continue;
        }
        if !names.contains(resource) {
            rejected.push((Some((seller_identity, buyer_identity)), 1));
            continue;
        }
        let terms = read
            .entry((seller_identity, buyer_identity, resource))
            .or_insert((seller, buyer, Terms::default()));
        terms.2 = terms
            .2
            .add(operand)
            .ok_or_else(|| dashboard_core::invalid("recorded trade terms overflow their sum"))?;
    }
    Ok(Some(read))
}

/// Scans one branch for every trade deal, or those of `tags` as seller or buyer.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tags: Option<&[String]>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedFlows> {
    let history = country_history::scan_histories(replay, branch, false, &[], cancelled)?;
    scan_with_history(replay, branch, tags, &history, cancelled)
}

fn scan_with_history(
    replay: &Replay,
    branch: u64,
    tags: Option<&[String]>,
    history: &RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<ScannedFlows> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::ResourceDefinitions,
        TableId::ResourceExchanges,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Trade request cancelled");
    let mut scan = Scan {
        wanted: tags.map(|tags| country_history::selected_identities(history, tags)),
        ..Default::default()
    };
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    scan.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    scan.result.country_identities = history
        .countries
        .iter()
        .filter(|(_, country)| !country.identities.is_empty())
        .map(|(tag, country)| TradeCountryIdentity {
            tag: tag.clone(),
            runs: country.identities.as_ref().clone(),
        })
        .collect();
    Ok(scan.result)
}

#[derive(Default)]
struct Scan {
    result: ScannedFlows,
    learned: Learned,
    index: HashMap<(Identity, Identity, u32), usize>,
    known: bool,
    carried: Vec<(u32, String, u32)>,
    last_hour: Option<i32>,
    wanted: Option<BTreeSet<Identity>>,
    inputs: Inputs,
}

impl Scan {
    fn observe(&mut self, decoded: &DecodedFrame) -> io::Result<()> {
        let requested = self.wanted.as_ref().map(|ids| {
            let mut requested = ids
                .iter()
                .map(|&id| (identity(id), "actor".into()))
                .collect::<Vec<_>>();
            if requested.is_empty() {
                requested.push((String::new(), String::new()));
            }
            requested
        });
        self.inputs = Inputs::read(decoded);
        let sampled = sample::sample_inputs(decoded, requested.as_deref(), &self.inputs)?;
        self.observe_sampled(decoded, &sampled)
    }

    fn observe_sampled(
        &mut self,
        decoded: &DecodedFrame,
        sampled: &dashboard_base::prepared::query::progressive::Samples,
    ) -> io::Result<()> {
        let Self {
            result,
            learned,
            index,
            known,
            carried,
            last_hour,
            wanted,
            inputs,
        } = self;
        let hour = decoded.hour;
        let continues = last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        *last_hour = Some(hour);
        if !continues {
            *known = false;
        }
        if sampled
            .get(&(String::new(), "known".into()))
            .and_then(|(_, value)| *value)
            .is_none()
        {
            // A read that cannot say who trades what: unknown from here.
            *known = false;
            return Ok(());
        }
        if let Some(names) = &inputs.names {
            learned.learn_names(names);
        }
        let ids: HashMap<u32, (&str, u32)> = sampled
            .iter()
            .filter_map(|((tag, field), (holder, _))| {
                (field == "holder")
                    .then_some(holder.map(|(slot, id)| (slot, (tag.as_str(), id))))
                    .flatten()
            })
            .collect();
        let mut rejected = Vec::new();
        let mut read = BTreeMap::new();
        for ((token, field), (_, amount)) in sampled {
            if field == "rejected" {
                let pair: Option<(Identity, Identity)> = serde_json::from_str(token)?;
                if pair.is_none_or(|(seller, buyer)| {
                    wanted
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&seller) || ids.contains(&buyer))
                }) {
                    rejected.push((
                        pair,
                        u32::try_from(amount.unwrap_or(0)).map_err(|_| {
                            dashboard_core::invalid("Invalid sampled rejected deal count")
                        })?,
                    ));
                }
            }
            if field != "delivered" {
                continue;
            }
            let (seller, buyer, resource) = sample::pair(token)?;
            if wanted
                .as_ref()
                .is_some_and(|ids| !ids.contains(&seller) && !ids.contains(&buyer))
            {
                continue;
            }
            let mut amounts = [0; 5];
            for (field, amount) in sample::FIELDS.iter().zip(&mut amounts) {
                *amount = sampled
                    .get(&(token.clone(), (*field).to_owned()))
                    .and_then(|(_, value)| *value)
                    .ok_or_else(|| dashboard_core::invalid("Missing native sampled trade terms"))?;
            }
            read.insert(
                (seller, buyer, resource),
                (ids[&seller.0].0, ids[&buyer.0].0, sample::terms(amounts)?),
            );
        }
        let same = carried
            .iter()
            .all(|(slot, tag, id)| ids.get(slot) == Some(&(tag.as_str(), *id)));
        // The slots this read's flows name, with the country each held.
        let named: BTreeSet<u32> = read
            .keys()
            .flat_map(|(seller, buyer, _)| [seller.0, buyer.0])
            .collect();
        // Deals this read lists but cannot attribute: the hour is known only in part.
        let mut skipped = 0_u32;
        for (pair, count) in &rejected {
            if pair.is_some() {
                result.unnamed += *count as usize;
            } else {
                result.unresolved += *count as usize;
            }
            skipped = skipped.saturating_add(*count);
        }
        result.reads.push(hour);
        mark(&mut result.known, hour);
        if skipped > 0 {
            mark_partial(&mut result.partial, hour, skipped);
        }
        let was_known = *known && same;
        *known = true;
        *carried = named
            .iter()
            .map(|slot| {
                let (tag, id) = ids[slot];
                (*slot, tag.to_owned(), id)
            })
            .collect();
        for ((seller_identity, buyer_identity, resource), (seller, buyer, terms)) in read {
            let key = (seller_identity, buyer_identity, resource);
            let flow = *index.entry(key).or_insert_with(|| {
                result.flows.push(Flow {
                    seller: seller.to_owned(),
                    buyer: buyer.to_owned(),
                    seller_identity,
                    buyer_identity,
                    resource,
                    runs: Vec::new(),
                });
                result.flows.len() - 1
            });
            let runs = &mut result.flows[flow].runs;
            match runs.last_mut() {
                Some(run)
                    if was_known
                        && run.last_hour.checked_add(1) == Some(hour)
                        && run.terms == terms
                        && run.seller_tag == seller
                        && run.buyer_tag == buyer =>
                {
                    run.last_hour = hour
                }
                _ => {
                    runs.push(Run {
                        first_hour: hour,
                        last_hour: hour,
                        terms,
                        seller_tag: seller.to_owned(),
                        buyer_tag: buyer.to_owned(),
                    });
                }
            }
        }
        result.resources = learned.list();
        Ok(())
    }
}

/// Each branch's latest scan, by the countries it was scanned for (none for the whole world).
#[derive(Default)]
struct Cache {
    latest: BTreeMap<u64, (Option<Vec<String>>, Arc<ScannedFlows>)>,
}

/// The whole-recording reply, from the row frames.
pub(super) fn load(
    source: &Source,
    params: &TradeFlowParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<TradeFlowsReply> {
    let (branch, countries) = (params.branch(), params.countries());
    let state = source.state::<Cache>();
    let cached = state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .latest
        .get(&branch)
        .filter(|(cached, _)| *cached == countries)
        .map(|(_, scanned)| Arc::clone(scanned));
    let scanned = match cached {
        Some(scanned) => scanned,
        None => {
            let history =
                country_history::branch_histories(source, params.branch(), false, &[], cancelled)?;
            let scanned = Arc::new(scan_with_history(
                &source.replay(),
                branch,
                countries.as_deref(),
                &history,
                cancelled,
            )?);
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache.latest.insert(branch, (countries, Arc::clone(&scanned)));
            scanned
        }
    };
    super::reply(source, params, context, &scanned, cancelled)
}
