
use crate::{GameFileResolver, MapBuildingPlacement, Result};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;

pub const O05_MAP_BUILDINGS_SOURCE: &str = "map/buildings.txt";

pub const O05_FIXED_OBJECT_SOURCE_HASHES: &[(&str, &str)] = &[
    (
        "map/ambient_object.txt",
        "4eb1a8378b91fcb463810e84f34ad5b3023f78e7bd877d1bba07fb42e1187f72",
    ),
    (
        "gfx/entities/mapitems.asset",
        "594cf3bed1bc78686f6e01f1ed0484effcdec7922b63bf05f3adcc2302491d09",
    ),
    (
        "gfx/entities/mapitems.gfx",
        "15a0e31b76284139d20be4f69b87cf13de764744e536c01f7d7de587df09cdce",
    ),
    (
        O05_MAP_BUILDINGS_SOURCE,
        "4a336b6245026fcc693ed2be7ae316a164135585aec945ea190e32b8e65f4094",
    ),
    (
        "history/states/9-Czechoslovakia.txt",
        "0305620291a5478cedb34f4c710c50b1e7cf2810baedb6f91d5a59706568cc45",
    ),
    (
        "common/buildings/01_landmark_buildings.txt",
        "9528c42836e08ba01019d62d42607d0d5fb2473b8aa7aa7cb852d95e7a08002c",
    ),
    (
        "gfx/entities/landmarks.asset",
        "27b106b4b27716b92884099a96ed8d5be2d155ac9cb2858bf7829dc277b37289",
    ),
    (
        "gfx/entities/landmarks.gfx",
        "b5006bad950680621dee52387811ded3d2043d5c584b0d43bc8f4f32d5d53e71",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum O05FixedObjectClass {
    NonBuildingMapObject,
    Landmark,
}

impl O05FixedObjectClass {
    pub fn evidence_label(self) -> &'static str {
        match self {
            Self::NonBuildingMapObject => "non_building_map_object",
            Self::Landmark => "landmark",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct O05FixedObjectAdmission {
    pub semantic_id: String,
    pub class: O05FixedObjectClass,
    pub retained_oracle_ordinal: u32,
    pub source_mesh: String,
    pub shape: String,
    pub source_position: [f32; 3],
    pub source_rotation: f32,
    pub axis_scales: [f32; 3],
    pub world_matrix_raw: Vec<u8>,
}

impl O05FixedObjectAdmission {
    pub fn world_matrix_f32(&self) -> [[f32; 4]; 4] {
        let mut out = [[0.0; 4]; 4];
        for (index, chunk) in self.world_matrix_raw.chunks_exact(4).enumerate() {
            out[index / 4][index % 4] =
                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct O05StaticBuildingSourceIdentity {
    pub semantic_id: String,
    pub source_sha256: String,
    pub source_line: usize,
    pub placement: MapBuildingPlacement,
}

struct FixedAuthorityRow {
    semantic_id: &'static str,
    class: O05FixedObjectClass,
    ordinal: u32,
    mesh: &'static str,
    shape: &'static str,
    position: [f32; 3],
    rotation: f32,
    scales: [f32; 3],
    matrix_hex: &'static str,
}

const FIXED_AUTHORITY: &[FixedAuthorityRow] = &[
    FixedAuthorityRow {
        semantic_id: "map/ambient_object.txt@4eb1a8378b91fcb463810e84f34ad5b3023f78e7bd877d1bba07fb42e1187f72#object:frame_border_bottom_entity_bottom",
        class: O05FixedObjectClass::NonBuildingMapObject,
        ordinal: 41,
        mesh: "gfx/models/border_bottom_mesh.mesh",
        shape: "locator4Shape",
        position: [0.0, 0.0, -140.0],
        rotation: 0.0,
        scales: [100.0, 100.0, 100.0],
        matrix_hex: "0000c842000000000000000000000000000000000000c842000000000000000000000000000000000000c84200000000000000000000000000000cc30000803f",
    },
    FixedAuthorityRow {
        semantic_id: "map/buildings.txt@4a336b6245026fcc693ed2be7ae316a164135585aec945ea190e32b8e65f4094#L66261+state:9+province:11542+building:landmark_prague_castle",
        class: O05FixedObjectClass::Landmark,
        ordinal: 43,
        mesh: "gfx/models/buildings/landmarks/landmark_prague_castle.mesh",
        shape: "landmark_prague_castle_modelShape",
        position: [3031.29, 11.3, 1525.67],
        rotation: 5.96,
        scales: [0.11999999335423293, 0.11999999731779099, 0.11999999335423293],
        matrix_hex: "6209e93d00000000e3191cbd00000000000000008fc2f53d0000000000000000e3191c3d000000006209e93d00000000a4743d45cdcc344171b5be440000803f",
    },
    FixedAuthorityRow {
        semantic_id: "map/ambient_object.txt@4eb1a8378b91fcb463810e84f34ad5b3023f78e7bd877d1bba07fb42e1187f72#object:frame_border_logo_entity_top",
        class: O05FixedObjectClass::NonBuildingMapObject,
        ordinal: 44,
        mesh: "gfx/models/hoi4_logo.mesh",
        shape: "pCube3Shape",
        position: [3000.0, -1.0, 2130.0],
        rotation: 0.0,
        scales: [300.0, 300.0, 300.0],
        matrix_hex: "00009643000000000000000000000000000000000000964300000000000000000000000000000000000096430000000000803b45000080bf002005450000803f",
    },
    FixedAuthorityRow {
        semantic_id: "map/ambient_object.txt@4eb1a8378b91fcb463810e84f34ad5b3023f78e7bd877d1bba07fb42e1187f72#object:frame_border_entity_top",
        class: O05FixedObjectClass::NonBuildingMapObject,
        ordinal: 49,
        mesh: "gfx/models/border_mesh.mesh",
        shape: "locator5Shape",
        position: [0.0, 0.0, 2190.0],
        rotation: 0.0,
        scales: [100.0, 100.0, 100.0],
        matrix_hex: "0000c842000000000000000000000000000000000000c842000000000000000000000000000000000000c84200000000000000000000000000e008450000803f",
    },
];

pub fn o05_fixed_object_source_hashes_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    for (source, _) in O05_FIXED_OBJECT_SOURCE_HASHES {
        let path = resolver.resolve_required(source, "O05 fixed-object admission source")?;
        let bytes = resolver.read_required_bytes(source, source).map_err(|error| {
            format!(
                "O05 fixed-object admission could not read {}: {error}",
                path.display()
            )
        })?;
        hashes.insert((*source).to_string(), Sha256::hex(&bytes));
    }
    Ok(hashes)
}

pub fn admit_o05_fixed_objects(
    source_hashes: &BTreeMap<String, String>,
) -> Result<Vec<O05FixedObjectAdmission>> {
    for (source, expected) in O05_FIXED_OBJECT_SOURCE_HASHES {
        let actual = source_hashes.get(*source).ok_or_else(|| {
            format!("O05 fixed-object admission blocked: required source hash missing for {source}")
        })?;
        if actual != expected {
            return Err(format!(
                "O05 fixed-object admission blocked: {source} SHA-256 {actual} != frozen {expected}"
            )
            .into());
        }
    }
    FIXED_AUTHORITY
        .iter()
        .map(|row| {
            Ok(O05FixedObjectAdmission {
                semantic_id: row.semantic_id.to_string(),
                class: row.class,
                retained_oracle_ordinal: row.ordinal,
                source_mesh: row.mesh.to_string(),
                shape: row.shape.to_string(),
                source_position: row.position,
                source_rotation: row.rotation,
                axis_scales: row.scales,
                world_matrix_raw: decode_hex(row.matrix_hex)?,
            })
        })
        .collect()
}

pub fn admit_o05_fixed_objects_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<O05FixedObjectAdmission>> {
    let hashes = o05_fixed_object_source_hashes_from_resolver(resolver)?;
    admit_o05_fixed_objects(&hashes)
}

pub fn o05_static_building_source_identities_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<O05StaticBuildingSourceIdentity>> {
    let path =
        resolver.resolve_required(O05_MAP_BUILDINGS_SOURCE, "O05 static-building identity")?;
    let bytes =
        resolver.read_required_bytes(O05_MAP_BUILDINGS_SOURCE, O05_MAP_BUILDINGS_SOURCE).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    o05_static_building_source_identities_from_source(&bytes)
}

pub fn o05_static_building_source_identities_from_source(
    source_bytes: &[u8],
) -> Result<Vec<O05StaticBuildingSourceIdentity>> {
    let source_sha256 = Sha256::hex(source_bytes);
    let text: String = source_bytes.iter().map(|byte| char::from(*byte)).collect();
    let mut out = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let source_line = index + 1;
        let row = raw_line.split('#').next().unwrap_or_default().trim();
        if row.is_empty() {
            continue;
        }
        let fields = row.split(';').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 7 || fields.iter().any(|field| field.is_empty()) {
            return Err(format!(
                "{O05_MAP_BUILDINGS_SOURCE} line {source_line}: expected exactly seven non-empty semicolon fields"
            )
            .into());
        }
        let parse_u32 = |value: &str, name: &str| -> Result<u32> {
            value.parse::<u32>().map_err(|error| {
                format!("{O05_MAP_BUILDINGS_SOURCE} line {source_line}: {name} is not u32: {error}")
                    .into()
            })
        };
        let parse_f32 = |value: &str, name: &str| -> Result<f32> {
            value.parse::<f32>().map_err(|error| {
                format!("{O05_MAP_BUILDINGS_SOURCE} line {source_line}: {name} is not f32: {error}")
                    .into()
            })
        };
        out.push(O05StaticBuildingSourceIdentity {
            semantic_id: format!("{O05_MAP_BUILDINGS_SOURCE}@{source_sha256}#L{source_line}"),
            source_sha256: source_sha256.clone(),
            source_line,
            placement: MapBuildingPlacement {
                state_id: parse_u32(fields[0], "state_id")?,
                building: fields[1].to_string(),
                pos: [
                    parse_f32(fields[2], "x")?,
                    parse_f32(fields[3], "y")?,
                    parse_f32(fields[4], "z")?,
                ],
                rotation: parse_f32(fields[5], "rotation")?,
                adjacent_sea_province: parse_u32(fields[6], "adjacent_sea_province")?,
            },
        });
    }
    Ok(out)
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if value.len() != 128 || !value.len().is_multiple_of(2) {
        return Err("O05 frozen WorldMatrix hex must be exactly 64 bytes".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|error| format!("O05 frozen WorldMatrix hex is invalid: {error}").into())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frozen_hashes() -> BTreeMap<String, String> {
        O05_FIXED_OBJECT_SOURCE_HASHES
            .iter()
            .map(|(source, hash)| ((*source).to_string(), (*hash).to_string()))
            .collect()
    }

    #[test]
    fn static_identity_is_raw_source_hash_plus_one_based_line() {
        let bytes = b"# header\n9;landmark_spawn;3031.29;11.30;1525.67;5.96;0\n";
        let rows = o05_static_building_source_identities_from_source(bytes).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_line, 2);
        assert_eq!(
            rows[0].semantic_id,
            format!("map/buildings.txt@{}#L2", Sha256::hex(bytes))
        );
        assert_eq!(rows[0].placement.state_id, 9);
        assert_eq!(rows[0].placement.pos, [3031.29, 11.3, 1525.67]);
    }

    #[test]
    fn static_identity_parser_rejects_malformed_rows() {
        let error = o05_static_building_source_identities_from_source(
            b"9;landmark_spawn;3031.29;11.30;1525.67;5.96\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("exactly seven"));
    }

    #[test]
    fn fixed_admission_returns_only_the_four_frozen_rows() {
        let rows = admit_o05_fixed_objects(&frozen_hashes()).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter()
                .map(|row| row.retained_oracle_ordinal)
                .collect::<Vec<_>>(),
            vec![41, 43, 44, 49]
        );
        assert!(rows.iter().all(|row| row.world_matrix_raw.len() == 64));
        assert_eq!(rows[0].world_matrix_f32()[3], [0.0, 0.0, -140.0, 1.0]);
        assert_eq!(rows[2].world_matrix_f32()[3], [3000.0, -1.0, 2130.0, 1.0]);
    }

    #[test]
    fn fixed_admission_rejects_any_source_drift() {
        let mut hashes = frozen_hashes();
        hashes.insert(
            "map/ambient_object.txt".to_string(),
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        );
        let error = admit_o05_fixed_objects(&hashes).unwrap_err();
        assert!(error.to_string().contains("admission blocked"));
        assert!(error.to_string().contains("map/ambient_object.txt"));
    }
}
