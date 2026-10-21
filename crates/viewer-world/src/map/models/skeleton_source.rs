//! Geometry-free source retained by an entity animation rig.
use installed_content_core::mesh;

pub(super) fn skeleton(decoded: &mesh::Mesh) -> mesh::Mesh {
    mesh::Mesh {
        lod_distances: decoded.lod_distances.clone(),
        shapes: Vec::new(),
        skeleton: decoded.skeleton.clone(),
        locators: decoded.locators.clone(),
    }
}

#[cfg(test)]
mod tests;
