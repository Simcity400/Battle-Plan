
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CountryRelations {
    pub reciprocal_donors: Vec<u32>,
    pub wartime_allies: Vec<u32>,
    pub enemies: Vec<u32>,
}

pub trait CountryVectors {
    fn vectors(&self, slot: u32) -> io::Result<CountryRelations>;
    fn donors(&self, slot: u32, donors: &mut Vec<u32>) -> io::Result<()>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AirParticipants {
    pub a: Vec<u32>,
    pub b: Vec<u32>,
}

fn canonical(raw: u32, links: &BTreeMap<u32, u32>) -> io::Result<u32> {
    if raw == 0 {
        return Ok(0);
    }
    links.get(&raw).copied().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("air participant country reference {raw} has no CountryLinks row"),
        )
    })
}

#[derive(Default)]
struct Roster {
    list: Vec<u32>,
    raws: BTreeSet<u32>,
    slots: BTreeSet<u32>,
}

fn append_unique(roster: &mut Roster, raw: u32, links: &BTreeMap<u32, u32>) -> io::Result<()> {
    let slot = canonical(raw, links)?;
    if roster.raws.contains(&raw) || (raw != 0 && roster.slots.contains(&slot)) {
        return Ok(());
    }
    roster.list.push(raw);
    roster.raws.insert(raw);
    if raw != 0 {
        roster.slots.insert(slot);
    }
    Ok(())
}

fn append_country_and_donors(
    list: &mut Roster,
    country: u32,
    relations: &impl CountryVectors,
    donors: &mut Vec<u32>,
    links: &BTreeMap<u32, u32>,
) -> io::Result<()> {
    append_unique(list, country, links)?;
    relations.donors(canonical(country, links)?, donors)?;
    for &donor in donors.iter() {
        append_unique(list, donor, links)?;
    }
    Ok(())
}

