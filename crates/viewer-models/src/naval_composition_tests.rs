
use crate::*;

fn floats(bytes: &mut Vec<u8>, name: &str, values: &[f32]) {
    bytes.extend([b'!', name.len() as u8]);
    bytes.extend(name.as_bytes());
    bytes.push(b'f');
    bytes.extend((values.len() as u32).to_le_bytes());
    bytes.extend(values.iter().flat_map(|value| value.to_le_bytes()));
}

#[test]
fn binary_locator_bake_keeps_actor_scale_without_double_import_scale() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_naval_locator_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut bytes = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    floats(&mut bytes, "p", &[0., 0., 0., 1., 0., 0., 0., 1., 0.]);
    floats(&mut bytes, "n", &[0., 1., 0., 0., 1., 0., 0., 1., 0.]);
    bytes.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    bytes.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    bytes.extend(b"[locator\0[[mount\0");
    floats(&mut bytes, "p", &[1., 0., 0.]);
    floats(&mut bytes, "q", &[0., 0., 0., 1.]);
    std::fs::write(root.join("body.mesh"), bytes).unwrap();
    let descriptors = mesh::parse_descriptors(
        br#"
        objectTypes = {
            pdxmesh = { name = parent_mesh file = "body.mesh" scale = 3 }
            pdxmesh = { name = child_mesh file = "body.mesh" scale = 1 }
        }
        entity = { name = parent pdxmesh = parent_mesh scale = 2 attach = { mount = child } }
        entity = { name = child pdxmesh = child_mesh scale = 5 }
    "#,
        "gfx/synthetic.gfx",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let library = ModelLibrary::load(
        &layers,
        Decode::default(),
        &descriptors,
        &["parent".into()],
        &mut Vec::new(),
    )
    .unwrap();
    std::fs::remove_file(root.join("body.mesh")).unwrap();
    std::fs::remove_dir(&root).unwrap();
    let slot = library.slot("parent").unwrap();
    assert_eq!(library.meshes[slot].attachments, 1);
    let rig = library.rig(slot);
    let child = &rig.nodes[1];
    assert_eq!(
        library.palette[child.rigid as usize].rows[0],
        [10., 0., 0., 6.]
    );
    let actors = crate::mesh_pose::actors(rig, None, true);
    let world = actors[1].world.as_ref().unwrap();
    assert_eq!(world[0], [10., 0., 0., 0.]);
    assert_eq!(world[3], [6., 0., 0., 1.]);
}

#[test]
fn raw_clone_refuses_instead_of_guessing_a_registration_snapshot() {
    let descriptors = mesh::parse_descriptors(
        br#"
        entity = { name = base locator = { name = mount position = { 1 0 0 } } }
        entity = { name = copy clone = base locator = { name = mount position = { 9 0 0 } } }
    "#,
        "gfx/raw.asset",
    )
    .unwrap();
    let mut warnings = Vec::new();
    let library = ModelLibrary::load(
        &ResolvedLayers::default(),
        Decode::default(),
        &descriptors,
        &["copy".into()],
        &mut warnings,
    )
    .unwrap();
    assert!(library.slot("copy").is_none());
    assert!(
        warnings
            .iter()
            .any(|(_, reason)| reason.contains("native registration snapshot"))
    );
}
