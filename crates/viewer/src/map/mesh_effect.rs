//! Native material admission, before a tree selects record zero of its LOD.
use installed_content::{ResolvedLayers, mesh};
use shader_pipeline::library::Library;
use std::{io, sync::Arc};

pub struct Materials {
    library: Library,
}

impl Materials {
    pub fn new(layers: &ResolvedLayers, map: [u32; 2]) -> Self {
        Self {
            library: Library::new(Arc::new(layers.clone()))
                .with_globals(super::shader_sources::globals(map)),
        }
    }

    pub fn admitted(&mut self, material: &mesh::Material, sub: &mesh::SubMesh) -> io::Result<bool> {
        // 14224BA50 uses the string at142AE6A98; 14224BB00 replaces the
        // source for every variant only when meshsettings.shader_file is nonempty.
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
        // 14224BB00 uses the installed global fallback on a base lookup miss.
        // 14208CA80 registers these from142AE6A98 without material UV defines;
        // 142251E30 stores their handles. Only a missing fallback skips the record.
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

/// 14224F250 checks the authored key before applying any meshsettings override.
pub fn candidate(sub: &mesh::SubMesh) -> bool {
    sub.material
        .as_ref()
        .and_then(|m| m.shader.as_deref())
        .is_some_and(|s| s != "Collision")
}
