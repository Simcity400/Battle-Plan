use super::*;
use engine::{
    ccountry, cgame_state, csub_unit_definition as subunit,
    memory::{CountedMemory, SliceMemory},
};

/// Base zero; the subunit database pointer reads as 0x8800.
struct Fixture {
    bytes: Vec<u8>,
}
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == subunit::DATABASE_RVA && output.len() == 8 {
            output.copy_from_slice(&0x8800_u64.to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}
fn fixture() -> Fixture {
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: u64, count: u32) {
        word(bytes, at, data);
        word(bytes, at + 8, u64::from(count) | (u64::from(count) << 32));
    }
    fn text(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
        word(bytes, at + 16, value.len() as u64);
        word(bytes, at + 24, 15);
    }
    let mut bytes = vec![0; 0x9000];
    array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x2000, 2);
    array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x2100, 2);
    word(&mut bytes, 0x2008, 0x3000);
    bytes[0x2104..0x2108].copy_from_slice(&1_u32.to_le_bytes());
    word(&mut bytes, 0x3000, ccountry::VTABLE_RVA as u64);
    word(&mut bytes, 0x3008, 1);
    array(&mut bytes, 0x3000 + ccountry::DIVISION_TEMPLATES, 0x4000, 1);
    word(&mut bytes, 0x4000, 0x5000);
    word(&mut bytes, 0x5000, native::VTABLE_RVA as u64);
    word(&mut bytes, 0x5008, 0xffff_ffff_0000_0034);
    text(&mut bytes, 0x5000 + native::NAME, &[b'A', 0, 255]);
    text(&mut bytes, 0x5000 + native::NAME_KEY, b"fallback");
    text(&mut bytes, 0x5000 + native::MODEL_ENTITY, b"GER_x_entity");
    array(&mut bytes, 0x5000 + native::SUBUNIT_COUNTS, 0x6000, 1);
    word(&mut bytes, 0x6000, 0x7000);
    word(&mut bytes, 0x6008, 3);
    word(&mut bytes, 0x5000 + native::PRIORITY_UNIT, 0x7000);
    word(&mut bytes, 0x7000, subunit::VTABLE_RVA as u64);
    word(&mut bytes, 0x7000 + subunit::TOKEN, 777);
    bytes[0x7000 + subunit::IS_REAL] = 1;
    text(&mut bytes, 0x7000 + subunit::NAME, b"infantry");
    word(&mut bytes, 0x7000 + subunit::DATABASE_INDEX, 1);
    array(&mut bytes, 0x8800 + subunit::DATABASE_VECTOR, 0x8900, 2);
    word(&mut bytes, 0x8908, 0x7000);
    Fixture { bytes }
}
fn capacity() -> Capacities {
    Capacities {
        rows: 2,
        bytes: 32,
        counts: 4,
        variants: 4,
        icon_rows: 2,
        icon_bytes: 32,
        name_bytes: 32,
        database: 8,
        locals: 8,
    }
}

