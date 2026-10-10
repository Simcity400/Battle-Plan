//! Current-hour simulation operands for a freshly searched counter route.
use super::*;

/// Independent domains sampled at `sample_hour`. A consumer requires that hour
/// to equal its frame hour before using any domain. Missing inputs never fall
/// back to the legacy carried, stored-route operands.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FreshCounterEta {
    pub sample_hour: i32,
    /// Actual stored first-edge speed, used only for progress subtraction when
    /// the fresh route begins with that same edge.
    pub current_speed: Option<i64>,
    pub normal: Option<FreshNormalEta>,
    pub rail: Option<StrategicRedeploymentInputs>,
}
value_struct!(FreshCounterEta {
    sample_hour: i32,
    current_speed: Option<i64>,
    normal: Option<FreshNormalEta>,
    rail: Option<StrategicRedeploymentInputs>,
});

/// Prepared normal-speed graph at the division's actual simulation context,
/// plus every terrain contribution admitted by the live route domain. A failed
/// terrain acquisition withholds this whole domain, never a numerical zero.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FreshNormalEta {
    pub speed_modifier: i64,
    pub speed_base: i64,
    /// Native unreached organisation/fuel branches retain absence.
    pub speed_org: Option<i64>,
    pub speed_fuel: Option<i64>,
    pub fort_penalty: i64,
    pub snow_penalty: i64,
    pub river_adjustment: i64,
    pub river_leader: i64,
    pub terrain_speed: Vec<TerrainSpeed>,
}
value_struct!(FreshNormalEta {
    speed_modifier: i64,
    speed_base: i64,
    speed_org: Option<i64>,
    speed_fuel: Option<i64>,
    fort_penalty: i64,
    snow_penalty: i64,
    river_adjustment: i64,
    river_leader: i64,
    terrain_speed: Vec<TerrainSpeed>,
});

/// Owner-shared current-hour inputs. Present lists carry the complete demanded
/// key domain in unique increasing `target_slot` or `region` order; an absent
/// list or entry value is unavailable, not zero. Owners
/// retain the enclosing country row's key, while river targets are canonical
/// slots and transport keys are strategic-region IDs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryMovementInputs {
    pub sample_hour: i32,
    pub river: Option<Vec<ControllerRiverSpeed>>,
    pub transport: Option<Vec<RegionTransportSpeed>>,
    pub naval: Option<NavalPathInputs>,
    pub theatre: Option<TheatreControllerAreas>,
}
value_struct!(CountryMovementInputs {
    sample_hour: i32,
    river: Option<Vec<ControllerRiverSpeed>>,
    transport: Option<Vec<RegionTransportSpeed>>,
    naval: Option<NavalPathInputs>,
    theatre: Option<TheatreControllerAreas>,
});

/// A demanded roster owner's native navy slot and independent stored lists.
/// Foreign indices are physical manager slots, in native order with repeated
/// and zero occurrences retained. They are never country-link references.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalPathInputs {
    pub navy_slot: u32,
    pub present: bool,
    pub foreign_navy_slots: Option<Vec<u32>>,
    pub supply_hubs: Option<Vec<u32>>,
}
value_struct!(NavalPathInputs {
    navy_slot: u32,
    present: bool,
    foreign_navy_slots: Option<Vec<u32>>,
    supply_hubs: Option<Vec<u32>>,
});

/// Exact membership of the target's actual controller-area pointer in ALL
/// owned theatre lists. Provinces are positive, sorted and unique; the null
/// witness preserves a native null-area comparison independently of the list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TheatreControllerAreas {
    pub provinces: Vec<u32>,
    pub null_controller_area: bool,
}
value_struct!(TheatreControllerAreas {
    provinces: Vec<u32>,
    null_controller_area: bool,
});

