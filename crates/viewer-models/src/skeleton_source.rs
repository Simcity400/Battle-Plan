use content_reader_mesh::mesh;

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
