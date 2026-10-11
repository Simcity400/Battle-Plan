use crate::test_fixtures::{CAMPAIGN, records, scale_frame, source};
use replay::{
    container::Replay,
    frame::{FrameDecoder, FrameEncoder, SharedView, TableUpdate, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, TableRows, tables},
};
use std::{
    io,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

#[test]
fn initial_catalogue_discards_countries_and_player_when_block_verification_fails() {
    let mut first = source(1, 100, Some(1));
    *first.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![replay::rows::LocalPlayerRow {
            country: 11,
            observer_country: 0,
        }]));
    let healthy = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![first, scale_frame(1, 101, 5000)])],
    )
    .unwrap();
    let healthy = Arc::new(healthy);
    let path = Path::new("memory.battleplan");
    let good = super::read(&healthy, path, &AtomicBool::new(false)).unwrap();
    assert_eq!(good.countries.len(), 1);
    assert_eq!(good.player_country_id.as_deref(), Some("7"));
    let block = &healthy.index[0];
    let mut bytes = healthy.bytes().to_vec();
    // Leave the decoded prefix intact and invalidate only the zstd checksum.
    bytes[(block.offset + block.length - 1) as usize] ^= 1;
    let damaged = Arc::new(Replay::open(bytes).unwrap());
    let mut observed = false;
    assert!(
        damaged
            .visit_block(&damaged.index[0], Some(&super::CATALOGUE_TABLES), |_| {
                observed = true;
                Ok(false)
            })
            .is_err()
    );
    assert!(
        observed,
        "fixture must supply a country frame before failing verification"
    );
    let metadata = super::read(&damaged, path, &AtomicBool::new(false)).unwrap();
    assert!(metadata.countries.is_empty());
    assert_eq!(metadata.player_country_id, None);
    assert!(
        metadata
            .warnings
            .iter()
            .any(|line| line.contains("initial block is unreadable"))
    );
}

#[test]
fn link_tags_balance_reads_keep_mapping_and_identity_changes_rebuild_at_5000_countries() {
    let mut sample = scale_frame(1, 100, 5000);
    let mut tags = super::LinkTags::default();
    assert!(tags.update(&sample).unwrap());
    assert!(!tags.update(&sample).unwrap());
    let allocation = tags.get(100_000).unwrap().as_ptr();
    let previous = sample.tables[TableId::Countries.index()].clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        for country in rows {
            country.political_power_raw = Some(12_345);
        }
    }
    assert!(!SharedView::ptr_eq(
        &previous,
        &sample.tables[TableId::Countries.index()],
    ));
    assert!(!tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000).unwrap().as_ptr(), allocation);
    assert!(
        tags.from
            .as_ref()
            .unwrap()
            .countries
            .is(&sample.tables[TableId::Countries.index()])
    );
    // Naming the table does not hold its rows, so a later hour updates them in place.
    let countries = std::mem::replace(
        &mut sample.tables[TableId::Countries.index()],
        TableView::Unavailable {
            reason: "taken".into(),
        }
        .into(),
    );
    sample.tables[TableId::Countries.index()] = countries
        .try_unwrap()
        .expect("only the frame holds the rows")
        .into();
    for country in sample.complete::<tables::Countries>().unwrap() {
        assert_eq!(
            tags.get(country.id).unwrap().as_bytes(),
            country.tag.as_slice(),
        );
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed opaque tag".to_vec();
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000).unwrap(), "renamed opaque tag");
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 5000;
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000), None);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *sample.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = 1;
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000), tags.get(100_001));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows.remove(1);
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000), None);
}

#[test]
fn link_tags_incomplete_reads_are_errors_and_recovery_rebuilds_even_the_same_views() {
    let complete = source(1, 100, Some(1));
    let mut tags = super::LinkTags::default();
    for table in [TableId::Countries, TableId::CountryLinks] {
        for truncated in [false, true] {
            assert!(tags.update(&complete).unwrap());
            let mut failed = complete.clone();
            let rows = match &*failed.tables[table.index()] {
                TableView::Complete(rows) => rows.clone(),
                _ => unreachable!(),
            };
            *failed.tables[table.index()] = if truncated {
                TableView::Truncated { rows, missing: 1 }
            } else {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
            };
            assert!(tags.update(&failed).is_err());
            assert_eq!(tags.get(11).unwrap(), "GER");
            assert!(tags.from.is_none());
        }
    }
    assert!(tags.update(&complete).unwrap());
    assert!(!tags.update(&complete).unwrap());
}

