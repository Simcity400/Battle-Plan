use super::{FrameInputs, Inputs, StateInputs};
use crate::prepared::served::{self, Ask, Branch, Plan};
use replay::container::{BlockChange, Replay, TableFields};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Sample {
    hour: i32,
    provinces: Vec<u32>,
    states: Vec<u32>,
}

struct Held {
    provinces: BTreeMap<u32, (u32, Option<Vec<u8>>)>,
    states: BTreeMap<u32, StateInputs>,
    renames: (bool, String),
    states_availability: (Option<String>, String),
    reserved_tag: Result<String, String>,
}

impl Held {
    fn of(frame: &DecodedFrame) -> Self {
        let inputs = FrameInputs::new(frame);
        Self {
            provinces: inputs
                .provinces()
                .iter()
                .map(|(&id, &state)| (id, (state, inputs.rename(id).map(<[u8]>::to_vec))))
                .collect(),
            states: inputs
                .state_ids()
                .filter_map(|id| inputs.state(id).map(|state| (id, state.into_owned())))
                .collect(),
            renames: (inputs.renames_complete(), inputs.renames_gap().to_owned()),
            states_availability: (inputs.states_unrecorded(), inputs.states_gap().to_owned()),
            reserved_tag: inputs
                .reserved_tag()
                .map(str::to_owned)
                .map_err(str::to_owned),
        }
    }

    fn select_provinces(&self, ids: &[u32]) -> Vec<u32> {
        let mut selected = Vec::new();
        for stateless in [false, true] {
            if let Some(&id) = ids.iter().find(|id| {
                self.provinces
                    .get(id)
                    .is_some_and(|province| (province.0 == 0) == stateless)
            }) {
                selected.push(id);
            }
        }
        let remaining: Vec<u32> = ids
            .iter()
            .copied()
            .filter(|id| !selected.contains(id))
            .collect();
        selected.extend(served::spread(&remaining, served::KEYS - selected.len()));
        selected.sort_unstable();
        selected
    }

    fn sample(&self, hour: i32) -> Sample {
        Sample {
            hour,
            provinces: self.select_provinces(&self.provinces.keys().copied().collect::<Vec<_>>()),
            states: served::spread(
                &self.states.keys().copied().collect::<Vec<_>>(),
                served::KEYS,
            ),
        }
    }

    fn changed(&self, previous: &Self, hour: i32) -> Sample {
        let state_availability = self.states_availability != previous.states_availability;
        let states: BTreeSet<u32> = self
            .states
            .keys()
            .chain(previous.states.keys())
            .copied()
            .chain(
                self.provinces
                    .values()
                    .chain(previous.provinces.values())
                    .map(|province| province.0)
                    .filter(|&id| id != 0),
            )
            .filter(|id| state_availability || self.states.get(id) != previous.states.get(id))
            .collect();
        let ids: BTreeSet<u32> = self
            .provinces
            .keys()
            .chain(previous.provinces.keys())
            .copied()
            .collect();
        let mut provinces = Vec::new();
        let mut owned = Vec::new();
        for id in ids {
            let current = self.provinces.get(&id);
            let before = previous.provinces.get(&id);
            if current != before
                || self.renames != previous.renames
                || (self.reserved_tag != previous.reserved_tag
                    && current
                        .into_iter()
                        .chain(before)
                        .any(|province| province.0 == 0))
            {
                provinces.push(id);
            } else if current
                .into_iter()
                .chain(before)
                .any(|province| states.contains(&province.0))
            {
                owned.push(id);
            }
        }
        let mut selected = Vec::new();
        for held in [previous, self] {
            selected.extend(held.select_provinces(&provinces));
            selected.extend(held.select_provinces(&owned));
        }
        selected.sort_unstable();
        selected.dedup();
        Sample {
            hour,
            provinces: selected,
            states: served::spread(&states.into_iter().collect::<Vec<_>>(), served::KEYS),
        }
    }
}

struct Discovery {
    hours: std::iter::Peekable<std::vec::IntoIter<i32>>,
    held: Option<Held>,
    last: Option<i32>,
    samples: Vec<Sample>,
    changes: Vec<(Sample, Sample)>,
}

impl Discovery {
    fn new(branch: &Branch) -> Self {
        Self {
            hours: branch.hours().into_iter().peekable(),
            held: None,
            last: None,
            samples: Vec::new(),
            changes: Vec::new(),
        }
    }

