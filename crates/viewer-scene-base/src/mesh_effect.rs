use content_reader_layers::ResolvedLayers;
use content_reader_mesh::mesh;
use content_reader_image::native::Decode;
use shader_pipeline::library::Library;
use std::{io, sync::Arc};

pub struct Materials {
    library: Library,
}

impl Materials {
    pub fn new(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> Self {
        Self {
            library: Library::new(Arc::new(layers.clone()))
                .with_globals(super::shader_sources::globals(map, decode)),
        }
    }

    pub fn admitted(&mut self, material: &mesh::Material, sub: &mesh::SubMesh) -> io::Result<bool> {
        let file = material
            .shader_file
            .as_deref()
            .unwrap_or("gfx/FX/pdxmesh.lua");
        let defines = if sub.uv1.is_some() {
            vec!["PDX_MESH_UV1".to_owned()]
        } else {
            Vec::new()
        };
        let effect = self
            .library
            .load(file.as_bytes(), &defines)
            .map_err(io::Error::other)?;
        let skinned = sub.skin.is_some();
        let base = material.shader.as_deref().unwrap_or_default();
        let name = format!("{base}{}", if skinned { "Skinned" } else { "" });
        if !base.is_empty() && effect.select(&name).map_err(io::Error::other)?.is_some() {
            return Ok(true);
        }
        let fallback = self
            .library
            .load(b"gfx/FX/pdxmesh.lua", &[])
            .map_err(io::Error::other)?;
        let name = if skinned {
            "PdxMeshColorSkinned"
        } else {
            "PdxMeshColor"
        };
        Ok(fallback.select(name).map_err(io::Error::other)?.is_some())
    }
}

pub fn candidate(sub: &mesh::SubMesh) -> bool {
    sub.material
        .as_ref()
        .and_then(|m| m.shader.as_deref())
        .is_some_and(|s| s != "Collision")
}