#[test]
fn link_tags_aliases_join_slots_without_requiring_a_countries_own_link() {
    // Each raw link resolves to a slot; distinct links may name the same
    // country. Missing target rows name none.
    let mut sample = source(1, 100, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"GER".to_vec(),
            ..Default::default()
        });
    }
    *sample.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(11, 7), (12, 7), (13, 8), (14, 9), (9001, 7), (9002, 8)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .to_vec(),
    ));
    let mut tags = super::LinkTags::default();
    assert!(tags.update(&sample).unwrap());
    for raw in [11, 12, 13] {
        assert_eq!(tags.get(raw).unwrap(), "GER");
    }
    assert_eq!(tags.get(14), None);
    // Equal tag strings do not merge the two canonical countries.
    assert_eq!(tags.get(9001).unwrap(), "GER");
    assert_eq!(tags.get(9002).unwrap(), "GER");
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *sample.tables[TableId::CountryLinks.index()]
    {
        rows.truncate(4);
    }
    assert!(tags.update(&sample).unwrap());
    for raw in [11, 12, 13] {
        assert_eq!(tags.get(raw).unwrap(), "GER");
    }
    assert_eq!(tags.get(9001), None);
    assert_eq!(tags.get(9002), None);
}

#[test]
fn link_tags_contradictory_raw_links_and_conflicting_slots_never_reach_a_scan() {
    for duplicate_slot in [false, true] {
        let mut sample = source(1, 100, Some(1));
        let table = if duplicate_slot {
            TableId::Countries
        } else {
            TableId::CountryLinks
        };
        let mut wire = FrameEncoder::new().encode(sample.clone(), false).unwrap();
        if duplicate_slot {
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *sample.tables[TableId::Countries.index()]
            {
                rows.push(CountryRow {
                    id: 9002,
                    canonical_slot: 7,
                    tag: b"other".to_vec(),
                    ..Default::default()
                });
            }
        } else if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *sample.tables[TableId::CountryLinks.index()]
        {
            rows.insert(
                1,
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 8,
                },
            );
        }
        let TableView::Complete(rows) = &*sample.tables[table.index()] else {
            unreachable!();
        };
        wire.tables[table.index()] = TableUpdate::Snapshot(rows.clone());
        let error = FrameEncoder::new().encode(sample, false).unwrap_err();
        let reason = if duplicate_slot {
            "duplicate canonical slots"
        } else {
            "unique ascending keys"
        };
        assert!(error.to_string().contains(reason), "{error}");
        let decoded = FrameDecoder::new().apply(&wire).unwrap();
        assert!(matches!(
            &*decoded.tables[table.index()],
            TableView::Unavailable { reason: why } if why.contains(reason)
        ));
        assert!(super::LinkTags::default().update(&decoded).is_err());
    }
}

#[test]
#[ignore = "manual LinkTags::update timing in the test profile"]
fn link_tags_update_cost_5000_countries() {
    use std::{hint::black_box, time::Instant};
    let first = scale_frame(1, 100, 5000);
    // Prepare allocations before timing; Countries changes every built hour,
    // while CountryLinks retains the same view as an hourly replay decoder.
    let hours: Vec<_> = (101..=124)
        .map(|hour| {
            let mut sample = first.clone();
            sample.hour = hour;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *sample.tables[TableId::Countries.index()]
            {
                for country in rows {
                    country.political_power_raw = Some(i64::from(hour));
                }
            }
            sample
        })
        .collect();
    let mut costs = Vec::new();
    for _ in 0..5 {
        let mut tags = super::LinkTags::default();
        tags.update(&first).unwrap();
        let start = Instant::now();
        for _ in 0..10 {
            for hour in &hours {
                black_box(tags.update(black_box(hour)).unwrap());
            }
        }
        let ns_per_hour = start.elapsed().as_nanos() / (10 * hours.len()) as u128;
        assert_eq!(tags.iter().count(), 5000);
        costs.push(ns_per_hour);
    }
    costs.sort_unstable();
    eprintln!(
        "LinkTags::update 5000 countries, 240 built hours/sample: {costs:?} ns/hour; median {} ns/hour",
        costs[2]
    );
}

