//! A place-names reply from the family's publication ([`super::emit`]): each requested hour's
//! provinces and states as their latest events by then hold them ([`Held`]), named by the same
//! ladder the frame reader names them by.

use super::{
    PlaceName, PlaceNamesParams, PlaceNamesReply, PlaceStateName,
    emit::{FAMILY, OBSERVED, PROVINCE, STATE, TABLES, kind},
    facts::{Province, Tables},
    named_place, named_state, place_name, resolve, state_name,
};
use dashboard_base::{
    Source,
    metadata::check_cancel,
    products::{
        LoadContext,
        place_names::{Inputs, NOT_RECORDED, Owner, StateInputs},
    },
};
use dashboard_series::{
    Blob, Cell, Event, Observation, PageRead, Selection, Snapshot,
    codec::{Binary, Decode},
    family::Directory,
    query::EventView,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};

fn malformed() -> io::Error {
    dashboard_core::invalid("A place-names publication event is malformed")
}

fn bytes<P: PageRead>(snapshot: &mut Snapshot<'_, P>, blob: Blob) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(bytes)
}

fn text<P: PageRead>(snapshot: &mut Snapshot<'_, P>, cell: Cell) -> io::Result<String> {
    match cell {
        Cell::Text(blob) => String::from_utf8(bytes(snapshot, blob)?).map_err(|_| malformed()),
        _ => Err(malformed()),
    }
}

/// `measure`'s (of `id`) latest event by `hour` and its cells.
fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    (measure, id): (u64, Option<u32>),
    hour: i32,
) -> io::Result<Option<(Event, Vec<Cell>)>> {
    let Some(key) = directory.key(FAMILY, measure, id.map(u64::from)) else {
        return Ok(None);
    };
    let selection = Selection {
        key,
        membership: None,
    };
    let Some(EventView::Family(event)) = snapshot.latest_event(selection, hour, None)? else {
        return Ok(None);
    };
    let fields = bytes(snapshot, event.fields)?;
    let cells = fields
        .chunks_exact(Cell::SIZE)
        .map(|chunk| Cell::decode(&mut Decode::new(chunk)))
        .collect::<io::Result<_>>()?;
    Ok(Some((event, cells)))
}

/// Whether the family walked `hour`.
pub fn walked<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hour: i32,
) -> io::Result<bool> {
    let Some(key) = directory.key(FAMILY, OBSERVED, None) else {
        return Ok(false);
    };
    Ok(matches!(
        snapshot.value_at(key, hour)?,
        Observation::Value { .. }
    ))
}

/// What the family holds at a walked hour of the provinces and states a reply names, and of
/// the states those provinces are in.
pub struct Held {
    hour: i32,
    tables: Tables,
    provinces: HashMap<u32, Option<Province>>,
    states: HashMap<u32, Option<StateInputs>>,
}

impl Held {
    pub fn at<P: PageRead>(
        snapshot: &mut Snapshot<'_, P>,
        directory: &Directory,
        hour: i32,
        provinces: &[u32],
        states: &[u32],
    ) -> io::Result<Self> {
        let Some((event, cells)) = latest(snapshot, directory, (TABLES, None), hour)? else {
            return Err(malformed());
        };
        let [renames, renames_gap, mode, states_gap, known, reserved] = cells[..] else {
            return Err(malformed());
        };
        let (Cell::Boolean(renames_complete), Cell::Boolean(known)) = (renames, known) else {
            return Err(malformed());
        };
        if event.kind != kind::TABLES {
            return Err(malformed());
        }
        let reserved = text(snapshot, reserved)?;
        let tables = Tables {
            renames_complete,
            renames_gap: text(snapshot, renames_gap)?,
            states_unrecorded: match mode {
                Cell::Null => None,
                mode => Some(text(snapshot, mode)?),
            },
            states_gap: text(snapshot, states_gap)?,
            reserved: if known { Ok(reserved) } else { Err(reserved) },
        };
        let mut held = Self {
            hour,
            tables,
            provinces: HashMap::new(),
            states: HashMap::new(),
        };
        for &id in provinces {
            if held.provinces.contains_key(&id) {
                continue;
            }
            let province = match latest(snapshot, directory, (PROVINCE, Some(id)), hour)? {
                None => None,
                Some((event, _)) if event.kind == kind::GONE => None,
                Some((event, cells)) if event.kind == kind::PROVINCE => match cells[..] {
                    [Cell::Unsigned(state), rename] => Some(Province {
                        state: u32::try_from(state).map_err(|_| malformed())?,
                        rename: match rename {
                            Cell::Null => None,
                            Cell::Text(blob) => Some(bytes(snapshot, blob)?.into_boxed_slice()),
                            _ => return Err(malformed()),
                        },
                    }),
                    _ => return Err(malformed()),
                },
                Some(_) => return Err(malformed()),
            };
            held.provinces.insert(id, province);
        }
        let in_states: Vec<u32> = held
            .provinces
            .values()
            .flatten()
            .map(|province| province.state)
            .filter(|&state| state != 0)
            .collect();
        for &id in states.iter().chain(&in_states) {
            if held.states.contains_key(&id) {
                continue;
            }
            let state = match latest(snapshot, directory, (STATE, Some(id)), hour)? {
                None => None,
                Some((event, _)) if event.kind == kind::GONE => None,
                Some((event, cells)) if event.kind == kind::STATE => match cells[..] {
                    [Cell::Text(custom), Cell::Text(owner_name), Cell::Unsigned(owner), of] => {
                        Some(StateInputs {
                            custom: bytes(snapshot, custom)?,
                            owner_name: bytes(snapshot, owner_name)?,
                            owner: match (owner, of) {
                                (0, of) => Owner::Tag(Ok(text(snapshot, of)?)),
                                (1, of) => Owner::Tag(Err(text(snapshot, of)?)),
                                (2, Cell::Unsigned(link)) => Owner::Unlinked(
                                    u32::try_from(link).map_err(|_| malformed())?,
                                ),
                                _ => return Err(malformed()),
                            },
                        })
                    }
                    _ => return Err(malformed()),
                },
                Some(_) => return Err(malformed()),
            };
            held.states.insert(id, state);
        }
        Ok(held)
    }
}

