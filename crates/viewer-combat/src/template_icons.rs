//! Exiled-division combat badges and entry flags from the exile operands the
//! combat row records for its committed divisions.
//!
//! Frame 14 shows when an opposing committed owner occupies the exile
//! country. Frame 15 shows when the division stands in a state the exile
//! country cores. These frames are derived, not recorded.
use replay::{
    frame::DecodedFrame,
    rows::{DivisionRow, LandCombatDetailsRow, LandCombatDivisionRow, TableSchema, tables},
};
use std::{collections::BTreeMap, fmt, io};
use viewer_foundation::table_access::{Available, rows};

/// The template origin indicating an exiled division.
const EXILE_ORIGIN: i32 = 2;

/// Why a table this hour holds no row for a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Gap<'a> {
    /// The table is complete and has no row for the key.
    Absent,
    /// The table is truncated and its walk did not reach the key.
    Unreached,
    /// The table failed this hour, with the collector's reason.
    Unavailable(&'a str),
}

/// The recorded row a native operand of an entry flag or exile badge comes
/// from, missing this hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Refused<'a> {
    /// A division's owner (its own, or an opposing committed unit's).
    Division { division: u64, gap: Gap<'a> },
    /// A country's canonical slot, which the game compares.
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
        }
    }
}

/// Whether any candidate passes `test`. A pass decides even when another
/// candidate is undecided; otherwise the first undecided candidate leaves the
/// answer undecided. Used by `Inputs::contains` and by opponent
/// loop, which shows frame 14 when any opponent qualifies.
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

/// A keyed table and what its absence of a key means.
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

    /// Use the exile country for an exiled template when positive. Exactly origin
    /// 2 is exiled; every other origin follows the ordinary-owner path.
    fn exile_country(row: &LandCombatDivisionRow) -> Option<u32> {
        let exile = row.exile.as_ref()?;
        (exile.origin_type == EXILE_ORIGIN).then_some(if exile.country >= 1 {
            exile.country as u32
        } else {
            exile.country_fallback as u32
        })
    }

    /// A unit's owner for these tests: the expeditionary owner when positive,
    /// else the roster owner.
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

    /// inline flag country for a committed entry: an exiled
    /// template's country, else the division's owner.
    pub(super) fn entry_owner(&self, row: &LandCombatDivisionRow) -> Result<u32, Refused<'a>> {
        match Self::exile_country(row) {
            Some(country) => Ok(country),
            None => self.owner(row.division),
        }
    }

    /// raw equality first; distinct nonzero references compare
    /// canonical slots. Zero never goes through the alias lookup.
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

    /// Whether any of `countries` equals `country`. A match decides even when
    /// another candidate's link is missing; otherwise the first missing link
    /// leaves the answer undecided.
    fn contains(&self, countries: &[u32], country: u32) -> Result<bool, Refused<'a>> {
        any(countries, |candidate| self.equal(*candidate, country))
    }

    /// Frame 14: an opposing committed unit whose owner occupies the exile
    /// country, gated on this side's positive occupier bonus (attack, defence).
    /// the game counts every qualifying opponent and
    /// shows the frame when the count is positive, so one qualifying opponent
    /// decides even when another's owner or link is unknown.
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

    /// The opposing-unit count behind frame 14, which the badge's hover
    /// shows: opposing committed units only, zero without the bonus. Unlike
    /// the badge it needs every opponent's owner.
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

    /// Frame 15: the division's location state is a core of the exile country.
    fn on_core(&self, row: &LandCombatDivisionRow) -> Result<bool, Refused<'a>> {
        let (Some(country), Some(exile)) = (Self::exile_country(row), &row.exile) else {
            return Ok(false);
        };
        self.contains(&exile.location_cores, country)
    }

    /// Both badges of a committed division, in frame order.
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
