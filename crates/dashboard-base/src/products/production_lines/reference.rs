use super::*;

// Independent pre-family hourly reference; no owned-input fold or page assembler.
use crate::metadata::tagged_country;
use equipment::VariantHour;
use replay_frame::frame::{FrameDecoder, SharedView};

fn observe_design(scanned: &mut ScannedLines, hour: &ScanHour, variant: u64) {
    let row = hour.designs.as_ref().and_then(|view| match &**view {
        replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::VariantStats(
            rows,
        )) => rows
            .binary_search_by_key(&variant, |row| row.variant)
            .ok()
            .map(|index| &rows[index]),
        _ => None,
    });
    let read = DesignRead {
        first_hour: hour.hour,
        read_hour: row.and(hour.design_read_hour),
        author: row.map(|row| row.organisation),
        trait_count: row.map(|row| row.organisation_trait_count),
        bonus_count: row.map(|row| row.organisation_bonuses.len() as u32),
    };
    version(scanned.designs.entry(variant).or_default(), read, |a, b| {
        DesignRead {
            first_hour: 0,
            ..*a
        } == DesignRead {
            first_hour: 0,
            ..*b
        }
    });
    if let Some(author) = read.author.filter(|&id| id != 0)
        && !scanned.organisations.contains_key(&author)
        && let Some(frame) = hour.frame.as_ref()
        && let (Ok(rows), Ok(countries), Ok(links), Ok(lexical)) = (
            frame.complete::<tables::IndustrialOrganisations>(),
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::LexicalTokens>(),
        )
        && let Ok(index) = rows.binary_search_by_key(&author, |row| row.organisation)
    {
        let tags = Tags::new(
            countries,
            links,
            frame.complete::<tables::CountryAppearance>().ok(),
        );
        scanned.organisations.insert(
            author,
            vec![raw_organisation(&rows[index], hour.hour, &tags, lexical)],
        );
    }
}

fn select_country(
    decoded: &DecodedFrame,
    tag: &str,
    holder: Option<(u32, u32)>,
) -> Option<Country> {
    let _phase = replay_frame::cpu::scope_phase("production-lines:country");
    crate::prepared::prepass::note("production-lines.country_lookup");
    decoded
        .complete::<tables::Countries>()
        .ok()
        .zip(decoded.complete::<tables::CountryLinks>().ok())
        .map(|(countries, links)| {
            let selected_tag = match holder {
                None => Some(tag),
                Some((slot, id)) => countries
                    .iter()
                    .find(|country| country.canonical_slot == slot && country.id == id)
                    .and_then(|country| std::str::from_utf8(&country.tag).ok()),
            };
            match selected_tag.map_or(TaggedCountry::Absent, |tag| {
                tagged_country(countries, links, tag)
            }) {
                TaggedCountry::Absent => Country::Absent,
                TaggedCountry::Unresolved => Country::Unresolved,
                TaggedCountry::Found(country, slot) => Country::Found(slot, country.id),
            }
        })
}

struct ScanHour {
    hour: i32,
    last_hour: i32,
    country: Option<Country>,
    output_due: Option<bool>,
    variants: VariantHour,
    definitions_read: bool,
    lines_complete: bool,
    lines_read: bool,
    output_complete: bool,
    organisations_complete: bool,
    modifiers_complete: bool,
    modifiers_read: bool,
    designs: Option<SharedView>,
    design_read_hour: Option<i32>,
    frame: Option<DecodedFrame>,
}

fn scan_hour_selected(
    format: replay_data::format::FormatVersion,
    hour: i32,
    decoded: &DecodedFrame,
    country: Option<Country>,
) -> ScanHour {
    let reads = slot_reads(format, &decoded.metrics, decoded.hour);
    let read = |table: TableId| reads[table.index()];
    let lines_complete = decoded.complete::<tables::ProductionLines>().is_ok();
    let lines_read = read(TableId::ProductionLines);
    let output_complete = decoded.complete::<tables::ProductionLineOutput>().is_ok();
    let output_due = read(TableId::ProductionLineOutput).then_some(match country {
        Some(Country::Absent) => true,
        Some(Country::Found(..)) => lines_read && lines_complete && output_complete,
        Some(Country::Unresolved) | None => false,
    });
    let variants = VariantCatalogue::hour_read(decoded, read(TableId::VariantDefinitions));
    let definitions_read = (equipment::design_changed(decoded)
        || read(TableId::VariantDefinitions))
        && decoded.complete::<tables::VariantDefinitions>().is_ok();
    let lines_of_country =
        matches!(country, Some(Country::Found(..))) && lines_complete && lines_read;
    let designs = decoded
        .complete::<tables::VariantStats>()
        .ok()
        .map(|_| decoded.tables[TableId::VariantStats.index()].clone());
    let design_read_hour = designs.as_ref().and_then(|view| {
        view.sampled_at()
            .or_else(|| decoded.tables[TableId::VariantModules.index()].sampled_at())
    });
    let keep = variants == VariantHour::Read
        || definitions_read
        || lines_of_country
        || read(TableId::VariantStats)
        || read(TableId::VariantModules);
    ScanHour {
        hour,
        last_hour: hour,
        country,
        output_due,
        variants,
        definitions_read,
        designs,
        design_read_hour,
        lines_complete,
        lines_read,
        output_complete,
        organisations_complete: decoded
            .complete::<tables::IndustrialOrganisations>()
            .is_ok(),
        modifiers_complete: decoded
            .complete::<tables::CountryProductionModifiers>()
            .is_ok()
            && decoded
                .complete::<tables::ProductionLineModifiers>()
                .is_ok(),
        modifiers_read: read(TableId::CountryProductionModifiers)
            && read(TableId::ProductionLineModifiers),
        frame: keep.then(|| decoded.clone()),
    }
}