impl Inputs for Held {
    fn hour(&self) -> i32 {
        self.hour
    }

    fn province(&self, id: u32) -> Option<u32> {
        self.provinces
            .get(&id)?
            .as_ref()
            .map(|province| province.state)
    }

    fn rename(&self, id: u32) -> Option<&[u8]> {
        self.provinces.get(&id)?.as_ref()?.rename.as_deref()
    }

    fn renames_complete(&self) -> bool {
        self.tables.renames_complete
    }

    fn renames_gap(&self) -> &str {
        &self.tables.renames_gap
    }

    fn states_unrecorded(&self) -> Option<String> {
        self.tables.states_unrecorded.clone()
    }

    fn state(&self, id: u32) -> Option<Cow<'_, StateInputs>> {
        self.states.get(&id)?.as_ref().map(Cow::Borrowed)
    }

    fn states_gap(&self) -> &str {
        &self.tables.states_gap
    }

    fn reserved_tag(&self) -> Result<&str, &str> {
        self.tables
            .reserved
            .as_deref()
            .map_err(String::as_str)
    }
}

/// The reply to `params` on `branch` from the family's publication; none while an hour it
/// names is past what the family has accepted.
pub fn reply<P: PageRead>(
    source: &Source,
    context: &LoadContext,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    branch: u64,
    params: &PlaceNamesParams,
    cancelled: &AtomicBool,
) -> io::Result<Option<PlaceNamesReply>> {
    let accepted = snapshot.publication().accepted.last;
    let mut hours = BTreeMap::<i32, (Vec<usize>, Vec<usize>)>::new();
    for (at, request) in params.places.iter().enumerate() {
        hours.entry(request.hour).or_default().0.push(at);
    }
    for (at, request) in params.states.iter().enumerate() {
        hours.entry(request.hour).or_default().1.push(at);
    }
    if hours.keys().next_back().is_some_and(|&last| last > accepted) {
        return Ok(None);
    }
    let replay = source.replay_ref();
    let reply = resolve(source, params, context, true, |cat, names, regions| {
        let unrecorded = || (None, Some(NOT_RECORDED.to_owned()));
        let mut places: Vec<PlaceName> = params
            .places
            .iter()
            .map(|request| named_place(request, unrecorded()))
            .collect();
        let mut states: Vec<PlaceStateName> = params
            .states
            .iter()
            .map(|request| named_state(request, unrecorded()))
            .collect();
        for (&hour, (place_ats, state_ats)) in &hours {
            check_cancel(cancelled)?;
            // An hour a recorded gap covers is unobserved.
            if replay.gaps.iter().any(|gap| {
                gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour
            }) {
                let reason = format!("Not in this recording: {}", replay.gap_reason(branch, hour)?);
                for &at in place_ats {
                    places[at].missing = Some(reason.clone());
                }
                for &at in state_ats {
                    states[at].missing = Some(reason.clone());
                }
                continue;
            }
            if !walked(snapshot, directory, hour)? {
                continue;
            }
            let provinces: Vec<u32> = place_ats
                .iter()
                .map(|&at| params.places[at].province_id)
                .collect();
            let wanted: Vec<u32> = state_ats
                .iter()
                .map(|&at| params.states[at].state_id)
                .collect();
            let held = Held::at(snapshot, directory, hour, &provinces, &wanted)?;
            for &at in place_ats {
                let request = &params.places[at];
                places[at] = named_place(
                    request,
                    place_name(&held, request.province_id, cat, names, regions, params.land),
                );
            }
            for &at in state_ats {
                let request = &params.states[at];
                states[at] = named_state(request, state_name(&held, request.state_id, cat, names));
            }
        }
        Ok((places, states))
    })?;
    super::checked(reply).map(Some)
}
