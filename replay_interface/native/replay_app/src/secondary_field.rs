//! Runtime-generated `ProvinceSecondaryColorMap` for occupied-territory stripes.
//!
//! the game does not ship this texture as a file. The 2026-07-02 capture descriptor
//! at `map_frame_bundle.json:45066-45104` shows the active PS slot 6 resource:
//! `2816x1024`, `B8G8R8A8_UNORM_SRGB` (DXGI 91), one mip, one array slice,
//! one sample, row pitch 11264, byte length 11534336. The captured payload was
//! an all-zero load-screen placeholder, so this module derives encoding only
//! from shader reads.
//!
//! `standardfuncsgfx.fxh:979-987` samples `.rgba`, multiplies the shader-made
//! occupation stripe by `.a`, and blends toward `.rgb`. Therefore this generated
//! map encodes only owner-side RGB and alpha; diagonal stripe shape remains in
//! the ported `CalculateOccupationMask` HLSL. Self-generated runtime texture
//! validation plan: next live capture of `GradientBorderChannel1/2` plus
//! `ProvinceSecondaryColorMap` from an occupied-territory save, then side-by-side
//! HOI4/replay screenshots at matching camera/hour.

use std::collections::BTreeMap;
use std::ptr;
use std::time::{Duration, Instant};

use winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM_SRGB;
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11Resource, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};

use crate::bundle::MapBundle;
use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::Result;

#[cfg(test)]
const PROVINCE_SECONDARY_CAPTURED_WIDTH: usize = 2816;
#[cfg(test)]
const PROVINCE_SECONDARY_CAPTURED_HEIGHT: usize = 1024;
pub(crate) const PROVINCE_SECONDARY_DXGI_FORMAT: u32 = DXGI_FORMAT_B8G8R8A8_UNORM_SRGB;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Clone, Debug)]
pub(crate) struct SecondaryRebuildStats {
    pub(crate) rebuilt: bool,
    pub(crate) elapsed: Duration,
    pub(crate) occupied_cells: usize,
}

pub(crate) struct SecondaryColorField {
    width: usize,
    height: usize,
    state_cells: Vec<u32>,
    texture: ComPtr<ID3D11Texture2D>,
    srv: ComPtr<ID3D11ShaderResourceView>,
    last_signature: u64,
    last_rebuild: Option<Duration>,
    last_occupied_cells: usize,
}

impl SecondaryColorField {
    pub(crate) fn new(gfx: &Gfx, bundle: &MapBundle) -> Result<Self> {
        let dimensions = SecondaryDimensions::from_map(bundle.width, bundle.height)?;
        let state_cells = crate::gb_field::build_half_res_state_cells(
            bundle.width,
            bundle.height,
            &bundle.province_ids,
            &bundle.province_to_state,
        )?;
        let empty = vec![0u8; dimensions.texture_byte_len()?];
        let (texture, srv) =
            unsafe { create_secondary_texture(gfx, dimensions.width, dimensions.height, &empty)? };
        Ok(Self {
            width: dimensions.width,
            height: dimensions.height,
            state_cells,
            texture,
            srv,
            last_signature: 0,
            last_rebuild: None,
            last_occupied_cells: 0,
        })
    }

    pub(crate) fn srv(&self) -> *mut ID3D11ShaderResourceView {
        self.srv.as_raw()
    }