struct Extracted {
    hours: Vec<ScanHour>,
    lost_from: Option<i32>,
}
fn frame_not_kept(hour: i32) -> io::Error {
    io::Error::other(format!("Missing reference frame at {hour}"))
}
pub(super) fn scan(
    replay: &Replay,
    blocks: &[&BlockIndex],
    tag: &str,
    icons: Option<&IconContent>,
    holder: Option<(u32, u32)>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedLines> {
    let mut selected = vec![
        TableId::Campaign,
        TableId::ProductionLines,
        TableId::ProductionLineOutput,
        TableId::IndustrialOrganisations,
        TableId::IndustrialOrganisationDetails,
        TableId::IndustrialOrganisationPolicies,
        TableId::VariantStats,
        TableId::CountryProductionModifiers,
        TableId::ProductionLineModifiers,
        TableId::ModifierDefinitions,
    ];
    selected.extend(VariantCatalogue::TABLES);
    if icons.is_some() {
        selected.extend([
            TableId::PoliticalParties,
            TableId::Politics,
            TableId::Factions,
            TableId::FactionMembers,
            TableId::States,
        ]);
    }
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Production request cancelled");
    let mut result = ScannedLines::default();
    let recorded_modifier_language = recorded_language(replay);
    let mut last: Option<((u32, u32), i32)> = None;
    let mut open: BTreeMap<u64, Open> = BTreeMap::new();
    let mut carried = false;
    let mut broken: BTreeMap<u64, Open> = BTreeMap::new();
    let mut last_read_identity: Option<(u32, u32)> = None;
    let mut broken_identity: Option<(u32, u32)> = None;
    let mut first_read = true;
    let mut output_open = false;
    let mut organisations_open = false;
    let mut last_census: Option<i32> = None;
    let mut appearances: BTreeMap<u64, u32> = BTreeMap::new();
    let mut latest: BTreeMap<u64, usize> = BTreeMap::new();
    let mut definitions: BTreeMap<u64, VariantDefinitionRow> = BTreeMap::new();
    let mut pending: BTreeSet<u64> = BTreeSet::new();
    let mut last_output_due: Option<i32> = None;
    let mut last_modifier_country: Option<Arc<replay_data::rows::CountryProductionModifiersRow>> =
        None;
    let mut last_definitions: Option<SharedView> = None;

    let mut last_designs: Option<SharedView> = None;
    let mut last_design_read_hour = None;
    for block in blocks {
        let extracted = (|| -> io::Result<Extracted> {
            let frames = replay.read_block_recorded_selected(block, &selected)?;
            let mut decoder = FrameDecoder::for_tables(&selected);
            let mut hours = Vec::new();
            let mut lost_from = None;
            for frame in &frames {
                crate::metadata::check_cancel(cancelled)?;
                let Ok(decoded) = decoder.advance(frame) else {
                    lost_from = Some(frame.hour);
                    break;
                };
                if crate::metadata::validate_campaign(decoded, replay).is_err() {
                    lost_from = Some(frame.hour);
                    break;
                }
                hours.push(scan_hour_selected(
                    replay.format,
                    decoded.hour,
                    decoded,
                    select_country(decoded, tag, holder),
                ));
            }
            Ok(Extracted { hours, lost_from })
        })();
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let lost = |result: &mut ScannedLines, first: i32, last: i32| {
            result.unreadable_stretches += 1;
            mark_span(&mut result.unread_output, first.min(last), last);
        };
        let Ok(extracted) = extracted else {
            lost(&mut result, block.first_hour, block.last_hour);
            continue;
        };
        for scan_hour in &extracted.hours {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let hour = scan_hour.hour;
            let through = scan_hour.last_hour;
            let identity = scan_hour.country.and_then(Country::found);
            let continues = identity.is_some_and(|identity| {
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour))
            });
            last = identity.map(|identity| (identity, through));
            if let Some(read) = scan_hour.output_due {
                if !read {
                    result.unread_output_reads += 1;
                    let first = last_output_due
                        .and_then(|due| due.checked_add(1))
                        .unwrap_or(hour);
                    mark_span(&mut result.unread_output, first.min(hour), hour);
                }
                last_output_due = Some(hour);
            }
            let decoded = scan_hour.frame.as_ref();
            let lexical =
                decoded.and_then(|decoded| decoded.complete::<tables::LexicalTokens>().ok());
            let mut kept = Ok(());
            result
                .variants
                .observe_hour(hour, scan_hour.variants, |_| match decoded {
                    Some(decoded) => {
                        let countries = decoded.complete::<tables::Countries>().unwrap();
                        let links = decoded.complete::<tables::CountryLinks>().unwrap();
                        let lexical = decoded.complete::<tables::LexicalTokens>().unwrap();
                        let tags = Tags::new(
                            countries,
                            links,
                            decoded.complete::<tables::CountryAppearance>().ok(),
                        );
                        let modules = decoded
                            .complete::<tables::VariantModules>()
                            .ok()
                            .map(equipment::modules_by_variant);
                        let upgrades = decoded
                            .complete::<tables::VariantUpgrades>()
                            .ok()
                            .map(equipment::upgrades_by_variant);
                        decoded
                            .complete::<tables::VariantDefinitions>()
                            .unwrap()
                            .iter()
                            .map(|definition| {
                                let design = modules.as_ref().zip(upgrades.as_ref()).map(
                                    |(modules, upgrades)| {
                                        (
                                            modules
                                                .get(&definition.id)
                                                .map_or(&[][..], Vec::as_slice),
                                            upgrades
                                                .get(&definition.id)
                                                .map_or(&[][..], Vec::as_slice),
                                        )
                                    },
                                );
                                (
                                    definition.id,
                                    equipment::read_raw(definition, hour, &tags, lexical, design),
                                )
                            })
                            .collect()
                    }
                    None => {
                        kept = Err(frame_not_kept(hour));
                        Vec::new()
                    }
                });
            kept?;
            if through != hour {
                result.variants.advance(through);
            }
            if scan_hour.definitions_read
                && let Some(decoded) = decoded
                && let Ok(rows) = decoded.complete::<tables::VariantDefinitions>()
            {
                let mut refresh: BTreeSet<u64> = BTreeSet::new();
                let view = &decoded.tables[TableId::VariantDefinitions.index()];
                if !last_definitions
                    .as_ref()
                    .is_some_and(|held| SharedView::ptr_eq(held, view))
                {
                    for definition in rows {
                        let changed = match definitions.get(&definition.id) {
                            Some(before) if before == definition => false,
                            before => {
                                let changed = before.is_some();
                                definitions.insert(definition.id, definition.clone());
                                changed
                            }
                        };
                        if changed && result.icons.contains_key(&definition.id) {
                            refresh.insert(definition.id);
                        }
                    }
                    last_definitions = Some(view.clone());
                }
                refresh.extend(
                    pending
                        .iter()
                        .copied()
                        .filter(|id| definitions.contains_key(id)),
                );
                refresh.retain(|id| result.variants.current(*id).is_some());
                if let (Some(content), Some(lexical), false) = (icons, lexical, refresh.is_empty())
                {
                    let state = IconHour::read(decoded, content);
                    for id in refresh {
                        pending.remove(&id);
                        let definition = &definitions[&id];
                        let icon = match &state {
                            Err(error) => IconChoice::unknown(error.to_string()),
                            Ok(state) => icon_choice(
                                content,
                                state,
                                definition,
                                &equipment::variant_token(definition, lexical),
                            ),
                        };
                        version(result.icons.entry(id).or_default(), (hour, icon), |a, b| {
                            a.1 == b.1
                        });
                    }
                }
            }
            if carried && (!continues || !scan_hour.lines_complete) {
                broken = std::mem::take(&mut open);
                broken_identity = last_read_identity;
                carried = false;
                output_open = false;
                organisations_open = false;
            }
            let designs_changed = match (&last_designs, &scan_hour.designs) {
                (Some(before), Some(now)) => {
                    !SharedView::ptr_eq(before, now)
                        || last_design_read_hour != scan_hour.design_read_hour
                }
                (None, None) => false,
                _ => true,
            };
            last_designs = scan_hour.designs.clone();
            last_design_read_hour = scan_hour.design_read_hour;
            let (Some(identity), true) = (identity, scan_hour.lines_complete) else {
                continue;
            };
            let slot = identity.0;
            if carried && designs_changed {
                let demanded: BTreeSet<_> = open
                    .values()
                    .filter_map(|line| result.lines[line.index].runs.last().map(|run| run.variant))
                    .collect();
                for variant in demanded {
                    observe_design(&mut result, scan_hour, variant);
                }
            }
            if !scan_hour.lines_read {
                if carried {
                    let _phase = replay_frame::cpu::scope_phase("production-lines:carry");
                    mark_span(&mut result.line_hours, hour, through);
                    output_open &= scan_hour.output_complete;
                    organisations_open &= scan_hour.organisations_complete;
                    if organisations_open {
                        mark_span(&mut result.organisation_hours, hour, through);
                    }
                    let modifiers_carried = scan_hour.modifiers_complete;
                    carry_lines(
                        &mut result,
                        &open,
                        (hour, through),
                        modifiers_carried,
                        output_open,
                    );
                    if output_open {
                        mark_span(&mut result.output_hours, hour, through);
                    }
                }
                continue;
            }
            let decoded = decoded.ok_or_else(|| frame_not_kept(hour))?;
            let countries = decoded.complete::<tables::Countries>()?;
            let links = decoded.complete::<tables::CountryLinks>()?;
            let rows = decoded.complete::<tables::ProductionLines>()?;
            result.reads.push(hour);
            mark(&mut result.line_hours, hour);
            let previous_read = last_census;
            last_census = Some(hour);
            if broken_identity != Some(identity) {
                broken.clear();
            }
            last_read_identity = Some(identity);
            let mut still = BTreeMap::new();
            let ours = |country: u32, handle: u64| match link_slot(links, country) {
                Some(linked) => linked == slot,
                None => open.contains_key(&handle) || broken.contains_key(&handle),
            };
            result.unlinked_rows += rows
                .iter()
                .filter(|row| link_slot(links, row.country).is_none())
                .count();
            let mine: Vec<&ProductionLineRow> = rows
                .iter()
                .filter(|row| ours(row.country, row.handle))
                .collect();
            let naming_tags = Tags::new(
                countries,
                links,
                decoded.complete::<tables::CountryAppearance>().ok(),
            );
            let modifier_inputs = scan_hour
                .modifiers_read
                .then(|| {
                    decoded
                        .complete::<tables::CountryProductionModifiers>()
                        .ok()
                        .zip(decoded.complete::<tables::ProductionLineModifiers>().ok())
                })
                .flatten();
            let modifier_inputs = modifier_inputs.filter(|_| {
                match replay_derive::fixed_layout::validate_recording(
                    decoded,
                    replay_derive::fixed_layout::PRODUCTION_MODIFIERS,
                ) {
                    Ok(()) => true,
                    Err(reason) => {
                        *result.modifier_layout_refusals.entry(reason).or_default() += 1;
                        false
                    }
                }
            });
            let naming_organisations = decoded.complete::<tables::IndustrialOrganisations>().ok();
            let modifier_country = modifier_inputs
                .and_then(|(countries, _)| {
                    countries
                        .binary_search_by_key(&identity.1, |country| country.country)
                        .ok()
                        .map(|index| &countries[index])
                })
                .map(|country| {
                    if last_modifier_country.as_deref() != Some(country) {
                        last_modifier_country = Some(Arc::new(country.clone()));
                    }
                    Arc::clone(
                        last_modifier_country
                            .as_ref()
                            .expect("recorded modifier country"),
                    )
                });
            for row in &mine {
                let Some(kind) = LineKind::of(row.kind) else {
                    result.unknown_kinds += 1;
                    continue;
                };
                let (index, starts_segment) = if let Some(line) = open.remove(&row.handle) {
                    (line.index, false)
                } else if let Some(line) = broken.remove(&row.handle) {
                    (line.index, true)
                } else {
                    let appearance = appearances.entry(row.handle).or_default();
                    *appearance += 1;
                    result.lines.push(ScannedLine {
                        key: format!("{}#{appearance}", row.handle),
                        handle: row.handle,
                        kind: Some(kind),
                        runs: Vec::new(),
                        output: Vec::new(),
                        final_output: None,
                        ended_by: None,
                        before_recording: first_read,
                        between_reads: None,
                        variants: Vec::new(),
                    });
                    (result.lines.len() - 1, true)
                };
                latest.insert(row.handle, index);
                observe_design(&mut result, scan_hour, row.variant);
                let values = RawRun {
                    first_hour: hour,
                    last_hour: hour,
                    starts_segment,
                    variant: row.variant,
                    mio: row.mio,
                    active_factories: row.active_factories,
                    requested_factories: row.requested_factories,
                    max_factories: row.max_factories,
                    cost_raw: row.cost_raw,
                    speed_raw: row.speed_raw,
                    efficiency_raw: row.efficiency_raw,
                    priority: row.priority,
                    amount: row.amount,
                };
                let line = &mut result.lines[index];
                if let Some((_, lines)) = modifier_inputs
                    && let Some(country) = modifier_country
                        .as_ref()
                        .filter(|country| country.country == row.country)
                    && let Ok(position) = lines.binary_search_by_key(&row.handle, |line| line.line)
                {
                    let operands = &lines[position];
                    if (
                        operands.country,
                        operands.variant,
                        operands.organisation,
                        operands.kind,
                        operands.active_factories,
                    ) == (
                        row.country,
                        row.variant,
                        row.mio,
                        row.kind,
                        row.active_factories,
                    ) {
                        let catalogue = icons.and_then(|icons| icons.catalogue.as_deref());
                        let organisation = naming_organisations.and_then(|rows| {
                            rows.binary_search_by_key(&row.mio, |organisation| {
                                organisation.organisation
                            })
                            .ok()
                            .map(|index| &rows[index])
                        });
                        let organisation_name =
                            organisation
                                .zip(lexical)
                                .and_then(|(organisation, lexical)| {
                                    let raw =
                                        raw_organisation(organisation, hour, &naming_tags, lexical);
                                    organisation_name(
                                        &raw,
                                        catalogue.map(|catalogue| catalogue as &dyn Texts),
                                    )
                                });
                        result.modifiers.entry(line.key.clone()).or_default().push(
                            breakdown::RawModifierRun {
                                first_hour: hour,
                                last_hour: hour,
                                country: Arc::clone(country),
                                line: operands.clone(),
                                run: values,
                                organisation: organisation_name,
                                localisation: catalogue
                                    .map(|catalogue| Arc::clone(&catalogue.localisation)),
                                recorded_language: Arc::clone(&recorded_modifier_language),
                            },
                        );
                    }
                }
                match line.runs.last_mut() {
                    Some(run)
                        if !starts_segment
                            && run.last_hour.checked_add(1) == Some(hour)
                            && same_values(run, &values) =>
                    {
                        run.last_hour = hour
                    }
                    _ => {
                        line.runs.push(values);
                    }
                }
                still.insert(row.handle, Open { index });
            }
            for line in open.values().chain(broken.values()) {
                result.lines[line.index].ended_by = Some(hour);
            }
            broken.clear();
            open = still;
            carried = true;
            first_read = false;
            output_open = false;
            let mut produced_between: BTreeSet<u64> = BTreeSet::new();
            if let Ok(output) = decoded.complete::<tables::ProductionLineOutput>() {
                output_open = true;
                mark(&mut result.output_hours, hour);
                let mut by_line: BTreeMap<u64, Vec<(&ProductionLineOutputRow, bool)>> =
                    BTreeMap::new();
                for row in output {
                    let linked = link_slot(links, row.country);
                    if linked.is_none() {
                        result.unlinked_rows += 1;
                    }
                    let mine = match linked {
                        Some(linked) => linked == slot,
                        None => latest.contains_key(&row.line),
                    };
                    if !mine {
                        continue;
                    }
                    by_line
                        .entry(row.line)
                        .or_default()
                        .push((row, linked.is_some()));
                    produced_between.insert(row.variant);
                }
                for (handle, rows) in by_line {
                    let index = match latest.get(&handle) {
                        Some(&index) => index,
                        None if rows.iter().all(|(row, _)| row.ended) => {
                            let whole_life =
                                rows.iter().all(|(row, _)| row.observed_since_creation);
                            let since = rows
                                .iter()
                                .map(|(row, _)| row.since_hour)
                                .min()
                                .unwrap_or(hour);
                            let (first_hour, before) = match previous_read {
                                Some(read) if whole_life => (read + 1, false),
                                _ => (since.min(hour), true),
                            };
                            let appearance = appearances.entry(handle).or_default();
                            *appearance += 1;
                            result.lines.push(ScannedLine {
                                key: format!("{handle}#{appearance}"),
                                handle,
                                kind: None,
                                runs: Vec::new(),
                                output: Vec::new(),
                                final_output: None,
                                ended_by: Some(hour),
                                before_recording: before,
                                between_reads: Some(crate::products::country_groups::HourSpan {
                                    first_hour,
                                    last_hour: hour,
                                }),
                                variants: rows.iter().map(|(row, _)| row.variant).collect(),
                            });
                            let index = result.lines.len() - 1;
                            latest.insert(handle, index);
                            produced_between.extend(rows.iter().map(|(row, _)| row.variant));
                            index
                        }
                        None => {
                            result.unlisted_output += 1;
                            continue;
                        }
                    };
                    let run = output_of(&rows, hour);
                    let line = &mut result.lines[index];
                    if rows.iter().any(|(row, _)| row.ended) || !open.contains_key(&handle) {
                        line.final_output = Some(run);
                    } else {
                        line.output.push(run);
                    }
                }
            }
            organisations_open = false;
            if let (Ok(organisations), Some(lexical)) = (
                decoded.complete::<tables::IndustrialOrganisations>(),
                lexical,
            ) {
                organisations_open = true;
                mark(&mut result.organisation_hours, hour);
                let named: BTreeSet<u64> = mine
                    .iter()
                    .flat_map(|row| {
                        let author =
                            decoded
                                .complete::<tables::VariantStats>()
                                .ok()
                                .and_then(|stats| {
                                    stats
                                        .binary_search_by_key(&row.variant, |s| s.variant)
                                        .ok()
                                        .map(|index| stats[index].organisation)
                                });
                        std::iter::once(row.mio).chain(author)
                    })
                    .filter(|mio| *mio != 0)
                    .collect();
                for row in organisations
                    .iter()
                    .filter(|row| named.contains(&row.organisation))
                {
                    version(
                        result.organisations.entry(row.organisation).or_default(),
                        {
                            let mut raw = raw_organisation(row, hour, &naming_tags, lexical);
                            raw.state = decoded
                                .complete::<tables::IndustrialOrganisationDetails>()
                                .ok()
                                .and_then(|details| {
                                    details
                                        .binary_search_by_key(&row.organisation, |details| {
                                            details.organisation
                                        })
                                        .ok()
                                        .map(|index| &details[index])
                                        .filter(|details| details.country == row.country)
                                })
                                .map(|details| {
                                    let mut state = organisation_state(
                                        details,
                                        decoded.tables
                                            [TableId::IndustrialOrganisationDetails.index()]
                                        .sampled_at(),
                                        lexical,
                                    );
                                    state.policy_name = decoded
                                        .complete::<tables::IndustrialOrganisationPolicies>()
                                        .ok()
                                        .and_then(|rows| {
                                            rows.binary_search_by_key(
                                                &details.country,
                                                |policies| policies.country,
                                            )
                                            .ok()
                                            .map(|index| &rows[index])
                                        })
                                        .and_then(|policies| {
                                            policies
                                                .policies
                                                .iter()
                                                .find(|policy| policy.token == details.state.policy)
                                        })
                                        .map(|policy| text(&policy.name));
                                    state
                                });
                            raw
                        },
                        |a, b| {
                            RawOrganisation {
                                first_hour: 0,
                                ..a.clone()
                            } == RawOrganisation {
                                first_hour: 0,
                                ..b.clone()
                            }
                        },
                    );
                }
            }
            let produced: BTreeSet<u64> = mine
                .iter()
                .map(|row| row.variant)
                .chain(produced_between)
                .collect();
            if let (Some(content), Some(lexical), false) = (icons, lexical, produced.is_empty()) {
                let state = IconHour::read(decoded, content);
                for id in produced {
                    let icon = match (&state, definitions.get(&id)) {
                        (Err(error), _) => IconChoice::unknown(error.to_string()),
                        (Ok(_), None) => {
                            pending.insert(id);
                            IconChoice::unknown(
                                "the variant's definition is not recorded yet".to_owned(),
                            )
                        }
                        (Ok(_), Some(_)) if result.variants.current(id).is_none() => {
                            pending.insert(id);
                            IconChoice::unknown(
                                "the variant's definition is not recorded at this time".to_owned(),
                            )
                        }
                        (Ok(state), Some(definition)) => icon_choice(
                            content,
                            state,
                            definition,
                            &equipment::variant_token(definition, lexical),
                        ),
                    };
                    version(result.icons.entry(id).or_default(), (hour, icon), |a, b| {
                        a.1 == b.1
                    });
                }
            }
        }
        if let Some(from) = extracted.lost_from {
            lost(&mut result, from, block.last_hour);
        }
    }
    Ok(result)
}

