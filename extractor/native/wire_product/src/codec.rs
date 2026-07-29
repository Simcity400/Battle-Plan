use crate::model::{
    ArtifactRole, BranchParent, BranchSpec, BundleEntryDescriptor, ProductError,
};
use std::collections::{BTreeMap, BTreeSet};

const BUNDLE_MAGIC: &[u8; 4] = b"H4WB";
const BRANCH_MAGIC: &[u8; 4] = b"H4BR";
const SCHEMA_MAGIC: &[u8; 4] = b"H4SR";
const VERSION: u16 = 1;
pub(crate) const MAX_ARTIFACTS: usize = 1_000_000;
const MAX_BRANCHES: usize = 1_000_000;
const MAX_SCHEMAS: usize = 65_536;
const MAX_PATH_BYTES: usize = 4096;
const MAX_SCHEMA_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactDescriptor {
    pub path: String,
    pub role: ArtifactRole,
    pub len: u64,
    pub blake3: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BundleMetadata {
    pub ticks_per_hour_log2: u8,
    pub content_manifest_sha256: [u8; 32],
    pub definition_csv_sha256: [u8; 32],
    pub artifacts: Vec<ArtifactDescriptor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SchemaDefinition {
    pub domain: u32,
    pub schema_hash: u64,
    pub canonical_bytes: Vec<u8>,
}

pub(crate) fn encode_bundle_metadata_entry_descriptors(
    content_manifest_sha256: [u8; 32],
    definition_csv_sha256: [u8; 32],
    entries: &[BundleEntryDescriptor],
) -> Result<Vec<u8>, ProductError> {
    encode_bundle_metadata_descriptors(
        content_manifest_sha256,
        definition_csv_sha256,
        entries
            .iter()
            .map(|entry| (&entry.path, entry.role, entry.len, entry.blake3)),
    )
}

fn encode_bundle_metadata_descriptors<'a>(
    content_manifest_sha256: [u8; 32],
    definition_csv_sha256: [u8; 32],
    entries: impl IntoIterator<Item = (&'a String, ArtifactRole, u64, [u8; 32])>,
) -> Result<Vec<u8>, ProductError> {
    let mut descriptors = entries
        .into_iter()
        .filter(|(_, role, _, _)| *role != ArtifactRole::BundleMetadata)
        .map(|(path, role, len, blake3)| ArtifactDescriptor {
            path: path.clone(),
            role,
            len,
            blake3,
        })
        .collect::<Vec<_>>();
    if descriptors.len() > MAX_ARTIFACTS {
        return Err(ProductError::new(format!(
            "rebuilt-wire metadata exceeds the {MAX_ARTIFACTS} artifact production bound"
        )));
    }
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    if descriptors.windows(2).any(|pair| pair[0].path == pair[1].path) {
        return Err(ProductError::new(
            "rebuilt-wire artifact paths are not unique",
        ));
    }
    let mut out = Vec::new();
    preamble(&mut out, BUNDLE_MAGIC);
    out.push(0); // hourly tick grid
    out.extend_from_slice(&[0; 7]);
    out.extend_from_slice(&content_manifest_sha256);
    out.extend_from_slice(&definition_csv_sha256);
    put_u64(&mut out, descriptors.len() as u64);
    for descriptor in descriptors {
        put_string(&mut out, &descriptor.path)?;
        out.push(descriptor.role as u8);
        out.extend_from_slice(&[0; 7]);
        put_u64(&mut out, descriptor.len);
        out.extend_from_slice(&descriptor.blake3);
    }
    Ok(out)
}

pub(crate) fn decode_bundle_metadata(bytes: &[u8]) -> Result<BundleMetadata, ProductError> {
    let mut cursor = Cursor::new(bytes);
    cursor.preamble(BUNDLE_MAGIC)?;
    let ticks_per_hour_log2 = cursor.u8()?;
    if ticks_per_hour_log2 != 0 {
        return Err(ProductError::new(
            "d81 production bundle must use raw game-hour ticks",
        ));
    }
    cursor.zeroes(7)?;
    let content_manifest_sha256 = cursor.array32()?;
    let definition_csv_sha256 = cursor.array32()?;
    let count = cursor.bounded_count(MAX_ARTIFACTS, "artifact")?;
    let mut artifacts = Vec::with_capacity(count);
    let mut previous: Option<Vec<u8>> = None;
    for _ in 0..count {
        let path = cursor.string(MAX_PATH_BYTES)?;
        if previous
            .as_ref()
            .is_some_and(|prior| prior.as_slice() >= path.as_bytes())
        {
            return Err(ProductError::new(
                "bundle artifact descriptors are not strictly bytewise sorted",
            ));
        }
        previous = Some(path.as_bytes().to_vec());
        let role = ArtifactRole::from_code(cursor.u8()?)?;
        if role == ArtifactRole::BundleMetadata {
            return Err(ProductError::new(
                "bundle metadata cannot recursively describe itself",
            ));
        }
        cursor.zeroes(7)?;
        artifacts.push(ArtifactDescriptor {
            path,
            role,
            len: cursor.u64()?,
            blake3: cursor.array32()?,
        });
    }
    cursor.end()?;
    Ok(BundleMetadata {
        ticks_per_hour_log2,
        content_manifest_sha256,
        definition_csv_sha256,
        artifacts,
    })
}

pub(crate) fn encode_branches(branches: &[BranchSpec]) -> Result<Vec<u8>, ProductError> {
    validate_branches(branches)?;
    let mut ordered = branches.to_vec();
    ordered.sort_by_key(|branch| branch.branch);
    let mut out = Vec::new();
    preamble(&mut out, BRANCH_MAGIC);
    put_u64(&mut out, ordered.len() as u64);
    for branch in ordered {
        put_u32(&mut out, branch.branch);
        put_u64(&mut out, branch.first_tick);
        match branch.parent {
            None => {
                out.push(0);
                out.extend_from_slice(&[0; 3]);
                put_u32(&mut out, 0);
                put_u64(&mut out, 0);
            }
            Some(parent) => {
                out.push(1);
                out.extend_from_slice(&[0; 3]);
                put_u32(&mut out, parent.branch);
                put_u64(&mut out, parent.through_tick);
            }
        }
        out.extend_from_slice(&branch.session_fingerprint);
    }
    Ok(out)
}

pub(crate) fn decode_branches(bytes: &[u8]) -> Result<Vec<BranchSpec>, ProductError> {
    let mut cursor = Cursor::new(bytes);
    cursor.preamble(BRANCH_MAGIC)?;
    let count = cursor.bounded_count(MAX_BRANCHES, "branch")?;
    let mut branches = Vec::with_capacity(count);
    for _ in 0..count {
        let branch = cursor.u32()?;
        let first_tick = cursor.u64()?;
        let has_parent = cursor.u8()?;
        cursor.zeroes(3)?;
        let parent_branch = cursor.u32()?;
        let through_tick = cursor.u64()?;
        let parent = match has_parent {
            0 if parent_branch == 0 && through_tick == 0 => None,
            1 => Some(BranchParent {
                branch: parent_branch,
                through_tick,
            }),
            0 => {
                return Err(ProductError::new(
                    "parent-absent branch carries nonzero parent fields",
                ))
            }
            _ => return Err(ProductError::new("branch parent discriminator is not 0 or 1")),
        };
        branches.push(BranchSpec {
            branch,
            parent,
            first_tick,
            session_fingerprint: cursor.array32()?,
        });
    }
    cursor.end()?;
    validate_branches(&branches)?;
    Ok(branches)
}

pub(crate) fn validate_branches(branches: &[BranchSpec]) -> Result<(), ProductError> {
    if branches.is_empty() {
        return Err(ProductError::new("bundle has no branches"));
    }
    validate_branch_count(branches.len())?;
    let by_id = branches
        .iter()
        .map(|branch| (branch.branch, branch))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != branches.len() {
        return Err(ProductError::new("branch ids are not unique"));
    }
    let root = by_id
        .get(&0)
        .ok_or_else(|| ProductError::new("branch zero is missing"))?;
    if root.parent.is_some() {
        return Err(ProductError::new("branch zero cannot have a parent"));
    }
    // Validate every direct edge before any transitive walk. Otherwise an
    // earlier branch can traverse through a later malformed branch and turn
    // adversarial bytes into an indexing panic.
    for branch in branches {
        if branch.session_fingerprint == [0; 32] {
            return Err(ProductError::new(format!(
                "branch {} has an empty decoded-stream fingerprint",
                branch.branch
            )));
        }
        if branch.branch != 0 {
            let parent = branch.parent.ok_or_else(|| {
                ProductError::new(format!("branch {} has no parent", branch.branch))
            })?;
            if parent.branch == branch.branch || !by_id.contains_key(&parent.branch) {
                return Err(ProductError::new(format!(
                    "branch {} has an invalid parent {}",
                    branch.branch, parent.branch
                )));
            }
            let expected = parent
                .through_tick
                .checked_add(1)
                .ok_or_else(|| ProductError::new("branch parent tick overflows"))?;
            if branch.first_tick != expected {
                return Err(ProductError::new(format!(
                    "branch {} first tick {} does not immediately follow inherited tick {}",
                    branch.branch, branch.first_tick, parent.through_tick
                )));
            }
        }
    }
    // Functional-graph traversal with memoized completion. Every branch enters
    // and leaves the visiting state once, so a deep valid rollover chain is
    // not re-walked from each descendant.
    let mut visit = BTreeMap::<u32, u8>::new();
    for branch in branches {
        if visit.get(&branch.branch) == Some(&2) {
            continue;
        }
        let mut path = Vec::new();
        let mut cursor = branch;
        loop {
            match visit.get(&cursor.branch).copied() {
                Some(2) => break,
                Some(1) => {
                    return Err(ProductError::new("branch ancestry contains a cycle"))
                }
                None => {}
                Some(_) => unreachable!("branch visit state is internal"),
            }
            visit.insert(cursor.branch, 1);
            path.push(cursor.branch);
            let Some(parent) = cursor.parent else {
                break;
            };
            cursor = by_id.get(&parent.branch).copied().ok_or_else(|| {
                ProductError::new(format!(
                    "branch {} ancestry names missing parent {}",
                    cursor.branch, parent.branch
                ))
            })?;
        }
        for branch_id in path {
            visit.insert(branch_id, 2);
        }
    }
    Ok(())
}

fn validate_branch_count(count: usize) -> Result<(), ProductError> {
    if count > MAX_BRANCHES {
        return Err(ProductError::new(format!(
            "branch count {count} exceeds the {MAX_BRANCHES} production bound"
        )));
    }
    Ok(())
}

pub(crate) fn encode_schemas(
    definitions: &[SchemaDefinition],
) -> Result<Vec<u8>, ProductError> {
    validate_schema_registry(definitions)?;
    let mut ordered = definitions.to_vec();
    ordered.sort_by_key(|definition| (definition.schema_hash, definition.domain));
    let mut out = Vec::new();
    preamble(&mut out, SCHEMA_MAGIC);
    put_u64(&mut out, ordered.len() as u64);
    for definition in ordered {
        put_u32(&mut out, definition.domain);
        put_u64(&mut out, definition.schema_hash);
        put_u64(&mut out, definition.canonical_bytes.len() as u64);
        out.extend_from_slice(&definition.canonical_bytes);
    }
    Ok(out)
}

pub(crate) fn decode_schemas(bytes: &[u8]) -> Result<Vec<SchemaDefinition>, ProductError> {
    let mut cursor = Cursor::new(bytes);
    cursor.preamble(SCHEMA_MAGIC)?;
    let count = cursor.bounded_count(MAX_SCHEMAS, "schema")?;
    let mut definitions = Vec::with_capacity(count);
    for _ in 0..count {
        let domain = cursor.u32()?;
        let schema_hash = cursor.u64()?;
        let len = cursor.bounded_count(MAX_SCHEMA_BYTES, "schema byte")?;
        definitions.push(SchemaDefinition {
            domain,
            schema_hash,
            canonical_bytes: cursor.take(len)?.to_vec(),
        });
    }
    cursor.end()?;
    validate_schema_registry(&definitions)?;
    Ok(definitions)
}

pub(crate) fn validate_schema_registry(
    definitions: &[SchemaDefinition],
) -> Result<(), ProductError> {
    if definitions.is_empty() {
        return Err(ProductError::new("schema registry is empty"));
    }
    let mut by_hash: BTreeMap<u64, &[u8]> = BTreeMap::new();
    let mut domains = BTreeSet::new();
    for definition in definitions {
        if !domains.insert(definition.domain) {
            return Err(ProductError::new(format!(
                "schema registry repeats domain {}",
                definition.domain
            )));
        }
        match by_hash.insert(definition.schema_hash, &definition.canonical_bytes) {
            Some(prior) if prior != definition.canonical_bytes => {
                return Err(ProductError::new(format!(
                    "schema hash {:#018x} collides across unequal definitions",
                    definition.schema_hash
                )))
            }
            _ => {}
        }
        let digest = blake3::hash(&definition.canonical_bytes);
        let actual =
            u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("eight-byte slice"));
        if actual != definition.schema_hash {
            return Err(ProductError::new(format!(
                "schema definition for domain {} does not match declared hash {:#018x}",
                definition.domain, definition.schema_hash
            )));
        }
    }
    Ok(())
}

fn preamble(out: &mut Vec<u8>, magic: &[u8; 4]) {
    out.extend_from_slice(magic);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), ProductError> {
    if value.len() > MAX_PATH_BYTES {
        return Err(ProductError::new("artifact path exceeds byte bound"));
    }
    put_u64(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

#[cfg(test)]
mod branch_validation_tests {
    use super::*;

    fn branch(branch: u32, parent: Option<u32>) -> BranchSpec {
        BranchSpec {
            branch,
            parent: parent.map(|parent| BranchParent {
                branch: parent,
                through_tick: 0,
            }),
            first_tick: u64::from(branch != 0),
            session_fingerprint: [1; 32],
        }
    }

    #[test]
    fn transitive_missing_parent_fails_closed_in_adversarial_order() {
        let branches = [branch(0, None), branch(1, Some(2)), branch(2, Some(99))];
        let error = validate_branches(&branches).unwrap_err();
        assert!(error.to_string().contains("invalid parent 99"));
    }

    #[test]
    fn self_parent_and_transitive_cycle_remain_rejected() {
        let self_parent = [branch(0, None), branch(1, Some(1))];
        assert!(validate_branches(&self_parent)
            .unwrap_err()
            .to_string()
            .contains("invalid parent 1"));

        let cycle = [
            branch(0, None),
            branch(1, Some(2)),
            branch(2, Some(1)),
        ];
        assert!(validate_branches(&cycle)
            .unwrap_err()
            .to_string()
            .contains("cycle"));
    }

    #[test]
    fn producer_branch_bound_and_reverse_deep_chain_are_bounded() {
        assert!(validate_branch_count(MAX_BRANCHES).is_ok());
        assert!(validate_branch_count(MAX_BRANCHES + 1).is_err());

        let mut branches = vec![branch(0, None)];
        branches.extend((1..=4096).rev().map(|id| branch(id, Some(id - 1))));
        validate_branches(&branches).unwrap();
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn preamble(&mut self, magic: &[u8; 4]) -> Result<(), ProductError> {
        if self.take(4)? != magic {
            return Err(ProductError::new("rebuilt-wire artifact has wrong magic"));
        }
        let version = u16::from_le_bytes(self.take(2)?.try_into().expect("two-byte slice"));
        if version != VERSION {
            return Err(ProductError::new(format!(
                "unsupported rebuilt-wire artifact version {version}"
            )));
        }
        if self.take(2)? != [0, 0] {
            return Err(ProductError::new(
                "rebuilt-wire artifact has nonzero reserved preamble",
            ));
        }
        Ok(())
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ProductError> {
        let end = self
            .at
            .checked_add(len)
            .ok_or_else(|| ProductError::new("artifact cursor overflows"))?;
        if end > self.bytes.len() {
            return Err(ProductError::new("rebuilt-wire artifact is truncated"));
        }
        let result = &self.bytes[self.at..end];
        self.at = end;
        Ok(result)
    }

    fn u8(&mut self) -> Result<u8, ProductError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ProductError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four-byte slice"),
        ))
    }

    fn u64(&mut self) -> Result<u64, ProductError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("eight-byte slice"),
        ))
    }

    fn array32(&mut self) -> Result<[u8; 32], ProductError> {
        Ok(self.take(32)?.try_into().expect("32-byte slice"))
    }

    fn zeroes(&mut self, len: usize) -> Result<(), ProductError> {
        if self.take(len)?.iter().any(|byte| *byte != 0) {
            return Err(ProductError::new(
                "rebuilt-wire artifact has nonzero reserved bytes",
            ));
        }
        Ok(())
    }

    fn bounded_count(&mut self, max: usize, label: &str) -> Result<usize, ProductError> {
        let raw = self.u64()?;
        let value = usize::try_from(raw)
            .map_err(|_| ProductError::new(format!("{label} count does not fit usize")))?;
        if value > max {
            return Err(ProductError::new(format!(
                "{label} count {value} exceeds {max}"
            )));
        }
        Ok(value)
    }

    fn string(&mut self, max: usize) -> Result<String, ProductError> {
        let len = self.bounded_count(max, "string byte")?;
        let text = std::str::from_utf8(self.take(len)?)
            .map_err(|_| ProductError::new("artifact path is not UTF-8"))?;
        crate::model::validate_entry_path(text)?;
        Ok(text.to_string())
    }

    fn end(self) -> Result<(), ProductError> {
        if self.at != self.bytes.len() {
            return Err(ProductError::new(
                "rebuilt-wire artifact carries trailing bytes",
            ));
        }
        Ok(())
    }
}
