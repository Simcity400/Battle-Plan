use super::*;
use engine_intelligence::network_auxiliary::{Entry, NATIONAL, kind};
use content_reader_save::intel::{IntelNetworkEntry, network_auxiliary as saved_aux};
use replay_data::rows::IntelNetworkAuxiliaryRow;

type Key = (u8, u32, u64);
type Expected = (Entry, Option<Vec<u8>>);
fn entry(kind: u8, scope: u32, ordinal: usize) -> Entry {
    Entry {
        kind,
        subnetwork: scope,
        ordinal: ordinal as u64,
        ..Entry::default()
    }
}
fn raw(value: &Option<content_reader_save::Decimal>, name: &str) -> io::Result<i64> {
    value
        .as_ref()
        .map(|v| v.raw)
        .ok_or_else(|| invalid(format!("saved network auxiliary {name} absent")))
}
fn records(
    out: &mut Vec<Expected>,
    records: &[saved_aux::Record],
    scope: u32,
    tokens: &TokenNames<'_>,
) -> io::Result<()> {
    for (ordinal, saved) in records.iter().enumerate() {
        let mut row = entry(kind::RECORD, scope, ordinal);
        let fields = if scope == NATIONAL {
            &[0x4bfe, 0x2e3b, 0x3d2b, 0x4b56, 0x4b58][..]
        } else {
            &[0x4bfe, 0x2e3b][..]
        };
        if saved.fields.len() != fields.len() {
            return Err(invalid(
                "saved network record operand membership differs from its native writer",
            ));
        }
        let mut values = Vec::new();
        for token in fields {
            let key = tokens.get(token).ok_or_else(|| {
                invalid(format!(
                    "network record operand token {token} has no exact recorded spelling"
                ))
            })?;
            let value = saved.fields.get(*key).ok_or_else(|| {
                invalid(format!(
                    "saved network record lacks {}",
                    String::from_utf8_lossy(key)
                ))
            })?;
            values.push(value.raw);
        }
        let count = |raw: i64| -> io::Result<i32> {
            if raw % 100_000 != 0 {
                return Err(invalid("saved network record count has fractional units"));
            }
            i32::try_from(raw / 100_000)
                .map_err(|_| invalid("saved network record count exceeds i32"))
        };
        row.count_a = count(values[0])?;
        row.count_b = count(values[1])?;
        if scope == NATIONAL {
            row.first = values[2];
            row.second = values[3];
            row.third = values[4];
        }
        out.push((row, Some(saved.country.clone())));
    }
    Ok(())
}
fn expected(saved: &IntelNetworkEntry, tokens: &TokenNames<'_>) -> io::Result<Vec<Expected>> {
    let aux = &saved.auxiliary;
    let mut out = Vec::new();
    let rows = saved.state_ids.len();
    let vertices = if rows == 0 {
        aux.vertices.unwrap_or(0)
    } else {
        aux.vertices
            .ok_or_else(|| invalid("saved network auxiliary vertices absent"))?
    };
    if vertices as usize != rows {
        return Err(invalid(
            "saved network vertices and acquired state count differ",
        ));
    }
    let mut header = entry(kind::NETWORK, NATIONAL, 0);
    header.count_a = saved.subnetworks.len() as i32;
    header.count_b = if rows == 0 {
        0
    } else {
        aux.previous_max_depth
            .ok_or_else(|| invalid("saved network auxiliary prev_max_depth absent"))?
    };
    header.count_c = vertices as i32;
    out.push((header, None));
    if rows != 0 {
        if aux.depth.len() != rows {
            return Err(invalid(
                "saved network auxiliary depth length differs from vertices",
            ));
        }
        let mut header = entry(kind::NETWORK, NATIONAL, 1);
        header.count_a = aux
            .subnetwork_count
            .ok_or_else(|| invalid("saved network auxiliary sub_network_count absent"))?;
        out.push((header, None));
        for (ordinal, &state) in saved.state_ids.iter().enumerate() {
            let mut row = entry(kind::VERTEX, NATIONAL, ordinal);
            row.count_a = state;
            row.count_b = aux.depth[ordinal];
            out.push((row, None));
        }
        for (ordinal, &(from, to)) in aux.edges.iter().enumerate() {
            let mut row = entry(kind::EDGE, NATIONAL, ordinal);
            row.first = from as i64;
            row.second = to as i64;
            out.push((row, None));
        }
        for (ordinal, &state) in aux.previous_sources.iter().enumerate() {
            let mut row = entry(kind::PREVIOUS_SOURCE, NATIONAL, ordinal);
            row.count_a = state;
            out.push((row, None));
        }
        for (ordinal, &quiet) in saved.local_quiet.iter().enumerate() {
            let mut row = entry(kind::QUIET, NATIONAL, ordinal);
            row.count_a = i32::from(quiet);
            out.push((row, None));
        }
    }
    for (ordinal, &(handle, state)) in aux.operatives.iter().enumerate() {
        let mut row = entry(kind::OPERATIVE, NATIONAL, ordinal);
        row.first = handle as i64;
        row.count_a = state;
        out.push((row, None));
    }
    records(&mut out, &aux.records, NATIONAL, tokens)?;
    if aux.subnetworks.len() != saved.subnetworks.len() {
        return Err(invalid(
            "saved subnetwork auxiliary membership differs from subnetwork census",
        ));
    }
    for (ordinal, (aux, saved_sub)) in aux.subnetworks.iter().zip(&saved.subnetworks).enumerate() {
        let scope = ordinal as u32;
        out.push((entry(kind::SUBNETWORK, scope, 0), None));
        for (kind, value, name) in [
            (kind::COVERAGE, aux.coverage, "coverage_stats"),
            (kind::COVERABLE, aux.coverable, "total_coverable"),
        ] {
            let [a, b, c] =
                value.ok_or_else(|| invalid(format!("saved subnetwork {name} absent")))?;
            let mut row = entry(kind, scope, 0);
            row.count_a = a;
            row.count_b = b;
            row.count_c = c;
            out.push((row, None));
        }
        let mut row = entry(kind::AGGREGATES, scope, 0);
        row.first = raw(&aux.aggregates[0], "strength_sum")?;
        row.second = raw(&aux.aggregates[1], "strength_sum_over_cores")?;
        row.third = raw(&aux.aggregates[2], "national_coverage")?;
        out.push((row, None));
        let mut row = entry(kind::MODIFIERS, scope, 0);
        row.first = raw(&aux.detection[0], "own_operative_detection_chance")?;
        row.second = raw(&aux.detection[1], "own_operative_detection_chance_factor")?;
        out.push((row, None));
        records(&mut out, &aux.records, scope, tokens)?;
        for (ordinal, &handle) in aux.operatives.iter().enumerate() {
            let mut row = entry(kind::OPERATIVE, scope, ordinal);
            row.first = handle as i64;
            out.push((row, None));
        }
        for (ordinal, (state, strength)) in aux.states.iter().enumerate() {
            let mut row = entry(kind::STATE, scope, ordinal);
            row.count_a = *state;
            row.first = strength.raw;
            out.push((row, None));
        }
        for (ordinal, &state) in aux.core_states.iter().enumerate() {
            let mut row = entry(kind::CORE_STATE, scope, ordinal);
            row.count_a = state;
            out.push((row, None));
        }
        if !saved.selectors.contains(&(scope as i32)) {
            let mut row = entry(kind::UNSELECTED_STRENGTH, scope, 0);
            row.first = saved_sub.strength.raw;
            row.second = saved_sub.gain_factor.raw;
            row.third = saved_sub.strength_target.raw;
            out.push((row, None));
            let mut row = entry(kind::UNSELECTED_TARGET, scope, 0);
            row.first = saved_sub.target_from_operatives.raw;
            row.second = saved_sub.target_from_counterintelligence.raw;
            out.push((row, None));
            let mut row = entry(kind::UNSELECTED_QUIET, scope, 0);
            row.count_a = i32::from(saved_sub.quiet);
            out.push((row, None));
        }
    }
    Ok(out)
}
fn value(row: &IntelNetworkAuxiliaryRow) -> Entry {
    Entry {
        kind: row.kind,
        subnetwork: row.subnetwork,
        ordinal: row.ordinal,
        country: row.country,
        count_a: row.count_a,
        count_b: row.count_b,
        count_c: row.count_c,
        first: row.first,
        second: row.second,
        third: row.third,
    }
}
pub(super) fn compare(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual: BTreeMap<(u32, Vec<u8>, Key), Expected> = BTreeMap::new();
    for row in complete::<tables::IntelNetworkAuxiliaries>(frame)? {
        let owner = countries.get(&row.owner).ok_or_else(|| {
            invalid("network auxiliary owner lacks authenticated country identity")
        })?;
        let target = required_country_tag(row.target, countries)?.to_vec();
        let mut entry = value(row);
        let country = if row.kind == kind::RECORD {
            entry.country = 0;
            Some(required_country_tag(row.country, countries)?.to_vec())
        } else {
            None
        };
        if matches!(row.kind, kind::QUIET | kind::UNSELECTED_QUIET) {
            entry.count_a = i32::from(entry.count_a != 0);
        }
        if actual
            .insert(
                (owner.canonical_slot, target, entry.key()),
                (entry, country),
            )
            .is_some()
        {
            return Err(invalid("duplicate network auxiliary logical occurrence"));
        }
    }
    let mut matched = 0;
    for operative in save
        .strategic_operatives
        .iter()
        .flat_map(|objects| objects.values())
    {
        for network in &operative.networks {
            for expected in expected(network, tokens)? {
                let key = (operative.index, network.target.clone(), expected.0.key());
                let label = format!(
                    "network_auxiliary.{}.{}.{:?}",
                    operative.index,
                    text(&network.target),
                    expected.0.key()
                );
                if let Some(actual) = actual.remove(&key) {
                    report.scalar_checks += 10;
                    if actual != expected {
                        report.difference(label, actual, expected);
                    }
                    matched += 1;
                } else {
                    report.difference(label, "absent", expected);
                }
            }
        }
    }
    for (key, value) in actual {
        report.difference(
            format!("network_auxiliary.{key:?}"),
            value,
            "absent saved occurrence",
        );
    }
    report.compared("IntelNetworkAuxiliaries", matched);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableId;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    #[test]
    fn network_auxiliary_oracle_compares_ordered_lists_and_each_independent_word() {
        let network = IntelNetworkEntry {
            target: b"TGT".to_vec(),
            state_ids: vec![17, 23],
            selectors: vec![-1, -1],
            local_quiet: vec![true],
            auxiliary: saved_aux::Network {
                vertices: Some(2),
                depth: vec![-7, 9],
                subnetwork_count: Some(0),
                previous_max_depth: Some(9),
                previous_sources: vec![17, 17],
                edges: vec![(0, 1), (1, 0)],
                ..Default::default()
            },
            ..Default::default()
        };
        let tokens = TokenNames::new();
        let rows: Vec<_> = expected(&network, &tokens)
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, (row, _))| IntelNetworkAuxiliaryRow {
                owner: 101,
                target: 102,
                id: index as u64 + 1,
                kind: row.kind,
                subnetwork: row.subnetwork,
                ordinal: row.ordinal,
                country: row.country,
                count_a: row.count_a,
                count_b: row.count_b,
                count_c: row.count_c,
                first: row.first,
                second: row.second,
                third: row.third,
            })
            .collect();
        let owner = CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"OWN".to_vec(),
            ..Default::default()
        };
        let target = CountryRow {
            id: 102,
            canonical_slot: 8,
            tag: b"TGT".to_vec(),
            ..Default::default()
        };
        let countries = BTreeMap::from([(101, &owner), (102, &target)]);
        let pack = saved::Pack {
            strategic_operatives: Some(BTreeMap::from([(
                7,
                saved::StrategicOperative {
                    index: 7,
                    networks: vec![network],
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };
        let check = |rows| compared(rows, &pack, &countries, &tokens);
        assert!(check(rows.clone()).differences.is_empty());
        assert!(!check(Vec::new()).differences.is_empty());
        for index in 0..rows.len() {
            let mut changed = rows.clone();
            changed[index].first = changed[index].first.wrapping_add(1);
            assert!(!check(changed).differences.is_empty());
        }

        let save = br#"countries={OWN={} YUG={} ROM={}}
            strategic_operatives={ strategic_operative={ 7={ intel_network={ target="YUG"
                intel_network={ vertices=1 state_id={ 107 } gain={ 0.4 }
                    gain_breakdown={ { operatives=0.4 adjacencies=0 } } strength={ 38.4 }
                    depth={ 0 } sub_network_id={ 0 } sub_network_count=1
                    sub_network_is_quiet={ no } prev_max_depth=0 prev_gain_sources={ 107 } }
                sub_intel_networks={ {
                    modifiers={ intel_network_gain_factor=1 own_operative_detection_chance=0 own_operative_detection_chance_factor=1 }
                    strength=38.4 strength_sum=38.4 strength_sum_over_cores=38.4 national_coverage=0
                    sub_network_strength_target=57.5 sub_network_strength_target_from_operatives=15
                    sub_network_strength_target_from_counterintelligence=-47.5 sub_network_is_quiet=no
                    coverage_stats={ core_states=1 controlled_states=1 owned_worth=1 }
                    total_coverable={ core_states=1 controlled_states=1 owned_worth=1 }
                    coverage_per_occupied={
                      { "ROM"
 {
                          owned_worth=0
                          states=1
                        }
}
                     }
                    states={ { 107 38.4 } } core_states={ 107 } } }
                max_coverage_by_occupied_tag={
                  { "ROM"
 {
                      owned_worth=54
                      states=2
                      intel_network_gain_factor=0
                      enemy_operative_detection_chance=0
                      enemy_operative_detection_chance_factor=0
                    }
}
                 }
            } } } }"#;
        let pack = saved::parse(&content_reader_parser::parser::parse(save).unwrap().items).unwrap();
        let network = &pack.strategic_operatives.as_ref().unwrap()[&7].networks[0];
        let tokens = TokenNames::from([
            (0x4bfe, b"owned_worth".as_slice()),
            (0x2e3b, b"states"),
            (0x3d2b, b"intel_network_gain_factor"),
            (0x4b56, b"enemy_operative_detection_chance"),
            (0x4b58, b"enemy_operative_detection_chance_factor"),
        ]);
        let occupied = CountryRow {
            id: 103,
            canonical_slot: 9,
            tag: b"ROM".to_vec(),
            ..Default::default()
        };
        let target = CountryRow {
            tag: b"YUG".to_vec(),
            ..target
        };
        let countries = BTreeMap::from([(101, &owner), (102, &target), (103, &occupied)]);
        let mut rows: Vec<_> = expected(network, &tokens)
            .unwrap()
            .into_iter()
            .filter(|(row, _)| row.kind != kind::RECORD)
            .map(|(row, _)| IntelNetworkAuxiliaryRow {
                owner: 101,
                target: 102,
                kind: row.kind,
                subnetwork: row.subnetwork,
                ordinal: row.ordinal,
                country: row.country,
                count_a: row.count_a,
                count_b: row.count_b,
                count_c: row.count_c,
                first: row.first,
                second: row.second,
                third: row.third,
                ..Default::default()
            })
            .collect();
        for (subnetwork, count_a, count_b) in [(NATIONAL, 54, 2), (0, 0, 1)] {
            rows.push(IntelNetworkAuxiliaryRow {
                owner: 101,
                target: 102,
                kind: kind::RECORD,
                subnetwork,
                country: 103,
                count_a,
                count_b,
                ..Default::default()
            });
        }
        for (index, row) in rows.iter_mut().enumerate() {
            row.id = index as u64 + 1;
        }
        let report = compared(rows, &pack, &countries, &tokens);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
    }

    fn compared(
        rows: Vec<IntelNetworkAuxiliaryRow>,
        pack: &saved::Pack,
        countries: &BTreeMap<u32, &CountryRow>,
        tokens: &TokenNames<'_>,
    ) -> Report {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::IntelNetworkAuxiliaries.index()] =
            TableView::Complete(TableRows::IntelNetworkAuxiliaries(rows));
        let mut report = Report::default();
        compare(&frame, pack, countries, tokens, &mut report).unwrap();
        report
    }
}
