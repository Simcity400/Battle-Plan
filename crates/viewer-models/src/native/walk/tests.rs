use crate::native::walk::*;

fn append(library: &mut ModelLibrary) {
    let slot = library.meshes.len();
    library.meshes.push(ModelMesh {
        asset: crate::AssetKey::synthetic(&(format!("route_{slot}"))),
        name: format!("route_{slot}"),
        path: String::new(),
        scale: 1.,
        lod_distances: Vec::new(),
        native: Vec::new(),
        radius: slot as f32 + 1.,
        height: 1.,
        visual: slot == 0,
        own_radius: 1.,
        bound: 1.,
        convoy_radius: None,
        attachments: 0,
        cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
    });
    library.rigs.push(Arc::new(Rig::default()));
}

#[test]
fn installed_routes_match_direct_mutation_and_demand_order_across_512_frames() {
    content_reader_script::test_game::enter();
    let mut library = ModelLibrary::default();
    for _ in 0..7 {
        append(&mut library);
    }
    library.alternatives.insert(0, 1);
    let mut pass = ModelPass::new(&library, "unit models");
    let mut random = 0xc78b_51a9u32;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        random
    };
    let mut seen = [false; 7];
    for frame in 0..512u32 {
        let roll = next();
        if frame == 80 || frame == 240 {
            append(&mut library);
            library
                .alternatives
                .insert(library.meshes.len() - 2, library.meshes.len() - 1);
            pass.sync(&library);
            seen[0] = true;
        }
        let placements: Vec<_> = (0..roll % 121)
            .map(|index| {
                let mesh = (index as usize * 3 + roll as usize) % library.meshes.len();
                (
                    mesh,
                    ModelInstance {
                        position: [(index % 7) as f32 * 93., 0., (index % 11) as f32 * 75.],
                        animation_seed: index % 5,
                        animation_epoch: index % 3,
                        animation_identity: if index % 3 == 0 { 9 } else { 0 },
                        animation: [
                            entity_animation::STOPPED,
                            entity_animation::DEFAULT_STATE,
                            17,
                        ][index as usize % 3],
                        scale: 1.,
                        roll: if frame % 7 == 0 && index % 4 == 0 {
                            0.25
                        } else {
                            0.
                        },
                        ..Default::default()
                    },
                )
            })
            .collect();
        if frame.is_multiple_of(3) {
            pass.update_placed(placements, 128.);
            seen[1] = true;
        } else {
            let mut field = ModelField::build(&library, placements, 128.);
            if frame.is_multiple_of(5) && !field.native_order.is_empty() {
                field.native_order.reverse();
                field.native_order.push(field.native_order[0]);
                seen[2] = true;
            }
            pass.replace_field(&library, &mut field);
        }
        if frame == 101 {
            pass.falling_back
                .borrow_mut()
                .insert(0, "synthetic deferred draw refusal".into());
            pass.fall_back();
            seen[3] = true;
        }
        let hidden = pass
            .field
            .native_order
            .iter()
            .filter_map(|&(group, index)| {
                let instance = pass.field.instances[index];
                let (seed, identity) = instance.animation_key();
                ((index as u32 + frame).is_multiple_of(4)).then_some((
                    pass.field.groups[group].mesh,
                    seed,
                    identity,
                ))
            })
            .collect();
        pass.set_hidden(hidden);
        if frame.is_multiple_of(17) {
            pass.reset_animation();
        }
        for &(group, index) in &pass.field.native_order {
            let mesh = pass.drawn_slot(pass.field.groups[group].mesh);
            let (seed, identity) = pass.field.instances[index].animation_key();
            if (index as u32 + frame).is_multiple_of(3) {
                pass.players
                    .entry((mesh, seed, identity))
                    .or_insert_with(|| entity_animation::Player::new(&library.rigs[mesh]));
            }
        }
        assert!(pass.native_walk.installed);
        assert_eq!(pass.native_walk.rows.len(), pass.field.native_order.len());
        let routes: Vec<_> = pass.native_walk.routes(&pass.field)
            .map(|(group, instance, row)| {
                assert!(row.is_some());
                (group, instance)
            }).collect();
        assert_eq!(routes, pass.field.native_order);
        pass.native_walk.installed = false;
        let scalar_routes: Vec<_> = pass.native_walk.routes(&pass.field)
            .map(|(group, instance, row)| {
                assert!(row.is_none());
                (group, instance)
            }).collect();
        assert_eq!(scalar_routes, pass.field.native_order);
        pass.native_walk.installed = true;
        for (row, &(group_index, index)) in
            pass.native_walk.rows.iter().zip(&pass.field.native_order)
        {
            let group = &pass.field.groups[group_index];
            let instance = pass.field.instances[index];
            let (seed, identity) = instance.animation_key();
            assert_eq!(
                (row.group, row.instance, row.mesh),
                (group_index, index, group.mesh)
            );
            assert_eq!(row.library, pass.shader_library(group.mesh));
            assert_eq!(
                row.hidden,
                pass.hidden.contains(&(group.mesh, seed, identity))
            );
            let key = pass.native_walk.players[row.player];
            assert_eq!(key, (pass.drawn_slot(group.mesh), seed, identity));
            let a = pass.player(group.mesh, &instance);
            let b = pass.players.get(&key);
            assert!(match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => std::ptr::eq(a, b),
                _ => false,
            });
            seen[4] |= row.hidden;
            seen[5] |= a.is_some();
        }
        let storage = pass.native_walk.rows.as_ptr();
        if let Some(instance) = pass.field.instances.first_mut() {
            instance.position[1] =
                f32::from_bits([0, 0x8000_0000, 0x7fc0_1234][frame as usize % 3]);
            instance.native_flags ^= 4;
            instance.native_basis = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        }
        for (view, offset) in [0., -500., 500.].into_iter().enumerate() {
            let visible_rect = (view != 0).then_some([-50., -50., 550., 550.]);
            let demand = |group: usize, index: usize, library: usize, hidden: bool| {
                let group = &pass.field.groups[group];
                if !visible(group, offset, visible_rect) || hidden {
                    return None;
                }
                let instance = pass.field.instances[index];
                let world = instance
                    .native_transform()
                    .map(|world| bytemuck::bytes_of(&matrix(&world)).to_vec())
                    .map_err(|why| why.to_string());
                Some((index, group.mesh, library, world))
            };
            let direct: Vec<_> = pass
                .field
                .native_order
                .iter()
                .filter_map(|&(group, index)| {
                    let mesh = pass.field.groups[group].mesh;
                    let (seed, identity) = pass.field.instances[index].animation_key();
                    demand(
                        group,
                        index,
                        pass.shader_library(mesh),
                        pass.hidden.contains(&(mesh, seed, identity)),
                    )
                })
                .collect();
            let installed: Vec<_> = pass
                .native_walk
                .rows
                .iter()
                .filter_map(|row| demand(row.group, row.instance, row.library, row.hidden))
                .collect();
            assert_eq!(direct, installed, "frame {frame} view {view}");
            seen[6] |= direct.iter().any(|row| row.3.is_err());
        }
        assert_eq!(storage, pass.native_walk.rows.as_ptr());
        assert!(
            pass.native.borrow().is_none(),
            "route installation cannot demand shaders/uploads"
        );
        assert!(
            pass.static_actors.borrow().is_empty(),
            "route installation cannot sample poses"
        );
        assert!(pass.refusals.borrow().1.is_empty());
    }
    assert!(
        seen.into_iter().all(|flag| flag),
        "all mutation/demand cases exercised"
    );
}

