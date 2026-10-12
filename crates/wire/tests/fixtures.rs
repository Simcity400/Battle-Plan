use wire::{fixtures, model::*, reader, shapes, writer};

#[test]
fn every_fixture_round_trips_or_refuses_by_name() {
    let registry = shapes::registry();
    let corpus = fixtures::fixtures().expect("Rust-generated fixture corpus");
    for fixture in &corpus {
        let context = ReadContext {
            expected_bounds: fixture.expected_bounds,
            require_bounds_match: fixture.expected_bounds.is_some(),
        };
        let decoded = reader::read(&fixture.bytes, &registry, context);
        if let Some(error) = fixture.expected_error {
            assert_eq!(
                decoded.expect_err(&fixture.name).name,
                error,
                "{}",
                fixture.name
            );
        } else {
            let decoded = decoded.unwrap_or_else(|e| panic!("{}: {e}", fixture.name));
            assert_eq!(
                writer::write(&decoded.to_owned(), &registry, context).expect("writer"),
                fixture.bytes,
                "{}",
                fixture.name
            );
        }
    }
    let successful: std::collections::BTreeSet<_> = corpus
        .iter()
        .filter(|f| f.expected_error.is_none())
        .map(|f| f.shape_id)
        .collect();
    for shape in &registry {
        assert!(
            successful.contains(&shape.id),
            "missing shape fixture: {}",
            shape.name
        );
    }
}

#[test]
fn complete_byte_walk_and_century_arithmetic() {
    let registry = shapes::registry();
    let corpus = fixtures::valid_fixtures().expect("successful fixtures");
    let fixture = corpus
        .iter()
        .find(|f| f.name == "c12-1-totals-diagnostics")
        .expect("totals");
    let reply = reader::read(
        &fixture.bytes,
        &registry,
        ReadContext {
            expected_bounds: fixture.expected_bounds,
            require_bounds_match: true,
        },
    )
    .expect("read");
    assert_eq!(fixture.bytes.len(), 2384);
    assert_eq!(
        reply
            .sections
            .iter()
            .map(|s| (s.kind, s.count, s.offset, s.length))
            .collect::<Vec<_>>(),
        vec![
            (1, 3, 248, 192),
            (2, 3, 440, 12),
            (3, 3, 456, 72),
            (9, 1, 528, 24),
            (5, 1, 552, 1832)
        ]
    );
    let diagnostic = reply.diagnostics.expect("embedded diagnostics");
    assert_eq!(diagnostic.descriptors.len(), 14);
    assert_eq!(diagnostic.buffers.len(), 22);
    assert_eq!(diagnostic.bytes.len(), 1832);
    assert_eq!(fixtures::CENTURY_COUNTRY_RUNS, 4_380_000_000);
    assert!(fixtures::CENTURY_COUNTRY_RUNS > u32::MAX as u64);
    let chart = registry.shape(shapes::CHART_SERIES).expect("chart");
    for name in ["runs.$item.first", "runs.$item.last"] {
        let i = chart
            .names
            .iter()
            .position(|n| n == name)
            .expect("chart hour");
        assert_eq!(
            chart.columns[i].descriptor.logical_type,
            LogicalType::Signed
        );
        assert_eq!(chart.columns[i].descriptor.type_param_a, 32);
    }
    assert!(chart.names.iter().any(|n| n == "directory"));
}

#[test]
fn state_witnesses_and_union_names_are_part_of_the_same_contract() {
    for shape in shapes::registry() {
        let mut fields = std::collections::BTreeSet::new();
        for (i, column) in shape.columns.iter().enumerate() {
            let name = &shape.names[i];
            assert!(fields.insert(name), "duplicate static name: {name}");
            let parent = name.rsplit_once('.').map_or("", |(parent, _)| parent);
            for witness in [
                &column.state_mapping.knownness_witness,
                &column.state_mapping.refusal_witness,
            ]
            .into_iter()
            .flatten()
            {
                if shape.id == shapes::CONSTRUCTS {
                    continue;
                }
                let witness_path = if parent.is_empty() {
                    witness.clone()
                } else {
                    format!("{parent}.{witness}")
                };
                assert!(
                    shape.names.contains(&witness_path),
                    "missing sibling witness {witness_path} for {name}"
                );
            }
            if column.descriptor.gate_column != u16::MAX {
                let kind = format!("{parent}.$kind");
                assert!(shape.names.contains(&kind), "missing union kind for {name}");
            }
        }
    }
}
