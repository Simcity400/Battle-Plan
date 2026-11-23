use super::entity_animation::Request;
use content_reader_mesh::mesh;

pub fn callback(id: &str, side: u8, track: usize, count: usize) -> Option<u32> {
    let bytes = id.as_bytes();
    let rest = if bytes.starts_with(b"a_") {
        if side != 0 {
            return None;
        }
        &bytes[2..]
    } else if bytes.starts_with(b"d_") {
        if side != 1 {
            return None;
        }
        &bytes[2..]
    } else {
        bytes
    };
    let state = if rest.first().is_some_and(u8::is_ascii_digit) {
        let number = content_reader_parser::fixed5::decimal_i32(rest);
        if number > 0 && number as usize <= count && number as usize != track + 1 {
            return None;
        }
        rest.get(2..).unwrap_or_default()
    } else {
        rest
    };
    Some(
        state
            .iter()
            .take_while(|byte| **byte != 0)
            .fold(0x811c_9dc5u32, |hash, byte| {
                (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
            }),
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn scenario_requests(
    events: &[mesh::EntityEvent],
    speed: f32,
    actor: &super::entity_animation::Rig,
    count: usize,
    track: usize,
) -> Vec<Request> {
    scenario_side_requests(events, speed, actor, 0, count, track)
}

pub fn scenario_side_requests(
    events: &[mesh::EntityEvent],
    speed: f32,
    actor: &super::entity_animation::Rig,
    side: u8,
    count: usize,
    track: usize,
) -> Vec<Request> {
    let mut requests = Vec::new();
    for event in events {
        let Some(hash) = event
            .id
            .as_deref()
            .and_then(|id| callback(id, side, track, count))
        else {
            continue;
        };
        let Some(state) = actor.nodes[0]
            .clips
            .iter()
            .map(|clip| &clip.state.name)
            .find(|name| {
                name.bytes().fold(0x811c_9dc5u32, |hash, byte| {
                    (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
                }) == hash
            })
        else {
            continue;
        };
        requests.push(Request {
            time: event.time / speed,
            phase_time: None,
            state: state.clone(),
        });
    }
    requests.sort_by(|a, b| a.time.total_cmp(&b.time));
    requests
}
