//! Same-hour selected transport suffix checks. E14640's saved source and
//! combat count compare exactly; cached weight/coastal predicates need native
//! witnesses. No viewer-local selection or prospective result is recorded.
use super::{Report, invalid, pass_inputs::eq};
use installed_content::save::{Save, counter_inputs::Pack};
use replay::{
    frame::DecodedFrame,
    rows::{Recorded, UnitTransferInputs, UnitTransportInputs, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub(super) fn compare(frame: &DecodedFrame, saved: &Save, report: &mut Report) {
    report.attempt("DivisionMovement selected transport", |r| {
        divisions(frame, saved, r)
    });
    report.attempt("RailwayGunCounterInputs selected transport", |r| {
        guns(frame, saved, r)
    });
    report.attempt("CountryAppearance original transport convoys", |r| {
        convoys(frame, saved, r)
    });
    report.attempt("original transport navy access modes", |r| {
        navy_access(frame, saved, r)
    });
    report.attempt("original transport strategic search graph", |r| {
        naval_search(frame, r)
    });
}

fn naval_search(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::ReservedCountryNaming>()?;
    let [row] = rows else {
        return Err(invalid(
            "transport naval search needs one original singleton",
        ));
    };
    let field = "ReservedCountryNaming.transport_mission_inputs.naval_search";
    let Some(packet) = report.recorded(&row.transport_mission_inputs, field, field) else {
        return Ok(());
    };
    let packet = packet
        .as_ref()
        .ok_or_else(|| invalid("transport original mission packet refused"))?;
    eq(
        report,
        &format!("{field}.sample_hour"),
        packet.sample_hour,
        frame.hour,
    );
    let graph = packet
        .naval_search
        .as_ref()
        .ok_or_else(|| invalid("transport original naval search graph refused"))?;
    required(report, &format!("{field}.width"), graph.width);
    required(report, &format!("{field}.regions"), graph.regions.as_ref());
    report.limitation("the native strategic-region neighbour and parallel adjacency order, signed integer centroid, nullable rule references and map-width source are not serialized in text saves; validate them at the frozen native boundary through independent witnesses, not a saved actual route", field);
    report.compared(
        "original transport strategic search graph admission only; no saved counterpart",
        1,
    );
    Ok(())
}

fn navy_access(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    use replay::transport_mission_view::MissionView;
    let rows = frame.complete::<tables::ReservedCountryNaming>()?;
    let [row] = rows else {
        return Err(invalid(
            "transport navy access needs one original singleton",
        ));
    };
    let field = "ReservedCountryNaming.transport_mission_inputs.access_modes";
    if report
        .recorded(&row.transport_mission_inputs, field, field)
        .is_none()
    {
        return Ok(());
    }
    let view = MissionView::new(&row.transport_mission_inputs, frame.hour).map_err(invalid)?;
    let navies = view.physical_navies().map_err(invalid)?;
    let countries = frame.complete::<tables::Countries>()?;
    for country in countries {
        let label = format!("{field} slot {}", country.canonical_slot);
        let Some(saved_country) = saved.countries.get(country.tag.as_slice()) else {
            report.difference(&label, "original country present", "absent from save");
            continue;
        };
        let present = if navies.is_empty() {
            false
        } else {
            navies
                .get(country.canonical_slot as usize)
                .ok_or_else(|| invalid("canonical country outside original physical navy domain"))?
                .present
        };
        let modes = match view.access_modes(country.canonical_slot) {
            Ok(modes) => modes,
            Err(reason) => {
                report.difference(&label, reason, "original current access bytes");
                continue;
            }
        };
        // E9D0C0 writes per_region_access only when its array is nonempty.
        // The independently parsed enclosing strategic_navy block proves
        // presence even when that child was omitted.
        eq(
            report,
            &format!("{label}.saved navy presence"),
            present,
            saved
                .naval_missions
                .presence
                .contains_key(country.tag.as_slice()),
        );
        // The serializer compares CHAR > 0. High-bit raw bytes are retained
        // by capture but omitted by native serialization; omission proves
        // signed nonpositive truth, never the exact raw byte.
        let actual: BTreeMap<_, _> = modes
            .iter()
            .enumerate()
            .filter(|(_, mode)| (**mode as i8) > 0)
            .map(|(region, &mode)| (region as u32, mode))
            .collect();
        let expected =
            saved_access_modes(saved_country.naval_region_access.as_deref().unwrap_or(&[]))?;
        eq(
            report,
            &format!("{label}.saved signed-positive access modes"),
            actual,
            expected,
        );
        for (region, &mode) in modes
            .iter()
            .enumerate()
            .filter(|(_, mode)| (**mode as i8) <= 0)
        {
            report.limitation("native navy access serializer E9D0C0 omission proves signed nonpositive; exact raw zero/high-bit byte is not saved",
                format!("{label} region {region} raw {mode}"));
        }
    }
    report.limitation("only canonical countries' saved strategic navies independently expose signed-positive access modes; omitted zero/high-bit bytes, extra physical slots, array extent, original header/loan identity, writer coverage and region-search geometry/order remain native-witness obligations", field);
    report.compared(
        "original hourly transport navy access modes",
        countries.len(),
    );
    Ok(())
}

/// E9D0C0 secondary+A8 (=primary+B0) emits a pair only for CHAR > 0.
/// A fresh native save cannot explicitly contain zero/high-bit modes. Keep
/// duplicates and unsupported saved forms visible instead of normalizing them.
pub(super) fn saved_access_modes(pairs: &[(u32, u8)]) -> io::Result<BTreeMap<u32, u8>> {
    let mut expected = BTreeMap::new();
    for &(region, mode) in pairs {
        if (mode as i8) <= 0 {
            return Err(invalid(format!(
                "saved navy access region {region} mode {mode} cannot be emitted by native signed-positive gate"
            )));
        }
        if expected.insert(region, mode).is_some() {
            return Err(invalid(format!(
                "saved navy access has repeated signed-positive region {region}"
            )));
        }
    }
    Ok(expected)
}

fn convoys(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let countries: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let rows = frame.complete::<tables::CountryAppearance>()?;
    for row in rows {
        let label = format!("CountryAppearance.transport_convoys {}", row.country);
        let Some(packet) = report.recorded(
            &row.transport_convoys,
            "CountryAppearance.transport_convoys",
            &label,
        ) else {
            continue;
        };
        let Some(packet) = packet else {
            report.difference(
                &label,
                "capture refused",
                "original current-hour convoy packet",
            );
            continue;
        };
        eq(
            report,
            &format!("{label}.sample_hour"),
            packet.sample_hour,
            frame.hour,
        );
        if packet.sample_hour != frame.hour {
            continue;
        }
        required(report, &format!("{label}.priority"), packet.priority);
        required(report, &format!("{label}.unreserved"), packet.unreserved);
        required(report, &format!("{label}.cache"), packet.cache);
        required(
            report,
            &format!("{label}.reservations"),
            packet.reservations.as_ref(),
        );
        required(
            report,
            &format!("{label}.first_variant"),
            packet.first_variant.as_ref(),
        );
        let country = countries
            .get(&row.country)
            .ok_or_else(|| invalid("convoy packet has no original Countries receiver"))?;
        if let Some(cache) = packet.cache {
            eq(
                report,
                &format!("{label}.original held cache"),
                cache.raw,
                country.convoys_raw,
            );
            if cache.valid == 0 {
                eq(
                    report,
                    &format!("{label}.dirty cache raw unreached"),
                    cache.raw,
                    None,
                );
            } else {
                required(report, &format!("{label}.held cache raw"), cache.raw);
            }
        }
        let saved_country = saved
            .countries
            .get(country.tag.as_slice())
            .ok_or_else(|| invalid("convoy packet original country missing from save"))?;
        let saved_pool = saved_country
            .convoy_equipment
            .as_deref()
            .ok_or_else(|| invalid("save lacks original convoy equipment pool"))?;
        let Some(pool) = packet.pool.as_deref() else {
            report.difference(
                format!("{label}.pool"),
                "capture refused",
                "complete original unfiltered pool",
            );
            continue;
        };
        // FFDB00 omits zero entries unless +38 permits them. Comparing the
        // nonzero projection never guesses that unsampled byte. Order and
        // repeated full handles survive; mask1 never selects this census.
        let actual: Vec<_> = pool.iter().filter(|entry| entry.quantity != 0).collect();
        let expected: Vec<_> = saved_pool
            .iter()
            .filter(|entry| entry.amount.raw != 0)
            .collect();
        eq(
            report,
            &format!("{label}.saved nonzero pool count"),
            actual.len(),
            expected.len(),
        );
        for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            let entry = format!("{label}.pool[{index}]");
            let handle = (u64::from(expected.id) << 32) | u64::from(expected.id_type);
            eq(
                report,
                &format!("{entry}.variant"),
                actual.variant,
                Some(handle),
            );
            report.scalar(
                &entry,
                "quantity",
                Some(actual.quantity),
                Some(&expected.amount),
            );
        }
        if let Some(first) = packet.first_variant.as_ref() {
            eq(
                report,
                &format!("{label}.first unfiltered nonzero variant"),
                first.as_ref().map(|value| value.variant),
                expected
                    .first()
                    .map(|entry| (u64::from(entry.id) << 32) | u64::from(entry.id_type)),
            );
            if let Some(first) = first {
                required(
                    report,
                    &format!("{label}.first_variant.visibility"),
                    first.visibility,
                );
                required(
                    report,
                    &format!("{label}.first_variant.surface_detection"),
                    first.surface_detection,
                );
                required(
                    report,
                    &format!("{label}.first_variant.submarine_detection"),
                    first.submarine_detection,
                );
                required(report, &format!("{label}.first_variant.speed"), first.speed);
            }
        }
        if packet.cache.is_some_and(|cache| cache.valid == 0) {
            for (index, entry) in pool.iter().enumerate() {
                required(
                    report,
                    &format!("{label}.dirty classifier[{index}]"),
                    entry.flags,
                );
            }
        }
        report.limitation("signed reservations, runtime transfer priority, +1288 alternate stock, held-cache invalidation, zero-entry inclusion, variant classifier/stat caches and original loan lifetime are not independently serialized here; saved nonzero pool agreement does not certify those operands", &label);
    }
    report.compared(
        "CountryAppearance.transport_convoys original pool and cache joins",
        rows.len(),
    );
    Ok(())
}

fn inverse(saved: &Pack) -> io::Result<BTreeMap<u64, u64>> {
    let mut result = BTreeMap::new();
    for (&transfer, units) in &saved.naval_transfers {
        for &unit in units {
            if result
                .insert(unit, transfer)
                .is_some_and(|previous| previous != transfer)
            {
                return Err(invalid("saved unit belongs to multiple naval transfers"));
            }
        }
    }
    Ok(result)
}

fn required<T>(report: &mut Report, label: &str, value: Option<T>) {
    if value.is_none() {
        report.difference(
            label,
            "capture refused",
            "required current-hour native input",
        );
    }
}

struct UnitSource<'a> {
    handle: u64,
    class13: bool,
    transfer_prefix: bool,
    field: &'a Recorded<Option<UnitTransportInputs>>,
}

