
use recorder_derive_framework::{
    forces::{invalid, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::rows::{AirRegionActivityRow, RaidAirScoreRow, TableId};
use std::{
    collections::{HashMap, HashSet},
    io,
};

pub const DAY: i32 = replay_data::cadence::STAGGER_PERIOD;

const SCORES: [usize; 2] = columns::<RaidAirScoreRow, 2>(["friendly", "enemy"]);
const OWN: [usize; 5] = columns::<AirRegionActivityRow, 5>(["day", "friendly", "enemy", "friendly_total", "enemy_total"]);

const READS: &[Read] = &[Read { table: TableId::RaidAirScores, columns: Columns::These(&SCORES), watch: Watch::Changed, required: true }];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub day: i32,
    pub friendly: i64,
    pub enemy: i64,
    pub friendly_total: i64,
    pub enemy_total: i64,
}

impl Activity {
    pub fn before(&self, day: i32) -> (i128, i128) {
        let days = i128::from(day.saturating_sub(self.day).max(0));
        (i128::from(self.friendly_total) + i128::from(self.friendly) * days, i128::from(self.enemy_total) + i128::from(self.enemy) * days)
    }

    pub fn through(&self, day: i32) -> (i128, i128) {
        let (friendly, enemy) = self.before(day);
        (friendly + i128::from(self.friendly), enemy + i128::from(self.enemy))
    }

    fn next(&self, day: i32, friendly: i64, enemy: i64) -> io::Result<Self> {
        let (friendly_total, enemy_total) = self.before(day);
        let narrow = |total: i128| i64::try_from(total).map_err(|_| invalid("A region's summed air scores pass a 64-bit total"));
        Ok(Self { day, friendly, enemy, friendly_total: narrow(friendly_total)?, enemy_total: narrow(enemy_total)? })
    }

    pub fn read(view: &TableView<'_>, slot: u32) -> Option<Self> {
        let number = |column: usize| view.number(OWN[column], slot);
        Some(Self { day: i32::try_from(number(0)?).ok()?, friendly: number(1)?, enemy: number(2)?, friendly_total: number(3)?, enemy_total: number(4)? })
    }
}

fn part(view: &TableView<'_>, slot: u32) -> io::Result<(u128, u32, i64, i64)> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("RaidAirScores slot {slot} holds no row")))?;
    let score = |column: usize| {
        view.number(SCORES[column], slot).ok_or_else(|| invalid("RaidAirScores' scores are not recorded and the row cannot say so")).map(|score| score.max(0))
    };
    Ok((key, key as u32, score(0)?, score(1)?))
}

#[derive(Default)]
pub struct AirRegionActivity {
    rebuild: bool,
    parts: HashMap<u128, (u32, i64, i64)>,
    sums: HashMap<u32, (i64, i64)>,
    dirty: HashSet<u32>,
}

impl AirRegionActivity {
    fn add(&mut self, region: u32, friendly: i64, enemy: i64, sign: i64) {
        let sum = self.sums.entry(region).or_default();
        sum.0 += sign * friendly;
        sum.1 += sign * enemy;
        if *sum == (0, 0) {
            self.sums.remove(&region);
        }
        self.dirty.insert(region);
    }

    fn set(&mut self, key: u128, part: Option<(u32, i64, i64)>) {
        let before = match part {
            Some(part) => self.parts.insert(key, part),
            None => self.parts.remove(&key),
        };
        if before == part {
            return;
        }
        if let Some((region, friendly, enemy)) = before {
            self.add(region, friendly, enemy, -1);
        }
        if let Some((region, friendly, enemy)) = part {
            self.add(region, friendly, enemy, 1);
        }
    }
}

impl Writer for AirRegionActivity {
    fn name(&self) -> &'static str {
        "AirRegionActivity"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirRegionActivity)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let table = TableId::AirRegionActivity;
        let whole = hour.fresh || self.rebuild;
        let view = hour.table(TableId::RaidAirScores);
        if whole {
            self.parts.clear();
            self.sums.clear();
            self.rebuild = false;
            for slot in view.rows() {
                let (key, region, friendly, enemy) = part(&view, slot)?;
                self.set(key, Some((region, friendly, enemy)));
            }
        } else {
            for key in hour.deleted_keys(TableId::RaidAirScores) {
                self.set(key, None);
            }
            for slot in written(hour, TableId::RaidAirScores, &SCORES) {
                let (key, region, friendly, enemy) = part(&view, slot)?;
                self.set(key, Some((region, friendly, enemy)));
            }
        }
        let mut regions: Vec<u32> = std::mem::take(&mut self.dirty).into_iter().collect();
        if whole {
            regions.extend(recorder_derive_framework::forces::held(out, table).into_iter().map(|key| key as u32));
        }
        regions.sort_unstable();
        regions.dedup();
        let day = hour.hour.div_euclid(DAY);
        for region in regions {
            let (friendly, enemy) = self.sums.get(&region).copied().unwrap_or_default();
            let held = {
                let own = out.table(table);
                own.slot(u128::from(region)).and_then(|slot| Activity::read(&own, slot))
            };
            let row = match held {
                Some(held) if (held.friendly, held.enemy) == (friendly, enemy) => continue,
                Some(held) => held.next(day, friendly, enemy)?,
                None if (friendly, enemy) == (0, 0) => continue,
                None => Activity { day, friendly, enemy, ..Activity::default() },
            };
            out.set(table, u128::from(region), vec![Value::I32(row.day), Value::I64(row.friendly), Value::I64(row.enemy), Value::I64(row.friendly_total), Value::I64(row.enemy_total)]);
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("AirRegionActivity's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