/// Both tables are read every hour, the dictionary from the icon walk, and
/// unreadable memory fails both rather than claiming an empty roster.
#[test]
fn every_hour_reads_both_tables_and_bad_memory_cannot_claim_empty() {
    let mut capture = Captured::with_capacities(&capacity());
    let present = CountedMemory::new(fixture());
    let missing = CountedMemory::new(SliceMemory {
        base: 0,
        bytes: &[],
    });
    for (hour, force) in (-24..24).map(|hour| (hour, false)).chain([(0, true)]) {
        assert!(replay::cadence::should_walk(TABLES[0], hour, force));
        let reads = crate::read_accounting::Capture::new();
        capture.capture(&present, 0, 0x1000, hour, force);
        let read = reads.finish();
        assert!(read[TABLES[1].index()].spans > 0);
        // Unobserved, the name key behind the nonempty literal is read.
        assert!(read[TABLES[0].index()].spans > 0, "{hour}");
        assert_eq!(
            capture.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        capture.capture(&missing, 0, 0x1000, hour, force);
        let [Outcome::Unavailable { reason }, Outcome::Unavailable { .. }] = capture.outcomes()
        else {
            panic!("unreadable templates must fail at hour {hour}");
        };
        assert_ne!(reason, replay::cadence::SKIPPED_REASON);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }
}

/// The hourly job reads the icon and demanded names directly, then publishes
/// both tables from that walk; taking its preacquisition performs no live read.
#[test]
fn hourly_template_job_publishes_direct_walk_rows() {
    use crate::acquisition_jobs::TemplatesRequest;
    use engine::country_memory::Cache;
    use std::sync::Arc;

    let mut source = fixture();
    let mut capture = Captured::vacant();
    for (hour, literal, custom_icon) in [(7, &b"Alpha"[..], 11_i32), (8, &b"Bravo"[..], 22)] {
        set_name(&mut source, literal, b"fallback");
        source.bytes[0x5000 + native::CUSTOM_ICON..][..4]
            .copy_from_slice(&custom_icon.to_le_bytes());
        let mut countries = Arc::new(Cache::try_with_capacity(256).unwrap());
        let countries = Cache::prepare_shared(&mut countries, &source, 0, 0x1000).unwrap();
        let mut request = TemplatesRequest::take(&countries, 0, 0x1000, &mut capture, hour, false);
        request.run(&source);
        request.restore(&mut capture, None);
        // An unreadable source cannot affect the already acquired tables.
        capture.capture(
            &SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0x1000,
            hour,
            false,
        );
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: 1 }; N]);
        let [
            TableView::Complete(replay::rows::TableRows::DivisionTemplates(dictionary)),
            TableView::Complete(replay::rows::TableRows::DivisionTemplateIcons(icons)),
        ] = capture.tables()
        else {
            panic!("complete template job tables at {hour}");
        };
        assert_eq!(dictionary.len(), 1);
        assert_eq!(dictionary[0].name, literal);
        assert_eq!(
            dictionary[0].name_key,
            Recorded::Value(b"fallback".to_vec())
        );
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].template, dictionary[0].template);
        assert_eq!(icons[0].custom_icon, custom_icon);
        assert_eq!(icons[0].priority_unit_name, b"infantry");
        assert_eq!(
            icons[0]
                .display_name_inputs(hour, Some(&dictionary[0]))
                .unwrap()
                .literal,
            literal
        );
    }
}

#[test]
fn a_failed_capture_counts_no_model_failures() {
    let mut capture = Captured::with_capacities(&capacity());
    capture
        .model_failures
        .add("template grouped subunits unreadable");
    assert_eq!(capture.model_failures().rows, 1);
    capture.fail_capture(3, false, "slow acquisition failed");
    assert_eq!(capture.model_failures(), ModelFailures::default());
}