#[derive(Default)]
pub(super) struct Selected {
    literal: Option<ScannedLines>,
    native: BTreeMap<(u32, u32), ScannedLines>,
}
pub(super) fn selected(
    source: &Source,
    holder: Option<&prepared::NativeCountry>,
) -> Option<ScannedLines> {
    let state = source.state::<Selected>();
    let state = state.lock().unwrap();
    holder
        .and_then(|holder| state.native.get(&(holder.slot, holder.id)))
        .or(state.literal.as_ref())
        .cloned()
}
pub(super) fn with<T>(source: &Source, scanned: ScannedLines, render: impl FnOnce() -> T) -> T {
    with_native(source, scanned, BTreeMap::new(), render)
}
pub(super) fn with_native<T>(
    source: &Source,
    literal: ScannedLines,
    native: BTreeMap<(u32, u32), ScannedLines>,
    render: impl FnOnce() -> T,
) -> T {
    let state = source.state::<Selected>();
    let before = std::mem::replace(
        &mut *state.lock().unwrap(),
        Selected {
            literal: Some(literal),
            native,
        },
    );
    struct Reset<'a>(&'a std::sync::Mutex<Selected>, Option<Selected>);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            *self.0.lock().unwrap() = self.1.take().unwrap();
        }
    }
    let _reset = Reset(&state, Some(before));
    render()
}

