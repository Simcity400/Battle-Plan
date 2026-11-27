use super::*;
use std::cell::RefCell;

fn ace_frame(rows: Vec<AceRangeInputRow>) -> replay_frame::frame::DecodedFrame {
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut frame = replay_frame::frame::DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    *frame.tables[TableId::AceRangeInputs.index()] =
        TableView::Complete(TableRows::AceRangeInputs(rows));
    frame
}

#[test]
fn frame_ace_lookup_indexes_interleaved_country_handles_once() {
    let frame = ace_frame(vec![
        AceRangeInputRow {
            canonical_slot: 1,
            ace: 90,
            range: 900,
            ..Default::default()
        },
        AceRangeInputRow {
            canonical_slot: 2,
            ace: 3,
            range: 30,
            ..Default::default()
        },
        AceRangeInputRow {
            canonical_slot: 3,
            ace: 70,
            range: 700,
            ..Default::default()
        },
    ]);
    let sources = FrameSources::new(&frame);
    assert!(sources.aces.get().is_none());
    assert_eq!(sources.ace(3).unwrap().unwrap().range, 30);
    let index = std::ptr::from_ref(sources.aces.get().unwrap());
    assert_eq!(sources.ace(90).unwrap().unwrap().range, 900);
    assert_eq!(sources.ace(70).unwrap().unwrap().range, 700);
    assert_eq!(sources.ace(4).unwrap(), None);
    assert_eq!(std::ptr::from_ref(sources.aces.get().unwrap()), index);
}

#[test]
fn frame_ace_duplicates_share_equal_rows_and_refuse_conflicts_by_handle() {
    let shared = AceRangeInputRow {
        canonical_slot: 1,
        ace: 90,
        range: 900,
        ..Default::default()
    };
    let other = AceRangeInputRow {
        canonical_slot: 2,
        ace: 3,
        range: 30,
        ..Default::default()
    };
    let frame = ace_frame(vec![shared.clone(), shared.clone(), other.clone()]);
    assert_eq!(FrameSources::new(&frame).ace(90).unwrap(), Some(&shared));
    let frame = ace_frame(vec![
        shared.clone(),
        AceRangeInputRow {
            canonical_slot: 3,
            range: 901,
            ..shared
        },
        other.clone(),
    ]);
    let sources = FrameSources::new(&frame);
    let reason = sources.ace(90).unwrap_err();
    assert!(reason.contains("0x5a") && reason.contains("conflicting"));
    assert_eq!(sources.ace(3).unwrap(), Some(&other));
}

struct Fixture {
    countries: Vec<RaidCountryInputRow>,
    command: Vec<CountryCommandPowerRow>,
    wings: Vec<AirWingRow>,
    aces: Vec<AceRangeInputRow>,
    reads: RefCell<Vec<String>>,
}

impl Sources for Fixture {
    fn slot(&self, raw: u32) -> Option<u32> {
        self.reads.borrow_mut().push(format!("slot:{raw}"));
        (raw == 11).then_some(1).or((raw == 22).then_some(2))
    }
    fn country(&self, slot: u32) -> Result<&RaidCountryInputRow, String> {
        self.reads.borrow_mut().push(format!("country:{slot}"));
        self.countries
            .iter()
            .find(|row| row.canonical_slot == slot)
            .ok_or_else(|| "country fixture unavailable".into())
    }
    fn command_power(&self, slot: u32) -> Result<&CountryCommandPowerRow, String> {
        self.reads.borrow_mut().push(format!("command:{slot}"));
        self.command
            .iter()
            .find(|row| row.canonical_slot == slot)
            .ok_or_else(|| "command fixture unavailable".into())
    }
    fn wing(&self, handle: u64) -> Result<Option<&AirWingRow>, String> {
        self.reads.borrow_mut().push(format!("wing:{handle}"));
        Ok(self.wings.iter().find(|row| row.id == handle))
    }
    fn ace(&self, handle: u64) -> Result<Option<&AceRangeInputRow>, String> {
        self.reads.borrow_mut().push(format!("ace:{handle}"));
        Ok(self.aces.iter().find(|row| row.ace == handle))
    }
}

fn defines() -> Defines {
    Defines {
        map_scale: 200_000,
        ace_wing_size: 100_000,
        ace_wing_size_max_bonus: 230_000,
    }
}

