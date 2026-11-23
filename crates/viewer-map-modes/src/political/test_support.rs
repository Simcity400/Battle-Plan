use super::*;
use content_reader_map::map::{CountryColors, ProvinceDefinition};
use content_reader_regions::regions::StrategicRegion;
use replay_data::rows::{CountryAppearanceRow, CountryLinkRow, CountryRow, ProvinceRow, TableId, TableRows};
use replay_frame::frame::{Metrics, TableView};

pub fn map() -> MapData {
    let kinds = [
        (0, ProvinceKind::Land),
        (1, ProvinceKind::Land),
        (2, ProvinceKind::Land),
        (3, ProvinceKind::Land),
        (4, ProvinceKind::Land),
        (5, ProvinceKind::Sea),
    ];
    MapData {
        width: 6,
        height: 2,
        province_ids: vec![1, 1, 2, 3, 4, 5, 1, 1, 2, 3, 4, 5],
        definitions: kinds
            .into_iter()
            .map(|(id, kind)| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind,
                        continent: 0,
                    },
                )
            })
            .collect(),
        country_colors: CountryColors::default(),
    }
}

pub fn province(id: u32, state: u32, owner: u32, controller: u32) -> ProvinceRow {
    ProvinceRow {
        id,
        state,
        owner,
        controller,
        victory_points: 0,
        ..Default::default()
    }
}

pub fn frame(rows: Vec<ProvinceRow>) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for the political layers".into(),
    });
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 100,
            canonical_slot: 0,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 200,
            canonical_slot: 1,
            tag: b"SOV".to_vec(),
            ..Default::default()
        },
    ]));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
        CountryLinkRow {
            id: 100,
            canonical_slot: 0,
        },
        CountryLinkRow {
            id: 200,
            canonical_slot: 1,
        },
    ]));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(rows));
    tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            appearance(100, [0.75, 0.5, 0.25]),
            appearance(200, [0.5, 0.0, 0.25]),
        ]));
    tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
    tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(Vec::new()));
    DecodedFrame {
        branch: 1,
        hour: 42,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

pub fn occupied() -> DecodedFrame {
    frame(vec![
        province(1, 10, 100, 100),
        province(2, 10, 100, 100),
        province(3, 11, 100, 200),
        province(4, 11, 100, 100),
        province(5, 0, 0, 0),
    ])
}

pub fn strategic_regions() -> BTreeMap<u32, StrategicRegion> {
    BTreeMap::from([(
        4,
        StrategicRegion {
            id: 4,
            name_key: "STRATEGICREGION_4".into(),
            provinces: vec![5],
            naval_terrain: None,
            static_modifiers: false,
            static_modifier_names: Vec::new(),
            ..Default::default()
        },
    )])
}

pub(super) fn appearance(country: u32, rgb: [f32; 3]) -> CountryAppearanceRow {
    CountryAppearanceRow {
        country,
        red: rgb[0].to_bits(),
        green: rgb[1].to_bits(),
        blue: rgb[2].to_bits(),
        alpha: 1.0f32.to_bits(),
        ..Default::default()
    }
}
