use super::{Actor, Batch, Category, Event, Kind, Origin, Span};

pub fn actor(id: u64, key: i32, since: u64) -> Actor {
    Actor {
        at_qpc: since,
        tag: b"GER".to_vec(),
        id,
        player: key,
        country: 3,
        since_qpc: since,
        user: b"same".to_vec(),
        name: b"same".to_vec(),
    }
}

pub fn event(actor: u64, player: i32, qpc: u64) -> Event {
    Event {
        paused: false,
        actor,
        player,
        qpc,
        hour: 100,
        kind: 1,
        command_country: Some(3),
        origin: Origin::PlayerOrder,
        sequence: qpc as u32,
    }
}

pub fn batch() -> Batch {
    Batch {
        kinds: vec![Kind {
            id: 1,
            token: 9,
            name: "synthetic movement".into(),
            category: Category::Army,
        }],
        actors: vec![
            actor(1, 10, 100),
            actor(2, 20, 130),
            actor(3, 10, 160),
            actor(4, 30, 100),
        ],
        departures: vec![(1, 160)],
        events: vec![event(1, 10, 110), event(2, 20, 140), event(3, 10, 170)],
        spans: vec![Span {
            paused: false,
            hour: 100,
            from_qpc: 100,
            to_qpc: 220,
            complete: true,
        }],
    }
}
