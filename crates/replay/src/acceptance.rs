//! Synthetic source states, faulting spool I/O, and indexed reads share no oracle logic.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    rc::Rc,
};

use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    branches::{BranchReason, BranchStart, BranchTracker, Gap, Parentage},
    container::{Replay, SeekResult},
    frame::{DecodedFrame, FrameDecoder, FrameEncoder, Metrics, TableUpdate, TableView},
    identity::{Identity, Roster, SessionStart, Verification},
    rows::{
        AirWingRow, ArmyOrderRow, CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionRow,
        FactionMemberRow, FactionRow, FleetRow, HumanControllerRow, LandCombatRow,
        NavalCombatParticipantRow, NavalCombatRow, PlayerRow, ProvinceRow, RowCodec, RowRef,
        ShipRow, StateRow, TableId, TableRows, TableSchema, TaskForceRow, WarRelationRow, tables,
    },
    spool::{
        ControlRecord, Record, Recovery, SegmentBackend, SegmentSink, SegmentedSpool,
        encode_record, recover_prefix,
    },
};

const CAMPAIGN: &[u8] = b"synthetic-campaign-native-values";
const HOURS: usize = 2_000;
// Native handles retain the type in the low word and the instance id in the high word.
const DIVISION_A: u64 = (0x8000_0041_u64 << 32) | 53;
const DIVISION_B: u64 = (0x8000_0041_u64 << 32) | 54;
const DIVISION_EDGE_KEYS: [u64; 2] = [0, u64::MAX];
const OPTIONAL_SCALAR_CASES: [Option<i64>; 4] = [None, Some(0), Some(i64::MIN), Some(i64::MAX)];
// Every ordered pair of scalar cases occurs once per cycle, including the wraparound.
const OPTIONAL_SCALAR_ORDER: [usize; 16] = [0, 0, 1, 0, 2, 0, 3, 1, 1, 2, 1, 3, 2, 2, 3, 3];
const DOMAIN_TABLES: [TableId; 182] = [
    TableId::WarRelations,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::Players,
    TableId::HumanControllers,
    TableId::LandCombats,
    TableId::Fleets,
    TableId::TaskForces,
    TableId::Ships,
    TableId::NavalCombats,
    TableId::NavalCombatParticipants,
    TableId::AirWings,
    TableId::ArmyOrders,
    TableId::Focus,
    TableId::CompletedFocuses,
    TableId::ResearchTechnologies,
    TableId::ResearchSlots,
    TableId::TechnologyDefinitions,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::ActiveIdeas,
    TableId::DecisionCounts,
    TableId::Decisions,
    TableId::DynamicModifiers,
    TableId::PreferredTactics,
    TableId::PreferredTacticOperands,
    TableId::TechSharingGroups,
    TableId::TechSharingMembers,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::PowerBalanceRanges,
    TableId::PowerBalanceOverrides,
    TableId::ProductionLines,
    TableId::Stockpiles,
    TableId::DivisionEquipment,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::VariantUpgrades,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
    TableId::CountryFuel,
    TableId::ResourceDefinitions,
    TableId::CountryResources,
    TableId::ResourceExchanges,
    TableId::PurchaseContracts,
    TableId::ProvinceSupply,
    TableId::HubMotorization,
    TableId::DisruptedSupply,
    TableId::ForeignHomebases,
    TableId::CountryExperience,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::StateBaseResources,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::OccupationSettings,
    TableId::StateOccupationLaws,
    TableId::StateGarrisons,
    TableId::StateAssignedManpower,
    TableId::CountryOccupationReserveManpower,
    TableId::StateResistance,
    TableId::CountryCollaboration,
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaders,
    TableId::UnitLeaderTraits,
    TableId::UnitLeaderTraitExperience,
    TableId::ActiveAbilities,
    TableId::Capitulation,
    TableId::Subjects,
    TableId::Autonomy,
    TableId::IntelligenceAgencies,
    TableId::OperativeMemberships,
    TableId::Cryptology,
    TableId::IntelNetworks,
    TableId::ScriptFlags,
    TableId::ScriptVariables,
    TableId::Operations,
    TableId::OperationAssignments,
    TableId::WingEquipment,
    TableId::AirBases,
    TableId::WingCumulative,
    TableId::AirRegionHistory,
    TableId::AirRegionDamage,
    TableId::AirAces,
    TableId::AirStatistics,
    TableId::SunkConvoys,
    TableId::RailCooldowns,
    TableId::RailEdges,
    TableId::LandTheatres,
    TableId::LandTheatreGroups,
    TableId::LandTheatreMembers,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::SunkShips,
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectBreakthroughs,
    TableId::Nukes,
    TableId::RaidCountries,
    TableId::Raids,
    TableId::PeaceConferences,
    TableId::WeatherLimits,
    TableId::WeatherRegions,
    TableId::WeatherProvinces,
    TableId::OperationTokens,
    TableId::LexicalTokens,
    TableId::NavalSupremacy,
    TableId::RadarCoverage,
    TableId::VolunteerHosts,
    TableId::TheatreAreas,
    TableId::StationedUnits,
    TableId::UnitIntel,
    TableId::AirBaseRegistrations,
    TableId::WorldTension,
    TableId::ProductionLicenses,
    TableId::DeploymentConveyors,
    TableId::DeploymentLines,
    TableId::DeploymentManpower,
    TableId::DeploymentEquipment,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
    TableId::ProductionLicenseRecords,
    TableId::OrdersGroups,
    TableId::OrdersGroupMembers,
    TableId::ArmyGroupMembers,
    TableId::IncomingDiplomaticActions,
    TableId::Scientists,
    TableId::ScientistSkills,
    TableId::RelationOpinions,
    TableId::RelationTuples,
    TableId::CountryIntel,
    TableId::ProvinceNames,
    TableId::LandCombatProgress,
    TableId::DivisionTemplates,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::StateFlags,
    TableId::DivisionEta,
    TableId::ProvinceVisibility,
    TableId::ArmyOrderPlans,
    TableId::OrdersGroupColors,
    TableId::Fronts,
    TableId::CountryAppearance,
    TableId::ArmyIconInputs,
    TableId::ShipModels,
    TableId::ProvinceUnitOrder,
    TableId::CounterLayers,
    TableId::CharacterPortraits,
    TableId::LeaderPortraits,
    TableId::WeatherPassState,
    TableId::WeatherProvincePasses,
    TableId::WeatherProvinceNeighbours,
    TableId::WeatherRegionPasses,
    TableId::CounterSettings,
    TableId::SelectedUnits,
    TableId::TaskForceCounterInputs,
    TableId::FleetCounterInputs,
    TableId::CountryCounterInputs,
    TableId::OrdersGroupCounterInputs,
    TableId::DivisionOrderCaches,
    TableId::LeaderPortraitPaths,
    TableId::RaidTargets,
    TableId::RaidProvinces,
    TableId::NavalBases,
    TableId::SupplyNodes,
    TableId::TrainInventory,
    TableId::LandCombatDetails,
    TableId::LandCombatDivisions,
    TableId::LandCombatModifiers,
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    TableId::NavalCombatResultDetails,
];
/// The fault campaign drives every table except the six the clock, campaign,
/// country, province and state fixtures build directly, so a new table cannot
/// be added without being exercised here.
const DIRECT_TABLES: [TableId; 7] = [
    TableId::Clock,
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Provinces,
    TableId::States,
    TableId::Divisions,
];

