use super::{Place, Strikes};
use viewer_layer_core::{layers::{Inputs, Layer, View}, test_support::sound_recording::{column, complete, play, quick, record, sought, units}, playhead_host::{MapSide, mirror::{self, Mirror, Mirrors}, open}};
use playhead::{changes::Changes, lane::Lane, pages::Pages};
use replay_columnar::{state::Cell, store::Store};
use replay_data::rows::TableId;
use std::sync::Arc;
use viewer_models::strike_effects::{Effect, Impacts};

fn place() -> Place {
    Arc::new(|province| match province {
        10 | 11 => Some(([province as f32, 0.0, 1.0], [0.0, 0.0, 2.0, 2.0])),
        _ => None,
    })
}

fn append(store: &mut Store, key: u128, province: u32, nuke_type: u32) {
    let table = TableId::NuclearStrikes;
    let mut cells: Vec<Option<Cell<'_>>> = vec![None; store.kinds(table.index()).len()];
    cells[column(table, "province")] = Some(Cell::U32(province));
    cells[column(table, "nuke_type")] = Some(Cell::U32(nuke_type));
    store.table(table.index()).unwrap().append(key, &cells).unwrap();
}

fn strikes(at: u32, store: &mut Store) {
    complete(store, at, &[TableId::NuclearStrikes]);
    let table = TableId::NuclearStrikes;
    match at {
        3 => append(store, 1, 10, 0),
        4 => store.table(table.index()).unwrap().forget(0).unwrap(),
        9 => append(store, 2, 999, 1),
        12 => append(store, 3, 11, 1),
        13 => store.table(table.index()).unwrap().forget(1).unwrap(),
        _ => {}
    }
}

fn layer(side: &MapSide) -> &Strikes {
    side.parts().unwrap().1[0].any().downcast_ref::<Strikes>().unwrap()
}

fn ids(impacts: &Impacts) -> Vec<u64> {
    impacts.impacts.iter().map(|impact| impact.id).collect()
}

#[test]
fn a_row_appended_or_forgotten_updates_the_impacts_at_the_head_as_composed() {
    let entry = open(units(record(20, strikes), "strikes"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Strikes::new(place()))], View::default()).unwrap();
    sought(&mut side, 9);
    let at_seek = layer(&side).product();
    assert_eq!(ids(&at_seek), Vec::<u64>::new(), "impact 2 has no place");
    assert_eq!(at_seek.refused, ["1 nuclear impacts have no effect: their province has no standstill anchor or painted box"]);
    assert_eq!(layer(&side).notes(), ["Field Nuclear impact effects: 1 nuclear impacts have no effect: their province has no standstill anchor or painted box"], "today's limitation line of each refusal");
    let mut seen = Vec::new();
    play(&mut side, 9, |side, frame| {
        let hour = frame.applied.unwrap();
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = layer(side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(layer(side).differ(&fresh), (0, None), "settled equals composed at hour {hour}");
        fresh.settle(mirrors, viewer_layer_core::layers::Settling::Whole).unwrap();
        assert_eq!(layer(side).notes(), fresh.notes(), "the notes settled equal those composed at hour {hour}");
        seen.push((hour, layer(side).product()));
        hour >= 19
    });
    let (_, last) = seen.last().unwrap();
    assert_eq!(ids(last), [3]);
    assert_eq!((last.impacts[0].effect, last.impacts[0].province, last.impacts[0].position), (Effect::Thermonuclear, 11, [11.0, 0.0, 1.0]));
    assert!(last.refused.is_empty(), "impact 2 was forgotten: {:?}", last.refused);
    assert!(layer(&side).notes().is_empty(), "no refusal, no note: {:?}", layer(&side).notes());
}

#[test]
fn a_strike_appended_and_forgotten_inside_a_step_reaches_the_layer_in_birth_order() {
    let recording = record(20, strikes);
    let kinds = replay_recording::schema::schema().unwrap();
    let pages: Arc<dyn Pages> = recording;
    let table = TableId::NuclearStrikes;
    let columns = [column(table, "province"), column(table, "nuke_type")];
    let mut lane = Lane::new(Arc::clone(&pages), 0, table.index(), &columns, true).unwrap();
    lane.seek(0).unwrap();
    let mut layer = Strikes::new(place());
    layer.inputs(&View::default(), &mut Inputs::new(&kinds, "strikes")).unwrap();
    let rebuild = mirror::rebuild(pages);
    let mirrors = |lane: &Lane| {
        let mut mirrors = Mirrors::default();
        let held = rebuild(lane, &columns).unwrap().downcast::<Mirror>().map_err(|_| "a mirror").unwrap();
        mirrors.insert(held);
        mirrors
    };
    layer.compose(&mirrors(&lane)).unwrap();
    let mut changes = Changes::default();
    let mut to = 4;
    let mut appended = 0;
    while to < 20 {
        lane.fetch_through(to).unwrap();
        let head = lane.step(to, &mut changes).unwrap();
        appended += changes.log().map_or(0, |log| log.len());
        let held = mirrors(&lane);
        layer.apply_changes(&changes, &held);
        layer.settle(&held, viewer_layer_core::layers::Settling::Whole).unwrap();
        if head == to {
            to += 4;
        }
    }
    assert_eq!(appended, 3, "impact 1 was appended and forgotten inside the step of hours 1 to 4");
    let (events, unplaced) = layer.take_events();
    let events: Vec<(u64, u32, Effect)> = events.iter().map(|impact| (impact.id, impact.province, impact.effect)).collect();
    assert_eq!(events, [(1, 10, Effect::Nuclear), (3, 11, Effect::Thermonuclear)], "every impact the head passed, in birth order");
    assert_eq!(unplaced, 1, "impact 2 has no place");
    assert_eq!(layer.take_events(), (Vec::new(), 0), "taken once");
    assert_eq!(ids(&layer.product()), [3], "an impact forgotten is not at the head");
}
