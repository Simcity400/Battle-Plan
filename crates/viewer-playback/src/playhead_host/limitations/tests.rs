use super::{BEING_READ, NOT_IN_TIME, Reason};
use crate::{
    layers::{View, recorded::record},
    playhead_host::{MapSide, details, open, rows::tests::hourly_steps, shared_maps_tests::Reads, units::Units},
};
use replay_columnar::manifest::Status;
use replay_data::{rows::TableId, scope::CaptureMode};
use std::{sync::Arc, time::Duration};

const REASON: &str = "the collector could not read it";

/// Twelve hours in which `LandCombats` is unavailable for hours 4 to 7 with [`REASON`].
fn unavailable(name: &str) -> Units {
    record(name, 12, |at, store| {
        let reason = store.intern(REASON.as_bytes()).unwrap();
        let status = if (4..8).contains(&at) { Status::Unavailable { reason } } else { Status::Complete };
        store.status(TableId::LandCombats.index(), status, true, Some(at as i32), true).unwrap();
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    })
}

/// What `read` gives once it no longer says the words are being read; the read runs on its own
/// thread, so each look yields to it.
fn once_read(mut read: impl FnMut() -> String) -> String {
    for _ in 0..50_000_000u64 {
        let words = read();
        if !words.contains(BEING_READ) {
            return words;
        }
        std::thread::yield_now();
    }
    panic!("the reason's words were never read");
}

#[test]
fn an_unavailable_table_no_layer_declares_gives_its_reasons_words_at_the_head() {
    let entry = open(unavailable("limitations words"), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let (layer, _) = Reads::new(&[TableId::Countries]);
    let mut side = MapSide::new(Arc::clone(&entry), vec![layer], View::default()).unwrap();
    side.anchor(5, false, 0.0, true);
    side.catch_up(5).unwrap();
    assert!(side.parts().is_none_or(|(mirrors, _)| mirrors.get(TableId::LandCombats).is_none()), "no mirror holds the table");
    let line = |lines: Vec<String>, start: &str| lines.into_iter().find(|line| line.starts_with(start)).unwrap_or_default();
    let detail = line(details::of_map(&side), "LandCombats: ");
    assert_eq!(detail, format!("LandCombats: unavailable ({REASON})"), "the report waits for the words: asked once, first, it gives them");
    let limitation = once_read(|| line(super::lines(&side, &[]).unwrap(), "Table LandCombats "));
    assert!(limitation.starts_with(&format!("Table LandCombats unavailable: {REASON}: ")), "the reason's words, never its id: {limitation}");
}

#[test]
fn a_reason_id_the_recording_holds_no_text_for_is_named_as_such() {
    let entry = open(unavailable("limitations no text"), hourly_steps()).unwrap();
    let words = super::words(&entry, TableId::LandCombats, 9_999, (0, 0));
    assert_eq!(words, "the recording holds no text for its reason (string 9999)");
}

#[test]
fn a_reason_still_unread_is_being_read_on_a_frame_and_not_read_in_time_in_a_report() {
    let reason = Reason::default();
    assert_eq!(reason.words(Duration::ZERO), BEING_READ, "a frame never waits");
    assert_eq!(reason.words(Duration::from_nanos(1)), NOT_IN_TIME, "a report that waited says so by name");
    reason.set(Ok(Some(REASON.as_bytes().to_vec())), 7);
    assert_eq!(reason.words(Duration::ZERO), REASON);
}

/// Reads NavalBases' unreadable country lists, and DivisionTemplateIcons' keys alone.
struct RowNotes;

impl crate::layers::Layer for RowNotes {
    fn name(&self) -> &'static str {
        "row notes"
    }

    fn inputs(&mut self, _view: &View, inputs: &mut crate::layers::Inputs<'_>) -> std::io::Result<()> {
        inputs.number::<bool>(TableId::NavalBases, "countries_unreadable")?;
        inputs.table(TableId::DivisionTemplateIcons, true)
    }

    fn compose(&mut self, _mirrors: &crate::playhead_host::mirror::Mirrors) -> std::io::Result<()> {
        Ok(())
    }

    fn apply(&mut self, _step: &playhead::Step, _mirrors: &crate::playhead_host::mirror::Mirrors) {}

    fn settle(&mut self, _mirrors: &crate::playhead_host::mirror::Mirrors, _budget: &mut crate::layers::Budget) -> std::io::Result<()> {
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut crate::layers::Gpu<'_>, _budget: &mut crate::layers::Budget) -> std::io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> crate::layers::Carried {
        crate::layers::Carried::default()
    }

    fn held(&self) -> usize {
        0
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn a_tables_row_note_is_todays_from_the_column_a_drawn_layer_reads_and_named_where_none_reads_it() {
    use crate::layers::recorded::column;
    use replay_columnar::state::Cell;
    let units = record("limitations row notes", 12, |at, store| {
        for table in [TableId::NavalBases, TableId::DivisionTemplateIcons] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        if at == 6 {
            let mut bases = store.table(TableId::NavalBases.index()).unwrap();
            for (slot, unreadable) in [false, true, true].into_iter().enumerate() {
                bases.insert_at(slot as u32, 100 + slot as u128).unwrap();
                bases.write(slot as u32, column(TableId::NavalBases, "countries_unreadable"), Cell::Bool(unreadable)).unwrap();
            }
            let mut templates = store.table(TableId::DivisionTemplateIcons.index()).unwrap();
            templates.insert_at(0, 9).unwrap();
            templates.write(0, column(TableId::DivisionTemplateIcons, "model_unreadable"), Cell::U8(1)).unwrap();
        }
    });
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(RowNotes)], View::default()).unwrap();
    let unread = format!("{}Field Division template model inputs: no drawn layer reads DivisionTemplateIcons.model_unreadable", super::NOT_GIVEN);
    let bases = "Field Naval base access lists: 2 naval bases recorded an unreadable country list; ";
    side.anchor(4, false, 0.0, true);
    side.catch_up(4).unwrap();
    let lines = super::lines(&side, &[]).unwrap();
    assert!(!lines.iter().any(|line| line.starts_with("Field Naval base access lists") || *line == unread), "tables holding no rows give no row note: {lines:?}");
    side.anchor(8, false, 0.0, true);
    side.catch_up(8).unwrap();
    let lines = super::lines(&side, &[]).unwrap();
    assert!(lines.iter().any(|line| line.starts_with(bases)), "the count of the column a layer reads, in today's words: {lines:?}");
    assert!(lines.contains(&unread), "the note whose column no drawn layer reads is named, not dropped: {lines:?}");
}