#[test]
fn conversion_preserves_exact_bytes_and_rejects_every_invalid_span() {
    let mut capture = Captured::with_capacities(&capacity());
    let mut source = fixture();
    source.bytes
        [0x5000 + native::FORMATION_DIVISION_COUNT..0x5000 + native::FORMATION_DIVISION_COUNT + 4]
        .copy_from_slice(&(-17_i32).to_le_bytes());
    capture.capture(&source, 0, 0x1000, 3, true);
    let [
        TableView::Complete(replay::rows::TableRows::DivisionTemplates(rows)),
        TableView::Complete(replay::rows::TableRows::DivisionTemplateIcons(icons)),
    ] = capture.tables()
    else {
        panic!("complete templates");
    };
    assert_eq!(rows[0].template, 0xffff_ffff_0000_0034);
    assert_eq!(rows[0].name, &[b'A', 0, 255]);
    assert_eq!(*rows[0].name_key.require("name_key").unwrap(), b"fallback");
    assert_eq!(icons[0].model_entity, b"GER_x_entity");
    assert_eq!(
        (icons[0].representative, icons[0].representative_local),
        (777, 0)
    );
    assert_eq!(
        icons[0].subunits,
        [TemplateSubunit {
            token: 777,
            local: 0,
            count: 3
        }]
    );
    assert_eq!(icons[0].template, 0xffff_ffff_0000_0034);
    assert_eq!(icons[0].priority_unit, 777);
    assert_eq!(icons[0].priority_unit_name, b"infantry");
    assert_eq!(
        icons[0].formation_division_count,
        replay::rows::Recorded::Value(Some(-17))
    );
    // A row whose model inputs failed publishes the flag, keeps its
    // counter fields and is counted by reason.
    let good = capture.icon_rows.rows[0];
    capture.icon_rows.rows[0].model_failure = Some("template grouped subunits unreadable");
    let [
        _,
        TableView::Complete(replay::rows::TableRows::DivisionTemplateIcons(flagged)),
    ] = capture.tables()
    else {
        panic!("complete icons");
    };
    assert_eq!(flagged[0].model_unreadable, 1);
    assert_eq!(flagged[0].priority_unit_name, b"infantry");
    assert_eq!(
        flagged[0].formation_division_count,
        replay::rows::Recorded::Value(Some(-17))
    );
    capture.icon_rows.rows[0] = good;
    let mut failures = ModelFailures::default();
    for reason in ["a", "b", "a"] {
        failures.add(reason);
    }
    assert_eq!(
        (failures.rows, &failures.reasons[..3], failures.other),
        (3, &[("a", 2), ("b", 1), ("", 0)][..], 0)
    );
    let daily = capture.rows.rows[0];
    for name_key in [false, true] {
        capture.rows.rows[0] = daily;
        let row = &mut capture.rows.rows[0];
        let name = if name_key {
            &mut row.name_key
        } else {
            &mut row.name
        };
        *name = native::Span {
            start: usize::MAX,
            len: 1,
        };
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
    let original = capture.icon_rows.rows[0];
    for which in 0..4 {
        capture.icon_rows.rows[0] = original;
        let row = &mut capture.icon_rows.rows[0];
        let span = match which {
            0 => &mut row.priority_unit_name,
            1 => &mut row.model_entity,
            2 => &mut row.counts,
            _ => &mut row.excluded_variants,
        };
        *span = native::Span {
            start: usize::MAX,
            len: 1,
        };
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }
}

#[test]
#[ignore = "synthetic template fallback capture and encoded delta costs"]
fn border_effect_template_key_scale_bench() {
    use replay::rows::RowCodec;
    use std::time::Instant;

    for templates in [193, 1_500, 5_000] {
        let list = 0xa000;
        let objects = (list + templates * 8 + 15) & !15;
        let mut memory = fixture();
        let template = memory.bytes[0x5000..0x5240].to_vec();
        memory.bytes.resize(objects + templates * 0x240, 0);
        memory.bytes[0x3000 + ccountry::DIVISION_TEMPLATES..][..8]
            .copy_from_slice(&(list as u64).to_le_bytes());
        for offset in [8, 12] {
            memory.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + offset..][..4]
                .copy_from_slice(&(templates as u32).to_le_bytes());
        }
        for index in 0..templates {
            let object = objects + index * 0x240;
            memory.bytes[object..object + 0x240].copy_from_slice(&template);
            memory.bytes[list + index * 8..][..8].copy_from_slice(&(object as u64).to_le_bytes());
            memory.bytes[object + 8..][..8].copy_from_slice(
                &(((index as u64 + 1) << 32) | u64::from(native::REFERENCE_TYPE)).to_le_bytes(),
            );
        }
        let mut capacities = capacity();
        capacities.rows = templates;
        capacities.icon_rows = templates;
        capacities.bytes = templates * 16;
        capacities.icon_bytes = templates * 32;
        capacities.counts = templates;
        let mut captured = Captured::with_capacities(&capacities);
        let rows = |captured: &Captured| {
            let TableView::Complete(replay::rows::TableRows::DivisionTemplates(rows)) =
                captured.tables().into_iter().next().unwrap()
            else {
                panic!("complete template dictionary");
            };
            rows
        };
        for (shape, changed) in [("quiet", 0), ("typical", 1), ("all_changed", templates)] {
            captured.capture(&memory, 0, 0x1000, 3, true);
            let mut previous = rows(&captured);
            let mut times = Vec::new();
            let mut delta_sizes = Vec::new();
            for pass in 0..5 {
                for index in 0..changed {
                    memory.bytes[objects + index * 0x240 + native::NAME_KEY + 7] = b'0' + pass;
                }
                let started = Instant::now();
                captured.capture(&memory, 0, 0x1000, 3, true);
                times.push(started.elapsed());
                let current = rows(&captured);
                let mut bytes = Vec::new();
                let mut key = 0;
                for (row, old) in current.iter().zip(&previous) {
                    if row.encode_change(Some(old), key, &mut bytes).unwrap() {
                        assert_eq!(row.delta(Some(old)).unwrap().unwrap().mask, 1 << 3);
                        key = row.key();
                    }
                }
                previous = current;
                delta_sizes.push(bytes.len());
            }
            times.sort_unstable();
            assert!(delta_sizes.iter().all(|size| *size == delta_sizes[0]));
            let counted = CountedMemory::new(memory);
            let accounting = crate::read_accounting::Capture::new();
            captured.capture(&counted, 0, 0x1000, 3, true);
            let requests = accounting.finish()[TABLES[0].index()];
            println!(
                "TEMPLATE_KEY_BENCH templates={templates} shape={shape} whole_capture_p50_us={} dictionary_requests={requests:?} added_header_bytes={} prior_requested_bytes={} baseline_delta_bytes=0 delta_bytes={} initial_field_bytes={}",
                times[times.len() / 2].as_micros(),
                templates * 32,
                requests.bytes - (templates * 32) as u64,
                delta_sizes[0],
                templates * 12
            );
            memory = counted.0;
        }
    }
}

