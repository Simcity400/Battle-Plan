use super::super::super::tests::{frame, path_assets, put};
use super::*;
use installed_content::unit_path::{Army, CommonMoveWorld, CommonUnit};
use replay::rows::{DivisionMovementRow, NavalBaseRow, RailwayGunCounterInputRow, Recorded};

fn query<T>(frame: &replay::frame::DecodedFrame, run: impl FnOnce(&Moves<'_, '_>) -> T) -> T {
    let assets = path_assets();
    query_with_assets(frame, &assets, run)
}

fn query_with_assets<T>(
    frame: &replay::frame::DecodedFrame,
    assets: &RaidAssets,
    run: impl FnOnce(&Moves<'_, '_>) -> T,
) -> T {
    let world = World::read(frame, assets.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    run(&Moves::for_common(&world, assets, &cells, &common).unwrap())
}

#[test]
fn nested_caller_absent_variant_returns_zero_but_zero_visibility_demands_air() {
    use replay::rows::{
        CountryAppearanceRow, ReservedCountryNamingRow, TransportConvoyVariant, TransportConvoys,
        TransportMissionInputs, TransportMissionReceiver, TransportReceiverRisk,
        TransportTaskForceStats,
    };
    let mut frame = frame();
    let hour = frame.hour;
    let mut assets = path_assets();
    assets.transport_damage =
        replay::transport_damage_graph::Defines::read(|_, _| Ok::<_, String>(100_000));
    assets.transport_detection = Ok(replay::transport_spotting::Defines {
        base: 100_000,
        radar_bonus: 0,
        air_superiority_bonus: 0,
    });
    assets.transport_spotting =
        replay::transport_spotting_graph::Defines::read(|_, _| Ok::<_, String>(100_000));
    assets.transport_chance = Ok(
        installed_content::unit_path::selected_transport_risk::ChanceDefines {
            spotting_weight: 100_000,
            damage_weight: 100_000,
            base: 100_000,
        },
    );
    let receiver = TransportMissionReceiver {
        task_force: (7_u64 << 32) | 61,
        mission_owner: Some(11),
        in_regions: Some(1),
        combat_references: Some(0),
        combined_detection: Some(0),
        detection_cache: Some([0, 0]),
        received_supply_ratio: Some(100_000),
        risk: Some(TransportReceiverRisk {
            region: Some(0),
            ship_count: Some(0),
            stats: Some(TransportTaskForceStats {
                detection_138: Some(0),
                carrier_detection_160: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    put::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            transport_mission_inputs: Recorded::Value(Some(TransportMissionInputs {
                sample_hour: hour,
                navies: Some(vec![]),
                receivers: Some(vec![receiver]),
                ..Default::default()
            })),
            ..Default::default()
        }],
    );
    let mut convoys = TransportConvoys {
        sample_hour: hour,
        first_variant: Some(None),
        ..Default::default()
    };
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 11,
            transport_convoys: Recorded::Value(Some(convoys.clone())),
            ..Default::default()
        }],
    );
    assert_eq!(
        query_with_assets(&frame, &assets, |moves| graph::chance(
            moves, &receiver, 11, 1
        )),
        Ok(0)
    );
    convoys.first_variant = Some(Some(TransportConvoyVariant {
        visibility: Some(0),
        ..Default::default()
    }));
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 11,
            transport_convoys: Recorded::Value(Some(convoys)),
            ..Default::default()
        }],
    );
    assert!(
        query_with_assets(&frame, &assets, |moves| graph::chance(
            moves, &receiver, 11, 1
        ))
        .unwrap_err()
        .contains("air")
    );
}