#[test]
fn the_fault_campaign_covers_every_table_exactly_once() {
    let mut covered: Vec<TableId> = DOMAIN_TABLES
        .into_iter()
        .chain(DIRECT_TABLES)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    covered.sort();
    assert_eq!(covered, TableId::ALL);
    assert_eq!(
        DOMAIN_TABLES.len() + DIRECT_TABLES.len(),
        TableId::ALL.len()
    );
}

const RAW_FLAG_CASES: [u8; 3] = [0, 2, 255];
const FACTION_A: u64 = (0x8000_0033_u64 << 32) | 75;

#[derive(Default)]
struct Disk {
    segments: BTreeMap<u32, Vec<u8>>,
    /// A partial write succeeds up to this many bytes; its next write fails.
    fail_after: Option<usize>,
}

#[derive(Clone, Default)]
struct MemoryBackend(Rc<RefCell<Disk>>);

struct MemorySink {
    disk: Rc<RefCell<Disk>>,
    index: u32,
}

impl SegmentBackend for MemoryBackend {
    type Sink = MemorySink;

    fn create(&mut self, index: u32) -> io::Result<Self::Sink> {
        assert!(
            self.0
                .borrow_mut()
                .segments
                .insert(index, Vec::new())
                .is_none()
        );
        Ok(MemorySink {
            disk: self.0.clone(),
            index,
        })
    }
}

