use super::*;

fn one(text: &str) -> SpriteFacts {
    let sprites = read_sprites(text.as_bytes());
    assert_eq!(sprites.sprites.len(), 1, "{sprites:?}");
    sprites.sprites.into_iter().next().unwrap()
}

fn plain(text: &str) -> (Plain, Vec<Refusal>) {
    let sprite = one(&format!(
        "spriteTypes = {{ spriteType = {{ name = \"GFX_a\" {text} }} }}"
    ));
    (sprite.kind.plain().unwrap().clone(), sprite.refusals)
}

#[test]
fn a_plain_sprite_keeps_what_its_reader_keeps() {
    content_reader_script::test_game::enter();
    let (sprite, refusals) = plain(
        "texturefile = \"a.dds\" noOfFrames = 0x3 noOfFrames = many anchor = { x = 4.5 y = -2 } \
         masking_texture = \"m.dds\" legacy_lazy_load = no framedirectiony = yes \
         transparencecheck = maybe clicksound = click defaultAnimationTime = 2 \
         effectFile = \"\" loadType = \"INGAME\"",
    );
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(sprite.frames, 3);
    assert_eq!(sprite.anchor, [4.5, -2.0]);
    assert_eq!(sprite.texture, b"a.dds");
    assert_eq!(sprite.masking_texture, b"m.dds");
    assert!(!sprite.legacy_lazy_load && sprite.frame_direction_y());
    assert!(!sprite.transparence_check && sprite.transparence_check_set);
    assert_eq!(sprite.effect(), DEFAULT_EFFECT);
    assert_eq!(sprite.grid(), [1, 3]);
    assert_eq!(sprite.default_animation_time, 2.0);
    let loaded = one("spriteTypes = { spriteType = { name = \"x\" loadType = \"INGAME\" } }");
    assert_eq!(loaded.load_type, 3);
    let (malformed, _) = plain("framedirectiony = yes framedirectiony = maybe");
    assert!(!malformed.frame_direction_y());
    let (stretched, refusals) = plain("anchor = { 7 8 9 2 3 }");
    assert!(refusals.is_empty());
    assert_eq!(
        (stretched.anchor, stretched.stretch, stretched.v_scale),
        ([7.0, 8.0], 2.0, 3.0)
    );
    let (bytes, _) = plain("transparencecheck = yes dx_offset = { 0.5 0.5 1.0 }");
    assert!(!bytes.transparence_check && !bytes.transparence_check_set);
    assert!(bytes.can_be_lowres && bytes.legacy_lazy_load);
    assert_eq!(bytes.dx_offset, [0.5, 0.5]);
}

#[test]
fn anchor_components_land_on_the_fields_past_the_anchor() {
    content_reader_script::test_game::enter();
    let (sixth, refusals) = plain("anchor = { 0 0 1 1 1 7 }");
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        (sixth.anchor, sixth.stretch, sixth.v_scale),
        ([0.0, 0.0], 1.0, 1.0)
    );
    let (down, refusals) = plain(
        "noOfFrames = 4 anchor = { 1 2 3 4 5 6 0.25 0.5 9 1.4e-45 1.0000001 } \
         alwaystransparent = yes",
    );
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        (down.anchor, down.stretch, down.v_scale),
        ([1.0, 2.0], 4.0, 5.0)
    );
    assert_eq!(down.frame_step, [0.25, 0.5]);
    assert_eq!((down.frame_direction, down.grid()), (1, [1, 4]));
    assert!(down.flip_u && down.always_transparent);
    assert_eq!(down.draw_step([0.1, 0.2]), [0.0, 0.2]);
    let (kept, _) = plain("anchor = { 0 0 0 0 0 0 0.25 0.5 0 2.5 }");
    assert_eq!(kept.frame_direction, 2.5f32.to_bits() as i32);
    assert_eq!(
        (kept.grid(), kept.draw_step([0.1, 0.2])),
        ([1, 1], [0.25, 0.5])
    );
    let (rewritten, _) = plain("anchor = { 0 0 0 0 0 0 0 0 0 1.4e-45 } framedirectiony = no");
    assert_eq!(rewritten.frame_direction, 0);
    let mut ids = vec!["0"; 46];
    ids.push("1.4e-45");
    let (id, refusals) = plain(&format!("anchor = {{ {} }}", ids.join(" ")));
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(id.texture_id, 1);
}

