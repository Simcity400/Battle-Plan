use crate::*;

pub const ORDERS: [[&str; 3]; 3] = [
    ["zeta", "alpha", "middle"],
    ["middle", "alpha", "zeta"],
    ["alpha", "zeta", "middle"],
];

pub fn library(order: [&str; 3], native: bool) -> ModelLibrary {
    let descriptors = mesh::parse_descriptors(
        b"entity = { name = zeta } entity = { name = alpha } entity = { name = middle }",
        "synthetic.asset",
    )
    .unwrap();
    let shards = std::thread::scope(|scope| {
        let jobs: Vec<_> = order
            .into_iter()
            .map(|name| {
                let descriptors = &descriptors;
                scope.spawn(move || {
                    let mut shard = ModelLibrary::load(
                        &ResolvedLayers::default(),
                        Decode::default(),
                        descriptors,
                        &[name.into()],
                        &mut Vec::new(),
                    )
                    .unwrap();
                    let mut rig = entity_animation::pose_rig(1., false, native);
                    for clip in &mut rig.nodes[0].clips {
                        clip.state.start = 0.;
                        clip.state.speed = 1.;
                        clip.state.events[0].time = 0.;
                        clip.state.events[0].node.clear();
                        clip.state.events[0].sound = Some("limited".into());
                        clip.state.events[0].light = Some("lamp".into());
                    }
                    shard.palette = vec![PaletteJoint::IDENTITY; rig.end];
                    shard.rigs = vec![Arc::new(rig)];
                    shard
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut library = ModelLibrary::default();
    for shard in shards {
        library.merge(shard);
    }
    library
}

pub fn placed(library: &ModelLibrary) -> Vec<(usize, ModelInstance)> {
    ["zeta", "alpha", "middle", "alpha", "zeta", "middle"]
        .into_iter()
        .enumerate()
        .map(|(at, name)| {
            (
                library.slot(name).unwrap(),
                ModelInstance {
                    position: [4. + at as f32 * 0.1, 0., 4.],
                    scale: 1.,
                    animation: entity_animation::IDLE,
                    animation_seed: 70 + at as u32,
                    animation_identity: 100 + at as u32,
                    animation_epoch: 3,
                    ..Default::default()
                },
            )
        })
        .collect()
}

pub fn events(order: [&str; 3], retained: bool) -> Vec<entity_animation::Event> {
    let library = library(order, true);
    let mut pass = ModelPass::new(&library, if retained { "unit models" } else { "general" });
    pass.update_placed(placed(&library), 256.);
    pass.animate(1., None, &[0.]);
    pass.events
        .into_iter()
        .map(|mut event| {
            event.key.pass = 0;
            event
        })
        .collect()
}