#[test]
fn todays_lines_of_a_layer_the_map_does_not_register_are_counted_apart_by_owner_and_not_compared() {
    const TRAINS: &[&str] = &["trains"];
    let phase = "Field Supply train phase: the scheduler is unrecorded".to_owned();
    let by_status = vec![(TRAINS, vec![phase.clone()])];
    let today: Vec<String> = [
        "Table Countries truncated (3 missing rows): names may be missing",
        "Viewing country GER: the recording client's stored viewpoint; fog of war on",
        "Fog of war: 12 map cells use the viewing country's recorded visibility.",
        "Field ArmyOrderPlans (geometry): fronts",
        "Field ArmyOrderPlans (presentation): plans",
        &phase,
        "Field ShipModels formation: the capital slot",
    ]
    .map(str::to_owned)
    .to_vec();
    let split = |registered: &[&str], held: &[TableId]| {
        let (compared, apart) = super::by_registered(today.iter(), &by_status, |name| registered.contains(&name), |table| held.contains(&table));
        (compared.into_iter().map(|line| &line[..12]).collect::<Vec<_>>(), apart.into_iter().collect::<Vec<_>>())
    };
    let apart = |owners: &[(&str, usize)]| owners.iter().map(|&(owner, count)| (owner.to_owned(), count)).collect::<Vec<_>>();
    assert_eq!(
        split(&["vision", "orders"], &[]),
        (vec!["Table Countr", "Viewing coun", "Field ArmyOr", "Field ArmyOr"], apart(&[("intel", 1), ("readers of ShipModels", 1), ("trains", 1)])),
        "fog of war is vision's and intel's; the train phase is the trains'; the formation is given while a drawn layer reads ShipModels"
    );
    assert_eq!(split(&["vision", "intel", "orders", "trains"], &[TableId::ShipModels]).1, apart(&[]), "every owner there: every line compared");
    assert_eq!(split(&["vision", "intel", "orders", "trains"], &[TableId::ShipModels]).0.len(), 7);
    assert_eq!(split(&[], &[]), (vec!["Table Countr"], apart(&[("orders", 2), ("readers of ShipModels", 1), ("trains", 1), ("vision", 2)])), "a table's note is compared whoever is registered; a line two layers own counts for the first missing");
}

