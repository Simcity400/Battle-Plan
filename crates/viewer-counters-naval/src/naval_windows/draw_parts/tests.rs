use super::{Layout, native_gui, place};
use content_reader_gui::gui::native;
use std::sync::Arc;
use viewer_native_gui::{
    native_gui::{Part, PartKind, lifecycle::{Call, Instance, Tick}},
    native_shader::draw,
    sprites::Batch,
    test_support::gui::fixture,
};
use shader_pipeline::library;

const VIEWPORT: [i32; 2] = [800, 600];
const GUI: &[u8] = br#"guiTypes={
containerWindowType={name=unit_counter_navy size={width=80 height=50}
 fade_type=linear fade_time=100 moveable=yes
 show_animation_type=linear show_animation_time=250 show_position={x=20 y=30}
 hide_animation_type=linear hide_animation_time=250 hide_position={x=-20 y=-30}
 iconType={name=type spriteType=GFX_animated}
 gridBoxType={name=wings_box position={x=3 y=9} size={width=45 height=20}
   slotsize={width=10 height=10} format=UPPER_LEFT}
 instantTextBoxType={name=text font=Main_14 position={x=0 y=20} maxWidth=60 maxHeight=28 text=MOD_KEY}
 iconType={name=after spriteType=GFX_odd position={x=30 y=3}}
 gridBoxType={name=tail position={x=8 y=15} size={width=30 height=10}
   slotsize={width=10 height=10} format=UPPER_LEFT}
}
containerWindowType={name=wing size={width=16 height=10}
 iconType={name=type spriteType=GFX_even}
 instantTextBoxType={name=text font=Main_14 text=MOD_KEY}
}
containerWindowType={name=naval_base_mapicon size={width=90 height=70}
 fade_type=linear fade_time=100
 iconType={name=type spriteType=GFX_even}
 instantTextBoxType={name=text font=Main_14 text=MOD_KEY}
 gridBoxType={name=box_ships position={x=4 y=31} size={width=80 height=40}
   slotsize={width=20 height=10} format=UPPER_LEFT}
}}"#;

fn states(views: &[([Instance; 2], &native::Element)], side: usize) -> Vec<String> {
    views
        .iter()
        .map(|(pair, _)| format!("{:?}", pair[side]))
        .collect()
}

fn reference_place(
    holder: &native_gui::Holder,
    at: [i32; 2],
    layout: &Layout,
    composed: &[Vec<Part>],
) -> Vec<Part> {
    let mut composed_layout = layout.clone();
    composed_layout.parts = layout.composed_parts(composed).cloned().collect();
    holder.place(at, &composed_layout)
}

fn native_asset() -> Arc<library::Effect> {
    let program =
        content_reader_gui::pdxshader::program::read_with(b"gfx/FX/naval_draw_fixture.lua", &|_| {
            Some(b"PixelShader={MainCode P [[ ]]}".to_vec())
        })
        .unwrap();
    library::Effect::from_compiled(Arc::new(program), Vec::new(), Vec::new())
}

fn carry_native(layout: &mut Layout, asset: &Arc<library::Effect>) {
    for part in &mut layout.parts {
        if matches!(part.kind, PartKind::Sprite { .. } | PartKind::Cell { .. }) {
            part.shading.compiled = Some(draw::Effect {
                asset: Arc::clone(asset),
                technique: "Absent".into(),
            });
        }
    }
}

fn assert_native_bits(a: &draw::Quad, b: &draw::Quad) {
    assert_eq!(a, b);
    assert_eq!(a.vertices, b.vertices);
    let bits = |values: &[f32]| {
        values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&a.constants), bits(&b.constants));
    for ((_, a, _), (_, b, _)) in a.extra_constants.iter().zip(&b.extra_constants) {
        assert_eq!(bits(a), bits(b));
    }
    if let (Some(a), Some(b)) = (&a.bake, &b.bake) {
        assert_eq!(a.size, b.size);
        assert_eq!(a.draws.len(), b.draws.len());
        for (a, b) in a.draws.iter().zip(&b.draws) {
            assert_native_bits(a, b);
        }
    }
}

