
use super::*;

#[test]
fn cloned_locator_array_keeps_duplicates_origin_and_declaration_time_snapshot() {
    let document = parser::parse(
        br#"
        entity = { name = base pdxmesh = original
            locator = { name = same position = { 1 0 0 } parent_joint = mount }
            locator = { name = same position = { 2 0 0 } scale = 0 }
        }
        entity = { name = copy clone = base pdxmesh = replacement
            locator = { name = child_only position = { 9 0 0 } }
        }
        entity = { name = base pdxmesh = original
            locator = { name = same position = { 7 0 0 } }
        }
    "#,
    )
    .unwrap();
    let mut roster = Roster::default();
    for name in ["original", "replacement"] {
        roster.descriptors.meshes.insert(
            name.into(),
            MeshDefinition {
                name: name.into(),
                ..Default::default()
            },
        );
    }
    let mut registry = Entities::default();
    for item in &document.items {
        registry
            .declare(
                &item.value,
                &content_reader_layers::script_files::ScriptPath::from("gfx/entities/registered.asset"),
                &mut roster,
            )
            .unwrap();
    }
    let copy = &registry.0["copy"].0;
    assert_eq!(copy.clone, None);
    assert_eq!(copy.mesh.as_deref(), Some("replacement"));
    assert_eq!(copy.locators.len(), 2);
    assert_eq!(copy.locators[0].0, "same");
    assert_eq!(copy.locators[1].0, "same");
    assert_eq!(copy.locators[0].1.transform.translation[0], 1.);
    assert_eq!(copy.locators[1].1.transform.translation[0], 2.);
    assert_eq!(copy.locators[0].1.parent_joint.as_deref(), Some("mount"));
    assert_eq!(copy.locators[0].1.origin_mesh.as_deref(), Some("original"));
    assert_eq!(copy.locators[1].1.origin_mesh.as_deref(), Some("original"));
    assert_eq!(copy.locators[1].1.transform.basis, [[0.; 3]; 3]);
    assert_eq!(
        registry.0["base"].0.locators[0].1.transform.translation[0],
        7.
    );
}

#[test]
fn a_meshless_parent_clone_keeps_unparented_locator_signed_scale() {
    let document = parser::parse(
        br#"
        entity = { name = root
            locator = { name = position position = { 3 0 0 } scale = -2 }
        }
        entity = { name = copy clone = root pdxmesh = body
            locator = { name = ignored position = { 99 0 0 } }
        }
    "#,
    )
    .unwrap();
    let mut roster = Roster::default();
    roster.descriptors.meshes.insert(
        "body".into(),
        MeshDefinition {
            name: "body".into(),
            ..Default::default()
        },
    );
    let mut registry = Entities::default();
    for item in &document.items {
        registry
            .declare(
                &item.value,
                &content_reader_layers::script_files::ScriptPath::from("gfx/entities/meshless.asset"),
                &mut roster,
            )
            .unwrap();
    }
    let copy = &registry.0["copy"].0;
    assert_eq!(copy.locators.len(), 1);
    assert_eq!(copy.locators[0].0, "position");
    assert_eq!(copy.locators[0].1.origin_mesh, None);
    assert_eq!(copy.locators[0].1.parent_joint, None);
    assert_eq!(copy.locators[0].1.transform.translation[0], 3.);
    assert_eq!(
        copy.locators[0].1.transform.basis,
        [[-2., 0., 0.], [0., -2., 0.], [0., 0., -2.]]
    );
}
