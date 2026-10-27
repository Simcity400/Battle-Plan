//! The regional-weather keyed family. WeatherRegions dates acquisition, not
//! native weather updates. Province temperature failures never hide known flags.

use super::admission;
use dashboard_base::prepared::facts::Credit;
use dashboard_base::prepared::{
    frame_changes::{FrameChanges, field_mask},
    prepass::{Extract, Family, Folded, HeldBytes},
};
use dashboard_keyed::{Delta, Scan, Values};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{Recorded, TableId, WeatherRegionRow, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
    io,
};

mod retained;

#[derive(Default, Serialize, Deserialize)]
pub struct Observer {
    temperature: super::temperature::Observation,
    #[serde(skip)]
    regions: BTreeMap<u32, Regional>,
    #[serde(skip)]
    regional_known: bool,
    #[serde(skip)]
    inputs: InputQueue,
    #[serde(skip)]
    previous: Option<(u64, i32)>,
}

impl Observer {
    /// Release live operands after a failed or cancelled fold.
    pub fn retire(&mut self) {
        self.temperature.retire();
        self.regions.clear();
        self.regional_known = false;
        self.inputs.clear();
        self.previous = None;
    }

    /// Bind once per source/content binding before folding a branch, including
    /// after restoring its catalogue. Content failures refuse temperature only.
    pub fn bind(
        &mut self,
        source: &dashboard_base::Source,
        options: Option<&dashboard_core::config::ContentOptions>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.previous = None;
        self.inputs.clear();
        let result = self.temperature.bind(source, options, cancelled);
        if result.is_err() {
            self.retire();
        }
        result
    }

    /// The same binding hook for a preparation driver that supplies a check
    /// callback rather than a cancellation flag. Call before fold and fallback.
    pub fn bind_checked(
        &mut self,
        source: &dashboard_base::Source,
        options: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.previous = None;
        self.inputs.clear();
        let result = self.temperature.bind_checked(source, options, check);
        if result.is_err() {
            self.retire();
        }
        result
    }

    pub fn temperature_status(&self) -> &str {
        self.temperature.status()
    }

    pub fn temperature_warnings(&self) -> Vec<String> {
        self.temperature.warnings()
    }
}

const GENERIC: Family = dashboard_keyed::block::family::<Observer>(
    "regional-weather",
    "regional-weather.extract",
    "regional-weather.sequential",
    "regional-weather.fallback",
);

pub static FAMILY: Family = Family {
    fields: None,
    start,
    ..GENERIC
};

struct Inputs {
    point: (u64, i32),
    previous: Option<(u64, i32)>,
    regions: Option<Vec<u128>>,
    provinces: Option<Vec<u128>>,
    _credit: Option<Credit>,
}

#[derive(Default)]
struct InputQueue {
    values: VecDeque<Inputs>,
    credit: Option<Credit>,
}