fn assert_batch_bits(a: &Batch, b: &Batch) {
    assert_eq!(a.quads, b.quads);
    for (a, b) in a.quads.iter().zip(&b.quads) {
        assert_eq!(a.rect.map(f32::to_bits), b.rect.map(f32::to_bits));
        assert_eq!(a.uv.map(f32::to_bits), b.uv.map(f32::to_bits));
        assert_eq!(a.color.map(f32::to_bits), b.color.map(f32::to_bits));
        assert_eq!(a.rotation.map(f32::to_bits), b.rotation.map(f32::to_bits));
        assert_eq!(a.shade.to_bits(), b.shade.to_bits());
        if let (Some(a), Some(b)) = (a.anchor, b.anchor) {
            assert_eq!(a.world.map(f32::to_bits), b.world.map(f32::to_bits));
            assert_eq!(a.cutoff.to_bits(), b.cutoff.to_bits());
        }
        if let (Some(a), Some(b)) = (a.present, b.present) {
            assert_eq!(a.pivot.map(f32::to_bits), b.pivot.map(f32::to_bits));
            assert_eq!(a.scale.to_bits(), b.scale.to_bits());
            assert_eq!(a.alpha.to_bits(), b.alpha.to_bits());
        }
        if let (Some(a), Some(b)) = (&a.exact, &b.exact) {
            assert_eq!(
                a.corners.map(|p| p.map(f32::to_bits)),
                b.corners.map(|p| p.map(f32::to_bits))
            );
            assert_eq!(
                a.uv.map(|p| p.map(f32::to_bits)),
                b.uv.map(|p| p.map(f32::to_bits))
            );
            assert_eq!(a.lod_bias.map(f32::to_bits), b.lod_bias.map(f32::to_bits));
            assert_eq!(a.offsets.map(f32::to_bits), b.offsets.map(f32::to_bits));
            assert_eq!(a.inputs.map(f32::to_bits), b.inputs.map(f32::to_bits));
            assert_eq!(
                a.second_color.map(f32::to_bits),
                b.second_color.map(f32::to_bits)
            );
            if let (Some(a), Some(b)) = (&a.native, &b.native) {
                assert_native_bits(a, b);
            }
        }
    }
}