#[test]
fn anchor_components_reaching_the_animation_records() {
    content_reader_script::test_game::enter();
    let zeros = |count: usize| vec!["0"; count].join(" ");
    let (sprite, refusals) = plain(&format!("anchor = {{ {} }}", zeros(18)));
    assert!(refusals.is_empty(), "{refusals:?}");
    assert!(sprite.animations.is_empty());
    let (_, refusals) = plain(&format!("anchor = {{ {} 5 }}", zeros(12)));
    assert!(
        refusals[0].reason.contains("animation records' vector"),
        "{refusals:?}"
    );
    let (_, refusals) = plain(&format!("anchor = {{ {} 1.4e-45 }}", zeros(15)));
    assert!(
        refusals[0].reason.contains("texture release"),
        "{refusals:?}"
    );
    let record = "animation = { animationtype = scrolling }";
    let (_, refusals) = plain(&format!("anchor = {{ {} 1.4e-45 }} {record}", zeros(14)));
    assert_eq!(refusals[0].key, "animation", "{refusals:?}");
    let written = format!("anchor = {{ {} 7 7 }}", zeros(16));
    let (_, refusals) = plain(&written);
    assert!(refusals.is_empty(), "{refusals:?}");
    let (_, refusals) = plain(&format!("{written} {record}"));
    assert!(
        refusals[0].reason.contains("allocator pointer"),
        "{refusals:?}"
    );
    let (hidden, refusals) = plain(&format!("{record} anchor = {{ {} - - - 0 }}", zeros(12)));
    assert!(refusals.is_empty(), "{refusals:?}");
    assert!(hidden.animations.is_empty());
    let (shown, _) = plain(&format!("{record} {record}"));
    assert_eq!(shown.animations.len(), 2);
}

#[test]
fn dx_offset_components_land_to_the_end_of_the_object() {
    content_reader_script::test_game::enter();
    let masking = |size: u32| {
        let mut components = vec!["0"; 42];
        for slot in &mut components[38..42] {
            *slot = "-";
        }
        let size = f32::from_bits(size).to_string();
        plain(&format!(
            "masking_texture = \"abc\" anchor = {{ {} {size} }}",
            components.join(" ")
        ))
    };
    let (long, refusals) = masking(40);
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(long.masking_texture.len(), 32);
    assert_eq!(&long.masking_texture[..4], b"abc\0");
    assert_eq!(long.masking_texture[16], 40);
    let (_, refusals) = masking(0x71);
    assert!(
        refusals[0].reason.contains("past the heap block"),
        "{refusals:?}"
    );
    let (flags, refusals) = plain("dx_offset = { 0.5 0.5 0 1.0000305 0 0 3 4 5 6 }");
    assert!(refusals.is_empty(), "{refusals:?}");
    assert!(flags.generate_mip_maps && !flags.legacy_lazy_load);
    let (_, refusals) = plain("dx_offset = { 0.5 0.5 0 0 1 }");
    assert!(
        refusals[0].reason.contains("second vector's data pointer"),
        "{refusals:?}"
    );
    let (_, refusals) = plain("dx_offset = { 0.5 0.5 0 0 0 0 0 0 0 0 1 }");
    assert!(
        refusals[0].reason.contains("heap overflow") && refusals[0].reason.contains("0x1F0"),
        "{refusals:?}"
    );
    let frame = one("spriteTypes = { frameAnimatedSpriteType = { name = \"f\" \
         dx_offset = { 0.5 0.5 0 0 0 0 0 0 0 0 0.125 0 3 } } }");
    assert!(frame.refusals.is_empty(), "{:?}", frame.refusals);
    let SpriteKind::FrameAnimated { animation, .. } = &frame.kind else {
        panic!()
    };
    assert_eq!(
        (animation.seconds_per_frame, animation.pause_on_loop),
        (0.125, 3.0)
    );
    let tile = one("spriteTypes = { tileSpriteType = { name = \"t\" \
         dx_offset = { 0.5 0.5 0 0 0 0 0 0 0 0 1.4e-45 0 0 0 9 } } }");
    let SpriteKind::Tile { size, .. } = &tile.kind else {
        panic!()
    };
    assert_eq!(*size, [1, 0]);
    assert!(
        tile.refusals[0].reason.contains("0x200"),
        "{:?}",
        tile.refusals
    );
    let shield = one("spriteTypes = { maskedShieldType = { name = \"s\" \
         dx_offset = { 0.5 0.5 0 0 0 0 0 0 0 0 1 } } }");
    assert!(
        shield.refusals[0].reason.contains("flag atlas selection"),
        "{:?}",
        shield.refusals
    );
}