    pub(crate) fn update(
        &mut self,
        gfx: &Gfx,
        owners: &BTreeMap<u32, String>,
        controllers: &BTreeMap<u32, String>,
        country_colors: &BTreeMap<String, [u8; 3]>,
    ) -> Result<SecondaryRebuildStats> {
        // USER-CONFIRMED BEHAVIOR (2026-07-02): plain occupied territory is
        // NOT striped in the game's political map mode — occupied land shows the
        // controller's solid color. Both live captures also read
        // ProvinceSecondaryColorMap as all-zero. The engine's fill rule for
        // this texture (peace conferences / diplomacy target highlighting are
        // the likely users) is still unknown, so the generator below stays
        // for that future mode but the political map keeps the texture empty,
        // matching the game.
        if !OCCUPATION_STRIPES_ENABLED {
            return Ok(SecondaryRebuildStats {
                rebuilt: false,
                elapsed: Duration::ZERO,
                occupied_cells: 0,
            });
        }
        let signature = occupation_signature(owners, controllers, country_colors);
        if signature == self.last_signature {
            return Ok(SecondaryRebuildStats {
                rebuilt: false,
                elapsed: Duration::ZERO,
                occupied_cells: self.last_occupied_cells,
            });
        }

        let start = Instant::now();
        let generated = generate_secondary_color_map(
            self.width,
            self.height,
            &self.state_cells,
            owners,
            controllers,
            country_colors,
        )?;
        upload_texture(
            gfx,
            &self.texture,
            self.width,
            self.height,
            &generated.bgra,
            "ProvinceSecondaryColorMap",
        )?;
        let elapsed = start.elapsed();
        self.last_signature = signature;
        self.last_rebuild = Some(elapsed);
        self.last_occupied_cells = generated.occupied_cells;
        Ok(SecondaryRebuildStats {
            rebuilt: true,
            elapsed,
            occupied_cells: generated.occupied_cells,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SecondaryDimensions {
    pub(crate) width: usize,
    pub(crate) height: usize,
}

impl SecondaryDimensions {
    pub(crate) fn from_map(map_width: usize, map_height: usize) -> Result<Self> {
        if map_width == 0 || map_height == 0 || map_width % 2 != 0 || map_height % 2 != 0 {
            return Err(format!(
                "ProvinceSecondaryColorMap requires non-empty even map dimensions, got {map_width}x{map_height}"
            )
            .into());
        }
        Ok(Self {
            width: map_width / 2,
            height: map_height / 2,
        })
    }

    fn texture_byte_len(&self) -> Result<usize> {
        self.width
            .checked_mul(self.height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "ProvinceSecondaryColorMap byte length overflow".into())
    }
}

/// Off by default per user-confirmed game behavior (see [`SecondaryColorField::update`]).
/// Flip only when the engine's real fill rule for `ProvinceSecondaryColorMap`
/// is identified (candidate: peace-conference/diplomacy target striping).
pub(crate) const OCCUPATION_STRIPES_ENABLED: bool = false;

#[derive(Clone, Debug)]
pub(crate) struct GeneratedSecondaryColorMap {
    pub(crate) bgra: Vec<u8>,
    pub(crate) occupied_cells: usize,
}

pub(crate) fn generate_secondary_color_map(
    width: usize,
    height: usize,
    state_cells: &[u32],
    owners: &BTreeMap<u32, String>,
    controllers: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> Result<GeneratedSecondaryColorMap> {
    let expected_cells = width
        .checked_mul(height)
        .ok_or("ProvinceSecondaryColorMap cell count overflow")?;
    if state_cells.len() != expected_cells {
        return Err(format!(
            "ProvinceSecondaryColorMap state cell plane has {} cells, expected {expected_cells}",
            state_cells.len()
        )
        .into());
    }

    let mut bgra = vec![0u8; expected_cells * 4];
    let mut occupied_cells = 0usize;
    for (index, state_id) in state_cells.iter().copied().enumerate() {
        if state_id == 0 {
            continue;
        }
        let Some(owner) = owners.get(&state_id) else {
            continue;
        };
        let Some(controller) = controllers.get(&state_id) else {
            continue;
        };
        if owner == controller {
            continue;
        }
        // SEMANTIC-ASSUMPTION: political base color follows controller, and
        // controller != owner marks occupation with owner-side stripe color.
        // Validation: next live occupied-save capture of GradientBorderChannel
        // plus ProvinceSecondaryColorMap, with side-by-side screenshots.
        let Some(owner_color) = country_colors.get(owner) else {
            continue;
        };
        if !country_colors.contains_key(controller) {
            continue;
        }
        let out = index * 4;
        // Captured DXGI 91 memory is BGRA; shader sampling reads `.rgb`.
        bgra[out] = owner_color[2];
        bgra[out + 1] = owner_color[1];
        bgra[out + 2] = owner_color[0];
        bgra[out + 3] = 255;
        occupied_cells += 1;
    }

    Ok(GeneratedSecondaryColorMap {
        bgra,
        occupied_cells,
    })
}

fn occupation_signature(
    owners: &BTreeMap<u32, String>,
    controllers: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> u64 {
    let mut hash = FNV_OFFSET;
    tag_map_signature(&mut hash, b"owners", owners, country_colors);
    tag_map_signature(&mut hash, b"controllers", controllers, country_colors);
    hash
}

fn tag_map_signature(
    hash: &mut u64,
    label: &[u8],
    map: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) {
    fnv_bytes(hash, label);
    fnv_bytes(hash, &[0]);
    for (state_id, tag) in map {
        fnv_bytes(hash, &state_id.to_le_bytes());
        fnv_bytes(hash, tag.as_bytes());
        if let Some(color) = country_colors.get(tag) {
            fnv_bytes(hash, color);
        }
        fnv_bytes(hash, &[0]);
    }
}

fn fnv_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

unsafe fn create_secondary_texture(
    gfx: &Gfx,
    width: usize,
    height: usize,
    bytes: &[u8],
) -> Result<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("ProvinceSecondaryColorMap texture byte length overflow")?;
    if bytes.len() != expected {
        return Err(format!(
            "ProvinceSecondaryColorMap init has {} bytes, expected {expected}",
            bytes.len()
        )
        .into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width as UINT,
        Height: height as UINT,
        MipLevels: 1,
        ArraySize: 1,
        Format: PROVINCE_SECONDARY_DXGI_FORMAT,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: (width * 4) as UINT,
        SysMemSlicePitch: 0,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, &init, &mut texture);
    hr_ok(
        hr,
        "CreateTexture2D for generated ProvinceSecondaryColorMap",
    )?;
    let texture = ComPtr::from_raw(texture, "generated ProvinceSecondaryColorMap texture")?;
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateShaderResourceView(
        texture.as_raw() as *mut _,
        ptr::null(),
        &mut srv,
    );
    hr_ok(
        hr,
        "CreateShaderResourceView for generated ProvinceSecondaryColorMap",
    )?;
    let srv = ComPtr::from_raw(srv, "generated ProvinceSecondaryColorMap SRV")?;
    Ok((texture, srv))
}

fn upload_texture(
    gfx: &Gfx,
    texture: &ComPtr<ID3D11Texture2D>,
    width: usize,
    height: usize,
    bytes: &[u8],
    role: &str,
) -> Result<()> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("ProvinceSecondaryColorMap upload byte length overflow")?;
    if bytes.len() != expected {
        return Err(format!(
            "{role} upload has {} bytes, expected {expected}",
            bytes.len()
        )
        .into());
    }
    unsafe {
        (*gfx.context.as_raw()).UpdateSubresource(
            texture.as_raw() as *mut ID3D11Resource,
            0,
            ptr::null(),
            bytes.as_ptr() as *const _,
            (width * 4) as UINT,
            (width * height * 4) as UINT,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> BTreeMap<String, [u8; 3]> {
        BTreeMap::from([
            ("FRA".to_string(), [10, 20, 30]),
            ("GER".to_string(), [40, 50, 60]),
            ("ITA".to_string(), [70, 80, 90]),
        ])
    }

    fn owners() -> BTreeMap<u32, String> {
        BTreeMap::from([
            (10, "FRA".to_string()),
            (20, "GER".to_string()),
            (30, "ITA".to_string()),
        ])
    }

    #[test]
    fn captured_descriptor_matches_product_half_res_texture() {
        let dimensions = SecondaryDimensions::from_map(5632, 2048).expect("valid the game dimensions");
        assert_eq!(dimensions.width, PROVINCE_SECONDARY_CAPTURED_WIDTH);
        assert_eq!(dimensions.height, PROVINCE_SECONDARY_CAPTURED_HEIGHT);
        assert_eq!(PROVINCE_SECONDARY_DXGI_FORMAT, 91);
        assert_eq!(dimensions.texture_byte_len().expect("byte len"), 11_534_336);
    }

    #[test]
    fn encodes_owner_side_color_and_alpha_only_for_divergence() {
        let state_cells = vec![10, 20, 30, 0];
        let controllers = BTreeMap::from([
            (10, "GER".to_string()),
            (20, "GER".to_string()),
            (30, "FRA".to_string()),
        ]);
        let generated =
            generate_secondary_color_map(2, 2, &state_cells, &owners(), &controllers, &colors())
                .expect("generate secondary map");

        assert_eq!(generated.occupied_cells, 2);
        assert_eq!(&generated.bgra[0..4], &[30, 20, 10, 255]);
        assert_eq!(&generated.bgra[4..8], &[0, 0, 0, 0]);
        assert_eq!(&generated.bgra[8..12], &[90, 80, 70, 255]);
        assert_eq!(&generated.bgra[12..16], &[0, 0, 0, 0]);
    }

    #[test]
    fn missing_measured_side_or_color_fails_closed_to_zero_alpha() {
        let state_cells = vec![10, 20, 30, 40];
        let controllers = BTreeMap::from([
            (10, "GER".to_string()),
            (20, "ZZZ".to_string()),
            (40, "FRA".to_string()),
        ]);
        let generated =
            generate_secondary_color_map(4, 1, &state_cells, &owners(), &controllers, &colors())
                .expect("generate secondary map");

        assert_eq!(generated.occupied_cells, 1);
        assert_eq!(&generated.bgra[0..4], &[30, 20, 10, 255]);
        assert!(generated.bgra[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn signature_changes_when_controller_or_owner_color_changes() {
        let controllers = BTreeMap::from([(10, "GER".to_string())]);
        let baseline = occupation_signature(&owners(), &controllers, &colors());

        let mut changed_controllers = controllers.clone();
        changed_controllers.insert(10, "ITA".to_string());
        assert_ne!(
            baseline,
            occupation_signature(&owners(), &changed_controllers, &colors())
        );

        let mut changed_colors = colors();
        changed_colors.insert("FRA".to_string(), [1, 2, 3]);
        assert_ne!(
            baseline,
            occupation_signature(&owners(), &controllers, &changed_colors)
        );
    }
}