/// One current-hour shared packet. Each demanded physical navy slot occurs
/// once, in unique increasing order. The reserved theatre domain remains
/// independent of a nearest-cache refusal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryNavalMovementInputs {
    pub sample_hour: i32,
    pub nearest_caches: Option<Vec<PhysicalNavyNearestCache>>,
    pub reserved_theatre: Option<TheatreControllerAreas>,
}
value_struct!(CountryNavalMovementInputs {
    sample_hour: i32,
    nearest_caches: Option<Vec<PhysicalNavyNearestCache>>,
    reserved_theatre: Option<TheatreControllerAreas>,
});

/// The native cache is indexed by province ID. A null entry is None, while a
/// nonnull authenticated naval base whose stored province is zero is Some(0).
/// An absent vector is a refusal specific to this valid physical navy.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PhysicalNavyNearestCache {
    pub slot: u32,
    pub present: bool,
    pub nearest_bases: Option<Vec<Option<u32>>>,
}
value_struct!(PhysicalNavyNearestCache {
    slot: u32,
    present: bool,
    nearest_bases: Option<Vec<Option<u32>>>,
});

/// Effective RIVER_CROSSING_FACTOR_AGAINST operand for a canonical controller
/// slot. Slot zero selects the native relation at zero and is a real key.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ControllerRiverSpeed {
    pub target_slot: u32,
    pub modifier: Option<i64>,
}
value_struct!(ControllerRiverSpeed {
    target_slot: u32,
    modifier: Option<i64>,
});

/// Raw native transport speed before the route caller's scale, keyed by the
/// source province's strategic-region ID rather than a runtime census slot.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegionTransportSpeed {
    pub region: u32,
    pub speed: Option<i64>,
}
value_struct!(RegionTransportSpeed {
    region: u32,
    speed: Option<i64>,
});

/// Selected source rail-info building. `building: None` is an observed null
/// building; the enclosing row's absent optional state is unreached or
/// unavailable. Recorded branch gates distinguish when it is demanded.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RailwayBuildingState {
    pub building: Option<RailwayBuildingHealth>,
}
value_struct!(RailwayBuildingState {
    building: Option<RailwayBuildingHealth>,
});

/// Native signed building values for directed departure health. Partial health
/// is required only on the reached damaged branch; preserve its raw sentinels.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RailwayBuildingHealth {
    pub level: i16,
    pub healthy: i16,
    pub partial: Option<i64>,
}
value_struct!(RailwayBuildingHealth {
    level: i16,
    healthy: i16,
    partial: Option<i64>,
});

/// Current-hour E733D0/E765F0 inputs, independent of sorted RailEdges.
/// Levels retain every signed +20 entry in native definition-neighbour order.
/// None in a domain is a counted capture refusal. Building-level None can also
/// be outside both gates; Some(None) observes the native null +18 pointer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RailwaySelectionInputs {
    pub levels: Option<Vec<i32>>,
    pub pair_count: Option<i32>,
    pub building_level: Option<Option<i16>>,
}
value_struct!(RailwaySelectionInputs {
    levels: Option<Vec<i32>>,
    pair_count: Option<i32>,
    building_level: Option<Option<i16>>,
});