fn fixture() -> Fixture {
    Fixture {
        countries: vec![RaidCountryInputRow {
            canonical_slot: 1,
            air_range_factor: Some(25_000),
            ..Default::default()
        }],
        command: Vec::new(),
        wings: Vec::new(),
        aces: Vec::new(),
        reads: RefCell::default(),
    }
}

#[test]
fn live_ace_uses_its_owner_and_assigned_wing_before_the_wing_country_bonus() {
    let mut sources = fixture();
    sources.command = vec![
        CountryCommandPowerRow {
            canonical_slot: 2,
            ace_effectiveness_factor: Some(50_000),
            ..Default::default()
        },
        CountryCommandPowerRow {
            canonical_slot: 1,
            air_ace_bonuses_factor: Some(5_000),
            ..Default::default()
        },
    ];
    sources.wings.push(AirWingRow {
        id: 9,
        planes: 2,
        ..Default::default()
    });
    sources.aces.push(AceRangeInputRow {
        ace: 7,
        canonical_slot: 2,
        wing: 9,
        alive: 1,
        range: 20_000,
        type_range: 1,
        ..Default::default()
    });
    let wing = AirWingRow {
        owner: 22,
        country_override: 11,
        ace: 7,
        base_range_raw: 200_000,
        ..Default::default()
    };
    assert_eq!(wing_range(&wing, defines(), &sources).unwrap(), 145_000);
    assert_eq!(
        *sources.reads.borrow(),
        [
            "slot:11",
            "country:1",
            "ace:7",
            "command:2",
            "wing:9",
            "command:1"
        ]
    );
    sources.reads.borrow_mut().clear();
    sources.aces[0].type_range = 0;
    sources.command.pop();
    assert_eq!(wing_range(&wing, defines(), &sources).unwrap(), 140_000);
    assert_eq!(
        *sources.reads.borrow(),
        ["slot:11", "country:1", "ace:7", "command:2", "wing:9"]
    );
}

#[test]
fn ace_scale_preserves_missing_zero_signed_and_sentinel_cases() {
    let mut sources = fixture();
    let mut ace = AceRangeInputRow::default();
    assert_eq!(ace_scale(&ace, defines(), &sources).unwrap(), 100_000);
    ace.wing = 9;
    assert_eq!(ace_scale(&ace, defines(), &sources).unwrap(), 100_000);
    sources.wings.push(AirWingRow {
        id: 9,
        ..Default::default()
    });
    assert_eq!(ace_scale(&ace, defines(), &sources).unwrap(), 100_000);
    sources.wings[0].planes = -2;
    assert_eq!(ace_scale(&ace, defines(), &sources).unwrap(), -50_000);
    sources.wings[0].planes = 1;
    let sentinel = Defines {
        ace_wing_size: i64::MAX,
        ace_wing_size_max_bonus: i64::MAX,
        ..defines()
    };
    assert_eq!(ace_scale(&ace, sentinel, &sources).unwrap(), 0xffff_ffff);
}

#[test]
fn invalid_scale_and_wrapping_arithmetic_keep_native_sentinels() {
    assert_eq!(to_map(200_000, 0), 0xffff_ffff);
    assert_eq!(to_map(i64::MAX, 200_000), 0xffff_ffff);
    assert_eq!(to_map(i64::MIN, 200_000), 0xffff_ffff);
    assert_eq!(to_map(i64::MIN + 1, 200_000), 0xffff_ffff);
    assert_eq!(to_map(i64::MAX - 1, 200_000), -1);
    assert_eq!(to_map(i64::MAX - 2, 200_000), -1);
    assert!(!finite(i64::MAX));
    assert!(!finite(i64::MIN));
    assert!(!finite(i64::MIN + 1));
    assert!(finite(i64::MIN + 2));
    assert!(finite(i64::MAX - 1));
    assert!(finite(i64::MAX - 2));
    let mut sources = fixture();
    sources.countries[0].air_range_factor = Some(i64::MAX);
    let wing = AirWingRow {
        owner: 11,
        base_range_raw: 200_000,
        ..Default::default()
    };
    assert_eq!(wing_range(&wing, defines(), &sources).unwrap(), 99_999);
}

#[test]
fn missing_reached_modifier_refuses_instead_of_becoming_zero() {
    let mut sources = fixture();
    sources.countries[0].air_range_factor = None;
    let wing = AirWingRow {
        owner: 11,
        base_range_raw: 200_000,
        ..Default::default()
    };
    assert_eq!(
        wing_range(&wing, defines(), &sources).unwrap_err(),
        "air_range_factor is not a registered modifier"
    );
}
