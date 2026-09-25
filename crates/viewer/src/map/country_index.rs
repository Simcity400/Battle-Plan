//! One hour's countries, read once and shared by every layer that names one.
//!
//! The overlay, the labels, the movers and the name pass each resolve raw
//! country link ids to canonical slots and slots to tags. This index holds
//! those maps once per hour. `Countries` carries hourly power values, so its
//! table is a new allocation every hour while the index built from it rarely
//! changes; [`CountryIndex::settled`] hands on an earlier hour's equal index
//! ([`Settled`]), so a layer keyed on the index's identity hits across hours.
//!
//! Every map is sized from the recorded rows; nothing assumes a country count
//! or a tag list, so a mod's dynamic and added countries index like vanilla's.
//!
//! The index refuses an hour whose `CountryLinks` or `Countries` is missing or
//! truncated, as the map layers do. A reader that tolerates a partial
//! `CountryLinks` through [`rows`] (the combat windows) keeps reading its own
//! rows rather than switching to the index.

use std::{collections::BTreeMap, io, sync::Arc};

use replay::{
    frame::DecodedFrame,
    rows::{TableId, tables},
};

use super::{
    memo::Settled,
    tables::{core, rows},
};

/// The tables the index reads, and all it reads.
pub const TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::Countries,
    TableId::CountryAppearance,
];

/// One hour's country identities, by raw link id and by canonical slot.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CountryIndex {
    /// Raw country link id to canonical slot, from `CountryLinks`.
    links: BTreeMap<u32, u32>,
    /// Canonical slot to tag, from `Countries`.
    tags: BTreeMap<u32, String>,
    /// Canonical slot to the country's own raw link id, from `Countries`.
    ids: BTreeMap<u32, u32>,
    /// Canonical slot to the cosmetic tag and the name tag its flag is chosen
    /// by, from `CountryAppearance`: whatever rows the hour holds, so a
    /// country the table lacks has none.
    flag_tags: BTreeMap<u32, (String, String)>,
}

