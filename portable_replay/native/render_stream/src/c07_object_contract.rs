use crate::UsedContentKey;
use serde::{Deserialize, Serialize};

pub const MAP_OBJECT_PARTITION_PRIMARY: u8 = 1;
pub const MAP_OBJECT_PARTITION_WRAP_WEST: u8 = 2;
pub const MAP_OBJECT_PARTITION_WRAP_EAST: u8 = 4;
pub const MAP_OBJECT_PARTITION_ALL: u8 =
    MAP_OBJECT_PARTITION_PRIMARY | MAP_OBJECT_PARTITION_WRAP_WEST | MAP_OBJECT_PARTITION_WRAP_EAST;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapObjectMaterialFamily {
    Standard,
    FixedMapFurnitureBorder,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapObjectDrawArguments {
    Draw {
        vertex_count: u32,
        start_vertex: u32,
    },
    DrawIndexed {
        index_count: u32,
        start_index: u32,
        base_vertex: i32,
    },
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index: u32,
        base_vertex: i32,
        start_instance: u32,
    },
}

impl MapObjectDrawArguments {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Draw { vertex_count, .. } if *vertex_count == 0 => {
                Err("map-object Draw has zero vertices".into())
            }
            Self::DrawIndexed { index_count, .. } if *index_count == 0 => {
                Err("map-object DrawIndexed has zero indices".into())
            }
            Self::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                ..
            } if *index_count_per_instance == 0 || *instance_count == 0 => {
                Err("map-object DrawIndexedInstanced has an empty dimension".into())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapObjectSubmeshContent {
    pub selected_material: String,
    pub material_source: UsedContentKey,
    pub material_family: MapObjectMaterialFamily,
    pub ordered_texture_sources: Vec<UsedContentKey>,
    pub draw: MapObjectDrawArguments,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapObjectMeshGroupContent {
    pub submeshes: Vec<MapObjectSubmeshContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapObjectRenderContent {
    pub entity_name: String,
    pub entity_source: UsedContentKey,
    pub selected_mesh: String,
    pub mesh_source: UsedContentKey,
    pub groups: Vec<MapObjectMeshGroupContent>,
    pub world_matrix_f32_bits: [u32; 16],
    pub skinned: bool,
}

impl MapObjectRenderContent {
    pub fn validate(&self, effective_vfs_manifest_sha256: &str) -> Result<(), String> {
        validate_sha256(effective_vfs_manifest_sha256, "effective-VFS manifest")?;
        if self.entity_name.trim().is_empty() || self.selected_mesh.trim().is_empty() {
            return Err("map-object entity or selected mesh identity is empty".into());
        }
        if self.groups.is_empty() {
            return Err("map-object selected mesh has no groups".into());
        }
        if self
            .world_matrix_f32_bits
            .iter()
            .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err("map-object world matrix contains a non-finite lane".into());
        }

        validate_content(&self.entity_source, effective_vfs_manifest_sha256)?;
        validate_content(&self.mesh_source, effective_vfs_manifest_sha256)?;
        for group in &self.groups {
            if group.submeshes.is_empty() {
                return Err("map-object mesh group has no submeshes".into());
            }
            for submesh in &group.submeshes {
                if submesh.selected_material.trim().is_empty() {
                    return Err("map-object selected material identity is empty".into());
                }
                validate_content(&submesh.material_source, effective_vfs_manifest_sha256)?;
                for texture in &submesh.ordered_texture_sources {
                    validate_content(texture, effective_vfs_manifest_sha256)?;
                }
                submesh.draw.validate()?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMapObjectDraw {
    pub visible_order: u32,
    pub stable_typed_id: String,
    pub partition_mask: u8,
    pub render: MapObjectRenderContent,
}

impl PreparedMapObjectDraw {
    pub fn validate(
        &self,
        expected_visible_order: u32,
        effective_vfs_manifest_sha256: &str,
    ) -> Result<(), String> {
        if self.visible_order != expected_visible_order {
            return Err("prepared map-object visible order is not contiguous".into());
        }
        if self.stable_typed_id.trim().is_empty()
            || self
                .stable_typed_id
                .bytes()
                .any(|byte| byte.is_ascii_whitespace())
        {
            return Err("prepared map-object stable typed identity is invalid".into());
        }
        if self.partition_mask == 0 || self.partition_mask & !MAP_OBJECT_PARTITION_ALL != 0 {
            return Err("prepared map-object partition mask is invalid".into());
        }
        self.render.validate(effective_vfs_manifest_sha256)
    }
}

fn validate_content(
    content: &UsedContentKey,
    effective_vfs_manifest_sha256: &str,
) -> Result<(), String> {
    content.validate()?;
    if content.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256 {
        return Err("map-object content belongs to a different effective-VFS manifest".into());
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} SHA-256 is not canonical lowercase hex"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn content(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "11".repeat(32),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base-game".into(),
            source_sha256: "22".repeat(32),
            payload_sha256: "33".repeat(32),
            payload_bytes: 4,
        }
    }

    fn prepared() -> PreparedMapObjectDraw {
        PreparedMapObjectDraw {
            visible_order: 0,
            stable_typed_id: "tree:17:379".into(),
            partition_mask: MAP_OBJECT_PARTITION_PRIMARY,
            render: MapObjectRenderContent {
                entity_name: "forest".into(),
                entity_source: content("gfx/entities/trees.gfx"),
                selected_mesh: "forestShape".into(),
                mesh_source: content("gfx/models/mapitems/trees/forest.mesh"),
                groups: vec![MapObjectMeshGroupContent {
                    submeshes: vec![MapObjectSubmeshContent {
                        selected_material: "forestMaterial".into(),
                        material_source: content("gfx/models/mapitems/trees/forest.mesh"),
                        material_family: MapObjectMaterialFamily::Tree,
                        ordered_texture_sources: vec![content(
                            "gfx/models/mapitems/trees/forest_diffuse.dds",
                        )],
                        draw: MapObjectDrawArguments::DrawIndexedInstanced {
                            index_count_per_instance: 6,
                            instance_count: 5,
                            start_index: 0,
                            base_vertex: 0,
                            start_instance: 0,
                        },
                    }],
                }],
                world_matrix_f32_bits: [
                    1.0f32.to_bits(),
                    0,
                    0,
                    0,
                    0,
                    1.0f32.to_bits(),
                    0,
                    0,
                    0,
                    0,
                    1.0f32.to_bits(),
                    0,
                    0,
                    0,
                    0,
                    1.0f32.to_bits(),
                ],
                skinned: false,
            },
        }
    }

    #[test]
    fn prepared_draw_accepts_one_exact_effective_vfs() {
        prepared().validate(0, &"11".repeat(32)).unwrap();
    }

    #[test]
    fn prepared_draw_rejects_cross_manifest_content_and_nonfinite_matrices() {
        let mut draw = prepared();
        draw.render.mesh_source.effective_vfs_manifest_sha256 = "44".repeat(32);
        assert!(draw.validate(0, &"11".repeat(32)).is_err());

        let mut draw = prepared();
        draw.render.world_matrix_f32_bits[3] = f32::NAN.to_bits();
        assert!(draw.validate(0, &"11".repeat(32)).is_err());
    }
}