impl std::ops::Deref for InputQueue {
    type Target = VecDeque<Inputs>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl std::ops::DerefMut for InputQueue {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}
impl InputQueue {
    fn reserve_one(&mut self) -> io::Result<()> {
        if self.values.len() < self.values.capacity() {
            return Ok(());
        }
        let count = admission::add(self.values.capacity(), 1)?
            .checked_next_power_of_two()
            .ok_or_else(|| {
                admission::pressure("block key queue", io::ErrorKind::OutOfMemory.into())
            })?;
        let next = admission::admit(admission::bytes::<Inputs>(count)?, "block key queue")?;
        let mut values = VecDeque::new();
        values.try_reserve_exact(count).map_err(|error| {
            admission::pressure(
                "block key queue",
                io::Error::new(io::ErrorKind::OutOfMemory, error),
            )
        })?;
        values.append(&mut self.values);
        self.values = values;
        self.credit = Some(next);
        Ok(())
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
}

struct Extracted {
    facts: Box<dyn Any + Send>,
    inputs: InputQueue,
}

struct WeatherExtract {
    inner: Box<dyn Extract>,
    previous: Option<(u64, i32)>,
    inputs: InputQueue,
    input_bytes: usize,
}

fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(WeatherExtract {
        inner: (GENERIC.start)(format),
        previous: None,
        inputs: InputQueue::default(),
        input_bytes: 0,
    })
}

fn keys(changes: &FrameChanges, table: TableId, mask: u64) -> io::Result<Option<Vec<u128>>> {
    let Some((changed, deleted)) = changes.rows(table) else {
        return Ok(None);
    };
    let mut keys = Vec::new();
    let count = changed.iter().filter(|row| row.mask & mask != 0).count();
    keys.try_reserve_exact(admission::add(count, deleted.len())?)
        .map_err(|error| {
            admission::pressure(
                "block changed keys",
                io::Error::new(io::ErrorKind::OutOfMemory, error),
            )
        })?;
    keys.extend(
        changed
            .iter()
            .filter(|row| row.mask & mask != 0)
            .map(|row| row.key),
    );
    keys.extend_from_slice(deleted);
    keys.sort_unstable();
    keys.dedup();
    Ok(Some(keys))
}

impl Extract for WeatherExtract {
    fn observe(
        &mut self,
        replay: &replay::container::Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let point = (frame.branch, frame.hour);
        self.inputs.reserve_one()?;
        let key_count = |table| {
            changes.rows(table).map_or(Ok(0), |(changed, deleted)| {
                admission::add(changed.len(), deleted.len())
            })
        };
        let credit = admission::admit(
            admission::bytes::<u128>(admission::add(
                key_count(TableId::WeatherRegions)?,
                key_count(TableId::WeatherProvinces)?,
            )?)?,
            "block changed keys",
        )?;
        let inputs = Inputs {
            point,
            _credit: Some(credit),
            previous: self.previous.filter(|&(branch, hour)| {
                branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
            }),
            regions: keys(changes, TableId::WeatherRegions, u64::MAX)?,
            provinces: keys(
                changes,
                TableId::WeatherProvinces,
                field_mask(
                    TableId::WeatherProvinces,
                    &[
                        "temperature.temperature",
                        "temperature.drift",
                        "temperature.staged",
                        "temperature.working",
                    ],
                ),
            )?,
        };
        self.inner.observe(replay, frame, changes)?;
        self.input_bytes += inputs
            .regions
            .as_ref()
            .map_or(0, |keys| keys.capacity() * size_of::<u128>())
            + inputs
                .provinces
                .as_ref()
                .map_or(0, |keys| keys.capacity() * size_of::<u128>());
        self.inputs.push_back(inputs);
        self.previous = Some(point);
        Ok(())
    }

    fn bytes(&self) -> u64 {
        self.inner.bytes().saturating_add(
            (self.input_bytes + self.inputs.capacity() * size_of::<Inputs>()) as u64,
        )
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(Extracted {
            facts: self.inner.finish()?,
            inputs: self.inputs,
        }))
    }
}

pub fn fold(
    scan: &mut Scan<Observer>,
    facts: Box<dyn Any + Send>,
    replay: &replay::container::Replay,
    index: &replay::container::BlockIndex,
    observed: &mut dyn FnMut(i32, io::Result<()>) -> io::Result<bool>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let folded = (|| {
        let facts = facts
            .downcast::<Extracted>()
            .map_err(|_| io::Error::other("Weather pre-pass facts have another family"))?;
        scan.observer.inputs = facts.inputs;
        dashboard_keyed::block::fold(scan, facts.facts, replay, index, observed, check)
    })();
    scan.observer.inputs.clear();
    if folded.is_err() {
        scan.observer.retire();
    }
    folded
}

const MEASURES: [&str; 14] = [
    "light_rain",
    "heavy_rain",
    "snow",
    "blizzard",
    "sandstorm",
    "arctic_water",
    "mod:air_detection",
    "mod:naval_detection",
    "mod:air_attack",
    "mod:air_defence",
    "mod:army_attack_factor",
    "mod:breakthrough_factor",
    "mod:army_speed_factor",
    "mod:army_attack_speed_factor",
];

struct Regional {
    names: [String; 14],
    values: [Option<i128>; 14],
    _credit: Credit,
}

impl Regional {
    fn new(id: u32, values: [Option<i128>; 14]) -> io::Result<Self> {
        let names_bytes = MEASURES.iter().try_fold(0, |bytes, measure| {
            admission::add(bytes, 11 + measure.len())
        })?;
        let credit = admission::admit(
            admission::add(admission::tree_entry::<u32, Self>()?, names_bytes)?,
            "region names and scalars",
        )?;
        let mut names = std::array::from_fn(|_| String::new());
        for (name, measure) in names.iter_mut().zip(MEASURES) {
            *name = admission::name(id, measure)?;
        }
        Ok(Self {
            names,
            values,
            _credit: credit,
        })
    }
}

