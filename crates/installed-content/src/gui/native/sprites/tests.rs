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

/// A plain sprite keeps every field `CSpriteType::read@14222C300` stores: `%i` frames,
/// `anchor` and `dx_offset` components (a fourth anchor component the stretch, a third
/// `dx_offset` component the flag bytes), `framedirectiony`'s zeroed local.
#[test]
fn a_plain_sprite_keeps_what_its_reader_keeps() {
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
    // 1.0's bits are 00 00 80 3F: the third `dx_offset` component clears
    // `transparencecheck` and its flag and sets `can_be_lowres` and `legacy_lazy_load`.
    let (bytes, _) = plain("transparencecheck = yes dx_offset = { 0.5 0.5 1.0 }");
    assert!(!bytes.transparence_check && !bytes.transparence_check_set);
    assert!(bytes.can_be_lowres && bytes.legacy_lazy_load);
    assert_eq!(bytes.dx_offset, [0.5, 0.5]);
}

/// `read_axis_components@140ACEBE0` writes `anchor`'s components on from `+0xE8` into the
/// sprite type: each lands on its field ([`SpriteBytes`]), stored when a later read sees it,
/// nothing when the game rewrites it first.
#[test]
fn anchor_components_land_on_the_fields_past_the_anchor() {
    // The sixth lands on the frame index `CSpriteType::draw@14222D5A0` writes before
    // reading it; the third on the geometry scale `CreateVertexBuffer` rewrites.
    let (sixth, refusals) = plain("anchor = { 0 0 1 1 1 7 }");
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        (sixth.anchor, sixth.stretch, sixth.v_scale),
        ([0.0, 0.0], 1.0, 1.0)
    );
    // The seventh and eighth are the frame step, the tenth the direction (1.4e-45 is the
    // bits 1: the frames run down), the eleventh's low bytes the U flip and
    // `alwaystransparent` (1.0000001 is 0x3F800001).
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
    // A direction other than 0, 1 and 2: `build_geometry` leaves the written step.
    let (kept, _) = plain("anchor = { 0 0 0 0 0 0 0.25 0.5 0 2.5 }");
    assert_eq!(kept.frame_direction, 2.5f32.to_bits() as i32);
    assert_eq!(
        (kept.grid(), kept.draw_step([0.1, 0.2])),
        ([1, 1], [0.25, 0.5])
    );
    // A later key rewrites what a component wrote.
    let (rewritten, _) = plain("anchor = { 0 0 0 0 0 0 0 0 0 1.4e-45 } framedirectiony = no");
    assert_eq!(rewritten.frame_direction, 0);
    // The 47th is the texture id `load_base_texture@14222B2A0` loads only while it is -1.
    let mut ids = vec!["0"; 46];
    ids.push("1.4e-45");
    let (id, refusals) = plain(&format!("anchor = {{ {} }}", ids.join(" ")));
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(id.texture_id, 1);
}

/// From `+0x118` the components reach the animation records' vector: zeros over the null
/// vector are nothing, a data pointer or a count the readers follow is refused, a capacity or
/// an allocator only when a later record is appended through them.
#[test]
fn anchor_components_reaching_the_animation_records() {
    let zeros = |count: usize| vec!["0"; count].join(" ");
    let (sprite, refusals) = plain(&format!("anchor = {{ {} }}", zeros(18)));
    assert!(refusals.is_empty(), "{refusals:?}");
    assert!(sprite.animations.is_empty());
    let (_, refusals) = plain(&format!("anchor = {{ {} 5 }}", zeros(12)));
    assert!(refusals[0].reason.contains("+0x118"), "{refusals:?}");
    // A count of 1 over no record: `ReleaseTextures` reads one from the null pointer.
    let (_, refusals) = plain(&format!("anchor = {{ {} 1.4e-45 }}", zeros(15)));
    assert!(
        refusals[0].reason.contains("ReleaseTextures"),
        "{refusals:?}"
    );
    // A capacity of 1: the next record is copied through the null pointer.
    let record = "animation = { animationtype = scrolling }";
    let (_, refusals) = plain(&format!("anchor = {{ {} 1.4e-45 }} {record}", zeros(14)));
    assert_eq!(refusals[0].key, "animation", "{refusals:?}");
    // The allocator written: nothing until a record is appended through it.
    let written = format!("anchor = {{ {} 7 7 }}", zeros(16));
    let (_, refusals) = plain(&written);
    assert!(refusals.is_empty(), "{refusals:?}");
    let (_, refusals) = plain(&format!("{written} {record}"));
    assert!(
        refusals[0].reason.contains("allocator pointer"),
        "{refusals:?}"
    );
    // After a record, the count lowered to 0 hides it from every reader.
    // (`-` is a number `%f` stores nothing for: the data pointer and capacity are skipped.)
    let (hidden, refusals) = plain(&format!("{record} anchor = {{ {} - - - 0 }}", zeros(12)));
    assert!(refusals.is_empty(), "{refusals:?}");
    assert!(hidden.animations.is_empty());
    let (shown, _) = plain(&format!("{record} {record}"));
    assert_eq!(shown.animations.len(), 2);
}