#[test]
fn complete_nested_caller_uses_current_context_terrain_targeted_and_night_inputs() {
    use replay::rows::{
        CountryAppearanceRow, ReservedCountryNamingRow, TransportAirCountry, TransportAirInputs,
        TransportAirRegion, TransportConvoyVariant, TransportConvoys, TransportMissionInputs,
        TransportMissionReceiver, TransportRadarCounts, TransportReceiverRisk,
        TransportSpottingContext, TransportSpottingOwner, TransportSpottingRegion,
        TransportTaskForceStats, WeatherProvinceRow,
    };
    let mut frame = frame();
    let hour = frame.hour;
    let mut assets = path_assets();
    assets.transport_damage = replay::transport_damage_graph::Defines::read(|_, name| {
        Ok::<_, String>(if name == "BASE_UNIT_TRANSFER_SPOTTING_SPEED" {
            5_000_000
        } else {
            0
        })
    });
    assets.transport_detection = Ok(replay::transport_spotting::Defines {
        base: 100_000,
        radar_bonus: 0,
        air_superiority_bonus: 0,
    });
    assets.transport_spotting = Ok(replay::transport_spotting_graph::Defines {
        base: 0,
        visibility_scale: 100_000,
        navy_scale: 100_000,
        navy_base: 0,
        radar_base: 0,
        decryption_base: 0,
        intel_minimum: 0,
        intel_maximum: 100_000,
    });
    assets.transport_chance = Ok(
        installed_content::unit_path::selected_transport_risk::ChanceDefines {
            spotting_weight: 100_000,
            damage_weight: 100_000,
            base: 10_000_000,
        },
    );
    let receiver = TransportMissionReceiver {
        task_force: (7_u64 << 32) | 61,
        mission_owner: Some(11),
        combined_detection: Some(0),
        detection_cache: Some([0, 0]),
        received_supply_ratio: Some(100_000),
        in_regions: Some(1),
        combat_references: Some(0),
        risk: Some(TransportReceiverRisk {
            region: Some(0),
            location: Some(1),
            ship_count: Some(0),
            stats: Some(TransportTaskForceStats {
                detection_138: Some(0),
                carrier_detection_160: Some(0),
                visibility_148: Some(0),
                visibility_150: Some(0),
            }),
            visibility: Some(0),
            detached_activity: Some(0),
            speed_base: Some(0),
            speed_modifier: Some(0),
            actual_terrain_speed: Some(0),
            coordination: Some(0),
            spotting_chance: Some(0),
            targeted_spotting_modifier: Some(0),
            night_spotting_chance: None,
            ..Default::default()
        }),
        ..Default::default()
    };
    let packet = TransportMissionInputs {
        sample_hour: hour,
        navies: Some(vec![]),
        receivers: Some(vec![receiver]),
        spotting_context: Some(TransportSpottingContext {
            sample_hour: hour,
            owners_complete: true,
            regions_complete: true,
            owners: vec![TransportSpottingOwner {
                owner_slot: 1,
                owner_raw_id: 11,
                target_owned_states: Some(0),
                radar: Some(TransportRadarCounts {
                    signed_count: 0,
                    counts: None,
                }),
                navy_intel: None,
            }],
            regions: vec![TransportSpottingRegion {
                region_id: 0,
                signed_province_count: Some(1),
                weather_detection: Some(None),
                ..Default::default()
            }],
            ..Default::default()
        }),
        air: Some(TransportAirInputs {
            sample_hour: hour,
            countries: Some(vec![
                TransportAirCountry {
                    slot: 0,
                    present: false,
                    ..Default::default()
                },
                TransportAirCountry {
                    slot: 1,
                    present: true,
                    country: Some(11),
                    regions: Some(vec![TransportAirRegion {
                        region: 0,
                        count: Some(0),
                        missions: Some(vec![]),
                    }]),
                    factors: None,
                },
            ]),
        }),
        ..Default::default()
    };
    put::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            transport_mission_inputs: Recorded::Value(Some(packet.clone())),
            ..Default::default()
        }],
    );
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 11,
            transport_convoys: Recorded::Value(Some(TransportConvoys {
                sample_hour: hour,
                first_variant: Some(Some(TransportConvoyVariant {
                    visibility: Some(0),
                    surface_detection: Some(0),
                    submarine_detection: Some(0),
                    speed: Some(0),
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        }],
    );
    put::<tables::WeatherProvinces>(
        &mut frame,
        vec![WeatherProvinceRow {
            id: 1,
            night_factor: Recorded::Value(0),
            ..Default::default()
        }],
    );
    // Native 50 percent damage with the 100 percent caller base gives Q15/2.
    // Empty air and zero radar/owned-state gates skip their unrelated bodies.
    assert_eq!(
        query_with_assets(&frame, &assets, |moves| graph::chance(
            moves, &receiver, 11, 1
        )),
        Ok(0x4000)
    );
    let mut failed = receiver;
    failed.risk.as_mut().unwrap().targeted_spotting_modifier = None;
    assert!(
        query_with_assets(&frame, &assets, |moves| graph::chance(
            moves, &failed, 11, 1
        ))
        .unwrap_err()
        .contains("targeted spotting modifier refused")
    );
    put::<tables::WeatherProvinces>(
        &mut frame,
        vec![WeatherProvinceRow {
            id: 1,
            night_factor: Recorded::Value(80_001),
            ..Default::default()
        }],
    );
    assert!(
        query_with_assets(&frame, &assets, |moves| graph::chance(
            moves, &receiver, 11, 1
        ))
        .unwrap_err()
        .contains("night spotting modifier refused")
    );
    let mut stale = packet;
    stale.spotting_context.as_mut().unwrap().sample_hour -= 1;
    put::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            transport_mission_inputs: Recorded::Value(Some(stale)),
            ..Default::default()
        }],
    );
    assert!(
        query_with_assets(&frame, &assets, |moves| graph::chance(
            moves, &receiver, 11, 1
        ))
        .unwrap_err()
        .contains("stale")
    );
}