fn regional(row: &WeatherRegionRow) -> [Option<i128>; 14] {
    let mut values = [None; 14];
    for (at, value) in [
        row.light_rain,
        row.heavy_rain,
        row.snow,
        row.blizzard,
        row.sandstorm,
        row.arctic_water,
    ]
    .into_iter()
    .enumerate()
    {
        values[at] = Some(i128::from(value));
    }
    if let Recorded::Value(Some(value)) = row.air_detection {
        values[6] = Some(i128::from(value.unwrap_or(0)));
    }
    if row.map_modifiers_known == Recorded::Value(true) {
        for (at, value) in [row.naval_detection, row.air_attack, row.air_defence]
            .into_iter()
            .enumerate()
        {
            if let Recorded::Value(value) = value {
                values[7 + at] = Some(i128::from(value.unwrap_or(0)));
            }
        }
    }
    if let Recorded::Value(Some(combat)) = row.combat {
        for (at, value) in [
            combat.army_attack_factor,
            combat.breakthrough_factor,
            combat.army_speed_factor,
            combat.army_attack_speed_factor,
        ]
        .into_iter()
        .enumerate()
        {
            values[10 + at] = Some(i128::from(value.unwrap_or(0)));
        }
    }
    values
}

impl Observer {
    fn input(&mut self, frame: &DecodedFrame) -> Option<Inputs> {
        self.input_point((frame.branch, frame.hour))
    }

    fn input_point(&mut self, point: (u64, i32)) -> Option<Inputs> {
        while self
            .inputs
            .front()
            .is_some_and(|input| input.point != point)
        {
            self.inputs.pop_front();
        }
        let inputs = self
            .inputs
            .pop_front()
            .filter(|input| input.previous.is_some() && input.previous == self.previous);
        self.previous = Some(point);
        inputs
    }

    fn update_region(
        &mut self,
        id: u32,
        now: Option<&WeatherRegionRow>,
        delta: &mut Delta,
    ) -> io::Result<()> {
        self.update_values(id, now.map(regional), delta)
    }

    fn update_values(
        &mut self,
        id: u32,
        now: Option<[Option<i128>; 14]>,
        delta: &mut Delta,
    ) -> io::Result<()> {
        match now {
            Some(values) => {
                if let std::collections::btree_map::Entry::Vacant(entry) = self.regions.entry(id) {
                    entry.insert(Regional::new(id, [None; 14])?);
                }
                let held = self.regions.get_mut(&id).unwrap();
                for (at, &value) in values.iter().enumerate() {
                    if held.values[at] != value {
                        match value {
                            Some(value) => delta.set(held.names[at].clone(), value),
                            None => delta.remove(held.names[at].clone()),
                        }
                    }
                }
                held.values = values;
            }
            None => {
                if let Some(held) = self.regions.remove(&id) {
                    for (name, value) in held.names.into_iter().zip(held.values) {
                        if value.is_some() {
                            delta.remove(name);
                        }
                    }
                }
                if self.regions.is_empty() {
                    self.regions.clear();
                }
            }
        }
        Ok(())
    }
}

impl dashboard_keyed::Observer for Observer {
    const NAME: &'static str = "regional-weather";
    const TABLES: &'static [TableId] = &[TableId::WeatherRegions, TableId::WeatherProvinces];
    const READ: TableId = TableId::WeatherRegions;

