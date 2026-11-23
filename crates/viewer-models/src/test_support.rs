use crate::*;

pub(crate) fn library_with_one_mesh(radius: f32) -> ModelLibrary {
    ModelLibrary {
        meshes: vec![ModelMesh {
            asset: crate::AssetKey::synthetic("m"),
            name: "m".into(),
            path: "m.mesh".into(),
            scale: 1.0,
            lod_distances: Vec::new(),
            native: Vec::new(),
            radius,
            height: 1.0,
            visual: false,
            own_radius: radius,
            bound: radius.hypot(1.0),
            convoy_radius: None,
            attachments: 0,
            cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
        }],
        ..ModelLibrary::default()
    }
}

pub fn lifetime_fixture() -> (ModelLibrary, ModelPass) {
    let mut library = library_with_one_mesh(1.0);
    library.palette = vec![PaletteJoint::IDENTITY];
    library.rigs.push(Arc::new(Rig {
        start: 0,
        end: 1,
        nodes: vec![entity_animation::Node {
            native: None,
            mesh: Arc::new(mesh::Mesh::default()),
            clips: vec![entity_animation::Clip {
                state: mesh::EntityState {
                    name: "idle".into(),
                    chance: 1.0,
                    speed: 1.0,
                    events: vec![mesh::EntityEvent {
                        time: 0.2,
                        trigger_once: true,
                        particle: Some("synthetic".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                animation: None,
            }],
            default: "idle".into(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
    }));
    let pass = ModelPass::new(&library, "synthetic unit lifetime");
    (library, pass)
}

pub fn extend_lifetime_palette(library: &mut ModelLibrary, pass: &mut ModelPass) {
    library.palette.push(PaletteJoint::IDENTITY);
    pass.sync(library);
}

pub fn lifetime_observation(pass: &ModelPass) -> (Vec<ModelInstance>, Vec<f32>) {
    (
        pass.field.instances.clone(),
        pass.events.iter().map(|event| event.age).collect(),
    )
}

pub fn triangle_mesh(reach: f32) -> Vec<u8> {
    let mut bytes = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [0_f32, 0., 0., 1., 0., 0., 0., 0., reach]),
        ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
    ] {
        bytes.extend([b'!', name.len() as u8]);
        bytes.extend(name.as_bytes());
        bytes.push(b'f');
        bytes.extend((values.len() as u32).to_le_bytes());
        bytes.extend(values.into_iter().flat_map(f32::to_le_bytes));
    }
    bytes.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    bytes.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    bytes
}

pub fn visual_stack(
    name: &str,
    files: &[(&str, &[u8])],
) -> (std::path::PathBuf, ResolvedLayers) {
    let root = std::env::temp_dir().join(format!(
        "hoi4_model_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (relative, bytes) in files {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let art = root.join("art").display().to_string().replace('\\', "/");
    std::fs::create_dir_all(root.join("user/mod")).unwrap();
    std::fs::write(
        root.join("user/mod/art.mod"),
        format!("name=\"Art\"\npath=\"{art}\"\n"),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
    let options = content_reader_layers::ResolveOptions {
        game_dir: root.join("steamapps/common/Game"),
        user_dir: root.join("user"),
        cache_dir: root.join("cache"),
    };
    let base = root.join("base");
    let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
    let selected = [content_reader_layers::content::visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    }];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel).unwrap();
    (root, layers)
}

pub const MESHES_GFX: &[u8] = br#"objectTypes = {
    pdxmesh = { name = body file = "gfx/models/plane.mesh" scale = 1 }
    pdxmesh = { name = hull file = "gfx/models/tank.mesh" scale = 1 }
}"#;

pub const PLANE_ASSET: &[u8] =
    br#"entity = { name = plane pdxmesh = body scale = 1 state = { name = idle } }"#;

pub fn animated_root_fixture(
    scale: f32,
    label: &'static str,
    instance: ModelInstance,
) -> (ModelPass, ModelInstance) {
    let mut rig = entity_animation::pose_rig(2.0, true, true);
    let root = &mut rig.nodes[0];
    root.mesh_scale = scale.signum();
    root.scale = scale;
    let source = Arc::make_mut(root.native.as_mut().unwrap());
    source.mesh_scale = scale.signum();
    source.entity_scale = 2.0;
    for node in &mut rig.nodes {
        Arc::make_mut(node.native.as_mut().unwrap()).frozen = node.clips[0].animation.clone();
    }
    let library = ModelLibrary {
        rigs: vec![Arc::new(rig)],
        meshes: vec![ModelMesh {
            asset: AssetKey::synthetic("unit"),
            name: "unit".into(),
            path: String::new(),
            scale,
            lod_distances: Vec::new(),
            native: Vec::new(),
            radius: 1.0,
            height: 1.0,
            visual: false,
            own_radius: 1.0,
            bound: 1.0,
            convoy_radius: None,
            attachments: 0,
            cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
        }],
        ..Default::default()
    };
    let mut pass = ModelPass::new(&library, label);
    pass.update_placed(vec![(0, instance)], 256.0);
    pass.animate(0.5, None, &[0.0]);
    let drawn = pass.field.instances[0];
    (pass, drawn)
}
