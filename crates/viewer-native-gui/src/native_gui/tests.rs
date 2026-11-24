use super::*;
use viewer_gpu::gpu_contract::device_descriptor;
use crate::native_effects::synthetic;
use viewer_gpu::test_support::gpu;
use crate::test_support::gui::*;
#[cfg(test)]
use viewer_gpu::test_support::serial;
use content_reader_gui::gui::native::read;
use lifecycle::{
    Admission, Call, Calls, Held, Instance, MapIconPool, Pool, Rewrote, Tick, Unknown, Withheld,
};
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::Arc;

#[test]
fn tooltip_bake_keeps_soft_wrap_indent_and_hard_line_start() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    resources.templates.push(read(
        br#"guiTypes = { textBoxType = {
        name = ToolTip font = Main_14 maxWidth = 50 borderSize = { x = 2 y = 3 }
    } }"#,
    ));
    let layout = text_sprites::tooltip(&resources, " ab cd ef\nxy", 0.0, false).unwrap();
    let PartKind::TextSprite { bake, .. } = &layout.parts[0].kind else {
        panic!("native text bake");
    };
    let at = |ch| bake.glyphs.iter().find(|glyph| glyph.ch == ch).unwrap().at;
    assert_eq!(at('a'), [9.0, 3.0]);
    assert_eq!(at('c'), [9.0, 17.0]);
    assert_eq!(at('e'), [9.0, 31.0]);
    assert_eq!(at('x'), [2.0, 45.0]);
}

#[test]
fn tooltip_fallback_fixed_size_and_named_null_refusal() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let refused = text_sprites::tooltip(&resources, "abcdef", 0.0, false).unwrap_err();
    let mut counts = Counts::default();
    counts.refused(&refused);
    assert_eq!(counts.0["null_read"], 1);
    resources.templates.push(read(
        br#"guiTypes = { textBoxType = {
        name = defaultTextBox font = Main_14 maxWidth = 50 maxHeight = 1
        fixedsize = yes format = right borderSize = { x = 2 y = 3 }
    } }"#,
    ));
    let layout = text_sprites::tooltip(&resources, "abcdef", 0.0, false).unwrap();
    assert_eq!(layout.size, [48, 34]);
    let PartKind::TextSprite { bake, .. } = &layout.parts[0].kind else {
        panic!("native text bake");
    };
    assert_eq!(bake.glyphs[0].at[0], 2.0);
}

#[test]
fn tooltip_bakes_native_inline_icons_and_flags_and_counts_missing_inputs() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    resources.templates.push(read(
        br#"guiTypes = { textBoxType = {
        name = ToolTip font = Main_14 maxWidth = 100 borderSize = { x = 1 y = 1 }
    } }"#,
    ));
    let text = format!("{}odd|2 {}GER x", native_text::ICON, native_text::FLAG);
    let layout = text_sprites::tooltip(&resources, &text, 0.0, false).unwrap();
    let PartKind::TextSprite { bake, .. } = &layout.parts[0].kind else {
        panic!("native text bake");
    };
    assert_eq!(bake.inline.len(), 2);
    assert_eq!(bake.inline[0].0, InlineSource::Sprite(b"GFX_odd".to_vec()));
    assert_eq!(bake.inline[1].0, InlineSource::Texture(9));
    let refused =
        text_sprites::tooltip(&resources, &format!("{}XXX", native_text::FLAG), 0.0, false)
            .unwrap_err();
    let mut counts = Counts::default();
    counts.refused(&refused);
    assert_eq!(counts.0["missing_input"], 1);
    let refused = text_sprites::tooltip(
        &resources,
        &format!("{}unpreloaded", native_text::ICON),
        0.0,
        false,
    )
    .unwrap_err();
    assert!(refused.iter().any(|why| {
        why.kind == RefusalKind::MissingResource
            && why.detail.contains("GFX_unpreloaded")
            && why.detail.contains("has not been loaded")
    }));
    counts.refused(&refused);
    assert_eq!(counts.0["missing_resource"], 1);
    let layout =
        text_sprites::tooltip(&resources, &format!("{}NUL", native_text::FLAG), 0.0, false)
            .unwrap();
    counts.noted(&layout.diagnostics);
    assert_eq!(counts.0["null_text_flag"], 1);
}

#[test]
fn resolved_flag_text_reaches_native_walk_and_tooltip_only_when_bound() {
    content_reader_script::test_game::enter();
    let value = format!("{}GER-Western_group, x", native_text::RESOLVED_FLAG);
    let laid = |resolved_flags| {
        laid_bound(
            "instantTextBoxType = { name = t font = Main_14 }",
            None,
            |window, bindings| {
                let bound = bindings.bind(window.find(Kind::TextBox, "t").unwrap());
                bound.text = Some(value.clone());
                bound.resolved_flags = resolved_flags;
            },
        )
        .unwrap()
        .unwrap()
    };
    let bound = laid(true);
    assert!(matches!(&bound.parts[0].kind,
        PartKind::Inline { source: InlineSource::Texture(10), rect, .. }
        if *rect == [0.0, 5.0, 10.0, 7.0]));
    let PartKind::Text { glyphs, .. } = &bound.parts[1].kind else {
        panic!("native text");
    };
    assert_eq!(glyphs[0].ch, ',');
    assert_eq!(glyphs[0].at[0], 12.0);
    assert!(
        laid(false)
            .parts
            .iter()
            .all(|part| !matches!(part.kind, PartKind::Inline { .. }))
    );

    let mut resources = fixture();
    resources.templates.push(read(
        br#"guiTypes = { textBoxType = {
        name = ToolTip font = Main_14 maxWidth = 200 borderSize = { x = 1 y = 1 }
    } }"#,
    ));
    let tooltip = text_sprites::tooltip(&resources, &value, 0.0, true).unwrap();
    let PartKind::TextSprite { bake, .. } = &tooltip.parts[0].kind else {
        panic!("native bake");
    };
    assert_eq!(bake.inline.len(), 1);
    assert_eq!(bake.inline[0].0, InlineSource::Texture(10));
    assert_eq!(bake.inline[0].1[2..], [10.0, 7.0]);
    let authored = text_sprites::tooltip(&resources, &value, 0.0, false).unwrap();
    let PartKind::TextSprite { bake, .. } = &authored.parts[0].kind else {
        panic!("native bake");
    };
    assert!(bake.inline.is_empty());
    let refused = text_sprites::tooltip(
        &resources,
        &format!("{}UNKNOWN_key", native_text::RESOLVED_FLAG),
        0.0,
        true,
    )
    .unwrap_err();
    let mut counts = Counts::default();
    counts.refused(&refused);
    assert_eq!(counts.0["missing_input"], 1);
    assert!(refused[0].detail.contains("resolved atlas key UNKNOWN_key"));
    for key in ["X", ""] {
        let refused = text_sprites::tooltip(
            &resources,
            &format!("{}{key}", native_text::RESOLVED_FLAG),
            0.0,
            true,
        )
        .unwrap_err();
        let mut counts = Counts::default();
        counts.refused(&refused);
        assert_eq!(counts.0["missing_input"], 1);
        assert!(refused[0].detail.contains("resolved atlas key"));
    }
}

thread_local! {
    static TIME: Cell<f32> = const { Cell::new(0.0) };
}

fn dt(seconds: f32) -> Tick {
    let time = TIME.with(|time| {
        time.set(time.get() + seconds);
        time.get()
    });
    Tick { time, dt: seconds }
}

struct One(SpriteSheet);

impl Sheets for One {
    fn sheet(&self, _: &[u8]) -> Option<&SpriteSheet> {
        Some(&self.0)
    }
    fn progress(&self, _: &[u8]) -> Option<[&SpriteSheet; 2]> {
        Some([&self.0, &self.0])
    }
    fn masking(&self, _: &[u8]) -> Option<&SpriteSheet> {
        Some(&self.0)
    }
    fn mask(&self, _: &[u8]) -> Option<&SpriteSheet> {
        Some(&self.0)
    }
}

fn one() -> One {
    One(even_sheet())
}

fn even_sheet() -> SpriteSheet {
    SpriteSheet {
        name: "GFX_even".into(),
        texture: 3,
        frames: 1,
        size: [20, 10],
    }
}

fn gui(body: &str) -> native::Gui {
    read(format!("guiTypes = {{ containerWindowType = {{ name = \"w\" {body} }} }}\n").as_bytes())
}

