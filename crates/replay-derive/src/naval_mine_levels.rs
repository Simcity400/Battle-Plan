use replay_data::{rows::NavalRegionalMineRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;

struct Navy {
    count: Option<u32>,
    start: usize,
    end: usize,
}

pub struct Levels<'a> {
    links: &'a BTreeMap<u32, u32>,
    rows: &'a [NavalRegionalMineRow],
    navies: BTreeMap<u32, Navy>,
}
impl<'a> Levels<'a> {
    pub fn read(frame: &'a DecodedFrame, links: &'a BTreeMap<u32, u32>) -> Result<Self, String> {
        Self::from_rows(
            frame
                .complete::<tables::NavalRegionalMines>()
                .map_err(|e| e.to_string())?,
            links,
        )
    }
    pub fn from_rows(
        rows: &'a [NavalRegionalMineRow],
        links: &'a BTreeMap<u32, u32>,
    ) -> Result<Self, String> {
        let mut navies = BTreeMap::new();
        let mut first = 0;
        while first < rows.len() {
            let raw = rows[first].country;
            let end = first + rows[first..].partition_point(|row| row.country == raw);
            let slot = *links
                .get(&raw)
                .ok_or("regional mine country link is unavailable")?;
            let group = &rows[first..end];
            let count = if group[0].kind == 0 {
                if group[0].amount != 0 || group.get(1).is_some_and(|row| row.kind == 0) {
                    return Err("regional mine vector has malformed or duplicate witnesses".into());
                }
                Some(group[0].region)
            } else {
                None
            };
            if let Some(count) = count
                && group
                    .last()
                    .is_some_and(|row| row.kind == 1 && row.region >= count)
            {
                return Err("regional mine value lies beyond its witnessed vector".into());
            }
            if navies
                .insert(
                    slot,
                    Navy {
                        count,
                        start: first,
                        end,
                    },
                )
                .is_some()
            {
                return Err("regional mine vector duplicates a canonical country".into());
            }
            first = end;
        }
        Ok(Self {
            links,
            rows,
            navies,
        })
    }
    pub fn amount(&self, country: u32, region: u32) -> Result<i64, String> {
        let slot = self
            .links
            .get(&country)
            .ok_or("regional mine country link is unavailable")?;
        let navy = self
            .navies
            .get(slot)
            .ok_or("regional mine country completeness witness is unavailable")?;
        let count = navy
            .count
            .ok_or("regional mine country completeness witness is unavailable")?;
        if region >= count {
            return Err("regional mine region is outside its witnessed vector".into());
        }
        let values = &self.rows[navy.start..navy.end];
        Ok(values
            .binary_search_by_key(&(1, region), |row| (row.kind, row.region))
            .ok()
            .map_or(0, |index| values[index].amount))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_requires_an_owner_witness_and_a_valid_index_while_signed_amounts_survive() {
        let links = [(11, 1), (111, 1), (22, 2)].into_iter().collect();
        let levels = Levels::from_rows(
            &[
                NavalRegionalMineRow {
                    country: 11,
                    kind: 0,
                    region: 5,
                    amount: 0,
                },
                NavalRegionalMineRow {
                    country: 11,
                    kind: 1,
                    region: 3,
                    amount: -17,
                },
            ],
            &links,
        )
        .unwrap();
        assert_eq!(levels.amount(111, 3).unwrap(), -17);
        assert_eq!(levels.amount(11, 4).unwrap(), 0);
        assert!(levels.amount(11, 5).is_err());
        assert!(levels.amount(22, 2).is_err());
    }
    #[test]
    fn duplicates_and_amounts_beyond_the_vector_are_refused() {
        let links = [(11, 1)].into_iter().collect();
        let witness = NavalRegionalMineRow {
            country: 11,
            kind: 0,
            region: 2,
            amount: 0,
        };
        assert!(Levels::from_rows(&[witness, witness], &links).is_err());
        assert!(
            Levels::from_rows(
                &[
                    witness,
                    NavalRegionalMineRow {
                        country: 11,
                        kind: 1,
                        region: 2,
                        amount: 1
                    }
                ],
                &links
            )
            .is_err()
        );
        let missing = Levels::from_rows(
            &[NavalRegionalMineRow {
                country: 11,
                kind: 1,
                region: 1,
                amount: 1,
            }],
            &links,
        )
        .unwrap();
        assert!(missing.amount(11, 1).is_err());
    }
}
