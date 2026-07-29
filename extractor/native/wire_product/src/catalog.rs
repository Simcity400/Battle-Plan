use crate::model::ProductError;
use portable_content_manifest::{ContentManifest, RequiredContentKind};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFINITION_CSV_PATH: &str = "map/definition.csv";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvinceBinding {
    pub(crate) province_id: u32,
    pub(crate) symbol: String,
}

/// A province-id resolver admitted by the exact portable content manifest.
///
/// Symbols use the definition's unique RGB identity, not the recorder-local
/// numeric id: `province/rgb/R/G/B`. The source id remains bound under
/// `hoi4.province` only inside the symbol-manifest resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedProvinceCatalog {
    pub(crate) content_manifest_sha256: [u8; 32],
    pub(crate) definition_csv_sha256: [u8; 32],
    pub(crate) bindings: Vec<ProvinceBinding>,
    pub(crate) definition_csv: Vec<u8>,
}

pub(crate) struct ProvinceCatalogArtifact {
    pub content_manifest_sha256: [u8; 32],
    pub definition_csv_sha256: [u8; 32],
    pub bindings: Vec<ProvinceBinding>,
}

impl AuthenticatedProvinceCatalog {
    pub fn content_manifest_sha256(&self) -> [u8; 32] {
        self.content_manifest_sha256
    }

    pub fn definition_csv_sha256(&self) -> [u8; 32] {
        self.definition_csv_sha256
    }

    pub fn province_count(&self) -> usize {
        self.bindings.len()
    }

    pub fn encode_artifact(&self) -> Result<Vec<u8>, ProductError> {
        let definition_csv_sha256: [u8; 32] = Sha256::digest(&self.definition_csv).into();
        if definition_csv_sha256 != self.definition_csv_sha256
            || parse_definition_csv(&self.definition_csv)? != self.bindings
        {
            return Err(ProductError::new(
                "authenticated province catalog no longer matches its definition.csv authority",
            ));
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"H4PC");
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&self.content_manifest_sha256);
        out.extend_from_slice(&self.definition_csv_sha256);
        out.extend_from_slice(&(self.definition_csv.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.definition_csv);
        Ok(out)
    }

    /// Reauthenticate a persisted catalog against the exact admitted
    /// `definition.csv` membership in an authenticated content manifest.
    ///
    /// The artifact alone is never an authority-bearing constructor.
    pub fn from_artifact(
        content_manifest_json: &[u8],
        bytes: &[u8],
    ) -> Result<Self, ProductError> {
        let untrusted = decode_artifact(bytes)?;
        let definition_csv = artifact_definition_bytes(bytes)?;
        let authenticated = Self::from_content_manifest(content_manifest_json, definition_csv)?;
        if authenticated.content_manifest_sha256 != untrusted.content_manifest_sha256
            || authenticated.definition_csv_sha256 != untrusted.definition_csv_sha256
            || authenticated.bindings != untrusted.bindings
            || authenticated.encode_artifact()? != bytes
        {
            return Err(ProductError::new(
                "persisted province catalog differs from authenticated manifest authority",
            ));
        }
        Ok(authenticated)
    }

    pub fn union(catalogs: &[Self]) -> Result<Self, ProductError> {
        let first = catalogs
            .first()
            .ok_or_else(|| ProductError::new("cannot union zero province catalogs"))?;
        let canonical = first.encode_artifact()?;
        for catalog in catalogs.iter().skip(1) {
            if catalog.encode_artifact()? != canonical {
                return Err(ProductError::new(
                    "stitched province catalogs are not the same canonical authenticated artifact",
                ));
            }
        }
        Ok(first.clone())
    }

    pub fn from_content_manifest(
        content_manifest_json: &[u8],
        definition_csv: &[u8],
    ) -> Result<Self, ProductError> {
        let manifest = ContentManifest::decode_json(content_manifest_json)
            .map_err(|error| ProductError::new(format!("content manifest rejected: {error}")))?;
        let mut matching = manifest.required_content.iter().filter(|required| {
            required.content.normalized_game_relative_path == DEFINITION_CSV_PATH
        });
        let required = matching.next().ok_or_else(|| {
            ProductError::new(
                "content manifest does not admit the effective-VFS map/definition.csv",
            )
        })?;
        if matching.next().is_some() {
            return Err(ProductError::new(
                "content manifest admits map/definition.csv more than once",
            ));
        }
        if required.kind != RequiredContentKind::Definition {
            return Err(ProductError::new(
                "map/definition.csv is not admitted as definition content",
            ));
        }
        let admitted_content = required.content.clone();
        Self::from_validated_manifest_and_resolution(
            manifest,
            &admitted_content,
            definition_csv,
        )
    }