fn packet(
    hour: i32,
    transfer: Option<UnitTransferInputs>,
    coastal: Option<bool>,
) -> Recorded<Option<UnitTransportInputs>> {
    Recorded::Value(Some(UnitTransportInputs {
        sample_hour: hour,
        weight: None,
        transfer,
        coastal_template: coastal,
    }))
}

#[test]
fn target_base_rejection_never_demands_absent_unit_or_nested_graph() {
    let frame = frame();
    assert_eq!(
        query(&frame, |moves| moves.transport_admission(
            CommonClass::Army,
            7,
            3,
            2
        )),
        Ok(Admission {
            allowed: false,
            reason: Some(law::Reason::NoTargetBase)
        })
    );
}

#[test]
fn current_hour_typed_packets_refuse_legacy_stale_and_independent_leaf_faults() {
    let mut frame = frame();
    let hour = frame.hour;
    put::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: 7,
            transport: packet(hour, Some(UnitTransferInputs::Absent), Some(false)),
            ..Default::default()
        }],
    );
    put::<tables::RailwayGunCounterInputs>(
        &mut frame,
        vec![RailwayGunCounterInputRow {
            gun: 7,
            transport: packet(hour - 1, Some(UnitTransferInputs::Absent), None),
            ..Default::default()
        }],
    );
    assert_eq!(
        query(&frame, |moves| moves
            .transport_unit(CommonClass::Army, 7)
            .map(|value| value.coastal_template)),
        Ok(Some(false))
    );
    assert!(
        query(&frame, |moves| moves
            .transport_unit(CommonClass::RailwayGun, 7)
            .map(|_| ()))
        .unwrap_err()
        .contains("stale")
    );
    put::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: 7,
            transport: Recorded::NotRecorded,
            ..Default::default()
        }],
    );
    assert!(
        query(&frame, |moves| moves
            .transport_unit(CommonClass::Army, 7)
            .map(|_| ()))
        .unwrap_err()
        .contains("not recorded")
    );
}

#[test]
fn coastal_army_and_nonzero_or_same_source_transfer_return_before_convoy_and_risk_demand() {
    let mut frame = frame();
    let hour = frame.hour;
    put::<tables::NavalBases>(
        &mut frame,
        vec![NavalBaseRow {
            province: 2,
            ..Default::default()
        }],
    );
    put::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: 7,
            transport: packet(hour, None, Some(true)),
            ..Default::default()
        }],
    );
    assert_eq!(
        query(&frame, |moves| moves.transport_admission(
            CommonClass::Army,
            7,
            3,
            2
        )),
        Ok(Admission {
            allowed: true,
            reason: None
        })
    );
    assert_eq!(
        query(&frame, |moves| moves.transport_admission(
            CommonClass::RailwayGun,
            7,
            3,
            2
        )),
        Ok(Admission {
            allowed: false,
            reason: Some(law::Reason::NoSourceBase)
        })
    );
    put::<tables::NavalBases>(
        &mut frame,
        vec![
            NavalBaseRow {
                province: 2,
                ..Default::default()
            },
            NavalBaseRow {
                province: 3,
                ..Default::default()
            },
        ],
    );
    for (status, source, reason) in [
        (-3, None, law::Reason::DuringCombat),
        (0, Some(3), law::Reason::AlreadyActive),
    ] {
        put::<tables::DivisionMovement>(
            &mut frame,
            vec![DivisionMovementRow {
                division: 7,
                transport: packet(
                    hour,
                    Some(UnitTransferInputs::Present {
                        transfer: u64::MAX,
                        status: Some(status),
                        source,
                    }),
                    None,
                ),
                ..Default::default()
            }],
        );
        assert_eq!(
            query(&frame, |moves| moves.transport_admission(
                CommonClass::Army,
                7,
                3,
                2
            )),
            Ok(Admission {
                allowed: false,
                reason: Some(reason)
            })
        );
    }
}

