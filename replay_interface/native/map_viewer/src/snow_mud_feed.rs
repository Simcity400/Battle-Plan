//! B13 — recording-fed `SnowMudData` synthesis (the SNOW_LAW §6 *product* path).
//!
//! [`snow_law`](crate::snow_law) proved two things about the engine's runtime
//! `SnowMudData` texture: (1) the deterministic addressing / channel-packing /
//! `SnowMudFade` blend law, and (2) that the *per-texel snow/mud values above
//! the `min_snow_level` floor are accumulated, seeded weather-simulation state*
//! that no shipped file reproduces byte-for-byte. SNOW_LAW §6 therefore makes
//! the product path **recording-fed**: the extractor emits, per hour, the
//! engine's own accumulated per-province ground snow/mud (requirements item 6,
//! surfaced by `replay_core`'s `SnowMudView` = `weather.snow_mud`), and this
//! module rasterizes that feed into the quarter-resolution `SnowMudData` payload
//! the terrain shader samples — **replacing the zero-fill** that made winter
//! replay frames wildly wrong versus the engine's own mined winter frames (the
//! documented "zero SnowMudData winter confound").
//!
//! # Why this is lossless w.r.t. the recording
//!
//! The engine texture packs *two* temporal keyframes per channel (`snow_fade0`
//! `.b` / `snow_fade1` `.g`, `mud_fade0` `.r` / `mud_fade1` `.a`) and cross-fades
//! them by the scene `SnowMudFade`. The recording, however, supplies **one
//! already-resolved current level per province per hour** (the accumulate/melt
//! integral the sim has reached), not two spatial keyframes. So the faithful,
//! information-preserving reconstruction writes the recorded current level into
//! **both** fade endpoints — exactly as [`SnowFloorTexture::to_snow_mud_payload`]
//! does for the deterministic floor — which makes the displayed value
//! (`lerp(fade0, fade1, SnowMudFade)`) equal the recorded level for *any*
//! `SnowMudFade`. The recorded `fade_phase` is carried through to bind into the
//! scene CB0 `SnowMudFade` for provenance, but with equal endpoints it does not
//! change the displayed result (there is no second keyframe in the feed to blend
//! toward — see the honest gap note in `documents/SNOW_LAW.md` / the lane
//! report). This is a deliberate, documented collapse, not a fit.
//!
//! # Orientation
//!
//! The texel → map-pixel mapping and the `flip_source_y` Y-flip are **identical**
//! to [`crate::snow_law::synthesize_snow_floor_texture`] (world `z=0` is SOUTH per
//! `CalcGlobeNormal`, so top-down `provinces.bmp` row 0 = NORTH must be flipped
//! into world-z order). A unit test pins that the two rasterizers agree on which
//! province lands in which texel, so the recording feed and the files-only floor
//! never disagree on orientation.

use crate::*;
use std::collections::BTreeMap;

/// One province's recorded accumulated ground-cover level, in the engine's
/// `0..=255` byte range (requirements item 6, `SnowMudCell`). The caller
/// (`replay_app`) resolves these from `replay_core`'s `SnowMudView.by_province`
/// (clamping the `u32` `snow_level_0_255` / `mud_level_0_255` leaves to bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordedSnowMudLevel {
    /// Accumulated snow, `0..=255` (the `snow_visual_min = 128` visibility fix
    /// is already baked into the recorded value by the sim).
    pub snow_0_255: u8,
    /// Accumulated mud, `0..=255`.
    pub mud_0_255: u8,
}

/// Inputs to the recording-fed rasterizer: the recorded per-province levels plus
/// the province raster to address them onto the quarter-res texture. Mirrors
/// [`crate::snow_law::SnowFloorInputs`] so orientation is guaranteed identical.
pub struct SnowMudFeedInputs<'a> {
    /// province id -> recorded accumulated level (from the recording feed).
    /// Provinces absent here render as zero (no snow/mud), the honest
    /// fail-closed default (most provinces carry no accumulation).
    pub levels_by_province: &'a BTreeMap<u32, RecordedSnowMudLevel>,
    /// Province id per map pixel, row-major, `map_width * map_height`, with
    /// `y = 0` at the TOP of `provinces.bmp` (the `load_provinces_bmp`
    /// convention). Flipped into world-z order when `flip_source_y` is set.
    pub province_ids: &'a [u32],
    pub map_width: usize,
    pub map_height: usize,
    /// Addressing law (defaults to vanilla quarter-res).
    pub addressing: SnowMudAddressing,
    /// Whether texture row 0 maps to the BOTTOM of the top-down BMP (world
    /// SOUTH). Production callers pass `true`; synthetic already-oriented
    /// rasters in tests pass `false`. Same semantics as
    /// [`crate::snow_law::SnowFloorInputs::flip_source_y`].
    pub flip_source_y: bool,
}

