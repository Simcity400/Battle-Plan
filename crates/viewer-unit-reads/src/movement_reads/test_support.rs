use super::*;
use viewer_foundation::trigger_content::TriggerContent;
use dashboard_text::triggers::LeaderRoles;
use content_reader_key_names::buildings::BuildingType;
use content_reader_map::province_geometry::ProvinceGeometry;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::ProvinceRow, rows::StateRow, rows::TableId,
    rows::TableRows,
};
use replay_frame::frame::DecodedFrame;
use replay_frame::{frame::Metrics, frame::TableView};

pub struct MovementFixture {
    pub trigger_content: viewer_foundation::trigger_content::TriggerContent,
    pub geometry: ProvinceGeometry,
    pub moves: Result<content_reader_unit_path::unit_path::MoveGraph, String>,
    pub regions: BTreeMap<u32, u32>,
    pub day_night: Result<replay_data::day_night::Defines, String>,
    pub combat_minimum_time: i32,
    pub naval_combat_min_duration: Result<i32, String>,
    pub avoid_enemy: bool,
}
impl MovementFixture {
    pub fn movement_reads(&self) -> ReadOperands<'_> {
        ReadOperands {
            moves: &self.moves,
            trigger_content: &self.trigger_content,
            regions: &self.regions,
            geometry: &self.geometry,
            combat_minimum_time: self.combat_minimum_time,
            naval_combat_min_duration: &self.naval_combat_min_duration,
            avoid_enemy: self.avoid_enemy,
            day_night: &self.day_night,
        }
    }
}
pub fn assets() -> MovementFixture {
    if !engine_executable::executable_tables::is_initialized() {
        content_reader_script::test_game::enter();
    }
    let mut assets = unnamed_assets();
    let facts = &mut assets.trigger_content;
    let ideologies = facts.ideologies.get_or_insert_with(Default::default);
    let mut triggers = (*facts.triggers).clone();
    triggers.counts = Some(content_reader_key_names::key_name_products::count_trigger_names(
        &facts.buildings,
        ideologies,
        &Default::default(),
    ));
    facts.triggers = Arc::new(triggers);
    assets
}
pub fn unnamed_assets() -> MovementFixture {
    let mut geometry = ProvinceGeometry {
        width: 4,
        height: 1,
        ..Default::default()
    };
    for province in 1..=4 {
        let x = province as i32 - 1;
        geometry.centroids.insert(province, [x, 0]);
        geometry.bounds.insert(province, [x, 0, 1, 1]);
    }
    MovementFixture {
        trigger_content: TriggerContent {
            triggers: Default::default(),
            character_traits: None,
            tag_aliases: Default::default(),
            buildings: content_reader_key_names::buildings::Buildings {
                types: BTreeMap::from([(
                    "dam".into(),
                    BuildingType {
                        name: "dam".into(),
                        need_detection: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
            ideologies: None,
            continents: Err("the test has no continent content".into()),
            dlcs: BTreeSet::from(["Gotterdammerung".to_owned()]),
            state_provinces: BTreeMap::from([(10, vec![1, 2]), (20, vec![3, 4])]),
            neighbor_states: Ok(BTreeMap::from([(10, vec![20]), (20, vec![10])])),
            impassable: BTreeSet::new(),
            terrain: None,
            resource_bonuses: [15_000, 20_000],
            leader_roles: Ok(Arc::new(LeaderRoles::new(Default::default()))),
            invasion_prepare_days: Ok(60),
            script_constants: Err("the test has no script constants".into()),
        },
        geometry,
        moves: Err("no province graph in the test".into()),
        regions: BTreeMap::new(),
        day_night: Ok(replay_data::day_night::Defines {
            gmt_offset: 0,
            south_pole_offset: 17_000,
            north_pole_offset: 93_000,
            day_night_feather: 2_400,
        }),
        combat_minimum_time: 4,
        naval_combat_min_duration: Ok(7),
        avoid_enemy: true,
    }
}

pub fn put<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

pub fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 5,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    put::<tables::Countries>(
        &mut frame,
        vec![
            CountryRow {
                id: 10,
                canonical_slot: 0,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 11,
                canonical_slot: 1,
                tag: b"ENG".to_vec(),
                ..Default::default()
            },
        ],
    );
    put::<tables::CountryLinks>(
        &mut frame,
        vec![
            CountryLinkRow {
                id: 10,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            },
        ],
    );
    put::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![replay_data::rows::ReservedCountryNamingRow {
            faction: replay_data::rows::Recorded::Value(Some(0)),
            ..Default::default()
        }],
    );
    put::<tables::Provinces>(
        &mut frame,
        (1..=4)
            .map(|id| {
                let raw = if id <= 2 { 10 } else { 11 };
                ProvinceRow {
                    id,
                    state: if id <= 2 { 10 } else { 20 },
                    owner: raw,
                    controller: raw,
                    ..Default::default()
                }
            })
            .collect(),
    );
    put::<tables::States>(
        &mut frame,
        vec![
            StateRow {
                id: 10,
                owner: 10,
                controller: 10,
                ..Default::default()
            },
            StateRow {
                id: 20,
                owner: 11,
                controller: 11,
                ..Default::default()
            },
        ],
    );
    put::<tables::CountryCounterInputs>(
        &mut frame,
        [10, 11]
            .map(|country| replay_data::rows::CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country,
                owned_state_count: 1,
                ..Default::default()
            })
            .into(),
    );
    frame
}
pub fn positive_frame() -> DecodedFrame {
    let mut frame = frame();
    put::<tables::ReservedCountryNaming>(&mut frame, Vec::new());
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    for row in &mut countries {
        if row.canonical_slot == 0 {
            row.canonical_slot = 2;
        }
    }
    put::<tables::Countries>(&mut frame, countries);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    for row in &mut links {
        if row.canonical_slot == 0 {
            row.canonical_slot = 2;
        }
    }
    put::<tables::CountryLinks>(&mut frame, links);
    frame
}
pub fn path_assets() -> MovementFixture {
    let mut graph = content_reader_map::province_graph::ProvinceGraph::default();
    for id in 1..=4_u32 {
        let next: Vec<_> = [id.wrapping_sub(1), id + 1]
            .into_iter()
            .filter(|n| (1..=4).contains(n))
            .map(|n| (n, Some(50_000)))
            .collect();
        graph.edges.insert(id, next);
    }
    let mut assets = assets();
    assets.geometry.land = BTreeSet::from([3, 4]);
    assets.moves = Ok(content_reader_unit_path::unit_path::MoveGraph::new(
        &graph,
        &assets.geometry,
    ));
    assets
}