pub fn ordered_participants(
    ours: u32,
    relations: &impl CountryVectors,
    outgoing_volunteer_hosts: &[u32],
    links: &BTreeMap<u32, u32>,
) -> io::Result<AirParticipants> {
    let (mut a, mut b) = (Roster::default(), Roster::default());
    let mut donors = Vec::new();

    let own = relations.vectors(ours)?;
    for &donor in &own.reciprocal_donors {
        append_unique(&mut a, donor, links)?;
    }
    for &ally in &own.wartime_allies {
        append_country_and_donors(&mut a, ally, relations, &mut donors, links)?;
    }
    for &enemy in &own.enemies {
        append_country_and_donors(&mut b, enemy, relations, &mut donors, links)?;
    }

    for &host in outgoing_volunteer_hosts {
        append_unique(&mut a, host, links)?;
        let host_relations = relations.vectors(canonical(host, links)?)?;
        for &ally in &host_relations.wartime_allies {
            append_country_and_donors(&mut a, ally, relations, &mut donors, links)?;
        }
        for &enemy in &host_relations.enemies {
            append_country_and_donors(&mut b, enemy, relations, &mut donors, links)?;
        }
    }

    Ok(AirParticipants {
        a: a.list,
        b: b.list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    impl CountryVectors for BTreeMap<u32, CountryRelations> {
        fn vectors(&self, slot: u32) -> io::Result<CountryRelations> {
            Ok(self.get(&slot).cloned().unwrap_or_default())
        }
        fn donors(&self, slot: u32, donors: &mut Vec<u32>) -> io::Result<()> {
            donors.clear();
            donors.extend(self.vectors(slot)?.reciprocal_donors);
            Ok(())
        }
    }

    fn links(ids: impl IntoIterator<Item = u32>) -> BTreeMap<u32, u32> {
        ids.into_iter().map(|id| (id, id)).collect()
    }

    #[test]
    fn preserves_native_vector_and_host_traversal_order() {
        let mut relations = BTreeMap::new();
        relations.insert(
            1,
            CountryRelations {
                reciprocal_donors: vec![2, 3],
                wartime_allies: vec![4, 5],
                enemies: vec![8],
            },
        );
        relations.insert(
            4,
            CountryRelations {
                reciprocal_donors: vec![6],
                ..Default::default()
            },
        );
        relations.insert(
            5,
            CountryRelations {
                reciprocal_donors: vec![7],
                ..Default::default()
            },
        );
        relations.insert(
            8,
            CountryRelations {
                reciprocal_donors: vec![9],
                ..Default::default()
            },
        );
        relations.insert(
            10,
            CountryRelations {
                reciprocal_donors: vec![16],
                wartime_allies: vec![11],
                enemies: vec![13],
            },
        );
        relations.insert(
            11,
            CountryRelations {
                reciprocal_donors: vec![12],
                wartime_allies: vec![17],
                ..Default::default()
            },
        );
        relations.insert(
            13,
            CountryRelations {
                reciprocal_donors: vec![14],
                ..Default::default()
            },
        );
        relations.insert(
            15,
            CountryRelations {
                wartime_allies: vec![18],
                enemies: vec![19],
                ..Default::default()
            },
        );

        let result = ordered_participants(1, &relations, &[10, 15], &links(1..=19)).unwrap();
        assert_eq!(result.a, vec![2, 3, 4, 6, 5, 7, 10, 11, 12, 15, 18]);
        assert_eq!(result.b, vec![8, 9, 13, 14, 19]);
    }

    #[test]
    fn aliases_deduplicate_within_each_array_but_not_across_arrays() {
        let mut relations = BTreeMap::new();
        relations.insert(
            1,
            CountryRelations {
                reciprocal_donors: vec![20, 21, 0, 0],
                wartime_allies: vec![30],
                enemies: vec![22, 31],
            },
        );
        relations.insert(
            3,
            CountryRelations {
                reciprocal_donors: vec![23],
                ..Default::default()
            },
        );
        relations.insert(
            4,
            CountryRelations {
                reciprocal_donors: vec![24],
                ..Default::default()
            },
        );
        let mut aliases = BTreeMap::from([
            (1, 1),
            (20, 2),
            (21, 2),
            (22, 2),
            (23, 5),
            (24, 5),
            (30, 3),
            (31, 4),
            (32, 3),
        ]);
        aliases.insert(40, 0);
        relations.get_mut(&1).unwrap().reciprocal_donors.push(40);

        let result = ordered_participants(1, &relations, &[32, 30], &aliases).unwrap();
        assert_eq!(result.a, vec![20, 0, 40, 30, 23]);
        assert_eq!(result.b, vec![22, 31, 24]);
    }

    #[test]
    fn repeated_hosts_expand_even_when_the_host_was_already_present() {
        let mut relations = BTreeMap::new();
        relations.insert(
            1,
            CountryRelations {
                reciprocal_donors: vec![2],
                ..Default::default()
            },
        );
        relations.insert(
            2,
            CountryRelations {
                wartime_allies: vec![3],
                enemies: vec![4],
                ..Default::default()
            },
        );
        let result = ordered_participants(1, &relations, &[2, 2], &links(1..=4)).unwrap();
        assert_eq!(result.a, vec![2, 3]);
        assert_eq!(result.b, vec![4]);
    }

    #[test]
    fn a_missing_nonzero_country_link_is_an_error() {
        let relations = BTreeMap::from([(
            1,
            CountryRelations {
                reciprocal_donors: vec![2],
                ..Default::default()
            },
        )]);
        let error = ordered_participants(1, &relations, &[], &links([1])).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn viewing_slot_need_not_be_a_raw_reference() {
        let relations = BTreeMap::from([(
            100,
            CountryRelations {
                reciprocal_donors: vec![2],
                ..Default::default()
            },
        )]);
        let aliases = BTreeMap::from([(1, 100), (2, 200)]);
        let result = ordered_participants(100, &relations, &[], &aliases).unwrap();
        assert_eq!(result.a, vec![2]);
    }
}