#[test]
fn global_offset_components_land_on_the_readers_frame() {
    content_reader_script::test_game::enter();
    let read = |text: &str| read_sprites(format!("spriteTypes = {{ {text} }}").as_bytes());
    let quiet = read("cursor_offset = { 3 4 5 6 0 0 7 8 9 }");
    assert!(quiet.refusals.is_empty(), "{:?}", quiet.refusals);
    assert_eq!(quiet.cursor_offset, Some([3, 4]));
    let pointer = read("selection_offset = { 3 4 5 6 1 }");
    assert!(pointer.refusals[0].reason.contains("calls through"));
    let mut registers = vec!["0"; 32];
    registers.push("1");
    let past = read(&format!("cursor_offset = {{ {} }}", registers.join(" ")));
    assert!(past.refusals[0].reason.contains("saved R14"));
}

#[test]
fn mesh_settings_keys_follow_their_token_ids() {
    content_reader_script::test_game::enter();
    let text = |key: &str| {
        format!(
            "spriteTypes = {{\n\
             pdxmesh = {{ name = \"m\" meshsettings = {{ {key} = {{ }} texture_diffuse = \"d\" }} }}\n\
             spriteType = {{ name = \"GFX_a\" }}\n\
             }}\n\
             spriteTypes = {{ spriteType = {{ name = \"GFX_b\" }} }}\n"
        )
    };
    let names = |read: &Sprites| -> Vec<Vec<u8>> {
        read.sprites
            .iter()
            .map(|sprite| sprite.name.clone())
            .collect()
    };
    let registered = read_sprites(text("size").as_bytes());
    assert_eq!(names(&registered), [b"GFX_a".to_vec(), b"GFX_b".to_vec()]);
    assert!(registered.refusals.is_empty(), "{:?}", registered.refusals);
    let unregistered = read_sprites(text("texture_extra").as_bytes());
    assert!(
        names(&unregistered).is_empty(),
        "{:?}",
        names(&unregistered)
    );
    let dynamic = DynamicTokens::from_files([&b"texture_extra\n"[..]]);
    let tokened = read_sprites_with(text("texture_extra").as_bytes(), &dynamic);
    assert_eq!(names(&tokened), [b"GFX_a".to_vec(), b"GFX_b".to_vec()]);
    let quoted = read_sprites(text("\"size\"").as_bytes());
    assert!(names(&quoted).is_empty());
}