#[test]
fn railway_retreat_never_demands_commander_ban_and_empty_localisation_does_not_admit() {
    let frame = frame();
    let army = Army::default();
    for attacking in [false, true] {
        let unit = CommonUnit {
            class: CommonClass::RailwayGun,
            handle: 7,
            logical_owner_raw: 0,
            combat_count: 0,
            army: &army,
        };
        assert_eq!(
            query(&frame, |moves| moves.unit_retreat_ban(&unit, attacking)),
            Ok(false)
        );
    }
    for (localised, expected) in [("", "prior"), ("blocked", "priorblocked")] {
        let loc = [("TRANSPORT_ALREADY_ACTIVE".into(), localised.into())]
            .into_iter()
            .collect();
        let keys = BTreeMap::new();
        let text = CommonText {
            frame: &frame,
            loc: &loc,
            recorded_language: "",
            state_keys: &keys,
        };
        let mut body = "prior".to_owned();
        assert_eq!(
            query(&frame, |moves| moves.transport_reason(
                Admission {
                    allowed: false,
                    reason: Some(law::Reason::AlreadyActive)
                },
                &mut body,
                &text
            )),
            Ok(false)
        );
        assert_eq!(body, expected);
    }
}

#[test]
fn original_stock_uses_raw_equality_alias_priority_and_independent_alternate_count() {
    use replay::rows::{
        CountryAppearanceRow, CountryLinkRow, TransportConvoyCache, TransportConvoyReservation,
        TransportConvoys,
    };
    let mut frame = frame();
    let hour = frame.hour;
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: 12,
        canonical_slot: 1,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    let mut packet = TransportConvoys {
        sample_hour: hour,
        priority: Some(-2),
        unreserved: Some(-91),
        cache: Some(TransportConvoyCache {
            valid: 1,
            raw: Some(1_000_000),
        }),
        reservations: Some(vec![
            TransportConvoyReservation {
                priority: -2,
                quantity: -3,
            },
            TransportConvoyReservation {
                priority: -1,
                quantity: 100,
            },
        ]),
        ..Default::default()
    };
    for roster in [11, 12, 10] {
        put::<tables::RailwayGunCounterInputs>(
            &mut frame,
            vec![RailwayGunCounterInputRow {
                gun: 7,
                roster_owner: roster,
                logical_owner: 11,
                ..Default::default()
            }],
        );
        put::<tables::CountryAppearance>(
            &mut frame,
            vec![CountryAppearanceRow {
                country: 11,
                transport_convoys: Recorded::Value(Some(packet.clone())),
                ..Default::default()
            }],
        );
        assert_eq!(
            query(&frame, |moves| moves
                .transport_stock(CommonClass::RailwayGun, 7)),
            Ok(if roster == 10 { -91 } else { 13 })
        );
    }
    packet.priority = None;
    packet.cache = None;
    packet.reservations = None;
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 11,
            transport_convoys: Recorded::Value(Some(packet.clone())),
            ..Default::default()
        }],
    );
    assert_eq!(
        query(&frame, |moves| moves
            .transport_stock(CommonClass::RailwayGun, 7)),
        Ok(-91)
    );
    put::<tables::RailwayGunCounterInputs>(
        &mut frame,
        vec![RailwayGunCounterInputRow {
            gun: 7,
            roster_owner: 12,
            logical_owner: 11,
            ..Default::default()
        }],
    );
    assert!(
        query(&frame, |moves| moves
            .transport_stock(CommonClass::RailwayGun, 7))
        .unwrap_err()
        .contains("priority refused")
    );
    packet.sample_hour -= 1;
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 11,
            transport_convoys: Recorded::Value(Some(packet)),
            ..Default::default()
        }],
    );
    assert!(
        query(&frame, |moves| moves
            .transport_stock(CommonClass::RailwayGun, 7))
        .unwrap_err()
        .contains("stale")
    );
}

