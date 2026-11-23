use super::*;

pub(super) fn test_force(kind: ForceKind, position: [f32; 3], direction: [f32; 3], amount: f32) -> Force {
    Force {
        kind,
        amount: Parameter {
            base: amount,
            ..Parameter::default()
        },
        position,
        direction,
        yaw: [0.0; 2],
        local: false,
    }
}

pub fn long_lived_test_emitter() -> Emitter {
    let mut emitter = activation_test_emitter();
    emitter.life.base = 10.;
    emitter
}

pub fn activation_test_emitter() -> Emitter {
    let scalar = |base: f32| Parameter {
        base,
        spread: 0.0,
        curve: None,
    };
    Emitter {
        scale: 1.0,
        page: 0,
        registration: 0,
        actor: 0,
        radius: scalar(0.0),
        sphere_pitch: scalar(0.0),
        sphere_yaw: scalar(0.0),
        emitter_yaw: scalar(0.0),
        emitter_pitch: scalar(0.0),
        position: [scalar(0.0), scalar(0.0), scalar(0.0)],
        velocity: scalar(10.0),
        pitch: scalar(0.0),
        yaw: scalar(0.0),
        life: scalar(3.0),
        mass: scalar(1.0),
        start: scalar(0.0),
        duration: scalar(-1.0),
        size: scalar(5.0),
        rotation: [scalar(0.0), scalar(0.0), scalar(0.0)],
        spin: [scalar(0.0), scalar(0.0), scalar(0.0)],
        color: [scalar(200.0), scalar(200.0), scalar(200.0), scalar(255.0)],
        emission: scalar(60.0),
        max: 100,
        forces: vec![test_force(ForceKind::Planar, [0.0, 0.0, 0.0], [0.0, -9.0, 0.0], 1.0)],
        noise_divisions: Vec::new(),
        leads: true,
        delay: 0.0,
        local_space: false,
        render: content_reader_mesh::particles::RenderDescriptor::read(&[]).unwrap(),
        system_local_space: false,
    }
}
