use super::*;
use content_reader_save::{ActualCountry, Save, intel::sources as saved};
use replay_data::rows::{IntelContributionRow as Contribution, IntelSourceRow as Source};

type ContributionKey = (u8, u32, u32);
type SourceKey = (u32, u32, u32);

fn contribution_key(row: &Contribution) -> ContributionKey {
    (row.kind, row.pool, row.ordinal)
}
fn source_key(row: &Source) -> SourceKey {
    (row.pool, row.source, row.ordinal)
}

fn same_country(left: u32, right: u32, countries: &BTreeMap<u32, &CountryRow>) -> bool {
    let slot = |raw| {
        if raw == 0 {
            Some(0)
        } else {
            countries.get(&raw).map(|row| row.canonical_slot)
        }
    };
    slot(left)
        .zip(slot(right))
        .is_some_and(|(left, right)| left == right)
}

fn expected(owner: u32, state: &saved::Sources) -> io::Result<(Vec<Contribution>, Vec<Source>)> {
    let mut contributions = Vec::new();
    let mut push = |kind, pool, ordinal, target, values: [i64; 4]| {
        contributions.push(Contribution {
            owner,
            kind,
            pool,
            ordinal,
            target,
            civilian: values[0],
            army: values[1],
            navy: values[2],
            air: values[3],
        });
    };
    for (kind, dense) in [(0, &state.allied), (1, &state.cryptology)] {
        push(kind, 0, u32::MAX, dense.len() as i32, [0; 4]);
        for (ordinal, &values) in dense.iter().enumerate() {
            if values != [0; 4] {
                push(kind, 0, ordinal as u32, ordinal as i32, values);
            }
        }
    }
    for (index, pools) in state.contributions.iter().enumerate() {
        let kind = index as u8 + 2;
        push(kind, u32::MAX, u32::MAX, pools.len() as i32, [0; 4]);
        for (pool, entries) in pools.iter().enumerate() {
            push(kind, pool as u32, u32::MAX, entries.len() as i32, [0; 4]);
            for (ordinal, entry) in entries.iter().enumerate() {
                push(
                    kind,
                    pool as u32,
                    ordinal as u32,
                    entry.target,
                    entry.values,
                );
            }
        }
    }
    for (pool, state) in state.dynamics.iter().enumerate() {
        for (kind, entries) in [(5, &state.pending), (6, &state.accumulated)] {
            push(kind, pool as u32, u32::MAX, entries.len() as i32, [0; 4]);
            for (ordinal, entry) in entries.iter().enumerate() {
                push(
                    kind,
                    pool as u32,
                    ordinal as u32,
                    entry.target,
                    entry.values,
                );
            }
        }
    }
    let mut sources = vec![Source {
        owner,
        pool: u32::MAX,
        source: u32::MAX,
        ordinal: u32::MAX,
        count: state.statics.len() as u32,
        other_count: state.dynamics.len() as u32,
        discriminant: state
            .discriminant
            .ok_or_else(|| invalid("saved intel source root lacks discriminant"))?,
        ..Default::default()
    }];
    for (pool, state) in state.statics.iter().enumerate() {
        sources.push(Source {
            owner,
            pool: pool as u32,
            source: u32::MAX,
            ordinal: u32::MAX,
            count: state.sources.len() as u32,
            next_id: state.next_id,
            ..Default::default()
        });
        for (source, state) in state.sources.iter().enumerate() {
            sources.push(Source {
                owner,
                pool: pool as u32,
                source: source as u32,
                ordinal: u32::MAX,
                count: state.values.len() as u32,
                id: state.id,
                removed: u8::from(state.removed),
                ..Default::default()
            });
            for (ordinal, entry) in state.values.iter().enumerate() {
                sources.push(Source {
                    owner,
                    pool: pool as u32,
                    source: source as u32,
                    ordinal: ordinal as u32,
                    target: entry.target,
                    civilian: entry.values[0],
                    army: entry.values[1],
                    navy: entry.values[2],
                    air: entry.values[3],
                    ..Default::default()
                });
            }
        }
    }
    Ok((contributions, sources))
}

