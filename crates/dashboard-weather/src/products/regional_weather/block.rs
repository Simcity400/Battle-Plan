//! The regional-weather keyed family. WeatherRegions dates acquisition, not
//! native weather updates. Province temperature failures never hide known flags.

use super::admission;
use dashboard_base::prepared::{facts::Credit, prepass::HeldBytes};
use dashboard_keyed::{Delta, Values, delta::Names};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{Recorded, TableId, WeatherRegionRow, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

mod segment;

#[derive(Default, Serialize, Deserialize)]
pub struct Observer {
    temperature: super::temperature::Observation,
    #[serde(skip)]
    regions: BTreeMap<u32, Regional>,
    #[serde(skip)]
    regional_known: bool,
    /// The last hour observed.
    #[serde(skip)]
    previous: Option<(u64, i32)>,
}

impl Observer {
    /// Release live operands after a failed or cancelled fold.
    pub fn retire(&mut self) {
        self.temperature.retire();
        self.regions.clear();
        self.regional_known = false;
        self.previous = None;
    }

    /// Bind once per source/content binding before folding a branch, including
    /// after restoring its catalogue. Content failures refuse temperature only.
    #[cfg(test)]
    pub fn bind(
        &mut self,
        source: &dashboard_base::Source,
        options: Option<&dashboard_core::config::ContentOptions>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.previous = None;
        let result = self.temperature.bind(source, options, cancelled);
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
    /// The key the scan numbered each name with, once learnt.
    keys: [Option<u64>; 14],
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
            keys: [None; 14],
            values,
            _credit: credit,
        })
    }

    /// Pushes each value it holds, by key where the scan numbered its name.
    fn push(&self, values: &mut Values) {
        for at in 0..MEASURES.len() {
            if let Some(value) = self.values[at] {
                match self.keys[at] {
                    Some(key) => values.push_key(key, value),
                    None => values.push(self.names[at].clone(), value),
                }
            }
        }
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
    fn update_region(
        &mut self,
        id: u32,
        now: Option<&WeatherRegionRow>,
        names: &Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<()> {
        self.update_values(id, now.map(regional), names, delta)
    }

    fn update_values(
        &mut self,
        id: u32,
        now: Option<[Option<i128>; 14]>,
        names: &Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<()> {
        // A name the scan numbered changes by key; one it never numbered holds no value to end.
        let change = |held: &mut Regional, at: usize, value: Option<i128>, delta: &mut Delta| {
            if held.keys[at].is_none() {
                held.keys[at] = names.key(&held.names[at]);
            }
            match (held.keys[at], value) {
                (Some(key), Some(value)) => delta.set_key(key, value),
                (Some(key), None) => delta.remove_key(key),
                (None, Some(value)) => delta.set(held.names[at].clone(), value),
                (None, None) => {}
            }
        };
        match now {
            Some(values) => {
                if let std::collections::btree_map::Entry::Vacant(entry) = self.regions.entry(id) {
                    entry.insert(Regional::new(id, [None; 14])?);
                }
                let held = self.regions.get_mut(&id).unwrap();
                for (at, &value) in values.iter().enumerate() {
                    if held.values[at] != value {
                        change(&mut *held, at, value, &mut *delta);
                    }
                }
                held.values = values;
            }
            None => {
                if let Some(mut held) = self.regions.remove(&id) {
                    for at in 0..MEASURES.len() {
                        if held.values[at].is_some() {
                            change(&mut held, at, None, &mut *delta);
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

    fn native_keeper() -> Option<Box<dyn dashboard_keyed::native::Keeper<Self>>> {
        Some(Box::new(segment::Keeper::default()))
    }

    fn unspool_operands(
        input: &mut replay_spool::Reader<'_>,
    ) -> io::Result<Box<dyn dashboard_keyed::block::Operands<Self>>> {
        segment::unspool(input)
    }

    type Content = super::temperature::Resolved;

    fn content(
        source: &dashboard_base::Source,
        options: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Self::Content> {
        super::temperature::resolve(source, options, check)
    }

    fn bind(&mut self, content: &std::sync::Arc<Self::Content>) -> io::Result<()> {
        self.previous = None;
        let result = self.temperature.bind_resolved(content);
        if result.is_err() {
            self.retire();
        }
        result
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
        self.previous = Some((frame.branch, frame.hour));
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
        names: &Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        if self.previous.is_none() {
            return Ok(None);
        }
        self.previous = Some((frame.branch, frame.hour));
        let temperature_known = self.temperature.observe_changes(frame, None, delta)?;
        let rows = frame.complete::<tables::WeatherRegions>();
        let regional_known = rows.is_ok();
        let rows = rows.unwrap_or_default();
        let (mut ended, _credit) = admission::vector(self.regions.len(), "retired region keys")?;
        ended.extend(
            self.regions
                .keys()
                .copied()
                .filter(|id| rows.binary_search_by_key(id, |row| row.id).is_err()),
        );
        for id in ended {
            self.update_region(id, None, names, delta)?;
        }
        for row in rows {
            self.update_region(row.id, Some(row), names, delta)?;
        }
        self.regional_known = regional_known;
        Ok(Some(regional_known || temperature_known))
    }

    fn held(&self, bytes: &mut HeldBytes) {
        bytes.tree::<u32, Regional>(self.regions.len());
        for row in self.regions.values() {
            for name in &row.names {
                bytes.heap(name);
            }
        }
        self.temperature.held(bytes);
    }
}

