
use recorder_derive_framework::{
    forces::{invalid, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::rows::{AirOwnerDayRow, RaidAirScoreRow, TableId};
use std::io;

pub const DAY: i32 = replay_data::cadence::STAGGER_PERIOD;

const SCORES: [usize; 2] = columns::<RaidAirScoreRow, 2>(["friendly", "enemy"]);
const OWN: [usize; 5] = columns::<AirOwnerDayRow, 5>(["day", "friendly", "enemy", "friendly_total", "enemy_total"]);

const READS: &[Read] = &[Read { table: TableId::RaidAirScores, columns: Columns::These(&SCORES), watch: Watch::Changed, required: true }];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnerDay {
    pub day: i32,
    pub friendly: i64,
    pub enemy: i64,
    pub friendly_total: i64,
    pub enemy_total: i64,
}

impl OwnerDay {
    pub fn before(&self, day: i32) -> (i128, i128) {
        let days = i128::from(day.saturating_sub(self.day).max(0));
        (i128::from(self.friendly_total) + i128::from(self.friendly) * days, i128::from(self.enemy_total) + i128::from(self.enemy) * days)
    }

    pub fn through(&self, day: i32) -> (i128, i128) {
        let (friendly, enemy) = self.before(day);
        (friendly + i128::from(self.friendly), enemy + i128::from(self.enemy))
    }

    pub fn read(view: &TableView<'_>, slot: u32) -> Option<Self> {
        let number = |at: usize| view.number(OWN[at], slot);
        Some(Self { day: i32::try_from(number(0)?).ok()?, friendly: number(1)?, enemy: number(2)?, friendly_total: number(3)?, enemy_total: number(4)? })
    }
}

fn scores(view: &TableView<'_>, slot: u32) -> io::Result<(u128, (i64, i64))> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("RaidAirScores slot {slot} holds no row")))?;
    let score = |at: usize| view.number(SCORES[at], slot).ok_or_else(|| invalid("RaidAirScores' scores are not recorded and the row cannot say so")).map(|score| score.max(0));
    Ok((key, (score(0)?, score(1)?)))
}

#[derive(Default)]
pub struct AirOwnerDays;

impl AirOwnerDays {
    fn note(out: &mut Rows, key: u128, (friendly, enemy): (i64, i64), day: i32) -> io::Result<()> {
        let table = TableId::AirOwnerDays;
        let held = {
            let own = out.table(table);
            own.slot(key).and_then(|slot| OwnerDay::read(&own, slot))
        };
        let next = match held {
            Some(held) if (held.friendly, held.enemy) == (friendly, enemy) => return Ok(()),
            Some(held) => {
                let (friendly_total, enemy_total) = held.before(day);
                let narrow = |total: i128| i64::try_from(total).map_err(|_| invalid("An owner's summed air scores pass a 64-bit total"));
                OwnerDay { day, friendly, enemy, friendly_total: narrow(friendly_total)?, enemy_total: narrow(enemy_total)? }
            }
            None if (friendly, enemy) == (0, 0) => return Ok(()),
            None => OwnerDay { day, friendly, enemy, ..OwnerDay::default() },
        };
        out.set(table, key, vec![Value::I32(next.day), Value::I64(next.friendly), Value::I64(next.enemy), Value::I64(next.friendly_total), Value::I64(next.enemy_total)]);
        Ok(())
    }
}

impl Writer for AirOwnerDays {
    fn name(&self) -> &'static str {
        "AirOwnerDays"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirOwnerDays)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let day = hour.hour.div_euclid(DAY);
        let view = hour.table(TableId::RaidAirScores);
        if hour.fresh {
            let mut gone: std::collections::BTreeSet<u128> = recorder_derive_framework::forces::held(out, TableId::AirOwnerDays).into_iter().collect();
            for slot in view.rows() {
                let (key, now) = scores(&view, slot)?;
                gone.remove(&key);
                Self::note(out, key, now, day)?;
            }
            for key in gone {
                Self::note(out, key, (0, 0), day)?;
            }
            return Ok(());
        }
        for key in hour.deleted_keys(TableId::RaidAirScores) {
            Self::note(out, key, (0, 0), day)?;
        }
        for slot in written(hour, TableId::RaidAirScores, &SCORES) {
            let (key, now) = scores(&view, slot)?;
            Self::note(out, key, now, day)?;
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("AirOwnerDays' state holds nothing"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