opaque_value!(Option<FreshCounterEta>);
opaque_value!(Option<CountryMovementInputs>);
opaque_value!(Option<CountryNavalMovementInputs>);
opaque_value!(Option<RailwayBuildingState>);
opaque_value!(Option<RailwaySelectionInputs>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    fn round_trip<T: Value + std::fmt::Debug + PartialEq>(value: &T) -> Vec<u8> {
        let mut bytes = Vec::new();
        Value::encode(value, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(T::decode(&mut input).unwrap(), *value);
        input.finish().unwrap();
        bytes
    }

    fn fresh() -> FreshCounterEta {
        FreshCounterEta {
            sample_hour: i32::MIN,
            current_speed: Some(i64::MIN),
            normal: Some(FreshNormalEta {
                speed_modifier: i64::MIN,
                speed_base: i64::MAX,
                speed_org: Some(-1),
                speed_fuel: None,
                fort_penalty: 0,
                snow_penalty: -1,
                river_adjustment: i64::MAX,
                river_leader: i64::MIN,
                terrain_speed: vec![
                    TerrainSpeed {
                        terrain: 1,
                        modifier: 0,
                    },
                    TerrainSpeed {
                        terrain: i32::MAX,
                        modifier: i64::MAX,
                    },
                ],
            }),
            rail: Some(StrategicRedeploymentInputs {
                country_modifier: i64::MIN,
                state_modifier: Some(i64::MAX),
                friendly_state_modifier: None,
            }),
        }
    }

    #[test]
    fn fresh_operands_preserve_raw_widths_and_non_route_terrain_keys() {
        let bytes = round_trip(&fresh());
        assert_eq!(&bytes[..4], &i32::MIN.to_le_bytes());
        assert_eq!(bytes[4], 1);
        assert_eq!(&bytes[5..13], &i64::MIN.to_le_bytes());
        let health = RailwayBuildingHealth {
            level: i16::MIN,
            healthy: i16::MAX,
            partial: Some(i64::MIN),
        };
        let bytes = round_trip(&health);
        assert_eq!(&bytes[..5], &[0, 0x80, 0xff, 0x7f, 1]);
        assert_eq!(&bytes[5..], &i64::MIN.to_le_bytes());
    }

    #[test]
    fn one_unavailable_speed_domain_preserves_the_others() {
        for value in [
            FreshCounterEta {
                normal: None,
                ..fresh()
            },
            FreshCounterEta {
                rail: None,
                ..fresh()
            },
            FreshCounterEta {
                current_speed: None,
                ..fresh()
            },
        ] {
            round_trip(&value);
        }
        let river = vec![
            ControllerRiverSpeed {
                target_slot: 0,
                modifier: Some(0),
            },
            ControllerRiverSpeed {
                target_slot: 91,
                modifier: None,
            },
            ControllerRiverSpeed {
                target_slot: u32::MAX,
                modifier: Some(i64::MIN),
            },
        ];
        let transport = vec![RegionTransportSpeed {
            region: u32::MAX,
            speed: Some(i64::MAX),
        }];
        for value in [
            CountryMovementInputs {
                river: Some(river.clone()),
                transport: Some(transport.clone()),
                ..Default::default()
            },
            CountryMovementInputs {
                river: None,
                transport: Some(transport),
                ..Default::default()
            },
            CountryMovementInputs {
                river: Some(river),
                transport: None,
                ..Default::default()
            },
            CountryMovementInputs {
                river: Some(Vec::new()),
                transport: Some(Vec::new()),
                ..Default::default()
            },
        ] {
            round_trip(&value);
        }
    }

    #[test]
    fn historical_gap_refusal_null_building_and_zero_health_stay_distinct() {
        type Field = Recorded<Option<RailwayBuildingState>>;
        let historical = <Field as Value>::not_recorded().unwrap();
        assert_eq!(historical, Recorded::NotRecorded);
        assert_eq!(Value::recorded_bytes(&historical).unwrap(), 0);
        assert!(Value::encode(&historical, &mut Vec::new()).is_err());

        let refusal = Recorded::Value(None);
        let null = Recorded::Value(Some(RailwayBuildingState { building: None }));
        let zero = Recorded::Value(Some(RailwayBuildingState {
            building: Some(RailwayBuildingHealth {
                level: 0,
                healthy: 0,
                partial: None,
            }),
        }));
        let encoded = [
            round_trip::<Field>(&refusal),
            round_trip::<Field>(&null),
            round_trip::<Field>(&zero),
        ];
        assert_eq!(encoded[0], [0]);
        assert_eq!(encoded[1], [1, 0]);
        assert_eq!(encoded[2], [1, 1, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn malformed_fresh_payloads_refuse_instead_of_defaulting_domains() {
        let bytes = round_trip(&fresh());
        for end in 0..bytes.len() {
            assert!(FreshCounterEta::decode(&mut Decoder::new(&bytes[..end])).is_err());
        }
        let mut invalid_speed_tag = bytes;
        invalid_speed_tag[4] = 2;
        assert!(FreshCounterEta::decode(&mut Decoder::new(&invalid_speed_tag)).is_err());
        // Present river domain declaring more entries than its payload holds.
        assert!(
            CountryMovementInputs::decode(&mut Decoder::new(&[
                0, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff
            ]))
            .is_err()
        );
        assert!(CountryMovementInputs::decode(&mut Decoder::new(&[0, 0, 0, 0, 2, 0])).is_err());
        assert!(RailwayBuildingState::decode(&mut Decoder::new(&[2])).is_err());
    }

    fn naval() -> CountryNavalMovementInputs {
        CountryNavalMovementInputs {
            sample_hour: i32::MIN,
            nearest_caches: Some(vec![
                PhysicalNavyNearestCache {
                    slot: 0,
                    present: false,
                    nearest_bases: Some(Vec::new()),
                },
                PhysicalNavyNearestCache {
                    slot: u32::MAX,
                    present: true,
                    nearest_bases: Some(vec![None, Some(0), Some(u32::MAX)]),
                },
            ]),
            reserved_theatre: Some(TheatreControllerAreas {
                provinces: vec![1, u32::MAX],
                null_controller_area: true,
            }),
        }
    }

    #[test]
    fn physical_cache_null_zero_repeat_and_independent_domains_round_trip() {
        let bytes = round_trip(&naval());
        for end in 0..bytes.len() {
            assert!(CountryNavalMovementInputs::decode(&mut Decoder::new(&bytes[..end])).is_err());
        }
        round_trip(&CountryNavalMovementInputs {
            nearest_caches: None,
            ..naval()
        });
        round_trip(&CountryNavalMovementInputs {
            reserved_theatre: None,
            ..naval()
        });
        round_trip(&CountryMovementInputs {
            sample_hour: i32::MAX,
            river: None,
            transport: Some(Vec::new()),
            naval: Some(NavalPathInputs {
                navy_slot: 71,
                present: true,
                foreign_navy_slots: Some(vec![0, 3, 3, 0]),
                supply_hubs: None,
            }),
            theatre: Some(TheatreControllerAreas {
                provinces: vec![1, 71],
                null_controller_area: false,
            }),
        });
        let absent = round_trip(&Recorded::<Option<CountryMovementInputs>>::Value(None));
        let empty = round_trip(&Recorded::Value(Some(CountryMovementInputs::default())));
        assert_ne!(absent, empty);
    }

    #[test]
    fn reserved_shared_packet_changes_only_its_existing_table_suffix() {
        let previous = ReservedCountryNamingRow {
            tag: b"zero".to_vec(),
            ..Default::default()
        };
        check_suffix(
            TableId::ReservedCountryNaming,
            15,
            &previous,
            &ReservedCountryNamingRow {
                movement: Some(naval()).into(),
                ..previous.clone()
            },
        );
        let row = ReservedCountryNamingRow {
            movement: Some(naval()).into(),
            ..previous
        };
        let decoded = historical_prefix(TableId::ReservedCountryNaming, 15, &row);
        assert_eq!(decoded.movement, Recorded::NotRecorded);
        assert_eq!(decoded.tag, b"zero");
    }

    fn check_suffix<T: RowCodec + Fields + std::fmt::Debug>(
        table: TableId,
        index: usize,
        previous: &T,
        row: &T,
    ) {
        assert_eq!(table.field_count(), T::SCHEMA.len());
        assert_eq!(table.mask_width(), T::SCHEMA.len().div_ceil(8));
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(&T::decode_fields(row.key(), &mut input).unwrap(), row);
        input.finish().unwrap();
        bytes.clear();
        let delta = row.delta(Some(previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << index);
        encode_delta(table, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let delta = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(&delta.apply(Some(previous.clone())).unwrap(), row);
    }

    #[test]
    fn fresh_suffixes_round_trip_and_change_without_legacy_fields() {
        let previous = DivisionMovementRow {
            division: u64::MAX,
            numerator: i64::MIN,
            ..Default::default()
        };
        check_suffix(
            TableId::DivisionMovement,
            18,
            &previous,
            &DivisionMovementRow {
                fresh: Some(fresh()).into(),
                ..previous.clone()
            },
        );
        let previous = CountryCounterInputRow {
            forward_visibility: crate::rows::Recorded::Value(None),
            country: u32::MAX,
            ..Default::default()
        };
        check_suffix(
            TableId::CountryCounterInputs,
            13,
            &previous,
            &CountryCounterInputRow {
                movement: Some(CountryMovementInputs {
                    sample_hour: i32::MIN,
                    river: Some(vec![ControllerRiverSpeed {
                        target_slot: 0,
                        modifier: Some(0),
                    }]),
                    transport: Some(vec![RegionTransportSpeed {
                        region: 71,
                        speed: None,
                    }]),
                    ..Default::default()
                })
                .into(),
                ..previous.clone()
            },
        );
        let previous = StateRow {
            id: u32::MAX,
            movement_infrastructure: 918,
            ..Default::default()
        };
        for value in [0, i64::MIN, i64::MAX] {
            check_suffix(
                TableId::States,
                7,
                &previous,
                &StateRow {
                    railway_infrastructure: Some(value).into(),
                    ..previous.clone()
                },
            );
        }
        let previous = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: -1,
            ..Default::default()
        };
        for building in [
            None,
            Some(RailwayBuildingHealth {
                level: 1,
                healthy: 0,
                partial: Some(i64::MIN),
            }),
        ] {
            check_suffix(
                TableId::RailCooldowns,
                1,
                &previous,
                &RailCooldownRow {
                    building: Some(RailwayBuildingState { building }).into(),
                    ..previous.clone()
                },
            );
        }
    }

    fn historical_prefix<T: RowCodec + Fields + std::fmt::Debug>(
        table: TableId,
        index: usize,
        row: &T,
    ) -> T {
        let introduced = table
            .last_change()
            .appended
            .iter()
            .find(|field| field.index == index)
            .expect("fresh suffix has a compatibility entry")
            .introduced;
        let format = crate::format::FormatVersion::read(introduced - 1).unwrap();
        let fields = format.fields(table);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        for kind in &T::SCHEMA[..fields] {
            kind.decode(&mut input).unwrap();
        }
        let prefix = &bytes[..bytes.len() - input.remaining()];
        let mut input = Decoder::new(prefix);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = T::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(
            decoded.not_recorded_fields(),
            full_mask(T::SCHEMA.len()) ^ full_mask(fields)
        );
        assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
        assert!(decoded.encode_fields(&mut Vec::new()).is_err());

        let delta = row.delta(None).unwrap().unwrap();
        let mut input = Decoder::new(&delta.values);
        for kind in &T::SCHEMA[..fields] {
            kind.decode_change(&mut input).unwrap();
        }
        bytes.clear();
        table.encode_key(row.key(), 0, &mut bytes).unwrap();
        bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
        bytes.extend_from_slice(&delta.values[..delta.values.len() - input.remaining()]);
        let mut input = Decoder::new(&bytes);
        input.format = format.version();
        input.recorded_fields = fields;
        let delta = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(delta.apply::<T>(None).unwrap(), decoded);
        decoded
    }

    #[test]
    fn selected_railway_suffix_preserves_signed_native_domains_and_legacy_absence() {
        let previous = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: -1,
            building: Some(RailwayBuildingState { building: None }).into(),
            ..Default::default()
        };
        let input = RailwaySelectionInputs {
            levels: Some(vec![i32::MIN, 0, i32::MAX]),
            pair_count: Some(i32::MIN),
            building_level: Some(Some(i16::MIN)),
        };
        let row = RailCooldownRow {
            selection: Some(input.clone()).into(),
            ..previous.clone()
        };
        check_suffix(TableId::RailCooldowns, 2, &previous, &row);
        check_suffix(
            TableId::RailCooldowns,
            3,
            &row,
            &RailCooldownRow {
                selection_hour: Some(i32::MIN).into(),
                ..row.clone()
            },
        );
        let current = RailCooldownRow {
            selection_hour: Some(12).into(),
            ..row.clone()
        };
        check_suffix(
            TableId::RailCooldowns,
            3,
            &current,
            &RailCooldownRow {
                selection_hour: Some(13).into(),
                ..current.clone()
            },
        );
        let old = historical_prefix(TableId::RailCooldowns, 1, &row);
        assert_eq!(old.selection, Recorded::NotRecorded);
        assert_eq!(old.selection_hour, Recorded::NotRecorded);
        assert_eq!(old.building, Recorded::NotRecorded);
        assert_eq!(old.cooldown_ticks, -1);
        // All three suffixes share their introduction, so this selects the
        // same historical prefix rather than one containing `building`.
        let old = historical_prefix(TableId::RailCooldowns, 2, &row);
        assert_eq!(old.building, Recorded::NotRecorded);
        assert_eq!(old.selection, Recorded::NotRecorded);
        assert_eq!(old.selection_hour, Recorded::NotRecorded);
        let missing = round_trip(&Recorded::<Option<RailwaySelectionInputs>>::Value(None));
        let empty = round_trip(&Recorded::Value(Some(RailwaySelectionInputs {
            levels: Some(Vec::new()),
            pair_count: Some(0),
            building_level: Some(None),
        })));
        assert_ne!(missing, empty);
        let unqueried = round_trip(&RailwaySelectionInputs {
            building_level: None,
            ..input.clone()
        });
        let null = round_trip(&RailwaySelectionInputs {
            building_level: Some(None),
            ..input.clone()
        });
        let zero = round_trip(&RailwaySelectionInputs {
            building_level: Some(Some(0)),
            ..input
        });
        assert_ne!(unqueried, null);
        assert_ne!(null, zero);
    }

    #[test]
    fn older_snapshot_and_delta_prefixes_preserve_values_and_mark_fresh_gaps() {
        let row = DivisionMovementRow {
            division: u64::MAX,
            source: 71,
            target: u32::MAX,
            numerator: i64::MIN,
            retreat: u8::MAX,
            naval_transfer: u8::MAX,
            combat_count: u32::MAX,
            path_total: 3,
            path: vec![31, 31, u32::MAX],
            alternate_total: 3,
            alternate_path: vec![u32::MAX, 17, 17],
            combat_role: u8::MAX,
            combat_province: u32::MAX,
            map_entity: u8::MAX,
            withdrawal: Some(u8::MAX).into(),
            movement_priority: Some(i32::MIN).into(),
            fresh: Some(fresh()).into(),
            ..Default::default()
        };
        let decoded = historical_prefix(TableId::DivisionMovement, 18, &row);
        assert_eq!(
            decoded,
            DivisionMovementRow {
                withdrawal: Recorded::NotRecorded,
                movement_priority: Recorded::NotRecorded,
                order_transport: Recorded::NotRecorded,
                fresh: Recorded::NotRecorded,
                combat_references: Recorded::NotRecorded,
                selected: Recorded::NotRecorded,
                transport: Recorded::NotRecorded,
                ..row.clone()
            }
        );
        let published = crate::format::FormatVersion::read(99).unwrap();
        assert_eq!(published.fields(TableId::DivisionMovement), 15);
        assert_eq!(published.fields(TableId::DivisionMovement).div_ceil(8), 2);
        assert_eq!(
            decoded.not_recorded_fields(),
            full_mask(DivisionMovementRow::SCHEMA.len()) ^ full_mask(15)
        );
        let row = CountryCounterInputRow {
            forward_visibility: crate::rows::Recorded::Value(None),
            country: u32::MAX,
            first_division: u64::MAX,
            movement: Some(CountryMovementInputs {
                sample_hour: i32::MAX,
                river: Some(Vec::new()),
                transport: None,
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        let decoded = historical_prefix(TableId::CountryCounterInputs, 13, &row);
        assert_eq!(decoded.movement, Recorded::NotRecorded);
        assert_eq!(decoded.first_division, row.first_division);
        let row = StateRow {
            id: u32::MAX,
            controller: 71,
            movement_infrastructure: i64::MIN,
            railway_infrastructure: Some(0).into(),
            ..Default::default()
        };
        let decoded = historical_prefix(TableId::States, 7, &row);
        assert_eq!(decoded.railway_infrastructure, Recorded::NotRecorded);
        assert_eq!(decoded.controller, row.controller);
        assert_eq!(decoded.movement_infrastructure, row.movement_infrastructure);
        let row = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: i32::MIN,
            building: Some(RailwayBuildingState { building: None }).into(),
            selection: Default::default(),
            selection_hour: Default::default(),
        };
        let decoded = historical_prefix(TableId::RailCooldowns, 1, &row);
        assert_eq!(decoded.building, Recorded::NotRecorded);
        assert_eq!(decoded.cooldown_ticks, row.cooldown_ticks);
    }

    #[test]
    fn movement_idle_refused_and_observed_zero_profiles_have_distinct_encodings() {
        let mut idle = fresh();
        idle.current_speed = None;
        let mut zero = idle.clone();
        zero.current_speed = Some(0);
        let packets = [None, Some(idle), Some(zero)];
        let baseline = DivisionMovementRow {
            division: 71,
            ..Default::default()
        };
        let mut encoded = Vec::new();
        for packet in packets {
            let row = DivisionMovementRow {
                fresh: Recorded::Value(packet),
                ..baseline.clone()
            };
            let mut bytes = Vec::new();
            row.encode_fields(&mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                DivisionMovementRow::decode_fields(row.key(), &mut input).unwrap(),
                row
            );
            input.finish().unwrap();
            encoded.push(bytes);
        }
        assert_ne!(encoded[0], encoded[1]);
        assert_ne!(encoded[1], encoded[2]);
        assert_ne!(encoded[0], encoded[2]);
        let previous = DivisionMovementRow {
            fresh: Some(fresh()).into(),
            ..baseline
        };
        for packet in [
            None,
            Some(FreshCounterEta {
                current_speed: None,
                ..fresh()
            }),
        ] {
            check_suffix(
                TableId::DivisionMovement,
                18,
                &previous,
                &DivisionMovementRow {
                    fresh: Recorded::Value(packet),
                    ..previous.clone()
                },
            );
        }
    }

    #[test]
    fn published_eta_snapshot_and_delta_keep_all_eighteen_fields_and_bytes() {
        let row = DivisionEtaInputsRow {
            division: u64::MAX,
            sample_hour: i32::MIN,
            speed_modifier: i64::MIN,
            speed_base: i64::MAX,
            speed_org: Some(-1),
            speed_fuel: None,
            current_speed: -7,
            terrain_speed: vec![TerrainSpeed {
                terrain: 17,
                modifier: i64::MIN,
            }],
            fort_penalty: -2,
            snow_penalty: -3,
            river_adjustment: -4,
            river_leader: -5,
            class: u32::MAX,
            rail_country_modifier: Some(i64::MIN),
            rail_state_modifier: None,
            rail_friendly_state_modifier: Some(i64::MAX),
            leg_river_combined: vec![i64::MIN, 0, i64::MAX],
            leg_transport_speed: vec![-1, 0, 1],
            leg_rail_speed: vec![7, -7, 0],
        };
        // Accepted99 fields 5/14/15/16 are Opaque: each u32 byte-length
        // envelope precedes the vector's separate u32 element count. Keep
        // this golden independent of the row, Field and Value encoders.
        let mut expected = Vec::new();
        expected.extend_from_slice(&i64::MIN.to_le_bytes()); // 0: speed_modifier
        expected.extend_from_slice(&i64::MAX.to_le_bytes()); // 1: speed_base
        expected.push(1); // 2: speed_org = Some(-1)
        expected.extend_from_slice(&(-1_i64).to_le_bytes());
        expected.push(0); // 3: speed_fuel = None
        expected.extend_from_slice(&(-7_i64).to_le_bytes()); // 4: current_speed
        expected.extend_from_slice(&16_u32.to_le_bytes()); // 5: opaque payload length
        expected.extend_from_slice(&1_u32.to_le_bytes()); // terrain entry count
        expected.extend_from_slice(&17_i32.to_le_bytes());
        expected.extend_from_slice(&i64::MIN.to_le_bytes());
        for value in [-2_i64, -3, -4, -5] {
            // 6..9: fort, snow, river, leader
            expected.extend_from_slice(&value.to_le_bytes());
        }
        expected.extend_from_slice(&u32::MAX.to_le_bytes()); // 10: class
        expected.push(1); // 11: rail_country_modifier = Some(MIN)
        expected.extend_from_slice(&i64::MIN.to_le_bytes());
        expected.push(0); // 12: rail_state_modifier = None
        expected.push(1); // 13: rail_friendly_state_modifier = Some(MAX)
        expected.extend_from_slice(&i64::MAX.to_le_bytes());
        for values in [[i64::MIN, 0, i64::MAX], [-1, 0, 1], [7, -7, 0]] {
            expected.extend_from_slice(&28_u32.to_le_bytes()); // 14..16: opaque length
            expected.extend_from_slice(&3_u32.to_le_bytes()); // vector element count
            for value in values {
                expected.extend_from_slice(&value.to_le_bytes());
            }
        }
        expected.extend_from_slice(&i32::MIN.to_le_bytes()); // 17: sample_hour
        let mut actual = Vec::new();
        row.encode_fields(&mut actual).unwrap();
        assert_eq!(actual.len(), 209);
        for (offset, length, count) in [
            (34, 16_u32, 1_u32),
            (109, 28, 3),
            (141, 28, 3),
            (173, 28, 3),
        ] {
            assert_eq!(&actual[offset..offset + 4], &length.to_le_bytes());
            assert_eq!(&actual[offset + 4..offset + 8], &count.to_le_bytes());
        }
        assert_eq!(actual, expected);
        let format = crate::format::FormatVersion::read(99).unwrap();
        let fields = format.fields(TableId::DivisionEtaInputs);
        assert_eq!(fields, 18);
        assert_eq!(DivisionEtaInputsRow::SCHEMA.len(), fields);
        assert_eq!(TableId::DivisionEtaInputs.mask_width(), 3);
        let mut input = Decoder::new(&actual);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = DivisionEtaInputsRow::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded, row);
        assert_eq!(decoded.not_recorded_fields(), 0);
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.mask, full_mask(18));
        let mut wire = Vec::new();
        encode_delta(TableId::DivisionEtaInputs, &delta, 0, &mut wire).unwrap();
        assert_eq!(
            &wire[..13],
            &[
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0xff, 0xff, 0x03,
            ]
        );
        let mut input = Decoder::new(&wire);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = decode_delta(TableId::DivisionEtaInputs, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded.mask, full_mask(18));
        assert_eq!(decoded.apply::<DivisionEtaInputsRow>(None).unwrap(), row);
    }
}