#[test]
#[ignore = "read-only G2 recording; BATTLE_PLAN_COMPAT_RECORDINGS supplies its path"]
fn compatibility_real_g2_product_impact() {
    use crate::{
        products::{self, LoadContext, Product},
        scope::{Context, RecordedScopes, Target, Unresolved},
    };
    use installed_content::{
        localisation::Localisation,
        scope::{Keyword, Scope},
        tag_aliases::TagAliases,
    };
    let path = std::env::var("BATTLE_PLAN_COMPAT_RECORDINGS").or_else(|_| std::env::var("HOI4_COMPAT_RECORDINGS")).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = crate::Source::open(
        crate::SourceKey {
            source_id: "g2".into(),
            revision: "read-only".into(),
        },
        path.into(),
        &cancelled,
    )
    .unwrap();
    let replay = source.replay();
    assert_eq!(replay.format.version(), 94);
    let replay::container::SeekResult::Frame(frame) = replay.seek(2, 60827796).unwrap() else {
        panic!("missing G2 comparison hour")
    };
    let countries = frame.complete::<replay::rows::tables::Countries>().unwrap();
    let country = countries
        .iter()
        .find(|country| {
            country.canonical_slot > 0
                && country.tag.len() == 3
                && country.tag.iter().all(u8::is_ascii_alphabetic)
        })
        .unwrap();
    let tag = std::str::from_utf8(&country.tag).unwrap();
    let localisation = Localisation::default();
    let styles = products::country_styles::HourStyles::read(&frame, Some(&localisation)).unwrap();
    let coloured = countries
        .iter()
        .filter(|country| {
            let tag = std::str::from_utf8(&country.tag).unwrap();
            styles.get(tag).is_some_and(|style| style.color.is_some())
        })
        .count();
    assert!(coloured > 0);
    let flags = products::country_flags::choices(&frame).unwrap();
    assert!(!flags.is_empty());
    let appearances = frame
        .complete::<replay::rows::tables::CountryAppearance>()
        .unwrap();
    let links = frame
        .complete::<replay::rows::tables::CountryLinks>()
        .unwrap();
    for flag in &flags {
        let canonical = links
            .iter()
            .find(|row| row.id == flag.raw)
            .unwrap()
            .canonical_slot;
        let country = countries
            .iter()
            .rev()
            .find(|row| row.canonical_slot == canonical)
            .unwrap();
        let Some(appearance) = appearances.iter().find(|row| row.country == country.id) else {
            assert!(
                flag.refusal
                    .as_deref()
                    .is_some_and(|why| why.contains("CountryAppearance"))
            );
            continue;
        };
        // Format 94 retains cosmetic candidates but has no exact signed
        // original-tag selection operand, even for an aliased raw link.
        let refusal = installed_content::country_names::original_tag(country, appearance).unwrap_err();
        assert!(refusal.contains("CountryAppearance.original_tag_reference not recorded"));
        assert!(
            flag.refusal
                .as_deref()
                .is_some_and(|why| why.contains(&refusal))
        );
    }
    let names = installed_content::country_names::CountryNames::read(&frame, &localisation);
    assert!(names.operands(country.canonical_slot).unwrap().is_ok());
    let scopes = RecordedScopes::read(&frame, Arc::new(TagAliases::default()));
    assert_eq!(scopes.resolve_tag(tag).unwrap(), country.canonical_slot);
    let context = Context::country(country.canonical_slot);
    assert!(matches!(
        scopes.resolve(&Scope::Keyword(Keyword::Overlord), &context),
        Ok(_) | Err(Unresolved::Nothing(_))
    ));
    assert_eq!(
        scopes
            .resolve(&Scope::Keyword(Keyword::Root), &context)
            .unwrap(),
        Target::Country(country.canonical_slot)
    );
    let context = LoadContext { content: None };
    let measure = products::country_measure::CountryMeasure::load(
        &source,
        &products::country_measure::MeasureParams {
            branch_id: "2".into(),
            measure: products::country_measure::Measure::CommandPower,
            tags: vec![tag.into()],
        },
        &context,
        &cancelled,
    )
    .unwrap();
    assert!(measure.series.iter().any(|series| !series.runs.is_empty()));
    let owners = products::land_owners::land_owners(&replay, None, &cancelled).unwrap();
    println!(
        "G2_PRODUCT country-styles: {} colour rows; country naming operands retained (localisation not loaded in this check); country-flags: {} native cosmetic/original choices; OVERLORD retained",
        coloured,
        flags.len()
    );
    println!(
        "G2_PRODUCT command-power: {} runs; land-owners: {} countries",
        measure
            .series
            .iter()
            .map(|series| series.runs.len())
            .sum::<usize>(),
        owners.countries.len()
    );
    for table in [
        replay::rows::TableId::CountryAppearance,
        replay::rows::TableId::DivisionTemplates,
        replay::rows::TableId::Decisions,
        replay::rows::TableId::DivisionMovement,
    ] {
        assert!(matches!(
            &*frame.tables[table.index()],
            TableView::Complete(_)
        ));
    }
}

