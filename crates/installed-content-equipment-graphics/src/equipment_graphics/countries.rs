
use std::{collections::BTreeMap, io};

use replay_data::{rows::TableSchema, rows::tables};
use replay_frame::{frame::DecodedSource, frame::TableView};

fn rows<T: TableSchema>(frame: &dyn DecodedSource) -> io::Result<(&[T::Row], bool)> {
    fn typed<T: TableSchema>(
        rows: &replay_data::rows::TableRows,
        hour: replay_data::Hour,
    ) -> io::Result<&[T::Row]> {
        rows.rows::<T>().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("hour {hour} {:?} contains another table's rows", T::ID),
            )
        })
    }
    let typed = |rows| typed::<T>(rows, frame.hour());
    match frame.view(T::ID.index()) {
        TableView::Complete(rows) => Ok((typed(rows)?, true)),
        TableView::Truncated { rows, .. } => Ok((typed(rows)?, false)),
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => Ok((&[], false)),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct ModelCountry {
    pub tag: String,
    pub cosmetic: String,
    pub graphical_culture: Option<String>,
    pub original_tag: Option<String>,
    pub original_graphical_culture: Option<String>,
    pub ruling_ideology: Option<Option<String>>,
}

pub fn read(frame: &dyn DecodedSource) -> io::Result<BTreeMap<u32, ModelCountry>> {
    let links: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let countries = frame.complete::<tables::Countries>()?;
    let by_tag: BTreeMap<&[u8], u32> = countries
        .iter()
        .map(|row| (row.tag.as_slice(), row.canonical_slot))
        .collect();
    let appearance: BTreeMap<u32, _> = rows::<tables::CountryAppearance>(frame)?
        .0
        .iter()
        .filter_map(|row| Some((*links.get(&row.country)?, row)))
        .collect();
    let (parties, parties_known) = rows::<tables::PoliticalParties>(frame)?;
    let mut ideology = BTreeMap::new();
    for party in parties {
        if party.is_ruling != 0
            && let Some(slot) = links.get(&party.country)
        {
            ideology.insert(*slot, std::str::from_utf8(&party.token).ok());
        }
    }
    let culture = |slot: &u32| {
        appearance
            .get(slot)
            .and_then(|row| std::str::from_utf8(&row.graphical_culture).ok())
            .map(str::to_owned)
    };
    Ok(countries
        .iter()
        .filter_map(|country| {
            let slot = country.canonical_slot;
            let row = appearance.get(&slot)?;
            let tag = std::str::from_utf8(&country.tag).ok()?;
            if tag.is_empty() {
                return None;
            }
            let (original_tag, original_graphical_culture) = if row.name_tag.is_empty() {
                (Some(tag.to_owned()), culture(&slot))
            } else {
                match by_tag.get(row.name_tag.as_slice()) {
                    Some(original) => (
                        std::str::from_utf8(&row.name_tag).ok().map(str::to_owned),
                        culture(original),
                    ),
                    None => (None, None),
                }
            };
            Some((
                slot,
                ModelCountry {
                    tag: tag.to_owned(),
                    cosmetic: std::str::from_utf8(&row.cosmetic_tag).ok()?.to_owned(),
                    graphical_culture: culture(&slot),
                    original_tag,
                    original_graphical_culture,
                    ruling_ideology: match ideology.get(&slot) {
                        _ if !parties_known => None,
                        None => Some(None),
                        Some(&token) => token.map(|token| Some(token.to_owned())),
                    },
                },
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow, TableId, TableRows,
    };
    use replay_frame::frame::{DecodedFrame, Metrics};

    fn frame(parties: Vec<PoliticalPartyRow>) -> DecodedFrame {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "fixture".into(),
        });
        let countries = [(1, b"AAA"), (2, b"BBB"), (3, b"CCC")];
        tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            countries
                .iter()
                .map(|(id, tag)| CountryRow {
                    id: *id,
                    canonical_slot: *id,
                    tag: tag.to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
        tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
            countries
                .iter()
                .map(|(id, _)| CountryLinkRow {
                    id: *id,
                    canonical_slot: *id,
                })
                .collect(),
        ));
        tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(
                countries
                    .iter()
                    .map(|(id, _)| CountryAppearanceRow {
                        country: *id,
                        graphical_culture: b"synthetic_gfx".to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ));
        tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(parties));
        DecodedFrame {
            branch: 1,
            hour: 0,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }
    }

    #[test]
    fn a_ruling_token_that_is_not_utf8_is_unknown_not_a_repaired_key() {
        let party = |country, token: &[u8]| PoliticalPartyRow {
            country,
            id: 1,
            is_ruling: 1,
            token: token.to_vec(),
            ..Default::default()
        };
        let countries = read(&frame(vec![
            party(1, b"synthetic_group"),
            party(2, &[b'x', 0xff]),
        ]))
        .unwrap();
        assert_eq!(
            countries[&1].ruling_ideology,
            Some(Some("synthetic_group".into()))
        );
        assert_eq!(countries[&2].ruling_ideology, None);
        assert_eq!(countries[&3].ruling_ideology, Some(None));
        assert_eq!(
            countries[&2].graphical_culture.as_deref(),
            Some("synthetic_gfx")
        );
    }
}