#[test]
fn borrowed_naval_parts_keep_1200_frames_of_draw_bytes_and_carried_state() {
    content_reader_script::test_game::enter();
    let gui = native::read_at(GUI, VIEWPORT);
    let navy = gui.template("unit_counter_navy").unwrap();
    let wing = gui.template("wing").unwrap();
    let base = gui.template("naval_base_mapicon").unwrap();
    let mut resources = fixture();
    let mut views: Vec<_> = [navy, navy, wing, base]
        .into_iter()
        .map(|root| {
            let instance = Instance::construct(root, VIEWPORT, &resources).unwrap();
            ([instance.clone(), instance], root)
        })
        .collect();
    let asset = native_asset();
    let mut time = 0.0;
    let mut native_quads = 0;
    let mut layouts_refused = 0;
    let mut draws_refused = 0;
    let mut composed_items = 0;
    let mut empty_compositions = 0;
    let mut outputs_changed = 0;
    let mut prior = Batch::default();
    for frame in 0u32..1200 {
        let phase = frame % 120;
        let dt = if (80..90).contains(&phase) {
            0.0
        } else {
            1.0 / 60.0
        };
        time += dt;
        if phase == 100 {
            time = 0.0;
        }
        if phase == 0 && frame != 0 {
            let instance = Instance::construct(navy, VIEWPORT, &resources).unwrap();
            views[0].0 = [instance.clone(), instance];
        }
        resources.sprites.get_mut("GFX_odd").unwrap().refusals = if (46..49).contains(&phase) {
            vec![(
                native_gui::RefusalKind::MissingResource,
                "fixture sprite temporarily unavailable".into(),
            )]
        } else {
            Vec::new()
        };
        for (role, (pair, root)) in views.iter_mut().enumerate() {
            let icon = root.find(native::Kind::Icon, "type").unwrap();
            let text = root.find(native::Kind::TextBox, "text").unwrap();
            let calls = [
                (
                    root.path.clone(),
                    if (35..40).contains(&phase) {
                        Call::Hide
                    } else {
                        Call::GuiShow
                    },
                ),
                (
                    icon.path.clone(),
                    Call::SetFrame((frame / 13 % 3 + 1) as i32),
                ),
                (
                    icon.path.clone(),
                    Call::SetSprite(
                        if phase == 51 {
                            b"GFX_undefined".as_slice()
                        } else if role == 2 {
                            b"GFX_even".as_slice()
                        } else {
                            b"GFX_animated".as_slice()
                        }
                        .to_vec(),
                        native_gui::SpriteSetter::Icon,
                    ),
                ),
                (
                    text.path.clone(),
                    Call::SetText(if phase < 60 { "MOD_LONG" } else { "MOD_KEY" }.into()),
                ),
            ];
            for instance in pair {
                viewer_counter_kit::gui_windows::apply(instance, root, &calls, &resources).unwrap();
                instance
                    .set_position(root, root, [((frame / 7) % 17) as i32 - 8, role as i32 * 3])
                    .unwrap();
                instance.control(root, root).unwrap().scale =
                    Some(if phase < 60 { 1.0 } else { 1.25 });
            }
        }
        for side in [0, 1] {
            for (pair, root) in &mut views {
                pair[side]
                    .tick_clocks(root, &resources, Tick { time, dt })
                    .unwrap();
            }
            let mut order: Vec<_> = (0..views.len()).collect();
            order.sort_by_key(|&at| std::cmp::Reverse(views[at].0[side].registration()));
            for at in order {
                let (pair, root) = &mut views[at];
                pair[side].tick_transitions(root).unwrap();
            }
            for (pair, root) in &mut views {
                pair[side].tick_draws(root, &resources).unwrap();
            }
        }
        let before = [states(&views, 0), states(&views, 1)];
        assert_eq!(before[0], before[1], "carried state frame {frame}");
        let mut batches: [Batch; 2] = std::array::from_fn(|_| Batch::default());
        let mut refusals: [Vec<String>; 2] = std::array::from_fn(|_| Vec::new());
        for side in [0, 1] {
            let layouts: Vec<_> = views
                .iter()
                .map(|(pair, root)| pair[side].layout_with_regions(root, &resources, None, true))
                .collect();
            for layout in &layouts {
                if let Err(why) = layout {
                    layouts_refused += 1;
                    refusals[side].push(format!("layout: {why:?}"));
                }
            }
            for role in [0, 1] {
                let Ok(Some(mut laid)) = layouts[role].clone() else {
                    continue;
                };
                carry_native(&mut laid, &asset);
                let mut composed = vec![Vec::new(); laid.holders.len()];
                if phase < 70
                    && let Ok(Some(child)) = &layouts[2]
                {
                    let mut child = child.clone();
                    carry_native(&mut child, &asset);
                    for (at, holder) in laid.holders.iter().enumerate() {
                        if at == 0 || phase.is_multiple_of(3) {
                            composed[at] = holder.place([phase as i32 % 5, -2], &child);
                            composed_items += composed[at].len();
                        }
                    }
                } else {
                    empty_compositions += 1;
                }
                let holder = if role == 0 {
                    let Ok(Some(parent)) = &layouts[3] else {
                        continue;
                    };
                    parent
                        .holders
                        .iter()
                        .find(|holder| holder.path.ends_with("box_ships"))
                        .unwrap()
                        .clone()
                } else {
                    native_gui::Holder {
                        path: "units_box".into(),
                        child: Some(2),
                        content: [90, 60],
                        origin: [phase as f32 * 0.125, -7.5],
                        scale: if phase < 60 { 0.75 } else { 1.25 },
                        hit_scales: None,
                        logical: [3, -7],
                        clip: (phase < 95).then_some([-20, -30, 500, 400]),
                        opacity: 1.0,
                        before: 0,
                        position: Some([-3, 4]),
                    }
                };
                let at = [((frame / 11) % 7) as i32 - 3, (phase % 9) as i32];
                let parts = if side == 0 {
                    reference_place(&holder, at, &laid, &composed)
                } else {
                    place(&holder, at, &laid, &composed)
                };
                let result = viewer_counter_kit::gui_windows::draw_held(
                    &holder,
                    parts,
                    [40, 55],
                    &resources,
                    &mut batches[side],
                    (10, role as u32),
                )
                .unwrap();
                if let Err(why) = result {
                    draws_refused += 1;
                    refusals[side].push(format!("draw: {why:?}"));
                }
                if phase == 75 {
                    let mut failed = laid.clone();
                    if let Some(part) = failed
                        .parts
                        .iter_mut()
                        .find(|part| matches!(part.kind, PartKind::Sprite { .. }))
                        && let PartKind::Sprite { sprite, .. } = &mut part.kind
                    {
                        *sprite = b"GFX_missing_sheet".to_vec();
                        part.native = None;
                    }
                    let parts = if side == 0 {
                        reference_place(&holder, at, &failed, &composed)
                    } else {
                        place(&holder, at, &failed, &composed)
                    };
                    let first = batches[side].quads.len();
                    let result = viewer_counter_kit::gui_windows::draw_held(
                        &holder,
                        parts,
                        [40, 55],
                        &resources,
                        &mut batches[side],
                        (10, role as u32),
                    )
                    .unwrap();
                    assert!(result.is_err());
                    assert_eq!(batches[side].quads.len(), first);
                    draws_refused += 1;
                    refusals[side].push(format!("draw: {:?}", result.unwrap_err()));
                }
            }
        }
        assert_eq!(refusals[0], refusals[1], "ordered refusals frame {frame}");
        assert_batch_bits(&batches[0], &batches[1]);
        native_quads += batches[1]
            .quads
            .iter()
            .filter(|quad| {
                quad.exact
                    .as_ref()
                    .is_some_and(|exact| exact.native.is_some())
            })
            .count();
        outputs_changed += usize::from(prior.quads != batches[1].quads);
        prior = batches[1].clone();
        assert_eq!(
            before[0],
            states(&views, 0),
            "reference draw mutated state frame {frame}"
        );
        assert_eq!(
            before[1],
            states(&views, 1),
            "borrowed draw mutated state frame {frame}"
        );
    }
    assert!(
        native_quads > 1200,
        "must compare real emitted native byte payloads"
    );
    assert!(layouts_refused > 0 && draws_refused > 0);
    assert!(composed_items > 0 && empty_compositions > 0);
    assert!(
        outputs_changed > 500,
        "must not prove only a constant paused output"
    );
}