#[test]
fn metadata_compatibility_opens_format94_and_surfaces_every_named_gap() {
    use replay::{codec::put_bytes, format::FormatVersion, spool::Record};

    let format = FormatVersion::read(94).unwrap();
    let mut frame = source(1, 1000, Some(1));
    for table in [
        TableId::CountryAppearance,
        TableId::DivisionTemplates,
        TableId::Decisions,
        TableId::DivisionMovement,
    ] {
        // Empty snapshots have the same wire framing in both contracts.
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    let mut records = records(1, vec![frame]);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!();
    };
    session.identity.coverage_policy = 3;
    let Record::Frame(frame) = &records[2] else {
        unreachable!();
    };
    let mut current = Vec::new();
    let sizes = frame.encode_with_sizes(&mut current).unwrap();
    let mut payload = Vec::new();
    payload.extend_from_slice(&frame.branch.to_le_bytes());
    payload.extend_from_slice(&frame.hour.to_le_bytes());
    payload.push(u8::from(frame.keyframe));
    payload.extend_from_slice(&frame.metrics.total_ns.to_le_bytes());
    for table in TableId::ALL {
        if table.last_change().present(format.version()) {
            payload.extend_from_slice(&frame.metrics.table_ns[table.index()].to_le_bytes());
        }
    }
    payload.extend_from_slice(&frame.metrics.pool_bytes.to_le_bytes());
    payload.extend_from_slice(&frame.metrics.dropped_frames.to_le_bytes());
    let mut at = current.len() - sizes.iter().sum::<usize>();
    for table in TableId::ALL {
        let end = at + sizes[table.index()];
        if table.last_change().present(format.version()) {
            // Only unchanged nonempty prefixes, empty snapshots and unavailable
            // updates can be copied directly into this historical fixture.
            assert!(sizes[table.index()] > 0, "{table:?}");
            match &frame.tables[table.index()] {
                TableUpdate::Snapshot(rows) if !rows.is_empty() => {
                    assert!(format.table_refusal(table).is_none(), "{table:?}");
                    assert_eq!(format.field_gaps(table).count(), 0, "{table:?}");
                    assert!(matches!(
                        table,
                        TableId::Campaign | TableId::Countries | TableId::CountryLinks
                    ));
                }
                TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. } => {}
                _ => panic!("historical fixture has a dependent {table:?} update"),
            }
            payload.extend_from_slice(&current[at..end]);
        }
        at = end;
    }
    assert_eq!(at, current.len());
    let mut raw = 1_u32.to_le_bytes().to_vec();
    put_bytes(&mut raw, &payload).unwrap();
    let compressed = zstd::encode_all(raw.as_slice(), 3).unwrap();
    let current = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    assert_eq!(current.index.len(), 1);
    let block = &current.index[0];
    let trailer_start = (block.offset + block.length) as usize;
    let footer_start = current.bytes().len() - (4 + 8 + 8 + 4);
    let mut trailer = current.bytes()[trailer_start..footer_start].to_vec();
    // The sole index entry is the trailer's final field. Keep all session and
    // branch metadata, replacing only the block's compressed/raw lengths.
    let lengths_at = trailer.len() - (8 + 8);
    assert_eq!(
        &trailer[lengths_at..],
        [block.length.to_le_bytes(), block.raw_length.to_le_bytes()].concat()
    );
    trailer[lengths_at..lengths_at + 8].copy_from_slice(&(compressed.len() as u64).to_le_bytes());
    trailer[lengths_at + 8..].copy_from_slice(&(raw.len() as u64).to_le_bytes());
    let mut bytes = current.bytes()[..block.offset as usize].to_vec();
    bytes[4..6].copy_from_slice(&format.version().to_le_bytes());
    bytes.extend_from_slice(&compressed);
    let trailer_start = bytes.len() as u64;
    bytes.extend_from_slice(&trailer);
    bytes.extend_from_slice(b"H4FT");
    bytes.extend_from_slice(&trailer_start.to_le_bytes());
    bytes.extend_from_slice(&(trailer.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&crc32fast::hash(&trailer).to_le_bytes());
    let replay = Arc::new(Replay::open(bytes).unwrap());
    let metadata = crate::metadata::read(
        &replay,
        Path::new("format94.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(metadata.format, 94);
    assert_eq!(metadata.countries.len(), 1);
    assert_eq!(metadata.countries[0].tag, "GER");
    let expected = TableId::ALL
        .into_iter()
        .flat_map(|table| {
            format
                .table_refusal(table)
                .into_iter()
                .chain(format.field_gaps(table))
        })
        .chain(format.nested_field_gaps())
        .collect::<Vec<_>>();
    assert_eq!(replay.compatibility_gaps(), expected);
    assert_eq!(metadata.warnings, expected);
    for table in [
        TableId::CountryAppearance,
        TableId::DivisionTemplates,
        TableId::Decisions,
        TableId::DivisionMovement,
    ] {
        assert!(metadata.warnings.iter().any(|warning| {
            warning.starts_with(&format!("{table:?}."))
                && warning.contains("this recording is format 94")
        }));
    }
}

#[test]
fn metadata_partial_marker_keeps_the_failure_separate_from_known_and_unknown_hour_gaps() {
    use replay::{
        branches::Gap,
        container::SeekResult,
        spool::{ControlRecord, Record},
    };
    for failed_hour in [Some(1017), None] {
        let mut records = records(1, vec![source(1, 1000, Some(1))]);
        if let Some(hour) = failed_hour {
            records.push(Record::Gap(Gap {
                branch: 1,
                first_hour: hour,
                last_hour: hour,
                reason: "collector allocation failed".into(),
            }));
        }
        records.push(Record::Control(ControlRecord::Partial {
            session: 1,
            reason: "collector allocation failed".into(),
            failed_hour,
        }));
        records.push(Record::Control(ControlRecord::Detach {
            session: 1,
            last_hour: Some(1000),
        }));
        let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records]).unwrap());
        let metadata = crate::metadata::read(
            &replay,
            Path::new("partial.battleplan"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            metadata.partial,
            Some(crate::PartialRecording {
                session_id: "1".into(),
                reason: "collector allocation failed".into(),
                failed_hour,
            })
        );
        assert_eq!(
            metadata.branches[0].gaps.len(),
            usize::from(failed_hour.is_some())
        );
        assert_eq!(metadata.branches[0].last_hour, failed_hour.unwrap_or(1000));
        assert!(matches!(
            replay.seek(1, 1000).unwrap(),
            SeekResult::Frame(_)
        ));
        if failed_hour.is_some() {
            assert_eq!(metadata.branches[0].gaps[0].first_hour, 1017);
            assert_eq!(
                metadata.branches[0].gaps[0].reason,
                "collector allocation failed"
            );
            assert!(matches!(
                replay.seek(1, 1017).unwrap(),
                SeekResult::Gap { .. }
            ));
        }
    }
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 1000, Some(1))])],
        )
        .unwrap(),
    );
    let metadata = crate::metadata::read(
        &replay,
        Path::new("healthy.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(metadata.partial, None);
}

#[test]
fn metadata_skips_an_unavailable_first_census_warns_on_bad_links_and_stops_when_cancelled() {
    let mut first = source(1, 100, Some(1));
    *first.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "census failed".into(),
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![first, source(1, 101, Some(1))])],
        )
        .unwrap(),
    );
    let path = Path::new("memory.battleplan");
    let metadata = crate::metadata::read(&replay, path, &AtomicBool::new(false)).unwrap();
    assert_eq!(metadata.branches[0].first_hour, 100);
    assert_eq!(metadata.countries.len(), 1);
    assert_eq!(metadata.countries[0].observed_hour, 101);
    assert_eq!(metadata.countries[0].tag, "GER");
    assert_eq!(metadata.player_country_id, None);
    assert_eq!(
        crate::metadata::read(&replay, path, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );

    let mut bad = source(1, 100, Some(1));
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *bad.tables[TableId::CountryLinks.index()]
    {
        links[1].canonical_slot = 8;
    }
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![bad])]).unwrap());
    let metadata = crate::metadata::read(&replay, path, &AtomicBool::new(false)).unwrap();
    assert!(metadata.countries.is_empty());
    assert!(
        metadata
            .warnings
            .iter()
            .any(|warning| warning.contains("CountryLinks"))
    );
}

