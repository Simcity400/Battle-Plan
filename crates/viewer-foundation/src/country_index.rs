
use std::{collections::BTreeMap, io, sync::Arc};

use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;

use viewer_runtime::memo::Settled;
use super::tables::{core, rows};

pub const TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::Countries,
    TableId::CountryAppearance,
];

pub type FlagTags = (Result<String, String>, Result<String, String>);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CountryIndex {
    links: BTreeMap<u32, u32>,
    tags: BTreeMap<u32, String>,
    ids: BTreeMap<u32, u32>,
    flag_tags: BTreeMap<u32, FlagTags>,
}

impl CountryIndex {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let countries = core::<tables::Countries>(frame)?;
        let by_slot: BTreeMap<_, _> = countries
            .iter()
            .map(|country| (country.canonical_slot, country))
            .collect();
        let appearances: BTreeMap<_, _> = rows::<tables::CountryAppearance>(frame)?
            .rows()
            .iter()
            .map(|row| (row.country, row))
            .collect();
        let flag_tags = by_slot
            .iter()
            .filter_map(|(&slot, country)| {
                let row = appearances.get(&country.id)?;
                Some((
                    slot,
                    (
                        std::str::from_utf8(&row.cosmetic_tag)
                            .map(str::to_owned)
                            .map_err(|_| "country cosmetic tag is not UTF-8".into()),
                        content_reader_country_names::country_names::original_tag(country, row)
                            .map(str::to_owned),
                    ),
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

    pub fn settled(memo: &Settled<Self>, frame: &DecodedFrame) -> io::Result<Arc<Self>> {
        for table in TABLES {
            let _ = &*frame.tables[table.index()];
        }
        memo.get_or_build(frame, Self::read)
    }

    pub fn memo() -> Settled<Self> {
        Settled::new(TABLES)
    }

    pub fn links(&self) -> &BTreeMap<u32, u32> {
        &self.links
    }

    pub fn slot(&self, link: u32) -> Option<u32> {
        self.links.get(&link).copied()
    }

    pub fn tags(&self) -> &BTreeMap<u32, String> {
        &self.tags
    }

    pub fn tag(&self, slot: u32) -> Option<&str> {
        self.tags.get(&slot).map(String::as_str)
    }

    pub fn ids(&self) -> &BTreeMap<u32, u32> {
        &self.ids
    }

    pub fn flag_tags(&self) -> &BTreeMap<u32, FlagTags> {
        &self.flag_tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TABLE_COUNT;
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::SharedView, frame::TableView};

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
                        original_tag: replay_data::rows::Recorded::Value(Some(Vec::new())),
                        original_tag_reference: replay_data::rows::Recorded::Value(Some(0)),
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
        assert_eq!(
            index.flag_tags()[&1],
            (Ok("COS".to_owned()), Ok("C001".to_owned()))
        );
        assert!(!index.flag_tags().contains_key(&2));
    }

    #[test]
    fn the_flag_index_tracks_exact_original_tag_and_each_refusal() {
        use replay_data::rows::Recorded;
        let hour = |number, naming: &[u8], original| {
            let [links, countries, appearance] = tables(3, 0);
            let TableView::Complete(TableRows::CountryAppearance(rows)) = &*appearance else {
                unreachable!();
            };
            let mut rows = rows.clone();
            rows[1].name_tag = naming.to_vec();
            rows[1].original_tag = original;
            rows[1].original_tag_reference = Recorded::Value(Some(1));
            frame(
                number,
                &[
                    links,
                    countries,
                    SharedView::from(TableView::Complete(TableRows::CountryAppearance(rows))),
                ],
            )
        };
        let memo = CountryIndex::memo();
        let known = CountryIndex::settled(
            &memo,
            &hour(1, b"GEN", Recorded::Value(Some(b"ORI".to_vec()))),
        )
        .unwrap();
        assert_eq!(known.flag_tags()[&1].1.as_deref(), Ok("ORI"));
        let renamed = CountryIndex::settled(
            &memo,
            &hour(2, b"OTHER", Recorded::Value(Some(b"ORI".to_vec()))),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&known, &renamed));
        for (number, captured, reason) in [
            (3, Recorded::Value(None), "original tag of country appearance capture refused"),
            (4, Recorded::NotRecorded, "original tag of country appearance not recorded"),
            (5, Recorded::Value(Some(vec![0xff])), "not UTF-8"),
        ] {
            let refused = CountryIndex::settled(&memo, &hour(number, b"GEN", captured)).unwrap();
            assert!(!Arc::ptr_eq(&known, &refused));
            assert!(
                refused.flag_tags()[&1]
                    .1
                    .as_ref()
                    .unwrap_err()
                    .contains(reason),
                "{:?}",
                refused.flag_tags()[&1].1
            );
            assert_eq!(refused.tags(), known.tags());
            assert_eq!(refused.links(), known.links());
        }
        let empty =
            CountryIndex::read(&hour(6, b"GEN", Recorded::Value(Some(Vec::new())))).unwrap();
        assert_eq!(empty.flag_tags()[&1].1.as_deref(), Ok(""));
        for (captured, reason) in [
            (
                Recorded::Value(None),
                "original tag reference of country appearance capture refused",
            ),
            (Recorded::NotRecorded, "original tag reference of country appearance not recorded"),
        ] {
            let mut frame = hour(7, b"GEN", Recorded::Value(Some(b"ORI".to_vec())));
            let TableView::Complete(TableRows::CountryAppearance(rows)) =
                &mut *frame.tables[TableId::CountryAppearance.index()]
            else {
                unreachable!()
            };
            rows[1].original_tag_reference = captured;
            let refused = CountryIndex::settled(&memo, &frame).unwrap();
            assert!(!Arc::ptr_eq(&known, &refused));
            assert!(
                refused.flag_tags()[&1]
                    .1
                    .as_ref()
                    .unwrap_err()
                    .contains(reason)
            );
        }
    }

    #[test]
    fn alias_appearance_does_not_replace_the_resolved_country_objects_flag_inputs() {
        let [links, countries, appearance] = tables(3, 0);
        let TableView::Complete(TableRows::CountryLinks(link_rows)) = &*links else {
            unreachable!()
        };
        let mut link_rows = link_rows.clone();
        link_rows.push(CountryLinkRow {
            id: 9000,
            canonical_slot: 1,
        });
        let TableView::Complete(TableRows::CountryAppearance(appearance_rows)) = &*appearance
        else {
            unreachable!()
        };
        let mut appearance_rows = appearance_rows.clone();
        let mut alias = appearance_rows[1].clone();
        alias.country = 9000;
        alias.cosmetic_tag = vec![0xff];
        alias.original_tag_reference = replay_data::rows::Recorded::NotRecorded;
        appearance_rows.push(alias);
        let index = CountryIndex::read(&frame(
            1,
            &[
                SharedView::from(TableView::Complete(TableRows::CountryLinks(link_rows))),
                countries,
                SharedView::from(TableView::Complete(TableRows::CountryAppearance(
                    appearance_rows,
                ))),
            ],
        ))
        .unwrap();
        assert_eq!(index.slot(9000), Some(1));
        assert_eq!(index.flag_tags()[&1], (Ok("COS".into()), Ok("C001".into())));
    }

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

    #[test]
    fn hourly_values_do_not_change_the_settled_index() {
        let memo = CountryIndex::memo();
        let first = CountryIndex::settled(&memo, &frame(1, &tables(40, 1))).unwrap();
        assert_eq!(first.tags().len(), 40);
        let next = CountryIndex::settled(&memo, &frame(2, &tables(40, 2))).unwrap();
        assert!(Arc::ptr_eq(&first, &next));
        let grown = CountryIndex::settled(&memo, &frame(3, &tables(41, 2))).unwrap();
        assert!(!Arc::ptr_eq(&first, &grown));
        assert_eq!(grown.tag(40), Some("C040"));
    }
}
