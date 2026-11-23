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

pub fn behind_at(side: &MapSide<impl crate::playhead_host::PlaybackProducts>, hour: Hour) -> Vec<String> {
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