fn unit(
    hour: i32,
    source: UnitSource<'_>,
    saved: &Pack,
    inverse: &BTreeMap<u64, u64>,
    report: &mut Report,
) {
    let UnitSource {
        handle,
        class13,
        transfer_prefix,
        field,
    } = source;
    let name = if class13 {
        "RailwayGunCounterInputs.transport"
    } else {
        "DivisionMovement.transport"
    };
    let label = format!("{name} {handle}");
    let Some(packet) = report.recorded(field, name, &label) else {
        return;
    };
    let Some(packet) = packet else {
        report.difference(
            &label,
            "capture refused",
            "required current-hour transport packet",
        );
        return;
    };
    eq(
        report,
        &format!("{label}.sample_hour"),
        packet.sample_hour,
        hour,
    );
    if packet.sample_hour != hour {
        return;
    }
    required(report, &format!("{label}.weight"), packet.weight);
    if class13 {
        eq(
            report,
            &format!("{label}.coastal predicate unreached"),
            packet.coastal_template,
            None,
        );
    } else {
        required(
            report,
            &format!("{label}.coastal_template"),
            packet.coastal_template,
        );
    }
    let Some(transfer) = packet.transfer else {
        report.difference(
            format!("{label}.transfer"),
            "capture refused",
            "native null or authenticated transfer",
        );
        return;
    };
    let actual = match transfer {
        UnitTransferInputs::Absent => None,
        UnitTransferInputs::Present { transfer, .. } => Some(transfer),
    };
    eq(
        report,
        &format!("{label}.transfer prefix"),
        actual.is_some(),
        transfer_prefix,
    );
    eq(
        report,
        &format!("{label}.saved transfer membership"),
        actual,
        inverse.get(&handle).copied(),
    );
    if let UnitTransferInputs::Present {
        transfer,
        status,
        source,
    } = transfer
    {
        required(report, &format!("{label}.transfer.status"), status);
        let Some(expected) = saved.naval_transfer_inputs.get(&transfer) else {
            report.difference(
                format!("{label}.transfer saved identity"),
                transfer,
                "absent",
            );
            return;
        };
        if let Some(status) = status {
            match i32::try_from(expected.combat.len()) {
                Ok(count) => eq(
                    report,
                    &format!("{label}.transfer saved combat count"),
                    status,
                    count,
                ),
                Err(_) => report.difference(
                    format!("{label}.transfer saved combat count"),
                    status,
                    "outside signed32",
                ),
            }
            if status == 0 {
                required(report, &format!("{label}.transfer.source"), source);
                if expected.source.is_none() {
                    report.difference(
                        format!("{label}.transfer saved source"),
                        "missing province",
                        "mandatory E14640 source",
                    );
                } else {
                    eq(
                        report,
                        &format!("{label}.transfer saved source"),
                        source,
                        expected.source,
                    );
                }
            } else {
                eq(
                    report,
                    &format!("{label}.transfer source unreached"),
                    source,
                    None,
                );
            }
        }
    }
    report.limitation("stat+2B8 weight and the real template B8EBD0 coastal predicate are runtime inputs: the save does not independently serialize these getter operands. Current-hour/identity checks do not certify pointer lifetime, cache writers or quiescence; use the matched native witness and smoke workflow", &label);
}