#[test]
fn animation_blocks_keep_every_field() {
    content_reader_script::test_game::enter();
    let (sprite, refusals) = plain(
        "animation = { animationmaskfile = \"m\" animationtexturefile = \"t\" \
         animationrotation = 90 animationlooping = no animationtime = 2 animationdelay = 0.5 \
         animationtype = \"Rotating_CCW\" animationblendmode = \"overlay\" \
         animationtexturescale = { x = 2 y = 0 } animationrotationoffset = { 1 2 } \
         animationframes = { 1 3 } animationtriggerframes = { 2 } animationtriggerframes = { 4 } \
         unknown = { animationdelay = 9 } } \
         animation = { animationtime = 3 }",
    );
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(sprite.animations.len(), 1);
    let animation = &sprite.animations[0];
    assert_eq!(animation.rotation, 90.0 * RADIANS);
    assert!(!animation.looping);
    assert_eq!((animation.time, animation.delay), (2.0, 9.0));
    assert_eq!((animation.kind, animation.blend_mode), (3, 2));
    assert_eq!(animation.texture_scale, [0.5, 1.0]);
    assert_eq!(animation.rotation_offset, [1.0, 2.0]);
    assert_eq!(animation.frames, 0b1010);
    assert_eq!(animation.trigger_frames, 0b10100);
    let scale = |components: &str| {
        plain(&format!(
            "animation = {{ animationtype = scrolling animationtexturescale = {{ {components} }} }}"
        ))
    };
    let (quiet, refusals) = scale("2 4 3 3 3 3");
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(quiet.animations[0].texture_scale, [0.5, 0.25]);
    let (bad, refusals) = scale("1 2 3 4 5 6 7");
    assert!(bad.animations.is_empty() && refusals.len() == 1);
    assert!(
        refusals[0].reason.contains("buffer pointer"),
        "{refusals:?}"
    );
    let (_, refusals) = scale("2 4 - - - - - - 5 5 1");
    assert!(refusals[0].reason.contains("allocator"), "{refusals:?}");
    let (freed, refusals) = scale("2 4 - - - - 0 0 5 5 1 1 1 1 9");
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(freed.animations[0].texture_scale, [0.5, 0.25]);
    let mut far = vec!["-"; 141];
    far[0] = "2";
    far.push("3");
    let (deep, refusals) = scale(&far.join(" "));
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(deep.animations[0].texture_scale, [0.5, 1.0]);
    far.push("3");
    let (_, refusals) = scale(&far.join(" "));
    assert!(refusals[0].reason.contains("XMM6"), "{refusals:?}");
    let (split, refusals) = plain(
        "animation = { animationtype = scrolling animationtexturescale = { 2 4 - - - - 0 } \
         animationtexturescale = { 2 4 - - - - - 0 } }",
    );
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(split.animations.len(), 1);
    let offset = |components: &str, eager: bool| {
        plain(&format!(
            "legacy_lazy_load = {} animation = {{ animationtexturefile = \"t\" \
             animationrotationoffset = {{ {components} }} }}",
            if eager { "no" } else { "yes" }
        ))
    };
    let (sized, refusals) = offset("1 2 1.4e-45 0 0 0 0 0 0 0 2.8e-45 1.4e-45", true);
    assert!(refusals.is_empty(), "{refusals:?}");
    let record = &sized.animations[0];
    assert_eq!((record.kind, record.texture_size), (1, [2, 1]));
    let (_, refusals) = offset("1 2 1.4e-45 0 0 0 1", true);
    assert!(
        refusals[0].reason.contains("(mask or texture)"),
        "{refusals:?}"
    );
    let (_, refusals) = offset("1 2 1.4e-45 0 0 0 1", false);
    assert!(refusals.is_empty(), "{refusals:?}");
    let mut past = vec!["0"; 12];
    past[2] = "1.4e-45";
    past.push("1");
    let (_, refusals) = offset(&past.join(" "), true);
    assert!(
        refusals[0].reason.contains(
            "past the record, the saved register after the sprite reader's record: the caller's registers are undefined"
        ),
        "{refusals:?}"
    );
}