impl Write for MemorySink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if disk.fail_after == Some(0) {
            disk.fail_after = None;
            return Err(io::Error::other("injected disk write failure"));
        }
        let length = disk
            .fail_after
            .map_or(bytes.len(), |left| bytes.len().min(left));
        if let Some(left) = &mut disk.fail_after {
            *left -= length;
        }
        disk.segments
            .get_mut(&self.index)
            .unwrap()
            .extend_from_slice(&bytes[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SegmentSink for MemorySink {
    fn sync_all(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: 100 + id as u32,
        process_start_time: u64::MAX - id,
        qpc_frequency: 10_000_000,
        sample_boundary: crate::identity::SampleBoundary::EndOfHour,
        identity: Identity {
            game_version: "synthetic build".into(),
            exe_hash: [11; 32],
            exe_size: 123,
            vanilla_hash: [12; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: format!("test-session-{id}"),
            collector_hash: [id as u8; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

fn random(index: usize, salt: u64) -> u64 {
    let mut value = (index as u64)
        .wrapping_add(salt)
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The new map domains share fault scheduling, but each source keeps its native
/// key shape, scalar widths and ordered arrays independent of the codec.
fn push_map<S: TableSchema>(tables: &mut [TableView], row: S::Row) {
    let TableView::Complete(rows) = &mut tables[S::ID.index()] else {
        unreachable!()
    };
    let rows = rows.rows_mut::<S>().unwrap();
    rows.push(row);
    rows.sort_unstable_by_key(RowCodec::key);
}

fn add_map_rows(index: usize, tables: &mut [TableView]) {
    for ordinal in 0..3 {
        if ordinal == 2 && index % 17 >= 5 {
            continue;
        }
        let tick = index + ordinal;
        let id = [0, 0x8000_0041_0000_0035, u64::MAX][ordinal];
        let country = [0, 7, u32::MAX][(tick / 3) % 3];
        let scalar = |offset: usize| match (tick + offset) % 31 {
            0 => i64::MIN,
            1 => i64::MAX,
            value => (1_i64 << 53) + value as i64,
        };
        let handles = |offset: usize| match (tick + offset) % 4 {
            0 => Vec::new(),
            1 => vec![0, u64::MAX, DIVISION_A],
            2 => vec![DIVISION_A, 0, u64::MAX],
            _ => vec![u64::MAX, u64::MAX],
        };
        let geometry = |offset: usize| match (tick + offset) % 4 {
            0 => Vec::new(),
            1 => vec![0, u32::MAX, 7],
            2 => vec![7, u32::MAX, 0],
            _ => vec![u32::MAX, u32::MAX],
        };

        push_map::<tables::Players>(
            tables,
            PlayerRow {
                id: [0, i32::MAX, i32::MIN][ordinal],
                country,
                flags: RAW_FLAG_CASES[tick % RAW_FLAG_CASES.len()],
                user: vec![0xff, 0, (tick % 251) as u8],
                name: match (tick / 5) % 3 {
                    0 => Vec::new(),
                    1 => b"Duplicate name".to_vec(),
                    _ => vec![0, 0xff, (tick % 251) as u8],
                },
            },
        );
        push_map::<tables::HumanControllers>(
            tables,
            HumanControllerRow {
                country: [0, 7, u32::MAX][ordinal],
                count: [i8::MIN, -1, 0, i8::MAX][tick % 4],
            },
        );
        push_map::<tables::LandCombats>(
            tables,
            LandCombatRow {
                id,
                province: country,
                duration: [i32::MIN, i32::MAX, tick as i32][tick % 3],
                day: [i32::MIN, -1, i32::MAX][(tick + 1) % 3],
                attackers: handles(0),
                defenders: handles(1),
            },
        );
        push_map::<tables::Fleets>(
            tables,
            FleetRow {
                id,
                owner: country,
                home_base: [0, u32::MAX, tick as u32][tick % 3],
                home_base_flag: [0, u8::MAX, 1][tick % 3],
            },
        );
        push_map::<tables::TaskForces>(
            tables,
            TaskForceRow {
                id,
                fleet: [0, u64::MAX, FACTION_A][tick % 3],
                roster_owner: country,
                logical_owner: [0, 2, u32::MAX][tick % 3],
                expeditionary_owner: [0, 19, u32::MAX][(tick + 1) % 3],
                province: tick as u32,
                mission: [0, u32::MAX, 7][tick % 3],
            },
        );
        push_map::<tables::Ships>(
            tables,
            ShipRow {
                id,
                task_force: [0, u64::MAX, FACTION_A][tick % 3],
                strength: scalar(0),
                organisation: scalar(1),
                experience: scalar(2),
                category: [0, u32::MAX, tick as u32][tick % 3],
                sunk_convoys: tick as i32,
                class_id: [i32::MIN, 0, i32::MAX][tick % 3],
            },
        );
        push_map::<tables::NavalCombats>(
            tables,
            NavalCombatRow {
                id,
                province: country,
                attacker_positioning: scalar(0),
                defender_positioning: scalar(1),
                attacker_anti_air: scalar(2),
                defender_anti_air: scalar(3),
            },
        );
        push_map::<tables::NavalCombatParticipants>(
            tables,
            NavalCombatParticipantRow {
                combat: if ordinal == 2 {
                    u64::MAX
                } else {
                    0x8000_0041_0000_0061
                },
                side: (ordinal % 2) as u8,
                battle_slot: u32::MAX,
                ship: [0, u64::MAX, DIVISION_A][tick % 3],
                owner: country,
                state: [0, u32::MAX, tick as u32][tick % 3],
            },
        );
        push_map::<tables::AirWings>(
            tables,
            AirWingRow {
                id,
                owner: country,
                air_base: [0, u64::MAX, FACTION_A][tick % 3],
                base_override: [u64::MAX, FACTION_A, 0][tick % 3],
                country_override: [i32::MIN, -1, i32::MAX][tick % 3],
                planes: [i32::MIN, 0, i32::MAX][tick % 3],
                effective_planes: [i32::MAX, i32::MIN, 0][tick % 3],
                mission_mask: [0, u32::MAX, 0x8000_0001][tick % 3],
                mission_region: [None, Some(0), Some(u32::MAX)][tick % 3],
            },
        );
        push_map::<tables::ArmyOrders>(
            tables,
            ArmyOrderRow {
                owner_group: if ordinal == 2 {
                    u64::MAX
                } else {
                    0x8000_0041_0000_0051
                },
                instance: [0, 7, u32::MAX][ordinal],
                country,
                order_type: [i32::MIN, 0, i32::MAX][tick % 3],
                active: RAW_FLAG_CASES[tick % RAW_FLAG_CASES.len()],
                paradrop_target: [0, u32::MAX, tick as u32][tick % 3],
                path: geometry(0),
                target_states: geometry(1),
            },
        );
    }
}

/// Independent direct reads: no previous rows, field masks, or format reader participate.
fn source(index: usize, branch: BranchId, hour: Hour, signal: i64) -> DecodedFrame {
    let optional_scalar = |offset: usize| {
        OPTIONAL_SCALAR_CASES[OPTIONAL_SCALAR_ORDER[(index + offset) % OPTIONAL_SCALAR_ORDER.len()]]
    };
    let country = |id, canonical_slot| CountryRow {
        id,
        canonical_slot,
        tag: if id == 7 {
            b"GER".to_vec()
        } else {
            b"TAG".to_vec()
        },
        name: vec![0xff, id as u8, 0, b'N'],
        political_power_raw: optional_scalar(0),
        command_power_raw: -((index / 7) as i64) * 100_000,
        stability_raw: 78_901,
        war_support_raw: 54_321,
        convoys_raw: optional_scalar(5),
    };
    let mut countries = vec![country(7, 1), country(19, 3)];
    let mut links = vec![
        CountryLinkRow {
            id: 2,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 7,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 19,
            canonical_slot: 3,
        },
    ];
    if index % 13 < 7 {
        countries.push(country(29, 5));
        links.push(CountryLinkRow {
            id: 29,
            canonical_slot: 5,
        });
    }
    let mut provinces: Vec<_> = (1..=8)
        .map(|id| ProvinceRow {
            id,
            state: id / 3,
            owner: 2,
            controller: if id == 1 {
                if index.is_multiple_of(2) { 7 } else { 2 }
            } else {
                [0, 2, 19][(index / 9 + id as usize) % 3]
            },
            victory_points: [i32::MIN, -1, 0, i32::MAX][(index + id as usize) % 4],
        })
        .collect();
    if index % 17 < 5 {
        provinces.push(ProvinceRow {
            id: 9,
            state: u32::MAX,
            owner: 0,
            controller: 0,
            victory_points: i32::MAX,
        });
    }
    let mut states: Vec<_> = (0..=2)
        .map(|id| StateRow {
            id,
            owner: [2, 19][(index / 37 + id as usize) % 2],
            controller: [0, 7, 19][(index / 5 + id as usize) % 3],
        })
        .collect();
    if index % 17 < 5 {
        states.push(StateRow {
            id: u32::MAX,
            owner: 0,
            controller: 0,
        });
    }
    let native_value = |tick: usize| match tick % 31 {
        0 => i64::MIN,
        1 => i64::MAX,
        _ => (1_i64 << 42) - tick as i64,
    };
    let division = |id, ordinal: usize| {
        DivisionRow {
            id,
            roster_owner: [2, 19][(index / 101 + ordinal) % 2],
            logical_owner: [7, 19][(index / 79 + ordinal) % 2],
            expeditionary_owner: [0, 2, 19][(index / 83 + ordinal) % 3],
            province: ((index / 11 + ordinal) % 8 + 1) as u32,
            template: ((0x8000_0029 + (index / 127 + ordinal) as u64 % 2) << 32) | 91,
            // Alternating ticks isolate the HP and organisation field masks.
            hit_points: native_value(index / 2),
            organisation: native_value(index.div_ceil(2)),
            // Zero is the engine's own unassigned division, and it must reach
            // the codec as a real value alongside full 64-bit group handles.
            orders_group: [0, (57_u64 << 32) | 53, (u32::MAX as u64) << 32 | 53]
                [(index / 23 + ordinal) % 3],
        }
    };
    let mut divisions = vec![division(DIVISION_A, 0), division(DIVISION_B, 1)];
    if index % 17 < 5 {
        // Codec boundary keys exercise a zero first key and the full unsigned key range.
        divisions.insert(0, division(DIVISION_EDGE_KEYS[0], 2));
        divisions.push(division(DIVISION_EDGE_KEYS[1], 3));
    }
    let relation = |first_country, second_country, offset: usize| WarRelationRow {
        first_country,
        second_country,
        start_hour: [i32::MIN, hour, i32::MAX][(index / 7 + offset) % 3],
        end_hour: [i32::MAX, 0, i32::MIN][(index / 11 + offset) % 3],
        cancel: RAW_FLAG_CASES[(index + offset) % RAW_FLAG_CASES.len()],
        first_casualties: native_value(index + offset),
        second_casualties: native_value(index + offset + 1),
        first_unknown_casualties: native_value(index + offset + 2),
        second_unknown_casualties: native_value(index + offset + 3),
        first_was_instigator: RAW_FLAG_CASES[(index / 3 + offset) % RAW_FLAG_CASES.len()],
    };
    // Direction is part of the key: both orders must remain separate rows.
    let mut relations = vec![relation(7, 19, 0), relation(19, 7, 1)];
    if index % 17 < 5 {
        relations.insert(0, relation(0, u32::MAX, 2));
        relations.push(relation(u32::MAX, 0, 3));
    }
    let mut factions = vec![
        FactionRow {
            id: 0,
            name: b"Synthetic faction".to_vec(),
        },
        FactionRow {
            id: FACTION_A,
            name: match (index / 11) % 3 {
                0 => Vec::new(),
                1 => vec![0xff, 0, b'F'],
                _ => b"Renamed faction".to_vec(),
            },
        },
    ];
    if index % 17 < 5 {
        factions.push(FactionRow {
            id: u64::MAX,
            name: b"Temporary faction".to_vec(),
        });
    }
    let mut members = vec![
        FactionMemberRow {
            country: 7,
            faction: FACTION_A,
            ordinal: ((index / 3) % 2) as u32,
        },
        FactionMemberRow {
            country: 19,
            faction: FACTION_A,
            ordinal: (1 - (index / 3) % 2) as u32,
        },
    ];
    if index % 13 < 7 {
        members.push(FactionMemberRow {
            country: 29,
            faction: if index % 17 < 5 { u64::MAX } else { 0 },
            ordinal: 0,
        });
    }
    let initial_tables = [
        TableView::Complete(TableRows::Clock(vec![ClockRow {
            hour,
            play_time: (1_u64 << 40) + index as u64,
            play_session_started_at: signal,
            seed: i32::MIN,
            speed: (index % 5) as i32,
            // Counters advance most hours, hold on others, and go absent on
            // every seventh hour so present/absent transitions are exercised.
            debug_current_ref_id: (index % 7 != 3).then(|| u32::MAX - index as u32),
            unit: (index % 7 != 3).then_some(4012 + index as i32 / 3),
            order_index: (index % 7 != 3).then_some(i32::MAX - index as i32),
            front_index: (index % 7 != 3).then_some(8230),
            theatre_index: (index % 7 != 3).then_some(112),
            theater_group_index: (index % 7 != 3).then_some(924),
            military_deployment_line_index: (index % 7 != 3).then_some(7841),
            military_deployment_conveyor_index: (index % 7 != 3).then_some(744),
            equipment_variant_index: (index % 7 != 3).then_some(6295),
            country_leader_index: (index % 7 != 3).then_some(1),
            navy_id: (index % 7 != 3).then_some(8263),
            land_combat_id: (index % 7 != 3).then_some(15_359),
            multiplayer_random_count: (index % 7 != 3)
                .then_some(i32::MIN.wrapping_add(index as i32 * 997)),
        }])),
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.to_vec(),
        }])),
        TableView::Complete(TableRows::Countries(countries)),
        TableView::Complete(TableRows::CountryLinks(links)),
        TableView::Complete(TableRows::Provinces(provinces)),
        TableView::Complete(TableRows::States(states)),
        TableView::Complete(TableRows::Divisions(divisions)),
        TableView::Complete(TableRows::WarRelations(relations)),
        TableView::Complete(TableRows::Factions(factions)),
        TableView::Complete(TableRows::FactionMembers(members)),
    ];
    let mut tables =
        std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
    for (destination, initial) in tables.iter_mut().zip(initial_tables) {
        *destination = initial;
    }
    add_map_rows(index, &mut tables);
    for table in TableId::ALL.into_iter().skip(20) {
        tables[table.index()] =
            TableView::Complete(crate::rows::sample_extended_table(table, index).unwrap());
    }
    for table in TableId::ALL.into_iter().skip(2) {
        let draw = random(index, 91 + table.index() as u64);
        let force_partial = matches!(index, 0 | 38 | 758 | 830)
            && (table.index() >= TableId::Players.index()
                || matches!(
                    table,
                    TableId::Countries
                        | TableId::States
                        | TableId::Divisions
                        | TableId::WarRelations
                        | TableId::Factions
                        | TableId::FactionMembers
                ));
        if draw.is_multiple_of(47) || (index == 780 && table == TableId::Provinces) {
            tables[table.index()] = TableView::Unavailable {
                reason: format!("synthetic pointer unavailable for {table:?} at {hour}"),
            };
        } else if draw.is_multiple_of(43) || force_partial {
            let TableView::Complete(rows) = &tables[table.index()] else {
                unreachable!()
            };
            tables[table.index()] = TableView::Truncated {
                rows: {
                    let mut partial = rows.clone();
                    partial.truncate(if table.key_bits() == 0 { 0 } else { 1 });
                    partial
                },
                missing: (rows.len() - if table.key_bits() == 0 { 0 } else { 1 }) as u32,
            };
        }
    }
    DecodedFrame {
        branch,
        hour,
        tables,
        metrics: Metrics {
            total_ns: 100_000 + index as u64,
            table_ns: std::array::from_fn(|table| table as u64 + 1),
            pool_bytes: 1 << 25,
            dropped_frames: (index / 61) as u64,
        },
    }
}

/// Protect scenario boundaries and known parent witnesses from random queue drops.
fn may_drop(index: usize) -> bool {
    ![
        0_usize, 37, 38, 399, 448, 758, 780, 828, 830, 880, 929, 930, 999, 1000, 1499, 1500, 1999,
    ]
    .iter()
    .any(|anchor| index.abs_diff(*anchor) <= 2)
        && random(index, 7).is_multiple_of(61)
}

#[derive(Default)]
struct DomainCoverage {
    changed_fields: u64,
    changes: usize,
    insertions: usize,
    deletions: usize,
    unavailable: usize,
    truncated: usize,
    partial_keyframes: usize,
}

#[derive(Default)]
struct Coverage {
    keyframes: usize,
    periodic_keyframes: usize,
    delta_tables: usize,
    deletions: usize,
    insertions: usize,
    unchanged_tables: usize,
    unavailable_tables: usize,
    truncated_tables: usize,
    partial_keyframes: usize,
    state_delta_tables: usize,
    state_unavailable_tables: usize,
    state_truncated_tables: usize,
    state_partial_keyframes: usize,
    division_hp_only_deltas: usize,
    division_organisation_only_deltas: usize,
    division_deletions: usize,
    division_insertions: usize,
    division_unavailable_tables: usize,
    division_truncated_tables: usize,
    division_partial_keyframes: usize,
    division_edge_snapshots: [bool; 2],
    division_edge_insertions: [bool; 2],
    division_edge_deletions: [bool; 2],
    country_optional_transitions: [[bool; 4]; 4],
    domains: Vec<DomainCoverage>,
    war_field_changes: u64,
    war_flag_values: [[bool; RAW_FLAG_CASES.len()]; 2],
    war_casualty_extremes: [[bool; 2]; 4],
    member_ordinal_only_deltas: usize,
    member_faction_changes: usize,
}

struct Campaign {
    segments: Vec<(u32, Recovery)>,
    expected: BTreeMap<(BranchId, Hour), DecodedFrame>,
    missing: BTreeMap<(BranchId, Hour), String>,
    starts: Vec<BranchStart>,
    coverage: Coverage,
}

fn synthesize() -> Campaign {
    let backend = MemoryBackend::default();
    let disk = backend.0.clone();
    let mut spool = SegmentedSpool::new(backend);
    let mut tracker = BranchTracker::new();
    let mut encoder = FrameEncoder::new();
    let mut expected = BTreeMap::new();
    let mut missing = BTreeMap::new();
    let mut starts = Vec::new();
    let mut pending_gaps = Vec::new();
    let mut coverage = Coverage {
        domains: (0..DOMAIN_TABLES.len())
            .map(|_| DomainCoverage::default())
            .collect(),
        ..Coverage::default()
    };
    let mut current_session = session(1);
    let mut hour = 10_000;
    let mut signal = 1_000;
    let mut last_keyframe = None;
    let mut failed_length = None;
    let mut final_frame_bytes = 0;
    let mut final_hour = (0, 0);

    for index in 0..HOURS {
        if matches!(index, 0 | 1000 | 1500) {
            if index != 0 {
                spool
                    .append(&Record::Control(ControlRecord::Detach {
                        session: current_session.id,
                        last_hour: Some(hour),
                    }))
                    .unwrap();
                spool.finish_segment().unwrap();
            }
            current_session = session(if index == 0 {
                1
            } else if index == 1000 {
                2
            } else {
                3
            });
            tracker.begin_session(current_session.id).unwrap();
            if index == 1500 {
                hour = 10_449;
            }
        }
        match index {
            780 => {
                hour = 10_400;
                signal += 1;
            } // Earlier recorded hour, one possible parent.
            830 => {
                hour = 10_449;
                signal += 1;
            } // Repeat last sampled hour, ambiguous parent.
            880 => {
                hour = 12_000;
                signal += 1;
            } // Later save outside any recorded branch.
            930 => {
                signal += 1;
            } // Ordinary next-hour sample; only the load signal exposes it.
            _ => {}
        }
        let loaded_hour = hour;
        hour += 1;
        if may_drop(index) {
            let branch = tracker.current_branch().unwrap();
            let reason = "bounded queue overflow".to_owned();
            pending_gaps.push(Gap {
                branch,
                first_hour: hour,
                last_hour: hour,
                reason: reason.clone(),
            });
            assert!(missing.insert((branch, hour), reason).is_none());
            continue;
        }
        let observed = tracker
            .observe(Some(loaded_hour), hour, Some(signal), &pending_gaps)
            .unwrap();
        if let Some(start) = &observed.start {
            // Calculate ancestry from direct successfully written witnesses, not the tracker.
            let parents: Vec<_> = expected
                .keys()
                .filter_map(|&(branch, recorded)| (recorded == loaded_hour).then_some(branch))
                .collect();
            let parent = match parents.as_slice() {
                [branch] => Parentage::Probable {
                    branch: *branch,
                    hour: loaded_hour,
                },
                _ => Parentage::Unknown,
            };
            assert_eq!(start.parent, parent);
            starts.push(start.clone());
        }
        let source = source(index, observed.branch, hour, signal);
        let force = !spool.is_open() || observed.start.is_some();
        let periodic = !force
            && last_keyframe.is_some_and(|previous| {
                i64::from(hour) - i64::from(previous) >= i64::from(KEYFRAME_INTERVAL)
            });
        let keyframe = force || periodic;
        let frame = encoder.encode(source.clone(), force).unwrap();
        assert_eq!(
            frame.keyframe, keyframe,
            "keyframe cadence at source {index}"
        );
        if keyframe {
            last_keyframe = Some(hour);
        }
        coverage.keyframes += usize::from(keyframe);
        coverage.periodic_keyframes += usize::from(periodic);
        for (table, update) in TableId::ALL.into_iter().zip(&frame.tables) {
            let domain_index = DOMAIN_TABLES.iter().position(|id| *id == table);
            if table == TableId::WarRelations
                && let TableView::Complete(rows) = &source.tables[table.index()]
            {
                for row in rows {
                    let RowRef::WarRelation(row) = row else {
                        unreachable!()
                    };
                    for (field, value) in [row.cancel, row.first_was_instigator]
                        .into_iter()
                        .enumerate()
                    {
                        let case = RAW_FLAG_CASES.iter().position(|raw| *raw == value).unwrap();
                        coverage.war_flag_values[field][case] = true;
                    }
                    for (field, value) in [
                        row.first_casualties,
                        row.second_casualties,
                        row.first_unknown_casualties,
                        row.second_unknown_casualties,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        coverage.war_casualty_extremes[field][0] |= value == i64::MIN;
                        coverage.war_casualty_extremes[field][1] |= value == i64::MAX;
                    }
                }
            }
            match update {
                TableUpdate::Delta {
                    baseline,
                    changed,
                    deleted,
                } => {
                    coverage.delta_tables += 1;
                    coverage.state_delta_tables += usize::from(table == TableId::States);
                    coverage.deletions += deleted.len();
                    coverage.insertions += changed
                        .iter()
                        .filter(|row| row.mask == table.full_mask())
                        .count();
                    if let Some(domain_index) = domain_index {
                        let previous: &DecodedFrame = &expected[&(observed.branch, *baseline)];
                        let TableView::Complete(before) = &previous.tables[table.index()] else {
                            panic!("relation delta must have a complete direct source baseline");
                        };
                        coverage.domains[domain_index].deletions += deleted.len();
                        for row in changed {
                            if before.iter().any(|old| old.key() == row.key) {
                                coverage.domains[domain_index].changes += 1;
                                coverage.domains[domain_index].changed_fields |= row.mask;
                                if table == TableId::WarRelations {
                                    coverage.war_field_changes |= row.mask;
                                }
                                if table == TableId::FactionMembers {
                                    coverage.member_ordinal_only_deltas +=
                                        usize::from(row.mask == 1 << 1);
                                    coverage.member_faction_changes +=
                                        usize::from(row.mask & 1 != 0);
                                }
                            } else {
                                coverage.domains[domain_index].insertions += 1;
                            }
                        }
                    }
                    if table == TableId::Divisions {
                        coverage.division_hp_only_deltas +=
                            changed.iter().filter(|row| row.mask == 1 << 5).count();
                        coverage.division_organisation_only_deltas +=
                            changed.iter().filter(|row| row.mask == 1 << 6).count();
                        coverage.division_deletions += deleted.len();
                        coverage.division_insertions += changed
                            .iter()
                            .filter(|row| row.mask == table.full_mask())
                            .count();
                        for (edge, key) in DIVISION_EDGE_KEYS.into_iter().enumerate() {
                            coverage.division_edge_insertions[edge] |= changed.iter().any(|row| {
                                row.key == u128::from(key) && row.mask == table.full_mask()
                            });
                            coverage.division_edge_deletions[edge] |=
                                deleted.contains(&u128::from(key));
                        }
                    }
                    if table == TableId::Countries {
                        let previous: &DecodedFrame = &expected[&(observed.branch, *baseline)];
                        let (TableView::Complete(before), TableView::Complete(after)) = (
                            &previous.tables[table.index()],
                            &source.tables[table.index()],
                        ) else {
                            panic!("country delta must have complete direct source baselines");
                        };
                        let (Some(RowRef::Country(before)), Some(RowRef::Country(after))) =
                            (before.first(), after.first())
                        else {
                            unreachable!()
                        };
                        for (old, new) in [
                            (before.political_power_raw, after.political_power_raw),
                            (before.convoys_raw, after.convoys_raw),
                        ] {
                            let old = OPTIONAL_SCALAR_CASES
                                .iter()
                                .position(|value| *value == old)
                                .unwrap();
                            let new = OPTIONAL_SCALAR_CASES
                                .iter()
                                .position(|value| *value == new)
                                .unwrap();
                            coverage.country_optional_transitions[old][new] = true;
                        }
                    }
                }
                TableUpdate::Unchanged { .. } => coverage.unchanged_tables += 1,
                TableUpdate::Unavailable { .. } => {
                    coverage.unavailable_tables += 1;
                    if let Some(domain_index) = domain_index {
                        coverage.domains[domain_index].unavailable += 1;
                    }
                    coverage.state_unavailable_tables += usize::from(table == TableId::States);
                    coverage.division_unavailable_tables +=
                        usize::from(table == TableId::Divisions);
                }
                TableUpdate::Truncated { .. } => {
                    coverage.truncated_tables += 1;
                    if let Some(domain_index) = domain_index {
                        coverage.domains[domain_index].truncated += 1;
                    }
                    coverage.state_truncated_tables += usize::from(table == TableId::States);
                    coverage.division_truncated_tables += usize::from(table == TableId::Divisions);
                }
                TableUpdate::Snapshot(rows) => {
                    if table == TableId::Divisions {
                        for (edge, key) in DIVISION_EDGE_KEYS.into_iter().enumerate() {
                            coverage.division_edge_snapshots[edge] |=
                                rows.iter().any(|row| row.key() == u128::from(key));
                        }
                    }
                }
            }
        }

        if spool.is_open() {
            for gap in pending_gaps.drain(..) {
                spool.append(&Record::Gap(gap)).unwrap();
            }
            if let Some(start) = &observed.start {
                spool.append(&Record::BranchStart(start.clone())).unwrap();
            }
            if index == HOURS - 1 {
                spool
                    .append(&Record::Control(ControlRecord::ClockWitness {
                        session: current_session.id,
                        qpc: u64::MAX,
                        hour,
                        load_signal: Some(signal),
                    }))
                    .unwrap();
            }
            if index == 37 {
                disk.borrow_mut().fail_after =
                    Some(encode_record(&Record::Frame(frame.clone())).unwrap().len() / 2);
                assert!(spool.append(&Record::Frame(frame.clone())).is_err());
                assert!(!spool.is_open());
                let length = disk.borrow().segments[&0].len();
                assert!(spool.append(&Record::Frame(frame)).is_err());
                assert_eq!(disk.borrow().segments[&0].len(), length);
                failed_length = Some(length);
                let reason = "injected disk write failure".to_owned();
                pending_gaps.push(Gap {
                    branch: observed.branch,
                    first_hour: hour,
                    last_hour: hour,
                    reason: reason.clone(),
                });
                missing.insert((observed.branch, hour), reason);
                continue;
            }
            final_frame_bytes = encode_record(&Record::Frame(frame.clone())).unwrap().len();
            spool.append(&Record::Frame(frame)).unwrap();
        } else {
            let mut context = vec![
                Record::SessionStart(current_session.clone()),
                Record::BranchStart(tracker.branch_start(observed.branch).unwrap().clone()),
            ];
            context.extend(pending_gaps.drain(..).map(Record::Gap));
            spool.start_segment(&context, &frame).unwrap();
        }
        tracker.record_hour(observed.branch, hour).unwrap();
        let mut direct = source;
        if keyframe {
            for (id, table) in TableId::ALL.into_iter().zip(&mut direct.tables) {
                if let TableView::Truncated { missing, .. } = table {
                    coverage.partial_keyframes += 1;
                    coverage.state_partial_keyframes += usize::from(id == TableId::States);
                    coverage.division_partial_keyframes += usize::from(id == TableId::Divisions);
                    if let Some(domain_index) = DOMAIN_TABLES.iter().position(|table| *table == id)
                    {
                        coverage.domains[domain_index].partial_keyframes += 1;
                    }
                    *table = TableView::Unavailable {
                        reason: format!("truncated at keyframe: {missing} rows not read"),
                    };
                }
            }
        }
        final_hour = (observed.branch, hour);
        assert!(expected.insert(final_hour, direct).is_none());
    }
    spool.finish_segment().unwrap();
    assert!(pending_gaps.is_empty());
    let mut disk = disk.borrow_mut();
    assert_eq!(
        disk.segments[&0].len(),
        failed_length.unwrap(),
        "abandoned segment was appended to"
    );
    assert_eq!(disk.segments.len(), 4);
    // Model forced termination partway through the last record, after its clock witness.
    let last = disk.segments.last_entry().unwrap().into_mut();
    last.truncate(last.len() - final_frame_bytes / 2);
    expected.remove(&final_hour).unwrap();
    let mut segments = Vec::new();
    for (&number, bytes) in &disk.segments {
        let mut recovered = recover_prefix(bytes);
        assert_eq!(recovered.damage.is_some(), matches!(number, 0 | 3));
        if recovered.damage.is_some() {
            assert!(recovered.valid_bytes < bytes.len());
        }
        if number == 3 {
            let witness = recovered
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Record::Control(ControlRecord::ClockWitness { hour, .. }) => Some(*hour),
                    _ => None,
                })
                .unwrap();
            assert_eq!(witness, final_hour.1);
            let reason = recovered.damage.clone().unwrap();
            recovered.records.push(Record::Gap(Gap {
                branch: final_hour.0,
                first_hour: witness,
                last_hour: witness,
                reason: reason.clone(),
            }));
            missing.insert(final_hour, reason);
        }
        segments.push((number, recovered));
    }
    Campaign {
        segments,
        expected,
        missing,
        starts,
        coverage,
    }
}

fn assert_aliases(frame: &DecodedFrame) {
    let (
        TableView::Complete(countries),
        TableView::Complete(links),
        TableView::Complete(provinces),
    ) = (
        &frame.tables[TableId::Countries.index()],
        &frame.tables[TableId::CountryLinks.index()],
        &frame.tables[TableId::Provinces.index()],
    )
    else {
        return;
    };
    let Some(RowRef::Province(province)) = provinces.first() else {
        return;
    };
    assert_eq!(province.owner, 2);
    let slot = links
        .iter()
        .find_map(|row| match row {
            RowRef::CountryLink(link) if link.id == province.owner => Some(link.canonical_slot),
            _ => None,
        })
        .unwrap();
    let country = countries
        .iter()
        .find_map(|row| match row {
            RowRef::Country(country) if country.canonical_slot == slot => Some(country),
            _ => None,
        })
        .unwrap();
    assert_eq!((country.id, country.canonical_slot), (7, 1));
    assert_eq!(country.name, [0xff, 7, 0, b'N']);
    assert_ne!(province.owner, country.id);
}

fn assert_division_handles(frame: &DecodedFrame) {
    let TableView::Complete(divisions) = &frame.tables[TableId::Divisions.index()] else {
        return;
    };
    let handles: Vec<_> = divisions
        .iter()
        .filter_map(|row| match row {
            RowRef::Division(division) if division.id >> 32 == DIVISION_A >> 32 => {
                Some(division.id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(handles, [DIVISION_A, DIVISION_B]);
}

fn assert_relations(frame: &DecodedFrame) {
    if let TableView::Complete(relations) = &frame.tables[TableId::WarRelations.index()] {
        let ordered_pairs: Vec<_> = relations
            .iter()
            .filter_map(|row| match row {
                RowRef::WarRelation(row)
                    if matches!((row.first_country, row.second_country), (7, 19) | (19, 7)) =>
                {
                    Some((row.first_country, row.second_country))
                }
                _ => None,
            })
            .collect();
        assert_eq!(ordered_pairs, [(7, 19), (19, 7)]);
    }
    let TableView::Complete(members) = &frame.tables[TableId::FactionMembers.index()] else {
        return;
    };
    assert!(
        members
            .iter()
            .zip(members.iter().skip(1))
            .all(|(left, right)| left.key() < right.key())
    );
    let first_faction: Vec<_> = members
        .iter()
        .filter_map(|row| match row {
            RowRef::FactionMember(row) if row.faction == FACTION_A => Some(row),
            _ => None,
        })
        .collect();
    assert_eq!(
        first_faction
            .iter()
            .map(|row| row.country)
            .collect::<Vec<_>>(),
        [7, 19]
    );
    let mut ordinals: Vec<_> = first_faction.iter().map(|row| row.ordinal).collect();
    ordinals.sort_unstable();
    assert_eq!(ordinals, [0, 1]);
    if let TableView::Complete(factions) = &frame.tables[TableId::Factions.index()] {
        for member in members {
            let RowRef::FactionMember(member) = member else {
                unreachable!()
            };
            assert!(
                factions
                    .iter()
                    .any(|faction| faction.key() == u128::from(member.faction))
            );
        }
    }
}

#[test]
fn two_thousand_hours_survive_faults_and_append_with_exact_direct_state() {
    let campaign = synthesize();
    assert_eq!(campaign.expected.len() + campaign.missing.len(), HOURS);
    assert!(campaign.missing.len() > 20);
    assert_eq!(campaign.starts.len(), 7);
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.session)
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 2, 3]
    );
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.reason)
            .collect::<Vec<_>>(),
        [
            BranchReason::SessionStart,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::SessionStart,
            BranchReason::SessionStart,
        ]
    );
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.parent.clone())
            .collect::<Vec<_>>(),
        [
            Parentage::Unknown,
            Parentage::Probable {
                branch: 1,
                hour: 10_400
            },
            Parentage::Unknown,
            Parentage::Unknown,
            Parentage::Probable {
                branch: 4,
                hour: 12_050
            },
            Parentage::Probable {
                branch: 5,
                hour: 12_120
            },
            Parentage::Unknown,
        ]
    );
    let coverage = &campaign.coverage;
    assert!(coverage.keyframes >= 9 && coverage.periodic_keyframes > 0);
    assert!(coverage.delta_tables > 1_000 && coverage.unchanged_tables > 1_000);
    assert!(coverage.deletions > 100 && coverage.insertions > 100);
    assert!(coverage.unavailable_tables > 50 && coverage.truncated_tables > 50);
    assert!(coverage.partial_keyframes >= 3);
    assert!(coverage.state_delta_tables > 100);
    assert!(coverage.state_unavailable_tables > 0 && coverage.state_truncated_tables > 0);
    assert!(coverage.state_partial_keyframes > 0);
    assert!(coverage.division_hp_only_deltas > 100);
    assert!(coverage.division_organisation_only_deltas > 100);
    assert!(coverage.division_deletions > 10 && coverage.division_insertions > 10);
    assert!(coverage.division_unavailable_tables > 0 && coverage.division_truncated_tables > 0);
    assert!(coverage.division_partial_keyframes > 0);
    assert!(
        coverage
            .division_edge_snapshots
            .into_iter()
            .all(|seen| seen)
    );
    assert!(
        coverage
            .division_edge_insertions
            .into_iter()
            .all(|seen| seen)
    );
    assert!(
        coverage
            .division_edge_deletions
            .into_iter()
            .all(|seen| seen)
    );
    for (old, next) in coverage.country_optional_transitions.iter().enumerate() {
        for (new, seen) in next.iter().enumerate() {
            assert!(
                old == new || *seen,
                "optional scalar transition {old} -> {new} was not exercised"
            );
        }
    }
    for (table, covered) in DOMAIN_TABLES.into_iter().zip(&coverage.domains) {
        assert_eq!(
            covered.changed_fields,
            table.full_mask(),
            "{table:?} field coverage"
        );
        assert!(
            covered.changes > 10,
            "{table:?} never changed existing rows"
        );
        if table.key_bits() != 0 {
            assert!(covered.insertions > 10, "{table:?} never inserted new rows");
            assert!(covered.deletions > 10, "{table:?} never deleted rows");
        }
        assert!(
            covered.unavailable > 0,
            "{table:?} never became unavailable"
        );
        assert!(covered.truncated > 0, "{table:?} never truncated rows");
        assert!(
            covered.partial_keyframes > 0,
            "{table:?} never truncated at a keyframe"
        );
    }
    assert_eq!(
        coverage.war_field_changes,
        TableId::WarRelations.full_mask()
    );
    assert!(coverage.war_flag_values.iter().flatten().all(|seen| *seen));
    assert!(
        coverage
            .war_casualty_extremes
            .iter()
            .flatten()
            .all(|seen| *seen)
    );
    assert!(coverage.member_ordinal_only_deltas > 100);
    assert!(coverage.member_faction_changes > 10);

    let first = Replay::from_recovered(CAMPAIGN.to_vec(), &campaign.segments[..2]).unwrap();
    assert_eq!(first.sessions.len(), 1);
    let old_bytes = first.bytes().to_vec();
    let appended = first.append_recovered(&campaign.segments[2..]).unwrap();
    assert_eq!(first.bytes(), old_bytes);
    for block in &first.index {
        let start = block.offset as usize;
        let end = start + block.length as usize;
        assert_eq!(&appended.bytes()[start..end], &old_bytes[start..end]);
        assert!(appended.index.contains(block));
    }
    let replay = Replay::open(appended.bytes().to_vec()).unwrap();
    assert_eq!(replay.sessions, [session(1), session(2), session(3)]);
    assert_eq!(replay.branches, campaign.starts);
    assert_eq!(replay.campaign_id, CAMPAIGN);
    assert_eq!(
        replay
            .recovery_notes
            .iter()
            .map(|note| note.segment)
            .collect::<Vec<_>>(),
        [0, 3]
    );
    for note in &replay.recovery_notes {
        let recovery = &campaign.segments[note.segment as usize].1;
        assert_eq!(note.valid_bytes, recovery.valid_bytes as u64);
        assert_eq!(Some(note.reason.as_str()), recovery.damage.as_deref());
    }
    assert!(replay.index.iter().all(|block| i64::from(block.last_hour)
        - i64::from(block.first_hour)
        < i64::from(KEYFRAME_INTERVAL)));

    // Sequential playback checks every independently generated state, including
    // partial rows and metrics, while decompressing each block only once.
    let mut visited = BTreeSet::new();
    let mut cold_targets = BTreeSet::new();
    for block in &replay.index {
        let frames = replay.read_block(block).unwrap();
        let mut decoder = FrameDecoder::new();
        for frame in &frames {
            let key = (frame.branch, frame.hour);
            assert!(visited.insert(key), "duplicate replay hour {key:?}");
            let expected = &campaign.expected[&key];
            let actual = decoder.advance(frame).unwrap();
            assert_eq!(actual, expected, "branch {}, hour {}", key.0, key.1);
            assert_aliases(actual);
            assert_division_handles(actual);
            assert_relations(actual);
        }
        for frame in [frames.first(), frames.get(frames.len() / 2), frames.last()]
            .into_iter()
            .flatten()
        {
            cold_targets.insert((frame.branch, frame.hour));
        }
    }
    assert_eq!(visited, campaign.expected.keys().copied().collect());
    // Cold seeks independently exercise block boundaries, interior deltas and
    // both surviving sides of every dropped or damaged-record boundary.
    for &(branch, hour) in campaign.missing.keys() {
        for adjacent in [hour.checked_sub(1), hour.checked_add(1)]
            .into_iter()
            .flatten()
        {
            if campaign.expected.contains_key(&(branch, adjacent)) {
                cold_targets.insert((branch, adjacent));
            }
        }
    }
    for (branch, hour) in cold_targets {
        let expected = &campaign.expected[&(branch, hour)];
        match replay.seek(branch, hour).unwrap() {
            SeekResult::Frame(actual) => {
                assert_eq!(actual.as_ref(), expected, "branch {branch}, hour {hour}");
                assert_aliases(&actual);
                assert_division_handles(&actual);
                assert_relations(&actual);
            }
            other => panic!("readable branch {branch}, hour {hour}: {other:?}"),
        }
    }
    for (&(branch, hour), expected_reason) in &campaign.missing {
        match replay.seek(branch, hour).unwrap() {
            SeekResult::Gap {
                branch: actual_branch,
                hour: actual_hour,
                reason,
            } => {
                assert_eq!((actual_branch, actual_hour), (branch, hour));
                assert!(
                    reason.contains(expected_reason),
                    "{reason:?} lacks {expected_reason:?}"
                );
            }
            other => panic!("lost branch {branch}, hour {hour} was invented: {other:?}"),
        }
    }
}
