use super::Geography;
use std::io;
use viewer_rail::railways::{Bridge, BridgeKind, RailNetwork, RailRun};

pub struct Stub;

impl Geography for Stub {
    fn cutoff(&self) -> f32 {
        300.0
    }

    fn run(&self, network: &RailNetwork, province: u32) -> io::Result<Option<RailRun>> {
        let Some(neighbours) = network.graph.get(&province) else { return Ok(None) };
        let port = |id: u32| f32::from(u8::from(network.ports.contains(&id)));
        let mut vertices = Vec::new();
        for (&a, &level) in neighbours {
            vertices.push([province as f32, a as f32, level as f32, f32::from(u8::from(network.sources.contains(&province))), port(a), port(province)]);
            for &b in neighbours.keys().filter(|&&b| b > a) {
                let triangle = network.graph.get(&b).is_some_and(|other| other.contains_key(&a));
                vertices.push([province as f32, a as f32, b as f32, f32::from(u8::from(triangle)), port(b), -1.0]);
            }
        }
        Ok(Some(RailRun { segments: neighbours.len(), unanchored: 0, samples: vertices.len(), vertices }))
    }

    fn bridge(&self, network: &RailNetwork, key: (u32, u32)) -> Option<Bridge> {
        let port = |id: u32| f32::from(u8::from(network.ports.contains(&id)));
        ((key.0 + key.1) % 2 == 1).then(|| Bridge { position: [key.0 as f32, key.1 as f32, port(key.0) + 2.0 * port(key.1)], rotation: 0.0, kind: BridgeKind::Small })
    }
}
