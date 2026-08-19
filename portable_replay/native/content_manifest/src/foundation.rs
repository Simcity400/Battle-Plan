use crate::{normalize_game_relative_path, UsedContentKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const FOUNDATION_CONTENT_REQUEST_SCHEMA: &str =
    "hoi4.replay.xmap02-foundation-content-request.v1";
pub const RESOLVED_FOUNDATION_CONTENT_SCHEMA: &str =
    "hoi4.replay.xmap02-resolved-foundation-content.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationContentRequest {
    pub schema: String,
    pub effect_file: String,
    pub effect_name: String,
    pub texture_file: String,
    pub texture_sampler: String,
    pub map_width: u32,
    pub map_height: u32,
    pub pow2_ratio_branch: bool,
    pub max_texture_dimension: u32,
}

impl FoundationContentRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != FOUNDATION_CONTENT_REQUEST_SCHEMA {
            return Err(format!(
                "unsupported foundation content request schema {}",
                self.schema
            ));
        }
        for (value, label) in [
            (&self.effect_file, "effect file"),
            (&self.texture_file, "texture file"),
        ] {
            let normalized = normalize_game_relative_path(value)?;
            if normalized != *value {
                return Err(format!("foundation {label} is not canonical: {value}"));
            }
        }
        for (value, label) in [
            (&self.effect_name, "effect name"),
            (&self.texture_sampler, "texture sampler"),
        ] {
            if value.is_empty()
                || value.len() > 256
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(format!("foundation {label} is not a portable identifier"));
            }
        }
        if self.map_width == 0 || self.map_height == 0 {
            return Err("foundation map dimensions must be nonzero".into());
        }
        if self.pow2_ratio_branch && self.max_texture_dimension == 0 {
            return Err(
                "foundation pow2-ratio branch requires a nonzero texture dimension limit".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundationShaderStage {
    Vertex,
    Pixel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationShaderIdentity {
    pub stage: FoundationShaderStage,
    pub profile: String,
    pub entry_point: String,
    pub translation_unit_sha256: String,
    pub translation_unit_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationSamplerDescriptor {
    pub name: String,
    pub shader_register: u32,
    pub resource_type: String,
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
    pub mip_map_lod_bias: Option<String>,
    pub mip_map_min_lod: Option<String>,
    pub mip_map_max_lod: Option<String>,
    pub max_anisotropy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationTextureDescriptor {
    pub dimension: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub cube: bool,
    pub resource_identity_sha256: String,
    pub shader_resource_view_identity_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedFoundationContent {
    pub schema: String,
    pub request: FoundationContentRequest,
    pub manifest_digest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub effect_defines: Vec<String>,
    pub blend_state: Option<Vec<(String, String)>>,
    pub depth_stencil_state: Option<Vec<(String, String)>>,
    pub rasterizer_state: Option<Vec<(String, String)>>,
    pub vertex_shader: FoundationShaderIdentity,
    pub pixel_shader: FoundationShaderIdentity,
    pub texture_sampler: FoundationSamplerDescriptor,
    pub texture: FoundationTextureDescriptor,
    pub operation_identity_sha256: String,
}

impl ResolvedFoundationContent {
    pub fn computed_operation_identity_sha256(&self) -> Result<String, String> {
        let mut copy = self.clone();
        copy.operation_identity_sha256.clear();
        let bytes = serde_json::to_vec(&copy)
            .map_err(|error| format!("encode foundation content identity: {error}"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != RESOLVED_FOUNDATION_CONTENT_SCHEMA {
            return Err(format!(
                "unsupported resolved foundation content schema {}",
                self.schema
            ));
        }
        self.request.validate()?;
        validate_sha256(&self.manifest_digest_sha256, "manifest digest")?;
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_sha256(
            &self.effective_vfs_manifest_sha256,
            "effective VFS manifest",
        )?;
        validate_sha256(
            &self.texture.resource_identity_sha256,
            "texture resource identity",
        )?;
        validate_sha256(
            &self.texture.shader_resource_view_identity_sha256,
            "texture SRV identity",
        )?;
        validate_sha256(&self.operation_identity_sha256, "operation identity")?;
        if self.ordered_used_content.is_empty() {
            return Err("foundation content closure is empty".into());
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content.validate()?;
            if content.effective_vfs_manifest_sha256 != self.effective_vfs_manifest_sha256 {
                return Err("foundation content row names a different effective VFS".into());
            }
            if !paths.insert(content.normalized_game_relative_path.clone()) {
                return Err("foundation content closure repeats a logical path".into());
            }
        }
        for required in [&self.request.effect_file, &self.request.texture_file] {
            if !paths.contains(required) {
                return Err(format!(
                    "foundation content closure omits requested input {required}"
                ));
            }
        }
        if self.texture_sampler.name != self.request.texture_sampler {
            return Err("foundation sampler descriptor does not match the request".into());
        }
        if self.vertex_shader.stage != FoundationShaderStage::Vertex
            || self.pixel_shader.stage != FoundationShaderStage::Pixel
        {
            return Err("foundation shader stages are not vertex/pixel ordered".into());
        }
        for shader in [&self.vertex_shader, &self.pixel_shader] {
            validate_sha256(
                &shader.translation_unit_sha256,
                "shader translation-unit identity",
            )?;
            if shader.translation_unit_bytes == 0
                || shader.entry_point.is_empty()
                || shader.profile.is_empty()
            {
                return Err("foundation shader identity is incomplete".into());
            }
        }
        if self.texture.width == 0
            || self.texture.height == 0
            || self.texture.depth == 0
            || self.texture.mip_levels == 0
            || self.texture.array_size == 0
        {
            return Err("foundation texture descriptor has a zero extent".into());
        }
        let expected = self.computed_operation_identity_sha256()?;
        if self.operation_identity_sha256 != expected {
            return Err("foundation operation identity does not match its payload".into());
        }
        Ok(())
    }

    pub fn with_computed_operation_identity(mut self) -> Result<Self, String> {
        self.operation_identity_sha256 = self.computed_operation_identity_sha256()?;
        self.validate()?;
        Ok(self)
    }
}

pub fn identity_sha256<T: Serialize>(value: &T, label: &str) -> Result<String, String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| format!("encode {label} identity: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} is not a SHA-256"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_rejects_machine_paths_and_zero_dimensions() {
        let mut request = FoundationContentRequest {
            schema: FOUNDATION_CONTENT_REQUEST_SCHEMA.into(),
            effect_file: "C:/game/gfx/fx/pdxmap.shader".into(),
            effect_name: "underwater".into(),
            texture_file: "map/terrain/underwater_terrain_0.dds".into(),
            texture_sampler: "TerrainDiffuse".into(),
            map_width: 5632,
            map_height: 2048,
            pow2_ratio_branch: false,
            max_texture_dimension: 0,
        };
        assert!(request.validate().is_err());
        request.effect_file = "gfx/fx/pdxmap.shader".into();
        request.map_width = 0;
        assert!(request.validate().is_err());
    }
}