/// `dx_offset`'s components from `+0x1C8`: the flags' bytes, `generate_mip_maps`, the second
/// vector's null data pointer, then the end of the kind's allocation or its own tail.
#[test]
fn dx_offset_components_land_to_the_end_of_the_object() {
    // `anchor`'s components through the vectors, handles and texture pointers (zeros, with
    // no record), then the masking string's buffer kept and its size written: past the
    // 16-byte buffer the readers stop at the size's own zero byte; past the object's
    // 0x70 bytes from the string they read past the allocation.
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
    // 1.0000305 is 0x3F800100: `+0x1D4` 0 (the logged-once byte), `+0x1D5` 1.
    let (flags, refusals) = plain("dx_offset = { 0.5 0.5 0 1.0000305 0 0 3 4 5 6 }");
    assert!(refusals.is_empty(), "{refusals:?}");
    assert!(flags.generate_mip_maps && !flags.legacy_lazy_load);
    let (_, refusals) = plain("dx_offset = { 0.5 0.5 0 0 1 }");
    assert!(refusals[0].reason.contains("+0x1D8"), "{refusals:?}");
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
        shield.refusals[0].reason.contains("SelectFlagAtlas"),
        "{:?}",
        shield.refusals
    );
}

/// The global offsets are read into `ReadToken`'s 8-byte local: further components land on
/// its frame, where only the sub-reader's out-pointer is read back before the saved
/// registers.
#[test]
fn global_offset_components_land_on_the_readers_frame() {
    let read = |text: &str| read_sprites(format!("spriteTypes = {{ {text} }}").as_bytes());
    let quiet = read("cursor_offset = { 3 4 5 6 0 0 7 8 9 }");
    assert!(quiet.refusals.is_empty(), "{:?}", quiet.refusals);
    assert_eq!(quiet.cursor_offset, Some([3, 4]));
    let pointer = read("selection_offset = { 3 4 5 6 1 }");
    assert!(pointer.refusals[0].reason.contains("142226B65"));
    let mut registers = vec!["0"; 32];
    registers.push("1");
    let past = read(&format!("cursor_offset = {{ {} }}", registers.join(" ")));
    assert!(past.refusals[0].reason.contains("saved R14"));
}

/// `SMeshData::vf4@142251AE0` by token id: a registered key it does not take (`size`) is
/// unexpected, its block skipped; a word the table misses (0xF) is a texture override that
/// skips nothing, so its block's `}` ends the settings, and the pdxmesh, sprite and file
/// levels close early: the declarations after it are read where the game reads them.
#[test]
fn mesh_settings_keys_follow_their_token_ids() {
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
    // `texture_extra = {`: logged, nothing skipped; its `}` ends the settings,
    // `texture_diffuse = "d"` is the pdxmesh's (unexpected), the settings' `}` ends the
    // pdxmesh and the pdxmesh's `}` the `spriteTypes` block; `spriteType` at the file level
    // is unexpected (its block skipped), and the `spriteTypes` block's `}` ends the file's
    // level.
    let unregistered = read_sprites(text("texture_extra").as_bytes());
    assert!(
        names(&unregistered).is_empty(),
        "{:?}",
        names(&unregistered)
    );
    // A dynamic token is no string: unexpected, its block skipped, as a keyword's.
    let dynamic = DynamicTokens::from_files([&b"texture_extra\n"[..]]);
    let tokened = read_sprites_with(text("texture_extra").as_bytes(), &dynamic);
    assert_eq!(names(&tokened), [b"GFX_a".to_vec(), b"GFX_b".to_vec()]);
    // A quoted key is a string (0xF) too.
    let quoted = read_sprites(text("\"size\"").as_bytes());
    assert!(names(&quoted).is_empty());
}