/// A rasterized recording-fed `SnowMudData` texture: the exact `B8G8R8A8_UNORM`
/// payload to bind as the `SnowMudData` SRV (snow_law sampler slot 7), plus the
/// `SnowMudFade` to write into the scene CB0 and coverage diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedSnowMudTexture {
    pub width: usize,
    pub height: usize,
    /// `B8G8R8A8_UNORM` bytes (`width * height * 4`), snow in `.b`/`.g`, mud in
    /// `.r`/`.a`, both fade endpoints set to the recorded current level.
    pub payload: Vec<u8>,
    /// Texels whose province matched a recorded feed cell (recording coverage).
    pub covered_texels: usize,
    /// Texels with a nonzero province id that had NO recorded level (rendered
    /// zero; a diagnostic of feed completeness, not an error).
    pub land_texels_without_level: usize,
    /// The `SnowMudFade` (clamped `[0,1]`) to bind into the scene CB0
    /// `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.z`. Provenance only with
    /// equal endpoints (see module docs).
    pub fade: f32,
}

/// Rasterize the recording-fed per-province levels into a quarter-resolution
/// [`RecordedSnowMudTexture`]. Fails closed on a mismatched raster length or
/// zero texture dimensions.
pub fn synthesize_snow_mud_from_levels(
    inputs: &SnowMudFeedInputs<'_>,
    fade_phase: f32,
) -> Result<RecordedSnowMudTexture> {
    let width = inputs.addressing.texture_width;
    let height = inputs.addressing.texture_height;
    if width == 0 || height == 0 {
        return Err("recording-fed snow/mud synthesis: zero texture dimensions".into());
    }
    if inputs.province_ids.len() != inputs.map_width * inputs.map_height {
        return Err(format!(
            "recording-fed snow/mud synthesis: province raster {} != {}x{}",
            inputs.province_ids.len(),
            inputs.map_width,
            inputs.map_height
        )
        .into());
    }

    let mut payload = vec![0u8; width * height * 4];
    let mut covered_texels = 0usize;
    let mut land_texels_without_level = 0usize;
    // Quarter-res block size on each axis (rounded; vanilla = 4). IDENTICAL to
    // snow_law::synthesize_snow_floor_texture so orientation matches exactly.
    let bx = (inputs.map_width as f32 / width as f32).max(1.0);
    let by = (inputs.map_height as f32 / height as f32).max(1.0);
    for ty in 0..height {
        for tx in 0..width {
            let mx = ((tx as f32 + 0.5) * bx) as usize;
            let unflipped_my = ((ty as f32 + 0.5) * by) as usize;
            if mx >= inputs.map_width || unflipped_my >= inputs.map_height {
                continue;
            }
            let my = if inputs.flip_source_y {
                inputs.map_height - 1 - unflipped_my.min(inputs.map_height - 1)
            } else {
                unflipped_my
            };
            let pid = inputs.province_ids[my * inputs.map_width + mx];
            match inputs.levels_by_province.get(&pid) {
                Some(level) => {
                    // Recorded current level -> BOTH fade endpoints (lossless
                    // display for any SnowMudFade; see module docs).
                    let ch = SnowMudChannels {
                        snow_fade0: level.snow_0_255,
                        snow_fade1: level.snow_0_255,
                        mud_fade0: level.mud_0_255,
                        mud_fade1: level.mud_0_255,
                    };
                    let base = (ty * width + tx) * 4;
                    payload[base..base + 4].copy_from_slice(&encode_snow_mud_texel(ch));
                    covered_texels += 1;
                }
                None => {
                    // province id 0 is ocean/undefined; only count real land
                    // provinces that the feed omitted.
                    if pid != 0 {
                        land_texels_without_level += 1;
                    }
                }
            }
        }
    }

    Ok(RecordedSnowMudTexture {
        width,
        height,
        payload,
        covered_texels,
        land_texels_without_level,
        fade: fade_phase.clamp(0.0, 1.0),
    })
}