#[test]
fn every_kind_reads_its_own_switch() {
    content_reader_script::test_game::enter();
    let frame = one(
        "spriteTypes = { frameAnimatedSpriteType = { name = \"f\" animation_rate_fps = 4 \
         looping = yes noOfFrames = 8 } }",
    );
    let SpriteKind::FrameAnimated { plain, animation } = &frame.kind else {
        panic!()
    };
    assert_eq!((plain.frames, animation.seconds_per_frame), (8, 0.25));
    assert!(animation.looping);
    let tile = one(
        "spriteTypes = { corneredTileSpriteType = { name = \"t\" noOfFrames = 2 \
         size = { x = 8 y = 9 } borderSize = { x = 2 y = 3 } animation_rate_spf = 4 \
         tilingCenter = yes } }",
    );
    let SpriteKind::CorneredTile(tile) = &tile.kind else {
        panic!()
    };
    assert_eq!((tile.size, tile.border, tile.frames), ([8, 9], [2, 3], 2));
    assert_eq!(tile.animation_rate, Some(0.25));
    assert!(tile.tiling_center);
    assert_eq!(tile.effect(), DEFAULT_EFFECT);
    let bar = one(
        "spriteTypes = { progressbartype = { name = \"b\" textureFile1 = \"a\" \
         textureFile2 = \"b\" color = { 255 0 0.5 } colorTwo = 0x80ff0000 size = { x = 54 y = 8 } \
         steps = 20 horizontal = no effectFile = \"gfx/FX/progress.lua\" } }",
    );
    let SpriteKind::Progress(bar) = &bar.kind else {
        panic!()
    };
    assert_eq!(bar.color, [1.0, 0.0, 0.5, 1.0]);
    assert_eq!(bar.color_two, [1.0, 0.0, 0.0, 128.0 / 255.0]);
    assert_eq!((bar.size, bar.steps, bar.horizontal), ([54, 8], 20, false));
    assert_eq!(
        (bar.texture1.as_slice(), bar.texture2.as_slice()),
        (&b"a"[..], &b"b"[..])
    );
    let text = one(
        "spriteTypes = { textSpriteType = { name = \"x\" noOfFrames = -1 textureFile = \"t\" } }",
    );
    let SpriteKind::Text(text) = &text.kind else {
        panic!()
    };
    assert_eq!(text.frames, u32::MAX);
    let resize = one(
        "spriteTypes = { resizeableSpriteType = { name = \"r\" noOfFrames = 3 \
         defaultAnimationTime = 5 size = { x = 1 y = 2 } } }",
    );
    let SpriteKind::Resizeable(resize) = &resize.kind else {
        panic!()
    };
    assert_eq!((resize.frames, resize.size), (5, [1, 2]));
    let shield = one(
        "spriteTypes = { maskedShieldType = { name = \"s\" textureFile1 = \"a\" effectFile = \"e\" \
         small = yes noOfFrames = 2 } }",
    );
    let SpriteKind::MaskedShield { plain, shield } = &shield.kind else {
        panic!()
    };
    assert_eq!(
        (shield.effect_file.as_slice(), shield.small),
        (&b"e"[..], true)
    );
    assert_eq!((plain.effect(), plain.frames), (DEFAULT_EFFECT, 2));
    let circle = one(
        "spriteTypes = { circularProgressBarType = { name = \"c\" size = 30 rotation = 180 \
         amount = 0.5 textureFile1 = \"a\" } }",
    );
    let SpriteKind::CircularProgress(circle) = &circle.kind else {
        panic!()
    };
    assert_eq!(
        (circle.size, circle.rotation, circle.amount),
        (30, 180.0 * RADIANS, 0.5)
    );
    let pie = one(
        "spriteTypes = { PieChartType = { name = \"p\" size = 10 colors = { rgb = { 255 0 0 } \
         hsv = { 0 100 100 } other = 1 rgb = { 0 0 1 } } } }",
    );
    let SpriteKind::PieChart(pie) = &pie.kind else {
        panic!()
    };
    assert_eq!(pie.size, Some(10));
    assert_eq!(pie.colors.len(), 4);
    assert_eq!(pie.colors[0], [1.0, 0.0, 0.0, 1.0]);
    let hsv = pie.colors[1];
    assert!(
        (hsv[0] - 1.0).abs() < 1e-6 && hsv[1].abs() < 1e-6 && hsv[2].abs() < 1e-6,
        "{hsv:?}"
    );
    assert_eq!(pie.colors[2], pie.colors[1]);
    assert_eq!(pie.colors[3], [0.0, 0.0, 1.0, 1.0]);
    let line = one(
        "spriteTypes = { LineChartType = { name = \"l\" linewidth = 3.5 size = { x = 5 y = 6 } } }",
    );
    let SpriteKind::LineChart(line) = &line.kind else {
        panic!()
    };
    assert_eq!((line.half_width, line.size), (1.75, [5, 6]));
    assert_eq!(line.effect_file, b"gfx/FX/linechart.lua");
}

#[test]
fn colours_are_read_from_the_current_token() {
    content_reader_script::test_game::enter();
    let color = |value: &str| {
        let SpriteKind::Progress(bar) = one(&format!(
            "spriteTypes = {{ progressbartype = {{ name = \"b\" color = {value} }} }}"
        ))
        .kind
        else {
            panic!()
        };
        bar.color
    };
    assert_eq!(color("-"), [0.0; 4]);
    assert_eq!(color("1"), [0.0, 0.0, 1.0 / 255.0, 0.0]);
    assert_eq!(color("\"ff\""), [1.0; 4]);
    assert_eq!(color("{ 0.5 2 0 0.25 }"), [0.5, 2.0 / 255.0, 0.0, 0.25]);
    let at =
        one("@c = 0xff0000ff\nspriteTypes = { progressbartype = { name = \"b\" color = @c } }");
    let SpriteKind::Progress(bar) = at.kind else {
        panic!()
    };
    assert_eq!(bar.color, [1.0; 4]);
}