/// Whether a reference scan answers this source's replies.
pub(super) fn installed(source: &Source) -> bool {
    source.state::<Selected>().lock().unwrap().literal.is_some()
}

type ScannedView = (
    Option<Arc<Catalogue>>,
    Arc<ScannedLines>,
    Vec<super::super::country_groups::HourSpan>,
);

/// The installed reference scan as the replies read a whole scan before the index: the
/// content's warnings, the reference's refused layouts, and no prefix still being prepared.
fn scanned_view(
    source: &Source,
    _params: &CountryParams,
    holder: Option<prepared::NativeCountry>,
    context: &LoadContext,
    _cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<ScannedView> {
    let (catalogue, _) = scan_content(source, context, warnings);
    let scanned = selected(source, holder.as_ref())
        .ok_or_else(|| crate::invalid("No reference production scan"))?;
    warnings.extend(
        scanned
            .modifier_layout_refusals
            .iter()
            .map(|(reason, count)| format!("{reason} ({count} supplying reads)")),
    );
    Ok((catalogue, Arc::new(scanned), Vec::new()))
}

fn scanned_lines(
    source: &Source,
    params: &CountryParams,
    holder: Option<prepared::NativeCountry>,
    context: &LoadContext,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<(Option<Arc<Catalogue>>, Arc<ScannedLines>)> {
    let (catalogue, scanned, _) =
        scanned_view(source, params, holder, context, cancelled, warnings)?;
    Ok((catalogue, scanned))
}

/// `production-lines` as it answered from a whole scan.
pub(super) fn lines(
    source: &Source,
    params: &ProductionLinesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<ProductionLinesReply> {
    let mut warnings = Vec::new();
    let (catalogue, scanned, approximate) = scanned_view(
        source,
        &params.country(),
        None,
        context,
        cancelled,
        &mut warnings,
    )?;
    if params.exact == Some(true) && !approximate.is_empty() {
        return Err(crate::invalid("Production lines are still being prepared"));
    }
    if !approximate.is_empty() {
        warnings.push(format!(
            "{}'s production history after the published prefix is still being prepared.",
            params.tag
        ));
    }
    let texts = catalogue
        .as_ref()
        .map(|catalogue| catalogue.as_ref() as &dyn Texts);
    // The scan keeps each variant's icon as a choice with its artwork questions open
    // (kept with prepared scans too); this reply answers them, and draws the icon, as the
    // request's selection shows it.
    let pictured = catalogue.as_ref().map(|_| {
        context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
    });
    let mut images: BTreeMap<String, Option<String>> = BTreeMap::new();
    let (variants, unnamed, unknown_icons) = {
        let art = source.state::<catalogue::ByCatalogue<VariantArt>>();
        let mut art = art.lock().unwrap_or_else(|error| error.into_inner());
        let mut drawn = match (&pictured, &catalogue) {
            (Some(Ok(admitted)), Some(recorded)) => Ok(catalogue::Redrawn::bind(
                &mut art.of(admitted).0,
                admitted,
                recorded,
            )),
            (Some(Err(error)), _) => Err(format!("Equipment pictures are unavailable: {error}")),
            _ => Err("Equipment icons need the game folders.".to_owned()),
        };
        variant_entries(
            &scanned,
            texts,
            catalogue.is_some(),
            &mut |choice: &IconChoice| {
                if cancelled.load(Ordering::Acquire) {
                    return Err("cancelled".to_owned());
                }
                let drawn = drawn.as_mut().map_err(|why| why.clone())?;
                draw_variant_icon(drawn, &mut images, choice)
            },
        )
    };
    if cancelled.load(Ordering::Acquire) {
        return Err(cancelled_error());
    }
    warnings.extend(scanned.variants.warnings());
    let pictured = pictured.and_then(Result::ok);
    // Organisations: each one's choice of sprite and its picture drawn as one operation from
    // the selection, kept per catalogue, its picture under the picture's key.
    let mut organisations = Vec::new();
    let state = source.state::<Cache>();
    for (id, versions) in &scanned.organisations {
        let mut previous: Option<OrganisationName> = None;
        for raw in versions {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            let icon = pictured.as_ref().and_then(|catalogue| {
                let (key, image) =
                    kept_organisation_picture(&state, catalogue, raw, &mut warnings)?;
                images.entry(key.clone()).or_insert(image);
                Some(key)
            });
            let entry = OrganisationName {
                    id: id.to_string(),
                    first_hour: raw.first_hour,
                    token: raw.token.clone(),
                    name: organisation_name(raw, texts),
                    name_unknown: raw.owner_unknown.then(|| "The organisation's owner is unresolved at this read; its country-specific name is unknown.".to_owned()),
                    icon_unknown: (raw.owner_unknown && icon.is_none()).then(|| "The organisation's owner is unresolved at this read; its country-specific icon is unknown.".to_owned()),
                    icon,
                };
            if previous.as_ref().is_none_or(|old| {
                OrganisationName {
                    first_hour: entry.first_hour,
                    ..old.clone()
                } != entry
            }) {
                previous = Some(entry.clone());
                organisations.push(entry);
            }
        }
    }
    let icons_out: Vec<EquipmentIcon> = images
        .into_iter()
        .map(|(key, image)| EquipmentIcon { key, image })
        .collect();
    if !icons_out.is_empty() {
        warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
    }
    for reason in unknown_icons {
        warnings.push(format!("Some equipment icons are unknown: {reason}"));
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} produced variants have no recorded definition in this branch; they are shown by id."));
    }
    if scanned.unknown_kinds > 0 {
        warnings.push(format!(
            "{} recorded line reads are of an unknown native class; they are left out.",
            scanned.unknown_kinds
        ));
    }
    if scanned.unlisted_output > 0 {
        warnings.push(format!(
                "{} output reads name a line no census listed without its final count; they are left out.",
                scanned.unlisted_output
            ));
    }
    if scanned.unread_output_reads > 0 || scanned.unreadable_stretches > 0 {
        let stretches = match scanned.unreadable_stretches {
            0 => String::new(),
            count => format!(", nor {count} stretches of the recording"),
        };
        warnings.push(format!(
                "{} daily production output reads of {} could not be read{stretches}, so lines made and ended then may be missing and their final counts are not recorded; the history marks those hours.",
                scanned.unread_output_reads, params.tag
            ));
    }
    if scanned.unlinked_rows > 0 {
        warnings.push(format!(
                "{} production rows name a producer the recording does not link to a country; those naming a line of {} are kept with counts that are not exact, the others are left out.",
                scanned.unlinked_rows, params.tag
            ));
    }
    // A first view without reads says nothing of the hours before it.
    if scanned.reads.is_empty() && approximate.is_empty() {
        warnings.push(format!(
            "{}: no recorded production lines in this branch",
            params.tag
        ));
    }
    if let Some(catalogue) = &catalogue {
        warnings.extend(catalogue.refusal_lines());
    }
    bounded(
        ProductionLinesReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: EQUIPMENT_DECIMALS,
            lines: scanned.lines.iter().map(line_history).collect(),
            variants,
            organisations,
            icons: icons_out,
            reads: scanned.reads.clone(),
            output_hours: scanned.output_hours.clone(),
            line_hours: scanned.line_hours.clone(),
            organisation_hours: scanned.organisation_hours.clone(),
            unread_output: scanned.unread_output.clone(),
            approximate,
            detail_hours: DETAIL_HOURS,
            warnings,
        },
        "This country's production history",
        "its lines cannot be shown.",
    )
}

/// `production-line-detail` as it answered from a whole scan.
pub(super) fn detail(
    source: &Source,
    params: &ProductionDetailParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<ProductionLineDetailReply> {
    let country = CountryParams {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
    };
    let mut warnings = Vec::new();
    let (_, scanned) = scanned_lines(source, &country, None, context, cancelled, &mut warnings)?;
    let (first_hour, last_hour) = detail_window(params.window).expect("validated window");
    let lines = detail_lines(&scanned, first_hour, last_hour);
    warnings.sort();
    warnings.dedup();
    bounded(
        ProductionLineDetailReply {
            warnings,
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: EQUIPMENT_DECIMALS,
            window: params.window,
            first_hour,
            last_hour,
            lines,
        },
        "This month of production lines",
        "its values cannot be shown.",
    )
}

/// `production-line-breakdown` as it answered from a whole scan.
pub(super) fn breakdown(
    source: &Source,
    params: &ProductionBreakdownParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<ProductionLineBreakdownReply> {
    use super::breakdown::{FACTORS, ProductionModifierChoice};
    let country = CountryParams {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
    };
    let mut warnings = Vec::new();
    let (_, original) = scanned_lines(source, &country, None, context, cancelled, &mut warnings)?;
    let (scanned, selected_key) = if let Some(hour) = params.origin_hour {
        let holder = prepared::country_at(
            &mut prepared::open(source, context, country.branch(), None, cancelled)?,
            &params.tag,
            hour,
        )?;
        let (_, selected) = scanned_lines(
            source,
            &country,
            Some(holder),
            context,
            cancelled,
            &mut warnings,
        )?;
        // Literal-tag appearance numbers can include a previous holder's line.
        // Match the original line's recorded runs as well as its handle.
        let original_line = original.lines.iter().find(|line| line.key == params.key);
        let key = if let Some(line) = original_line {
            let mut matching = selected.lines.iter().filter(|candidate| {
                candidate.handle == line.handle
                    && candidate.runs.iter().any(|run| {
                        line.runs.iter().any(|old| {
                            old.first_hour <= run.last_hour && run.first_hour <= old.last_hour
                        })
                    })
            });
            let first = matching.next();
            if matching.next().is_some() {
                return Err(crate::invalid(
                    "Conflicting appearances for the original production line",
                ));
            }
            first.map(|line| line.key.clone())
        } else {
            None
        };
        (selected, key)
    } else {
        (original, Some(params.key.clone()))
    };
    let raw = selected_key
        .as_ref()
        .and_then(|key| scanned.modifiers.get(key))
        .map(Vec::as_slice)
        .unwrap_or_default();
    let reads = raw.iter().map(|run| run.first_hour).collect();
    let groups: Vec<ProductionModifierChoice> = if raw.is_empty() {
        Vec::new()
    } else {
        FACTORS
            .iter()
            .map(|(id, label)| ProductionModifierChoice {
                id: (*id).into(),
                label: (*label).into(),
            })
            .collect()
    };
    if !raw.is_empty() && !groups.iter().any(|group| group.id == params.group) {
        return Err(crate::invalid("Unknown production factor"));
    }
    let mut runs: Vec<ProductionModifierRun> = Vec::new();
    for raw in raw {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancelled_error());
        }
        let value = raw.project(Some(&params.group));
        match runs.last_mut() {
            Some(last)
                if last.last_hour.checked_add(1) == Some(value.first_hour)
                    && last.groups == value.groups
                    && last.warnings == value.warnings =>
            {
                last.last_hour = value.last_hour
            }
            _ => runs.push(value),
        }
    }
    if runs.is_empty() {
        warnings.push("Production modifier inputs were not recorded for this line".into());
    }
    bounded(
        ProductionLineBreakdownReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            key: params.key.clone(),
            groups,
            reads,
            runs,
            warnings,
        },
        "This production line's modifier history",
        "its full breakdown cannot be shown.",
    )
}