    fn hour(&mut self, hour: i32) {
        if self.hours.peek().is_some_and(|&wanted| wanted <= hour) {
            while self.hours.peek().is_some_and(|&wanted| wanted <= hour) {
                self.hours.next();
            }
            if let Some(held) = &self.held {
                self.samples.push(held.sample(hour));
            }
        }
        self.last = Some(hour);
    }

    fn changed(&mut self, held: Held, hour: i32) {
        if let (Some(previous), Some(last)) = (&self.held, self.last) {
            let after = held.changed(previous, hour);
            if !after.provinces.is_empty() || !after.states.is_empty() {
                let before = Sample {
                    hour: last,
                    ..after.clone()
                };
                self.changes.push((before, after));
            }
        }
        self.held = Some(held);
        self.hour(hour);
    }

    fn finish(mut self) -> Vec<Sample> {
        for (before, after) in served::spread(&self.changes, served::HOURS / 2) {
            self.samples.extend([before, after]);
        }
        self.samples.sort_by(|left, right| {
            (left.hour, &left.provinces, &left.states).cmp(&(
                right.hour,
                &right.provinces,
                &right.states,
            ))
        });
        self.samples.dedup();
        self.samples
    }
}

fn samples(replay: &Replay, branch: &Branch) -> io::Result<Vec<Sample>> {
    let fields = [
        TableFields::named(TableId::Provinces, &["state"])?,
        TableFields::named(TableId::ProvinceNames, &["name"])?,
        TableFields::named(TableId::States, &["owner", "custom_name", "owner_name"])?,
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        TableFields::named(TableId::CountryLinks, &["canonical_slot"])?,
        TableFields::named(
            TableId::CountryAppearance,
            &["original_tag", "original_tag_reference"],
        )?,
        TableFields::named(
            TableId::ReservedCountryNaming,
            &["tag", "original_tag", "original_tag_reference"],
        )?,
    ];
    let mut discovery = Discovery::new(branch);
    let recorded = |hour| {
        !branch
            .gaps
            .iter()
            .any(|&(from, to)| (from..=to).contains(&hour))
    };
    for block in replay
        .index
        .iter()
        .filter(|block| block.branch == branch.id)
    {
        replay.read_block_changes(
            block,
            &fields,
            |change| {
                match change {
                    BlockChange::Changed { frame, .. } => {
                        let held = Held::of(frame);
                        if recorded(frame.hour) {
                            discovery.changed(held, frame.hour);
                        } else {
                            discovery.held = Some(held);
                            discovery.last = None;
                        }
                    }
                    BlockChange::Quiet(quiet) => {
                        for &hour in quiet.hours().iter().filter(|&&hour| recorded(hour)) {
                            discovery.hour(hour);
                        }
                    }
                }
                Ok(())
            },
            || Ok(()),
        )?;
    }
    Ok(discovery.finish())
}

fn provinces(ask: &mut dyn Ask, id: &str, places: &[Value]) -> io::Result<()> {
    if places.is_empty() {
        return Ok(());
    }
    for land in [None, Some(false), Some(true)] {
        let mut params = json!({ "branchId": id, "places": places, "states": [] });
        if let Some(land) = land {
            params["land"] = json!(land);
        }
        ask.ask("place-names", params)?;
    }
    Ok(())
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let samples = samples(ask.recording()?, &branch)?;
        let mut dated = Vec::new();
        for sample in samples {
            let places: Vec<Value> = sample
                .provinces
                .iter()
                .map(|province| json!({ "provinceId": province, "hour": sample.hour }))
                .collect();
            provinces(ask, &id, &places)?;
            dated.extend(places);
            if !sample.states.is_empty() {
                let states: Vec<Value> = sample
                    .states
                    .iter()
                    .map(|state| json!({ "stateId": state, "hour": sample.hour }))
                    .collect();
                ask.ask(
                    "place-names",
                    json!({ "branchId": id, "places": [], "states": states }),
                )?;
            }
        }
        dated.sort_by_key(|place| (place["hour"].as_i64(), place["provinceId"].as_u64()));
        dated.dedup();
        for places in dated.chunks(served::BATCH) {
            provinces(ask, &id, places)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