fn compare_rows<K: Ord + std::fmt::Debug, T: PartialEq + std::fmt::Debug>(
    label: &str,
    actual: &BTreeMap<K, T>,
    expected: &BTreeMap<K, T>,
    report: &mut Report,
) {
    for (key, expected) in expected {
        match actual.get(key) {
            Some(actual) if actual == expected => (),
            Some(actual) => report.difference(
                format!("{label}.{key:?}"),
                format!("{actual:?}"),
                format!("{expected:?}"),
            ),
            None => report.difference(
                format!("{label}.{key:?}"),
                "absent",
                format!("{expected:?}"),
            ),
        }
    }
    for key in actual.keys().filter(|key| !expected.contains_key(*key)) {
        report.difference(format!("{label}.{key:?}"), "present", "absent");
    }
}

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    compare_pack(
        frame,
        &save.intel,
        save.country_emissions.as_deref(),
        countries,
        report,
    )
}

fn compare_pack(
    frame: &DecodedFrame,
    save: &content_reader_save::intel::Pack,
    emissions: Option<&[content_reader_save::SavedCountryEmission]>,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let contributions = complete::<tables::IntelContributions>(frame)?;
    let sources = complete::<tables::IntelSources>(frame)?;
    let mut roots = BTreeSet::new();
    for row in sources
        .iter()
        .filter(|row| source_key(row) == (u32::MAX, u32::MAX, u32::MAX))
    {
        if !roots.insert(row.owner) {
            return Err(invalid("duplicate intel source owner root"));
        }
    }
    let mut compared = BTreeSet::new();
    for (tag, country) in &save.countries {
        let Some(state) = &country.sources else {
            continue;
        };
        let reserved = emissions
            .iter()
            .flat_map(|rows| rows.iter())
            .any(|emission| {
                emission.identity.actual == ActualCountry::Reserved
                    && emission.identity.own_id == 0
                    && emission.identity.physical_slot > 0
                    && emission.identity.tag == *tag
            });
        let owner = if reserved {
            Some(0)
        } else {
            roots
                .iter()
                .copied()
                .filter(|owner| *owner != 0)
                .find(|owner| country_tag(*owner, countries).ok().flatten() == Some(tag.as_slice()))
        };
        let label = format!("intel.sources.{}", text(tag));
        let Some(owner) = owner else {
            report.difference(label, "absent", "saved source root");
            continue;
        };
        compared.insert(owner);
        let (expected_contributions, expected_sources) = expected(owner, state)?;
        let mut actual_contributions = BTreeMap::new();
        for row in contributions.iter().filter(|row| row.owner == owner) {
            if actual_contributions
                .insert(contribution_key(row), *row)
                .is_some()
            {
                return Err(invalid("duplicate intel contribution occurrence"));
            }
        }
        compare_rows(
            &format!("{label}.contributions"),
            &actual_contributions,
            &expected_contributions
                .into_iter()
                .map(|row| (contribution_key(&row), row))
                .collect(),
            report,
        );
        let mut actual_sources = BTreeMap::new();
        let mut references = BTreeMap::new();
        for &row in sources.iter().filter(|row| {
            row.owner == owner
                || (row.pool == u32::MAX
                    && row.source == 3
                    && same_country(row.owner, owner, countries))
        }) {
            if row.pool == u32::MAX && row.source < 4 {
                let tag = country_tag(row.reference_country, countries)?.map(<[u8]>::to_vec);
                let mut normalized = row;
                normalized.owner = owner;
                normalized.reference_country = 0;
                if references
                    .insert((row.source, row.ordinal), (normalized, tag))
                    .is_some()
                {
                    return Err(invalid("duplicate held intel source reference"));
                }
            } else {
                let mut normalized = row;
                normalized.removed = u8::from(row.removed != 0);
                if source_key(&row) == (u32::MAX, u32::MAX, u32::MAX) {
                    normalized.reference_country = 0;
                }
                if actual_sources
                    .insert(source_key(&row), normalized)
                    .is_some()
                {
                    return Err(invalid("duplicate static intel source occurrence"));
                }
            }
        }
        compare_rows(
            &format!("{label}.statics"),
            &actual_sources,
            &expected_sources
                .into_iter()
                .map(|row| (source_key(&row), row))
                .collect(),
            report,
        );
        let mut expected_refs = BTreeMap::new();
        let mut add_reference = |kind, target, reference: &saved::Reference| {
            expected_refs.insert(
                (kind, target),
                (
                    Source {
                        owner,
                        pool: u32::MAX,
                        source: kind,
                        ordinal: target,
                        id: reference.id,
                        discriminant: reference.discriminant,
                        reference_pool: reference.pool,
                        ..Default::default()
                    },
                    reference.country.clone(),
                ),
            );
        };
        for (kind, reference) in [
            (0, &state.cheat),
            (1, &country.radar_source),
            (2, &country.token_source),
        ] {
            add_reference(
                kind,
                0,
                reference.as_ref().ok_or_else(|| {
                    invalid(format!(
                        "{label}: saved held source reference {kind} absent"
                    ))
                })?,
            );
        }
        if let Some(objects) = &save.strategic_operatives {
            let slot = countries
                .get(&owner)
                .map_or(owner, |country| country.canonical_slot);
            if let Some(operative) = objects.get(&slot) {
                for network in &operative.networks {
                    let target = countries
                        .iter()
                        .find(|(_, country)| country.tag == network.target)
                        .map(|(&reference, _)| reference)
                        .ok_or_else(|| invalid("network source target lacks country identity"))?;
                    let actual_target = references
                        .keys()
                        .find(|(kind, raw)| {
                            *kind == 3
                                && country_tag(*raw, countries).ok().flatten()
                                    == Some(network.target.as_slice())
                        })
                        .map_or(target, |(_, raw)| *raw);
                    add_reference(
                        3,
                        actual_target,
                        network
                            .source
                            .as_ref()
                            .ok_or_else(|| invalid("saved network source reference absent"))?,
                    );
                }
            }
        }
        compare_rows(
            &format!("{label}.references"),
            &references,
            &expected_refs,
            report,
        );
    }
    for owner in roots.difference(&compared) {
        if *owner == 0 {
            report.limitation("reserved intel source root is not emitted without an authenticated positive physical alias", "IntelSources owner0");
        } else {
            report.difference(
                format!("intel.sources.owner.{owner}"),
                "recorded root",
                "absent saved root",
            );
        }
    }
    if contributions.iter().any(|row| !roots.contains(&row.owner))
        || sources.iter().any(|row| {
            !(roots.contains(&row.owner)
                || row.pool == u32::MAX
                    && row.source == 3
                    && roots
                        .iter()
                        .any(|owner| same_country(row.owner, *owner, countries)))
        })
    {
        return Err(invalid("intel source payload has no extent root"));
    }
    report.limitation(
        "source root owner and source-reference country aliases serialize as country tags",
        "IntelSources.reference_country",
    );
    report.compared("IntelContributions", contributions.len());
    report.compared("IntelSources", sources.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intel_sources_saved_accumulator_is_pending_and_values_are_accumulated() {
        let document = content_reader_parser::parser::parse(
            br#"intel={discriminant=7 dynamic_intel_pools={
                {}
                {accumulator={{-7 {}} {-7 {army=-0.00001}}}
                 values={{3 {civilian=1 navy=-2 airforce=0.00003}}}}
                {values={{-9 {}}}}
            }}"#,
        )
        .unwrap();
        let state = saved::parse(&document.items[0].value).unwrap();
        let (contributions, _) = expected(17, &state).unwrap();
        let dynamic: Vec<_> = contributions
            .iter()
            .filter(|row| matches!(row.kind, 5 | 6))
            .map(|row| {
                (
                    row.kind,
                    row.pool,
                    row.ordinal,
                    row.target,
                    [row.civilian, row.army, row.navy, row.air],
                )
            })
            .collect();
        assert_eq!(
            dynamic,
            [
                (5, 0, u32::MAX, 0, [0; 4]),
                (6, 0, u32::MAX, 0, [0; 4]),
                (5, 1, u32::MAX, 2, [0; 4]),
                (5, 1, 0, -7, [0; 4]),
                (5, 1, 1, -7, [0, -1, 0, 0]),
                (6, 1, u32::MAX, 1, [0; 4]),
                (6, 1, 0, 3, [100_000, 0, -200_000, 3]),
                (5, 2, u32::MAX, 0, [0; 4]),
                (6, 2, u32::MAX, 1, [0; 4]),
                (6, 2, 0, -9, [0; 4]),
            ]
        );
    }

    #[test]
    fn intel_sources_oracle_rejects_changed_values_missing_occurrences_and_incomplete_tables() {
        use replay_data::rows::TableId;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let state = saved::Sources {
            discriminant: Some(7),
            cheat: Some(saved::Reference::default()),
            contributions: [
                vec![vec![saved::Keyed {
                    target: -17,
                    values: [1, 2, 3, 4],
                }]],
                vec![],
                vec![],
            ],
            ..Default::default()
        };
        let country = CountryRow {
            id: 17,
            canonical_slot: 19,
            tag: b"MOD".to_vec(),
            ..Default::default()
        };
        let target = CountryRow {
            id: 23,
            canonical_slot: 29,
            tag: b"TGT".to_vec(),
            ..Default::default()
        };
        let countries = BTreeMap::from([(17, &country), (39, &country), (23, &target)]);
        let pack = content_reader_save::intel::Pack {
            countries: BTreeMap::from([(
                b"MOD".to_vec(),
                content_reader_save::intel::Country {
                    sources: Some(state.clone()),
                    radar_source: Some(saved::Reference::default()),
                    token_source: Some(saved::Reference::default()),
                    ..Default::default()
                },
            )]),
            strategic_operatives: Some(BTreeMap::from([(
                19,
                content_reader_save::intel::StrategicOperative {
                    index: 19,
                    networks: vec![content_reader_save::intel::IntelNetworkEntry {
                        target: b"TGT".to_vec(),
                        source: Some(saved::Reference::default()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };
        let (contributions, mut sources) = expected(17, &state).unwrap();
        sources.extend((0..3).map(|source| Source {
            owner: 17,
            pool: u32::MAX,
            source,
            ordinal: 0,
            ..Default::default()
        }));
        sources.push(Source {
            owner: 39,
            pool: u32::MAX,
            source: 3,
            ordinal: 23,
            ..Default::default()
        });
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::IntelContributions.index()] =
            TableView::Complete(TableRows::IntelContributions(contributions.clone()));
        *frame.tables[TableId::IntelSources.index()] =
            TableView::Complete(TableRows::IntelSources(sources));
        let check = |frame: &DecodedFrame| {
            let mut report = Report::default();
            compare_pack(frame, &pack, None, &countries, &mut report).unwrap();
            report
        };
        assert!(check(&frame).differences.is_empty());
        let mut changed = contributions.clone();
        changed
            .iter_mut()
            .find(|row| row.kind == 2 && row.ordinal == 0)
            .unwrap()
            .army = -99;
        *frame.tables[TableId::IntelContributions.index()] =
            TableView::Complete(TableRows::IntelContributions(changed));
        assert_eq!(check(&frame).differences.len(), 1);
        let mut missing = contributions;
        missing.retain(|row| !(row.kind == 2 && row.ordinal == 0));
        *frame.tables[TableId::IntelContributions.index()] =
            TableView::Complete(TableRows::IntelContributions(missing));
        assert_eq!(check(&frame).differences.len(), 1);
        *frame.tables[TableId::IntelContributions.index()] = TableView::Unavailable {
            reason: "lost writer".into(),
        };
        assert!(compare_pack(&frame, &pack, None, &countries, &mut Report::default()).is_err());
    }

    #[test]
    fn intel_sources_projection_keeps_empty_pools_duplicate_keys_and_zero_occurrences() {
        let state = saved::Sources {
            discriminant: Some(7),
            allied: vec![[0; 4]; 5000],
            contributions: [
                vec![vec![
                    saved::Keyed {
                        target: -17,
                        values: [0; 4]
                    };
                    2
                ]],
                vec![],
                vec![],
            ],
            statics: vec![saved::StaticPool {
                next_id: -9,
                sources: vec![saved::Source {
                    id: -8,
                    removed: true,
                    values: vec![saved::Keyed {
                        target: 2,
                        values: [1, 2, 3, 4],
                    }],
                }],
            }],
            ..Default::default()
        };
        let (contributions, sources) = expected(9, &state).unwrap();
        assert_eq!(contributions.iter().filter(|row| row.kind == 0).count(), 1);
        assert_eq!(contributions[0].target, 5000);
        let entries: Vec<_> = contributions
            .iter()
            .filter(|row| row.kind == 2 && row.ordinal != u32::MAX)
            .collect();
        assert_eq!(entries.len(), 2);
        assert_ne!(contribution_key(entries[0]), contribution_key(entries[1]));
        assert_eq!(entries[0].target, -17);
        assert_eq!(sources[0].discriminant, 7);
        assert_eq!(sources[1].next_id, -9);
        assert_eq!(sources[2].removed, 1);
        assert_eq!(sources[3].navy, 3);
    }
}
