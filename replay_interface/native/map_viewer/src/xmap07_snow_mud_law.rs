use crate::Result;
use portable_render_stream::{Xmap07PassiveSnowMudCarrier, XMAP07_SNOW_MUD_FORMAT};
use std::collections::BTreeSet;

/// The two names one engine texture answers to across the shipped shader corpus.
/// `GetMudSnowColor` in `gfx/FX/standardfuncsgfx.fxh` takes whichever the calling
/// translation unit declared; there is no second texture behind the second name.
pub const XMAP07_SNOW_MUD_SAMPLER_SPELLINGS: [&str; 2] = ["SnowMudData", "SnowMudTexture"];

/// The scene constant-buffer field whose `.z` lane is the global fade, and the
/// lane index inside it. `standardfuncsgfx.fxh` lerps mud `.r -> .a` and snow
/// `.b -> .g` by exactly that lane, so the four texture planes are meaningless
/// without it and the carrier certifies both together or neither.
pub const XMAP07_SNOW_MUD_FADE_SCENE_FIELD: &str = "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed";
pub const XMAP07_SNOW_MUD_FADE_SCENE_LANE: usize = 2;

/// The shader translation units the SnowMud census is taken over.
pub const XMAP07_SNOW_MUD_CONSUMER_EFFECT_FILES: [&str; 6] = [
    "gfx/FX/border.shader",
    "gfx/FX/pdxmap.shader",
    "gfx/FX/pdxmesh.shader",
    "gfx/FX/pdxwater.shader",
    "gfx/FX/river.shader",
    "gfx/FX/tree.shader",
];