fn divisions(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::DivisionMovement>()?;
    let inverse = inverse(&saved.counter_inputs)?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.division == 0 || !seen.insert(row.division) {
            return Err(invalid("null or repeated selected transport division"));
        }
        unit(
            frame.hour,
            UnitSource {
                handle: row.division,
                class13: false,
                transfer_prefix: row.naval_transfer != 0,
                field: &row.transport,
            },
            &saved.counter_inputs,
            &inverse,
            report,
        );
    }
    eq(
        report,
        "DivisionMovement selected transport saved census",
        seen,
        saved.divisions.keys().copied().collect(),
    );
    report.compared(
        "DivisionMovement.transport saved transfer / runtime packet checks",
        rows.len(),
    );
    Ok(())
}

fn guns(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::RailwayGunCounterInputs>()?;
    let inverse = inverse(&saved.counter_inputs)?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.gun == 0 || !seen.insert(row.gun) {
            return Err(invalid("null or repeated selected transport gun"));
        }
        unit(
            frame.hour,
            UnitSource {
                handle: row.gun,
                class13: true,
                transfer_prefix: row.naval_transfer != 0,
                field: &row.transport,
            },
            &saved.counter_inputs,
            &inverse,
            report,
        );
    }
    eq(
        report,
        "RailwayGunCounterInputs selected transport saved census",
        seen,
        saved.railway_guns.keys().copied().collect(),
    );
    report.compared(
        "RailwayGunCounterInputs.transport saved transfer / runtime packet checks",
        rows.len(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const UNIT: u64 = (7u64 << 32) | 51;
    const TRANSFER: u64 = (9u64 << 32) | 4713;
    fn pack(source: Option<u32>, combat: Vec<u64>) -> Pack {
        Pack {
            naval_transfers: BTreeMap::from([(TRANSFER, vec![UNIT, UNIT])]),
            naval_transfer_inputs: BTreeMap::from([(
                TRANSFER,
                installed_content::save::counter_inputs::NavalTransfer { source, combat },
            )]),
            ..Default::default()
        }
    }
    fn packet(status: Option<i32>, source: Option<u32>) -> Recorded<Option<UnitTransportInputs>> {
        Recorded::Value(Some(UnitTransportInputs {
            sample_hour: 12,
            weight: Some(i64::MIN),
            coastal_template: Some(false),
            transfer: Some(UnitTransferInputs::Present {
                transfer: TRANSFER,
                status,
                source,
            }),
        }))
    }
    fn check(field: Recorded<Option<UnitTransportInputs>>, saved: &Pack, hour: i32) -> Report {
        let mut report = Report::default();
        unit(
            hour,
            UnitSource {
                handle: UNIT,
                class13: false,
                transfer_prefix: true,
                field: &field,
            },
            saved,
            &inverse(saved).unwrap(),
            &mut report,
        );
        report
    }
    #[test]
    fn saved_source_and_exact_combat_count_do_not_waive_differences() {
        let saved = pack(Some(42), Vec::new());
        assert!(
            check(packet(Some(0), Some(42)), &saved, 12)
                .differences
                .is_empty()
        );
        assert_eq!(
            check(packet(Some(0), Some(41)), &saved, 12)
                .differences
                .len(),
            1
        );
        assert_eq!(
            check(packet(Some(0), None), &saved, 12).differences.len(),
            2
        );
        assert_eq!(
            check(packet(Some(0), Some(42)), &pack(None, Vec::new()), 12)
                .differences
                .len(),
            1
        );
        let saved = pack(Some(42), vec![0, (8u64 << 32) | 4713]);
        assert!(
            check(packet(Some(2), None), &saved, 12)
                .differences
                .is_empty()
        );
        assert_eq!(
            check(packet(Some(1), None), &saved, 12).differences.len(),
            1
        );
        assert_eq!(
            check(packet(Some(-1), None), &saved, 12).differences.len(),
            1
        );
    }
    #[test]
    fn historical_absence_refusal_and_current_hour_are_independent() {
        let saved = pack(Some(42), Vec::new());
        let old = check(Recorded::NotRecorded, &saved, 12);
        assert!(old.differences.is_empty());
        assert!(!old.limitations.is_empty());
        assert_eq!(
            check(Recorded::Value(None), &saved, 12).differences.len(),
            1
        );
        assert_eq!(
            check(packet(Some(0), Some(42)), &saved, 13)
                .differences
                .len(),
            1
        );
        assert_eq!(check(packet(None, None), &saved, 12).differences.len(), 1);
    }
    #[test]
    fn duplicate_same_transfer_occurrences_survive_but_two_owners_refuse() {
        let mut saved = pack(Some(42), Vec::new());
        assert_eq!(inverse(&saved).unwrap()[&UNIT], TRANSFER);
        saved
            .naval_transfers
            .insert(TRANSFER + (1u64 << 32), vec![UNIT]);
        assert!(inverse(&saved).is_err());
    }

    #[test]
    fn original_convoy_oracle_preserves_signed_order_repeats_and_reports_each_leaf_gap() {
        use replay::{
            frame::TableView,
            rows::{
                CountryAppearanceRow, TableId, TableRows, TransportConvoyCache,
                TransportConvoyEntry, TransportConvoyReservation, TransportConvoyVariant,
                TransportConvoys,
            },
        };
        const VARIANT: u64 = (71_u64 << 32) | 70;
        let (mut frame, mut saved, _, _) = super::super::tests::fixture();
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .convoy_equipment = Some(vec![
            installed_content::save::EquipmentAmount {
                id: 71,
                id_type: 70,
                amount: installed_content::save::Decimal::parse(b"-0.00001").unwrap(),
            },
            installed_content::save::EquipmentAmount {
                id: 71,
                id_type: 70,
                amount: installed_content::save::Decimal::parse(b"0.00001").unwrap(),
            },
        ]);
        let mut packet = TransportConvoys {
            sample_hour: frame.hour,
            priority: Some(-2),
            unreserved: Some(-8),
            cache: Some(TransportConvoyCache {
                valid: 255,
                raw: Some(0),
            }),
            pool: Some(vec![
                TransportConvoyEntry {
                    variant: None,
                    flags: None,
                    quantity: 0,
                },
                TransportConvoyEntry {
                    variant: Some(VARIANT),
                    flags: None,
                    quantity: -1,
                },
                TransportConvoyEntry {
                    variant: Some(VARIANT),
                    flags: None,
                    quantity: 1,
                },
            ]),
            reservations: Some(vec![TransportConvoyReservation {
                priority: -3,
                quantity: -7,
            }]),
            first_variant: Some(Some(TransportConvoyVariant {
                variant: VARIANT,
                visibility: Some(-5),
                surface_detection: Some(-3),
                submarine_detection: Some(4),
                speed: Some(6),
            })),
        };
        let check = |frame: &mut DecodedFrame, input| {
            *frame.tables[TableId::CountryAppearance.index()] =
                TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                    country: 101,
                    transport_convoys: input,
                    ..Default::default()
                }]));
            let mut report = Report::default();
            convoys(frame, &saved, &mut report).unwrap();
            report
        };
        assert!(
            check(&mut frame, Recorded::Value(Some(packet.clone())))
                .differences
                .is_empty()
        );
        packet
            .first_variant
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .speed = None;
        let report = check(&mut frame, Recorded::Value(Some(packet.clone())));
        assert_eq!(report.differences.len(), 1);
        assert!(
            report
                .difference_lines()
                .join("\n")
                .contains("first_variant.speed")
        );
        packet
            .first_variant
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .speed = Some(6);
        packet.pool.as_mut().unwrap()[2].variant = None;
        assert_eq!(
            check(&mut frame, Recorded::Value(Some(packet.clone())))
                .differences
                .len(),
            1
        );
        packet.pool.as_mut().unwrap()[2].variant = Some(VARIANT);
        packet.pool.as_mut().unwrap()[1].quantity = -2;
        assert_eq!(
            check(&mut frame, Recorded::Value(Some(packet.clone())))
                .differences
                .len(),
            1
        );
        packet.pool.as_mut().unwrap()[1].quantity = -1;
        packet.cache = Some(TransportConvoyCache {
            valid: 0,
            raw: None,
        });
        let report = check(&mut frame, Recorded::Value(Some(packet.clone())));
        assert_eq!(
            report.differences.len(),
            4,
            "original held-cache join plus every dirty classifier"
        );
        packet.sample_hour -= 1;
        assert_eq!(
            check(&mut frame, Recorded::Value(Some(packet)))
                .differences
                .len(),
            1
        );
        assert_eq!(
            check(&mut frame, Recorded::Value(None)).differences.len(),
            1
        );
        let historical = check(&mut frame, Recorded::NotRecorded);
        assert!(historical.differences.is_empty() && !historical.limitations.is_empty());
    }

    #[test]
    fn original_navy_access_oracle_uses_signed_writer_gate_and_enclosing_presence() {
        use replay::{
            frame::TableView,
            rows::{
                CountryRow, ReservedCountryNamingRow, TableId, TableRows, TransportMissionInputs,
                TransportNavy,
            },
        };
        let (mut frame, mut saved, _, _) = super::super::tests::fixture();
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 7,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                ..Default::default()
            }]));
        let mut packet = TransportMissionInputs {
            sample_hour: frame.hour,
            naval_search: None,
            navies: Some(vec![
                TransportNavy {
                    slot: 0,
                    present: false,
                    region_slots: Some(0),
                    ledgers: Some(vec![]),
                    access_modes: Some(vec![]),
                },
                TransportNavy {
                    slot: 1,
                    present: true,
                    region_slots: None,
                    ledgers: None,
                    access_modes: Some(vec![0, 1, 127, 128, 255, 2]),
                },
            ]),
            receivers: None,
            ..Default::default()
        };
        let install = |frame: &mut DecodedFrame, packet: TransportMissionInputs| {
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    ReservedCountryNamingRow {
                        transport_mission_inputs: Recorded::Value(Some(packet)),
                        ..Default::default()
                    },
                ]));
        };
        saved
            .naval_missions
            .presence
            .insert(b"GER".to_vec(), BTreeMap::new());
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .naval_region_access = Some(vec![(1, 1), (2, 127), (5, 2)]);
        install(&mut frame, packet.clone());
        let mut report = Report::default();
        navy_access(&frame, &saved, &mut report).unwrap();
        assert!(
            report.differences.is_empty(),
            "{:?}",
            report.difference_lines()
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("omission proves signed nonpositive"))
        );
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .naval_region_access = Some(vec![(1, 2), (2, 127), (5, 2)]);
        let mut report = Report::default();
        navy_access(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);

        packet.navies.as_mut().unwrap()[1].access_modes = Some(vec![0, 255]);
        install(&mut frame, packet.clone());
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .naval_region_access = Some(vec![]);
        let mut report = Report::default();
        navy_access(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        packet.navies.as_mut().unwrap()[1].access_modes = None;
        install(&mut frame, packet);
        let mut report = Report::default();
        navy_access(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn navy_access_saved_pairs_require_exact_signed_positive_gate_and_unique_regions() {
        assert_eq!(
            saved_access_modes(&[(7, 127), (3, 1)]).unwrap(),
            BTreeMap::from([(3, 1), (7, 127)])
        );
        for pairs in [
            vec![(7, 0)],
            vec![(7, 128)],
            vec![(7, 255)],
            vec![(7, 1), (7, 1)],
            vec![(7, 1), (7, 127)],
        ] {
            assert!(saved_access_modes(&pairs).is_err(), "{pairs:?}");
        }
        assert!(saved_access_modes(&[]).unwrap().is_empty());
    }
}
