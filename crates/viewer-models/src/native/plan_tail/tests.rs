use crate::native::plan_tail::*;

#[test]
fn caller_selection_preserves_payload_bits_and_deferred_errors() {
    let mut source = mesh_pose::Source {
        pdxmesh: true, skeleton: Vec::new(), entity_locators: Vec::new(),
        binary_locators: Vec::new(), mesh_scale: 1., entity_scale: 1.,
        frozen: None, game_data: None,
    };
    let labels = ["city sprawl", "railway bridges", "map buildings", "flying aircraft",
        "map frame", "unit models", "supply trains", "unavailable caller"];
    for authored in [None, Some(Err("synthetic authored refusal".to_owned())),
        Some(Ok(mesh::EntityGameData { atlas_size: [i32::MIN, -4], texture_anim_speed: f32::from_bits(0x7fc01234) })),
        Some(Ok(mesh::EntityGameData { atlas_size: [0, 2], texture_anim_speed: -0. })),
        Some(Ok(mesh::EntityGameData { atlas_size: [8, 4], texture_anim_speed: 1.25 }))]
    {
        source.game_data = authored;
        for label in labels {
            let caller = Caller::select(label);
            for flags in 0..8 {
                for alpha in [0., -0., 0.75, f32::from_bits(0x7fc05678)] {
                    let instance = ModelInstance {
                        native_flags: flags,
                        train_clip: [-0., f32::from_bits(0xffc01234), 1., -2.],
                        ..Default::default()
                    };
                    let expected = reference_userdata(label, &source, instance, alpha);
                    let actual = caller.evaluate(&source, instance, alpha);
                    match (expected, actual) {
                        (Ok(a), Ok(b)) => assert_eq!(a.map(|v| v.map(f32::to_bits)), b.map(|v| v.map(f32::to_bits))),
                        (Err(a), Err(b)) => assert_eq!((a.kind(), a.to_string()), (b.kind(), b.to_string())),
                        _ => panic!("different caller outcome: {label}, flags {flags}"),
                    }
                }
            }
        }
    }
}

#[test]
fn compact_supplies_and_reflected_recipes_preserve_slot_order_and_absence() {
    let bytes: [&[u8]; 5] = [&[0, 1], &[2, 3, 4], &[5], &[6, 7, 8, 9], &[]];
    for mask in 0..8 {
        let pixel = (mask & 1 != 0).then_some(bytes[2]);
        let skin = (mask & 2 != 0).then_some(bytes[3]);
        let grid = (mask & 4 != 0).then_some(bytes[4]);
        let supplies = Supplies::new(bytes[0], bytes[1], pixel, skin, grid);
        let mut expected: Constants = [None; CONSTANT_BINDINGS];
        expected[0] = Some(bytes[0]); expected[1] = Some(bytes[0]);
        expected[2] = Some(bytes[1]); expected[3] = pixel; expected[4] = skin; expected[5] = grid;
        assert_eq!(supplies.constants(), expected);
        let present = expected.iter().enumerate().fold(0, |m, (i, b)| m | if b.is_some() { 1 << i } else { 0 });
        assert_eq!(supplies.present(), present);
        for binding in 0..CONSTANT_BINDINGS as u32 + 4 {
            assert_eq!(supplies.get(binding), expected.get(binding as usize).copied().flatten());
        }
        for bindings in [vec![], vec![5, 2, 0, 4, 1, 3], vec![3, 31, 0], vec![1, 0, 1]] {
            let recipe = Recipe::from_bindings(bindings.iter().copied()).unwrap();
            assert_eq!(recipe.sources.len(), bindings.len());
            let got: Vec<_> = recipe.ordered(supplies).collect();
            let expected: Vec<_> = bindings.iter().map(|b| supplies.get(*b)).collect();
            assert_eq!(got, expected);
        }
    }
}

fn reference_userdata(
    label: &str,
    source: &mesh_pose::Source,
    instance: ModelInstance,
    alpha: f32,
) -> io::Result<Option<[f32; 8]>> {
    let mut data = [0., 0., 1., 1., 1., 0., 0., 0.];
    match label {
        "city sprawl" | "railway bridges" => return Ok(None),
        "map buildings" | "flying aircraft" | "map frame" => {}
        "unit models" => {
            if instance.native_flags & 1 != 0
                && let Some(authored) = &source.game_data
            {
                let authored = authored.as_ref().map_err(|e| invalid(e.clone()))?;
                data[0] = authored.atlas_size[0].wrapping_sub(1).min(3) as f32;
                data[2] = authored.atlas_size[0] as f32;
                data[3] = authored.atlas_size[1] as f32;
                data[4] = if instance.native_flags & 2 != 0 {
                    authored.texture_anim_speed
                } else {
                    0.
                };
            }
        }
        "supply trains" => {
            data = [
                1.,
                1.,
                1.,
                alpha,
                instance.train_clip[0],
                instance.train_clip[1],
                instance.train_clip[2],
                instance.train_clip[3],
            ];
        }
        _ => {
            return Err(invalid(format!(
                "native mesh userdata caller {label:?} unavailable"
            )));
        }
    }
    Ok(Some(data))
}