    fn family() -> &'static Family {
        &FAMILY
    }

    fn keeper() -> Option<Box<dyn dashboard_keyed::block::Keeper<Self>>> {
        Some(Box::new(retained::Keeper::default()))
    }

    fn bind(
        &mut self,
        source: &dashboard_base::Source,
        content: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.bind_checked(source, content, check)
    }

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::named(
                TableId::WeatherRegions,
                &[
                    "light_rain",
                    "heavy_rain",
                    "snow",
                    "blizzard",
                    "sandstorm",
                    "arctic_water",
                    "combat",
                    "air_detection",
                    "map_modifiers_known",
                    "naval_detection",
                    "air_attack",
                    "air_defence",
                ],
            )?,
            TableFields::named(
                TableId::WeatherProvinces,
                &[
                    "temperature.temperature",
                    "temperature.drift",
                    "temperature.staged",
                    "temperature.working",
                    "temperature_pass",
                ],
            )?
            .when_read(),
        ])
    }

    fn changed(changes: &dashboard_base::prepared::frame_changes::FrameChanges) -> bool {
        // An equal pass packet at a later hour is not a certificate for that
        // hour. Check its anchor/pass length even when its rows did not change.
        changes.slot_read[TableId::WeatherProvinces.index()]
            || Self::TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()])
    }

    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let _ = self.input(frame);
        let temperature_known = self.temperature.observe(frame, values)?;
        self.regions.clear();
        let rows = frame.complete::<tables::WeatherRegions>();
        let regional_known = rows.is_ok();
        self.regional_known = regional_known;
        for row in rows.unwrap_or_default() {
            self.regions
                .insert(row.id, Regional::new(row.id, regional(row))?);
            for (flag, value) in [
                ("light_rain", row.light_rain),
                ("heavy_rain", row.heavy_rain),
                ("snow", row.snow),
                ("blizzard", row.blizzard),
                ("sandstorm", row.sandstorm),
                ("arctic_water", row.arctic_water),
            ] {
                // Names retain explicit zero flags so a region with no active
                // conditions remains distinct from an absent region record.
                values.push(format!("{}|{flag}", row.id), i128::from(value));
            }
            if let Recorded::Value(Some(value)) = row.air_detection {
                values.push(
                    format!("{}|mod:air_detection", row.id),
                    i128::from(value.unwrap_or(0)),
                );
            }
            if row.map_modifiers_known == Recorded::Value(true) {
                for (modifier, scalar) in [
                    ("naval_detection", row.naval_detection),
                    ("air_attack", row.air_attack),
                    ("air_defence", row.air_defence),
                ] {
                    if let Recorded::Value(value) = scalar {
                        values.push(
                            format!("{}|mod:{modifier}", row.id),
                            i128::from(value.unwrap_or(0)),
                        );
                    }
                }
            }
            if let Recorded::Value(Some(combat)) = row.combat {
                for (modifier, value) in [
                    ("army_attack_factor", combat.army_attack_factor),
                    ("breakthrough_factor", combat.breakthrough_factor),
                    ("army_speed_factor", combat.army_speed_factor),
                    ("army_attack_speed_factor", combat.army_attack_speed_factor),
                ] {
                    values.push(
                        format!("{}|mod:{modifier}", row.id),
                        i128::from(value.unwrap_or(0)),
                    );
                }
            }
        }
        Ok(regional_known || temperature_known)
    }
    fn observe_changes(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        _: &dashboard_keyed::delta::Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        if self.previous.is_none() {
            return Ok(None);
        }
        let input = self.input(frame);
        let temperature_known = self.temperature.observe_changes(
            frame,
            input.as_ref().and_then(|input| input.provinces.as_deref()),
            delta,
        )?;
        let rows = frame.complete::<tables::WeatherRegions>();
        let regional_known = rows.is_ok();
        let rows = rows.unwrap_or_default();
        let keys = input
            .as_ref()
            .and_then(|input| input.regions.as_deref())
            .filter(|_| regional_known && self.regional_known);
        match keys {
            Some(keys) => {
                for &key in keys {
                    if let Ok(id) = u32::try_from(key) {
                        let row = rows
                            .binary_search_by_key(&id, |row| row.id)
                            .ok()
                            .map(|at| &rows[at]);
                        self.update_region(id, row, delta)?;
                    }
                }
            }
            None => {
                let (mut ended, _credit) =
                    admission::vector(self.regions.len(), "retired region keys")?;
                ended.extend(
                    self.regions
                        .keys()
                        .copied()
                        .filter(|id| rows.binary_search_by_key(id, |row| row.id).is_err()),
                );
                for id in ended {
                    self.update_region(id, None, delta)?;
                }
                for row in rows {
                    self.update_region(row.id, Some(row), delta)?;
                }
            }
        }
        self.regional_known = regional_known;
        Ok(Some(regional_known || temperature_known))
    }

    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.inputs.capacity() * size_of::<Inputs>());
        for input in &self.inputs.values {
            if let Some(keys) = &input.regions {
                bytes.heap(keys);
            }
            if let Some(keys) = &input.provinces {
                bytes.heap(keys);
            }
        }
        bytes.tree::<u32, Regional>(self.regions.len());
        for row in self.regions.values() {
            for name in &row.names {
                bytes.heap(name);
            }
        }
        self.temperature.held(bytes);
    }
}

#[cfg(test)]
mod tests;