fn reasons(refusals: &[Refusal]) -> String {
    refusals
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn request(bindings: &mut Bindings, element: &Element, change: impl FnOnce(&mut lifecycle::Shown)) {
    let bound = bindings.bind(element);
    let mut shown = bound
        .shown
        .unwrap_or_else(|| lifecycle::Shown::shown_by_code(element));
    change(&mut shown);
    bound.shown = Some(shown);
}

fn laid(body: &str, parent: Option<[i32; 2]>) -> Result<Option<Layout>, Refusals> {
    laid_bound(body, parent, |_, _| {})
}

fn laid_bound(
    body: &str,
    parent: Option<[i32; 2]>,
    bind: impl Fn(&Element, &mut Bindings),
) -> Result<Option<Layout>, Refusals> {
    let gui = gui(body);
    let window = gui.template("w").unwrap();
    let mut bindings = Bindings {
        regions: true,
        ..Bindings::default()
    };
    bind(window, &mut bindings);
    layout(window, parent, &fixture(), &bindings)
}

fn parts(body: &str) -> Vec<PartKind> {
    laid(body, None)
        .unwrap()
        .unwrap()
        .parts
        .into_iter()
        .map(|part| part.kind)
        .collect()
}

fn rect(part: &PartKind) -> Rect {
    match part {
        PartKind::Sprite { geometry, .. }
        | PartKind::Cell { geometry, .. }
        | PartKind::Progress { geometry, .. }
        | PartKind::TextSprite { geometry, .. } => geometry.rect,
        PartKind::Animated(animated) => animated.geometry.rect,
        PartKind::Text { glyphs, .. } => [glyphs[0].at[0], glyphs[0].at[1], 0.0, 0.0],
        PartKind::Inline { rect, .. } | PartKind::MaskedFlag { rect, .. } => *rect,
        PartKind::Tiles { quads, .. } => quads[0].0,
        PartKind::Fan { centre, radius, .. } => [centre[0], centre[1], radius[0], radius[1]],
    }
}

fn frame_of(part: &PartKind) -> i32 {
    match part {
        PartKind::Sprite { uv, offsets, .. } => {
            let step = if uv[2] > 0.0 && offsets[1] == 0.0 {
                offsets[0] / uv[2]
            } else {
                offsets[1] / uv[3]
            };
            step.round() as i32
        }
        _ => panic!("not a sprite"),
    }
}

fn size(width: &str, height: &str, extra: &str) -> native::Size {
    read(
        format!(
            "guiTypes = {{ containerWindowType = {{ name = \"w\" size = {{ width = {width} \
             height = {height} {extra} }} }} }}\n"
        )
        .as_bytes(),
    )
    .template("w")
    .unwrap()
    .container()
    .unwrap()
    .size
}

#[test]
fn sizes_evaluate_against_the_parent() {
    content_reader_script::test_game::enter();
    let evaluate = |size: &native::Size, parent| evaluate_size(size, parent, [10, 4], "s");
    assert_eq!(
        evaluate(&size("50%", "25%", ""), Some([101, 40])),
        Ok([51, 10])
    );
    assert_eq!(
        evaluate(&size("-5", "100%%", ""), Some([100, 40])),
        Ok([85, 36])
    );
    assert_eq!(
        evaluate(
            &size("33", "43", "min = { width = 80 } max = { height = 20 }"),
            None
        ),
        Ok([80, 20])
    );
    assert_eq!(
        evaluate(
            &size(
                "100",
                "50",
                "max = { width = 60 } preserve_aspect_ratio = yes"
            ),
            None
        ),
        Ok([60, 30])
    );
    assert!(evaluate(&size("50%", "4", ""), None).is_err());
    assert!(evaluate(&size("-5", "4", ""), None).is_err());
}

#[test]
fn a_root_is_placed_as_the_code_places_it() {
    content_reader_script::test_game::enter();
    assert!(laid("origo = center size = { width = 4 height = 4 }", None).is_ok());
    let refused = laid("size = { width = 50% height = 4 }", None).unwrap_err();
    assert_eq!(refused[0].kind, RefusalKind::NoParent);
    assert!(laid("size = { width = 50% height = 4 }", Some([10, 10])).is_ok());
    let gui = gui(
        "size = { width = 21 height = 11 } position = { x = 3 y = 4 } \
         orientation = center origo = center",
    );
    let window = gui.template("w").unwrap();
    let viewport = [101, 51];
    assert_eq!(
        root_origin(window, viewport, RootPlacement::Constructed),
        Ok([43, 24])
    );
    assert_eq!(
        root_origin(window, viewport, RootPlacement::Set([7, 8])),
        Ok([7, 8])
    );
    assert_eq!(
        root_origin(window, viewport, RootPlacement::Anchored([7, 8])),
        Ok([47, 28])
    );
}

#[test]
fn frames_follow_the_sprite_draw() {
    content_reader_script::test_game::enter();
    let authored = parts(
        "iconType = { name = \"a\" spriteType = \"GFX_odd\" } \
         iconType = { name = \"b\" spriteType = \"GFX_odd\" frame = 3 } \
         iconType = { name = \"c\" spriteType = \"GFX_odd\" frame = 0 } \
         buttonType = { name = \"d\" spriteType = \"GFX_odd\" } \
         buttonType = { name = \"e\" spriteType = \"GFX_odd\" frame = 2 } \
         iconType = { name = \"f\" spriteType = \"GFX_odd\" frame = 5 }",
    );
    assert_eq!(
        authored.iter().map(frame_of).collect::<Vec<_>>(),
        [0, 2, 0, 0, 1, 4]
    );
    let layout = laid(
        "iconType = { name = \"a\" spriteType = \"GFX_odd\" frame = 5 }",
        None,
    )
    .unwrap()
    .unwrap();
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    let exact = batch.quads[0].exact.as_ref().unwrap();
    let du = (45.0f32 * (1.0 / 45.0)) * (1.0 / 3.0);
    assert_eq!(exact.uv[2], [du, 1.0]);
    assert_eq!(exact.offsets[0], 4.0 * du);
}

#[test]
fn unreproduced_kinds_and_custom_effects_are_refused_by_name() {
    content_reader_script::test_game::enter();
    let refused = laid(
        "iconType = { name = \"a\" spriteType = \"GFX_tile\" } \
         iconType = { name = \"b\" spriteType = \"GFX_custom\" } \
         iconType = { name = \"c\" spriteType = \"GFX_custom\" }",
        None,
    )
    .unwrap_err();
    assert_eq!(refused.len(), 2, "{}", reasons(&refused));
    assert_eq!(refused[0].kind, RefusalKind::CustomEffect);
    let mut counts = Counts::default();
    counts.refused(&refused);
    assert_eq!(counts.0["custom_effect"], 2);
    assert!(parts("iconType = { name = \"a\" spriteType = \"GFX_tile\" }").is_empty());
}

#[test]
fn a_cornered_tile_button_uses_its_declared_size() {
    content_reader_script::test_game::enter();
    let sized = laid(
        "buttonType = { name = \"preview\" quadTextureSprite = \"GFX_cornered\" \
         position = { x = 5 y = 7 } size = { x = 111 y = 58 } }",
        None,
    )
    .unwrap()
    .unwrap();
    let PartKind::Tiles { ref quads, .. } = sized.parts[0].kind else {
        panic!("not a tile");
    };
    let right = quads
        .iter()
        .map(|(rect, _)| rect[0] + rect[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quads
        .iter()
        .map(|(rect, _)| rect[1] + rect[3])
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(right, 5.0 + 111.0);
    assert_eq!(bottom, 7.0 + 58.0);
    let default_sized = laid(
        "buttonType = { name = \"plain\" quadTextureSprite = \"GFX_cornered\" \
         position = { x = 5 y = 7 } }",
        None,
    )
    .unwrap()
    .unwrap();
    let PartKind::Tiles { ref quads, .. } = default_sized.parts[0].kind else {
        panic!("not a tile");
    };
    let right = quads
        .iter()
        .map(|(rect, _)| rect[0] + rect[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quads
        .iter()
        .map(|(rect, _)| rect[1] + rect[3])
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(right, 5.0 + 30.0);
    assert_eq!(bottom, 7.0 + 20.0);

    let centered = laid(
        "buttonType = { name = \"centered\" quadTextureSprite = \"GFX_cornered\" \
         position = { x = 100 y = 80 } size = { x = 110 y = 58 } centerposition = yes }",
        None,
    )
    .unwrap()
    .unwrap();
    let PartKind::Tiles { ref quads, .. } = centered.parts[0].kind else {
        panic!("not a tile");
    };
    let left = quads
        .iter()
        .map(|(rect, _)| rect[0])
        .fold(f32::INFINITY, f32::min);
    let top = quads
        .iter()
        .map(|(rect, _)| rect[1])
        .fold(f32::INFINITY, f32::min);
    let right = quads
        .iter()
        .map(|(rect, _)| rect[0] + rect[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quads
        .iter()
        .map(|(rect, _)| rect[1] + rect[3])
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!([left, top, right, bottom], [45.0, 51.0, 155.0, 109.0]);

    let standard_gui = native::read(
        br#"guiTypes = { windowType = { name = "w" size = { x = 400 y = 300 }
            guiButtonType = { name = "std_centered" quadTextureSprite = "GFX_cornered"
                position = { x = 100 y = 80 } size = { x = 110 y = 58 } centerposition = yes scale = 2.0 } } }"#,
    );
    let standard_window = standard_gui.template("w").unwrap();
    let standard_centered = layout(
        standard_window,
        None,
        &fixture(),
        &Bindings {
            regions: true,
            ..Bindings::default()
        },
    )
    .unwrap()
    .unwrap();
    let PartKind::Tiles { ref quads, .. } = standard_centered.parts[0].kind else {
        panic!("not a tile");
    };
    let left = quads
        .iter()
        .map(|(rect, _)| rect[0])
        .fold(f32::INFINITY, f32::min);
    let top = quads
        .iter()
        .map(|(rect, _)| rect[1])
        .fold(f32::INFINITY, f32::min);
    let right = quads
        .iter()
        .map(|(rect, _)| rect[0] + rect[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quads
        .iter()
        .map(|(rect, _)| rect[1] + rect[3])
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!([left, top, right, bottom], [45.0, 51.0, 155.0, 109.0]);

    let checkbox = laid(
        "checkboxType = { name = \"cb\" quadTextureSprite = \"GFX_cornered\" \
         position = { x = 5 y = 7 } size = { x = 110 y = 58 } }",
        None,
    )
    .unwrap()
    .unwrap();
    let PartKind::Tiles { ref quads, .. } = checkbox.parts[0].kind else {
        panic!("not a tile");
    };
    let right = quads
        .iter()
        .map(|(rect, _)| rect[0] + rect[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quads
        .iter()
        .map(|(rect, _)| rect[1] + rect[3])
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!([right, bottom], [5.0 + 30.0, 7.0 + 20.0]);
    let cb_region = checkbox
        .regions
        .iter()
        .find(|region| region.path.ends_with(".cb"))
        .unwrap();
    assert_eq!(cb_region.rect, [5.0, 7.0, 30.0, 20.0]);
}

#[test]
fn a_frame_animated_sprite_follows_its_clock() {
    content_reader_script::test_game::enter();
    let body = "iconType = { name = \"a\" spriteType = \"GFX_animated\" }";
    let error = laid(body, None).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::MissingInput);
    let tree = gui(body);
    let window = tree.template("w").unwrap();
    let resources = fixture();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(frame_of(&laid.parts[0].kind), 0);
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    let layout = instance.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(frame_of(&layout.parts[0].kind), 1);
    assert!((layout.parts[0].shading.animation - 0.2).abs() < 1e-5);
    instance.tick(window, &resources, dt(1.6)).unwrap();
    let layout = instance.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(frame_of(&layout.parts[0].kind), 0);
}

#[test]
fn a_shield_overlay_draws_with_zero_clocks() {
    content_reader_script::test_game::enter();
    use shader_pipeline::clock::Clock;
    let reads_time = || {
        clocked_effect(
            "Up",
            Clock {
                direct: vec![(1, 96, 104)],
                bounded: Vec::new(),
            },
        )
    };
    let mut resources = fixture();
    resources.buttonstate = Some(reads_time());
    resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(reads_time());
    let tree = gui("iconType = { name = \"s\" spriteType = \"GFX_shield\" } \
         iconType = { name = \"i\" spriteType = \"GFX_even\" position = { x = 40 y = 0 } }");
    let window = tree.template("w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let first = dt(0.0);
    instance.tick(window, &resources, first).unwrap();
    let now = dt(0.75);
    instance.tick(window, &resources, now).unwrap();
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    let clocks = |part: &Part| {
        let mut batch = Batch::default();
        crate::native_shader::draw::emit(part, [0, 0], &one(), &mut batch).unwrap();
        let native = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .clone()
            .unwrap();
        [24, 25].map(|index| native.constants[index])
    };
    let overlay = laid
        .parts
        .iter()
        .find(|part| part.path == "w.s" && matches!(part.kind, PartKind::Tiles { .. }))
        .unwrap();
    assert!(!overlay.shading.clocked);
    assert_eq!(clocks(overlay), [0.0, 0.0]);
    let icon = laid.parts.iter().find(|part| part.path == "w.i").unwrap();
    assert_eq!(clocks(icon), [now.time, first.time]);
}

#[test]
fn an_unknown_sprite_draws_the_fallback() {
    content_reader_script::test_game::enter();
    let parts =
        parts("iconType = { name = \"a\" spriteType = \"GFX_mod\" } iconType = { name = \"b\" }");
    assert!(
        parts.iter().all(
            |part| matches!(part, PartKind::Sprite { sprite, .. } if sprite == FALLBACK_SPRITE)
        )
    );
    let gui = gui("iconType = { name = \"a\" spriteType = \"GFX_mod\" } \
                   iconType = { name = \"b\" spriteType = \"GFX_mod\" }");
    let mut resources = fixture();
    resources.sprites.remove("GFX_default_fallback_texture");
    let layout = layout(
        gui.template("w").unwrap(),
        None,
        &resources,
        &Bindings::default(),
    )
    .unwrap()
    .unwrap();
    assert!(layout.parts.is_empty());
    let mut counts = Counts::default();
    counts.noted(&layout.diagnostics);
    assert_eq!(counts.0["missing_fallback_sprite"], 2);
}

#[test]
fn bound_state_overrides_the_authored() {
    content_reader_script::test_game::enter();
    let layout = laid_bound(
        "iconType = { name = \"a\" spriteType = \"GFX_odd\" } \
         iconType = { name = \"b\" spriteType = \"GFX_odd\" hide = yes } \
         iconType = { name = \"c\" spriteType = \"GFX_odd\" } \
         iconType = { name = \"d\" spriteType = \"GFX_bar\" }",
        None,
        |window, bindings| {
            let a = bindings.bind(window.find(Kind::Icon, "a").unwrap());
            a.sprite = Some("GFX_even".into());
            a.color = Some([0.5; 4]);
            request(bindings, window.find(Kind::Icon, "b").unwrap(), |shown| {
                shown.show();
            });
            request(bindings, window.find(Kind::Icon, "c").unwrap(), |shown| {
                shown.hide()
            });
            bindings
                .bind(window.find(Kind::Icon, "d").unwrap())
                .progress = Some(40);
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(layout.parts.len(), 2);
    assert!(
        matches!(&layout.parts[0].kind, PartKind::Sprite { sprite, .. } if sprite == b"GFX_even")
    );
    assert_eq!(layout.parts[0].color, [0.5; 4]);
    assert!(matches!(layout.parts[1].kind, PartKind::Progress { state, .. } if state == 0.4));
}

#[test]
fn sprite_setters_rebuild_as_the_game_does() {
    content_reader_script::test_game::enter();
    let body = "size = { width = 100 height = 100 } \
                buttonType = { name = \"b\" spriteType = \"GFX_odd\" orientation = center \
                centerPosition = yes scale = 2 rotation = 1.0 position = { x = 5 y = 5 } } \
                iconType = { name = \"i\" spriteType = \"GFX_odd\" rotation = 1.0 frame = 3 }";
    let with = |setter: SpriteSetter, name: &str| {
        laid_bound(body, None, |window, bindings| {
            for (kind, element) in [(Kind::Button, "b"), (Kind::Icon, "i")] {
                let bound = bindings.bind(window.find(kind, element).unwrap());
                bound.sprite = Some(name.into());
                bound.setter = setter;
            }
        })
        .unwrap()
        .unwrap()
        .parts
    };
    let geometry = |part: &Part| match part.kind {
        PartKind::Sprite { geometry, .. } => geometry,
        _ => panic!("not a sprite"),
    };
    let kept = with(SpriteSetter::ButtonStandard, "GFX_undefined");
    assert!(matches!(&kept[0].kind, PartKind::Sprite { sprite, .. } if sprite == b"GFX_odd"));
    assert_eq!(geometry(&kept[0]).rotation, 1.0);
    let standard = with(SpriteSetter::ButtonStandard, "GFX_even");
    let rebuilt = geometry(&standard[0]);
    assert_eq!(rebuilt.rotation, 0.0);
    assert_eq!(rebuilt.rect, [-5.0, 0.0, 40.0, 20.0]);
    assert_eq!(frame_of(&standard[0].kind), 0);
    let icon = with(SpriteSetter::Icon, "GFX_four");
    assert_eq!(geometry(&icon[1]).rotation, 0.0);
    assert_eq!(frame_of(&icon[1].kind), 0);
    let button = with(SpriteSetter::Button, "GFX_even");
    assert_eq!(geometry(&button[0]).rotation, 1.0);
    assert_eq!(geometry(&button[0]).rect[0], 35.0);
    assert_eq!(frame_of(&button[1].kind), 2);
}

#[test]
fn show_requests_and_the_hide_bit_are_separate() {
    content_reader_script::test_game::enter();
    let gui = gui("iconType = { name = \"h\" hide = yes } iconType = { name = \"s\" }");
    let window = gui.template("w").unwrap();
    let hidden = window.find(Kind::Icon, "h").unwrap();
    let shown = window.find(Kind::Icon, "s").unwrap();
    #[derive(Clone, Copy)]
    enum Request {
        Show,
        Hide,
        HideFlag(bool),
    }
    use Request::*;
    let after = |element: &Element, requests: &[Request]| {
        let mut shown = lifecycle::Shown::shown_by_code(element);
        for request in requests {
            match *request {
                Show => {
                    shown.show();
                }
                Hide => shown.hide(),
                HideFlag(flag) => shown.hide = flag,
            }
        }
        let bound = Bound {
            shown: Some(shown),
            ..Bound::default()
        };
        visible(element, Some(&bound))
    };
    assert!(!after(hidden, &[]));
    assert!(!after(hidden, &[Show]));
    assert!(after(hidden, &[HideFlag(false), Show]));
    assert!(after(shown, &[]));
    assert!(!after(shown, &[Hide]));
    assert!(!after(shown, &[Hide, HideFlag(true), Show]));
    assert!(after(shown, &[Hide, Show, HideFlag(true)]));
}

#[test]
fn a_button_draws_its_state() {
    content_reader_script::test_game::enter();
    let layout = laid(
        "buttonType = { name = \"a\" spriteType = \"GFX_four\" enabled = no } \
         buttonType = { name = \"b\" spriteType = \"GFX_odd\" enabled = no } \
         buttonType = { name = \"c\" spriteType = \"GFX_four\" enabled = no frame = 2 }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        layout
            .parts
            .iter()
            .map(|part| frame_of(&part.kind))
            .collect::<Vec<_>>(),
        [3, 0, 1]
    );
    let disable = shading(&synthetic::buttonstate(), "Disable", 0.0);
    assert!(layout.parts.iter().all(|part| part.shading == disable));
    let error = laid("buttonType = { name = \"b\" }", None).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::UndefinedRead);
    assert!(reasons(&error).contains("uninitialised"));
    assert_eq!(
        parts("buttonType = { name = \"b\" spriteType = \"GFX_odd\" text = \"KEY\" }").len(),
        1
    );
    let fallback = parts("buttonType = { name = \"b\" buttonFont = \"Main_14\" }");
    assert!(matches!(&fallback[0], PartKind::Sprite { sprite, .. } if sprite == FALLBACK_SPRITE));
    assert_eq!(fallback.len(), 1);
}

#[test]
fn a_missing_technique_keeps_the_previous_program() {
    content_reader_script::test_game::enter();
    let bind = |window: &Element, bindings: &mut Bindings| {
        bindings.pointer = Some(Pointer {
            at: [5.0, 5.0],
            pressed: Some("w.b".into()),
            over: None,
            clip_at: None,
            drawn: None,
            drawn_boxes: Vec::new(),
        });
        let _ = window;
    };
    let alone = laid_bound(
        "buttonType = { name = \"b\" spriteType = \"GFX_no_down\" }",
        None,
        bind,
    )
    .unwrap()
    .unwrap();
    assert_eq!(alone.parts[0].shading.fixture, None);
    let error = draw(&alone, [0, 0], &one(), &mut Batch::default()).unwrap_err();
    assert_eq!(error.kind, RefusalKind::MissingInput);
    let after = laid_bound(
        "iconType = { name = \"i\" spriteType = \"GFX_even\" } \
         buttonType = { name = \"b\" spriteType = \"GFX_no_down\" }",
        None,
        bind,
    )
    .unwrap()
    .unwrap();
    let mut batch = Batch::default();
    draw(&after, [0, 0], &one(), &mut batch).unwrap();
    let [first, second] = [
        batch.quads[0].exact.as_ref().unwrap(),
        batch.quads[1].exact.as_ref().unwrap(),
    ];
    assert_eq!(second.technique, first.technique);
    assert_eq!(second.samplers, first.samplers);
    assert_eq!(second.blend, Blend::Off);
}

#[test]
fn a_button_takes_its_sprite_from_its_parent() {
    content_reader_script::test_game::enter();
    let sprite = |body: &str| {
        parts(body)
            .into_iter()
            .filter_map(|part| match part {
                PartKind::Sprite { sprite, .. } => Some(sprite),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        sprite(
            "buttonType = { name = \"b\" parent = \"MID\" } \
             buttonType = { name = \"mid\" parent = \"top\" } \
             buttonType = { name = \"top\" spriteType = \"GFX_odd\" }"
        ),
        [
            b"GFX_odd".to_vec(),
            b"GFX_odd".to_vec(),
            b"GFX_odd".to_vec()
        ]
    );
    for (body, what) in [
        (
            "buttonType = { name = \"b\" parent = \"nobody\" }",
            "crashes",
        ),
        (
            "buttonType = { name = \"b\" parent = \"c\" } \
             buttonType = { name = \"c\" parent = \"b\" }",
            "hangs",
        ),
    ] {
        let error = laid(body, None).unwrap_err();
        assert_eq!(
            error[0].kind,
            RefusalKind::Reader,
            "{body}: {}",
            reasons(&error)
        );
        assert!(
            reasons(&error).contains(what),
            "{body}: {}",
            reasons(&error)
        );
    }
}

#[test]
fn a_check_box_draws_its_state() {
    content_reader_script::test_game::enter();
    let body = "size = { width = 50 height = 50 } \
         checkboxType = { name = \"u\" spriteType = \"GFX_four\" position = { x = 1 y = 2 } \
         orientation = center centerPosition = yes scale = 2 } \
         checkboxType = { name = \"c\" spriteType = \"GFX_four\" }";
    let unclocked = laid_bound(body, None, |window, bindings| {
        bindings
            .bind(window.find(Kind::Checkbox, "c").unwrap())
            .checked = Some(true);
    })
    .unwrap_err();
    assert_eq!(unclocked[0].kind, RefusalKind::MissingInput);
    let layout = laid_bound(body, None, |window, bindings| {
        let bound = bindings.bind(window.find(Kind::Checkbox, "c").unwrap());
        bound.checked = Some(true);
        bound.effect_since = Some(0.03);
    })
    .unwrap()
    .unwrap();
    assert_eq!(rect(&layout.parts[0].kind), [26.0, 27.0, 10.0, 10.0]);
    assert_eq!(frame_of(&layout.parts[0].kind), 0);
    assert_eq!(frame_of(&layout.parts[1].kind), 1);
    assert_eq!(
        layout.parts[1].shading,
        shading(&synthetic::buttonstate(), "Down", 0.03)
    );
    let faded = laid_bound(
        "fade_type = linear checkboxType = { name = \"c\" spriteType = \"GFX_four\" }",
        None,
        |window, bindings| bindings.bind(window).opacity = Some(0.5),
    )
    .unwrap()
    .unwrap();
    assert!(faded.parts.is_empty());
}

#[test]
fn the_pointer_hovers_and_presses_buttons() {
    content_reader_script::test_game::enter();
    let body = "size = { width = 50 height = 50 } \
                buttonType = { name = \"low\" spriteType = \"GFX_four\" } \
                buttonType = { name = \"high\" spriteType = \"GFX_four\" } \
                buttonType = { name = \"clear\" spriteType = \"GFX_through\" } \
                buttonType = { name = \"away\" spriteType = \"GFX_four\" position = { x = 30 y = 30 } }";
    let states = |pointer: Pointer, ticked: Option<(&str, u32)>| {
        let gui = gui(body);
        let window = gui.template("w").unwrap();
        let mut bindings = Bindings {
            pointer: Some(pointer),
            ..Bindings::default()
        };
        if let Some((name, state)) = ticked {
            let bound = bindings.bind(window.path_named(name).unwrap());
            bound.effect_state = Some(state);
            bound.effect_since = Some(1.0);
        }
        layout(window, None, &fixture(), &bindings)
            .unwrap()
            .unwrap()
            .parts
            .into_iter()
            .map(|part| (frame_of(&part.kind), part.shading))
            .collect::<Vec<_>>()
    };
    let effect = synthetic::buttonstate();
    let pointer = Pointer {
        at: [5.0, 5.0],
        pressed: None,
        over: None,
        clip_at: None,
        drawn: None,
        drawn_boxes: Vec::new(),
    };
    let hovered = states(pointer.clone(), None);
    assert_eq!(hovered[0], (0, shading(&effect, "Up", 0.0)));
    assert_eq!(hovered[1], (2, shading(&effect, "Over", 0.0)));
    assert_eq!(hovered[2], (0, shading(&effect, "Up", 0.0)));
    let ticked = states(pointer.clone(), Some(("high", 3)));
    assert_eq!(ticked[1], (2, shading(&effect, "Over", 1.0)));
    let changed = states(pointer.clone(), Some(("high", 2)));
    assert_eq!(changed[1], (2, shading(&effect, "Over", 0.0)));
    let pressed = states(
        Pointer {
            pressed: Some("w.away".into()),
            ..pointer.clone()
        },
        Some(("away", 2)),
    );
    assert_eq!(pressed[3], (1, shading(&effect, "Down", 1.0)));
    assert_eq!(pressed[1], (0, shading(&effect, "Up", 0.0)));
    let named = states(
        Pointer {
            at: [f32::NAN; 2],
            over: Some("w.away".into()),
            ..pointer
        },
        None,
    );
    assert_eq!(named[1], (0, shading(&effect, "Up", 0.0)));
    assert_eq!(named[3], (2, shading(&effect, "Over", 0.0)));
    let gui = gui("buttonType = { name = \"a\" spriteType = \"GFX_alpha\" }");
    let bindings = Bindings {
        pointer: Some(Pointer {
            at: [1.0, 1.0],
            ..Pointer::default()
        }),
        ..Bindings::default()
    };
    assert!(layout(gui.template("w").unwrap(), None, &fixture(), &bindings).is_err());
}

#[test]
fn a_progress_bar_runs_its_own_pipeline() {
    content_reader_script::test_game::enter();
    let parts =
        parts("iconType = { name = \"p\" spriteType = \"GFX_bar\" rotation = 1.5708 scale = 3 }");
    let PartKind::Progress {
        geometry,
        uv,
        state,
        colors,
        textured,
        ..
    } = &parts[0]
    else {
        panic!("not a progress bar");
    };
    assert_eq!(geometry.rect, [0.0, 1.0, 154.0, 14.0]);
    assert_eq!(geometry.pivot, [0.0, 14.0]);
    assert_eq!(uv[3], [0.0, 0.0]);
    assert_eq!(uv[1], [1.0, 1.0]);
    assert_eq!(*state, 1.0f32 / 100.0);
    assert_eq!(colors[1], [0.0, 0.0, 1.0, 0.5]);
    assert!(!textured);
    let bound = |frame: i32, sprite: &str| {
        let layout = laid_bound(
            &format!("buttonType = {{ name = \"p\" spriteType = \"{sprite}\" }}"),
            None,
            |window, bindings| {
                let bound = bindings.bind(window.find(Kind::Button, "p").unwrap());
                bound.frame = Some(frame);
                bound.color = Some([0.5; 4]);
                bound.stretch = Some(0.5);
            },
        )
        .unwrap()
        .unwrap();
        layout.parts.into_iter().next().unwrap()
    };
    let negative = bound(-30, "GFX_bar");
    assert!(matches!(negative.kind, PartKind::Progress { state, .. } if state == -0.3));
    assert_eq!(
        negative.shading,
        shading(&synthetic::progress(), "Color", 0.0)
    );
    let PartKind::Progress { colors, .. } = negative.kind else {
        panic!("not a progress bar");
    };
    assert_eq!(colors[0], [0.5, 0.0, 0.0, 0.5]);
    let upright = bound(7, "GFX_upright_bar");
    let PartKind::Progress { state, uv, .. } = upright.kind else {
        panic!("not a progress bar");
    };
    assert!(state.is_nan());
    assert_eq!(uv, [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]);
    assert_eq!(
        upright.shading,
        shading(&synthetic::progress(), "Texture", 0.0)
    );
    assert!(matches!(bound(400, "GFX_bar").kind, PartKind::Progress { state, .. } if state == 1.0));
    for kind in ["iconType", "buttonType"] {
        let layout = laid_bound(
            &format!(
                "{kind} = {{ name = \"p\" spriteType = \"GFX_bar\" rotation = 0.5 \
                 position = {{ x = 10 y = 40 }} }}"
            ),
            None,
            |window, bindings| {
                let element = window.child_named("p").unwrap();
                let bound = bindings.bind(element);
                bound.rotation = Some(std::f32::consts::PI);
                bound.offset = Some([154, -14]);
            },
        )
        .unwrap()
        .unwrap();
        let PartKind::Progress { geometry, .. } = &layout.parts[0].kind else {
            panic!("not a progress bar");
        };
        assert_eq!(geometry.rotation, std::f32::consts::PI);
        assert_eq!(geometry.rect, [164.0, 27.0, 154.0, 14.0]);
        assert_eq!(geometry.pivot, [164.0, 40.0]);
    }
    let layout = laid(
        "iconType = { name = \"p\" spriteType = \"GFX_upright_bar\" }",
        None,
    )
    .unwrap()
    .unwrap();
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    let exact = batch.quads[0].exact.as_ref().unwrap();
    assert_eq!(batch.quads[0].texture, 3);
    assert_eq!(exact.second, Some(3));
}

fn text_glyphs(body: &str, text: &str) -> Result<Vec<DrawnGlyph>, Refusals> {
    let layout = laid_bound(
        &format!("size = {{ width = 100 height = 50 }} instantTextBoxType = {{ name = \"t\" font = \"Main_14\" {body} }}"),
        None,
        |window, bindings| {
            bindings.bind(window.find(Kind::TextBox, "t").unwrap()).text = Some(text.into());
        },
    )?
    .unwrap();
    Ok(match &layout.parts[0].kind {
        PartKind::Text { glyphs, .. } => glyphs.clone(),
        _ => panic!("not a text"),
    })
}

fn text_at(body: &str, text: &str) -> Result<[f32; 2], Refusals> {
    Ok(text_glyphs(body, text)?[0].at)
}

#[test]
fn text_aligns_as_the_bitmap_font_draws_it() {
    content_reader_script::test_game::enter();
    let box_ =
        "position = { x = 10 y = 20 } maxWidth = 40 maxHeight = 30 borderSize = { x = 2 y = 3 }";
    assert_eq!(text_at(box_, "ab"), Ok([12.0, 23.0]));
    assert_eq!(
        text_at(&format!("{box_} format = centre"), "ab"),
        Ok([25.0, 23.0])
    );
    assert_eq!(
        text_at(&format!("{box_} format = right"), "ab"),
        Ok([36.0, 23.0])
    );
    assert_eq!(
        text_at(&format!("{box_} vertical_alignment = center"), "ab"),
        Ok([12.0, 29.0])
    );
    assert_eq!(
        text_at(&format!("{box_} vertical_alignment = bottom"), "ab"),
        Ok([12.0, 36.0])
    );
    assert_eq!(
        text_at("orientation = center format = center", "ab"),
        Ok([143.0, 25.0])
    );
    assert_eq!(
        text_at(
            &format!("{box_} orientation = center_up format = right"),
            "ab"
        ),
        Ok([10.0 + 50.0 + 2.0 + 40.0 - 14.0, 23.0])
    );
}

#[test]
fn text_wraps_and_cuts_as_the_game_does() {
    content_reader_script::test_game::enter();
    let box_ = "position = { x = 10 y = 20 } maxWidth = 20 borderSize = { x = 2 y = 0 }";
    let wrapped = text_glyphs(&format!("{box_} multiline = yes"), "abc").unwrap();
    assert_eq!(
        wrapped
            .iter()
            .map(|glyph| (glyph.ch, glyph.at))
            .collect::<Vec<_>>(),
        [
            ('a', [12.0, 20.0]),
            ('b', [19.0, 20.0]),
            ('c', [12.0, 34.0])
        ]
    );
    let cut = text_glyphs(
        "maxWidth = 30 borderSize = { x = 2 y = 0 } multiline = no",
        "abcd",
    )
    .unwrap();
    assert_eq!(cut.iter().map(|glyph| glyph.ch).collect::<String>(), "...");
    let short = text_glyphs(&format!("{box_} multiline = no"), "abcd").unwrap();
    assert_eq!(short.iter().map(|glyph| glyph.ch).collect::<String>(), "ab");
}

#[test]
fn text_colours_follow_the_codes() {
    content_reader_script::test_game::enter();
    let own = native_text::packed([128.0 / 255.0, 64.0 / 255.0, 32.0 / 255.0, 1.0]);
    let glyphs = text_glyphs("", "ab").unwrap();
    assert_eq!(glyphs[0].color, own);
    let coded = text_glyphs("text_color_code = b", "ab").unwrap();
    assert_eq!(coded[0].color, [0.0, 0.0, 1.0, 1.0]);
    let layout = laid(
        "size = { width = 100 height = 50 } \
         instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"MOD_COLOURED\" }",
        None,
    )
    .unwrap()
    .unwrap();
    let PartKind::Inline {
        source: InlineSource::Sprite(ref sprite),
        rect,
        uv,
        ..
    } = layout.parts[0].kind
    else {
        panic!("the icon first");
    };
    assert_eq!(
        (sprite.as_slice(), rect),
        (b"GFX_odd".as_slice(), [28.0, 3.0, 15.0, 9.0])
    );
    assert_eq!(uv, [0.0, 0.0, 1.0 / 3.0, 1.0]);
    assert_eq!(
        layout.parts[0].shading,
        shading(&synthetic::text(), "Text", 0.0)
    );
    let PartKind::Text { ref glyphs, .. } = layout.parts[1].kind else {
        panic!("not a text");
    };
    let colours: Vec<_> = glyphs.iter().map(|glyph| (glyph.ch, glyph.color)).collect();
    assert_eq!(colours[0], ('a', own));
    assert_eq!(colours[1], ('b', [1.0, 0.0, 0.0, 1.0]));
    assert_eq!(colours[2], ('c', own));
    assert_eq!(glyphs[3].at[0], 28.0 + 15.0);
}

#[test]
fn text_flags_and_icons_resolve_through_the_caller() {
    content_reader_script::test_game::enter();
    let flag = native_text::FLAG;
    let icon = native_text::ICON;
    let layout = laid_bound(
        "instantTextBoxType = { name = \"t\" font = \"Main_14\" }",
        None,
        |window, bindings| {
            bindings.bind(window.find(Kind::TextBox, "t").unwrap()).text =
                Some(format!("{flag}GER{flag}NULa{icon}nothing b"));
        },
    )
    .unwrap()
    .unwrap();
    let PartKind::Inline {
        source: InlineSource::Texture(9),
        rect,
        uv,
        ..
    } = layout.parts[0].kind
    else {
        panic!("the flag first: {:?}", layout.parts[0].kind);
    };
    assert_eq!(rect, [0.0, 5.0, 10.0, 7.0]);
    assert_eq!(uv, [0.25, 0.5, 0.5, 0.75]);
    let PartKind::Text { ref glyphs, .. } = layout.parts[1].kind else {
        panic!("not a text");
    };
    assert_eq!(glyphs[0].at[0], 24.0);
    assert_eq!(glyphs[1].at[0], 31.0);
    assert_eq!(
        layout.diagnostics[0].kind,
        DiagnosticKind::UndefinedTextIcon
    );
    let error = laid_bound(
        "instantTextBoxType = { name = \"t\" font = \"Main_14\" }",
        None,
        |window, bindings| {
            bindings.bind(window.find(Kind::TextBox, "t").unwrap()).text =
                Some(format!("{flag}XYZ"));
        },
    )
    .unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::MissingInput);
}

#[test]
fn long_text_flushes_its_glyphs_before_later_icons() {
    content_reader_script::test_game::enter();
    let icon = native_text::ICON;
    let kinds = |count: usize| {
        laid_bound(
            "instantTextBoxType = { name = \"t\" font = \"Main_14\" maxWidth = 100000 \
             multiline = yes }",
            None,
            |window, bindings| {
                bindings.bind(window.find(Kind::TextBox, "t").unwrap()).text =
                    Some(format!("{}{icon}odd b", "a".repeat(count)));
            },
        )
        .unwrap()
        .unwrap()
        .parts
        .into_iter()
        .map(|part| match part.kind {
            PartKind::Text { glyphs, .. } => format!("text {}", glyphs.len()),
            PartKind::Inline { .. } => "icon".to_owned(),
            other => panic!("{other:?}"),
        })
        .collect::<Vec<_>>()
    };
    assert_eq!(kinds(510), ["icon", "text 511"]);
    assert_eq!(kinds(511), ["icon", "text 512"]);
    assert_eq!(kinds(512), ["text 512", "icon", "text 1"]);
}

#[test]
fn a_null_font_page_draws_nothing_and_keeps_the_window() {
    content_reader_script::test_game::enter();
    let gui = gui(
        "instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"MOD_INNER\" } \
         iconType = { name = \"i\" spriteType = \"GFX_even\" }",
    );
    let window = gui.template("w").unwrap();
    let mut resources = fixture();
    resources.font.pages = vec![None];
    let layout = layout(window, None, &resources, &Bindings::default())
        .unwrap()
        .unwrap();
    let PartKind::Text { glyphs, .. } = &layout.parts[0].kind else {
        panic!("{:?}", layout.parts[0].kind);
    };
    assert_eq!(glyphs[0].texture, super::super::sprites::NO_TEXTURE);
    assert!(glyphs[0].uv[0].is_nan() || glyphs[0].uv[0].is_infinite());
    assert!(matches!(layout.parts[1].kind, PartKind::Sprite { .. }));
    assert_eq!(layout.diagnostics[0].kind, DiagnosticKind::NullFontPage);
    let mut counts = Counts::default();
    counts.noted(&layout.diagnostics);
    assert_eq!(counts.0.get("null_font_page"), Some(&1));
}

#[test]
fn reader_refusals_reach_the_layout() {
    content_reader_script::test_game::enter();
    let gui = gui("iconType = { name = \"i\" spriteType = \"GFX_even\" } \
         instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"MOD_INNER\" }");
    let window = gui.template("w").unwrap();
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().refusals =
        vec![(RefusalKind::Reader, "an undefined read".to_owned())];
    resources.font.refusals = vec![(RefusalKind::Reader, "a font refusal".to_owned())];
    let refused = layout(window, None, &resources, &Bindings::default()).unwrap_err();
    let mut counts = Counts::default();
    counts.refused(&refused);
    assert_eq!(counts.0.get("gui_reader"), Some(&2), "{refused:?}");
    assert!(
        refused[0].detail.contains("an undefined read")
            || refused[1].detail.contains("an undefined read")
    );
    assert!(
        refused
            .iter()
            .any(|refusal| refusal.detail.contains("a font refusal"))
    );
    let document = read(
        format!(
            "guiTypes = {{ {SCROLLBAR} containerWindowType = {{ name = \"w\" \
             size = {{ width = 10 height = 10 }} verticalScrollbar = \"sb\" \
             autohide_scrollbars = no }} }}"
        )
        .as_bytes(),
    );
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_four").unwrap().refusals =
        vec![(RefusalKind::Reader, "refused".to_owned())];
    let window = document.template("w").unwrap().clone();
    resources.templates.push(document);
    let refused = layout(&window, None, &resources, &Bindings::default()).unwrap_err();
    assert!(
        refused
            .iter()
            .all(|refusal| refusal.kind == RefusalKind::Reader)
    );
}

#[test]
fn authored_text_shows_its_localisation() {
    content_reader_script::test_game::enter();
    let shown = |key: &str| {
        parts(&format!(
            "instantTextBoxType = {{ name = \"t\" font = \"Main_14\" text = \"{key}\" }}"
        ))
        .into_iter()
        .map(|part| match part {
            PartKind::Text { glyphs, .. } => glyphs.iter().map(|glyph| glyph.ch).collect(),
            _ => panic!("not a text"),
        })
        .collect::<Vec<String>>()
    };
    assert_eq!(shown("MOD_KEY"), ["abc"]);
    assert_eq!(shown("MOD_MISSING"), ["MOD_MISSING"]);
    assert!(shown("").is_empty());
    assert_eq!(shown("MOD_SCRIPTED"), ["[GetDateText]"]);
    let gui = read(
        b"guiTypes = { containerWindowType = { name = \"w\" \
          instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"a\x80b\" } } }",
    );
    let layout = layout(
        gui.template("w").unwrap(),
        None,
        &fixture(),
        &Bindings::default(),
    )
    .unwrap()
    .unwrap();
    assert!(layout.parts.is_empty());
    assert_eq!(layout.diagnostics[0].kind, DiagnosticKind::UndecodableText);
    let mut counts = Counts::default();
    counts.noted(&layout.diagnostics);
    assert_eq!(counts.0["undecodable_text"], 1);
}

#[test]
fn a_window_refuses_what_needs_run_time() {
    content_reader_script::test_game::enter();
    assert_eq!(laid("fade_type = linear fade_time = 0", None), Ok(None));
    for body in [
        "fade_type = linear",
        "animation_type = linear show_position = { x = 4 }",
    ] {
        assert!(laid(body, None).is_err(), "{body}");
    }
    assert!(laid("animation_type = linear", None).is_err());
    assert!(
        laid_bound("animation_type = linear", None, |window, bindings| {
            bindings.bind(window).position = Some([0, 0]);
        })
        .is_ok()
    );
    let layout = laid("verticalScrollbar = \"v\" autohide_scrollbars = no", None)
        .unwrap()
        .unwrap();
    assert!(layout.parts.is_empty());
    assert_eq!(layout.diagnostics[0].kind, DiagnosticKind::MissingScrollbar);
}

#[test]
fn backgrounds_draw_under_the_content() {
    content_reader_script::test_game::enter();
    let layout = laid(
        "size = { width = 40 height = 30 } margin = { left = 5 top = 5 } \
         background = { spriteType = \"GFX_even\" position = { x = 1 y = 2 } frame = 1 \
         orientation = center centerPosition = yes scale = 3 } \
         iconType = { name = \"i\" spriteType = \"GFX_odd\" }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(layout.parts[0].child, None);
    assert_eq!(rect(&layout.parts[0].kind), [1.0, 2.0, 20.0, 10.0]);
    assert_eq!(layout.parts[0].clip, None);
    assert_eq!(layout.parts[1].clip, Some([5, 5, 40, 30]));
    let moved = laid(
        "size = { width = 40 height = 30 } clipping = no \
         background_margin = { left = 10% top = 3 } \
         background = { spriteType = \"GFX_even\" } \
         iconType = { name = \"i\" spriteType = \"GFX_odd\" }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(rect(&moved.parts[0].kind), [4.0, 3.0, 20.0, 10.0]);
    assert_eq!(moved.parts[1].clip, None);
}

#[test]
fn a_fonts_written_viewport_and_auxiliary_colour_reach_their_draws() {
    content_reader_script::test_game::enter();
    let body = "instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"x\" \
                position = { x = 3 y = 1 } maxWidth = 50 maxHeight = 20 format = left \
                vertical_alignment = top }";
    let window = gui(body);
    let window = window.template("w").unwrap();
    let written = |viewport: [Option<i32>; 2], aux: [f32; 4]| {
        let mut fixture = fixture();
        fixture.font.facts.viewport = viewport;
        fixture.font.facts.aux_color = aux;
        fixture
    };
    let mut bindings = Bindings {
        viewport: Some([800, 600]),
        ..Bindings::default()
    };
    let halved = written([Some(400), None], [0.5; 4]);
    let layout = layout(window, None, &halved, &bindings).unwrap().unwrap();
    let PartKind::Text {
        projection,
        modulate,
        font_viewport,
        ..
    } = layout.parts[0].kind
    else {
        panic!("not a text");
    };
    assert_eq!((projection, modulate), ([2.0, 1.0], [1.0; 4]));
    assert_eq!(font_viewport, Some([400, 600]));
    let mut batch = Batch::default();
    draw(&layout, [10, 4], &one(), &mut batch).unwrap();
    let mut plain = Batch::default();
    let unwritten = laid(body, None).unwrap().unwrap();
    draw(&unwritten, [10, 4], &one(), &mut plain).unwrap();
    let [x, y] = plain.quads[0].exact.as_ref().unwrap().corners[2];
    assert_eq!(
        batch.quads[0].exact.as_ref().unwrap().corners[2],
        [x * 2.0, y]
    );
    bindings.viewport = None;
    let refused = layout_refusals(layout_of(window, &halved, &bindings));
    assert!(refused.contains("missing_input"), "{refused}");
    bindings.viewport = Some([800, 600]);
    let bake = gui(
        "buttonType = { name = \"b\" spriteType = \"GFX_text\" font = \"Main_14\" \
         text = \"MOD_KEY\" }",
    );
    let bake = bake.template("w").unwrap();
    let refused = layout_refusals(layout_of(bake, &halved, &bindings));
    assert!(refused.contains("auxiliary colour"), "{refused}");
    let white = written([None, None], [1.0; 4]);
    assert!(layout_of(bake, &white, &bindings).is_ok());
}

#[test]
fn a_zero_font_viewport_keeps_its_siblings() {
    content_reader_script::test_game::enter();
    let window = gui(
        "iconType = { name = \"i\" spriteType = \"GFX_even\" position = { x = 5 y = 6 } } \
         instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"x\" \
         position = { x = 3 y = 1 } maxWidth = 50 maxHeight = 20 format = left \
         vertical_alignment = top }",
    );
    let window = window.template("w").unwrap();
    let bindings = Bindings {
        viewport: Some([800, 600]),
        ..Bindings::default()
    };
    let plain = layout(window, None, &fixture(), &bindings)
        .unwrap()
        .unwrap();
    let mut zero = fixture();
    zero.font.facts.viewport = [Some(0), None];
    let laid = layout(window, None, &zero, &bindings).unwrap().unwrap();
    assert_eq!(laid.parts.len(), plain.parts.len());
    let (projection, font_viewport) = laid
        .parts
        .iter()
        .find_map(|part| match &part.kind {
            PartKind::Text {
                projection,
                font_viewport,
                ..
            } => Some((*projection, *font_viewport)),
            _ => None,
        })
        .expect("the text is laid out");
    assert!(projection[0].is_infinite() && projection[0] > 0.0);
    assert_eq!(projection[1], 1.0);
    assert_eq!(font_viewport, Some([0, 600]));
    let (mut drawn, mut expected) = (Batch::default(), Batch::default());
    draw(&laid, [10, 4], &one(), &mut drawn).unwrap();
    draw(&plain, [10, 4], &one(), &mut expected).unwrap();
    assert_eq!(drawn.quads.len(), expected.quads.len());
    let finite = |quad: &Quad| {
        quad.exact
            .as_ref()
            .map_or(quad.rect.iter().all(|value| value.is_finite()), |exact| {
                exact
                    .corners
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
            })
    };
    let icons: Vec<_> = drawn.quads.iter().filter(|quad| finite(quad)).collect();
    assert!(!icons.is_empty());
    for quad in &icons {
        assert!(expected.quads.contains(quad));
    }
    assert!(drawn.quads.iter().any(|quad| !finite(quad)));
}

fn layout_of(
    window: &Element,
    fixture: &Fixture,
    bindings: &Bindings,
) -> Result<Option<Layout>, Refusals> {
    layout(window, None, fixture, bindings)
}

fn layout_refusals(laid: Result<Option<Layout>, Refusals>) -> String {
    match laid {
        Ok(_) => String::new(),
        Err(refusals) => refusals
            .iter()
            .map(|refusal| format!("{} {}", refusal.kind.name(), refusal.detail))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

#[test]
fn run_time_helpers_follow_the_game() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("fade_type = linear fade_time = 1000");
    let window = gui.template("w").unwrap();
    let mut state = Instance::construct(window, [100, 100], &resources).unwrap();
    state.show(window, window).unwrap();
    state.tick(window, &resources, dt(2.0)).unwrap();
    assert!(state.drawn());
    state.hide_window(window, window).unwrap();
    assert!(!state.drawn());
    let mut pulse = lifecycle::Pulse::default();
    pulse.update(1.0, 0.25).unwrap();
    assert_eq!(pulse.alpha(), 0.5);
    pulse.update(1.0, 0.5).unwrap();
    assert_eq!(pulse.alpha(), 0.5);
    assert!(
        lifecycle::Pulse::default()
            .update(f32::INFINITY, 0.1)
            .is_err()
    );
    assert!(lifecycle::Pulse::default().update(1e9, 1.0).is_err());
    let kind = SpriteKind::FrameAnimated {
        seconds_per_frame: 0.5,
        looping: false,
        play_on_show: false,
        pause: 0.0,
    };
    let mut clock = lifecycle::FrameClock::new();
    clock.update(&kind, 2, 0.1).unwrap();
    assert_eq!(clock.frame(), None);
    clock.play();
    clock.update(&kind, 2, 0.1).unwrap();
    assert_eq!(clock.frame(), Some(1));
    clock.update(&kind, 2, 1.0).unwrap();
    assert_eq!(clock.frame(), Some(2));
    let layout = laid_bound(
        "fade_type = linear gridBoxType = { name = \"g\" }",
        None,
        |window, bindings| bindings.bind(window).opacity = Some(0.25),
    )
    .unwrap()
    .unwrap();
    assert_eq!(layout.holders[0].item_opacity(), Some(0.25));
    let item = gui_item("orientation = center_down size = { width = 10 height = 10 }");
    assert_eq!(
        item_offset(item.template("item").unwrap(), [800, 600]),
        Ok([400, 600])
    );
}

fn gui_item(body: &str) -> native::Gui {
    read(
        format!("guiTypes = {{ containerWindowType = {{ name = \"item\" {body} }} }}\n").as_bytes(),
    )
}

#[test]
fn a_clipping_window_scissors_its_children() {
    content_reader_script::test_game::enter();
    let layout = laid(
        "size = { width = 10 height = 10 } clipping = yes \
         iconType = { name = \"i\" spriteType = \"GFX_even\" position = { x = 5 y = 5 } }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(layout.parts[0].clip, Some([0, 0, 10, 10]));
    let mut batch = Batch::default();
    draw(&layout, [100, 200], &one(), &mut batch).unwrap();
    let exact = batch.quads[0].exact.as_ref().unwrap();
    assert_eq!(exact.corners[0], [105.0, 205.0]);
    assert_eq!(exact.corners[2], [125.0, 215.0]);
    assert_eq!(exact.scissor, Some([100, 200, 110, 210]));
    assert_eq!(
        crate::sprites::device_scissor([100, 200, 110, 210], 1.25, [1000, 1000]),
        Some([125, 250, 12, 12])
    );
}

#[test]
fn a_refused_draw_pushes_nothing() {
    content_reader_script::test_game::enter();
    struct Some2(SpriteSheet);
    impl Sheets for Some2 {
        fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
            (sprite == b"GFX_even").then_some(&self.0)
        }
        fn progress(&self, _: &[u8]) -> Option<[&SpriteSheet; 2]> {
            None
        }
        fn masking(&self, _: &[u8]) -> Option<&SpriteSheet> {
            None
        }
        fn mask(&self, _: &[u8]) -> Option<&SpriteSheet> {
            None
        }
    }
    let layout = laid(
        "iconType = { name = \"a\" spriteType = \"GFX_even\" } \
         iconType = { name = \"b\" spriteType = \"GFX_odd\" }",
        None,
    )
    .unwrap()
    .unwrap();
    let mut batch = Batch::default();
    batch.cell(1, [0.0; 2], [1.0; 2], [0.0; 4], [1.0; 4]);
    let error = draw(&layout, [0, 0], &Some2(even_sheet()), &mut batch).unwrap_err();
    assert_eq!(error.kind, RefusalKind::MissingResource);
    assert_eq!(batch.quads.len(), 1);
}

#[test]
fn grid_slots_follow_the_formats_and_limits() {
    content_reader_script::test_game::enter();
    let slots = |body: &str, count| {
        let gui = gui(&format!(
            "gridBoxType = {{ name = \"g\" size = {{ width = 101 height = 51 }} slotsize = {{ width = 20 height = 10 }} {body} }}"
        ));
        grid_slots(
            gui.template("w").unwrap().find(Kind::Grid, "g").unwrap(),
            [300, 300],
            count,
            None,
        )
    };
    assert_eq!(
        slots("format = UPPER_LEFT max_slots_horizontal = 2", 3),
        Ok(vec![[0, 0], [20, 0], [0, 10]])
    );
    assert_eq!(
        slots(
            "format = UPPER_LEFT add_horizontal = no max_slots_vertical = 2",
            3
        ),
        Ok(vec![[0, 0], [0, 10], [20, 0]])
    );
    assert_eq!(
        slots(
            "add_horizontal = no max_slots_vertical = 1 max_slots_horizontal = 2",
            3
        )
        .map(|slots| slots.len()),
        Ok(2)
    );
    assert_eq!(slots("", 2), Ok(vec![[40, 0], [60, 0]]));
    assert_eq!(slots("format = DOWN", 1), Ok(vec![[40, 41]]));
    assert_eq!(slots("format = LEFT", 1), Ok(vec![[0, 20]]));
    assert_eq!(slots("format = RIGHT", 1), Ok(vec![[81, 20]]));
    assert_eq!(slots("format = LOWER_LEFT", 1), Ok(vec![[0, 41]]));
    assert_eq!(slots("format = UPPER_RIGHT", 1), Ok(vec![[81, 0]]));
    assert_eq!(
        slots("format = UPPER_LEFT orientation = center", 1),
        Ok(vec![[150, 150]])
    );
    assert_eq!(
        slots("format = UPPER_LEFT orientation = center_left", 1),
        Ok(vec![[0, 0]])
    );
}

#[test]
fn a_grid_finalises_its_items() {
    content_reader_script::test_game::enter();
    let gui = read(
        b"guiTypes = { containerWindowType = { name = \"w\" size = { width = 300 height = 300 } \
          gridBoxType = { name = \"g\" position = { x = 7 y = 8 } \
          size = { width = 100 height = 50 } slotsize = { width = 20 height = 10 } \
          format = UPPER_LEFT max_slots_horizontal = 2 \
          padding = { left = 3 top = 4 right = 5 bottom = 6 } } } \
          containerWindowType = { name = \"item\" position = { x = 2 y = 1 } \
          size = { width = 30 height = 12 } } }
",
    );
    let grid = gui.template("w").unwrap().find(Kind::Grid, "g").unwrap();
    let item = gui.template("item").unwrap();
    let finalised = finalize_grid(grid, [300, 300], [800, 600], &[item, item, item], None).unwrap();
    assert_eq!(finalised.items, [[2, 1], [22, 1], [2, 11]]);
    assert_eq!(finalised.origin, [7, 8]);
    assert_eq!(finalised.relaid, [10, 12]);
    assert_eq!(finalised.extent, [105, 56]);
    let gui = read(
        b"guiTypes = { containerWindowType = { name = \"w\" \
          gridBoxType = { name = \"g\" size = { width = 100 height = 50 } \
          slotsize = { width = 50% height = 10 } format = UPPER_LEFT } } \
          containerWindowType = { name = \"item\" orientation = center \
          size = { width = 10 height = 10 } } }
",
    );
    let grid = gui.template("w").unwrap().find(Kind::Grid, "g").unwrap();
    let item = gui.template("item").unwrap();
    let finalised = finalize_grid(grid, [300, 300], [800, 600], &[item, item], None).unwrap();
    assert_eq!(finalised.items, [[400, 300], [450, 300]]);
    assert_eq!(finalised.places(), [[400, 300], [450, 300]]);
    let window = gui.template("w").unwrap();
    let mut bindings = Bindings::default();
    bindings.bind(grid).position = Some([5, 6]);
    let layout = layout(window, Some([300, 300]), &fixture(), &bindings)
        .unwrap()
        .unwrap();
    assert_eq!(
        layout.holders[0].finalize(grid, [800, 600], &[item, item]),
        Ok(vec![[405, 306], [455, 306]])
    );
}

#[test]
fn explicit_grid_positions_bypass_append_limits_and_scale_the_coordinates() {
    content_reader_script::test_game::enter();
    let gui = read(
        br#"guiTypes = {
            containerWindowType = { name = w
                gridBoxType = { name = g position = { x = 7 y = 8 }
                    size = { width = 100 height = 50 }
                    slotsize = { width = 20 height = 3 } format = UPPER_LEFT
                    max_slots_horizontal = 1 max_slots_vertical = 1 } }
            containerWindowType = { name = item position = { x = 2 y = 1 }
                size = { width = 30 height = 12 } }
        }"#,
    );
    let grid = gui.template("w").unwrap().find(Kind::Grid, "g").unwrap();
    let item = gui.template("item").unwrap();
    let automatic = finalize_grid(grid, [300, 300], [800, 600], &[item, item], None).unwrap();
    assert_eq!(automatic.items, [[2, 1]]);
    let explicit = finalize_grid_positions(
        grid,
        [300, 300],
        [800, 600],
        [(item, [0, 12]), (item, [0, 27])],
        None,
    )
    .unwrap();
    assert_eq!(explicit.items, [[2, 37], [2, 82]]);
    assert_eq!(explicit.places(), [[9, 45], [9, 90]]);
    assert_eq!(explicit.extent, [100, 94]);
}

#[test]
fn explicit_grid_positions_keep_the_format_and_viewport_constructor_offset() {
    content_reader_script::test_game::enter();
    let gui = read(
        br#"guiTypes = {
            containerWindowType = { name = w
                gridBoxType = { name = g position = { x = 7 y = 8 }
                    size = { width = 100 height = 51 }
                    slotsize = { width = 50% height = 10 } format = RIGHT } }
            containerWindowType = { name = item orientation = center
                size = { width = 10 height = 10 } }
        }"#,
    );
    let grid = gui.template("w").unwrap().find(Kind::Grid, "g").unwrap();
    let item = gui.template("item").unwrap();
    let explicit =
        finalize_grid_positions(grid, [300, 300], [800, 600], [(item, [1, 2])], None).unwrap();
    assert_eq!(explicit.items, [[350, 330]]);
    assert_eq!(explicit.places(), [[357, 338]]);
}

#[test]
fn holder_items_compose_in_attach_order() {
    content_reader_script::test_game::enter();
    let layout = laid(
        "size = { width = 40 height = 40 } clipping = yes \
         iconType = { name = \"a\" spriteType = \"GFX_even\" } \
         gridBoxType = { name = \"g\" position = { x = 1 y = 2 } } \
         iconType = { name = \"b\" spriteType = \"GFX_odd\" }",
        None,
    )
    .unwrap()
    .unwrap();
    let item_gui = gui(
        "size = { width = 9 height = 9 } iconType = { name = \"i\" spriteType = \"GFX_four\" }",
    );
    let item = layout_item(&item_gui);
    let holder = &layout.holders[0];
    let placed = holder.place([3, 4], &item);
    assert_eq!(rect(&placed[0].kind), [3.0, 4.0, 10.0, 10.0]);
    assert_eq!(placed[0].clip, Some([0, 0, 40, 40]));
    let mut batch = Batch::default();
    draw_composed(&layout, &[placed], [0, 0], &one(), &mut batch).unwrap();
    let tops: Vec<_> = batch
        .quads
        .iter()
        .map(|quad| quad.exact.as_ref().unwrap().corners[0])
        .collect();
    assert_eq!(tops, [[0.0, 0.0], [3.0, 4.0], [0.0, 0.0]]);
    let background = laid(
        "size = { width = 40 height = 40 } background = { spriteType = \"GFX_even\" }",
        None,
    )
    .unwrap()
    .unwrap();
    let mut batch = Batch::default();
    draw_composed(&background, &[], [0, 0], &one(), &mut batch).unwrap();
    assert_eq!(batch.quads.len(), 1);
}

fn layout_item(gui: &native::Gui) -> Layout {
    layout(
        gui.template("w").unwrap(),
        None,
        &fixture(),
        &Bindings::default(),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn bound_holder_positions_replace_the_authored() {
    content_reader_script::test_game::enter();
    let gui = gui("size = { width = 100 height = 100 } \
         OverlappingElementsBoxType = { name = \"l\" size = { x = 100 y = 50 } position = { x = 3 y = 4 } } \
         gridBoxType = { name = \"g\" position = { x = 1 y = 2 } }");
    let window = gui.template("w").unwrap();
    let list = window.find(Kind::List, "l").unwrap();
    let grid = window.find(Kind::Grid, "g").unwrap();
    let mut bindings = Bindings::default();
    bindings.bind(list).position = Some([30, 40]);
    bindings.bind(grid).position = Some([10, 20]);
    let layout = layout(window, None, &fixture(), &bindings)
        .unwrap()
        .unwrap();
    let [list_holder, grid_holder] = [&layout.holders[0], &layout.holders[1]];
    assert_eq!(list_holder.position, Some([30, 40]));
    assert_eq!(
        list_places(list, [100, 100], [10, 10], 1, list_holder.position),
        Ok(vec![(0, [30, 40])])
    );
    assert_eq!(
        grid_slots(grid, [100, 100], 1, grid_holder.position),
        Ok(vec![[10, 20]])
    );
}

#[test]
fn a_position_holds_a_code_placed_window() {
    content_reader_script::test_game::enter();
    let gui = gui("size = { width = 100 height = 100 }          containerWindowType = { name = \"c\" position = { x = 20 y = 10 } size = { width = 50 height = 50 }          positionType = { name = \"p\" position = { x = 36 y = 6 } } }");
    let window = gui.template("w").unwrap();
    let layout = layout(window, None, &fixture(), &Bindings::default())
        .unwrap()
        .unwrap();
    let [holder] = &layout.holders[..] else {
        panic!("one holder: {:?}", layout.holders.iter().map(|holder| &holder.path).collect::<Vec<_>>());
    };
    assert!(holder.path.ends_with('p'), "{}", holder.path);
    assert_eq!(holder.content, [50, 50]);
}

#[test]
fn list_places_follow_the_layout() {
    content_reader_script::test_game::enter();
    let places = |body: &str, count| {
        let gui = gui(&format!(
            "OverlappingElementsBoxType = {{ name = \"l\" size = {{ x = 100 y = 50 }} {body} }}"
        ));
        list_places(
            gui.template("w").unwrap().find(Kind::List, "l").unwrap(),
            [300, 200],
            [42, 20],
            count,
            None,
        )
    };
    assert_eq!(places("", 2), Ok(vec![(0, [0, 0]), (1, [43, 0])]));
    assert_eq!(
        places("", 3),
        Ok(vec![(0, [0, 0]), (1, [29, 0]), (2, [58, 0])])
    );
    assert_eq!(places("format = center", 1), Ok(vec![(0, [28, 0])]));
    assert_eq!(
        places("format = right", 2),
        Ok(vec![(0, [101, 0]), (1, [58, 0])])
    );
    assert_eq!(
        places("format = up", 2),
        Ok(vec![(0, [0, 51]), (1, [0, 30])])
    );
    assert_eq!(
        places("format = down", 2),
        Ok(vec![(0, [0, 0]), (1, [0, 21])])
    );
    assert_eq!(
        places("first_on_top = yes", 2),
        Ok(vec![(1, [43, 0]), (0, [0, 0])])
    );
    let gui = gui("OverlappingElementsBoxType = { name = \"l\" size = { x = 10 y = 50 } }");
    let list = gui.template("w").unwrap().find(Kind::List, "l").unwrap();
    assert_eq!(
        list_places(list, [300, 200], [42, 20], 1, None),
        Ok(vec![(0, [i32::MIN, 0])])
    );
    assert_eq!(places("orientation = center", 1), Ok(vec![(0, [150, 100])]));
    assert_eq!(
        places("orientation = lower_right", 1),
        Ok(vec![(0, [300, 200])])
    );
    assert_eq!(places("orientation = center_up", 1), Ok(vec![(0, [0, 0])]));
}

#[test]
fn parts_name_their_top_level_child() {
    content_reader_script::test_game::enter();
    let layout = laid(
        "iconType = { name = \"a\" spriteType = \"GFX_odd\" } \
         gridBoxType = { name = \"g\" } \
         containerWindowType = { name = \"c\" size = { width = 9 height = 9 } \
         iconType = { name = \"b\" spriteType = \"GFX_odd\" } \
         iconType = { name = \"d\" spriteType = \"GFX_odd\" } }",
        None,
    )
    .unwrap()
    .unwrap();
    let children: Vec<_> = layout.parts.iter().map(|part| part.child).collect();
    assert_eq!(children, [Some(0), Some(2), Some(2)]);
    assert_eq!(layout.parts[2].path, "w.c.d");
    assert_eq!(layout.holders[0].before, 1);
}

#[test]
fn a_bound_texture_replaces_the_frame() {
    content_reader_script::test_game::enter();
    let texture = Texture {
        texture: 7,
        uv: [0.5, 0.25, 0.75, 0.5],
        size: [10, 6],
    };
    let layout = laid_bound(
        "iconType = { name = \"a\" spriteType = \"GFX_odd\" centerPosition = yes \
         position = { x = 20 y = 20 } scale = 2 } \
         iconType = { name = \"b\" spriteType = \"GFX_odd\" scale = 0.75 }",
        None,
        |window, bindings| {
            for name in ["a", "b"] {
                bindings
                    .bind(window.find(Kind::Icon, name).unwrap())
                    .texture = Some(texture);
            }
        },
    )
    .unwrap()
    .unwrap();
    let PartKind::Cell {
        texture: page,
        uv,
        geometry,
        ..
    } = layout.parts[0].kind
    else {
        panic!("not a cell");
    };
    assert_eq!((page, uv), (7, texture.uv));
    assert_eq!(geometry.rect, [10.0, 14.0, 20.0, 12.0]);
    assert_eq!(rect(&layout.parts[1].kind), [0.0, -0.5, 7.5, 4.5]);
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    assert_eq!(batch.quads[1].texture, 7);
    let exact = batch.quads[1].exact.as_ref().unwrap();
    assert_eq!(
        exact.corners,
        [[0.0, -0.5], [7.5, -0.5], [7.5, 4.0], [0.0, 4.0]]
    );
    assert_eq!(exact.uv[0], [0.5, 0.25]);
}

#[test]
fn assets_list_every_named_sprite_and_font() {
    content_reader_script::test_game::enter();
    let gui = gui("background = { spriteType = \"GFX_bg\" } \
         iconType = { name = \"a\" spriteType = \"GFX_odd\" } \
         containerWindowType = { name = \"c\" \
         buttonType = { name = \"b\" quadTextureSprite = \"GFX_even\" buttonFont = \"F\" } \
         instantTextBoxType = { name = \"t\" font = \"Main_14\" } \
         gridBoxType = { name = \"g\" background = { spriteType = \"GFX_grid\" } } }");
    let localisation = Localisation::default();
    let assets = assets_in(gui.template("w").unwrap(), &|_| None, &localisation);
    assert_eq!(
        assets.sprites.into_iter().collect::<Vec<_>>(),
        [
            b"GFX_bg".as_slice(),
            FALLBACK_SPRITE,
            b"GFX_even",
            b"GFX_grid",
            b"GFX_odd"
        ]
    );
    assert_eq!(
        assets.fonts.into_iter().collect::<Vec<_>>(),
        [b"F".to_vec(), b"Main_14".to_vec()]
    );
    let document = read(
        format!(
            "guiTypes = {{ {SCROLLBAR} containerWindowType = {{ name = \"w\" \
             verticalScrollbar = \"sb\" \
             listBoxType = {{ name = \"l\" background = \"GFX_list\" }} \
             editBoxType = {{ name = \"e\" font = \"E\" }} }} }}"
        )
        .as_bytes(),
    );
    let assets = assets_in(
        document.template("w").unwrap(),
        &|name| document.template(name),
        &localisation,
    );
    for sprite in ["GFX_list", "GFX_four", "GFX_bar3x17", "GFX_even"] {
        assert!(assets.sprites.contains(sprite.as_bytes()), "{sprite}");
    }
    assert!(assets.fonts.contains(b"E".as_slice()));
    let mut merged = Assets {
        textures: [b"gfx/edit.dds".to_vec()].into(),
        ..Assets::default()
    };
    merged.extend(assets.clone());
    assert!(merged.textures.contains(b"gfx/edit.dds".as_slice()));
    assert!(merged.fonts.contains(b"E".as_slice()));
    assert!(merged.sprites.is_superset(&assets.sprites));
}

#[test]
fn list_entries_choose_the_native_factory() {
    content_reader_script::test_game::enter();
    let document = read(
        b"guiTypes = { containerWindowType = { name = \"c\" } \
          windowType = { name = \"f\" } iconType = { name = \"i\" } }",
    );
    let lookup = |name: &[u8]| document.template(name);
    assert!(matches!(
        list_entry("l", b"c", &lookup),
        Ok(ListEntry::Container(window)) if window.kind == Kind::Container
    ));
    let Ok(ListEntry::Fixed(fixed)) = list_entry("l", b"f", &lookup) else {
        panic!("a windowType is a fixed window");
    };
    assert_eq!(fixed.kind, Kind::Window);
    assert_eq!(
        list_entry("l", b"i", &lookup).unwrap_err().kind,
        RefusalKind::TypeConfusion
    );
    assert_eq!(
        list_entry("l", b"none", &lookup).unwrap_err().kind,
        RefusalKind::MissingInput
    );
}

const FIXED_ENTRY: &[u8] = b"guiTypes = {
    windowType = { name = \"base\"
        guiButtonType = { name = \"frame\" spriteType = \"GFX_cornered\" position = { x = 2 y = 3 } }
    }
    windowType = { name = \"entry\" parent = \"base\" background = \"frame\"
        horizontalBorder = \"h\" verticalBorder = \"v\"
        position = { x = 10 y = 20 } size = { x = 60 y = 40 } orientation = \"center\"
        iconType = { name = \"flag\" spriteType = \"GFX_even\" position = { x = 4 y = 5 }
            orientation = \"lower_right\" }
        guiButtonType = { name = \"go\" spriteType = \"GFX_even\" position = { x = 1 y = 2 } }
        textBoxType = { name = \"label\" font = \"Main_14\" text = \"MOD_INNER\"
            position = { x = 0 y = 30 } maxWidth = 50 maxHeight = 14 }
        multiSpriteButtonType = { name = \"multi\" spriteType = \"GFX_even\" }
        checkboxType = { name = \"tick\" spriteType = \"GFX_even\" position = { x = 30 y = 0 } }
        windowType = { name = \"inner\" position = { x = 5 y = 6 } size = { x = 20 y = 10 }
            orientation = \"upper_right\"
            iconType = { name = \"dot\" spriteType = \"GFX_even\" } }
    }
}";

#[test]
fn a_window_type_entry_lays_out_as_its_fixed_window() {
    content_reader_script::test_game::enter();
    let resources = fixture_with(read(FIXED_ENTRY));
    let entry = resources.gui_type(b"entry").unwrap();
    assert!(matches!(
        list_entry("l", b"entry", &|name| resources.gui_type(name)),
        Ok(ListEntry::Fixed(window)) if std::ptr::eq(window, entry)
    ));
    assert_eq!(
        fixed::fixed_window_origin(entry, [800, 600]),
        Ok([410, 320])
    );
    let bindings = Bindings {
        viewport: Some([800, 600]),
        ..Bindings::default()
    };
    let layout = fixed_window_layout(entry, &resources, &bindings)
        .unwrap()
        .unwrap();
    assert_eq!(layout.size, [60, 40]);
    let drawn: Vec<(Option<usize>, Rect)> = layout
        .parts
        .iter()
        .map(|part| (part.child, rect(&part.kind)))
        .collect();
    assert_eq!(
        drawn,
        [
            (None, [2.0, 3.0, 4.0, 4.0]),
            (Some(0), [64.0, 45.0, 20.0, 10.0]),
            (Some(1), [1.0, 2.0, 20.0, 10.0]),
            (Some(2), [0.0, 30.0, 50.0, 14.0]),
            (Some(4), [30.0, 0.0, 20.0, 10.0]),
            (Some(5), [65.0, 6.0, 20.0, 10.0]),
        ]
    );
    assert!(matches!(
        &layout.parts[3].kind,
        PartKind::TextSprite { sprite, bake, .. }
            if sprite == b"label" && bake.size == [50, 14] && bake.glyphs.len() == 1
    ));
    let PartKind::Tiles { quads, .. } = &layout.parts[0].kind else {
        panic!("a cornered background");
    };
    let far = quads.iter().fold([0.0f32; 2], |far, (rect, _)| {
        [far[0].max(rect[0] + rect[2]), far[1].max(rect[1] + rect[3])]
    });
    assert_eq!(far, [62.0, 43.0]);
}

#[test]
fn a_window_type_entry_draws_through_its_instance() {
    content_reader_script::test_game::enter();
    let resources = fixture_with(read(FIXED_ENTRY));
    let entry = resources.gui_type(b"entry").unwrap();
    let mut row = Instance::construct(entry, [800, 600], &resources).unwrap();
    assert_eq!(row.layout(entry, &resources, None), Ok(None));
    row.gui_show(entry, entry).unwrap();
    let shown = row.layout(entry, &resources, None).unwrap().unwrap();
    assert_eq!(shown.parts.len(), 6);
    let flag = entry.find(Kind::Icon, "flag").unwrap();
    row.set_hide_flag(entry, flag, true).unwrap();
    row.hide(entry, flag).unwrap();
    let without = row.layout(entry, &resources, None).unwrap().unwrap();
    assert_eq!(without.parts.len(), 5);
    assert!(without.parts.iter().all(|part| part.child != Some(0)));
}

#[test]
fn a_fixed_window_attaches_as_the_game_attaches() {
    content_reader_script::test_game::enter();
    let resources = fixture_with(read(
        b"guiTypes = { windowType = { name = \"dup\" size = { x = 100 y = 100 }
            iconType = { name = \"a\" spriteType = \"GFX_even\" position = { x = 1 y = 1 } }
            iconType = { name = \"A\" spriteType = \"GFX_even\" position = { x = 2 y = 2 } }
            guiButtonType = { name = \"b\" spriteType = \"GFX_even\" position = { x = 3 y = 3 } }
            multiSpriteButtonType = { name = \"b\" spriteType = \"GFX_even\"
                position = { x = 4 y = 4 } }
            dropDownBoxType = { name = \"d\" }
        }
        windowType = { name = \"confused\" size = { x = 10 y = 10 }
            guiButtonType = { name = \"t\" spriteType = \"GFX_even\" }
            textBoxType = { name = \"t\" font = \"Main_14\" } }
        windowType = { name = \"lost\" background = \"nowhere\" }
        windowType = { name = \"icon_background\" background = \"i\"
            iconType = { name = \"i\" spriteType = \"GFX_even\" } }
        windowType = { name = \"wide\" position = { x = 100 y = 0 }
            size = { x = max_resolution y = 50 } }
        windowType = { name = \"inheriting\" parent = \"dup\" position = { x = 1 y = 1 }
            size = { x = max_resolution y = 50 } }
        }",
    ));
    let bindings = Bindings {
        viewport: Some([800, 600]),
        ..Bindings::default()
    };
    let laid =
        |name: &[u8]| fixed_window_layout(resources.gui_type(name).unwrap(), &resources, &bindings);
    let dup = laid(b"dup").unwrap().unwrap();
    let drawn: Vec<(Option<usize>, Rect)> = dup
        .parts
        .iter()
        .map(|part| (part.child, rect(&part.kind)))
        .collect();
    assert_eq!(
        drawn,
        [
            (Some(1), [2.0, 2.0, 20.0, 10.0]),
            (Some(3), [4.0, 4.0, 20.0, 10.0])
        ]
    );
    let window = resources.gui_type(b"dup").unwrap();
    let find = |find, name: &str| fixed::fixed_window_find(window, find, name);
    assert!(std::ptr::eq(
        find(fixed::FixedFind::Button, "B").unwrap(),
        &window.children[3]
    ));
    assert!(std::ptr::eq(
        find(fixed::FixedFind::Icon, "a").unwrap(),
        &window.children[1]
    ));
    assert!(find(fixed::FixedFind::InstantTextBox, "a").is_none());
    for lookup in [
        fixed::FixedFind::Button,
        fixed::FixedFind::InstantTextBox,
        fixed::FixedFind::Icon,
        fixed::FixedFind::TextBox,
        fixed::FixedFind::CheckBox,
        fixed::FixedFind::EditBox,
        fixed::FixedFind::Browser,
        fixed::FixedFind::CurveGraph,
        fixed::FixedFind::DropDownBox,
        fixed::FixedFind::Scrollbar,
        fixed::FixedFind::OverlappingElementsBox,
        fixed::FixedFind::StandardListBox,
        fixed::FixedFind::SmoothListBox,
        fixed::FixedFind::Subwindow,
    ] {
        assert!(find(lookup, "zz").is_none(), "{lookup:?}");
    }
    assert!(std::ptr::eq(
        find(fixed::FixedFind::DropDownBox, "d").unwrap(),
        &window.children[4]
    ));
    let kinds = |name: &[u8]| {
        laid(name)
            .unwrap_err()
            .iter()
            .map(|refusal| refusal.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(b"confused"), [RefusalKind::TypeConfusion]);
    assert_eq!(kinds(b"lost"), [RefusalKind::NullRead]);
    assert_eq!(kinds(b"icon_background"), [RefusalKind::TypeConfusion]);
    assert_eq!(kinds(b"inheriting"), [RefusalKind::MissingInput]);
    assert_eq!(laid(b"wide").unwrap().unwrap().size, [700, 50]);
    let unbound = fixed_window_layout(
        resources.gui_type(b"wide").unwrap(),
        &resources,
        &Bindings::default(),
    );
    assert_eq!(unbound.unwrap_err()[0].kind, RefusalKind::NoParent);
}

#[test]
fn a_fixed_window_scrollbar_draws_only_when_shown() {
    content_reader_script::test_game::enter();
    let bar = |name: &str, hide: &str| {
        format!(
            "scrollbarType = {{ name = \"{name}\" {hide} position = {{ x = 2 y = 3 }}                  size = {{ x = 10 y = 60 }} maxValue = 4 startValue = 1 minValue = 0                  slider = \"slider\" track = \"track\" leftButton = \"down\"                  rightButton = \"up\" buttonType = {{ name = \"up\" spriteType = \"GFX_four\" }}                  buttonType = {{ name = \"down\" spriteType = \"GFX_four\" }}                  guiButtonType = {{ name = \"track\" spriteType = \"GFX_bar3x17\" }}                  guiButtonType = {{ name = \"slider\" spriteType = \"GFX_bar3x17\" }} }}"
        )
    };
    let resources = fixture_with(read(
        format!(
            "guiTypes = {{ windowType = {{ name = \"bars\" size = {{ x = 100 y = 100 }} {} {} }} }}",
            bar("shown", ""),
            bar("hidden", "hide = yes")
        )
        .as_bytes(),
    ));
    let window = resources.gui_type(b"bars").unwrap();
    let bindings = Bindings {
        viewport: Some([800, 600]),
        ..Bindings::default()
    };
    let layout = fixed_window_layout(window, &resources, &bindings)
        .unwrap()
        .unwrap();
    assert_eq!(
        rects_of(&layout.parts),
        [
            [2.0, 3.0, 10.0, 10.0],
            [2.0, 53.0, 10.0, 10.0],
            [2.0, 13.0, 3.0, 17.0],
            [2.0, 18.0, 3.0, 17.0]
        ]
    );
    assert!(layout.parts.iter().all(|part| part.child == Some(0)));
    let mut row = Instance::construct(window, [800, 600], &resources).unwrap();
    row.gui_show(window, window).unwrap();
    let through = row.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(rects_of(&through.parts), rects_of(&layout.parts));
}

#[test]
fn a_text_box_name_used_as_a_sprite_is_session_history() {
    content_reader_script::test_game::enter();
    let body = "iconType = { name = \"i\" spriteType = \"label\" }";
    let mut resources = fixture_with(gui(body));
    let window = gui(body);
    let bindings = Bindings::default();
    let plain = layout(window.template("w").unwrap(), None, &resources, &bindings)
        .unwrap()
        .unwrap();
    assert!(matches!(
        &plain.parts[0].kind,
        PartKind::Sprite { sprite, .. } if sprite == FALLBACK_SPRITE
    ));
    resources.templates.push(read(FIXED_ENTRY));
    let refused = layout(window.template("w").unwrap(), None, &resources, &bindings).unwrap_err();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].kind, RefusalKind::MissingInput);
    let entry = resources.gui_type(b"entry").unwrap();
    let bindings = Bindings {
        viewport: Some([800, 600]),
        ..Bindings::default()
    };
    assert!(fixed_window_layout(entry, &resources, &bindings).is_ok());
}

#[test]
fn a_window_type_entry_collects_what_it_draws() {
    content_reader_script::test_game::enter();
    let document = read(FIXED_ENTRY);
    let entry = document.template("entry").unwrap();
    let assets = assets_in(
        entry,
        &|name| document.template(name),
        &fixture().localisation,
    );
    for sprite in [&b"GFX_cornered"[..], b"GFX_even", b"label"] {
        assert!(assets.sprites.contains(sprite), "{}", lossy(sprite));
    }
    assert!(assets.fonts.contains(&b"Main_14"[..]));
    let resources = Asked {
        inner: &fixture_with(read(FIXED_ENTRY)),
        sprites: Default::default(),
        fonts: Default::default(),
    };
    let entry = resources.inner.gui_type(b"entry").unwrap();
    let bindings = Bindings {
        viewport: Some([800, 600]),
        ..Bindings::default()
    };
    fixed_window_layout(entry, &resources, &bindings)
        .unwrap()
        .unwrap();
    for asked in resources.sprites.borrow().iter() {
        assert!(assets.sprites.contains(asked), "{}", lossy(asked));
    }
    for asked in resources.fonts.borrow().iter() {
        assert!(assets.fonts.contains(asked), "{}", lossy(asked));
    }
}

struct Asked<'r> {
    inner: &'r Fixture,
    sprites: RefCell<BTreeSet<Vec<u8>>>,
    fonts: RefCell<BTreeSet<Vec<u8>>>,
}

impl Resources for Asked<'_> {
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.sprites.borrow_mut().insert(name.to_vec());
        self.inner.sprite(name)
    }
    fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        self.fonts.borrow_mut().insert(name.to_vec());
        self.inner.font(name)
    }
    fn localisation(&self) -> &Localisation {
        self.inner.localisation()
    }
    fn text_colors(&self) -> &TextColors {
        self.inner.text_colors()
    }
    fn text_effect(&self) -> Result<Effect, String> {
        self.inner.text_effect()
    }
    fn effect(&self, file: &str) -> Result<Effect, String> {
        self.inner.effect(file)
    }
    fn shield_flag(&self, small: bool, index: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        self.inner.shield_flag(small, index)
    }
    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> {
        self.inner.text_flag(tag)
    }
    fn gui_type(&self, name: &[u8]) -> Option<&Element> {
        self.inner.gui_type(name)
    }
    fn session_text_sprite(&self, name: &[u8]) -> bool {
        self.inner.session_text_sprite(name)
    }
    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> {
        self.inner.sprite_animations(sprite)
    }
}

#[test]
fn acquisition_follows_construction() {
    content_reader_script::test_game::enter();
    let document = read(
        b"guiTypes = { scrollbarType = { name = \"base\" \
          buttonType = { name = \"up\" spriteType = \"GFX_four\" buttonFont = \"Main_14\" \
          buttonText = \"MOD_COLOURED\" } \
          buttonType = { name = \"down\" spriteType = \"GFX_four\" } \
          guiButtonType = { name = \"track\" spriteType = \"GFX_cornered\" } \
          guiButtonType = { name = \"slider\" spriteType = \"GFX_bar3x17\" } \
          iconType = { name = \"lo\" spriteType = \"GFX_even\" } \
          iconType = { name = \"hi\" spriteType = \"GFX_thirds\" } } \
          containerWindowType = { name = \"w\" size = { width = 100 height = 100 } \
          scrollbarType = { name = \"bar\" parent = \"base\" position = { x = 2 y = 3 } \
          size = { x = 10 y = 60 } borderSize = { x = 1 y = 1 } minValue = -4 maxValue = 4 \
          startValue = 1 stepSize = 1 priority = 1 slider = \"slider\" track = \"track\" \
          leftButton = \"down\" rightButton = \"up\" rangeLimitMinIcon = \"lo\" \
          rangeLimitMaxIcon = \"hi\" userangelimit = yes rangeLimitMin = -2 \
          rangeLimitMax = 2 } } }",
    );
    let mut fixture = fixture();
    fixture.templates.push(document);
    let window = fixture.templates[0].template("w").unwrap();
    let assets = assets_in(
        window,
        &|name| fixture.gui_type(name),
        &fixture.localisation,
    );
    for sprite in [
        "GFX_four",
        "GFX_cornered",
        "GFX_bar3x17",
        "GFX_even",
        "GFX_thirds",
        "GFX_odd",
    ] {
        assert!(assets.sprites.contains(sprite.as_bytes()), "{sprite}");
    }
    let asked = Asked {
        inner: &fixture,
        sprites: Default::default(),
        fonts: Default::default(),
    };
    layout(window, None, &asked, &Bindings::default())
        .unwrap()
        .unwrap();
    let missed: Vec<_> = asked
        .sprites
        .borrow()
        .difference(&assets.sprites)
        .map(|name| lossy(name))
        .collect();
    assert!(missed.is_empty(), "{missed:?}");
    assert!(asked.fonts.borrow().is_subset(&assets.fonts));
    assert!(!asked.sprites.borrow().is_empty());
}

#[test]
fn externally_resolved_parts_carry_their_refusals() {
    content_reader_script::test_game::enter();
    let base = |up_position: &str| {
        format!(
            "scrollbarType = {{ name = \"base\" size = {{ x = 10 y = 20 }} minValue = 0 \
             maxValue = 1 startValue = 0 stepSize = 1 slider = \"slider\" track = \"track\" \
             leftButton = \"down\" rightButton = \"up\" \
             buttonType = {{ name = \"up\" spriteType = \"GFX_four\" {up_position} }} \
             buttonType = {{ name = \"down\" spriteType = \"GFX_four\" }} \
             guiButtonType = {{ name = \"track\" spriteType = \"GFX_cornered\" }} \
             guiButtonType = {{ name = \"slider\" spriteType = \"GFX_bar3x17\" }} }}"
        )
    };
    let window = "containerWindowType = { name = \"w\" size = { width = 100 height = 100 } \
         scrollbarType = { name = \"bar\" parent = \"base\" position = { x = 2 y = 3 } \
         size = { x = 10 y = 60 } borderSize = { x = 1 y = 1 } minValue = -4 maxValue = 4 \
         startValue = 1 stepSize = 1 priority = 1 slider = \"slider\" track = \"track\" \
         leftButton = \"down\" rightButton = \"up\" } }";
    for (position, refused) in [
        ("position = { x = 1 y = 1 }", false),
        ("position = { x = bad y = 1 }", true),
    ] {
        let document = read(format!("guiTypes = {{ {} {window} }}", base(position)).as_bytes());
        let mut fixture = fixture();
        fixture.templates.push(document);
        let window = fixture.templates[0].template("w").unwrap();
        assert!(
            window.subtree_refusals().is_empty(),
            "{position}: {:?}",
            window.subtree_refusals()
        );
        let laid = layout(window, None, &fixture, &Bindings::default());
        if refused {
            let refusals = laid.unwrap_err();
            assert!(
                refusals
                    .iter()
                    .any(|refusal| refusal.kind == RefusalKind::Reader
                        && refusal.detail.contains("uninitialised")),
                "{refusals:?}"
            );
        } else {
            assert!(laid.unwrap().is_some());
        }
    }
    let document = read(
        format!(
            "guiTypes = {{ {} containerWindowType = {{ name = \"w\" \
             size = {{ width = 100 height = 100 }} listBoxType = {{ name = \"l\" \
             size = {{ x = 50 y = 20 }} scrollbarType = \"base\" }} }} }}",
            base("position = { x = bad y = 1 }")
        )
        .as_bytes(),
    );
    let mut fixture = fixture();
    fixture.templates.push(document);
    let list = fixture.templates[0].template("w").unwrap().children[0].clone();
    let refusals =
        widgets::list_box_rows(&list, &[[10, 10]; 4], None, &fixture, &Bindings::default())
            .unwrap_err();
    assert!(
        refusals
            .iter()
            .any(|refusal| refusal.kind == RefusalKind::Reader
                && refusal.detail.contains("uninitialised")),
        "{refusals:?}"
    );
}

#[test]
fn text_icons_are_acquired_and_animations_refused() {
    content_reader_script::test_game::enter();
    let mut assets = Assets::default();
    let icon = native_text::ICON;
    assets.add_text(&format!("a{icon}pp|2 b{icon}x c"));
    assert_eq!(
        assets.sprites.into_iter().collect::<Vec<_>>(),
        [b"GFX_pp".as_slice(), b"GFX_x", b"pp", b"x"]
    );
    let gui = gui("iconType = { name = \"a\" spriteType = \"GFX_even\" }");
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().animated = true;
    let error = layout(
        gui.template("w").unwrap(),
        None,
        &resources,
        &Bindings::default(),
    )
    .unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::MissingResource);
}

#[test]
fn a_grid_background_draws_under_its_items() {
    content_reader_script::test_game::enter();
    let layout = laid(
        "size = { width = 100 height = 100 } \
         gridBoxType = { name = \"g\" position = { x = 5 y = 6 } orientation = center \
         background = { spriteType = \"GFX_even\" position = { x = 1 y = 1 } } }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(rect(&layout.parts[0].kind), [56.0, 57.0, 20.0, 10.0]);
    assert_eq!(layout.parts[0].child, Some(0));
    assert_eq!(layout.holders[0].child, Some(0));
    assert_eq!(layout.holders[0].before, 1);
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_gui_text_preserves_bgra_border_and_vertex_constants() {
    content_reader_script::test_game::enter();
    use shader_pipeline::{self as shader, library};
    use std::sync::Arc;
    let source = br#"
VertexStruct In {
float3 at : POSITION;
float2 uv : TEXCOORD0;
float4 color : COLOR0;
float4 border : COLOR1;
};
VertexStruct Out {
float4 at : PDX_POSITION;
float4 color : TEXCOORD0;
};
VertexShader = {
ConstantBuffer(0,0) {
float4x4 renamed;
float4 multiplier;
};
MainCode V [[ Out main(In v) {
Out o; o.at=mul(renamed,float4(v.at,1)); o.color=v.border*multiplier*(v.at.z==0); return o;
} ]] }
PixelShader = { MainCode P [[ float4 main(Out v) : PDX_COLOR { return v.color; } ]] }
Effect Text { VertexShader = V PixelShader = P }
"#;
    let program =
        content_reader_gui::gui::native::shader::program::read_with(b"custom-text.shader", &|path| {
            Some(if path == b"custom-text.shader" {
                source.to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
    let compiled = shader::Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    let asset =
        library::Effect::from_compiled(Arc::new(program), Vec::new(), vec![Ok(Arc::new(compiled))]);
    let part = Part {
        path: "mod.text".into(),
        child: None,
        background_index: None,
        kind: PartKind::Text {
            font: "Main_14".into(),
            glyphs: vec![DrawnGlyph {
                ch: 'A',
                at: [2., 1.],
                color: [1.; 4],
                texture: 3,
                size: [7, 14],
                uv: [0., 0., 1., 1.],
            }],
            scale: 1.,
            modulate: [1.; 4],
            border_color: 0xff66_9933,
            projection: [1.; 2],
            font_viewport: None,
        },
        clip: None,
        color: [1.; 4],
        native: None,
        shading: shading(
            &Effect {
                file: "custom-text.shader".into(),
                compiled: Some(asset),
                techniques: Vec::new(),
                samplers: [None; 6],
            },
            "Text",
            0.,
        ),
    };
    let mut batch = Batch::default();
    crate::native_shader::draw::emit(&part, [0, 0], &one(), &mut batch).unwrap();
    let (_serial, device, queue) = gpu::device();
    let (pixels, refused, _) = crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert!(refused.is_empty(), "{refused:?}");
    for y in 0..16 {
        for x in 0..16 {
            let expected = if (2..9).contains(&x) && (1..15).contains(&y) {
                [102, 153, 51, 255]
            } else {
                [0; 4]
            };
            assert_eq!(
                &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4],
                &expected,
                "{x},{y}"
            );
        }
    }
    println!(
        "PASS compiled text: 256/256 pixels; native z0, BGRA COLOR1, renamed vertex c0 and native matrix"
    );
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_gui_samples_sixteen_bit_texels_at_full_precision() {
    content_reader_script::test_game::enter();
    use shader_pipeline::{self as shader, library};
    use content_reader_image::dds;
    use std::sync::Arc;
    let source = br#"
VertexStruct In {
float3 at : POSITION;
float2 uv : TEXCOORD0;
};
VertexStruct Out {
float4 at : PDX_POSITION;
float2 uv : TEXCOORD0;
};
ConstantBuffer(0, 0) {
float4x4 world;
};
VertexShader = { MainCode V [[ Out main(In v) { Out o; o.at=mul(world,float4(v.at,1)); o.uv=v.uv; return o; } ]] }
PixelShader = {
Samplers = { Page = { Index = 0 MagFilter = Linear MinFilter = Linear MipFilter = None AddressU = Clamp AddressV = Clamp } }
MainCode P [[ float4 main(Out v) : PDX_COLOR {
float4 c = Page_Texture.Sample(Page_Sampler, v.uv);
return float4(c.r > 0.0322575, c.g > 0.01587, c.b > 0.0322575, 1);
} ]]
}
Effect Up { VertexShader = V PixelShader = P }
"#;
    let program =
        content_reader_gui::gui::native::shader::program::read_with(b"sixteen.shader", &|path| {
            Some(if path == b"sixteen.shader" {
                source.to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
    let compiled = shader::Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    let asset =
        library::Effect::from_compiled(Arc::new(program), Vec::new(), vec![Ok(Arc::new(compiled))]);
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(Effect {
        file: "sixteen.shader".into(),
        techniques: Vec::new(),
        samplers: [None; 6],
        compiled: Some(asset),
    });
    let tree = gui("iconType = { name = \"a\" spriteType = \"GFX_even\" }");
    let window = tree.template("w").unwrap();
    let layout = layout(window, None, &resources, &Bindings::default())
        .unwrap()
        .unwrap();
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    let (_serial, device, queue) = gpu::device();
    let sixteen = dds::Texture {
        format: dds::TextureFormat::B5G6R5,
        width: 2,
        height: 1,
        faces: 1,
        mips: vec![vec![dds::MipLevel {
            width: 2,
            height: 1,
            data: ([0x0821_u16; 2].map(u16::to_le_bytes).concat()).into(),
        }]],
    };
    let filler = dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![0; 4],
    });
    let mut pages = vec![filler; 4];
    pages[3] = sixteen;
    let (pixels, refused, _) =
        crate::sprites::oracle::render_images(&device, &queue, &batch, &pages);
    assert!(refused.is_empty(), "{refused:?}");
    for y in 0..10 {
        for x in 0..16 {
            assert_eq!(
                &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4],
                &[255; 4],
                "{x},{y}"
            );
        }
    }
    println!("PASS 16-bit page: 160/160 pixels read c/31 and c/63 at f32 through a linear sampler");
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_gui_past_the_dynamic_offset_limit_is_refused_and_counted() {
    content_reader_script::test_game::enter();
    use shader_pipeline::{self as shader, library};
    use std::sync::Arc;
    let _serial = serial();
    let instance = shader_pipeline::gpu::instance();
    let adapter = pollster::block_on(
        instance.request_adapter(&eframe::wgpu::RequestAdapterOptions::default()),
    )
    .unwrap();
    let mut descriptor = device_descriptor(&adapter);
    let limit = eframe::wgpu::Limits::default().max_dynamic_uniform_buffers_per_pipeline_layout;
    descriptor
        .required_limits
        .max_dynamic_uniform_buffers_per_pipeline_layout = limit;
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    let blocks = limit + 1;
    assert!(blocks <= 14, "{blocks}");
    let declarations: String = (1..blocks)
        .map(|slot| format!("ConstantBuffer({slot},0) {{ float4 Value{slot};\n}};\n"))
        .collect();
    let sum: String = (1..blocks).map(|slot| format!(" + Value{slot}")).collect();
    let source = format!(
        r#"
VertexStruct In {{
float3 at : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct Out {{
float4 at : PDX_POSITION;
}};
ConstantBuffer(0, 0) {{
float4x4 world;
float4 tint;
}};
VertexShader = {{ MainCode V [[ Out main(In v) {{ Out o; o.at=mul(world,float4(v.at,1)); return o; }} ]] }}
PixelShader = {{
{declarations}
MainCode P [[ float4 main(Out v) : PDX_COLOR {{ return tint{sum}; }} ]]
}}
Effect Up {{ VertexShader = V PixelShader = P }}
"#
    );
    let program =
        content_reader_gui::gui::native::shader::program::read_with(b"blocks.shader", &|path| {
            Some(if path == b"blocks.shader" {
                source.as_bytes().to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
    let compiled = shader::Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    assert_eq!(
        compiled.vertex.buffers.len() + compiled.pixel.buffers.len(),
        blocks as usize + 1
    );
    let asset =
        library::Effect::from_compiled(Arc::new(program), Vec::new(), vec![Ok(Arc::new(compiled))]);
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(Effect {
        file: "blocks.shader".into(),
        techniques: Vec::new(),
        samplers: [None; 6],
        compiled: Some(asset),
    });
    let tree = gui("iconType = { name = \"a\" spriteType = \"GFX_even\" }");
    let window = tree.template("w").unwrap();
    let layout = layout(window, None, &resources, &Bindings::default())
        .unwrap()
        .unwrap();
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    let (pixels, refused, _) = crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert_eq!(refused.values().sum::<u64>(), 1, "{refused:?}");
    let expected = format!("exceed this device's {limit} dynamically offset buffers");
    assert!(
        refused.keys().all(|reason| reason.contains(&expected)),
        "{refused:?}"
    );
    assert!(
        pixels.iter().all(|byte| *byte == 0),
        "the refused draw draws nothing"
    );
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_gui_reads_past_the_written_sprite_constants_are_refused() {
    content_reader_script::test_game::enter();
    use shader_pipeline::{self as shader, library};
    use std::sync::Arc;
    let effect = |pixel: &str| {
        let source = format!(
            r#"
VertexStruct In {{
float3 at : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct Out {{
float4 at : PDX_POSITION;
}};
ConstantBuffer(0, 0) {{
float4x4 world;
float4 color;
float2 offset;
float2 next;
float time;
float animation;
float2 tail;
float4 slot;
}};
VertexShader = {{ MainCode V [[ Out main(In v) {{ Out o; o.at=mul(world,float4(v.at,1)); return o; }} ]] }}
PixelShader = {{ MainCode P [[ float4 main(Out v) : PDX_COLOR {{ return {pixel}; }} ]] }}
Effect Up {{ VertexShader = V PixelShader = P }}
"#
        );
        let program =
            content_reader_gui::gui::native::shader::program::read_with(b"tail.shader", &|path| {
                Some(if path == b"tail.shader" {
                    source.as_bytes().to_vec()
                } else {
                    b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
                })
            })
            .unwrap();
        let compiled = shader::Compiler::discover()
            .unwrap()
            .compile(&program, &program.techniques[0], &[])
            .unwrap();
        Effect {
            file: "tail.shader".into(),
            techniques: Vec::new(),
            samplers: [None; 6],
            compiled: Some(library::Effect::from_compiled(
                Arc::new(program),
                Vec::new(),
                vec![Ok(Arc::new(compiled))],
            )),
        }
    };
    let (_serial, device, queue) = gpu::device();
    let run = |pixel: &str| {
        let mut resources = fixture();
        resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(effect(pixel));
        let tree = gui("iconType = { name = \"a\" spriteType = \"GFX_even\" }");
        let window = tree.template("w").unwrap();
        let layout = layout(window, None, &resources, &Bindings::default())
            .unwrap()
            .unwrap();
        let mut batch = Batch::default();
        draw(&layout, [0, 0], &one(), &mut batch).unwrap();
        let native = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .clone()
            .unwrap();
        assert_eq!((native.constants.len() * 4, native.known), (0x100, 104));
        crate::sprites::oracle::render_batch(&device, &queue, &batch)
    };
    for (pixel, bytes) in [
        ("float4(tail.x, 0, 0, 1)", "104..108"),
        ("float4(tail.y, 0, 0, 1)", "108..112"),
        ("float4(slot.x, 0, 0, 1)", "112..116"),
    ] {
        let (pixels, refused, _) = run(pixel);
        assert_eq!(refused.values().sum::<u64>(), 1, "{refused:?}");
        assert!(
            refused.keys().all(|reason| reason.contains(&format!(
                "native pixel constant buffer b0 reads unavailable bytes {bytes} (physical size 256)"
            ))),
            "{refused:?}"
        );
        assert!(
            pixels.iter().all(|byte| *byte == 0),
            "the refused draw draws nothing"
        );
    }
    let (pixels, refused, _) = run("float4(1, 0, 0, 1) + animation * 0 + time * 0");
    assert!(refused.is_empty(), "{refused:?}");
    assert!(
        pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [255, 0, 0, 255])
    );
    println!("PASS sprite b0: reads of bytes 104, 108 and 112 refused by name, 0..104 drawn");
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().effect =
        Ok(effect("float4(1, 0, 0, 1) + animation * 0 + time * 0"));
    resources.text = Some(Effect {
        file: "tail.shader".into(),
        techniques: Vec::new(),
        samplers: [None; 6],
        compiled: Some(compiled_effect(
            "text_tail.shader",
            r#"
VertexStruct In {
float3 at : POSITION;
float2 uv : TEXCOORD0;
float4 color : COLOR0;
float4 border : COLOR1;
};
VertexStruct Out {
float4 at : PDX_POSITION;
};
VertexShader = {
ConstantBuffer(0, 0) {
float4x4 world;
float4 modulate;
};
MainCode V [[ Out main(In v) { Out o; o.at=mul(world,float4(v.at,1)); return o; } ]]
}
PixelShader = {
ConstantBuffer(0, 0) {
float4x4 m;
float4 c;
float4 offsets;
float4 clocks;
float4 slot;
};
MainCode P [[ float4 main(Out v) : PDX_COLOR { return float4(slot.x, 0, 0, 1); } ]]
}
Effect Text { VertexShader = V PixelShader = P }
"#,
        )),
    });
    let tree = gui("iconType = { name = \"a\" spriteType = \"GFX_even\" } \
         instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"A\" }");
    let window = tree.template("w").unwrap();
    let layout = layout(window, None, &resources, &Bindings::default())
        .unwrap()
        .unwrap();
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    let (_, refused, _) = crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert_eq!(refused.values().sum::<u64>(), 1, "{refused:?}");
    assert!(
        refused.keys().all(|reason| reason.contains(
            "native pixel constant buffer b0 reads unavailable bytes 112..116 (physical size 256)"
        )),
        "{refused:?}"
    );
    println!("PASS retained b0: a text's pixel read of the sprite's pooled slot refused");
}

fn compiled_effect(file: &str, source: &str) -> Arc<shader_pipeline::library::Effect> {
    use shader_pipeline::{self as shader, library};
    use std::sync::Arc;
    let program =
        content_reader_gui::gui::native::shader::program::read_with(file.as_bytes(), &|path| {
            Some(if path == file.as_bytes() {
                source.as_bytes().to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
    library::Effect::new(
        Arc::new(program),
        Vec::new(),
        Arc::new(shader::cache::Cache::default()),
    )
}

fn compiled_part(
    kind: PartKind,
    effect: Arc<shader_pipeline::library::Effect>,
    technique: &str,
) -> Part {
    Part {
        path: "w.a".into(),
        child: None,
        background_index: None,
        kind,
        clip: None,
        color: [1.; 4],
        native: None,
        shading: Shading {
            compiled: Some(crate::native_shader::draw::Effect {
                asset: effect,
                technique: technique.into(),
            }),
            fixture: None,
            since: 0.,
            animation: 0.,
            time: 0.,
            clocked: true,
        },
    }
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_sprite_draws_four_records_as_a_strip() {
    content_reader_script::test_game::enter();
    let effect = compiled_effect(
        "vertex_id.shader",
        r#"
VertexStruct In {
float3 at : POSITION;
float2 uv : TEXCOORD0;
};
VertexStruct Out {
float4 at : PDX_POSITION;
float4 color : TEXCOORD0;
};
ConstantBuffer(0, 0) {
float4x4 world;
};
VertexShader = { MainCode V [[ Out main(In v, uint id : SV_VertexID) { Out o; o.at=mul(world,float4(v.at,1)); o.color=float4(id > 3 ? 1 : 0, id == 3 ? 1 : 0, 0, 1); return o; } ]] }
PixelShader = { MainCode P [[ float4 main(Out v) : PDX_COLOR { return v.color; } ]] }
Effect Up { VertexShader = V PixelShader = P }
"#,
    );
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(Effect {
        file: "vertex_id.shader".into(),
        techniques: Vec::new(),
        samplers: [None; 6],
        compiled: Some(effect),
    });
    let tree = gui("iconType = { name = \"a\" spriteType = \"GFX_even\" }");
    let window = tree.template("w").unwrap();
    let layout = layout(window, None, &resources, &Bindings::default())
        .unwrap()
        .unwrap();
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    let native = batch.quads[0]
        .exact
        .as_ref()
        .unwrap()
        .native
        .clone()
        .unwrap();
    assert_eq!(native.vertices.len() / native.stride, 4);
    assert_eq!(
        native.topology,
        eframe::wgpu::PrimitiveTopology::TriangleStrip
    );
    let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
    let (pixels, refused, _) = crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert!(refused.is_empty(), "{refused:?}");
    let drawn: Vec<&[u8]> = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[3] == 255)
        .collect();
    assert!(!drawn.is_empty());
    assert!(drawn.iter().all(|pixel| pixel[0] == 0), "no id past 3");
    assert!(drawn.iter().any(|pixel| pixel[1] > 0), "id 3 drawn");
    println!(
        "PASS sprite strip: SV_VertexID 0..3 over {} pixels",
        drawn.len()
    );
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_flag_reads_its_zeroed_constant_tail() {
    let effect = compiled_effect(
        "flag.shader",
        r#"
VertexStruct In {
float3 at : POSITION;
float2 uv : TEXCOORD0;
};
VertexStruct Out {
float4 at : PDX_POSITION;
float4 color : TEXCOORD0;
};
ConstantBuffer(0, 0) {
float4x4 world;
float4 coords;
float4 tail;
};
VertexShader = { MainCode V [[ Out main(In v) { Out o; o.at=mul(world,float4(v.at,1)); o.color=float4(1,0,0,1)+tail; return o; } ]] }
PixelShader = { MainCode P [[ float4 main(Out v) : PDX_COLOR { return v.color; } ]] }
Effect Up { VertexShader = V PixelShader = P }
"#,
    );
    let part = compiled_part(
        PartKind::MaskedFlag {
            sprite: b"GFX_even".to_vec(),
            flag: 3,
            flag_coords: [0., 0., 1., 1.],
            rect: [0., 0., 16., 16.],
            native_origin: [0., 16.],
        },
        effect,
        "Up",
    );
    let mut batch = Batch::default();
    crate::native_shader::draw::emit(&part, [0, 0], &one(), &mut batch).unwrap();
    let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
    let (pixels, refused, _) = crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert!(refused.is_empty(), "{refused:?}");
    assert!(
        pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [255, 0, 0, 255]),
        "the zeroed tail adds nothing"
    );
    println!("PASS masked flag: b0 bytes 80..96 read as the initialized zeros");
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_animation_reads_past_the_written_records_are_refused() {
    let effect = |pixel: &str| {
        compiled_effect(
            "records.shader",
            &format!(
                r#"
VertexStruct In {{
float3 at : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct Out {{
float4 at : PDX_POSITION;
}};
ConstantBuffer(0, 0) {{
float4x4 world;
}};
ConstantBuffer(1, 0) {{
float4 A0;
float4 A1;
float4 A2;
float4 A3;
float4 A4;
float4 B0;
float4 B1;
float4 B2;
float4 B3;
float4 B4;
float4 C0;
}};
VertexShader = {{ MainCode V [[ Out main(In v) {{ Out o; o.at=mul(world,float4(v.at,1)); return o; }} ]] }}
PixelShader = {{
Samplers = {{ T = {{ Index = 0 MagFilter = Point MinFilter = Point MipFilter = None AddressU = Clamp AddressV = Clamp }} }}
MainCode P [[ float4 main(Out v) : PDX_COLOR {{ return {pixel}; }} ]]
}}
Effect Up {{ VertexShader = V PixelShader = P }}
"#
            ),
        )
    };
    let geometry = Geometry {
        rect: [0., 0., 16., 16.],
        local: [0., 0., 16., 16.],
        pivot: [8., 8.],
        rotation: 0.,
    };
    let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
    let run = |pixel: &str| {
        let part = compiled_part(
            PartKind::Animated(Box::new(animation::Animated {
                sprite: b"GFX_even".to_vec(),
                geometry,
                uv: [0., 0., 1., 1.],
                offsets: [0.; 4],
                effect: "records.shader".into(),
                technique: "Up",
                records: vec![animation::AnimationDraw {
                    mask: 3,
                    texture: 3,
                    constants: [[1.; 4]; 5],
                    typed: true,
                }],
                masked: false,
            })),
            effect(pixel),
            "Up",
        );
        let mut batch = Batch::default();
        crate::native_shader::draw::emit(&part, [0, 0], &one(), &mut batch).unwrap();
        crate::sprites::oracle::render_batch(&device, &queue, &batch)
    };
    for (pixel, bytes) in [
        ("float4(A4.y, 0, 0, 1)", "68..72"),
        ("float4(A4.w, 0, 0, 1)", "76..80"),
        ("float4(B3.x, 0, 0, 1)", "128..132"),
        ("float4(C0.x, 0, 0, 1)", "160..164"),
    ] {
        let (pixels, refused, _) = run(pixel);
        assert_eq!(refused.values().sum::<u64>(), 1, "{pixel}: {refused:?}");
        assert!(
            refused.keys().all(|reason| reason.contains(&format!(
                "native pixel constant buffer b1 reads unavailable bytes {bytes} (physical size 256)"
            ))),
            "{pixel}: {refused:?}"
        );
        assert!(
            pixels.iter().all(|byte| *byte == 0),
            "{pixel}: draws nothing"
        );
    }
    let (pixels, refused, _) = run("float4(A4.x, B0.x + B2.w, 0, 1)");
    assert!(refused.is_empty(), "{refused:?}");
    assert!(
        pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [255, 0, 0, 255])
    );
    println!(
        "PASS animation b1: reads of 68..80 and 128..160 refused by name, written bytes drawn"
    );
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_gui_runs_local_vertices_clocks_and_sorted_null_techniques() {
    content_reader_script::test_game::enter();
    use shader_pipeline::{self as shader, library};
    use std::sync::Arc;
    let source = br#"
VertexStruct In {
float3 at : POSITION;
float2 uv : TEXCOORD0;
};
VertexStruct Out {
float4 at : PDX_POSITION;
float3 local : TEXCOORD0;
};
ConstantBuffer(0, 0) {
float4x4 arbitraryMatrixName;
float4 arbitraryColorName;
float2 frame;
float2 nextFrame;
float clock;
float changed;
};
VertexShader = { MainCode V [[ Out main(In v) {
Out o; o.at=mul(arbitraryMatrixName,float4(v.at,1)); o.local=v.at; return o;
} ]] }
PixelShader = { MainCode P [[ float4 main(Out v) : PDX_COLOR {
return float4(v.local.x < 4, v.local.y > 4,
v.local.z == 1 && clock == 10 && changed == 7 && frame.x > 0.3 && nextFrame.x > 0.6, 1) * arbitraryColorName;
} ]] }
Effect Up { VertexShader = V PixelShader = P }
"#;
    let program =
        content_reader_gui::gui::native::shader::program::read_with(b"mod-owned.shader", &|path| {
            Some(if path == b"mod-owned.shader" {
                source.to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
    let compiler = shader::Compiler::discover().unwrap();
    let compiled = compiler
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    let asset =
        library::Effect::from_compiled(Arc::new(program), Vec::new(), vec![Ok(Arc::new(compiled))]);
    let mut resources = fixture();
    let info = resources.sprites.get_mut("GFX_even").unwrap();
    info.texture = [24, 8];
    info.frames = 3;
    info.effect = Ok(Effect {
        file: "mod-owned.shader".into(),
        techniques: Vec::new(),
        samplers: [None; 6],
        compiled: Some(asset.clone()),
    });
    let tree = gui("scale = 2 iconType = { name = \"a\" spriteType = \"GFX_even\" frame = 2 }");
    let window = tree.template("w").unwrap();
    let mut bindings = Bindings {
        time: 10.,
        ..Default::default()
    };
    bindings
        .bind(window.find(Kind::Icon, "a").unwrap())
        .effect_since = Some(3.);
    let layout = layout(window, None, &resources, &bindings)
        .unwrap()
        .unwrap();
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    assert_eq!(batch.quads.len(), 1);
    let mut missing = batch.quads[0].clone();
    let payload = Arc::make_mut(missing.exact.as_mut().unwrap().native.as_mut().unwrap());
    payload.effect.technique = "Down".into();
    payload.constants[17] = 0.;
    missing.layer = 2;
    batch.quads[0].layer = 1;
    batch.quads.insert(0, missing);
    let (_serial, device, queue) = gpu::device();
    let (pixels, refused, pipelines) =
        crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(pipelines, 1);
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(
                &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4],
                &[if x < 8 { 255 } else { 0 }, 0, 255, 255],
                "{x},{y}"
            );
        }
    }
    println!(
        "PASS compiled GUI: 256/256 pixels; local positions, parent scale, frame UVs, absolute clocks, sorted null fallback and pipeline reuse"
    );
    let mut absent = batch.quads[0].clone();
    let payload = Arc::make_mut(absent.exact.as_mut().unwrap().native.as_mut().unwrap());
    let empty = content_reader_gui::pdxshader::program::read_with::<Vec<u8>>(b"absent.lua", &|_| None).unwrap();
    payload.effect.asset = library::Effect::from_compiled(Arc::new(empty), vec![], vec![]);
    payload.effect.technique = "Up".into();
    absent.layer = 3;
    batch.quads.push(absent);
    let (missing_pixels, refused, pipelines) =
        crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(pipelines, 1);
    assert_eq!(
        missing_pixels, pixels,
        "a missing effect file retains the same sorted native program"
    );
    println!("PASS absent effect file: 256/256 pixels, retained program without a compiler/header");
    let slot = resources
        .sprites
        .get_mut("GFX_even")
        .unwrap()
        .effect
        .as_mut()
        .unwrap()
        .compiled
        .as_mut()
        .unwrap();
    *slot = library::Effect::from_compiled(
        slot.source.clone(),
        slot.defines.clone(),
        vec![Err(
            "effect mod-owned.shader technique Up: deliberate compiler diagnostic".into(),
        )],
    );
    let layout = super::layout(window, None, &resources, &bindings)
        .unwrap()
        .unwrap();
    let mut failing = Batch::default();
    draw(&layout, [0, 0], &one(), &mut failing).unwrap();
    let mut failing = failing.quads.remove(0);
    failing.layer = 4;
    let mut valid = batch.quads.iter().find(|q| q.layer == 1).unwrap().clone();
    valid.layer = 5;
    batch.quads.push(valid);
    let (without, refused, _) = crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert!(refused.is_empty(), "{refused:?}");
    batch.quads.push(failing.clone());
    let (pixels, refused, _) = crate::sprites::oracle::render_batch(&device, &queue, &batch);
    assert_eq!(refused.values().sum::<u64>(), 1, "{refused:?}");
    assert!(
        refused
            .keys()
            .all(|reason| reason.contains("deliberate compiler diagnostic"))
    );
    assert_eq!(pixels, without, "the refused draw changes nothing else");
    let up = batch.quads.iter().find(|q| q.layer == 1).unwrap().clone();
    let kept = batch.quads.iter().find(|q| q.layer == 2).unwrap().clone();
    let sequence = |middle: Option<crate::sprites::Quad>| {
        let mut sequence = Batch::default();
        let mut c = kept.clone();
        c.layer = 12;
        sequence.quads.push(c);
        if let Some(mut b) = middle {
            b.layer = 11;
            sequence.quads.push(b);
        }
        let mut a = up.clone();
        a.layer = 10;
        sequence.quads.push(a);
        crate::sprites::oracle::render_batch(&device, &queue, &sequence)
    };
    let (kept_pixels, refused, _) = sequence(None);
    assert!(refused.is_empty(), "{refused:?}");
    let mut alone = Batch::default();
    alone.quads.push(up.clone());
    let (up_pixels, refused, _) =
        crate::sprites::oracle::render_batch(&device, &queue, &alone);
    assert!(refused.is_empty(), "{refused:?}");
    assert_ne!(kept_pixels, up_pixels, "`c` draws over `a`");
    let not_resident = |quad: &crate::sprites::Quad| {
        let mut quad = quad.clone();
        let payload = Arc::make_mut(quad.exact.as_mut().unwrap().native.as_mut().unwrap());
        payload.textures = vec![(0, Some(1000))];
        quad
    };
    let (pixels, refused, _) = sequence(Some(not_resident(&up)));
    assert_eq!(refused.values().sum::<u64>(), 1, "{refused:?}");
    assert!(refused.keys().all(|reason| reason.contains("not resident")));
    assert_eq!(pixels, kept_pixels, "`c` runs the program `b` bound");
    let (pixels, refused, _) = sequence(Some(not_resident(&kept)));
    assert_eq!(refused.values().sum::<u64>(), 1, "{refused:?}");
    assert_eq!(pixels, kept_pixels, "`c` runs the program `a` bound");
    let (pixels, refused, _) = sequence(Some(failing.clone()));
    assert_eq!(refused.values().sum::<u64>(), 2, "{refused:?}");
    assert!(
        refused
            .keys()
            .any(|reason| reason.contains("deliberate compiler diagnostic"))
    );
    assert!(
        refused
            .keys()
            .any(|reason| reason.contains("needs the preceding native program"))
    );
    assert_eq!(pixels, up_pixels, "only `a` draws");
}

#[test]
fn the_shipped_naval_combat_window_lays_out() {
    content_reader_script::test_game::enter();
    let gui = read(
        br#"guiTypes = {
	containerWindowType = {
		name = "naval_combat_mapicon"
		position = { x=0 y=0 }
		size = { width=33 height=43 }
		orientation = upper_left
		clipping = no
		buttonType ={
			name ="bg_btn"
			quadTextureSprite = "GFX_odd"
			Orientation = center
			centerPosition = yes
			position = { x=1 y=-30 }
		}
		iconType = {
			name = "progress"
			spriteType = "GFX_bar"
			position = { x=0 y=0 }
			Orientation = center
			centerPosition = yes
		}
		gridBoxType = {
			name = "attacker_grid"
			position = { x = -60 y = 30 }
			size = { width = 100% height = 100%% }
			slotsize = { width = 36 height = 36 }
			max_slots_horizontal = 1
			add_horizontal = no
			format = "UPPER_LEFT"
		}
	}
}
"#,
    );
    let window = gui.template("naval_combat_mapicon").unwrap();
    let layout = layout(window, None, &fixture(), &Bindings::default())
        .unwrap()
        .unwrap();
    assert_eq!(layout.size, [33, 43]);
    assert_eq!(rect(&layout.parts[0].kind), [10.0, -13.0, 15.0, 9.0]);
    assert_eq!(rect(&layout.parts[1].kind), [-61.0, 15.0, 154.0, 14.0]);
    let grid = window.find(Kind::Grid, "attacker_grid").unwrap();
    assert_eq!(
        grid_slots(grid, layout.size, 2, None),
        Ok(vec![[-60, 30], [-60, 66]])
    );
}

fn laid_document(
    document: &str,
    viewport: Option<[i32; 2]>,
    bind: impl Fn(&Element, &mut Bindings),
) -> Result<Option<Layout>, Refusals> {
    let gui = read(document.as_bytes());
    let window = gui.template("w").unwrap().clone();
    let mut resources = fixture();
    resources.templates.push(gui);
    let mut bindings = Bindings {
        viewport,
        regions: true,
        ..Bindings::default()
    };
    bind(&window, &mut bindings);
    layout(&window, None, &resources, &bindings)
}

const SCROLLBAR: &str = "extendedScrollbarType = { name = \"sb\" \
     size = { width = 10 height = 50 } \
     background = { name = \"bg\" spriteType = \"GFX_even\" } \
     track = { name = \"track\" spriteType = \"GFX_bar3x17\" } \
     slider = { name = \"slider\" spriteType = \"GFX_bar3x17\" } \
     decreaseButton = { name = \"dec\" spriteType = \"GFX_four\" } \
     increaseButton = { name = \"inc\" spriteType = \"GFX_four\" position = { x = -10 y = -10 } } }";

fn scrollbar_rects(layout: &Layout) -> Vec<(String, Rect)> {
    layout
        .parts
        .iter()
        .filter(|part| part.path.contains("sb."))
        .map(|part| (part.path.clone(), rect(&part.kind)))
        .collect()
}

#[test]
fn a_container_shows_its_extended_scrollbar() {
    content_reader_script::test_game::enter();
    let document = format!(
        "guiTypes = {{ {SCROLLBAR} containerWindowType = {{ name = \"w\" \
         size = {{ width = 100 height = 80 }} margin = {{ top = 5 bottom = 5 }} \
         verticalScrollbar = \"sb\" autohide_scrollbars = no \
         iconType = {{ name = \"i\" spriteType = \"GFX_even\" }} }} }}"
    );
    let layout = laid_document(&document, None, |_, _| {}).unwrap().unwrap();
    assert_eq!(layout.parts[0].path, "w.i");
    let rects = scrollbar_rects(&layout);
    let names: Vec<&str> = rects.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        names,
        ["sb.bg", "sb.track", "sb.inc", "sb.dec", "sb.slider"]
    );
    assert_eq!(rects[0].1, [0.0, 5.0, 20.0, 10.0]);
    assert_eq!(rects[1].1, [0.0, 15.0, 3.0, 17.0]);
    assert_eq!(rects[2].1, [0.0, 65.0, 10.0, 10.0]);
    assert_eq!(rects[3].1, [0.0, 5.0, 10.0, 10.0]);
    assert_eq!(rects[4].1, [0.0, 15.0, 3.0, 17.0]);
    let hidden = document.replace("autohide_scrollbars = no", "");
    let layout = laid_document(&hidden, None, |_, _| {}).unwrap().unwrap();
    assert!(scrollbar_rects(&layout).is_empty());
}

#[test]
fn overflowing_children_show_and_move_the_scrollbar() {
    content_reader_script::test_game::enter();
    let document = |icon: &str| {
        format!(
            "guiTypes = {{ {SCROLLBAR} containerWindowType = {{ name = \"w\" \
             size = {{ width = 100 height = 80 }} verticalScrollbar = \"sb\" \
             background = {{ spriteType = \"GFX_even\" }} \
             iconType = {{ name = \"i\" spriteType = \"GFX_even\" {icon} }} }} }}"
        )
    };
    let rects = |icon: &str| {
        scrollbar_rects(
            &laid_document(&document(icon), None, |_, _| {})
                .unwrap()
                .unwrap(),
        )
    };
    assert!(rects("position = { x = 0 y = 10 }").is_empty());
    let below = rects("position = { x = 0 y = 75 }");
    assert_eq!(below.len(), 5);
    assert_eq!(below[4].1, [0.0, 10.0, 3.0, 17.0]);
    let above = rects("position = { x = 0 y = -20 }");
    assert_eq!(above[4].1, [0.0, 10.0 + 43.0, 3.0, 17.0]);
}

#[test]
fn scrollbar_children_draw_as_constructed() {
    content_reader_script::test_game::enter();
    let child = SCROLLBAR.replace(
        "size = { width = 10 height = 50 }",
        "size = { width = 10 height = 50 } position = { x = 5 y = 6 } \
         maxValue = 2 startValue = 1",
    );
    let layout = laid(
        &format!("size = {{ width = 100 height = 100 }} {child}"),
        None,
    )
    .unwrap()
    .unwrap();
    let rects = scrollbar_rects(&layout);
    assert_eq!(rects.len(), 5);
    assert_eq!(rects[1].1, [5.0, 16.0, 3.0, 17.0]);
    assert_eq!(rects[4].1, [5.0, 16.0 + 15.0, 3.0, 17.0]);
    let plain = |min: &str| {
        laid(
            &format!(
                "size = {{ width = 100 height = 100 }} scrollbarType = {{ name = \"bar\" \
                 position = {{ x = 2 y = 3 }} size = {{ x = 10 y = 60 }} maxValue = 4 \
                 startValue = 1 {min} slider = \"slider\" track = \"track\" \
                 leftButton = \"down\" rightButton = \"up\" \
                 buttonType = {{ name = \"up\" spriteType = \"GFX_four\" }} \
                 buttonType = {{ name = \"down\" spriteType = \"GFX_four\" }} \
                 guiButtonType = {{ name = \"track\" spriteType = \"GFX_bar3x17\" }} \
                 guiButtonType = {{ name = \"slider\" spriteType = \"GFX_bar3x17\" }} }}"
            ),
            None,
        )
    };
    let error = plain("").unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::UndefinedRead);
    let layout = plain("minValue = 0").unwrap().unwrap();
    let rects: Vec<Rect> = layout.parts.iter().map(|part| rect(&part.kind)).collect();
    assert_eq!(rects[0], [2.0, 3.0, 10.0, 10.0]);
    assert_eq!(rects[1], [2.0, 53.0, 10.0, 10.0]);
    assert_eq!(rects[2], [2.0, 13.0, 3.0, 17.0]);
    assert_eq!(rects[3], [2.0, 18.0, 3.0, 17.0]);
}

const PLAIN: &str = "scrollbarType = { name = \"lsb\" size = { x = 10 y = 60 } minValue = 0 \
     maxValue = 1 slider = \"s\" track = \"t\" leftButton = \"d\" rightButton = \"u\" \
     buttonType = { name = \"u\" spriteType = \"GFX_four\" } \
     buttonType = { name = \"d\" spriteType = \"GFX_four\" } \
     guiButtonType = { name = \"t\" spriteType = \"GFX_bar3x17\" } \
     guiButtonType = { name = \"s\" spriteType = \"GFX_bar3x17\" } }";

fn rects_of(parts: &[Part]) -> Vec<Rect> {
    parts.iter().map(|part| rect(&part.kind)).collect()
}

#[test]
fn list_boxes_hold_their_rows() {
    content_reader_script::test_game::enter();
    let body = "listBoxType = { name = \"l\" position = { x = 4 y = 6 } \
                size = { x = 30 y = 25 } borderSize = { x = 2 y = 9 } \
                background = \"GFX_cornered\" orientation = \"upper_right\" }";
    assert_eq!(laid(body, None).unwrap_err()[0].kind, RefusalKind::NoParent);
    let layout = laid_document(
        &format!("guiTypes = {{ containerWindowType = {{ name = \"w\" {body} }} }}"),
        Some([200, 100]),
        |window, bindings| bindings.bind(window).opacity = Some(0.5),
    )
    .unwrap()
    .unwrap();
    let PartKind::Tiles { quads, .. } = &layout.parts[0].kind else {
        panic!("not a cornered tile");
    };
    let right = quads
        .iter()
        .map(|(rect, _)| rect[0] + rect[2])
        .fold(0.0, f32::max);
    assert_eq!((quads[0].0[0], right), (204.0, 234.0));
    assert_eq!(layout.parts[0].color[3], 1.0);
    assert_eq!(layout.holders[0].path, "w.l");
    assert_eq!(layout.holders[0].item_opacity(), None);
    let gui = gui(body);
    let list = gui.template("w").unwrap().child_named("l").unwrap();
    let resources = fixture();
    let bindings = Bindings {
        viewport: Some([200, 100]),
        ..Bindings::default()
    };
    let rows = |heights: &[[i32; 2]]| {
        widgets::list_box_rows(list, heights, None, &resources, &bindings).unwrap()
    };
    assert_eq!(
        rows(&[[10, 10], [10, 10], [10, 12]]).places,
        [Some([206, 8]), Some([206, 18]), None]
    );
    assert_eq!(rows(&[[10, 10], [10, 0]]).places, [None, None]);
}

#[test]
fn list_box_scrollbars_show_as_their_rows_leave_them() {
    content_reader_script::test_game::enter();
    let document = |body: &str| {
        read(
            format!("guiTypes = {{ {PLAIN} containerWindowType = {{ name = \"w\" {body} }} }}")
                .as_bytes(),
        )
    };
    let run = |body: &str, rows: &[[i32; 2]], current: Option<f32>| {
        let gui = document(body);
        let window = gui.template("w").unwrap().clone();
        let mut resources = fixture();
        resources.templates.push(gui);
        let bindings = Bindings {
            viewport: Some([200, 100]),
            ..Bindings::default()
        };
        let list = window.children[0].clone();
        let rows = widgets::list_box_rows(&list, rows, current, &resources, &bindings).unwrap();
        let layout = layout(&window, None, &resources, &bindings)
            .unwrap()
            .unwrap();
        let placed = layout.holders[0].place_scrollbar(&rows);
        (rows, placed)
    };
    let standard = "listBoxType = { name = \"l\" position = { x = 4 y = 6 } \
                    size = { x = 30 y = 25 } borderSize = { x = 2 y = 2 } \
                    offset = { x = 1 y = 2 } scrollbarType = \"lsb\" }";
    let (rows, placed) = run(standard, &[[10, 10], [10, 10], [10, 10]], None);
    assert_eq!(rows.places, [Some([6, 8]), Some([6, 18]), None]);
    assert_eq!(
        rects_of(&placed),
        [
            [35.0, 8.0, 10.0, 10.0],
            [35.0, 23.0, 10.0, 10.0],
            [35.0, 18.0, 3.0, 17.0],
            [35.0, 18.0, 3.0, 17.0],
        ]
    );
    assert_eq!(rows.range, 1);
    let (scrolled, moved) = run(standard, &[[10, 10]; 3], Some(1.0));
    assert_eq!(scrolled.places, [None, Some([6, 8]), Some([6, 18])]);
    assert_ne!(rects_of(&moved)[3], rects_of(&placed)[3]);
    assert!(run(standard, &[[10, 10], [10, 10]], None).1.is_empty());
    let left = standard.replace("scrollbarType", "scrollbar_side = left scrollbarType");
    assert_eq!(
        rects_of(&run(&left, &[[10, 10]; 3], None).1)[0][0],
        4.0 + 1.0 - 10.0
    );
    let smooth = "smoothListboxType = { name = \"l\" position = { x = 4 y = 6 } \
                  size = { x = 30 y = 25 } borderSize = { x = 2 y = 9 } \
                  orientation = \"center\" scrollbarType = \"lsb\" }";
    let (rows, placed) = run(smooth, &[[10, 10], [10, 10], [10, 12]], None);
    assert_eq!(
        rows.places,
        [Some([206, 108]), Some([206, 118]), Some([206, 128])]
    );
    assert_eq!(rects_of(&placed)[0], [234.0, 106.0, 10.0, 10.0]);
    assert!(placed.iter().all(|part| part.clip.is_none()));
    let (rows, moved) = run(smooth, &[[10, 10], [10, 10], [10, 12]], Some(5.0));
    assert_eq!(rows.places[0], Some([206, 103]));
    assert_eq!(rows.range, 7);
    assert_ne!(rects_of(&moved)[3], rects_of(&placed)[3]);
    assert_eq!(widgets::smooth_list_step(5.0), 4.0);
    assert!(run(smooth, &[[10, 10]], None).1.is_empty());
}

#[test]
fn edit_boxes_drop_downs_browsers_and_curves() {
    content_reader_script::test_game::enter();
    let paths = |body: &str| {
        laid_document(
            &format!("guiTypes = {{ containerWindowType = {{ name = \"w\" {body} }} }}"),
            Some([200, 100]),
            |_, _| {},
        )
        .map(|layout| {
            layout
                .unwrap()
                .parts
                .into_iter()
                .map(|part| part.path)
                .collect::<Vec<_>>()
        })
    };
    let layout = laid(
        "editBoxType = { name = \"e\" font = \"Main_14\" text = \"ab\" \
         position = { x = 3 y = 4 } size = { x = 100 y = 20 } }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(layout.parts.len(), 1);
    assert_eq!(rect(&layout.parts[0].kind)[0], 3.0);
    let layout = laid(
        "editBoxType = { name = \"e\" font = \"Main_14\" text = \"MOD_KEY\" \
         position = { x = 3 y = 4 } size = { x = 50 y = 20 } instantTextBoxType = no }",
        None,
    )
    .unwrap()
    .unwrap();
    let PartKind::TextSprite { bake, geometry, .. } = &layout.parts[0].kind else {
        panic!("not a text sprite");
    };
    assert_eq!(bake.size, [50, 20]);
    assert!(bake.background.is_none());
    assert_eq!(
        bake.glyphs.iter().map(|glyph| glyph.at).collect::<Vec<_>>(),
        [[0.0, 0.0], [7.0, 0.0], [21.0, 0.0]]
    );
    assert_eq!(geometry.rect, [3.0, 4.0, 50.0, 20.0]);
    let measured = parts(
        "editBoxType = { name = \"e\" font = \"Main_14\" text = \"MOD_KEY\" \
         instantTextBoxType = no }",
    );
    let PartKind::TextSprite { bake, .. } = &measured[0] else {
        panic!("not a text sprite");
    };
    assert_eq!(bake.size, [28, 14]);
    let error = laid(
        "editBoxType = { name = \"e\" font = \"Main_14\" text = \"ab\" \
         textureFile = \"gfx/edit.dds\" instantTextBoxType = no }",
        None,
    )
    .unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::MissingResource);
    let drop = paths(
        "dropDownBoxType = { name = \"d\" position = { x = 10 y = 20 } \
         expandButton = { name = \"b\" spriteType = \"GFX_even\" position = { x = 1 y = 1 } } \
         expandedWindow = { name = \"x\" \
         iconType = { name = \"hidden\" spriteType = \"GFX_odd\" } } \
         iconType = { name = \"header_icon\" spriteType = \"GFX_odd\" } }",
    )
    .unwrap();
    assert_eq!(drop.len(), 2, "{drop:?}");
    assert!(drop[0].ends_with("header_icon"), "{drop:?}");
    assert!(drop[1].ends_with(".b"), "{drop:?}");
    let error = laid("browserType = { name = \"b\" }", None).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::MissingInput);
    assert_eq!(
        paths("curveGraphType = { name = \"c\" spriteType = \"GFX_even\" }"),
        Ok(Vec::new())
    );
    let shown = laid_bound(
        "curveGraphType = { name = \"c\" spriteType = \"GFX_even\" }",
        None,
        |window, bindings| {
            request(bindings, window.children.first().unwrap(), |shown| {
                shown.show();
            });
        },
    )
    .unwrap_err();
    assert_eq!(shown[0].kind, RefusalKind::MissingInput);
}

#[test]
fn drop_down_headers_take_the_viewport() {
    content_reader_script::test_game::enter();
    let body = |hide: &str| {
        format!(
            "guiTypes = {{ containerWindowType = {{ name = \"w\" \
             dropDownBoxType = {{ name = \"d\" position = {{ x = 10 y = 20 }} \
             size = {{ width = 50% height = 40 }} margin = {{ left = 5 top = 3 }} {hide} \
             expandButton = {{ name = \"b\" spriteType = \"GFX_even\" \
             position = {{ x = 1 y = 1 }} orientation = \"center\" }} \
             iconType = {{ name = \"header_icon\" spriteType = \"GFX_odd\" }} }} }} }}"
        )
    };
    let layout = laid_document(&body(""), Some([200, 100]), |_, _| {})
        .unwrap()
        .unwrap();
    assert_eq!(rect(&layout.parts[0].kind), [15.0, 23.0, 15.0, 9.0]);
    assert_eq!(rect(&layout.parts[1].kind), [61.0, 41.0, 20.0, 10.0]);
    let hidden = laid_document(&body("hide = yes"), Some([200, 100]), |_, _| {})
        .unwrap()
        .unwrap();
    assert_eq!(hidden.parts.len(), 1);
    assert!(hidden.parts[0].path.ends_with(".b"));
    let error = laid_document(&body(""), None, |_, _| {}).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::NoParent);
}

#[test]
fn scroll_values_snap_as_the_game_snaps_them() {
    use widgets::Range;
    let range = Range::new(0, 1_000_000, 300_000);
    assert_eq!(range.count, 4);
    assert_eq!(range.snap(-5), 0);
    assert_eq!(range.snap(2_000_000), 1_000_000);
    assert_eq!(range.snap(450_000), 300_000);
    assert_eq!(range.snap(460_000), 600_000);
    assert_eq!(Range::new(0, 1_000_000, 100).snap(123), 123);
    assert_eq!(Range::new(0, 100_000, 0).count, 42950);
    assert_eq!(widgets::fixed(1.5), 150_000);
    assert_eq!(widgets::fixed(-1.5), -150_000);
}

#[test]
fn charts_and_cornered_tiles_follow_their_native_state() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let even = resources.sprites["GFX_even"].clone();
    let chart = |kind: SpriteKind| SpriteInfo {
        kind,
        texture: [0; 2],
        ..even.clone()
    };
    resources
        .sprites
        .insert("GFX_pie", chart(SpriteKind::PieChart { size: Some(20) }));
    resources.sprites.insert(
        "GFX_unsized_pie",
        chart(SpriteKind::PieChart { size: None }),
    );
    resources
        .sprites
        .insert("GFX_line", chart(SpriteKind::LineChart { size: [30, 10] }));
    let mut looping = resources.sprites["GFX_cornered"].clone();
    looping.kind = SpriteKind::CorneredTile {
        size: [30, 20],
        border: [4, 4],
        tiling_center: false,
        looping: true,
        rate: None,
    };
    resources.sprites.insert("GFX_looping", looping);
    let run = |body: &str, opacity: Option<f32>| {
        let gui = gui(body);
        let window = gui.template("w").unwrap();
        let mut bindings = Bindings::default();
        bindings.bind(window).opacity = opacity;
        layout(window, None, &resources, &bindings)
    };
    let icon = |sprite: &str| format!("iconType = {{ name = \"i\" spriteType = \"{sprite}\" }}");
    assert_eq!(
        run(&icon("GFX_unsized_pie"), None).unwrap_err()[0].kind,
        RefusalKind::UndefinedRead
    );
    for sprite in ["GFX_pie", "GFX_line"] {
        assert!(run(&icon(sprite), None).unwrap().unwrap().parts.is_empty());
    }
    assert_eq!(
        run(&icon("GFX_looping"), None).unwrap_err()[0].kind,
        RefusalKind::UndefinedRead
    );
    let faded = run(&icon("GFX_cornered"), Some(0.25)).unwrap().unwrap();
    assert_eq!(faded.parts[0].color[3], 0.25);
}

#[test]
fn circular_progress_bars_draw_their_fan() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let circle = SpriteInfo {
        texture: [16, 16],
        kind: SpriteKind::CircularProgress {
            size: 10,
            rotation: 0.0,
            amount: 0.25,
        },
        effect: Ok(synthetic::circular()),
        ..plain([16, 16], 1)
    };
    resources.sprites.insert("GFX_circle", circle);
    let run = |percent: Option<i32>| {
        let gui = gui(
            "iconType = { name = \"c\" spriteType = \"GFX_circle\" position = { x = 30 y = 40 } }",
        );
        let window = gui.template("w").unwrap();
        let mut bindings = Bindings::default();
        bindings.bind(window.children.first().unwrap()).progress = percent;
        layout(window, None, &resources, &bindings)
            .unwrap()
            .unwrap()
    };
    let layout = run(None);
    let PartKind::Fan {
        centre,
        radius,
        weights,
        ..
    } = layout.parts[0].kind
    else {
        panic!("not a fan");
    };
    assert_eq!(
        (centre, radius, weights),
        ([30.0, 50.0], [10.0, 10.0], [75, 25])
    );
    assert_eq!(
        layout.parts[0].shading,
        shading(&synthetic::circular(), "CircularProgressBar", 0.0)
    );
    let mut batch = Batch::default();
    draw(&layout, [0, 0], &one(), &mut batch).unwrap();
    assert_eq!(batch.quads.len(), 100);
    let first = batch.quads[0].exact.as_ref().unwrap();
    assert_eq!(first.corners[0], [40.0, 50.0]);
    assert_eq!(first.corners[1], [30.0, 50.0]);
    let step = 6.283_185_5f32 / 100.0;
    assert_eq!(
        first.corners[2],
        [30.0 + step.cos() * 10.0, 50.0 - step.sin() * 10.0]
    );
    assert_eq!(first.uv[1], [0.0, 0.0]);
    assert_eq!(first.inputs[2], 0.0);
    assert_eq!(batch.quads[75].exact.as_ref().unwrap().inputs[2], 1.0);
    let PartKind::Fan { weights, .. } = run(Some(60)).parts[0].kind else {
        panic!("not a fan");
    };
    assert_eq!(weights, [40, 60]);
}

#[test]
fn plain_scrollbar_parts_follow_their_constructor() {
    content_reader_script::test_game::enter();
    let run = |templates: &str, bar: &str| {
        laid_document(
            &format!(
                "guiTypes = {{ {templates} containerWindowType = {{ name = \"w\" \
                 size = {{ width = 100 height = 100 }} {bar} }} }}"
            ),
            None,
            |_, _| {},
        )
    };
    let parts = "buttonType = { name = \"up\" spriteType = \"GFX_four\" orientation = center } \
                 buttonType = { name = \"down\" spriteType = \"GFX_four\" } \
                 guiButtonType = { name = \"track\" spriteType = \"GFX_cornered\" } \
                 guiButtonType = { name = \"slider\" spriteType = \"GFX_bar3x17\" }";
    let names = "slider = \"slider\" track = \"track\" leftButton = \"down\" \
                 rightButton = \"up\"";
    let bar = |extra: &str, children: &str| {
        format!(
            "scrollbarType = {{ name = \"bar\" position = {{ x = 2 y = 3 }} \
             size = {{ x = 10 y = 60 }} minValue = 0 maxValue = 4 startValue = 1 {names} \
             {extra} {children} }}"
        )
    };
    let own = run("", &bar("", parts)).unwrap().unwrap();
    let rects = rects_of(&own.parts);
    assert_eq!(rects[0], [2.0, 3.0, 10.0, 10.0]);
    assert_eq!(rects[1], [2.0, 53.0, 10.0, 10.0]);
    assert_eq!(rects[2], [2.0, 13.0, 4.0, 4.0]);
    assert_eq!(rects[3], [2.0, 18.0, 3.0, 17.0]);
    let full = |extra: &str, children: &str| {
        format!(
            "scrollbarType = {{ name = \"bar\" position = {{ x = 2 y = 3 }} \
             size = {{ x = 10 y = 60 }} borderSize = {{ x = 1 y = 1 }} minValue = -4 \
             maxValue = 4 startValue = 1 stepSize = 1 priority = 1 {names} {extra} \
             {children} }}"
        )
    };
    let own_full = rects_of(&run("", &full("", parts)).unwrap().unwrap().parts);
    assert_eq!(own_full[3], [2.0, 27.0, 3.0, 17.0]);
    let inherited = run(
        &format!("scrollbarType = {{ name = \"base\" {parts} }}"),
        &full("parent = \"base\"", ""),
    )
    .unwrap()
    .unwrap();
    assert_eq!(rects_of(&inherited.parts), own_full);
    let error = run("", &full("parent = \"nothing\"", "")).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::NullRead);
    let icon = parts.replace(
        "buttonType = { name = \"up\" spriteType = \"GFX_four\" orientation = center }",
        "iconType = { name = \"up\" spriteType = \"GFX_four\" }",
    );
    assert_eq!(
        rects_of(&run("", &bar("", &icon)).unwrap().unwrap().parts)[0],
        [2.0, 3.0, 10.0, 10.0]
    );
    let slider = parts.replace(
        "guiButtonType = { name = \"slider\" spriteType = \"GFX_bar3x17\" }",
        "iconType = { name = \"slider\" spriteType = \"GFX_bar3x17\" }",
    );
    assert_eq!(
        run("", &bar("", &slider)).unwrap_err()[0].kind,
        RefusalKind::TypeConfusion
    );
    let icons = format!(
        "{parts} iconType = {{ name = \"lo\" spriteType = \"GFX_four\" }} \
         iconType = {{ name = \"hi\" spriteType = \"GFX_four\" }}"
    );
    let limits = "useRangeLimit = yes rangeLimitMin = 1 rangeLimitMax = 3 \
                  rangeLimitMinIcon = \"lo\" rangeLimitMaxIcon = \"hi\"";
    let limited = run("", &bar(limits, &icons)).unwrap().unwrap();
    let rects = rects_of(&limited.parts);
    assert_eq!(rects.len(), 6);
    assert_eq!(rects[4], [2.0, 27.0, 10.0, 10.0]);
    assert_eq!(rects[5], [2.0, 38.0, 10.0, 10.0]);
    let off = limits.replace("useRangeLimit = yes", "");
    assert_eq!(run("", &bar(&off, &icons)).unwrap().unwrap().parts.len(), 4);
    let hidden = run("", &bar(&format!("{limits} hide = yes"), &icons))
        .unwrap()
        .unwrap();
    assert_eq!(rects_of(&hidden.parts), [rects[4], rects[5]]);
    let error = run("", &bar(&off.replace("\"hi\"", "\"none\""), &icons)).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::NullRead);
}

#[test]
fn text_box_scrollbars_show_when_the_text_overflows() {
    content_reader_script::test_game::enter();
    let run = |key: &str, scrollbar: &str, bind: Option<&str>| {
        laid_document(
            &format!(
                "guiTypes = {{ {PLAIN} {SCROLLBAR} containerWindowType = {{ name = \"w\" \
                 instantTextBoxType = {{ name = \"t\" font = \"Main_14\" text = \"{key}\" \
                 position = {{ x = 5 y = 7 }} maxWidth = 50 maxHeight = 20 \
                 scrollbarType = \"{scrollbar}\" }} }} }}"
            ),
            None,
            |window, bindings| {
                if let Some(text) = bind {
                    bindings.bind(&window.children[0]).text = Some(text.into());
                }
            },
        )
    };
    let layout = run("MOD_LONG", "lsb", None).unwrap().unwrap();
    let text = &layout.parts[0];
    assert!(matches!(text.kind, PartKind::Text { .. }));
    assert_eq!(text.clip, Some([5, 7, 55, 27]));
    assert_eq!(
        rects_of(&layout.parts[1..]),
        [
            [55.0, 7.0, 10.0, 10.0],
            [55.0, 17.0, 10.0, 10.0],
            [55.0, 17.0, 3.0, 17.0],
            [55.0, 17.0, 3.0, 17.0],
        ]
    );
    let short = run("MOD_KEY", "lsb", None).unwrap().unwrap();
    assert_eq!(short.parts.len(), 1);
    assert_eq!(short.parts[0].clip, None);
    let bound = run("MOD_KEY", "lsb", Some("aaaa bbbb cccc"))
        .unwrap()
        .unwrap();
    assert_eq!(bound.parts.len(), 5);
    assert_eq!(
        run("MOD_KEY", "none", None).unwrap_err()[0].kind,
        RefusalKind::NullRead
    );
    assert_eq!(
        run("MOD_KEY", "sb", None).unwrap_err()[0].kind,
        RefusalKind::TypeConfusion
    );
}

#[test]
fn text_box_bound_offsets_move_glyphs_regions_and_scrollbars() {
    content_reader_script::test_game::enter();
    let run = |offset| {
        laid_document(
            &format!(
                "guiTypes = {{ {PLAIN} containerWindowType = {{ name = w
                instantTextBoxType = {{ name = t font = Main_14 text = MOD_LONG
                    position = {{ x = 5 y = 7 }} maxWidth = 50 maxHeight = 20
                    scrollbarType = lsb }} }} }}"
            ),
            None,
            |window, bindings| {
                bindings.bind(&window.children[0]).offset = Some(offset);
            },
        )
        .unwrap()
        .unwrap()
    };
    let baseline = run([0, 0]);
    let moved = run([17, -9]);
    let region = moved
        .regions
        .iter()
        .find(|region| region.path == "w.t")
        .unwrap();
    assert_eq!(region.rect, [22.0, -2.0, 50.0, 20.0]);
    assert_eq!(moved.parts[0].clip, Some([22, -2, 72, 18]));
    let PartKind::Text { glyphs: before, .. } = &baseline.parts[0].kind else {
        panic!("native glyph quads");
    };
    let PartKind::Text { glyphs: after, .. } = &moved.parts[0].kind else {
        panic!("native glyph quads");
    };
    assert_eq!(after[0].at, [before[0].at[0] + 17.0, before[0].at[1] - 9.0]);
    assert_eq!(
        rects_of(&moved.parts[1..]),
        [
            [72.0, -2.0, 10.0, 10.0],
            [72.0, 8.0, 10.0, 10.0],
            [72.0, 8.0, 3.0, 17.0],
            [72.0, 8.0, 3.0, 17.0],
        ]
    );
}

fn finalized_grid_resources(smooth: bool) -> Fixture {
    let bar = SCROLLBAR.replace(
        "size = { width = 10 height = 50 }",
        "size = { width = 10 height = 50 } smooth_scrolling = 0.5",
    );
    fixture_with(read(
        format!(
            "guiTypes = {{ {bar} containerWindowType = {{ name = \"entry\" \
         size = {{ width = 60 height = 40 }} }} \
         containerWindowType = {{ name = \"w\" size = {{ width = 100 height = 80 }} \
         background = {{ spriteType = \"GFX_even\" }} verticalScrollbar = \"sb\" \
         vertical_scroll_step = 40 smooth_scrolling = {} \
         gridBoxType = {{ name = \"rows\" size = {{ width = 100 height = 80 }} \
         slotsize = {{ width = 100 height = 40 }} max_slots_horizontal = 1 }} }} }}",
            if smooth { "yes" } else { "no" },
        )
        .as_bytes(),
    ))
}

#[test]
fn runtime_container_size_keeps_flag_zero_child_geometry() {
    content_reader_script::test_game::enter();
    let resources = fixture_with(read(
        br#"guiTypes = {
        containerWindowType = {
            name = "w" size = { width = 10 height = 10 }
            margin = { left = 2 right = 3 top = 1 bottom = 2 } clipping = yes
            background = { spriteType = "GFX_cornered" }
            iconType = { name = "i" spriteType = "GFX_even" orientation = center }
        }
    }"#,
    ));
    let window = resources.gui_type(b"w").unwrap();
    let mut instance = Instance::construct(window, [200, 100], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let before = instance.layout(window, &resources, None).unwrap().unwrap();
    let icon = |layout: &Layout| {
        rect(
            &layout
                .parts
                .iter()
                .find(|part| part.path == "w.i")
                .unwrap()
                .kind,
        )
    };
    instance
        .set_size(window, window, [80, 40], &resources)
        .unwrap();
    let after = instance
        .layout_with_regions(window, &resources, None, true)
        .unwrap()
        .unwrap();
    assert_eq!(after.size, [80, 40]);
    assert_eq!(instance.content_size(window, window).unwrap(), [75, 37]);
    assert_eq!(icon(&after), icon(&before));
    assert_eq!(
        after
            .parts
            .iter()
            .find(|part| part.path == "w.i")
            .unwrap()
            .clip,
        Some([2, 1, 77, 38])
    );
    assert_eq!(
        after
            .regions
            .iter()
            .find(|region| region.path == "w")
            .unwrap()
            .rect,
        [0.0, 0.0, 80.0, 40.0]
    );
    let resources = fixture_with(read(br#"guiTypes = {
        containerWindowType = { name = "w" size = { width = 50% height = 3 min = { width = 40 height = 20 } } }
    }"#));
    let window = resources.gui_type(b"w").unwrap();
    let mut instance = Instance::construct(window, [200, 100], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance
        .set_size(window, window, [53, 10], &resources)
        .unwrap();
    assert_eq!(
        instance
            .layout(window, &resources, None)
            .unwrap()
            .unwrap()
            .size,
        [54, 20]
    );
}

#[test]
fn finalized_grid_resolved_offsets_keep_runtime_extents() {
    content_reader_script::test_game::enter();
    let resources = finalized_grid_resources(false);
    let window = resources.gui_type(b"w").unwrap();
    let grid = window.child_named("rows").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let layout = instance.layout(window, &resources, None).unwrap().unwrap();
    let items = [([-20, -7], [40, 30]), ([77, 90], [48, 50])];
    let finalized = instance
        .finalize_grid_resolved(window, grid, &layout.holders[0], &items)
        .unwrap();
    assert_eq!(finalized.items, [[-20, -7], [77, 90]]);
    assert_eq!(finalized.relaid, [-20, -7]);
    assert_eq!(finalized.extent, [145, 147]);
    instance
        .synchronize_scroll(window, window, &resources, true)
        .unwrap();
    assert_eq!(
        instance.scroll_range(window, window, true).unwrap(),
        Some([-7.0, 60.0])
    );
    instance
        .finalize_grid_resolved(window, grid, &layout.holders[0], &[])
        .unwrap();
    instance
        .synchronize_scroll(window, window, &resources, true)
        .unwrap();
    assert!(
        scrollbar_rects(&instance.layout(window, &resources, None).unwrap().unwrap()).is_empty()
    );
}

#[test]
fn scrollbar_regions_keep_native_owner_and_action() {
    content_reader_script::test_game::enter();
    let bar = SCROLLBAR
        .replace("name = \"inc\"", "name = \"more\"")
        .replace("name = \"dec\"", "name = \"less\"");
    let resources = fixture_with(read(format!(r#"guiTypes = {{ {bar}
        containerWindowType = {{ name = "w" size = {{ width = 250 height = 100 }}
            containerWindowType = {{ name = "left" size = {{ width = 100 height = 80 }} verticalScrollbar = "sb" autohide_scrollbars = no }}
            containerWindowType = {{ name = "right" position = {{ x = 100 y = 0 }} size = {{ width = 100 height = 80 }} verticalScrollbar = "sb" autohide_scrollbars = no }}
        }}
    }}"#).as_bytes()));
    let window = resources.gui_type(b"w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    assert!(
        instance
            .layout(window, &resources, None)
            .unwrap()
            .unwrap()
            .regions
            .is_empty()
    );
    let layout = instance
        .layout_with_regions(window, &resources, None, true)
        .unwrap()
        .unwrap();
    let actions: Vec<_> = layout
        .regions
        .iter()
        .filter_map(|region| {
            region
                .scroll
                .map(|action| (region.owner.as_deref(), region.path.as_str(), action))
        })
        .collect();
    assert_eq!(
        actions,
        [
            (Some("w.left"), "sb.more", (true, 1)),
            (Some("w.left"), "sb.less", (true, -1)),
            (Some("w.right"), "sb.more", (true, 1)),
            (Some("w.right"), "sb.less", (true, -1)),
        ]
    );
}

#[test]
fn finalized_grid_extents_synchronize_container_scrolling() {
    content_reader_script::test_game::enter();
    let resources = finalized_grid_resources(false);
    let window = resources.gui_type(b"w").unwrap();
    let grid = window.child_named("rows").unwrap();
    let entry = resources.gui_type(b"entry").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let initial = instance.layout(window, &resources, None).unwrap().unwrap();
    assert!(scrollbar_rects(&initial).is_empty());
    let finalized = instance
        .finalize_grid(window, grid, &initial.holders[0], &[entry; 3])
        .unwrap();
    assert_eq!(finalized.places(), [[0, 0], [0, 40], [0, 80]]);
    assert_eq!(finalized.extent, [100, 120]);
    instance
        .synchronize_scroll(window, window, &resources, false)
        .unwrap();
    assert_eq!(
        instance.scroll_range(window, window, true).unwrap(),
        Some([0.0, 40.0])
    );
    let full = instance.layout(window, &resources, None).unwrap().unwrap();
    assert!(!scrollbar_rects(&full).is_empty());
    instance
        .set_scroll_target(window, window, true, 47.0, &resources)
        .unwrap();
    let moved = instance.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(moved.holders[0].origin, [0.0, -40.0]);
    assert_eq!(moved.holders[0].clip, initial.holders[0].clip);
    instance
        .finalize_grid(window, grid, &moved.holders[0], &[entry])
        .unwrap();
    instance
        .synchronize_scroll(window, window, &resources, false)
        .unwrap();
    let hidden = instance.layout(window, &resources, None).unwrap().unwrap();
    assert!(scrollbar_rects(&hidden).is_empty());
    assert_eq!(hidden.holders[0].origin, [0.0, 0.0]);
    assert_eq!(
        instance.scroll_range(window, window, true).unwrap(),
        Some([0.0, 40.0])
    );
}

#[test]
fn native_scroll_inputs_keep_current_and_target_distinct() {
    content_reader_script::test_game::enter();
    let resources = finalized_grid_resources(true);
    let window = resources.gui_type(b"w").unwrap();
    let grid = window.child_named("rows").unwrap();
    let entry = resources.gui_type(b"entry").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let initial = instance.layout(window, &resources, None).unwrap().unwrap();
    instance
        .finalize_grid(window, grid, &initial.holders[0], &[entry; 5])
        .unwrap();
    instance
        .synchronize_scroll(window, window, &resources, false)
        .unwrap();
    assert_eq!(
        instance.scroll_range(window, window, true).unwrap(),
        Some([0.0, 120.0])
    );
    instance
        .set_scroll_target(window, window, true, 80.0, &resources)
        .unwrap();
    let origin = |instance: &Instance| {
        let layout = instance.layout(window, &resources, None).unwrap().unwrap();
        assert_eq!(layout.holders[0].clip, initial.holders[0].clip);
        layout.holders[0].origin
    };
    let tick = Tick {
        time: 1.0,
        dt: 0.016,
    };
    instance.tick(window, &resources, tick).unwrap();
    assert_eq!(origin(&instance), [0.0, -40.0]);
    instance
        .step_scroll(window, window, true, 1, &resources)
        .unwrap();
    assert_eq!(origin(&instance), [0.0, -40.0]);
    instance.tick(window, &resources, tick).unwrap();
    assert_eq!(origin(&instance), [0.0, -60.0]);

    instance
        .wheel_scroll(window, window, true, 1, &resources)
        .unwrap();
    assert_eq!(origin(&instance), [0.0, -95.0]);
    instance.tick(window, &resources, tick).unwrap();
    assert_eq!(origin(&instance), [0.0, -70.0]);
    instance
        .wheel_scroll(window, window, true, 10, &resources)
        .unwrap();
    assert_eq!(origin(&instance), [0.0, -120.0]);
    instance.tick(window, &resources, tick).unwrap();
    assert_eq!(origin(&instance), [0.0, -95.0]);
    instance
        .wheel_scroll(window, window, true, -20, &resources)
        .unwrap();
    assert_eq!(origin(&instance), [0.0, 0.0]);
    instance
        .step_scroll(window, window, true, -1, &resources)
        .unwrap();
    assert_eq!(origin(&instance), [0.0, -95.0]);
    instance.tick(window, &resources, tick).unwrap();
    assert_eq!(origin(&instance), [0.0, -67.0]);
    let refusal = instance
        .step_scroll(window, window, true, 0, &resources)
        .unwrap_err();
    assert_eq!(refusal.len(), 1);
    assert_eq!(refusal[0].kind, RefusalKind::MissingInput);

    instance
        .finalize_grid(window, grid, &initial.holders[0], &[])
        .unwrap();
    instance
        .synchronize_scroll(window, window, &resources, false)
        .unwrap();
    instance
        .wheel_scroll(window, window, true, 1, &resources)
        .unwrap();
    assert_eq!(origin(&instance), [0.0, 0.0]);
}

#[test]
fn finalized_grid_scrolling_runs_on_the_instance_clock() {
    content_reader_script::test_game::enter();
    let resources = finalized_grid_resources(true);
    let window = resources.gui_type(b"w").unwrap();
    let grid = window.child_named("rows").unwrap();
    let entry = resources.gui_type(b"entry").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let initial = instance.layout(window, &resources, None).unwrap().unwrap();
    instance
        .finalize_grid(window, grid, &initial.holders[0], &[entry; 3])
        .unwrap();
    instance
        .synchronize_scroll(window, window, &resources, false)
        .unwrap();
    instance
        .set_scroll_target(window, window, true, 40.0, &resources)
        .unwrap();
    assert_eq!(
        instance
            .layout(window, &resources, None)
            .unwrap()
            .unwrap()
            .holders[0]
            .origin,
        [0.0, 0.0]
    );
    instance
        .tick(
            window,
            &resources,
            Tick {
                time: 1.0,
                dt: 0.016,
            },
        )
        .unwrap();
    let moved = instance.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(moved.holders[0].origin, [0.0, -20.0]);
    assert_eq!(moved.holders[0].clip, initial.holders[0].clip);
    instance.hide(window, grid).unwrap();
    instance
        .synchronize_scroll(window, window, &resources, false)
        .unwrap();
    assert!(
        scrollbar_rects(&instance.layout(window, &resources, None).unwrap().unwrap()).is_empty()
    );
    instance
        .synchronize_scroll(window, window, &resources, true)
        .unwrap();
    assert!(
        !scrollbar_rects(&instance.layout(window, &resources, None).unwrap().unwrap()).is_empty()
    );
}

#[test]
fn container_scrollbars_move_their_content() {
    content_reader_script::test_game::enter();
    let bar = |extra: &str| {
        SCROLLBAR.replace(
            "size = { width = 10 height = 50 }",
            &format!("size = {{ width = 10 height = 50 }} minValue = 2 maxValue = 5 {extra}"),
        )
    };
    let run = |bar: &str, window: &str, bind: &dyn Fn(&Element, &mut Bindings)| {
        laid_document(
            &format!(
                "guiTypes = {{ {bar} containerWindowType = {{ name = \"w\" \
                 size = {{ width = 100 height = 80 }} verticalScrollbar = \"sb\" {window} \
                 iconType = {{ name = \"i\" spriteType = \"GFX_even\" \
                 position = {{ x = 0 y = 10 }} }} }} }}"
            ),
            None,
            bind,
        )
    };
    let hidden = run(&bar(""), "", &|_, _| {}).unwrap().unwrap();
    assert_eq!(rect(&hidden.parts[0].kind), [0.0, 8.0, 20.0, 10.0]);
    assert!(scrollbar_rects(&hidden).is_empty());
    let smooth = format!(
        "guiTypes = {{ {} containerWindowType = {{ name = \"w\"              size = {{ width = 100 height = 80 }} verticalScrollbar = \"sb\"              smooth_scrolling = yes background = {{ spriteType = \"GFX_even\" }}              iconType = {{ name = \"far\" spriteType = \"GFX_even\"              position = {{ x = 0 y = 75 }} }}              iconType = {{ name = \"i\" spriteType = \"GFX_even\"              position = {{ x = 0 y = 10 }} }} }} }}",
        bar("smooth_scrolling = 0.5")
    );
    let resources = fixture_with(read(smooth.as_bytes()));
    let window = resources.gui_type(b"w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let far = |instance: &Instance| {
        let laid = instance.layout(window, &resources, None).unwrap().unwrap();
        assert_eq!(rect(&laid.parts[0].kind)[1], 0.0);
        rect(&laid.parts[1].kind)[1]
    };
    assert_eq!(far(&instance), 73.0);
    let mut moved = Vec::new();
    for _ in 0..6 {
        assert!(instance.active());
        instance.tick(window, &resources, dt(0.016)).unwrap();
        moved.push(far(&instance));
    }
    assert_eq!(moved, [74.0, 74.0, 74.0, 74.0, 74.0, 73.0]);
    assert!(!instance.active());
    let mut hidden = Instance::construct(window, [800, 600], &resources).unwrap();
    hidden.tick(window, &resources, dt(0.016)).unwrap();
    hidden.gui_show(window, window).unwrap();
    assert_eq!(far(&hidden), 73.0);
    let slider = bar("smooth_scrolling = 0.5").replace(
        "slider = { name = \"slider\" spriteType = \"GFX_bar3x17\" }",
        "",
    );
    let error = run(
        &slider,
        "smooth_scrolling = yes background = { spriteType = \"GFX_even\" } \
         iconType = { name = \"far\" spriteType = \"GFX_even\" position = { x = 0 y = 75 } }",
        &|_, _| {},
    )
    .unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::NullRead);
    let framed = run(
        &bar("setTrackFrameOnChange = yes"),
        "autohide_scrollbars = no",
        &|_, _| {},
    )
    .unwrap()
    .unwrap();
    let track = framed
        .parts
        .iter()
        .find(|part| part.path.ends_with("sb.track"))
        .unwrap();
    assert_eq!(frame_of(&track.kind), 66);
}

#[test]
fn overflow_boxes_measure_each_kind() {
    content_reader_script::test_game::enter();
    let shows = |child: &str| {
        let layout = laid_document(
            &format!(
                "guiTypes = {{ {SCROLLBAR} {PLAIN} containerWindowType = {{ name = \"w\" \
                 size = {{ width = 100 height = 80 }} verticalScrollbar = \"sb\" \
                 background = {{ spriteType = \"GFX_even\" }} {child} }} }}"
            ),
            Some([200, 100]),
            |_, _| {},
        )
        .unwrap()
        .unwrap();
        !scrollbar_rects(&layout).is_empty()
    };
    let text = |key: &str, y: i32| {
        format!(
            "instantTextBoxType = {{ name = \"t\" font = \"Main_14\" text = \"{key}\" \
             position = {{ x = 0 y = {y} }} maxWidth = 50 }}"
        )
    };
    assert!(shows(&text("MOD_LINES", 0)));
    assert!(!shows(&text("MOD_LONG", 200)));
    assert!(shows(
        "listBoxType = { name = \"l\" position = { x = 0 y = 90 } orientation = \"upper_left\" }"
    ));
    assert!(!shows(
        "listBoxType = { name = \"l\" position = { x = 0 y = 50 } orientation = \"lower_left\" }"
    ));
    assert!(shows(
        &PLAIN
            .replace("name = \"lsb\"", "name = \"p\" position = { x = 0 y = 75 }")
            .replace("size = { x = 10 y = 60 }", "size = { x = 10 y = 10 }")
    ));
    assert!(shows(
        "buttonType = { name = \"b\" font = \"Main_14\" text = \"MOD_LINES\"              multiline = yes }"
    ));
    assert!(shows(
        "curveGraphType = { name = \"c\" spriteType = \"GFX_even\" size = { x = 10 y = 90 } }"
    ));
}

#[test]
fn a_turned_cornered_tile_draws_unturned() {
    content_reader_script::test_game::enter();
    let turned = parts("iconType = { name = \"c\" spriteType = \"GFX_cornered\" rotation = 30 }");
    let still = parts("iconType = { name = \"c\" spriteType = \"GFX_cornered\" }");
    assert_eq!(turned, still);
}

#[test]
fn text_bakes_submit_each_frames_glyphs_once() {
    content_reader_script::test_game::enter();
    let icon = native_text::ICON;
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_text").unwrap().texture = [9000, 20];
    let gui = gui("buttonType = { name = \"b\" spriteType = \"GFX_text\" font = \"Main_14\" }");
    let window = gui.template("w").unwrap();
    let mut bindings = Bindings::default();
    bindings.bind(window.find(Kind::Button, "b").unwrap()).text =
        Some(format!("{}{icon}odd b", "a".repeat(600)));
    let laid = layout(window, None, &resources, &bindings)
        .unwrap()
        .unwrap();
    let PartKind::TextSprite { bake, .. } = &laid.parts[0].kind else {
        panic!("not a text sprite");
    };
    assert_eq!(bake.frame_ends.len(), 2);
    let [glyphs, icons] = bake.frame_ends[0];
    assert!(
        glyphs > GLYPHS_PER_FLUSH,
        "{glyphs} glyphs in the first frame"
    );
    assert_eq!(icons, 1);
}

#[test]
fn text_sprites_follow_the_setter_and_measure_their_bake() {
    content_reader_script::test_game::enter();
    let gui = gui("size = { width = 100 height = 50 } \
         buttonType = { name = \"b\" spriteType = \"GFX_even\" font = \"Main_14\" \
         text = \"MOD_KEY\" position = { x = 5 y = 6 } orientation = center rotation = 1.0 } \
         buttonType = { name = \"t\" font = \"Main_14\" size = { x = 0 y = 30 } }");
    let window = gui.template("w").unwrap();
    let b = window.find(Kind::Button, "b").unwrap();
    let t = window.find(Kind::Button, "t").unwrap();
    let resources = fixture();
    let authored = drawable_extent(b, &resources, &Bindings::default()).unwrap();
    let mut bindings = Bindings::default();
    bindings.bind(b).sprite = Some(b"GFX_undefined".to_vec());
    bindings.bind(b).setter = SpriteSetter::ButtonStandard;
    assert_eq!(drawable_extent(b, &resources, &bindings).unwrap(), authored);
    bindings.bind(b).sprite = Some(b"GFX_text".to_vec());
    bindings.bind(b).frame = Some(2);
    assert_eq!(
        drawable_extent(b, &resources, &bindings),
        Ok(Some([60, 20]))
    );
    let laid = layout(window, None, &resources, &bindings)
        .unwrap()
        .unwrap();
    let PartKind::TextSprite {
        geometry, offsets, ..
    } = &laid.parts[0].kind
    else {
        panic!("{:?}", laid.parts[0].kind);
    };
    assert_eq!(geometry.rect[..2], [5.0, 6.0]);
    assert_eq!(geometry.rotation, 0.0);
    assert_eq!(offsets[0], 0.5);
    let mut bindings = Bindings::default();
    bindings.bind(t).text = Some("abc".into());
    assert_eq!(
        drawable_extent(t, &resources, &bindings),
        Ok(Some([200, 14]))
    );
}

#[test]
fn text_sprites_bake_as_the_game_bakes_them() {
    content_reader_script::test_game::enter();
    let baked = laid(
        "buttonType = { name = \"b\" spriteType = \"GFX_text\" font = \"Main_14\" \
         text = \"MOD_KEY\" position = { x = 5 y = 6 } }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(baked.parts.len(), 1);
    let PartKind::TextSprite {
        bake,
        geometry,
        uv,
        offsets,
        ..
    } = &baked.parts[0].kind
    else {
        panic!("not a text sprite");
    };
    assert_eq!(bake.size, [120, 20]);
    assert_eq!(bake.background.as_ref().unwrap().quads.len(), 9);
    assert_eq!(
        bake.glyphs.iter().map(|glyph| glyph.at).collect::<Vec<_>>(),
        [
            [16.0, 3.0],
            [23.0, 3.0],
            [37.0, 3.0],
            [76.0, 3.0],
            [83.0, 3.0],
            [97.0, 3.0],
        ]
    );
    assert_eq!(geometry.rect, [5.0, 6.0, 60.0, 20.0]);
    assert_eq!((*uv, *offsets), ([0.0, 0.0, 0.5, 1.0], [0.0; 4]));
    let centred = laid(
        "buttonType = { name = \"b\" spriteType = \"GFX_text\" font = \"Main_14\" \
         text = \"MOD_KEY\" position = { x = 5 y = 6 } centerPosition = yes }",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(rect(&centred.parts[0].kind)[..2], [5.0 - 14.0, 6.0 - 7.0]);
    assert!(
        parts(
            "checkboxType = { name = \"c\" spriteType = \"GFX_unknown\" font = \"Main_14\" \
         text = \"MOD_KEY\" }"
        )
        .is_empty()
    );
    let boxed = parts(
        "checkboxType = { name = \"c\" spriteType = \"GFX_text\" font = \"Main_14\" \
         text = \"MOD_KEY\" }",
    );
    let PartKind::TextSprite { bake, .. } = &boxed[0] else {
        panic!("not a text sprite");
    };
    assert_eq!(bake.glyphs[0].at, [0.0, 3.0]);
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_text").unwrap().frames = 0;
    let gui = gui("iconType = { name = \"i\" spriteType = \"GFX_text\" }");
    let error = layout(
        gui.template("w").unwrap(),
        None,
        &resources,
        &Bindings::default(),
    )
    .unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::DivideByZero);
}

#[test]
fn animation_records_run_on_the_instance_clock() {
    content_reader_script::test_game::enter();
    let record = |kind: i32, looping: bool| AnimationRecord {
        data: content_reader_gui::gui::native::sprites::AnimationData {
            kind,
            looping,
            ..Default::default()
        },
        mask: 21,
        texture: 22,
        texture_size: [20, 20],
    };
    let rotating = animation::record_draw(&record(2, true), 0.25, 0.0, 1, [10, 10]).unwrap();
    let [cos, minus_sin, sin, _] = rotating.constants[0];
    assert!(cos.abs() < 1e-6 && (minus_sin - 1.0).abs() < 1e-6 && (sin + 1.0).abs() < 1e-6);
    assert_eq!(rotating.constants[1], [1.0, 1.0, 0.5, 0.5]);
    assert_eq!(rotating.constants[3], [0.0, 1.0, 2.0, 0.0]);
    assert_eq!(rotating.constants[4][0], 0.0);
    let pulse = animation::record_draw(&record(4, true), 0.5, 0.0, 1, [10, 10]).unwrap();
    assert_eq!(pulse.constants[3][0], 1.0);
    assert!(animation::record_draw(&record(4, false), 1.5, 0.0, 1, [10, 10]).is_none());
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().animated = true;
    resources
        .animations
        .insert("GFX_even", vec![record(2, true)]);
    let gui = gui("iconType = { name = \"a\" spriteType = \"GFX_even\" }");
    let window = gui.template("w").unwrap();
    let error = layout(window, None, &resources, &Bindings::default()).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::MissingInput);
    let mut bindings = Bindings::default();
    bindings.bind(&window.children[0]).animation_time = Some(0.25);
    let animated_layout = layout(window, None, &resources, &bindings)
        .unwrap()
        .unwrap();
    let PartKind::Animated(animated) = &animated_layout.parts[0].kind else {
        panic!("not animated");
    };
    assert_eq!(animated.records.len(), 1);
    assert_eq!((animated.records[0].mask, animated.technique), (21, "Up"));
}

#[test]
fn text_bytes_decode_as_the_game_converts_them() {
    assert_eq!(
        native_text::decode("aé€😀".as_bytes()),
        Some("aé€😀".chars().collect())
    );
    for bad in [
        &b"\x80"[..],
        b"a\xc3",
        b"\xed\xa0\x80",
        b"\xc0\x80",
        b"\xf5\x80\x80\x80",
    ] {
        assert_eq!(native_text::decode(bad), None, "{bad:?}");
    }
    assert_eq!(
        native_text::decode(b"\xed\x41\x80"),
        Some(vec!['\u{c040}', '\0'])
    );
    assert_eq!(
        native_text::decode(b"\xf4\x41\x80\x80"),
        Some(vec!['\u{c1000}', '\0'])
    );
}

#[test]
fn a_rejected_show_changes_nothing() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui(
        "size = { width = 20 height = 20 } fade_type = linear fade_time = 1000 \
         show_animation_type = linear show_animation_time = 100 \
         show_position = { x = 50 y = 0 } \
         containerWindowType = { name = \"c\" hide = yes size = { width = 5 height = 5 } \
         show_animation_type = linear show_animation_time = 100 \
         show_position = { x = 3 y = 0 } }",
    );
    let window = gui.template("w").unwrap();
    let child = window.find(Kind::Container, "c").unwrap();
    let mut instance = Instance::construct(window, [200, 100], &resources).unwrap();
    assert_eq!(instance.viewport(), [200, 100]);
    instance.set_hide_flag(window, window, true).unwrap();
    let before = (instance.drawn(), instance.opacity(), instance.position());
    instance.gui_show(window, window).unwrap();
    instance.show(window, window).unwrap();
    assert_eq!(
        (instance.drawn(), instance.opacity(), instance.position()),
        before
    );
    instance.tick(window, &resources, dt(0.05)).unwrap();
    assert_eq!(instance.position(), before.2);
    assert!(!instance.active());
    instance.set_hide_flag(window, window, false).unwrap();
    instance.gui_show(window, window).unwrap();
    assert!(instance.drawn());
    let bindings = instance.bindings(window, &resources).unwrap();
    let bound = bindings.at(Site::new(child, None)).unwrap();
    assert!(!bound.shown.unwrap().visible);
    assert_eq!(bound.position, Some([0, 0]));
}

#[test]
fn an_immediate_hide_and_a_show_reach_every_child() {
    content_reader_script::test_game::enter();
    let gui = gui("size = { width = 100 height = 100 } \
         iconType = { name = \"a\" spriteType = \"GFX_even\" } \
         iconType = { name = \"b\" spriteType = \"GFX_even\" hide = yes } \
         gridBoxType = { name = \"g\" }");
    let window = gui.template("w").unwrap();
    let a = window.find(Kind::Icon, "a").unwrap();
    let b = window.find(Kind::Icon, "b").unwrap();
    let resources = fixture();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.hide(window, a).unwrap();
    let shown = |instance: &Instance, element: &Element| {
        instance
            .bindings(window, &resources)
            .unwrap()
            .at(Site::new(element, None))
            .unwrap()
            .shown
            .unwrap()
            .visible
    };
    assert!(!shown(&instance, a));
    instance.hide(window, window).unwrap();
    assert!(!instance.drawn());
    assert_eq!(
        instance.take_held().last(),
        Some(&Held {
            path: "w.g".into(),
            shown: false
        })
    );
    instance.gui_show(window, window).unwrap();
    assert!(shown(&instance, a));
    assert!(!shown(&instance, b));
    assert_eq!(
        instance.take_held(),
        vec![Held {
            path: "w.g".into(),
            shown: true
        }]
    );
}

#[test]
fn a_looping_tile_pulses_on_the_instance() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let tile = |rate: f32| SpriteInfo {
        texture: [12, 12],
        kind: SpriteKind::CorneredTile {
            size: [30, 20],
            border: [4, 4],
            tiling_center: false,
            looping: true,
            rate: Some(rate),
        },
        ..plain([12, 12], 1)
    };
    resources.sprites.insert("GFX_pulse", tile(1.0));
    resources.sprites.insert("GFX_hang", tile(f32::INFINITY));
    let gui = gui("iconType = { name = \"t\" spriteType = \"GFX_pulse\" }");
    let window = gui.template("w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.25)).unwrap();
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(laid.parts[0].color[3], 0.5);
    assert!(instance.active());
    let hang = self::gui("iconType = { name = \"t\" spriteType = \"GFX_hang\" }");
    let window = hang.template("w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.25)).unwrap();
    let refused = instance.layout(window, &resources, None).unwrap_err();
    assert_eq!(refused[0].kind, RefusalKind::Hang);
}

#[test]
fn a_code_check_restarts_the_drawables_clock() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_four").unwrap().animated = true;
    resources.animations.insert(
        "GFX_four",
        vec![AnimationRecord {
            data: content_reader_gui::gui::native::sprites::AnimationData {
                kind: 4,
                looping: true,
                trigger_frames: 1 << 2,
                ..Default::default()
            },
            mask: 21,
            texture: 22,
            texture_size: [20, 20],
        }],
    );
    let gui = gui("checkboxType = { name = \"c\" spriteType = \"GFX_four\" }");
    let window = gui.template("w").unwrap();
    let check = window.find(Kind::Checkbox, "c").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    let bindings = instance.bindings(window, &resources).unwrap();
    assert_eq!(
        bindings.at(Site::new(check, None)).unwrap().triggers,
        Some(vec![-1.0])
    );
    instance.control(window, check).unwrap().checked = Some(true);
    instance.tick(window, &resources, dt(0.02)).unwrap();
    instance.tick(window, &resources, dt(0.01)).unwrap();
    let bindings = instance.bindings(window, &resources).unwrap();
    let bound = bindings.at(Site::new(check, None)).unwrap();
    assert!((bound.effect_since.unwrap() - 0.01).abs() < 1e-6);
    assert!((bound.triggers.as_ref().unwrap()[0] - 1.02).abs() < 1e-6);
    let laid = layout(window, None, &resources, &bindings)
        .unwrap()
        .unwrap();
    let PartKind::Animated(animated) = &laid.parts[0].kind else {
        panic!("not animated");
    };
    assert_eq!(animated.technique, "Down");
    assert_eq!(animated.records.len(), 1);
}

#[test]
fn a_pool_reuses_released_windows_with_their_state() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("size = { width = 10 height = 10 } \
         instantTextBoxType = { name = \"t\" font = \"Main_14\" maxWidth = 40 maxHeight = 20 }");
    let window = gui.template("w").unwrap();
    let text = window.find(Kind::TextBox, "t").unwrap();
    let mut pool = Pool::new(
        window,
        [800, 600],
        MapIconPool::Mission,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    let first = pool.request(0u32).unwrap();
    assert_eq!(pool.request(0), Some(first));
    let instance = pool.get(first).unwrap();
    instance.control(window, text).unwrap().text = Some("kept".into());
    instance.gui_show(window, window).unwrap();
    for owner in 1..64 {
        assert!(pool.request(owner).is_some());
    }
    assert_eq!(pool.live().count(), MapIconPool::Mission.initial());
    assert_eq!(pool.erase(window, &0).unwrap(), Vec::new());
    assert!(pool.get(first).is_none());
    assert_eq!(pool.slot_of(&0), None);
    let again = pool.request(100).unwrap();
    assert_eq!(again, first);
    let instance = pool.get(again).unwrap();
    assert!(!instance.drawn());
    assert_eq!(
        instance.control(window, text).unwrap().text.as_deref(),
        Some("kept")
    );
    assert!(pool.request(101).is_none());
    assert!(pool.active());
    assert!(pool.take_diagnostics().is_empty());
    pool.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(pool.growths(), 1);
    let grown = pool.take_diagnostics();
    assert_eq!(grown.len(), 1);
    let mut counts = Counts::default();
    counts.noted(&grown);
    assert_eq!(counts.0["map_icon_pool_growth"], 1);
    assert!(pool.take_diagnostics().is_empty());
    assert!(pool.request(101).is_some());
    assert_eq!(
        [
            MapIconPool::Base,
            MapIconPool::Combat,
            MapIconPool::CombatResults
        ]
        .map(MapIconPool::initial),
        [0xC0, 0x20, 0x40]
    );
}

#[test]
fn an_icon_leaves_and_reenters_the_viewport() {
    content_reader_script::test_game::enter();
    let gui = gui("size = { width = 10 height = 10 } \
         OverlappingElementsBoxType = { name = \"l\" size = { width = 10 height = 10 } }");
    let window = gui.template("w").unwrap();
    let resources = fixture();
    let mut pool = Pool::new(
        window,
        [800, 600],
        MapIconPool::Mission,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    let slot = pool.request(7u32).unwrap();
    let pass = pool
        .update(window, |_, _| Admission::Inside([30, 40]))
        .unwrap();
    assert_eq!(pass.updated, vec![(7, slot)]);
    let instance = pool.get(slot).unwrap();
    assert!(instance.drawn());
    assert_eq!(instance.position(), [30, 40]);
    instance.take_held();
    let pass = pool.update(window, |_, _| Admission::Outside).unwrap();
    assert!(pass.updated.is_empty());
    assert_eq!(pass.erased.len(), 1);
    assert_eq!((pass.erased[0].owner, pass.erased[0].slot), (7, slot));
    assert_eq!(
        pass.erased[0].held,
        vec![Held {
            path: "w.l".into(),
            shown: false
        }]
    );
    assert_eq!(pool.slot_of(&7), None);
    let next = pool.request(7).unwrap();
    assert_ne!(next, slot);
    assert_eq!(
        pool.update(window, |_, _| Admission::Outside)
            .unwrap()
            .erased
            .len(),
        1
    );
    let back = pool.request(7).unwrap();
    pool.update(window, |_, _| Admission::Inside([5, 6]))
        .unwrap();
    let instance = pool.get(back).unwrap();
    assert!(instance.drawn());
    assert_eq!(instance.position(), [5, 6]);
    let pass = pool.update(window, |_, _| Admission::Empty).unwrap();
    assert_eq!(pass.erased.len(), 1);
}

#[test]
fn a_freed_fading_window_reports_its_lists_when_the_fade_ends() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui(
        "size = { width = 10 height = 10 } fade_type = linear fade_time = 100 \
         OverlappingElementsBoxType = { name = \"l\" size = { width = 10 height = 10 } }",
    );
    let window = gui.template("w").unwrap();
    let mut pool = Pool::new(
        window,
        [800, 600],
        MapIconPool::Mission,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    let slot = pool.request(1u32).unwrap();
    pool.update(window, |_, _| Admission::Inside([0, 0]))
        .unwrap();
    pool.take_held(slot);
    for _ in 0..5 {
        pool.tick(window, &resources, dt(0.05)).unwrap();
    }
    assert!(pool.drain_held().is_empty());
    assert!(pool.erase(window, &1).unwrap().is_empty());
    assert!(pool.get(slot).is_none());
    let mut taken = Vec::new();
    for _ in 0..5 {
        pool.tick(window, &resources, dt(0.05)).unwrap();
        taken.extend(pool.drain_held());
    }
    assert_eq!(
        taken,
        vec![(
            slot,
            vec![Held {
                path: "w.l".into(),
                shown: false
            }]
        )]
    );
    assert!(pool.take_held(slot).is_empty());
}

#[test]
fn the_visible_list_holds_the_live_icons_whose_window_is_visible() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui(
        "size = { width = 10 height = 10 } fade_type = linear fade_time = 100 \
         OverlappingElementsBoxType = { name = \"l\" size = { width = 10 height = 10 } }",
    );
    let window = gui.template("w").unwrap();
    let mut pool = Pool::new(
        window,
        [800, 600],
        MapIconPool::Base,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    for owner in [1u32, 2, 3] {
        pool.request(owner).unwrap();
    }
    pool.update(window, |_, _| Admission::Inside([0, 0]))
        .unwrap();
    for _ in 0..5 {
        pool.tick(window, &resources, dt(0.05)).unwrap();
    }
    pool.drain_held();
    let visible = |pool: &Pool<u32>| {
        pool.visible()
            .map(|icon| {
                assert!(icon.instance.drawn());
                assert_eq!(pool.slot_of(icon.owner), Some(icon.slot));
                *icon.owner
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(visible(&pool), [1, 2, 3]);
    let erased = pool.slot_of(&1).unwrap();
    assert!(pool.erase(window, &1).unwrap().is_empty());
    let three = pool.slot_of(&3).unwrap();
    let instance = pool.get(three).unwrap();
    instance.hide(window, window).unwrap();
    pool.tick(window, &resources, dt(0.01)).unwrap();
    assert!(pool.active());
    assert_eq!(visible(&pool), [3, 2]);
    let mut taken = Vec::new();
    for _ in 0..5 {
        pool.tick(window, &resources, dt(0.05)).unwrap();
        taken.extend(pool.drain_held());
    }
    assert!(taken.iter().any(|(slot, _)| *slot == erased));
    assert_eq!(visible(&pool), [2]);
}

#[test]
fn a_refusal_stands_until_what_it_left_unknown_is_written_again() {
    content_reader_script::test_game::enter();
    let mut withheld = Withheld::default();
    withheld.refuse('a', Unknown::Update, "an operand".into());
    assert_eq!(withheld.why(&'a'), Some("an operand"));
    withheld.rewrote(&'a', Rewrote::Update);
    assert_eq!(withheld.why(&'a'), None);
    withheld.refuse('b', Unknown::Part("bar"), "a bar".into());
    withheld.refuse('b', Unknown::Update, "an operand".into());
    withheld.rewrote(&'b', Rewrote::Update);
    assert_eq!(withheld.why(&'b'), Some("an operand"));
    withheld.rewrote(&'b', Rewrote::Part("other"));
    assert!(withheld.why(&'b').is_some());
    withheld.rewrote(&'b', Rewrote::Part("bar"));
    assert_eq!(withheld.why(&'b'), None);
    withheld.refuse('c', Unknown::UntilRebuilt, "a call does not apply".into());
    withheld.rewrote(&'c', Rewrote::Update);
    withheld.rewrote(&'c', Rewrote::Part("bar"));
    assert_eq!(withheld.why(&'c'), Some("a call does not apply"));
    let resources = fixture();
    let gui = gui("size = { width = 10 height = 10 }");
    let window = gui.template("w").unwrap();
    let mut pool = Pool::new(
        window,
        [800, 600],
        MapIconPool::Combat,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    let slot = pool.request(7u32).unwrap();
    pool.update(window, |_, _| Admission::Inside([0, 0]))
        .unwrap();
    pool.refuse(slot, Unknown::Part("bar"), "a bar".into());
    let listed: Vec<_> = pool
        .visible()
        .map(|icon| (*icon.owner, icon.withheld))
        .collect();
    assert_eq!(listed, [(7, Some("a bar"))]);
    pool.erase(window, &7).unwrap();
    assert_eq!(pool.visible().count(), 0);
    assert_eq!(pool.withheld(slot), Some("a bar"));
    pool.rewrote(slot, Rewrote::Part("bar"));
    assert_eq!(pool.withheld(slot), None);
}

#[test]
fn the_pass_walks_the_live_array_as_it_stood() {
    content_reader_script::test_game::enter();
    let gui = gui("size = { width = 10 height = 10 }");
    let window = gui.template("w").unwrap();
    let resources = fixture();
    let mut pool = Pool::new(
        window,
        [800, 600],
        MapIconPool::Mission,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    for owner in ['a', 'b', 'c'] {
        pool.request(owner).unwrap();
    }
    let pass = pool
        .update(window, |owner, _| match owner {
            'a' => Admission::Outside,
            _ => Admission::Inside([0, 0]),
        })
        .unwrap();
    let updated: Vec<char> = pass.updated.iter().map(|(owner, _)| *owner).collect();
    assert_eq!(updated, vec!['b', 'c']);
    let live: Vec<char> = pool.live().map(|(owner, ..)| *owner).collect();
    assert_eq!(live, vec!['c', 'b']);
}

#[test]
fn a_pool_exhausts_while_its_icons_are_off_screen() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("size = { width = 10 height = 10 }");
    let window = gui.template("w").unwrap();
    let mut pool = Pool::new(
        window,
        [800, 600],
        MapIconPool::Combat,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    let owners = MapIconPool::Combat.initial() as u32 + 1;
    let frame = |pool: &mut Pool<u32>| {
        let granted = (0..owners)
            .filter(|owner| pool.request(*owner).is_some())
            .count();
        let pass = pool.update(window, |_, _| Admission::Outside).unwrap();
        assert!(pass.updated.is_empty());
        assert_eq!(pass.erased.len(), granted);
        pool.tick(window, &resources, dt(0.016)).unwrap();
        granted
    };
    assert_eq!(frame(&mut pool), MapIconPool::Combat.initial());
    assert_eq!(pool.take_diagnostics().len(), 1);
    assert_eq!(frame(&mut pool), owners as usize);
    assert!(pool.take_diagnostics().is_empty());
    assert_eq!(pool.live().count(), 0);
}

#[test]
fn a_window_takes_an_opacity_set_on_it() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui(
        "size = { width = 20 height = 20 } fade_type = linear fade_time = 1000 \
         iconType = { name = \"own\" spriteType = \"GFX_even\" } \
         containerWindowType = { name = \"c\" size = { width = 10 height = 10 } \
         iconType = { name = \"inner\" spriteType = \"GFX_even\" } }",
    );
    let window = gui.template("w").unwrap();
    let child = window.find(Kind::Container, "c").unwrap();
    let mut item = Instance::construct(window, [800, 600], &resources).unwrap();
    item.gui_show(window, window).unwrap();
    item.tick(window, &resources, dt(2.0)).unwrap();
    assert_eq!(item.opacity(), 1.0);
    item.set_opacity(window, window, 0.25, true).unwrap();
    let laid = item.layout(window, &resources, None).unwrap().unwrap();
    let alpha = |laid: &Layout, path: &str| {
        laid.parts
            .iter()
            .find(|part| part.path.ends_with(path))
            .map(|part| part.color[3])
    };
    assert_eq!(alpha(&laid, ".own"), Some(0.25));
    assert_eq!(alpha(&laid, ".inner"), Some(0.25));
    item.set_opacity(window, window, 0.5, false).unwrap();
    let laid = item.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(alpha(&laid, ".own"), Some(0.5));
    assert_eq!(alpha(&laid, ".inner"), Some(0.25));
    item.set_opacity(window, child, 0.75, true).unwrap();
    let laid = item.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(alpha(&laid, ".inner"), Some(0.75));
    let own = window.find(Kind::Icon, "own").unwrap();
    assert!(item.set_opacity(window, own, 0.5, true).is_err());
    let mut instances = HashMap::from([(1u8, item)]);
    let mut calls = Calls::default();
    calls.push(1, window, Call::SetOpacity(0.125, true));
    calls.apply(window, &mut instances, &resources).unwrap();
    assert_eq!(instances[&1].opacity(), 0.125);
}

#[test]
fn a_window_type_root_shows_and_hides_its_children() {
    content_reader_script::test_game::enter();
    let gui = read(
        b"guiTypes = { windowType = { name = \"f\" size = { width = 40 height = 20 } \
          iconType = { name = \"a\" spriteType = \"GFX_even\" } \
          iconType = { name = \"b\" spriteType = \"GFX_even\" hide = yes } \
          windowType = { name = \"n\" iconType = { name = \"c\" spriteType = \"GFX_even\" } } } }",
    );
    let window = gui.template("f").unwrap();
    assert_eq!(window.kind, Kind::Window);
    let a = window.find(Kind::Icon, "a").unwrap();
    let b = window.find(Kind::Icon, "b").unwrap();
    let nested = window.find(Kind::Window, "n").unwrap();
    let c = nested.find(Kind::Icon, "c").unwrap();
    let resources = fixture();
    let mut row = Instance::construct(window, [800, 600], &resources).unwrap();
    let shown = |row: &Instance, element: &Element| {
        row.bindings(window, &resources)
            .unwrap()
            .at(Site::new(element, None))
            .unwrap()
            .shown
            .unwrap()
            .visible
    };
    assert!(!row.drawn());
    assert!(!shown(&row, a));
    row.gui_show(window, window).unwrap();
    assert!(row.drawn());
    assert!(shown(&row, a) && !shown(&row, b));
    assert!(shown(&row, nested) && shown(&row, c));
    assert_eq!(row.opacity(), 1.0);
    row.hide(window, window).unwrap();
    assert!(!row.drawn() && !shown(&row, a) && !shown(&row, nested) && !shown(&row, c));
    row.set_hide_flag(window, window, true).unwrap();
    row.show(window, window).unwrap();
    assert!(!row.drawn() && !shown(&row, a));
}

#[test]
fn a_controls_show_reaches_its_parts() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui(
        "size = { width = 100 height = 100 } scrollbarType = { name = \"bar\" \
         position = { x = 2 y = 3 } size = { x = 10 y = 60 } maxValue = 4 startValue = 1 \
         minValue = 0 slider = \"slider\" track = \"track\" leftButton = \"down\" \
         rightButton = \"up\" buttonType = { name = \"up\" spriteType = \"GFX_four\" } \
         buttonType = { name = \"down\" spriteType = \"GFX_four\" } \
         guiButtonType = { name = \"track\" spriteType = \"GFX_bar3x17\" } \
         guiButtonType = { name = \"slider\" spriteType = \"GFX_bar3x17\" } }",
    );
    let window = gui.template("w").unwrap();
    let bar = window.find(Kind::Scrollbar, "bar").unwrap();
    let stateless = layout(window, None, &resources, &Bindings::default())
        .unwrap()
        .unwrap();
    assert_eq!(stateless.parts.len(), 4);
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    let parts = |instance: &Instance| {
        instance
            .layout(window, &resources, None)
            .unwrap()
            .map_or(Vec::new(), |laid| laid.parts)
    };
    assert_eq!(parts(&instance), stateless.parts);
    instance.hide(window, bar).unwrap();
    assert!(parts(&instance).is_empty());
    instance.show(window, bar).unwrap();
    assert_eq!(parts(&instance).len(), 4);
    instance.hide(window, window).unwrap();
    instance.gui_show(window, window).unwrap();
    assert_eq!(parts(&instance).len(), 4);
    let drop = read(
        b"guiTypes = { containerWindowType = { name = \"w\" \
          dropDownBoxType = { name = \"d\" position = { x = 10 y = 20 } \
          size = { width = 50% height = 40 } \
          expandButton = { name = \"b\" spriteType = \"GFX_even\" } \
          iconType = { name = \"header_icon\" spriteType = \"GFX_odd\" } } } }",
    );
    let window = drop.template("w").unwrap();
    let mut instance = Instance::construct(window, [200, 100], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    let bindings = Bindings {
        viewport: Some([200, 100]),
        ..Bindings::default()
    };
    let stateless = layout(window, Some([200, 100]), &resources, &bindings)
        .unwrap()
        .unwrap();
    assert_eq!(laid.parts, stateless.parts);
    assert_eq!(laid.parts.len(), 2);
}

fn sprites_at(layout: &Layout, path: &str) -> Vec<(Vec<u8>, i32, f32)> {
    layout
        .parts
        .iter()
        .filter(|part| part.path == path)
        .filter_map(|part| match &part.kind {
            PartKind::Sprite { sprite, .. } => {
                Some((sprite.clone(), frame_of(&part.kind), rect(&part.kind)[0]))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn inherited_scrollbar_parts_run_on_their_own_clocks() {
    content_reader_script::test_game::enter();
    let bar = |name: &str, x: i32| {
        format!(
            "scrollbarType = {{ name = \"{name}\" position = {{ x = {x} y = 3 }} \
             size = {{ x = 10 y = 60 }} borderSize = {{ x = 1 y = 1 }} minValue = -4 \
             maxValue = 4 startValue = 1 stepSize = 1 priority = 1 slider = \"slider\" \
             track = \"track\" leftButton = \"down\" rightButton = \"up\" \
             parent = \"base\" }}"
        )
    };
    let document = format!(
        "guiTypes = {{ scrollbarType = {{ name = \"base\" \
         buttonType = {{ name = \"up\" spriteType = \"GFX_animated\" }} \
         buttonType = {{ name = \"down\" spriteType = \"GFX_four\" }} \
         guiButtonType = {{ name = \"track\" spriteType = \"GFX_bar3x17\" }} \
         guiButtonType = {{ name = \"slider\" spriteType = \"GFX_bar3x17\" }} }} \
         containerWindowType = {{ name = \"w\" size = {{ width = 100 height = 100 }} \
         {} {} }} }}",
        bar("a", 2),
        bar("b", 40)
    );
    let resources = fixture_with(read(document.as_bytes()));
    let window = resources.gui_type(b"w").unwrap();
    let a = window.find(Kind::Scrollbar, "a").unwrap();
    let error = layout(window, None, &resources, &Bindings::default()).unwrap_err();
    assert_eq!(error[0].kind, RefusalKind::MissingInput);
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    let ups = |instance: &Instance| {
        let laid = instance.layout(window, &resources, None).unwrap();
        laid.map_or(Vec::new(), |laid| {
            sprites_at(&laid, "base.up")
                .into_iter()
                .map(|(_, frame, x)| (x, frame))
                .collect::<Vec<_>>()
        })
    };
    instance.tick(window, &resources, dt(1.0)).unwrap();
    instance.gui_show(window, window).unwrap();
    assert_eq!(ups(&instance), [(2.0, 0), (40.0, 0)]);
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert_eq!(ups(&instance), [(2.0, 1), (40.0, 1)]);
    assert!(instance.active());
    instance.hide(window, a).unwrap();
    instance.tick(window, &resources, dt(0.5)).unwrap();
    assert_eq!(ups(&instance), [(40.0, 2)]);
    instance.show(window, a).unwrap();
    instance.tick(window, &resources, dt(0.25)).unwrap();
    assert_eq!(ups(&instance), [(2.0, 1), (40.0, 2)]);
    instance.hide(window, window).unwrap();
    instance.tick(window, &resources, dt(5.0)).unwrap();
    assert_eq!(ups(&instance), []);
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(ups(&instance), [(2.0, 1), (40.0, 2)]);
}

#[test]
fn list_box_backgrounds_and_window_scrollbars_run_on_the_instance() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("size = { width = 100 height = 100 } \
         listBoxType = { name = \"l\" position = { x = 4 y = 6 } size = { x = 30 y = 25 } \
         background = \"GFX_animated\" }");
    let window = gui.template("w").unwrap();
    let list = window.find(Kind::ListBox, "l").unwrap();
    assert_eq!(
        layout(window, None, &resources, &Bindings::default()).unwrap_err()[0].kind,
        RefusalKind::MissingInput
    );
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    let background = |instance: &Instance| {
        instance
            .layout(window, &resources, None)
            .unwrap()
            .map_or(Vec::new(), |laid| sprites_at(&laid, "w.l"))
    };
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert_eq!(background(&instance), [(b"GFX_animated".to_vec(), 1, 4.0)]);
    instance.hide(window, list).unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    assert_eq!(background(&instance), []);
    instance.show(window, list).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(background(&instance), [(b"GFX_animated".to_vec(), 1, 4.0)]);
    let animated = SCROLLBAR.replace(
        "slider = { name = \"slider\" spriteType = \"GFX_bar3x17\" }",
        "slider = { name = \"slider\" spriteType = \"GFX_animated\" }",
    );
    let resources = fixture_with(read(
        format!(
            "guiTypes = {{ {animated} containerWindowType = {{ name = \"w\" \
             size = {{ width = 100 height = 80 }} verticalScrollbar = \"sb\" \
             autohide_scrollbars = no }} }}"
        )
        .as_bytes(),
    ));
    let window = resources.gui_type(b"w").unwrap();
    assert_eq!(
        layout(window, None, &resources, &Bindings::default()).unwrap_err()[0].kind,
        RefusalKind::MissingInput
    );
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    let slider = |instance: &Instance| {
        instance
            .layout(window, &resources, None)
            .unwrap()
            .map_or(Vec::new(), |laid| sprites_at(&laid, "sb.slider"))
    };
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert_eq!(slider(&instance)[0].1, 1);
    instance.hide(window, window).unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    assert_eq!(slider(&instance), []);
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(slider(&instance)[0].1, 1);
}

#[test]
fn a_fallback_sprite_runs_on_the_instance() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let animated = resources.sprites["GFX_animated"].clone();
    resources
        .sprites
        .insert("GFX_default_fallback_texture", animated);
    let gui = gui(
        "iconType = { name = \"i\" spriteType = \"GFX_undefined\" }              buttonType = { name = \"b\" font = \"Main_14\" buttonText = \"\"              position = { x = 30 y = 0 } }",
    );
    let window = gui.template("w").unwrap();
    assert_eq!(
        layout(window, None, &resources, &Bindings::default()).unwrap_err()[0].kind,
        RefusalKind::MissingInput
    );
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    let drawn = |instance: &Instance| {
        instance
            .layout(window, &resources, None)
            .unwrap()
            .map_or(Vec::new(), |laid| {
                let mut drawn = sprites_at(&laid, "w.i");
                drawn.extend(sprites_at(&laid, "w.b"));
                drawn
            })
    };
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    let fallback = |frame: i32| {
        vec![
            (FALLBACK_SPRITE.to_vec(), frame, 0.0),
            (FALLBACK_SPRITE.to_vec(), frame, 30.0),
        ]
    };
    assert_eq!(drawn(&instance), fallback(1));
    instance.hide(window, window).unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    assert_eq!(drawn(&instance), []);
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.5)).unwrap();
    assert_eq!(drawn(&instance), fallback(2));
}

#[test]
fn state_effects_run_on_the_graphics_clock_for_as_long_as_they_change() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("checkboxType = { name = \"c\" spriteType = \"GFX_four\" } \
         checkboxType = { name = \"n\" spriteType = \"GFX_no_down\" \
         position = { x = 20 y = 0 } }");
    let window = gui.template("w").unwrap();
    let check = window.find(Kind::Checkbox, "c").unwrap();
    let none = window.find(Kind::Checkbox, "n").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    assert!(!instance.active());
    instance.control(window, check).unwrap().checked = Some(true);
    let restart = dt(0.5);
    instance.tick(window, &resources, restart).unwrap();
    let bindings = instance.bindings(window, &resources).unwrap();
    assert_eq!(bindings.time, restart.time);
    assert_eq!(
        bindings.at(Site::new(check, None)).unwrap().effect_since,
        Some(0.0)
    );
    assert!(instance.active());
    let later = dt(0.05);
    instance.tick(window, &resources, later).unwrap();
    let bindings = instance.bindings(window, &resources).unwrap();
    assert_eq!(bindings.time, later.time);
    assert_eq!(
        bindings.at(Site::new(check, None)).unwrap().effect_since,
        Some(later.time - restart.time)
    );
    assert!(instance.active());
    instance.tick(window, &resources, dt(0.02)).unwrap();
    assert!(!instance.active());
    instance.control(window, none).unwrap().checked = Some(true);
    instance.tick(window, &resources, dt(0.01)).unwrap();
    assert!(instance.active());
    instance.tick(window, &resources, dt(0.07)).unwrap();
    assert!(!instance.active());
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    assert!(laid.parts.len() >= 2);
}

fn clocked_icon(effect: Effect, seconds: usize) -> (f32, Vec<(bool, f32, f32)>) {
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(effect);
    let tree = gui("iconType = { name = \"i\" spriteType = \"GFX_even\" }");
    let window = tree.template("w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let first = dt(0.0);
    instance.tick(window, &resources, first).unwrap();
    let steps = (0..seconds * 10)
        .map(|_| {
            instance.tick(window, &resources, dt(0.1)).unwrap();
            let laid = instance.layout(window, &resources, None).unwrap().unwrap();
            let part = laid.parts.iter().find(|part| part.path == "w.i").unwrap();
            (instance.active(), part.shading.time, part.shading.animation)
        })
        .collect();
    (first.time, steps)
}

#[test]
fn compiled_effects_keep_frames_coming_for_as_long_as_their_programs_change() {
    content_reader_script::test_game::enter();
    use shader_pipeline::clock::{Bounded, Clock};
    let effect = |pixel| clocked_effect("Up", pixel);
    let (first, steps) = clocked_icon(
        effect(Clock {
            direct: vec![(1, 96, 100)],
            bounded: Vec::new(),
        }),
        1,
    );
    assert_eq!(steps.len(), 10);
    let mut before = first;
    for &(active, time, animation) in &steps {
        assert!(active, "{time}");
        assert!(time > before);
        assert_eq!(animation, first);
        before = time;
    }
    assert!(before - first > 0.25);
    let fading = Clock {
        direct: Vec::new(),
        bounded: vec![Bounded {
            terms: vec![((1, 96), 2.0), ((1, 100), -2.0)],
            constant: 0.0,
            low: Some(0.0),
            high: Some(1.0),
            open_low: false,
            slack: 0.0,
            per: Vec::new(),
            reach: 0.0,
            peak: 0.0,
        }],
    };
    let (_, steps) = clocked_icon(effect(fading.clone()), 1);
    let active: Vec<bool> = steps.iter().map(|step| step.0).collect();
    assert!(active[..4].iter().all(|active| *active), "{active:?}");
    assert!(!active[5..].iter().any(|active| *active), "{active:?}");
    let mut overflowing = fading;
    overflowing.bounded[0].peak = 1e38;
    let (_, steps) = clocked_icon(effect(overflowing), 1);
    assert!(steps.iter().all(|step| step.0), "{steps:?}");
    let (_, steps) = clocked_icon(
        effect(Clock {
            direct: vec![(1, 100, 104)],
            bounded: Vec::new(),
        }),
        1,
    );
    assert!(!steps.iter().any(|step| step.0));
}

#[test]
fn a_descending_step_keeps_its_instance_active_at_its_edge() {
    content_reader_script::test_game::enter();
    use shader_pipeline::clock::{Bounded, Clock};
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(clocked_effect(
        "Up",
        Clock {
            direct: Vec::new(),
            bounded: vec![Bounded {
                terms: vec![((1, 96), -1.0), ((1, 100), 1.0)],
                constant: 0.5,
                low: Some(0.0),
                high: Some(0.0),
                open_low: true,
                slack: 0.0,
                per: Vec::new(),
                reach: 0.0,
                peak: 0.0,
            }],
        },
    ));
    let tree = gui("iconType = { name = \"i\" spriteType = \"GFX_even\" }");
    let window = tree.template("w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    let first = dt(0.0);
    instance.tick(window, &resources, first).unwrap();
    assert!(instance.active());
    let edge = dt(0.5);
    instance.tick(window, &resources, edge).unwrap();
    assert_eq!(edge.time - first.time, 0.5);
    assert!(instance.active(), "still 1 at the edge");
    instance.tick(window, &resources, dt(0.25)).unwrap();
    assert!(!instance.active());
}

#[test]
#[ignore = "requires the shader compiler"]
fn compiled_programs_reading_time_keep_their_instance_active() {
    content_reader_script::test_game::enter();
    use shader_pipeline::{self as shader, library};
    use std::sync::Arc;
    let effect = |pixel: &str| {
        let source = format!(
            "VertexStruct VS_INPUT {{ float3 vPosition : POSITION;\nfloat2 vTexCoord : TEXCOORD0;\n}};\n\
             VertexStruct VS_OUTPUT {{ float4 vPosition : PDX_POSITION;\nfloat2 vTexCoord : TEXCOORD0;\n}};\n\
             ConstantBuffer( 0, 0 ) {{ float4x4 WorldViewProjectionMatrix;\nfloat4 Color;\nfloat2 Offset;\nfloat2 NextOffset;\nfloat Time;\nfloat AnimationTime;\n}};\n\
             VertexShader = {{ MainCode V [[ VS_OUTPUT main(const VS_INPUT v) {{ VS_OUTPUT Out; Out.vPosition = mul(WorldViewProjectionMatrix, float4(v.vPosition.xyz, 1)); Out.vTexCoord = v.vTexCoord + Offset; return Out; }} ]] }}\n\
             PixelShader = {{ MainCode P [[ float4 main(VS_OUTPUT v) : PDX_COLOR {{ return {pixel}; }} ]] }}\n\
             Effect Up {{ VertexShader = V PixelShader = P }}\n"
        );
        let program = content_reader_gui::pdxshader::program::read_with(b"gfx/FX/mod.lua", &|path| {
            Some(if path.ends_with(b".shader") {
                source.as_bytes().to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
        let compiled = shader::Compiler::discover()
            .unwrap()
            .compile(&program, &program.techniques[0], &[])
            .unwrap();
        Effect {
            file: "gfx/FX/mod.lua".into(),
            techniques: Vec::new(),
            samplers: [None; 6],
            compiled: Some(library::Effect::from_compiled(
                Arc::new(program),
                Vec::new(),
                vec![Ok(Arc::new(compiled))],
            )),
        }
    };
    let (first, steps) = clocked_icon(effect("Color * frac(Time)"), 1);
    assert!(steps.iter().all(|step| step.0));
    assert!(steps.last().unwrap().1 - first > 0.25);
    assert!(steps.iter().all(|step| step.2 == first));
    let (_, steps) = clocked_icon(effect("Color * saturate((Time - AnimationTime) * 2)"), 1);
    let active: Vec<bool> = steps.iter().map(|step| step.0).collect();
    assert!(active[..4].iter().all(|active| *active), "{active:?}");
    assert!(!active[5..].iter().any(|active| *active), "{active:?}");
}

#[test]
fn a_missing_technique_changes_for_as_long_as_the_previous_program() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let tree = gui("checkboxType = { name = \"c\" spriteType = \"GFX_four\" } \
         checkboxType = { name = \"n\" spriteType = \"GFX_no_down\" \
         position = { x = 20 y = 0 } }");
    let window = tree.template("w").unwrap();
    let check = window.find(Kind::Checkbox, "c").unwrap();
    let kept = window.find(Kind::Checkbox, "n").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.control(window, check).unwrap().checked = Some(true);
    instance.control(window, kept).unwrap().checked = Some(true);
    instance.hide(window, kept).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert!(!instance.active());
    instance.show(window, kept).unwrap();
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert!(instance.active());
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    let mut batch = Batch::default();
    draw(&laid, [0, 0], &one(), &mut batch).unwrap();
    let [first, second] = [0, 1].map(|index| batch.quads[index].exact.clone().unwrap());
    assert_eq!(laid.parts[1].shading.fixture, None);
    assert_eq!(second.technique, first.technique);
    assert_eq!(second.samplers, first.samplers);
    assert_eq!(second.blend, Blend::Off);
    assert!((first.inputs[0] - 0.2).abs() < 1e-5, "{:?}", first.inputs);
    assert_eq!(second.inputs[0], 0.0);
    instance.tick(window, &resources, dt(0.05)).unwrap();
    assert!(instance.active());
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    assert!((laid.parts[1].shading.since - 0.05).abs() < 1e-5);
    instance.tick(window, &resources, dt(0.02)).unwrap();
    assert!(!instance.active());
    let alone = gui("checkboxType = { name = \"n\" spriteType = \"GFX_no_down\" }");
    let window = alone.template("w").unwrap();
    let kept = window.find(Kind::Checkbox, "n").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.control(window, kept).unwrap().checked = Some(true);
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    assert!(instance.active());
}

fn clocked_effect(techniques: &str, clock: shader_pipeline::clock::Clock) -> Effect {
    use shader_pipeline::{Compiled, Shader, clock::Clock, library};
    use std::sync::Arc;
    let source: String = techniques
        .split(' ')
        .map(|name| format!("Effect {name} {{ VertexShader = V PixelShader = P }}\n"))
        .collect();
    let program = content_reader_gui::pdxshader::program::read_with(b"gfx/FX/mod.lua", &|path| {
        Some(if path.ends_with(b".shader") {
            source.as_bytes().to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let shader = |clock| Shader {
        wgsl: String::new(),
        independent_draws: false,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock,
    };
    let compiled = program
        .techniques
        .iter()
        .map(|_| {
            Ok(Arc::new(Compiled {
                vertex: shader(Clock::default()),
                pixel: shader(clock.clone()),
            }))
        })
        .collect();
    Effect {
        file: "gfx/FX/mod.lua".into(),
        techniques: Vec::new(),
        samplers: [None; 6],
        compiled: Some(library::Effect::from_compiled(
            Arc::new(program),
            Vec::new(),
            compiled,
        )),
    }
}

#[test]
fn a_missing_compiled_technique_runs_on_the_previous_program() {
    content_reader_script::test_game::enter();
    use shader_pipeline::clock::{Bounded, Clock};
    let run = |text: Clock| {
        let mut resources = fixture();
        resources.text = Some(clocked_effect("Text", text));
        resources.sprites.get_mut("GFX_even").unwrap().effect =
            Ok(clocked_effect("Down", Clock::default()));
        let tree = gui(
            "instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"A\" } \
             iconType = { name = \"k\" spriteType = \"GFX_even\" position = { x = 40 y = 0 } }",
        );
        let window = tree.template("w").unwrap();
        let kept = window.find(Kind::Icon, "k").unwrap();
        let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
        instance.gui_show(window, window).unwrap();
        instance.hide(window, kept).unwrap();
        instance.tick(window, &resources, dt(0.0)).unwrap();
        instance.tick(window, &resources, dt(1.0)).unwrap();
        let before = instance.active();
        instance.show(window, kept).unwrap();
        let shown = dt(0.1);
        instance.tick(window, &resources, shown).unwrap();
        let laid = instance.layout(window, &resources, None).unwrap().unwrap();
        let index = laid
            .parts
            .iter()
            .position(|part| part.path == "w.k")
            .unwrap();
        assert!(index > 0);
        assert!(matches!(laid.parts[index - 1].kind, PartKind::Text { .. }));
        let mut batch = Batch::default();
        draw(&laid, [0, 0], &one(), &mut batch).unwrap();
        let native =
            |quad: &crate::sprites::Quad| quad.exact.as_ref().unwrap().native.clone().unwrap();
        let last = native(batch.quads.last().unwrap());
        assert_eq!(last.path, "w.k");
        assert!(last.effect.asset.source.technique("Up").is_none());
        let text = native(&batch.quads[batch.quads.len() - 2]);
        assert_eq!(text.effect.technique, "Text");
        assert_eq!(last.constants[24..26], [shown.time, shown.time]);
        let mut active = vec![instance.active()];
        for _ in 0..10 {
            instance.tick(window, &resources, dt(0.1)).unwrap();
            active.push(instance.active());
        }
        (before, active)
    };
    let (before, active) = run(Clock {
        direct: vec![(1, 96, 100)],
        bounded: Vec::new(),
    });
    assert!(!before);
    assert!(active.iter().all(|active| *active), "{active:?}");
    let (before, active) = run(Clock {
        direct: Vec::new(),
        bounded: vec![Bounded {
            terms: vec![((1, 96), 2.0), ((1, 100), -2.0)],
            constant: 0.0,
            low: Some(0.0),
            high: Some(1.0),
            open_low: false,
            slack: 0.0,
            per: Vec::new(),
            reach: 0.0,
            peak: 0.0,
        }],
    });
    assert!(!before);
    assert!(active[..5].iter().all(|active| *active), "{active:?}");
    assert!(!active[6..].iter().any(|active| *active), "{active:?}");
}

#[test]
fn a_kept_draw_after_a_refused_one_does_not_keep_frames_coming() {
    content_reader_script::test_game::enter();
    use shader_pipeline::{clock::Clock, library};
    let mut resources = fixture();
    resources.text = Some(clocked_effect(
        "Text",
        Clock {
            direct: vec![(1, 96, 100)],
            bounded: Vec::new(),
        },
    ));
    let mut refused = clocked_effect("Up", Clock::default());
    let asset = refused.compiled.take().unwrap();
    refused.compiled = Some(library::Effect::from_compiled(
        asset.source.clone(),
        asset.defines.clone(),
        vec![Err("deliberate compiler diagnostic".into())],
    ));
    resources.sprites.get_mut("GFX_odd").unwrap().effect = Ok(refused);
    resources.sprites.get_mut("GFX_even").unwrap().effect =
        Ok(clocked_effect("Down", Clock::default()));
    let tree = gui(
        "instantTextBoxType = { name = \"t\" font = \"Main_14\" text = \"A\" } \
         iconType = { name = \"r\" spriteType = \"GFX_odd\" position = { x = 40 y = 0 } } \
         iconType = { name = \"k\" spriteType = \"GFX_even\" position = { x = 80 y = 0 } }",
    );
    let window = tree.template("w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    let at = |path: &str| {
        laid.parts
            .iter()
            .position(|part| part.path == path)
            .unwrap()
    };
    let (text, r, k) = (at("w.t"), at("w.r"), at("w.k"));
    assert!(text < r && k == r + 1);
    assert_eq!(laid.parts[r].shading.settles(), Settles::Refused);
    assert_eq!(laid.parts[k].shading.settles(), Settles::Previous);
    for _ in 0..3 {
        instance.tick(window, &resources, dt(0.1)).unwrap();
        assert!(!instance.active());
    }
}

#[test]
fn a_failed_replacement_keeps_the_retained_drawable() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("iconType = { name = \"type\" spriteType = \"GFX_even\" }");
    let window = gui.template("w").unwrap();
    let hull = window.find(Kind::Icon, "type").unwrap();
    let mut instances = HashMap::from([(
        (),
        Instance::construct(window, [800, 600], &resources).unwrap(),
    )]);
    let drawn = |instance: &Instance| {
        let laid = instance.layout(window, &resources, None).unwrap().unwrap();
        let extent = instance.drawable_extent(window, hull, &resources).unwrap();
        (sprites_at(&laid, "w.type"), extent)
    };
    let mut calls = Calls::default();
    calls.push((), window, Call::GuiShow);
    calls.push(
        (),
        hull,
        Call::SetSprite(b"GFX_odd".to_vec(), SpriteSetter::Button),
    );
    calls.push((), hull, Call::SetFrame(1));
    calls.apply(window, &mut instances, &resources).unwrap();
    let instance = instances.get_mut(&()).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(
        drawn(instance),
        (vec![(b"GFX_odd".to_vec(), 0, 0.0)], Some([15, 9]))
    );
    let mut calls = Calls::default();
    calls.push(
        (),
        hull,
        Call::SetSprite(b"GFX_no_intel".to_vec(), SpriteSetter::Icon),
    );
    calls.push((), hull, Call::SetFrame(2));
    calls.apply(window, &mut instances, &resources).unwrap();
    let instance = instances.get_mut(&()).unwrap();
    let kept = (vec![(b"GFX_odd".to_vec(), 1, 0.0)], Some([15, 9]));
    assert_eq!(drawn(instance), kept);
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert_eq!(drawn(instance), kept);
    let control = instance.control(window, hull).unwrap();
    control.sprite = Some(b"GFX_odd".to_vec());
    control.setter = SpriteSetter::Button;
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert_eq!(drawn(instance), kept);
    instance
        .set_sprite(window, hull, b"GFX_four", SpriteSetter::Icon)
        .unwrap();
    instance
        .set_sprite(window, hull, b"GFX_undefined", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert_eq!(
        drawn(instance),
        (vec![(b"GFX_four".to_vec(), 0, 0.0)], Some([10, 10]))
    );
    instance
        .set_sprite(window, hull, b"GFX_animated", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    instance
        .set_sprite(window, hull, b"GFX_undefined", SpriteSetter::Icon)
        .unwrap();
    instance
        .set_sprite(window, hull, b"GFX_animated", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(drawn(instance).0, [(b"GFX_animated".to_vec(), 2, 0.0)]);
}

#[test]
fn a_drop_down_headers_fade_runs_on_the_instance() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let drop = read(
        b"guiTypes = { containerWindowType = { name = \"w\" \
          size = { width = 200 height = 100 } \
          dropDownBoxType = { name = \"d\" position = { x = 10 y = 20 } \
          size = { width = 50 height = 40 } fade_type = linear fade_time = 1000 \
          expandButton = { name = \"b\" spriteType = \"GFX_even\" } \
          containerWindowType = { name = \"c\" size = { width = 20 height = 20 } \
          fade_type = linear fade_time = 1000 \
          iconType = { name = \"inner\" spriteType = \"GFX_even\" } } } } }",
    );
    let window = drop.template("w").unwrap();
    let native::Fields::DropDown(down) = &window.children[0].fields else {
        panic!("not a drop-down");
    };
    let header = &*down.inner;
    let nested = header.find(Kind::Container, "c").unwrap();
    let mut instance = Instance::construct(window, [200, 100], &resources).unwrap();
    let opacity = |instance: &Instance, element: &Element| {
        instance
            .bindings(window, &resources)
            .unwrap()
            .at(Site::new(element, None))
            .and_then(|bound| bound.opacity)
            .unwrap()
    };
    assert_eq!(opacity(&instance, header), 0.0);
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.5)).unwrap();
    assert!((opacity(&instance, header) - 0.5).abs() < 1e-6);
    assert_eq!(opacity(&instance, nested), 1.0);
    assert!(instance.active());
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert_eq!(opacity(&instance, header), 1.0);
    assert_eq!(opacity(&instance, nested), 1.0);
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    let inner = laid
        .parts
        .iter()
        .find(|part| part.path.ends_with(".inner"))
        .unwrap();
    assert_eq!(inner.color[3], 1.0);
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert!(!instance.active());
}

#[test]
fn a_replaced_drawables_compiled_effect_restarts_its_clock() {
    content_reader_script::test_game::enter();
    use shader_pipeline::clock::{Bounded, Clock};
    let fading = || {
        clocked_effect(
            "Up",
            Clock {
                direct: Vec::new(),
                bounded: vec![Bounded {
                    terms: vec![((1, 96), 2.0), ((1, 100), -2.0)],
                    constant: 0.0,
                    low: Some(0.0),
                    high: Some(1.0),
                    open_low: false,
                    slack: 0.0,
                    per: Vec::new(),
                    reach: 0.0,
                    peak: 0.0,
                }],
            },
        )
    };
    let mut resources = fixture();
    resources.sprites.get_mut("GFX_even").unwrap().effect = Ok(fading());
    resources.sprites.get_mut("GFX_odd").unwrap().effect = Ok(fading());
    let tree = gui("iconType = { name = \"i\" spriteType = \"GFX_even\" }");
    let window = tree.template("w").unwrap();
    let icon = window.find(Kind::Icon, "i").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    let clocks = |instance: &Instance| {
        let laid = instance.layout(window, &resources, None).unwrap().unwrap();
        let mut batch = Batch::default();
        draw(&laid, [0, 0], &one(), &mut batch).unwrap();
        let native = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .clone()
            .unwrap();
        [native.constants[24], native.constants[25]]
    };
    instance.gui_show(window, window).unwrap();
    let first = dt(0.0);
    instance.tick(window, &resources, first).unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    assert!(!instance.active());
    instance
        .set_sprite(window, icon, b"GFX_even", SpriteSetter::Icon)
        .unwrap();
    let kept = dt(0.1);
    instance.tick(window, &resources, kept).unwrap();
    assert!(!instance.active());
    assert_eq!(clocks(&instance), [kept.time, first.time]);
    instance
        .set_sprite(window, icon, b"GFX_odd", SpriteSetter::Icon)
        .unwrap();
    let replaced = dt(0.1);
    instance.tick(window, &resources, replaced).unwrap();
    assert!(instance.active());
    assert_eq!(clocks(&instance), [replaced.time, replaced.time]);
    instance.tick(window, &resources, dt(0.25)).unwrap();
    assert!(instance.active());
    instance.tick(window, &resources, dt(0.3)).unwrap();
    assert!(!instance.active());
}

#[test]
fn a_replaced_drawable_starts_its_own_history() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let record = |trigger_frames: u32| AnimationRecord {
        data: content_reader_gui::gui::native::sprites::AnimationData {
            kind: 4,
            looping: true,
            trigger_frames,
            ..Default::default()
        },
        mask: 21,
        texture: 22,
        texture_size: [20, 20],
    };
    resources.sprites.get_mut("GFX_four").unwrap().animated = true;
    resources
        .animations
        .insert("GFX_four", vec![record(1 << 2)]);
    resources.sprites.get_mut("GFX_odd").unwrap().animated = true;
    resources.animations.insert("GFX_odd", vec![record(1 << 1)]);
    let gui = gui("iconType = { name = \"i\" spriteType = \"GFX_four\" }");
    let window = gui.template("w").unwrap();
    let icon = window.find(Kind::Icon, "i").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    let history = |instance: &Instance, age: f32, triggers: Option<Vec<f32>>| {
        let bindings = instance.bindings(window, &resources).unwrap();
        let bound = bindings.at(Site::new(icon, None)).unwrap();
        assert!((bound.animation_time.unwrap() - age).abs() < 1e-3);
        assert_eq!(bound.triggers, triggers);
    };
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.tick(window, &resources, dt(10.0)).unwrap();
    history(&instance, 10.0, Some(vec![-1.0]));
    instance
        .set_sprite(window, icon, b"GFX_undefined", SpriteSetter::Icon)
        .unwrap();
    instance
        .set_sprite(window, icon, b"GFX_four", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(1.0)).unwrap();
    history(&instance, 11.0, Some(vec![-1.0]));
    instance
        .set_sprite(window, icon, b"GFX_odd", SpriteSetter::Icon)
        .unwrap();
    history(&instance, 0.0, None);
    instance.tick(window, &resources, dt(1.0)).unwrap();
    history(&instance, 0.0, Some(vec![0.0]));
    instance.tick(window, &resources, dt(2.0)).unwrap();
    history(&instance, 2.0, Some(vec![0.0]));

    let tile = |rate: f32| SpriteInfo {
        texture: [12, 12],
        kind: SpriteKind::CorneredTile {
            size: [30, 20],
            border: [4, 4],
            tiling_center: false,
            looping: true,
            rate: Some(rate),
        },
        ..plain([12, 12], 1)
    };
    resources.sprites.insert("GFX_pulse", tile(1.0));
    resources.sprites.insert("GFX_pulse_b", tile(1.0));
    resources.sprites.insert("GFX_hang", tile(f32::INFINITY));
    let gui = self::gui("iconType = { name = \"t\" spriteType = \"GFX_pulse\" }");
    let window = gui.template("w").unwrap();
    let cornered = window.find(Kind::Icon, "t").unwrap();
    let alpha = |instance: &Instance| {
        instance
            .layout(window, &resources, None)
            .map(|laid| laid.unwrap().parts[0].color[3])
    };
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.25)).unwrap();
    assert_eq!(alpha(&instance).unwrap(), 0.5);
    instance
        .set_sprite(window, cornered, b"GFX_pulse_b", SpriteSetter::Icon)
        .unwrap();
    assert_eq!(alpha(&instance).unwrap(), 0.0);
    instance.tick(window, &resources, dt(0.125)).unwrap();
    assert_eq!(alpha(&instance).unwrap(), 0.25);
    instance
        .set_sprite(window, cornered, b"GFX_hang", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert_eq!(alpha(&instance).unwrap_err()[0].kind, RefusalKind::Hang);
    instance
        .set_sprite(window, cornered, b"GFX_pulse", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.125)).unwrap();
    assert_eq!(alpha(&instance).unwrap(), 0.25);
}

#[test]
fn a_play_reaches_the_drawable_its_earlier_calls_left() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let still = SpriteInfo {
        kind: SpriteKind::FrameAnimated {
            seconds_per_frame: 0.5,
            looping: true,
            play_on_show: false,
            pause: 0.0,
        },
        ..plain([10, 4], 4)
    };
    resources.sprites.insert("GFX_still", still.clone());
    resources.sprites.insert("GFX_still_b", still);
    let gui = gui("iconType = { name = \"a\" spriteType = \"GFX_still\" }");
    let window = gui.template("w").unwrap();
    let icon = window.find(Kind::Icon, "a").unwrap();
    let shown = || {
        let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
        instance.gui_show(window, window).unwrap();
        instance
    };
    let elapsed = |instance: &Instance| {
        instance
            .bindings(window, &resources)
            .unwrap()
            .at(Site::new(icon, None))
            .and_then(|bound| bound.clock)
            .map(|(_, elapsed)| elapsed)
            .unwrap()
    };
    let mut instance = shown();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance
        .set_sprite(window, icon, b"GFX_still_b", SpriteSetter::Icon)
        .unwrap();
    instance.play(window, icon).unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert!((elapsed(&instance) - 0.6).abs() < 1e-6);
    let mut instance = shown();
    instance.play(window, icon).unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert!((elapsed(&instance) - 0.6).abs() < 1e-6);
    instance
        .set_sprite(window, icon, b"GFX_still_b", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert_eq!(elapsed(&instance), 0.0);
    assert!(!instance.active());
    let mut instance = shown();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.play(window, icon).unwrap();
    instance
        .set_sprite(window, icon, b"GFX_still_b", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert_eq!(elapsed(&instance), 0.0);
    let mut instance = shown();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    instance.play(window, icon).unwrap();
    instance
        .set_sprite(window, icon, b"GFX_undefined", SpriteSetter::Icon)
        .unwrap();
    instance.tick(window, &resources, dt(0.6)).unwrap();
    assert!((elapsed(&instance) - 0.6).abs() < 1e-6);
}

#[test]
fn a_compiled_frame_animation_plays_only_the_drawable_play_reached() {
    content_reader_script::test_game::enter();
    use shader_pipeline::clock::Clock;
    let mut resources = fixture();
    let still = SpriteInfo {
        kind: SpriteKind::FrameAnimated {
            seconds_per_frame: 0.5,
            looping: true,
            play_on_show: false,
            pause: 0.0,
        },
        effect: Ok(clocked_effect("Up", Clock::default())),
        ..plain([10, 4], 4)
    };
    resources.sprites.insert("GFX_still", still.clone());
    resources.sprites.insert("GFX_still_b", still);
    let gui = gui("iconType = { name = \"a\" spriteType = \"GFX_still\" }");
    let window = gui.template("w").unwrap();
    let icon = window.find(Kind::Icon, "a").unwrap();
    let animation_time = |instance: &Instance| {
        let laid = instance.layout(window, &resources, None).unwrap().unwrap();
        let mut batch = Batch::default();
        draw(&laid, [0, 0], &one(), &mut batch).unwrap();
        let native = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .clone()
            .unwrap();
        native.constants[25]
    };
    let run = |play_first: bool| {
        let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
        instance.gui_show(window, window).unwrap();
        instance.tick(window, &resources, dt(0.0)).unwrap();
        if play_first {
            instance.play(window, icon).unwrap();
        }
        instance
            .set_sprite(window, icon, b"GFX_still_b", SpriteSetter::Icon)
            .unwrap();
        if !play_first {
            instance.play(window, icon).unwrap();
        }
        instance.tick(window, &resources, dt(0.6)).unwrap();
        let first = (animation_time(&instance), instance.active());
        instance.tick(window, &resources, dt(0.6)).unwrap();
        (first, (animation_time(&instance), instance.active()))
    };
    let ((first, active), (then, still_active)) = run(false);
    assert!(active && still_active);
    assert_ne!(first, then);
    let ((first, active), (then, still_active)) = run(true);
    assert!(!active && !still_active);
    assert_eq!(first, then);
}

#[test]
fn a_frame_animation_whose_wrap_never_ends_is_a_hang() {
    content_reader_script::test_game::enter();
    let animated = |pause: f32| SpriteKind::FrameAnimated {
        seconds_per_frame: 0.25,
        looping: true,
        play_on_show: true,
        pause,
    };
    let wraps = |pause: f32, dt: f32| {
        let mut clock = lifecycle::FrameClock::new();
        clock
            .update(&animated(pause), 4, dt)
            .map(|()| clock.frame())
    };
    assert_eq!(wraps(0.0, 2.5), Ok(Some(3)));
    assert_eq!(wraps(0.5, 2.0), Ok(Some(3)));
    assert!(wraps(-1.0, 0.1).is_err());
    assert!(wraps(-2.0, 0.1).is_err());
    assert!(wraps(0.0, 1e9).is_err());
    let mut resources = fixture();
    resources.sprites.insert(
        "GFX_zero_period",
        SpriteInfo {
            kind: animated(-1.0),
            ..plain([10, 4], 4)
        },
    );
    let gui = gui("iconType = { name = \"a\" spriteType = \"GFX_zero_period\" }");
    let window = gui.template("w").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.tick(window, &resources, dt(0.1)).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert!(instance.layout(window, &resources, None).is_ok());
    instance.tick(window, &resources, dt(0.1)).unwrap();
    let refused = instance.layout(window, &resources, None).unwrap_err();
    assert_eq!(refused[0].kind, RefusalKind::Hang);
    assert!(refused[0].detail.contains("wrap period"));
    instance.tick(window, &resources, dt(0.1)).unwrap();
    assert_eq!(
        instance.layout(window, &resources, None).unwrap_err()[0].kind,
        RefusalKind::Hang
    );
}

#[test]
fn several_instances_tick_in_the_games_phase_order() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let icons = gui(
        "size = { width = 20 height = 20 } fade_type = linear fade_time = 1000 \
         gridBoxType = { name = \"g\" }",
    );
    let root = icons.template("w").unwrap();
    let items = gui("size = { width = 10 height = 10 } \
         iconType = { name = \"a\" spriteType = \"GFX_animated\" }");
    let item = items.template("w").unwrap();
    let elapsed = |instance: &Instance| {
        instance
            .bindings(item, &resources)
            .unwrap()
            .at(Site::new(&item.children[0], None))
            .and_then(|bound| bound.clock)
            .map(|(_, elapsed)| elapsed)
            .unwrap()
    };
    let run = |phased: bool| {
        let mut icon = Instance::construct(root, [800, 600], &resources).unwrap();
        let mut entry = Instance::construct(item, [800, 600], &resources).unwrap();
        assert!(entry.registration() > icon.registration());
        let reach = |icon: &mut Instance, entry: &mut Instance| {
            for held in icon.take_held() {
                if held.shown {
                    entry.gui_show(item, item).unwrap();
                } else {
                    entry.hide(item, item).unwrap();
                }
            }
        };
        icon.gui_show(root, root).unwrap();
        reach(&mut icon, &mut entry);
        for (step, hide) in [(0.0, false), (0.5, false), (0.6, true), (0.3, false)] {
            if hide {
                icon.hide(root, root).unwrap();
            }
            let tick = dt(step);
            if phased {
                icon.tick_clocks(root, &resources, tick).unwrap();
                entry.tick_clocks(item, &resources, tick).unwrap();
                entry.tick_transitions(item).unwrap();
                icon.tick_transitions(root).unwrap();
                reach(&mut icon, &mut entry);
                icon.tick_draws(root, &resources).unwrap();
                entry.tick_draws(item, &resources).unwrap();
            } else {
                icon.tick(root, &resources, tick).unwrap();
                reach(&mut icon, &mut entry);
                entry.tick(item, &resources, tick).unwrap();
            }
        }
        assert!(!icon.drawn());
        assert!(!entry.drawn());
        elapsed(&entry)
    };
    assert!((run(true) - 1.1).abs() < 1e-5);
    assert!((run(false) - 0.5).abs() < 1e-5);
    let mut instance = Instance::construct(item, [800, 600], &resources).unwrap();
    assert!(instance.tick_transitions(item).is_err());
    assert!(instance.tick_draws(item, &resources).is_err());
    instance.tick_clocks(item, &resources, dt(0.0)).unwrap();
    assert!(instance.tick_draws(item, &resources).is_err());
    instance.tick_transitions(item).unwrap();
    assert!(instance.tick_transitions(item).is_err());
    instance.tick_draws(item, &resources).unwrap();
    let pool = Pool::<u32>::new(
        root,
        [800, 600],
        MapIconPool::Combat,
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    let registrations: Vec<_> = pool.registrations().collect();
    assert!(registrations.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(registrations[0].0 > instance.registration());
}

fn text_box_window(key: &str, slider: &str, extra: &str) -> Fixture {
    let bar = PLAIN.replace("maxValue = 1", extra).replace(
        "guiButtonType = { name = \"s\" spriteType = \"GFX_bar3x17\" }",
        &format!("guiButtonType = {{ name = \"s\" spriteType = \"{slider}\" }}"),
    );
    fixture_with(read(
        format!(
            "guiTypes = {{ {bar} containerWindowType = {{ name = \"w\" \
             size = {{ width = 100 height = 100 }} \
             instantTextBoxType = {{ name = \"t\" font = \"Main_14\" text = \"{key}\" \
             position = {{ x = 5 y = 7 }} maxWidth = 50 maxHeight = 20 \
             scrollbarType = \"lsb\" }} }} }}"
        )
        .as_bytes(),
    ))
}

fn text_bar_slider(instance: &Instance, window: &Element, resources: &Fixture) -> Vec<(i32, f32)> {
    instance
        .layout(window, resources, None)
        .unwrap()
        .map_or(Vec::new(), |laid| {
            laid.parts
                .iter()
                .filter(|part| part.path == "lsb.s")
                .map(|part| (frame_of(&part.kind), rect(&part.kind)[1]))
                .collect()
        })
}

#[test]
fn a_text_write_refreshes_its_scrollbar_in_order() {
    content_reader_script::test_game::enter();
    let resources = text_box_window("MOD_KEY", "GFX_animated", "maxValue = 1");
    let window = resources.gui_type(b"w").unwrap();
    let text = window.find(Kind::TextBox, "t").unwrap();
    let mut shown = Instance::construct(window, [800, 600], &resources).unwrap();
    let mut calls = Calls::default();
    calls.push((), text, Call::SetText("aaaa bbbb cccc".into()));
    let mut instances = HashMap::from([((), shown)]);
    calls.apply(window, &mut instances, &resources).unwrap();
    shown = instances.remove(&()).unwrap();
    shown.tick(window, &resources, dt(1.0)).unwrap();
    shown.gui_show(window, window).unwrap();
    shown.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(text_bar_slider(&shown, window, &resources), [(2, 17.0)]);
    let mut hidden = Instance::construct(window, [800, 600], &resources).unwrap();
    hidden
        .set_text(window, text, "aaaa bbbb cccc", &resources)
        .unwrap();
    hidden.set_text(window, text, "ab", &resources).unwrap();
    hidden.tick(window, &resources, dt(1.0)).unwrap();
    hidden.gui_show(window, window).unwrap();
    hidden.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(text_bar_slider(&hidden, window, &resources), []);
    hidden
        .set_text(window, text, "aaaa bbbb cccc", &resources)
        .unwrap();
    hidden.tick(window, &resources, dt(0.1)).unwrap();
    assert_eq!(text_bar_slider(&hidden, window, &resources), [(0, 17.0)]);
    let clip = |instance: &Instance| {
        instance
            .layout(window, &resources, None)
            .unwrap()
            .unwrap()
            .parts[0]
            .clip
    };
    assert_eq!(clip(&hidden), Some([5, 7, 55, 27]));
    let mut calls = Calls::default();
    calls.push(
        (),
        text,
        Call::Control(Box::new(Bound {
            text: Some("ab".into()),
            ..Bound::default()
        })),
    );
    let mut instances = HashMap::from([((), hidden)]);
    calls.apply(window, &mut instances, &resources).unwrap();
    let mut hidden = instances.remove(&()).unwrap();
    hidden.tick(window, &resources, dt(0.1)).unwrap();
    assert_eq!(text_bar_slider(&hidden, window, &resources), []);
    assert_eq!(clip(&hidden), None);
    hidden.control(window, text).unwrap().text = Some("aaaa bbbb cccc".into());
    assert!(hidden.tick(window, &resources, dt(0.1)).is_err());
    assert!(hidden.layout(window, &resources, None).is_err());
}

#[test]
fn resolved_flag_text_setter_keeps_native_scroll_range() {
    content_reader_script::test_game::enter();
    let resources = text_box_window("MOD_KEY", "GFX_bar3x17", "maxValue = 1");
    let window = resources.gui_type(b"w").unwrap();
    let text = window.find(Kind::TextBox, "t").unwrap();
    let value = format!("{}GER-Western_group", native_text::RESOLVED_FLAG);
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.control(window, text).unwrap().resolved_flags = true;
    instance.set_text(window, text, &value, &resources).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(text_bar_slider(&instance, window, &resources), []);
    let layout = instance.layout(window, &resources, None).unwrap().unwrap();
    assert!(layout.parts.iter().any(|part| matches!(&part.kind,
        PartKind::Inline { source: InlineSource::Texture(10), rect, .. }
        if rect[2..] == [10.0, 7.0])));
}

#[test]
fn a_text_box_scrollbar_keeps_the_clamp_of_a_smaller_range() {
    content_reader_script::test_game::enter();
    let resources = text_box_window(
        "MOD_LONG",
        "GFX_bar3x17",
        "maxValue = 22 stepSize = 1 useRangeLimit = yes rangeLimitMin = 10 rangeLimitMax = 20",
    );
    let window = resources.gui_type(b"w").unwrap();
    let text = window.find(Kind::TextBox, "t").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(text_bar_slider(&instance, window, &resources), []);
    let top = |instance: &Instance| {
        let laid = instance.layout(window, &resources, None).unwrap().unwrap();
        rect(&laid.parts[0].kind)[1]
    };
    let moved = top(&instance);
    instance.set_text(window, text, "ab", &resources).unwrap();
    instance
        .set_text(window, text, "aaaa bbbb cccc", &resources)
        .unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(text_bar_slider(&instance, window, &resources), [(0, 16.0)]);
    assert_eq!(top(&instance), moved);
    let mut direct = Instance::construct(window, [800, 600], &resources).unwrap();
    direct.gui_show(window, window).unwrap();
    direct
        .set_text(window, text, "aaaa bbbb cccc bbbb", &resources)
        .unwrap();
    direct.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(text_bar_slider(&direct, window, &resources).len(), 1);
    assert_ne!(
        text_bar_slider(&direct, window, &resources),
        text_bar_slider(&instance, window, &resources)
    );
}

#[test]
fn retained_result_text_scroll_moves_glyphs_by_the_native_line_step() {
    content_reader_script::test_game::enter();
    let resources = text_box_window(
        "MOD_KEY",
        "GFX_bar3x17",
        "maxValue = 1 stepSize = 1 scroll_speed = 2",
    );
    let window = resources.gui_type(b"w").unwrap();
    let text = window.find(Kind::TextBox, "t").unwrap();
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance
        .set_text(window, text, "aaaa\nbbbb\ncccc\ndddd", &resources)
        .unwrap();
    instance
        .set_text_scroll_lines(window, text, 3, &resources)
        .unwrap();
    let top = |instance: &Instance| {
        let layout = instance.layout(window, &resources, None).unwrap().unwrap();
        rect(&layout.parts[0].kind)[1]
    };
    let before = top(&instance);
    let slider = text_bar_slider(&instance, window, &resources);
    instance
        .scroll_text(window, text, 1, false, &resources)
        .unwrap();
    assert_eq!(top(&instance), before - 12.0);
    assert_ne!(text_bar_slider(&instance, window, &resources), slider);
    instance
        .scroll_text(window, text, 1, true, &resources)
        .unwrap();
    assert_eq!(top(&instance), before - 36.0);
    instance
        .scroll_text(window, text, 1, true, &resources)
        .unwrap();
    assert_eq!(top(&instance), before - 36.0);
    instance
        .set_text(window, text, "aaaa\nbbbb\ncccc\ndddd", &resources)
        .unwrap();
    instance
        .set_text_scroll_lines(window, text, 3, &resources)
        .unwrap();
    assert_eq!(top(&instance), before - 36.0);
    instance
        .scroll_text(window, text, -1, true, &resources)
        .unwrap();
    instance
        .scroll_text(window, text, -1, false, &resources)
        .unwrap();
    assert_eq!(top(&instance), before);
    let layout = instance
        .layout_with_regions(window, &resources, None, true)
        .unwrap()
        .unwrap();
    let buttons: Vec<_> = layout
        .regions
        .iter()
        .filter_map(|region| {
            region
                .scroll
                .map(|action| (region.owner.as_deref(), action))
        })
        .collect();
    assert_eq!(
        buttons,
        [
            (Some(text.path.as_str()), (true, -1)),
            (Some(text.path.as_str()), (true, 1))
        ]
    );
}

#[test]
fn calls_prepared_off_the_view_apply_once() {
    content_reader_script::test_game::enter();
    let gui = gui(
        "size = { width = 10 height = 10 } iconType = { name = \"a\" spriteType = \"GFX_even\" }",
    );
    let window = gui.template("w").unwrap();
    let resources = fixture();
    let mut instances = HashMap::from([(
        7u32,
        Instance::construct(window, [800, 600], &resources).unwrap(),
    )]);
    let icon = window.find(Kind::Icon, "a").unwrap();
    let mut calls = Calls::default();
    calls.push(7, window, Call::SetPosition([5, 6]));
    calls.push(7, window, Call::GuiShow);
    calls.push(7, icon, Call::Hide);
    calls.push(7, icon, Call::SetHideFlag(true));
    calls.push(7, icon, Call::Show);
    calls.push(7, icon, Call::SetFrame(2));
    calls.push(7, icon, Call::Play);
    calls.push(
        7,
        icon,
        Call::Control(Box::new(Bound {
            color: Some([0.5; 4]),
            ..Bound::default()
        })),
    );
    assert!(!instances[&7].drawn());
    calls.apply(window, &mut instances, &resources).unwrap();
    let instance = instances.get_mut(&7).unwrap();
    assert!(instance.drawn());
    assert_eq!(instance.position(), [5, 6]);
    let bindings = instance.bindings(window, &resources).unwrap();
    let bound = bindings.at(Site::new(icon, None)).unwrap();
    assert!(!bound.shown.unwrap().visible);
    assert_eq!((bound.frame, bound.color), (Some(2), Some([0.5; 4])));
    let mut hides = Calls::default();
    hides.push(7, window, Call::HideWindow);
    hides.push(7, window, Call::Show);
    hides.apply(window, &mut instances, &resources).unwrap();
    assert!(instances[&7].drawn());
    let mut unknown = Calls::default();
    unknown.push(8, window, Call::GuiShow);
    assert!(unknown.apply(window, &mut instances, &resources).is_err());
}

#[test]
fn a_root_is_built_in_the_viewport() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui(
        "size = { width = 50% height = 20 } iconType = { name = \"a\" spriteType = \"GFX_even\" }",
    );
    let window = gui.template("w").unwrap();
    assert_eq!(
        layout(window, None, &resources, &Bindings::default()).unwrap_err()[0].kind,
        RefusalKind::NoParent
    );
    let mut instance = Instance::construct(window, [800, 600], &resources).unwrap();
    instance.gui_show(window, window).unwrap();
    instance.tick(window, &resources, dt(0.0)).unwrap();
    let laid = instance.layout(window, &resources, None).unwrap().unwrap();
    assert_eq!(laid.size, [400, 20]);
}

#[test]
fn alpha_hover_keeps_an_out_of_range_selected_frame_offset() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    resources.physical = Some([800, 600]);
    resources.add_sprite("strip", [8, 4], 2);
    resources.sprites.get_mut("strip").unwrap().hit = Hit::Alpha;
    resources.alpha.insert(
        "strip",
        AlphaMask {
            width: 8,
            transparent: vec![1; 32].into(),
        },
    );
    let gui = gui("iconType = { name = a spriteType = strip position = { x = 10 y = 20 } }");
    let root = gui.template("w").unwrap();
    let icon = root.find(Kind::Icon, "a").unwrap();
    let mut instance = Instance::construct(root, [800, 600], &resources).unwrap();
    instance.gui_show(root, root).unwrap();
    instance.set_frame(root, icon, 10).unwrap();
    let laid = instance
        .layout_with_regions(root, &resources, None, true)
        .unwrap()
        .unwrap();
    let hit = laid
        .regions
        .iter()
        .find(|region| region.path == icon.path)
        .unwrap()
        .hit
        .as_ref()
        .unwrap();
    assert!(hit.contains([10.2, 20.2]));
}

#[test]
fn normalized_progress_keeps_large_step_rounding_without_an_integer_reclamp() {
    content_reader_script::test_game::enter();
    let mut resources = fixture();
    let steps = 16_777_215;
    let bar = resources.sprites.get_mut("GFX_bar").unwrap();
    let SpriteKind::Progress { steps: limit, .. } = &mut bar.kind else {
        panic!("progress fixture");
    };
    *limit = steps;
    let document = gui("iconType={name=p spriteType=GFX_bar}");
    let root = document.template("w").unwrap();
    let element = root.find(Kind::Icon, "p").unwrap();
    let raw = normalized_progress_value(1.0, steps);
    assert_eq!(raw, 16_777_216);
    let state = |instance: &Instance| {
        let laid = instance.layout(root, &resources, None).unwrap().unwrap();
        let PartKind::Progress { state, .. } = laid.parts[0].kind else {
            panic!("progress draw");
        };
        state
    };
    let mut instance = Instance::construct(root, [800, 600], &resources).unwrap();
    instance.gui_show(root, root).unwrap();
    instance
        .set_progress_float_value(root, element, raw)
        .unwrap();
    assert!(state(&instance) > 1.0);
    instance.set_frame(root, element, raw).unwrap();
    assert_eq!(state(&instance), 1.0);
    instance
        .set_progress_float_value(root, element, normalized_progress_value(-1.0, steps))
        .unwrap();
    assert_eq!(state(&instance), 0.0);
}

#[test]
fn normalized_progress_uses_double_scaling_and_native_float_half_rounding() {
    assert_eq!(normalized_progress_value(0.125, 12), 2);
    assert_eq!(normalized_progress_value(-1.0, 12), 0);
    assert_eq!(normalized_progress_value(2.0, 12), 12);
    assert_eq!(normalized_progress_value(0.125, -12), -2);
    assert_eq!(normalized_progress_value(1.0, i32::MAX), i32::MIN);
}

#[test]
fn a_dragged_slider_gives_its_share_of_the_range() {
    let slide = Slide {
        vertical: true,
        travel: 100,
        thumb: 20,
    };
    assert_eq!(slide.dragged(0, [0, 400]), 80);
    assert_eq!(slide.dragged(30, [0, 400]), 200);
    assert_eq!(slide.dragged(-50, [0, 400]), 0);
    assert_eq!(slide.dragged(500, [0, 400]), 400);
    assert_eq!(slide.dragged(5, [0, 2]), 1);
    let still = Slide { travel: 0, ..slide };
    assert_eq!(still.dragged(10, [3, 9]), 3);
}

#[test]
fn a_clip_the_code_sets_bounds_a_windows_children() {
    content_reader_script::test_game::enter();
    let body = "size = { width = 40 height = 40 } \
         containerWindowType = { name = \"c\" \
            iconType = { name = \"i\" spriteType = \"GFX_even\" position = { x = 30 y = 5 } } }";
    let unbound = laid(body, None).unwrap().unwrap();
    assert_eq!(unbound.parts[0].clip, None);
    let layout = laid_bound(body, None, |window, bindings| {
        bindings.bind(window.path_named("c").unwrap()).clip = Some([2, 3, 20, 30]);
    })
    .unwrap()
    .unwrap();
    assert_eq!(layout.parts[0].clip, Some([2, 3, 20, 30]));
}

#[test]
fn adjacency_rule_pool_admits_64_icons_before_first_growth() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("size = { width = 10 height = 10 }");
    let window = gui.template("w").unwrap();
    assert_eq!(MapIconPool::AdjacencyRule.initial(), 64);
    for build in [
        "1.19.5.0.c184",
        "1.19.4.2.0bdb",
        "1.19.4.1.f584",
        "Countenance v1.19.5.0.c184",
        "Countenance v1.19.4.2.0bdb",
        "Countenance v1.19.4.1.f584",
    ] {
        assert_eq!(
            MapIconPool::AdjacencyRule.initial_for_build(Some(build)),
            Ok(64),
            "verified build {build}"
        );
    }
    let mut pool = Pool::new_for_build(
        window,
        [800, 600],
        MapIconPool::AdjacencyRule,
        Some("Countenance v1.19.5.0.c184"),
        Box::new(|_, _| Ok(())),
        &resources,
    )
    .unwrap();
    for owner in 0..64u32 {
        assert!(
            pool.request(owner).is_some(),
            "owner {owner} should fit in the initial 64 slots"
        );
    }
    assert_eq!(pool.live().count(), 64);
    assert_eq!(pool.growths(), 0);
    assert!(!pool.requests_pending());
    assert!(pool.request(64).is_none());
    assert!(pool.requests_pending());
    pool.tick(window, &resources, dt(0.0)).unwrap();
    assert_eq!(pool.growths(), 1);
    assert!(pool.request(64).is_some());
}

#[test]
fn unknown_build_refuses_map_icon_pool_capacity_by_name() {
    content_reader_script::test_game::enter();
    let resources = fixture();
    let gui = gui("size = { width = 10 height = 10 }");
    let window = gui.template("w").unwrap();
    for build in [
        None,
        Some(""),
        Some("   "),
        Some("1.19.5.0"),
        Some("Countenance v1.19.5.0.dead"),
        Some("1.19.4.2.c184"),
        Some("1.99.0.0.c184"),
        Some("1.19.5.0.c184.extra"),
    ] {
        let error = MapIconPool::AdjacencyRule
            .initial_for_build(build)
            .expect_err("unverified build must not fall back to a guessed capacity");
        assert!(error.contains("adjacency rule"), "{error}");
        if let Some(build) = build.map(str::trim).filter(|b| !b.is_empty()) {
            assert!(error.contains(build), "{error}");
        }
        let pool_error = Pool::<u32>::new_for_build(
            window,
            [800, 600],
            MapIconPool::AdjacencyRule,
            build,
            Box::new(|_, _| Ok(())),
            &resources,
        )
        .expect_err("pool construction must refuse unknown build capacity");
        assert_eq!(pool_error, error);
    }
}
