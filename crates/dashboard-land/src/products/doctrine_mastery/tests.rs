use super::*;
use replay_data::rows::{DoctrineFolderRow, DoctrineTrackRow, LexicalTokenRow, TableSchema};
use replay_frame::frame::{Metrics, TableView};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame { branch: 1, hour: 10, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Metrics::default() };
    set::<tables::LexicalTokens>(&mut frame, [(1, "land"), (2, "mass_assault"), (3, "mobile_infantry"), (4, "great_war_infantry")].map(|(token, literal)| LexicalTokenRow { token, literal: literal.as_bytes().to_vec() }).to_vec());
    set::<tables::DoctrineFolders>(
        &mut frame,
        vec![
            DoctrineFolderRow { slot: 7, ordinal: 0, folder: 1, grand_doctrine: Some(2), completed_subdoctrines: vec![4] },
            DoctrineFolderRow { slot: 8, ordinal: 0, folder: 1, grand_doctrine: None, completed_subdoctrines: Vec::new() },
        ],
    );
    set::<tables::DoctrineTracks>(
        &mut frame,
        vec![
            DoctrineTrackRow { slot: 7, folder: 0, ordinal: 0, sub_doctrine: Some(3), rewards: 2, mastery: 41_758, mastery_bank: 5, daily_mastery: 7, effective_daily_mastery: 9 },
            DoctrineTrackRow { slot: 7, folder: 0, ordinal: 1, sub_doctrine: None, rewards: 0, mastery: 0, mastery_bank: 0, daily_mastery: 0, effective_daily_mastery: 0 },
            DoctrineTrackRow { slot: 8, folder: 0, ordinal: 0, sub_doctrine: Some(3), rewards: 5, mastery: 1, mastery_bank: 1, daily_mastery: 1, effective_daily_mastery: 1 },
        ],
    );
    frame
}

fn context() -> Context {
    Context { database: Arc::new(Database::default()), localisation: Arc::new(Localisation::default()), default_reward: Ok(10_000_000) }
}

#[test]
fn a_countrys_folders_hold_its_own_tracks_as_recorded_and_unknown_names_are_counted_not_made_up() {
    let frame = frame();
    let cells = Cells::default();
    let (folders, refused) = folders(&frame, 7, &context(), &cells).unwrap();
    assert_eq!(folders.len(), 1, "another slot's folder is not this country's");
    let folder = &folders[0];
    assert_eq!((folder.id.as_str(), folder.name.as_deref()), ("land", None));
    assert_eq!(folder.grand.as_ref().map(|grand| grand.id.as_str()), Some("mass_assault"));
    assert_eq!(folder.completed.iter().map(|sub| sub.id.as_str()).collect::<Vec<_>>(), ["great_war_infantry"]);
    let track = &folder.tracks[0];
    assert_eq!(track.unlocked, 2);
    let live = cells.finish();
    assert_eq!(live.groups.len(), 1, "one followed query: the country's track rows in the four columns");
    assert_eq!(live.groups[0].keys, [7_u128 << 64, 7_u128 << 64 | 1], "only this country's tracks are followed");
    use dashboard_follow::live::CellValue::Number;
    assert_eq!(live.values(&frame)[0][..4], [Number(41_758), Number(5), Number(7), Number(9)], "mastery, bank, daily and effective gain are the row's cells");
    assert_eq!(track.sub.as_ref().map(|sub| sub.id.as_str()), Some("mobile_infantry"));
    assert!(folder.tracks[1].sub.is_none(), "no subdoctrine chosen is none, not a made-up one");
    assert!(!refused.lines("doctrine mastery content").is_empty(), "names the content lacks are named in the warnings");
}

#[test]
fn doctrines_not_recorded_at_the_hour_refuse_by_name() {
    let mut frame = frame();
    *frame.tables[TableId::DoctrineTracks.index()] = TableView::Unavailable { reason: "not read".into() };
    let error = folders(&frame, 7, &context(), &Cells::default()).unwrap_err();
    assert!(error.to_string().starts_with("Doctrine mastery is not recorded at this hour"), "{error}");
}