#[test]
fn a_failed_read_keeps_a_quoted_reference_literal() {
    content_reader_script::test_game::enter();
    for tail in [&b""[..], b"\xff"] {
        let text = [
            b"@v = 0.5\nspriteTypes = {\n progressbartype = {\n  name = \"p\"\n  color = { \"@v\""
                .as_slice(),
            tail,
        ]
        .concat();
        let sprites = read_sprites(&text);
        let SpriteKind::Progress(bar) = &sprites.sprites[0].kind else {
            panic!()
        };
        assert_eq!(bar.color, [1.0; 4], "{tail:?}");
    }
}

#[test]
fn the_file_level_and_the_registry_follow_the_game() {
    content_reader_script::test_game::enter();
    let sprites = read_sprites(
        b"spriteTypes = { spriteType = { name = \"GFX_a\" noOfFrames = 2 } \
          pdxmesh = { name = \"m\" meshsettings = { texture_diffuse = \"x\" } } \
          arrowType = { name = \"arrow\" color = { 1 0 0 } } \
          mystery = { spriteType = { name = \"GFX_skipped\" } } \
          cursor_offset = { 3 4 } \
          spriteType = { name = \"GFX_a\" noOfFrames = 5 } }\n\
          notSprites = { spriteType = { name = \"GFX_out\" } }\n\
          bitmapfonts = { spriteType = { name = \"GFX_in_fonts\" }",
    );
    let names: Vec<_> = sprites
        .sprites
        .iter()
        .map(|sprite| sprite.name.clone())
        .collect();
    assert_eq!(
        names,
        [
            b"GFX_a".to_vec(),
            b"GFX_a".to_vec(),
            b"GFX_in_fonts".to_vec()
        ]
    );
    assert_eq!(
        sprites.get(b"GFX_a").unwrap().kind.plain().unwrap().frames,
        5
    );
    assert!(sprites.get(b"gfx_a").is_none());
    assert_eq!(sprites.cursor_offset, Some([3, 4]));
    assert!(sprites.refusals.is_empty(), "{:?}", sprites.refusals);
}

#[test]
fn arrows_keep_native_fields_and_scalar_failures() {
    content_reader_script::test_game::enter();
    let read = read_sprites(
        b"@s = 2.75 @c = 0xff0000ff objectTypes = { arrowType = {
          name = \"a\xfe\" size = @s size = nope endAt = .5 height = -1.5 height = nope
          type = 010 type = nope heading = .4 scale = 3 scale = nope
          cull_distance = 5 cull_distance = nope
          texture = \"gfx/\x80.dds\" normal = n specular = s overlay = o effect = e
          color = { .5 2 0 .25 } colortwo = @c
          unknown = { name = skipped } \"size\" = 7 } }",
    );
    assert!(read.refusals.is_empty(), "{:?}", read.refusals);
    let object = read.object(b"a\xfe").unwrap();
    assert!(object.refusals.is_empty(), "{:?}", object.refusals);
    let ObjectKind::Arrow(arrow) = &object.kind else {
        panic!("{object:?}");
    };
    assert_eq!((object.scale, object.cull_distance_squared), (3.0, 625.0));
    assert_eq!((arrow.size, arrow.end_at, arrow.heading), (2.75, 0.5, 0.4));
    assert_eq!((arrow.height, arrow.arrow_type), (Some(-1.5), Some(8)));
    assert_eq!(arrow.color, [0.5, 2.0 / 255.0, 0.0, 0.25]);
    assert_eq!(arrow.color_two, [1.0; 4]);
    assert_eq!(arrow.texture, b"gfx/\x80.dds");
    assert_eq!(arrow.normal, b"n");
    assert_eq!(arrow.specular, b"s");
    assert_eq!(arrow.overlay, b"o");
    assert_eq!(arrow.effect, b"e");
}

#[test]
fn arrows_preserve_uninitialised_fields_and_native_block_walk() {
    content_reader_script::test_game::enter();
    let defaults = read_sprites(b"objectTypes = { arrowType = { name = a } }");
    let object = defaults.object(b"a").unwrap();
    assert_eq!(
        (object.scale, object.cull_distance_squared),
        (1.0, 184_900.0)
    );
    assert_eq!(object.kind, ObjectKind::Arrow(ArrowDefinition::default()));
    assert!(object.refusals.is_empty());
    let read =
        read_sprites(b"objectTypes = { arrowType = { name = a size = { height = 2 } type = 5 } }");
    let ObjectKind::Arrow(arrow) = &read.object(b"a").unwrap().kind else {
        panic!();
    };
    assert_eq!(arrow.size, 1.0);
    assert_eq!(arrow.height, Some(2.0));
    assert_eq!(arrow.arrow_type, None);
}

