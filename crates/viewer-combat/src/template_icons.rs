use replay_data::{
    rows::DivisionRow, rows::LandCombatDetailsRow, rows::LandCombatDivisionRow, rows::TableSchema,
    rows::tables, scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, fmt, io};
use dashboard_text::table_access::{Available, rows};

const EXILE_ORIGIN: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Gap<'a> {
    Absent,
    Unreached,
    Unavailable(&'a str),
    NotRecorded(CaptureMode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Refused<'a> {
    Division { division: u64, gap: Gap<'a> },
    CountryLink { country: u32, gap: Gap<'a> },
}

impl fmt::Display for Refused<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (table, key, gap) = match self {
            Self::Division { division, gap } => ("Divisions", format!("division {division}"), gap),
            Self::CountryLink { country, gap } => {
                ("CountryLinks", format!("country {country}"), gap)
            }
        };
        match gap {
            Gap::Absent => write!(f, "the {table} table has no row for {key}"),
            Gap::Unreached => write!(f, "the truncated {table} table did not reach {key}"),
            Gap::Unavailable(reason) => {
                write!(
                    f,
                    "the {table} table is unavailable ({reason}), so {key} is unknown"
                )
            }
            Gap::NotRecorded(mode) => write!(
                f,
                "the {table} table is not recorded in a {mode} recording, so {key} is unknown"
            ),
        }
    }
}

fn any<'a, T>(
    candidates: impl IntoIterator<Item = T>,
    mut test: impl FnMut(T) -> Result<bool, Refused<'a>>,
) -> Result<bool, Refused<'a>> {
    let mut undecided = None;
    for candidate in candidates {
        match test(candidate) {
            Ok(true) => return Ok(true),
            Ok(false) => (),
            Err(refused) => {
                undecided.get_or_insert(refused);
            }
        }
    }
    undecided.map_or(Ok(false), Err)
}

struct Keyed<'a, K, V> {
    rows: BTreeMap<K, V>,
    gap: Gap<'a>,
}

impl<'a, K: Ord, V: Copy> Keyed<'a, K, V> {
    fn read<T: TableSchema>(
        available: Available<'a, T>,
        entry: impl Fn(&'a T::Row) -> (K, V),
    ) -> Self {
        let gap = match &available {
            Available::Complete(_) => Gap::Absent,
            Available::Truncated { .. } => Gap::Unreached,
            Available::Unavailable { reason } => Gap::Unavailable(reason),
            Available::NotRecorded { mode } => Gap::NotRecorded(*mode),
        };
        Self {
            rows: available.rows().iter().map(entry).collect(),
            gap,
        }
    }

    fn get(&self, key: &K) -> Result<V, Gap<'a>> {
        self.rows.get(key).copied().ok_or(self.gap)
    }
}

pub(super) struct Inputs<'a> {
    divisions: Keyed<'a, u64, &'a DivisionRow>,
    aliases: Keyed<'a, u32, u32>,
}

impl<'a> Inputs<'a> {
    pub(super) fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        Ok(Self {
            divisions: Keyed::read(rows::<tables::Divisions>(frame)?, |row| (row.id, row)),
            aliases: Keyed::read(rows::<tables::CountryLinks>(frame)?, |row| {
                (row.id, row.canonical_slot)
            }),
        })
    }

    fn exile_country(row: &LandCombatDivisionRow) -> Option<u32> {
        let exile = row.exile.as_ref()?;
        (exile.origin_type == EXILE_ORIGIN).then_some(if exile.country >= 1 {
            exile.country as u32
        } else {
            exile.country_fallback as u32
        })
    }

    fn owner(&self, division: u64) -> Result<u32, Refused<'a>> {
        let unit = self
            .divisions
            .get(&division)
            .map_err(|gap| Refused::Division { division, gap })?;
        Ok(if (unit.expeditionary_owner as i32) >= 1 {
            unit.expeditionary_owner
        } else {
            unit.roster_owner
        })
    }

    pub(super) fn entry_owner(&self, row: &LandCombatDivisionRow) -> Result<u32, Refused<'a>> {
        match Self::exile_country(row) {
            Some(country) => Ok(country),
            None => self.owner(row.division),
        }
    }

    fn equal(&self, first: u32, second: u32) -> Result<bool, Refused<'a>> {
        if first == second {
            Ok(true)
        } else if first == 0 || second == 0 {
            Ok(false)
        } else {
            let slot = |country| {
                self.aliases
                    .get(&country)
                    .map_err(|gap| Refused::CountryLink { country, gap })
            };
            Ok(slot(first)? == slot(second)?)
        }
    }

    fn contains(&self, countries: &[u32], country: u32) -> Result<bool, Refused<'a>> {
        any(countries, |candidate| self.equal(*candidate, country))
    }

    fn against_occupier(
        &self,
        row: &LandCombatDivisionRow,
        detail: &LandCombatDetailsRow,
        bonuses: &[i64; 2],
    ) -> Result<bool, Refused<'a>> {
        let (Some(_), Some(exile)) = (Self::exile_country(row), &row.exile) else {
            return Ok(false);
        };
        let side = usize::from(row.side);
        let opponents = &detail.sides[1 - side].committed;
        if opponents.is_empty() || bonuses[side] <= 0 {
            return Ok(false);
        }
        any(opponents, |opponent| {
            self.contains(&exile.occupiers, self.owner(*opponent)?)
        })
    }

    pub(super) fn occupiers(
        &self,
        row: &LandCombatDivisionRow,
        detail: &LandCombatDetailsRow,
        bonuses: &[i64; 2],
    ) -> Result<usize, Refused<'a>> {
        let (Some(_), Some(exile)) = (Self::exile_country(row), &row.exile) else {
            return Ok(0);
        };
        let side = usize::from(row.side);
        if bonuses[side] <= 0 {
            return Ok(0);
        }
        let mut count = 0;
        for opponent in &detail.sides[1 - side].committed {
            count += usize::from(self.contains(&exile.occupiers, self.owner(*opponent)?)?);
        }
        Ok(count)
    }

    fn on_core(&self, row: &LandCombatDivisionRow) -> Result<bool, Refused<'a>> {
        let (Some(country), Some(exile)) = (Self::exile_country(row), &row.exile) else {
            return Ok(false);
        };
        self.contains(&exile.location_cores, country)
    }

    pub(super) fn frames(
        &self,
        row: &LandCombatDivisionRow,
        detail: &LandCombatDetailsRow,
        bonuses: &[i64; 2],
    ) -> [Result<bool, Refused<'a>>; 2] {
        [
            self.against_occupier(row, detail, bonuses),
            self.on_core(row),
        ]
    }
}

#[cfg(test)]
mod tests;