/// `SAnimationMapData::read@14222C690`: every field, an unknown key's block read on as keys,
/// a record without a type dropped.
#[test]
fn animation_blocks_keep_every_field() {
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
    // The unknown key's block was read as the animation's keys: its `}` closed the block,
    // and the rest was read as the sprite's.
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
    // `animationtexturescale`'s components go on over the key loop's frame: its argument
    // slots, then its buffer pointer (a null one skips the free), the capacity and count,
    // the allocator (read only to free an intact pointer), the inline entries and tokens,
    // and its saved XMM6.
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
    // Two keys write one frame: the halves of a null pointer, across keys.
    let (split, refusals) = plain(
        "animation = { animationtype = scrolling animationtexturescale = { 2 4 - - - - 0 } \
         animationtexturescale = { 2 4 - - - - - 0 } }",
    );
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(split.animations.len(), 1);
    // `animationrotationoffset` from `+0x60`: the type, blend mode and masks, then the
    // texture pointers (null here, so nothing), the texel size, and past the record.
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
    // A mask pointer written without a mask file: an eager sprite binds it.
    let (_, refusals) = offset("1 2 1.4e-45 0 0 0 1", true);
    assert!(refusals[0].reason.contains("+0x78"), "{refusals:?}");
    let (_, refusals) = offset("1 2 1.4e-45 0 0 0 1", false);
    assert!(refusals.is_empty(), "{refusals:?}");
    let mut past = vec!["0"; 12];
    past[2] = "1.4e-45";
    past.push("1");
    let (_, refusals) = offset(&past.join(" "), true);
    assert!(refusals[0].reason.contains("R15"), "{refusals:?}");
}

/// Each kind's own switch: frame rates inverted where the reader inverts them, the progress
/// bar's colours and textures, the text and resizeable kinds' `%u`, the masked shield's
/// fields checked first, the circular bar's radians, the charts.
#[test]
fn every_kind_reads_its_own_switch() {
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

/// `CColor::vf3@142237160` on the lexer's current token: a `%x` number that `sscanf` returns
/// EOF for stores zeroes; a reference value is no colour; a component above 1 is divided.
#[test]
fn colours_are_read_as_ccolor_reads_them() {
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

/// A read that returns 0 keeps the lexer's token, its quoted flag included
/// (`CTextLexer::read_token@1424A7A90`), and `resolve_variable_token@1424AB540` leaves a
/// quoted token: at the end of the text or a 0xFF after `"@v"`, `CColor::vf3@142237160`'s
/// further reads keep the literal, which `%f` does not scan, so the colour stays white.
#[test]
fn a_failed_read_keeps_a_quoted_reference_literal() {
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

/// Only the three container keys are read at file level; inside them, the object kinds'
/// walks are followed, an unknown key skipped, and the latest declaration of a name is the
/// one the lookup finds.
#[test]
fn the_file_level_and_the_registry_follow_the_game() {
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

/// The global `textcolors` table is the last block read; a table the file ends inside
/// stops on a frozen `}` and loops forever on anything else.
#[test]
fn text_colour_tables_follow_the_game() {
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

/// Every shipped `interface/**/*.gfx` reads with nothing refused.
#[test]
#[ignore = "reads the game's files; set BATTLE_PLAN_GAME_DIR"]
fn every_shipped_gfx_file_reads() {
    let Some(root) = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")) else {
        return;
    };
    let mut stack = vec![std::path::Path::new(&root).join("interface")];
    let (mut files, mut sprites) = (0, 0);
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "gfx") {
                continue;
            }
            files += 1;
            let read = read_sprites(&std::fs::read(&path).unwrap());
            sprites += read.sprites.len();
            for refusal in read
                .refusals
                .iter()
                .chain(read.sprites.iter().flat_map(|sprite| &sprite.refusals))
            {
                println!("{}: {refusal}", path.display());
            }
        }
    }
    println!("{files} files, {sprites} sprites");
    assert!(files > 0);
}