#[test]
fn object_registry_keeps_later_cross_kind_collisions() {
    content_reader_script::test_game::enter();
    let files = vec![
        (
            "interface/a.gfx".into(),
            read_sprites(
                b"objectTypes = { arrowType = { name = a texture = first }
                  arrowType = { name = A texture = upper } }
                  spriteTypes = { spriteType = { name = a } }",
            ),
        ),
        (
            "gfx/b.gfx".into(),
            read_sprites(
                b"objectTypes = { arrowType = { name = a texture = second }
                  pdxmesh = { name = a file = mesh } }",
            ),
        ),
    ];
    let objects = object_registry(&files);
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[b"a".as_slice()].kind, ObjectKind::PdxMesh);
    let ObjectKind::Arrow(upper) = &objects[b"A".as_slice()].kind else {
        panic!();
    };
    assert_eq!(upper.texture, b"upper");
    assert!(registry(&files).contains_key(b"a".as_slice()));
    assert_eq!(files[1].1.object(b"a").unwrap().kind, ObjectKind::PdxMesh);
}

#[test]
fn object_registry_refuses_after_an_animated_position_overrun() {
    content_reader_script::test_game::enter();
    let within =
        read_sprites(b"objectTypes = { animatedmaptext = { name = text position = { 0 0 0 0 } } }");
    assert!(within.refusals.is_empty());
    assert!(within.object(b"text").unwrap().refusals.is_empty());
    let outside = read_sprites(
        b"objectTypes = { animatedmaptext = { name = text position = { 0 0 0 0 0 } }
          arrowType = { name = later texture = arrow } }",
    );
    for name in [b"text".as_slice(), b"later".as_slice()] {
        let object = outside.object(name).unwrap();
        assert_eq!(object.refusals[0].path.as_bytes(), name);
        assert!(object.refusals[0].reason.contains("past the 0xF0-byte"));
        assert_eq!(object.refusals[0].cause, Cause::Unaligned);
    }
}

#[test]
fn animated_map_text_keeps_its_speed_width_position_and_text_block() {
    content_reader_script::test_game::enter();
    let read = read_sprites(
        b"objectTypes = { animatedmaptext = { name = siegeinfo speed = 0.25 maxwidth = 300           position = { 1 2 3 } textblock = { text = \"txt\" color = { 1.0 0.0 0.0 1.0 }           font = \"AnimatedMapText\" format = centre } }           animatedmaptext = { name = plain } }",
    );
    let object = read.object(b"siegeinfo").unwrap();
    let ObjectKind::AnimatedMapText(text) = &object.kind else {
        panic!("{:?}", object.kind);
    };
    assert_eq!(text.speed, 0.25);
    assert_eq!(text.max_width, 300);
    assert_eq!(text.position, [1.0, 2.0, 3.0]);
    assert_eq!(text.text_block.font, b"AnimatedMapText");
    assert_eq!(text.text_block.text, b"txt");
    assert_eq!(text.text_block.format, 2);
    assert_eq!(text.text_block.color, [1.0, 0.0, 0.0, 1.0]);
    let ObjectKind::AnimatedMapText(plain) = &read.object(b"plain").unwrap().kind else {
        panic!("plain is not animated map text");
    };
    assert_eq!(*plain, AnimatedMapTextDefinition::default());
    assert_eq!(plain.speed.to_bits(), 0x3C23_D70A);
    assert_eq!(plain.max_width, -1);
}

#[test]
fn text_colour_tables_follow_the_game() {
    content_reader_script::test_game::enter();
    let read = read_sprites(
        b"bitmapfonts = {\n\
          textcolors = { R = { 255 0 0 } G = { 0 1 0 } }\n\
          textcolors = { W = { 255 255 255 } }\n\
          }",
    );
    let global = read.text_colors.unwrap();
    assert_eq!(global.keys().copied().collect::<Vec<_>>(), [b'W']);
    let closed = read_sprites(b"bitmapfonts = { textcolors = { R = { 1 0 0 }");
    assert!(closed.refusals.is_empty(), "{:?}", closed.refusals);
    assert!(closed.text_colors.unwrap().contains_key(&b'R'));
    let open = read_sprites(b"bitmapfonts = { textcolors = { R = { 1 0 0 } G");
    assert!(!open.refusals.is_empty());
}