#[test]
fn transport_self_and_alias_access_return_before_missing_diplomacy_operands() {
    use replay::rows::CountryLinkRow;
    let mut frame = frame();
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: 12,
        canonical_slot: 1,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    for (receiver, target) in [(11, 11), (12, 11), (11, 12)] {
        assert_eq!(
            query(&frame, |moves| moves
                .common_transport_access(receiver, target)),
            Ok(true)
        );
    }
}

#[test]
fn not_enough_text_supplies_native_signed_free_and_need_arguments() {
    let frame = frame();
    let loc: installed_content::localisation::Localisation =
        [("TRANSPORT_NOT_ENOUGH".into(), "$FREE$ / $NEED$".into())]
            .into_iter()
            .collect();
    let keys = BTreeMap::new();
    let content = CommonText {
        frame: &frame,
        loc: &loc,
        recorded_language: "",
        state_keys: &keys,
    };
    let mut body = "prior ".to_owned();
    assert_eq!(
        query(&frame, |moves| moves.transport_reason(
            Admission {
                allowed: false,
                reason: Some(law::Reason::NotEnough {
                    available: -7,
                    required: 3
                })
            },
            &mut body,
            &content
        )),
        Ok(false)
    );
    assert_eq!(body, "prior -7 / 3");
}

#[test]
fn risk_speed_borrows_current_idle_owner_domain_and_refuses_stale_missing_or_reordered_regions() {
    use installed_content::unit_path::selected_transport_risk::Inputs as Risk;
    use replay::rows::{CountryCounterInputRow, CountryMovementInputs, RegionTransportSpeed};
    let mut frame = frame();
    let hour = frame.hour;
    put::<tables::RailwayGunCounterInputs>(
        &mut frame,
        vec![RailwayGunCounterInputRow {
            gun: 7,
            roster_owner: 11,
            logical_owner: 11,
            ordinary_path: vec![],
            ..Default::default()
        }],
    );
    let mut packet = CountryMovementInputs {
        sample_hour: hour,
        transport: Some(vec![
            RegionTransportSpeed {
                region: 0,
                speed: Some(30_000),
            },
            RegionTransportSpeed {
                region: 3,
                speed: Some(-70_000),
            },
        ]),
        ..Default::default()
    };
    let install = |frame: &mut replay::frame::DecodedFrame, packet| {
        put::<tables::CountryCounterInputs>(
            frame,
            vec![CountryCounterInputRow {
                country: 11,
                movement: Recorded::Value(Some(packet)),
                ..Default::default()
            }],
        );
    };
    install(&mut frame, packet.clone());
    let speed = |frame: &replay::frame::DecodedFrame, region| {
        query(frame, |moves| {
            RiskInputs { moves }.speed(Unit::RailwayGun(7), region)
        })
    };
    assert_eq!(speed(&frame, Some(0)), Ok(30_000));
    assert_eq!(speed(&frame, Some(3)), Ok(-70_000));
    assert!(
        speed(&frame, Some(4))
            .unwrap_err()
            .contains("outside captured domain")
    );
    assert!(
        speed(&frame, None)
            .unwrap_err()
            .contains("null previous region")
    );
    packet.sample_hour -= 1;
    install(&mut frame, packet.clone());
    assert!(speed(&frame, Some(0)).unwrap_err().contains("stale"));
    packet.sample_hour = hour;
    packet.transport.as_mut().unwrap().reverse();
    install(&mut frame, packet.clone());
    assert!(
        speed(&frame, Some(0))
            .unwrap_err()
            .contains("unique/increasing")
    );
    packet.transport = None;
    install(&mut frame, packet);
    assert!(
        speed(&frame, Some(0))
            .unwrap_err()
            .contains("domain refused")
    );
}