/// Production entry point: load the mod-aware province raster (`provinces.bmp` +
/// `definition.csv`) via the resolver and rasterize the recorded per-province
/// levels into the bind-ready `SnowMudData` payload. `replay_app` calls this
/// once per rendered hour with the levels + `fade_phase` resolved from
/// `replay_core`'s `SnowMudView`. Uses `flip_source_y = true` (world-z
/// orientation), matching [`crate::snow_law::synthesize_snow_floor_from_resolver`].
pub fn synthesize_snow_mud_from_resolver(
    levels_by_province: &BTreeMap<u32, RecordedSnowMudLevel>,
    fade_phase: f32,
    resolver: &GameFileResolver,
) -> Result<RecordedSnowMudTexture> {
    let definition_csv = resolver.resolve_required(SNOW_DEFINITION_CSV_SOURCE, "definition.csv")?;
    let provinces_bmp = resolver.resolve_required(SNOW_PROVINCES_BMP_SOURCE, "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;
    let inputs = SnowMudFeedInputs {
        levels_by_province,
        province_ids: &bmp.ids,
        map_width: bmp.width,
        map_height: bmp.height,
        addressing: SnowMudAddressing::default(),
        flip_source_y: true,
    };
    synthesize_snow_mud_from_levels(&inputs, fade_phase)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- fx_snow_mud_accumulator ground-truth, mirrored from the fixture
    // generator SOURCE (replay_fixture_gen/src/main.rs::fx_snow_mud_accumulator).
    // SNAPSHOT_COUNT = 12, ticks 0..=11. NOTE the deliberate reproduction of the
    // generator's precedence: `.max(0)` binds to the else arm only.
    fn fx_snow_1000(tick: i64) -> i64 {
        (tick * 21).min(180)
    }
    fn fx_mud_1000(tick: i64) -> i64 {
        if tick < 5 {
            tick * 9
        } else {
            (45 - (tick - 5) * 5).max(0)
        }
    }
    fn fx_snow_1001(tick: i64) -> i64 {
        (fx_snow_1000(tick) + 18).min(220)
    }
    fn fx_mud_1001(tick: i64) -> i64 {
        fx_mud_1000(tick) / 2
    }

    /// A synthetic already-oriented raster: left column = province 1000, right
    /// column = province 1001, everything else ocean (id 0). `flip_source_y =
    /// false` because the raster is built directly in texture order.
    fn two_province_inputs<'a>(
        levels: &'a BTreeMap<u32, RecordedSnowMudLevel>,
        addressing: SnowMudAddressing,
        ids: &'a [u32],
    ) -> SnowMudFeedInputs<'a> {
        SnowMudFeedInputs {
            levels_by_province: levels,
            province_ids: ids,
            map_width: addressing.texture_width,
            map_height: addressing.texture_height,
            addressing,
            flip_source_y: false,
        }
    }

    fn small_addressing() -> SnowMudAddressing {
        // Tiny 4x2 texture, 1:1 with the map raster (bx = by = 1).
        SnowMudAddressing {
            texture_width: 4,
            texture_height: 2,
            ..SnowMudAddressing::default()
        }
    }

    /// Build a 4x2 raster where columns 0-1 are province 1000 and columns 2-3
    /// are province 1001.
    fn raster_4x2() -> Vec<u32> {
        let mut ids = vec![0u32; 8];
        for y in 0..2 {
            for x in 0..4 {
                ids[y * 4 + x] = if x < 2 { 1000 } else { 1001 };
            }
        }
        ids
    }

    #[test]
    fn recording_fed_snow_matches_fx_snow_mud_accumulator_at_probe_hour() {
        // The fixture probe is at tick 8, expected snow_level_0_255 = 168.
        let tick = 8;
        assert_eq!(fx_snow_1000(tick), 168, "matches the fixture Probe value");
        let mut levels = BTreeMap::new();
        levels.insert(
            1000,
            RecordedSnowMudLevel {
                snow_0_255: fx_snow_1000(tick) as u8,
                mud_0_255: fx_mud_1000(tick) as u8,
            },
        );
        levels.insert(
            1001,
            RecordedSnowMudLevel {
                snow_0_255: fx_snow_1001(tick) as u8,
                mud_0_255: fx_mud_1001(tick) as u8,
            },
        );
        let addressing = small_addressing();
        let ids = raster_4x2();
        let inputs = two_province_inputs(&levels, addressing, &ids);
        let tex = synthesize_snow_mud_from_levels(&inputs, 0.5).expect("synthesize");

        // Texel (0,0) covers province 1000; texel (3,0) covers province 1001.
        let ch_1000 = decode_snow_mud_texel(&tex.payload, 0).unwrap();
        let ch_1001 = decode_snow_mud_texel(&tex.payload, 3).unwrap();

        // Snow field == the fixture's ground-truth accumulator, and it is
        // fade-invariant (both endpoints equal), so the *terrain-input* snow
        // equals the recorded level for any SnowMudFade.
        assert_eq!(ch_1000.snow_fade0, 168);
        assert_eq!(ch_1000.snow_fade1, 168);
        assert_eq!(ch_1001.snow_fade0, 186); // (168 + 18).min(220)
        assert_eq!(ch_1001.snow_fade1, 186);
        for fade in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            assert!((ch_1000.displayed_snow(fade) - 168.0 / 255.0).abs() < 1e-6);
            assert!((ch_1001.displayed_snow(fade) - 186.0 / 255.0).abs() < 1e-6);
        }
        // Mud field == the fixture's ground truth (tick 8: 30, and 30/2 = 15).
        assert_eq!(ch_1000.mud_fade0, 30);
        assert_eq!(ch_1000.mud_fade1, 30);
        assert_eq!(ch_1001.mud_fade0, 15);
        assert_eq!(ch_1001.mud_fade1, 15);

        assert_eq!(tex.covered_texels, 8, "all 8 texels matched a fed province");
        assert_eq!(tex.land_texels_without_level, 0);
        assert!((tex.fade - 0.5).abs() < 1e-6);
    }

    #[test]
    fn recording_fed_snow_tracks_full_fixture_sweep() {
        // Reproduce every tick of the fixture and confirm the rasterized snow
        // field equals the fixture accumulator at each hour.
        let addressing = small_addressing();
        let ids = raster_4x2();
        for tick in 0..12i64 {
            let mut levels = BTreeMap::new();
            levels.insert(
                1000,
                RecordedSnowMudLevel {
                    snow_0_255: fx_snow_1000(tick) as u8,
                    mud_0_255: fx_mud_1000(tick) as u8,
                },
            );
            levels.insert(
                1001,
                RecordedSnowMudLevel {
                    snow_0_255: fx_snow_1001(tick) as u8,
                    mud_0_255: fx_mud_1001(tick) as u8,
                },
            );
            let inputs = two_province_inputs(&levels, addressing, &ids);
            let phase = tick as f32 / 11.0;
            let tex = synthesize_snow_mud_from_levels(&inputs, phase).expect("synthesize");
            let ch_1000 = decode_snow_mud_texel(&tex.payload, 0).unwrap();
            let ch_1001 = decode_snow_mud_texel(&tex.payload, 3).unwrap();
            assert_eq!(
                ch_1000.snow_fade1 as i64,
                fx_snow_1000(tick),
                "snow 1000 @tick {tick}"
            );
            assert_eq!(
                ch_1001.snow_fade1 as i64,
                fx_snow_1001(tick),
                "snow 1001 @tick {tick}"
            );
            assert_eq!(
                ch_1000.mud_fade1 as i64,
                fx_mud_1000(tick),
                "mud 1000 @tick {tick}"
            );
            assert!((tex.fade - phase).abs() < 1e-6);
        }
    }

    #[test]
    fn provinces_absent_from_feed_render_zero_and_are_counted() {
        // Feed only province 1000; province 1001 land texels render zero and are
        // counted as land_texels_without_level.
        let mut levels = BTreeMap::new();
        levels.insert(
            1000,
            RecordedSnowMudLevel {
                snow_0_255: 100,
                mud_0_255: 10,
            },
        );
        let addressing = small_addressing();
        let ids = raster_4x2();
        let inputs = two_province_inputs(&levels, addressing, &ids);
        let tex = synthesize_snow_mud_from_levels(&inputs, 0.0).expect("synthesize");
        // province 1001 texels (columns 2,3 in both rows) are zero.
        let ch_1001 = decode_snow_mud_texel(&tex.payload, 3).unwrap();
        assert_eq!(ch_1001, SnowMudChannels::default());
        assert_eq!(tex.covered_texels, 4); // 2 columns x 2 rows for 1000
        assert_eq!(tex.land_texels_without_level, 4); // 2 columns x 2 rows for 1001
    }

    #[test]
    fn ocean_zero_province_is_not_counted_as_missing_land() {
        // A raster of pure ocean (id 0) with an empty feed: no coverage, and no
        // "land without level" (id 0 is not a real province).
        let levels = BTreeMap::new();
        let addressing = small_addressing();
        let ids = vec![0u32; 8];
        let inputs = two_province_inputs(&levels, addressing, &ids);
        let tex = synthesize_snow_mud_from_levels(&inputs, 0.0).expect("synthesize");
        assert_eq!(tex.covered_texels, 0);
        assert_eq!(tex.land_texels_without_level, 0);
        assert!(tex.payload.iter().all(|&b| b == 0));
    }

    #[test]
    fn orientation_matches_snow_floor_rasterizer() {
        // Build a full-map-sized raster (map = texture dims here for simplicity)
        // with province 1000 only in the SOUTHERN half (bottom rows of the
        // top-down BMP), and confirm that WITH flip_source_y=true both the
        // recording-fed rasterizer and the snow_law floor rasterizer light up
        // the SAME texel rows — proving identical orientation handling.
        let width = crate::SNOW_MUD_TEXTURE_WIDTH;
        let height = crate::SNOW_MUD_TEXTURE_HEIGHT;
        // Top-down raster: province 1000 in the bottom half (image rows >=
        // height/2 = SOUTH), province 2000 in the top half (NORTH). 1:1 map.
        let mut ids = vec![0u32; width * height];
        for y in 0..height {
            let pid = if y >= height / 2 { 1000 } else { 2000 };
            for x in 0..width {
                ids[y * width + x] = pid;
            }
        }

        // Recording feed: only province 1000 has snow.
        let mut levels = BTreeMap::new();
        levels.insert(
            1000,
            RecordedSnowMudLevel {
                snow_0_255: 200,
                mud_0_255: 0,
            },
        );
        let feed_inputs = SnowMudFeedInputs {
            levels_by_province: &levels,
            province_ids: &ids,
            map_width: width,
            map_height: height,
            addressing: SnowMudAddressing::default(),
            flip_source_y: true,
        };
        let feed = synthesize_snow_mud_from_levels(&feed_inputs, 0.0).unwrap();

        // Floor: give province 1000's region a min_snow_level floor.
        let mut region_weather = BTreeMap::new();
        region_weather.insert(
            9u32,
            RegionWeather {
                region_id: 9,
                name: "R9".to_string(),
                provinces: vec![1000],
                periods: vec![WeatherPeriod {
                    between_start: WeatherPoint { day: 0, month0: 0 },
                    between_end: WeatherPoint { day: 30, month0: 0 },
                    min_snow_level: 0.5,
                    snow_probability: 0.0,
                    blizzard_probability: 0.0,
                    mud_probability: 0.0,
                    temperature: [0.0, 0.0],
                }],
            },
        );
        let mut province_to_region = BTreeMap::new();
        province_to_region.insert(1000u32, 9u32);
        let floor_inputs = SnowFloorInputs {
            province_ids: &ids,
            map_width: width,
            map_height: height,
            province_to_region: &province_to_region,
            region_weather: &region_weather,
            addressing: SnowMudAddressing::default(),
            flip_source_y: true,
        };
        let floor = synthesize_snow_floor_texture(SnowDate::new(1, 1).unwrap(), &floor_inputs)
            .expect("floor");

        // Both must light up EXACTLY the same texels (the north rows, since the
        // SOUTH-half BMP flips into the high world-z / north texture rows).
        let mut mismatches = 0usize;
        let mut lit = 0usize;
        for ty in 0..height {
            for tx in 0..width {
                let i = ty * width + tx;
                let feed_lit = decode_snow_mud_texel(&feed.payload, i).unwrap().snow_fade1 > 0;
                let floor_lit = floor.at(tx, ty) > 0.0;
                if feed_lit {
                    lit += 1;
                }
                if feed_lit != floor_lit {
                    mismatches += 1;
                }
            }
        }
        assert!(lit > 0, "recording feed lit some texels");
        assert_eq!(
            mismatches, 0,
            "feed and floor agree on every texel's coverage"
        );
    }

    #[test]
    fn fails_closed_on_mismatched_raster_length() {
        let levels = BTreeMap::new();
        let addressing = small_addressing();
        let ids = vec![0u32; 7]; // wrong: 4x2 = 8 expected
        let inputs = SnowMudFeedInputs {
            levels_by_province: &levels,
            province_ids: &ids,
            map_width: 4,
            map_height: 2,
            addressing,
            flip_source_y: false,
        };
        let err = synthesize_snow_mud_from_levels(&inputs, 0.0).expect_err("length mismatch");
        assert!(err.to_string().contains("province raster"), "{err}");
    }

    #[test]
    fn fade_is_clamped_to_unit_range() {
        let levels = BTreeMap::new();
        let addressing = small_addressing();
        let ids = vec![0u32; 8];
        let inputs = two_province_inputs(&levels, addressing, &ids);
        assert_eq!(
            synthesize_snow_mud_from_levels(&inputs, 2.5).unwrap().fade,
            1.0
        );
        assert_eq!(
            synthesize_snow_mud_from_levels(&inputs, -1.0).unwrap().fade,
            0.0
        );
    }
}