#[test]
fn metadata_catalogue_lists_every_country_once_across_branches_within_the_reply_budget() {
    const COUNTRIES: u32 = 65_535;
    let branches: Vec<_> = (1..=3)
        .map(|branch| records(branch, vec![scale_frame(branch, 100, COUNTRIES)]))
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &branches).unwrap());
    let metadata = crate::metadata::read(
        &replay,
        Path::new("scale.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    // Not countries x branches: each country once, at its first branch.
    assert_eq!(metadata.countries.len(), COUNTRIES as usize);
    assert!(
        metadata
            .countries
            .iter()
            .all(|country| country.observed_branch_id == "1")
    );
    let reply = crate::transport::SourceReply {
        id: "lease".into(),
        path: metadata.path.clone(),
        revision: "revision".into(),
        metadata,
    };
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    assert!(
        bytes < crate::transport::MAX_REPLY_BYTES / 2,
        "{bytes} bytes for {COUNTRIES} countries"
    );
}

#[test]
fn a_tag_is_found_absent_or_unresolved() {
    use crate::metadata::{TaggedCountry, tagged_country};
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    let countries = [country(21, 8, b"ITA"), country(9001, 7, b"GER")];
    let links = [
        CountryLinkRow {
            id: 21,
            canonical_slot: 8,
        },
        CountryLinkRow {
            id: 9001,
            canonical_slot: 7,
        },
    ];
    assert_eq!(
        tagged_country(&countries, &links, "GER"),
        TaggedCountry::Found(&countries[1], 7)
    );
    assert_eq!(
        tagged_country(&countries, &links, "FRA"),
        TaggedCountry::Absent
    );
    assert_eq!(
        tagged_country(&countries, &links[..1], "GER"),
        TaggedCountry::Unresolved
    );
    assert_eq!(tagged_country(&countries, &links[..1], "GER").found(), None);
}

fn daily_slot_of(table: TableId) -> i32 {
    let replay::cadence::Cadence::Daily(slot) = replay::cadence::cadence(table) else {
        panic!("{table:?} is read in a daily slot");
    };
    slot as i32
}

#[test]
fn a_slot_hour_reads_its_tables_even_when_another_table_is_charged() {
    let slot = daily_slot_of(TableId::CountryFuel);
    let day = 1000 * replay::cadence::STAGGER_PERIOD;
    // The shared economy read is charged to hourly ProvinceSupply, so no
    // table of the slot is timed at its slot hour.
    let mut charged = replay::frame::Metrics::default();
    charged.table_ns[TableId::ProvinceSupply.index()] = 1_051_200;
    for table in [TableId::CountryFuel, TableId::CountryExperience] {
        assert!(super::slot_read(&charged, table, day + slot));
        assert!(!super::slot_read(&charged, table, day + slot + 1));
    }
    // A full hour times the slot's tables off their slot hour.
    let mut full = replay::frame::Metrics::default();
    full.table_ns[TableId::CountryExperience.index()] = 72_100;
    assert!(super::slot_read(
        &full,
        TableId::CountryFuel,
        day + slot + 9
    ));
}

#[test]
fn country_intel_is_dated_only_by_its_measured_walk() {
    let slot = daily_slot_of(TableId::CountryIntel);
    let hour = 1000 * replay::cadence::STAGGER_PERIOD + slot;
    assert!(!super::slot_read(
        &replay::frame::Metrics::default(),
        TableId::CountryIntel,
        hour
    ));
    let mut timed = replay::frame::Metrics::default();
    timed.table_ns[TableId::CountryIntel.index()] = 1;
    assert!(super::slot_read(&timed, TableId::CountryIntel, hour + 5));
    // Intel can move off its slot independently of the other daily walks.
    let sibling = TableId::LogisticsEquipment;
    assert_eq!(daily_slot_of(sibling), slot);
    assert!(!super::slot_read(&timed, sibling, hour + 5));
    let mut sibling_timed = replay::frame::Metrics::default();
    sibling_timed.table_ns[sibling.index()] = 1;
    for sampled_hour in [hour, hour + 5] {
        assert!(!super::slot_read(
            &sibling_timed,
            TableId::CountryIntel,
            sampled_hour
        ));
    }
}

#[test]
fn every_tag_is_tagged_in_one_pass_as_tagged_country_tags_it() {
    use crate::metadata::{tagged_countries, tagged_country};
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    // A second GER row the links resolve after one they do not, and a tag no link resolves.
    let countries = [
        country(5, 9, b"GER"),
        country(9001, 7, b"GER"),
        country(21, 8, b"ITA"),
        country(33, 4, b"FRA"),
    ];
    let links = [
        CountryLinkRow {
            id: 21,
            canonical_slot: 8,
        },
        CountryLinkRow {
            id: 9001,
            canonical_slot: 7,
        },
    ];
    let tagged = tagged_countries(&countries, &links);
    assert_eq!(tagged.len(), 3);
    for tag in ["GER", "ITA", "FRA"] {
        assert_eq!(
            tagged[tag.as_bytes()],
            tagged_country(&countries, &links, tag),
            "{tag}"
        );
    }
}

#[test]
fn a_table_is_read_by_its_slot_or_timing_and_a_deferred_intel_walk_proves_nothing_else() {
    use crate::metadata::slot_read;
    use crate::test_fixtures::slot_hour;
    let walked = |tables: &[TableId]| {
        let mut metrics = replay::frame::Metrics::default();
        for table in tables {
            metrics.table_ns[table.index()] = 1;
        }
        metrics
    };
    let slot = slot_hour(TableId::LogisticsEquipment);
    assert_eq!(slot, slot_hour(TableId::CountryIntel));
    // Intel deferred at the slot hour, then retried alone. Its retry cannot
    // date the logistics rows carried from the slot read.
    let deferred = walked(&[TableId::LogisticsEquipment]);
    assert!(slot_read(&deferred, TableId::LogisticsEquipment, slot));
    assert!(!slot_read(&deferred, TableId::CountryIntel, slot));
    let retry = walked(&[TableId::CountryIntel]);
    assert!(slot_read(&retry, TableId::CountryIntel, slot + 1));
    assert!(!slot_read(&retry, TableId::LogisticsEquipment, slot + 1));

    let fuel = slot_hour(TableId::CountryFuel);
    let shared = walked(&[TableId::CountryFuelConsumers]);
    assert!(slot_read(&shared, TableId::CountryFuel, fuel));
    assert!(!slot_read(&walked(&[]), TableId::CountryFuel, fuel + 1));
    // Off-slot timing dates its own slot only. A full read needs evidence
    // for the other slots too, and Intel always needs its own timing.
    let off_slot = fuel + 3;
    assert!(slot_read(&shared, TableId::CountryFuel, off_slot));
    for table in [
        TableId::Stockpiles,
        TableId::ProductionLines,
        TableId::LogisticsEquipment,
        TableId::CountryIntel,
    ] {
        assert!(!slot_read(&shared, table, off_slot), "{table:?}");
    }
    let full = walked(&[
        TableId::CountryFuelConsumers,
        TableId::DeploymentLines,
        TableId::LogisticsEquipment,
        TableId::CountryIntel,
    ]);
    for table in [
        TableId::CountryFuel,
        TableId::Stockpiles,
        TableId::ProductionLines,
        TableId::LogisticsEquipment,
        TableId::CountryIntel,
    ] {
        assert!(slot_read(&full, table, off_slot), "{table:?}");
    }
    let stock = slot_hour(TableId::Stockpiles);
    assert!(slot_read(
        &walked(&[TableId::DeploymentLines]),
        TableId::ProductionLines,
        stock
    ));
    assert!(!slot_read(&walked(&[]), TableId::Stockpiles, stock + 1));
    // Resources are now hourly; a lack of daily-slot timing cannot make them carried.
    for table in [TableId::CountryResources, TableId::ResourceExchanges] {
        assert!(slot_read(&walked(&[]), table, fuel));
        assert!(slot_read(&walked(&[]), table, fuel + 2));
    }
    // The shared economy read can be charged to an hourly anchor instead.
    assert!(slot_read(&walked(&[]), TableId::CountryFuel, fuel));
    assert!(slot_read(&walked(&[]), TableId::LogisticsEquipment, slot));
}