#[test]
fn borrowed_naval_parts_keep_empty_and_same_position_holder_attach_order() {
    let holder = native_gui::Holder {
        path: "outer".into(),
        child: None,
        content: [10, 10],
        origin: [0.0; 2],
        scale: 1.0,
        hit_scales: None,
        logical: [0; 2],
        clip: None,
        opacity: 1.0,
        before: 0,
        position: None,
    };
    let child = |name: &str| Part {
        path: name.into(),
        child: None,
        background_index: None,
        kind: PartKind::Cell {
            texture: 1,
            masking: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            geometry: native_gui::Geometry {
                rect: [-0.0, 0.0, 3.0, 4.0],
                local: [0.0; 4],
                pivot: [0.0; 2],
                rotation: 0.0,
            },
        },
        clip: None,
        color: [1.0; 4],
        shading: native_gui::Shading {
            compiled: None,
            fixture: None,
            since: 0.0,
            animation: 0.0,
            time: 0.0,
            clocked: false,
        },
        native: None,
    };
    let layout = Layout {
        size: [10, 10],
        parts: vec![child("first"), child("last")],
        holders: vec![holder.clone(), holder.clone()],
        regions: Vec::new(),
        diagnostics: Vec::new(),
        pointed: None,
        hovered: None,
        target: None,
        boxes: Vec::new(),
    };
    for (items, expected) in [
        (Vec::new(), vec!["first", "last"]),
        (vec![vec![child("a")]], vec!["a", "first", "last"]),
        (
            vec![vec![], vec![child("held")]],
            vec!["held", "first", "last"],
        ),
        (
            vec![vec![child("a")], vec![child("b")], vec![child("ignored")]],
            vec!["a", "b", "first", "last"],
        ),
    ] {
        let actual = place(&holder, [-3, 4], &layout, &items);
        assert_eq!(actual, reference_place(&holder, [-3, 4], &layout, &items));
        assert_eq!(
            actual
                .iter()
                .map(|part| part.path.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        for part in &actual {
            let PartKind::Cell { geometry, .. } = &part.kind else {
                panic!("expected a cell");
            };
            assert_eq!(
                geometry.rect.map(f32::to_bits),
                [-3.0_f32, 4.0, 3.0, 4.0].map(f32::to_bits)
            );
            assert_eq!(
                geometry.pivot.map(f32::to_bits),
                [-3.0_f32, 4.0].map(f32::to_bits)
            );
        }
    }
    let items = vec![vec![child("a")], vec![child("b")]];
    for (before, expected) in [
        ([1, 1], vec!["first", "a", "b", "last"]),
        ([2, 9], vec!["first", "last", "a", "b"]),
    ] {
        let mut laid = layout.clone();
        for (holder, before) in laid.holders.iter_mut().zip(before) {
            holder.before = before;
        }
        let actual = place(&holder, [0, 0], &laid, &items);
        assert_eq!(
            actual
                .iter()
                .map(|part| part.path.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    let mut laid = layout.clone();
    laid.holders.clear();
    let actual = place(&holder, [0, 0], &laid, &items);
    assert_eq!(
        actual
            .iter()
            .map(|part| part.path.as_str())
            .collect::<Vec<_>>(),
        ["first", "last"]
    );
    laid.parts.clear();
    assert!(place(&holder, [0, 0], &laid, &items).is_empty());
    laid.holders = layout.holders.clone();
    let actual = place(&holder, [0, 0], &laid, &items);
    assert_eq!(
        actual
            .iter()
            .map(|part| part.path.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}