fn displayed(inputs: [TableInput; N]) -> DivisionTemplateIconRow {
    let [
        _,
        TableInput::Sampled(TableView::Complete(replay::rows::TableRows::DivisionTemplateIcons(
            mut rows,
        ))),
    ] = inputs
    else {
        panic!("complete hourly icon fixture")
    };
    rows.remove(0)
}
fn set_name(source: &mut Fixture, literal: &[u8], key: &[u8]) {
    for (at, text) in [
        (0x5000 + native::NAME, literal),
        (0x5000 + native::NAME_KEY, key),
    ] {
        assert!(text.len() <= 15);
        source.bytes[at..at + 16].fill(0);
        source.bytes[at..at + text.len()].copy_from_slice(text);
        source.bytes[at + 16..at + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
        source.bytes[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
    }
}

#[test]
fn hourly_equality_and_same_day_rename_use_exact_same_frame_bytes() {
    let mut capture = Captured::with_capacities(&capacity());
    let mut source = fixture();
    capture.capture(&source, 0, 0x1000, 9, true);
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            value: TemplateNameValue::SameAsDivisionTemplates,
            ..
        }))
    ));
    capture.capture(&source, 0, 0x1000, 10, false);
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            sample_hour: replay::rows::PacketHour::Host,
            value: TemplateNameValue::SameAsDivisionTemplates
        }))
    ));
    // A rename reaches the same hour's dictionary, so the name certifies
    // against it.
    set_name(&mut source, b"Renamed", b"unused");
    capture.capture(&source, 0, 0x1000, 11, false);
    let [
        TableView::Complete(replay::rows::TableRows::DivisionTemplates(dictionary)),
        _,
    ] = capture.tables()
    else {
        panic!("renamed dictionary")
    };
    assert_eq!(dictionary[0].name, b"Renamed");
    let renamed = displayed(capture.table_inputs());
    assert_eq!(
        renamed
            .display_name_inputs(11, Some(&dictionary[0]))
            .unwrap()
            .literal,
        b"Renamed"
    );
    assert_eq!(renamed.priority_unit_name, b"infantry");
}

/// The dictionary is complete at every hour, so a display name equal to it
/// is a same-frame certificate; a failed dictionary needs a self-contained
/// override.
#[test]
fn every_hour_certifies_against_the_same_frame_and_a_failed_dictionary_overrides() {
    let mut capture = Captured::with_capacities(&capacity());
    let source = fixture();
    capture.capture(&source, 0, 0x1000, 10, false);
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            value: TemplateNameValue::SameAsDivisionTemplates,
            ..
        }))
    ));
    let mut small = capacity();
    small.rows = 0;
    let mut capture = Captured::with_capacities(&small);
    capture.capture(&source, 0, 0x1000, 9, true);
    capture.rows.outcome = Outcome::Unavailable {
        reason: "synthetic dictionary failure",
    };
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            value: TemplateNameValue::Changed { .. },
            ..
        }))
    ));
}