impl CountryIndex {
    /// Reads the index from the hour's tables; a missing or truncated
    /// `CountryLinks` or `Countries` refuses the hour, as every map layer does.
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let countries = core::<tables::Countries>(frame)?;
        let flag_tags = rows::<tables::CountryAppearance>(frame)?
            .rows()
            .iter()
            .filter_map(|row| {
                Some((
                    *links.get(&row.country)?,
                    (text(&row.cosmetic_tag), text(&row.name_tag)),
                ))
            })
            .collect();
        Ok(Self {
            tags: countries
                .iter()
                .map(|country| (country.canonical_slot, text(&country.tag)))
                .collect(),
            ids: countries
                .iter()
                .map(|country| (country.canonical_slot, country.id))
                .collect(),
            links,
            flag_tags,
        })
    }

    /// The hour's index, as the same allocation as an earlier hour's when the
    /// two are equal.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn settled(memo: &Settled<Self>, frame: &DecodedFrame) -> io::Result<Arc<Self>> {
        memo.get_or_build(frame, Self::read)
    }

    /// A memo for [`CountryIndex::settled`] holding `capacity` recent hours.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn memo(capacity: usize) -> Settled<Self> {
        Settled::new(TABLES, capacity)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn links(&self) -> &BTreeMap<u32, u32> {
        &self.links
    }

    /// The canonical slot a raw country link id refers to.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn slot(&self, link: u32) -> Option<u32> {
        self.links.get(&link).copied()
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn tags(&self) -> &BTreeMap<u32, String> {
        &self.tags
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn tag(&self, slot: u32) -> Option<&str> {
        self.tags.get(&slot).map(String::as_str)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn ids(&self) -> &BTreeMap<u32, u32> {
        &self.ids
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the per-hour layers switch over")
    )]
    pub fn flag_tags(&self) -> &BTreeMap<u32, (String, String)> {
        &self.flag_tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, SharedView, TABLE_COUNT, TableView},
        rows::{CountryAppearanceRow, CountryLinkRow, CountryRow, TableRows},
    };

    /// `count` countries whose raw link ids differ from their slots, the last
    /// one without an appearance row, with `power` as every hourly value.
    fn tables(count: u32, power: i64) -> [SharedView; 3] {
        let tag = |slot: u32| format!("C{slot:03}").into_bytes();
        let complete = |rows| SharedView::from(TableView::Complete(rows));
        [
            complete(TableRows::CountryLinks(
                (0..count)
                    .map(|slot| CountryLinkRow {
                        id: 1000 + slot,
                        canonical_slot: slot,
                    })
                    .collect(),
            )),
            complete(TableRows::Countries(
                (0..count)
                    .map(|slot| CountryRow {
                        id: 1000 + slot,
                        canonical_slot: slot,
                        tag: tag(slot),
                        command_power_raw: power,
                        ..Default::default()
                    })
                    .collect(),
            )),
            complete(TableRows::CountryAppearance(
                (0..count.saturating_sub(1))
                    .map(|slot| CountryAppearanceRow {
                        country: 1000 + slot,
                        cosmetic_tag: if slot == 1 {
                            b"COS".to_vec()
                        } else {
                            Vec::new()
                        },
                        name_tag: tag(slot),
                        ..Default::default()
                    })
                    .collect(),
            )),
        ]
    }

    fn frame(hour: i32, [links, countries, appearance]: &[SharedView; 3]) -> DecodedFrame {
        let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| {
            SharedView::from(TableView::Unavailable {
                reason: "not recorded".into(),
            })
        });
        tables[TableId::CountryLinks.index()] = links.clone();
        tables[TableId::Countries.index()] = countries.clone();
        tables[TableId::CountryAppearance.index()] = appearance.clone();
        DecodedFrame {
            branch: 1,
            hour,
            tables,
            metrics: Metrics::default(),
        }
    }

    #[test]
    fn the_index_resolves_links_tags_ids_and_flag_tags() {
        let index = CountryIndex::read(&frame(1, &tables(3, 0))).unwrap();
        assert_eq!(index.slot(1001), Some(1));
        assert_eq!(index.slot(1), None);
        assert_eq!(index.links().len(), 3);
        assert_eq!(index.tag(2), Some("C002"));
        assert_eq!(index.ids()[&0], 1000);
        assert_eq!(index.flag_tags()[&1], ("COS".to_owned(), "C001".to_owned()));
        // The last country has no appearance row, so no flag tags.
        assert!(!index.flag_tags().contains_key(&2));
    }

    /// `view`'s rows as a partial walk that missed one more.
    fn truncated(view: &SharedView) -> SharedView {
        match &**view {
            TableView::Complete(rows) => SharedView::from(TableView::Truncated {
                rows: rows.clone(),
                missing: 1,
            }),
            _ => unreachable!("the fixture's tables are complete"),
        }
    }

    fn unavailable() -> SharedView {
        SharedView::from(TableView::Unavailable {
            reason: "not recorded".into(),
        })
    }

    #[test]
    fn the_index_refuses_an_hour_with_a_missing_or_truncated_core_table() {
        let full = tables(3, 0);
        for core in [0, 1] {
            for gap in [unavailable(), truncated(&full[core])] {
                let mut hour = full.clone();
                hour[core] = gap;
                assert!(
                    CountryIndex::read(&frame(1, &hour)).is_err(),
                    "table {core}"
                );
            }
        }
    }

    #[test]
    fn a_missing_or_partial_appearance_table_leaves_only_flag_tags_short() {
        let full = tables(3, 0);
        let complete = CountryIndex::read(&frame(1, &full)).unwrap();
        let [links, countries, appearance] = full;
        let without = CountryIndex::read(&frame(
            1,
            &[links.clone(), countries.clone(), unavailable()],
        ))
        .unwrap();
        assert!(without.flag_tags().is_empty());
        assert_eq!(
            (without.links(), without.tags(), without.ids()),
            (complete.links(), complete.tags(), complete.ids())
        );
        // A partial walk's rows are real rows: they are indexed.
        let partial =
            CountryIndex::read(&frame(1, &[links, countries, truncated(&appearance)])).unwrap();
        assert_eq!(partial, complete);
    }

    #[test]
    fn an_appearance_row_of_an_unlinked_country_is_left_out() {
        let [links, countries, appearance] = tables(3, 0);
        let TableView::Complete(TableRows::CountryAppearance(rows)) = &*appearance else {
            unreachable!("the fixture's appearance table is complete");
        };
        let mut rows = rows.clone();
        rows.push(CountryAppearanceRow {
            country: 9999,
            name_tag: b"ORP".to_vec(),
            ..Default::default()
        });
        let orphaned = SharedView::from(TableView::Complete(TableRows::CountryAppearance(rows)));
        assert_eq!(
            CountryIndex::read(&frame(1, &[links.clone(), countries.clone(), orphaned])).unwrap(),
            CountryIndex::read(&frame(1, &[links, countries, appearance])).unwrap()
        );
    }

    /// The link, tag, id and flag-tag maps exactly as `counters::hour_tables`
    /// builds them (`map/counters.rs`, the lines before its `sides` entry).
    /// A stand-in until the overlay reads the index itself: the first PR that
    /// switches it over replaces this copy with a test on `hour_tables`.
    #[expect(
        clippy::type_complexity,
        reason = "the four maps, as hour_tables holds them"
    )]
    fn hour_tables_maps(
        frame: &DecodedFrame,
    ) -> io::Result<(
        BTreeMap<u32, u32>,
        BTreeMap<u32, String>,
        BTreeMap<u32, u32>,
        BTreeMap<u32, (String, String)>,
    )> {
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let rows = core::<tables::Countries>(frame)?;
        let tags: BTreeMap<u32, String> = rows
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    String::from_utf8_lossy(&country.tag).into_owned(),
                )
            })
            .collect();
        let country_ids = rows
            .iter()
            .map(|country| (country.canonical_slot, country.id))
            .collect();
        let flag_tags = super::super::tables::rows::<tables::CountryAppearance>(frame)?
            .rows()
            .iter()
            .filter_map(|row| {
                let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
                Some((
                    *links.get(&row.country)?,
                    (text(&row.cosmetic_tag), text(&row.name_tag)),
                ))
            })
            .collect();
        Ok((links, tags, country_ids, flag_tags))
    }

    #[test]
    fn the_index_matches_the_overlay_hour_tables_maps() {
        let [links, countries, appearance] = tables(40, 0);
        for hour in [
            [links.clone(), countries.clone(), appearance.clone()],
            [links.clone(), countries.clone(), truncated(&appearance)],
            [links, countries, unavailable()],
        ] {
            let frame = frame(1, &hour);
            let index = CountryIndex::read(&frame).unwrap();
            let (links, tags, ids, flag_tags) = hour_tables_maps(&frame).unwrap();
            assert_eq!(index.links(), &links);
            assert_eq!(index.tags(), &tags);
            assert_eq!(index.ids(), &ids);
            assert_eq!(index.flag_tags(), &flag_tags);
        }
    }

    #[test]
    fn hourly_values_do_not_change_the_settled_index_at_five_thousand_countries() {
        let memo = CountryIndex::memo(4);
        let first = CountryIndex::settled(&memo, &frame(1, &tables(5000, 1))).unwrap();
        assert_eq!(first.tags().len(), 5000);
        // Every table is a new allocation and the power values moved, but no
        // identity did: the same index.
        let next = CountryIndex::settled(&memo, &frame(2, &tables(5000, 2))).unwrap();
        assert!(Arc::ptr_eq(&first, &next));
        // A new country is a new index.
        let grown = CountryIndex::settled(&memo, &frame(3, &tables(5001, 2))).unwrap();
        assert!(!Arc::ptr_eq(&first, &grown));
        assert_eq!(grown.tag(5000), Some("C5000"));
    }
}