#[test]
fn the_lists_are_compared_by_their_lines_text_each_sides_extra_lines_named_and_the_order_counted_apart() {
    let lines = |text: &[&str]| text.iter().map(|line| (*line).to_owned()).collect::<Vec<String>>();
    let ours = lines(&["a", "b", "x", "c", "d"]);
    let today = lines(&["a", "b", "c", "y", "d"]);
    let (missing, extra, matched, reordered) = super::by_text(&ours, &today.iter().collect::<Vec<_>>());
    assert_eq!((missing, extra, matched, reordered), (lines(&["y"]), lines(&["x"]), 4, 0), "one line on each side shifts nothing after it");
    let today = lines(&["b", "a", "c", "d", "d"]);
    let ours = lines(&["a", "b", "c", "d"]);
    let (missing, extra, matched, reordered) = super::by_text(&ours, &today.iter().collect::<Vec<_>>());
    assert_eq!((missing, extra, matched, reordered), (lines(&["d"]), lines(&[]), 4, 2), "a line twice today is matched once at the head; two lines swapped are two out of place");
}

/// Every table unavailable: each status note of today's maker is stated.
struct Unread;

impl viewer_foundation::map::tables::Statuses for Unread {
    fn seen(&self, _table: TableId) -> viewer_foundation::map::tables::Seen {
        viewer_foundation::map::tables::Seen::Unavailable("unread".into())
    }
}

/// Today's maker (`map_products::ProductHour::limitations_with`) says every line it can: the tables'
/// notes with every row fact, then each note it adds in its order. A wording no owner names would
/// be compared whoever is registered, so a new note cannot be added without its owner.
#[test]
fn every_wording_todays_limitation_maker_can_give_has_an_owner() {
    use super::Owner;
    use viewer_foundation::map::{
        perspective::Resolution,
        tables::{DISPLAY_NOT_RECORDED, Limitations, Perspective, RowFacts, table_limitations},
    };
    let facts = RowFacts { unreadable_naval_bases: 1, divisions: true, task_forces: true, unreadable_templates: 1, ship_counters: Some(true), weather: true, weather_display: Some(Some(DISPLAY_NOT_RECORDED)) };
    let mut lines = table_limitations(&Unread, &facts).into_lines();
    lines.extend(table_limitations(&Unread, &RowFacts { ship_counters: Some(false), weather_display: Some(None), ..facts.clone() }).into_lines());
    let mut out = Limitations::default();
    out.note_trains(&Unread);
    out.note_aircraft(&Unread);
    out.note_point_lights();
    out.note_field("Nuclear impact effects", "refused");
    for resolution in [Resolution::Recorded, Resolution::Requested, Resolution::NoPlayer, Resolution::Absent("GER".into())] {
        for fog in [true, false] {
            out.note_perspective(&Perspective { tag: "ENG".into(), resolution: resolution.clone(), fog });
        }
    }
    out.note_battle_plan(1);
    out.note_raid_arrows(1, 1);
    out.note_fog_on_sight(&Unread);
    out.note_undecided_sight("UnitIntel");
    out.note_visibility_gap("no sight");
    out.note_fog_of_war(1, 0.5);
    map_products::note_truncated_perspective(&mut out, "UnitIntel");
    map_products::note_truncated_perspective(&mut out, "CountryIntel");
    map_products::note_unreadable_names(&mut out, &[1]);
    viewer_world::map::labels::note_withheld(&mut out, [(1, viewer_world::map::labels::NO_VICTORY_POINTS)]);
    lines.extend(out.into_lines());
    let by_status = super::status_notes(&Unread);
    let unowned: Vec<&String> = lines.iter().filter(|line| super::owner(line, &by_status).is_none()).collect();
    assert!(unowned.is_empty(), "lines today's maker gives that no owner names: {unowned:?}");
    let mut trains = Limitations::default();
    trains.note_trains(&Unread);
    let trains = trains.into_lines();
    assert!(!trains.is_empty() && trains.iter().all(|line| super::owner(line, &by_status) == Some(Owner::Layers(&["trains"]))), "a layer's status note is its own, not the tables'");
}
