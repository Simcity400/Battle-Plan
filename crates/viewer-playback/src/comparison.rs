use crate::playhead_host::MapSide;
use replay_data::Hour;
use serde_json::Value;

#[derive(Debug, Default)]
pub struct Compared {
    pub entries: u64,
    pub differing: u64,
    pub first: Option<String>,
    pub notes: Value,
}

/// The drawn layers whose published product stands at an hour older than `hour`: their own worker
/// holds the step and their arrays still hold the older hour's product. A product made on a
/// cadence is left out (it does not stand at the head), and so is a layer that has published
/// nothing at all: both are waited for by the line that compares them, which names what it waited
/// for and never counts a product it did not get as equal.
pub fn behind_at(side: &MapSide, hour: Hour) -> Vec<String> {
    use crate::layers::Published;
    let Some((_, layers)) = side.parts() else { return Vec::new() };
    side.layer_states()
        .into_iter()
        .zip(layers)
        .filter(|((_, draws, _), _)| *draws)
        .filter_map(|((name, ..), layer)| match layer.published() {
            Published::At(at) if at != hour => Some(format!("{name} publishes {:?}", Published::At(at))),
            _ => None,
        })
        .collect()
}