    fn from_validated_manifest_and_resolution(
        manifest: ContentManifest,
        resolution: &portable_content_manifest::UsedContentKey,
        definition_csv: &[u8],
    ) -> Result<Self, ProductError> {
        // `resolution` is reachable only through an exact `required_content`
        // row selected from the parsed manifest above. A naked, externally
        // fabricated UsedContentKey can never enter this authority boundary.
        resolution.validate().map_err(|error| {
            ProductError::new(format!("definition.csv content admission rejected: {error}"))
        })?;
        if resolution.normalized_game_relative_path != DEFINITION_CSV_PATH {
            return Err(ProductError::new(format!(
                "province catalog resolution names {:?}, expected {DEFINITION_CSV_PATH}",
                resolution.normalized_game_relative_path
            )));
        }
        if resolution.effective_vfs_manifest_sha256 != manifest.effective_vfs_manifest_sha256 {
            return Err(ProductError::new(
                "definition.csv admission belongs to a different effective-VFS manifest",
            ));
        }
        let layer = manifest
            .ordered_layers
            .iter()
            .find(|layer| layer.ordinal == resolution.winning_layer_ordinal)
            .ok_or_else(|| {
                ProductError::new("definition.csv winning layer ordinal is absent from manifest")
            })?;
        if layer.layer_kind != resolution.layer_kind || layer.layer_id != resolution.layer_id {
            return Err(ProductError::new(
                "definition.csv winning layer identity disagrees with manifest admission",
            ));
        }
        let actual_len = u64::try_from(definition_csv.len())
            .map_err(|_| ProductError::new("definition.csv length does not fit u64"))?;
        if resolution.payload_bytes != actual_len {
            return Err(ProductError::new(format!(
                "definition.csv byte length {} does not match admitted {}",
                actual_len, resolution.payload_bytes
            )));
        }
        let definition_csv_sha256: [u8; 32] = Sha256::digest(definition_csv).into();
        let expected_definition = decode_sha256(&resolution.payload_sha256)?;
        if definition_csv_sha256 != expected_definition {
            return Err(ProductError::new(
                "definition.csv SHA-256 does not match its manifest admission",
            ));
        }
        let manifest_digest = manifest.sha256().map_err(|error| {
            ProductError::new(format!("compute content manifest digest: {error}"))
        })?;
        let content_manifest_sha256 = decode_sha256(&manifest_digest)?;
        let bindings = parse_definition_csv(definition_csv)?;
        Ok(Self {
            content_manifest_sha256,
            definition_csv_sha256,
            bindings,
            definition_csv: definition_csv.to_vec(),
        })
    }
}

pub(crate) fn decode_artifact(bytes: &[u8]) -> Result<ProvinceCatalogArtifact, ProductError> {
        if bytes.len() < 80 || &bytes[..4] != b"H4PC" {
            return Err(ProductError::new("province catalog artifact is truncated or has wrong magic"));
        }
        if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) != 2
            || bytes[6..8] != [0, 0]
        {
            return Err(ProductError::new("province catalog artifact version/reserved bytes invalid"));
        }
        let content_manifest_sha256 = bytes[8..40].try_into().unwrap();
        let definition_csv_sha256 = bytes[40..72].try_into().unwrap();
        let definition_len = usize::try_from(u64::from_le_bytes(
            bytes[72..80].try_into().unwrap(),
        ))
        .map_err(|_| ProductError::new("province catalog definition length exceeds usize"))?;
        if definition_len != bytes.len().saturating_sub(80) {
            return Err(ProductError::new(
                "province catalog definition length is noncanonical",
            ));
        }
        let definition_csv = artifact_definition_bytes(bytes)?;
        if Sha256::digest(definition_csv).as_slice() != definition_csv_sha256 {
            return Err(ProductError::new(
                "province catalog definition digest does not match embedded bytes",
            ));
        }
        let bindings = parse_definition_csv(definition_csv)?;
        Ok(ProvinceCatalogArtifact {
            content_manifest_sha256,
            definition_csv_sha256,
            bindings,
        })
    }

fn artifact_definition_bytes(bytes: &[u8]) -> Result<&[u8], ProductError> {
    if bytes.len() < 80 {
        return Err(ProductError::new("province catalog artifact is truncated"));
    }
    Ok(&bytes[80..])
}

fn parse_definition_csv(bytes: &[u8]) -> Result<Vec<ProvinceBinding>, ProductError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ProductError::new("definition.csv is not valid UTF-8"))?;
    let mut by_id = BTreeMap::new();
    let mut rgb_seen = BTreeSet::new();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        if fields.len() < 8 {
            return Err(ProductError::new(format!(
                "definition.csv line {} has {} fields, expected at least 8",
                line_index + 1,
                fields.len()
            )));
        }
        let parse = |index: usize, label: &str| -> Result<u32, ProductError> {
            fields[index].parse::<u32>().map_err(|_| {
                ProductError::new(format!(
                    "definition.csv line {} has invalid {label}",
                    line_index + 1
                ))
            })
        };
        let province_id = parse(0, "province id")?;
        if province_id == 0 {
            return Err(ProductError::new(format!(
                "definition.csv line {} uses reserved province id zero",
                line_index + 1
            )));
        }
        let red = parse(1, "red component")?;
        let green = parse(2, "green component")?;
        let blue = parse(3, "blue component")?;
        if red > 255 || green > 255 || blue > 255 {
            return Err(ProductError::new(format!(
                "definition.csv line {} has RGB outside 0..=255",
                line_index + 1
            )));
        }
        if !rgb_seen.insert((red, green, blue)) {
            return Err(ProductError::new(format!(
                "definition.csv line {} repeats RGB {red},{green},{blue}",
                line_index + 1
            )));
        }
        let symbol = format!("province/rgb/{red}/{green}/{blue}");
        if by_id
            .insert(
                province_id,
                ProvinceBinding {
                    province_id,
                    symbol,
                },
            )
            .is_some()
        {
            return Err(ProductError::new(format!(
                "definition.csv repeats province id {province_id}"
            )));
        }
    }
    if by_id.is_empty() {
        return Err(ProductError::new(
            "definition.csv contains no province definitions",
        ));
    }
    Ok(by_id.into_values().collect())
}

pub(crate) fn decode_sha256(value: &str) -> Result<[u8; 32], ProductError> {
    if value.len() != 64 {
        return Err(ProductError::new("SHA-256 text is not 64 hexadecimal bytes"));
    }
    let mut out = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair)
            .map_err(|_| ProductError::new("SHA-256 text is not ASCII"))?;
        out[index] = u8::from_str_radix(text, 16)
            .map_err(|_| ProductError::new("SHA-256 text is not hexadecimal"))?;
    }
    Ok(out)
}