#[test]
fn name_arena_growth_keeps_the_first_heading_and_counts_exact_demand() {
    let mut size = capacity();
    size.name_bytes = 0;
    let mut capture = Captured::with_capacities(&size);
    capture.capture(&fixture(), 0, 0x1000, 9, true);
    let row = displayed(capture.table_inputs());
    assert!(matches!(row.display_name, Recorded::Value(Some(_))));
    assert_eq!(row.priority_unit_name, b"infantry");
    assert_eq!(row.model_unreadable, 0);
    assert_eq!(capture.name_bytes.required, 3);
    capture.grow(&mut size);
    assert!(size.name_bytes >= 3);
}

fn large_content(countries: usize, per_country: usize) -> Fixture {
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, count: usize) {
        word(bytes, at, data as u64);
        word(bytes, at + 8, (count as u64) | ((count as u64) << 32));
    }
    let mut source = fixture();
    let template = source.bytes[0x5000..0x5250].to_vec();
    let slots = countries + 1;
    let registry = 0x10000;
    let links = registry + slots * 8;
    let owners = (links + slots * 4).next_multiple_of(16);
    let rosters = owners + countries * 0x200;
    let templates = rosters + countries * per_country * 8;
    source
        .bytes
        .resize(templates + countries * per_country * template.len(), 0);
    array(
        &mut source.bytes,
        0x1000 + cgame_state::COUNTRIES,
        registry,
        slots,
    );
    array(
        &mut source.bytes,
        0x1000 + cgame_state::COUNTRY_LINKS,
        links,
        slots,
    );
    for country in 0..countries {
        let id = country + 1;
        let owner = owners + country * 0x200;
        word(&mut source.bytes, registry + id * 8, owner as u64);
        source.bytes[links + id * 4..links + id * 4 + 4]
            .copy_from_slice(&(id as u32).to_le_bytes());
        word(&mut source.bytes, owner, ccountry::VTABLE_RVA as u64);
        word(&mut source.bytes, owner + ccountry::ID, id as u64);
        let roster = rosters + country * per_country * 8;
        array(
            &mut source.bytes,
            owner + ccountry::DIVISION_TEMPLATES,
            roster,
            per_country,
        );
        for ordinal in 0..per_country {
            let index = country * per_country + ordinal;
            let at = templates + index * template.len();
            source.bytes[at..at + template.len()].copy_from_slice(&template);
            word(
                &mut source.bytes,
                at + native::TYPE,
                ((index as u64 + 1) << 32) | native::REFERENCE_TYPE as u64,
            );
            word(&mut source.bytes, roster + ordinal * 8, at as u64);
        }
    }
    source
}

#[test]
fn live_template_demand_and_same_capture_growth_keep_five_thousand_countries() {
    let source = large_content(5_000, 4);
    let mut capacity = Capacities::default();
    capacity.presize(&source, 0, 0x1000).unwrap();
    assert!(capacity.rows >= 20_000 && capacity.icon_rows >= 20_000);
    assert!(capacity.bytes >= 20_000 * 11);
    assert!(capacity.icon_bytes >= 20_000 * 20);
    assert!(capacity.counts >= 20_000);
    let mut captured = Captured::with_capacities(&capacity);
    captured.capture(&source, 0, 0x1000, 9, true);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 20_000 }; N]);
    let grown = large_content(5_000, 5);
    captured.capture(&grown, 0, 0x1000, 10, true);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 25_000 }; N]);
    assert_eq!(captured.name_bytes.len, 25_000 * 3);
    assert_eq!(captured.counts.len, 25_000);
}

#[test]
fn optional_model_failure_does_not_remove_fresh_name_inputs() {
    let mut capture = Captured::with_capacities(&capacity());
    let mut source = fixture();
    source.bytes[0x5000 + native::SUBUNIT_COUNTS + 8..0x5000 + native::SUBUNIT_COUNTS + 12]
        .copy_from_slice(&1_u32.to_le_bytes());
    source.bytes[0x5000 + native::SUBUNIT_COUNTS + 12..0x5000 + native::SUBUNIT_COUNTS + 16]
        .copy_from_slice(&2_u32.to_le_bytes()); // length beyond capacity
    capture.capture(&source, 0, 0x1000, 9, true);
    let row = displayed(capture.table_inputs());
    assert_eq!(row.model_unreadable, 1);
    assert!(matches!(row.display_name, Recorded::Value(Some(_))));
}