/// The function every consumer reads the texture through.
pub const XMAP07_SNOW_MUD_SHADER_READER: &str = "GetMudSnowColor";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap07SnowMudSamplerSite {
    pub effect_file: String,
    pub symbol: String,
    pub index: u32,
    /// The declaration is read by an uncommented `GetMudSnowColor` call in the
    /// same translation unit. A declared-but-unread sampler still binds a
    /// resource slot, so it is counted and distinguished, never dropped.
    pub sampled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap07SnowMudCensus {
    pub sites: Vec<Xmap07SnowMudSamplerSite>,
    pub effect_files: usize,
    pub declaring_effect_files: usize,
    pub sampled_sites: usize,
    pub spellings: Vec<String>,
}

/// Strip HLSL comments, keeping line structure so a commented-out read cannot be
/// counted as a live one.
fn strip_hlsl_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'/' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                if bytes[index] == b'\n' {
                    out.push('\n');
                }
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

fn reads_symbol(stripped: &str, symbol: &str) -> bool {
    let needle = format!("{XMAP07_SNOW_MUD_SHADER_READER}(");
    let mut cursor = 0usize;
    while let Some(hit) = stripped[cursor..].find(&needle) {
        let open = cursor + hit + needle.len();
        let Some(close) = stripped[open..].find(')') else {
            return false;
        };
        if stripped[open..open + close]
            .split(',')
            .any(|argument| argument.trim() == symbol)
        {
            return true;
        }
        cursor = open;
    }
    false
}

/// Every SnowMud sampler declaration in one shader translation unit.
pub fn xmap07_snow_mud_sampler_sites(
    effect_file: &str,
    source: &str,
) -> Result<Vec<Xmap07SnowMudSamplerSite>> {
    let stripped = strip_hlsl_comments(source);
    let Some((pixel_shader, _)) = crate::util::extract_block_after(&stripped, "PixelShader", 0)
    else {
        return Err(format!("XMAP07 SnowMud census: {effect_file} has no PixelShader block").into());
    };
    let Some((samplers, _)) = crate::util::extract_block_after(&pixel_shader, "Samplers", 0) else {
        return Err(
            format!("XMAP07 SnowMud census: {effect_file} has no PixelShader Samplers block").into(),
        );
    };
    let mut sites = Vec::new();
    for symbol in XMAP07_SNOW_MUD_SAMPLER_SPELLINGS {
        let Some((block, _)) = crate::util::extract_block_after(&samplers, symbol, 0) else {
            continue;
        };
        let index = crate::util::parse_u32_value(&block, "Index").ok_or_else(|| {
            format!("XMAP07 SnowMud census: {effect_file} sampler {symbol} declares no Index")
        })?;
        sites.push(Xmap07SnowMudSamplerSite {
            effect_file: effect_file.to_string(),
            symbol: symbol.to_string(),
            index,
            sampled: reads_symbol(&stripped, symbol),
        });
    }
    Ok(sites)
}

/// The census over the effective shader corpus handed in.
///
/// The caller supplies `(effect_file, source)` from the resolver, so a mod that
/// overrides a shader is measured rather than assumed, and a mod that adds a
/// SnowMud declaration is counted rather than silently unsupplied.
pub fn xmap07_snow_mud_census(sources: &[(String, String)]) -> Result<Xmap07SnowMudCensus> {
    let mut sites = Vec::new();
    let mut declaring = 0usize;
    for (effect_file, source) in sources {
        let unit = xmap07_snow_mud_sampler_sites(effect_file, source)?;
        if !unit.is_empty() {
            declaring += 1;
        }
        sites.extend(unit);
    }
    let sampled_sites = sites.iter().filter(|site| site.sampled).count();
    let spellings: Vec<String> = sites
        .iter()
        .map(|site| site.symbol.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    for spelling in &spellings {
        if !XMAP07_SNOW_MUD_SAMPLER_SPELLINGS.contains(&spelling.as_str()) {
            return Err(format!(
                "XMAP07 SnowMud census found the unadmitted spelling {spelling}"
            )
            .into());
        }
    }
    Ok(Xmap07SnowMudCensus {
        sites,
        effect_files: sources.len(),
        declaring_effect_files: declaring,
        sampled_sites,
        spellings,
    })
}

/// What the one recorded carrier supplies to every census site at one hour.
#[derive(Clone, Debug, PartialEq)]
pub struct Xmap07SnowMudSupply {
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub payload_sha256: String,
    pub authority_sha256: String,

}

/// The recorded grid remains required independently of viewer phase.
pub fn xmap07_snow_mud_supply(
    carrier: &Xmap07PassiveSnowMudCarrier,
) -> Result<Xmap07SnowMudSupply> {
    if !carrier.texture_complete {
        return Err(
            "XMAP07 SnowMud carrier has no complete grid"
                .into(),
        );
    }
    if !carrier.reset_complete || !carrier.initial_snapshot_complete {
        return Err("XMAP07 SnowMud carrier has no complete observation baseline".into());
    }
    if carrier.format != XMAP07_SNOW_MUD_FORMAT || carrier.bytes_per_texel != 4 {
        return Err("XMAP07 SnowMud carrier does not carry the four engine planes".into());
    }
    if carrier
        .width
        .checked_mul(carrier.bytes_per_texel)
        .is_none_or(|pitch| pitch != carrier.row_pitch)
    {
        return Err("XMAP07 SnowMud carrier row pitch is not its own tight grid pitch".into());
    }
    Ok(Xmap07SnowMudSupply {
        width: carrier.width,
        height: carrier.height,
        row_pitch: carrier.row_pitch,
        payload_sha256: carrier.payload_sha256.clone(),
        authority_sha256: carrier.authority_sha256.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VANILLA_PDXMAP: &str = r#"
PixelShader =
{
	Samplers =
	{
		TerrainDiffuse = { Index = 0 MagFilter = "Linear" MinFilter = "Linear" MipFilter = "Linear" AddressU = "Wrap" AddressV = "Wrap" }
		SnowMudData =
		{
			Index = 7
			MagFilter = "Linear"
			MinFilter = "Linear"
			MipFilter = "Linear"
			AddressU = "Wrap"
			AddressV = "Wrap"
		}
	}
	MainCode PixelShaderTerrain
	{
		Code
		[[
			float4 vMudSnow = GetMudSnowColor( Input.prepos, SnowMudData );
		]]
	}
}
"#;

    const VANILLA_BORDER: &str = r#"
PixelShader =
{
	Samplers =
	{
		SnowMudData =
		{
			Index = 1
			MagFilter = "Linear"
			MinFilter = "Linear"
			MipFilter = "Linear"
			AddressU = "Wrap"
			AddressV = "Wrap"
		}
	}
	MainCode PixelShader
	{
		Code
		[[
			//float4 vMudSnow = GetMudSnowColor( Input.pos, SnowMudData );
		]]
	}
}
"#;

    const VANILLA_RIVER: &str = r#"
PixelShader =
{
	Samplers =
	{
		SnowMudTexture =
		{
			Index = 8
			MagFilter = "Linear"
			MinFilter = "Linear"
			MipFilter = "Linear"
			AddressU = "Wrap"
			AddressV = "Wrap"
		}
	}
	MainCode PixelShader
	{
		Code
		[[
			/* float4 dead = GetMudSnowColor( x, Nothing ); */
			float4 vMudSnow = GetMudSnowColor( Input.vPrePos_Fade.xyz, SnowMudTexture );
		]]
	}
}
"#;

    fn carrier() -> Xmap07PassiveSnowMudCarrier {
        use portable_render_stream::{
            tiles_from_grid, Xmap07TileLayout, A729_EXECUTABLE_SHA256,
            SNOW_MUD_PRODUCER_FUNCTION_RVA, SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256,
            XMAP07_PASSIVE_SNOW_MUD_SCHEMA, XMAP07_PASSIVE_SNOW_MUD_SCOPE,
            XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256, XMAP07_SNOW_MUD_FADE_UPDATER_RVA,
            XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256, XMAP07_SNOW_MUD_SCENE_COPY_RVA,
            XMAP07_SNOW_MUD_TILE_EDGE,
        };
        let (width, height) = (2u32, 1u32);
        let grid: Vec<u8> = (0..(width * height * 4) as u8).collect();
        let layout = Xmap07TileLayout::new(width, height, XMAP07_SNOW_MUD_TILE_EDGE).unwrap();
        let tiles = tiles_from_grid(&grid, &layout).unwrap();
        let mut carrier = Xmap07PassiveSnowMudCarrier {
            schema: XMAP07_PASSIVE_SNOW_MUD_SCHEMA.into(),
            scope: XMAP07_PASSIVE_SNOW_MUD_SCOPE.into(),
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            texture_producer_rva: SNOW_MUD_PRODUCER_FUNCTION_RVA,
            texture_producer_instructions_sha256: SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256.into(),
            scene_copy_rva: XMAP07_SNOW_MUD_SCENE_COPY_RVA,
            scene_copy_instructions_sha256: XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256.into(),
            process_epoch: "process-1".into(),
            campaign_epoch: "campaign-1".into(),
            hook_epoch: "hook-1".into(),
            reset_epoch_ordinal: 1,
            capture_frontier_ordinal: 3,
            texture_observation_ordinal: 3,
            reset_hour: 10,
            initial_snapshot_hour: 10,
            payload_origin_hour: 10,
            texture_observation_hour: 10,
            displayed_hour: 10,
            carry_forward: false,
            reset_complete: true,
            initial_snapshot_complete: true,
            texture_complete: true,
            width,
            height,
            row_pitch: width * 4,
            bytes_per_texel: 4,
            format: XMAP07_SNOW_MUD_FORMAT.into(),
            tile_edge: XMAP07_SNOW_MUD_TILE_EDGE,
            tiles: tiles.clone(),
            payload_sha256: snapshot::sha256::Sha256::hex(&grid),
            payload_blob_sha256: Xmap07PassiveSnowMudCarrier::payload_blob_sha256(&tiles),
            authority_sha256: String::new(),
        };
        carrier.authority_sha256 = carrier.authority_digest();
        carrier
    }

    #[test]
    fn census_counts_both_spellings_and_separates_a_commented_out_read() {
        let census = xmap07_snow_mud_census(&[
            ("gfx/FX/pdxmap.shader".to_string(), VANILLA_PDXMAP.to_string()),
            ("gfx/FX/border.shader".to_string(), VANILLA_BORDER.to_string()),
            ("gfx/FX/river.shader".to_string(), VANILLA_RIVER.to_string()),
        ])
        .unwrap();
        assert_eq!(census.sites.len(), 3);
        assert_eq!(census.declaring_effect_files, 3);
        assert_eq!(census.sampled_sites, 2);
        assert_eq!(
            census.spellings,
            vec!["SnowMudData".to_string(), "SnowMudTexture".to_string()]
        );
        let border = census
            .sites
            .iter()
            .find(|site| site.effect_file == "gfx/FX/border.shader")
            .unwrap();
        assert_eq!(border.index, 1);
        assert!(!border.sampled);
        let river = census
            .sites
            .iter()
            .find(|site| site.effect_file == "gfx/FX/river.shader")
            .unwrap();
        assert_eq!(river.symbol, "SnowMudTexture");
        assert_eq!(river.index, 8);
        assert!(river.sampled);
    }

    #[test]
    fn census_refuses_a_translation_unit_with_no_sampler_block() {
        assert!(xmap07_snow_mud_sampler_sites("gfx/FX/x.shader", "Effect x = {}").is_err());
    }

    #[test]
    fn supply_requires_the_complete_grid() {
        let supply = xmap07_snow_mud_supply(&carrier()).unwrap();
        assert_eq!(supply.width, 2);
        assert_eq!(supply.row_pitch, 8);

        for break_half in [
            |carrier: &mut Xmap07PassiveSnowMudCarrier| carrier.texture_complete = false,
            |carrier: &mut Xmap07PassiveSnowMudCarrier| carrier.reset_complete = false,
            |carrier: &mut Xmap07PassiveSnowMudCarrier| carrier.initial_snapshot_complete = false,
            |carrier: &mut Xmap07PassiveSnowMudCarrier| carrier.bytes_per_texel = 1,
            |carrier: &mut Xmap07PassiveSnowMudCarrier| carrier.row_pitch = 7,
            |carrier: &mut Xmap07PassiveSnowMudCarrier| carrier.format = "DXGI_FORMAT_R8_UNORM".into(),
        ] {
            let mut broken = carrier();
            break_half(&mut broken);
            assert!(xmap07_snow_mud_supply(&broken).is_err());
        }
    }
}
