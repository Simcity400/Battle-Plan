//! XMAP02 resolved foundation content at ATTESTATION grain: the capture-side
//! record that the five map visual content families' inputs were classified
//! against the effective-VFS winners of exactly one finalized manifest.
//!
//! The viewer-local XMAP02 operation record
//! ([`super::foundation::ResolvedFoundationContent`]) binds shader translation
//! units and D3D resource identities only the playing side can compile. The
//! classification of each family's canonical closure (fixed files plus whole
//! namespaces) against a manifest's required-content rows is DETERMINISTIC -
//! producer and consumer share one derivation, so the wire must not carry the
//! derivation itself (smallest pre-compression size). It carries its SEAL
//! beside the manifest digest it attests. Playback re-derives locally and
//! verifies; any divergence between what the capture classified and what this
//! recording's own manifest yields - post-attach drift, tamper, a rewritten
//! record - refuses by name.
//!
//! A family whose FIXED closure inputs were not fully admitted does not seal
//! silently: its unbound inputs ride the classification as a named, witnessed
//! gap (`missing_paths_sha256` over the ascending missing list), and the
//! producer surfaces the names on its summary line. Production failure seals
//! with a typed gap marker carried beside the portable content manifest.

use super::ContentManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FOUNDATION_FAMILY_CONTENT_SET_SCHEMA: &str =
    "hoi4.replay.xmap02-resolved-foundation-content-family-set.v1";

/// One canonical XMAP02 content family's structural input closure: fixed
/// game-relative files plus whole namespaces (every admitted winner under the
/// prefix belongs to the family). Path lists are structural domain knowledge
/// pinned by
/// `replay_interface/re_evidence/rendering_laws/XMAP00/effective_vfs_dependency_ledger_20260730.tsv`
/// - no game CONTENT is enumerated here, only where families read.
pub struct FoundationFamilyClosure {
    pub family_id: &'static str,
    pub fixed_paths: &'static [&'static str],
    pub namespace_prefixes: &'static [&'static str],
}

/// The canonical XMAP02 content families in their required record order
/// (family identity = the QJ re-judge ledger row ids; order is part of the
/// format so a set cannot silently permute).
pub const FOUNDATION_FAMILY_CLOSURES: [FoundationFamilyClosure; 5] = [
    FoundationFamilyClosure {
        family_id: "content.borders",
        fixed_paths: &[
            "common/defines/00_graphics.lua",
            "gfx/fx/border.shader",
            "gfx/fx/constants.fxh",
            "gfx/fx/fow.fxh",
            "gfx/fx/pdxmap.fxh",
            "gfx/fx/shadow.fxh",
            "gfx/fx/standardfuncsgfx.fxh",
            "map/adjacencies.csv",
            "map/default.map",
            "map/definition.csv",
            "map/heightmap.bmp",
            "map/provinces.bmp",
            "map/terrain/border_country_0.dds",
            "map/terrain/border_country_1.dds",
            "map/terrain/border_country_2.dds",
            "map/terrain/border_impassable_0.dds",
            "map/terrain/border_impassable_1.dds",
            "map/terrain/border_impassable_2.dds",
            "map/terrain/border_province_0.dds",
            "map/terrain/border_province_1.dds",
            "map/terrain/border_province_2.dds",
            "map/terrain/border_sea_0.dds",
            "map/terrain/border_sea_1.dds",
            "map/terrain/border_sea_2.dds",
            "map/terrain/border_sea_region_0.dds",
            "map/terrain/border_sea_region_1.dds",
            "map/terrain/border_sea_region_2.dds",
            "map/terrain/border_state_0.dds",
            "map/terrain/border_state_1.dds",
            "map/terrain/border_state_2.dds",
        ],
        namespace_prefixes: &["history/states"],
    },
    FoundationFamilyClosure {
        family_id: "content.lighting_shadow",
        fixed_paths: &[
            "common/defines/00_graphics.lua",
            "gfx/fx/constants.fxh",
            "gfx/fx/pdxmap.shader",
            "gfx/fx/shadow.fxh",
            "gfx/fx/shadowblur.shader",
        ],
        namespace_prefixes: &[],
    },
    FoundationFamilyClosure {
        family_id: "content.political_fow",
        fixed_paths: &[
            "common/defines/00_graphics.lua",
            "gfx/fx/constants.fxh",
            "gfx/fx/fow.fxh",
            "gfx/fx/pdxmap.fxh",
            "gfx/fx/pdxmap.shader",
            "gfx/fx/shadow.fxh",
            "gfx/fx/standardfuncsgfx.fxh",
            "gfx/fx/tiled_pointlights.fxh",
        ],
        namespace_prefixes: &[
            "common/countries",
            "common/country_tags",
            "common/ideologies",
            "history/states",
        ],
    },
    FoundationFamilyClosure {
        family_id: "content.rivers_straits",
        fixed_paths: &[
            "common/defines/00_graphics.lua",
            "gfx/fx/constants.fxh",
            "gfx/fx/fow.fxh",
            "gfx/fx/river.shader",
            "gfx/fx/shadow.fxh",
            "gfx/fx/standardfuncsgfx.fxh",
            "gfx/fx/tiled_pointlights.fxh",
            "map/adjacencies.csv",
            "map/heightmap.bmp",
            "map/rivers.bmp",
            "map/terrain/riversurface_diffuse_0.dds",
            "map/terrain/riversurface_diffuse_1.dds",
            "map/terrain/riversurface_diffuse_2.dds",
            "map/terrain/riversurface_masks.dds",
            "map/terrain/riversurface_normal_0.dds",
            "map/terrain/riversurface_normal_1.dds",
            "map/terrain/riversurface_normal_2.dds",
        ],
        namespace_prefixes: &[],
    },
    FoundationFamilyClosure {
        family_id: "content.water_ice",
        fixed_paths: &[
            "common/defines/00_graphics.lua",
            "gfx/fx/constants.fxh",
            "gfx/fx/fow.fxh",
            "gfx/fx/pdxmap.fxh",
            "gfx/fx/pdxmap.shader",
            "gfx/fx/pdxwater.shader",
            "gfx/fx/shadow.fxh",
            "gfx/fx/standardfuncsgfx.fxh",
            "gfx/fx/tiled_pointlights.fxh",
            "map/heightmap.bmp",
            "map/terrain.bmp",
            "map/terrain/colormap_water_0.dds",
            "map/terrain/colormap_water_1.dds",
            "map/terrain/colormap_water_2.dds",
            "map/terrain/fow_rgb_waterspec_a.dds",
            "map/terrain/ice_diffuse.dds",
            "map/terrain/ice_noise_0.dds",
            "map/terrain/ice_noise_1.dds",
            "map/terrain/ice_noise_2.dds",
            "map/terrain/lean1.dds",
            "map/terrain/lean2.dds",
            "map/terrain/reflection.dds",
            "map/terrain/underwater_terrain_0.dds",
            "map/terrain/underwater_terrain_1.dds",
            "map/terrain/underwater_terrain_2.dds",
        ],
        namespace_prefixes: &[],
    },
];

pub fn canonical_closure(family_id: &str) -> Option<&'static FoundationFamilyClosure> {
    FOUNDATION_FAMILY_CLOSURES
        .iter()
        .find(|closure| closure.family_id == family_id)
}

fn valid_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} is not a SHA-256"));
    }
    Ok(())
}

/// The derived classification identity for ONE family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationFamilyDigestRow {
    pub family_id: String,
    /// Witness over the ascending list of closure paths bound by admission.
    pub bound_paths_sha256: String,
    /// Named-gap witness: digest over the ascending list of FIXED closure
    /// paths attach admission could NOT bind. All-zero-equivalent only when
    /// the list is empty (the empty-list digest), never a silent absence.
    pub missing_paths_sha256: String,
}

/// The full derived classification of one finalized manifest - computed by
/// both sides from shared code, sealed for comparison, never shipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationFamilyClassification {
    pub schema: String,
    /// The finalized manifest digest whose winners were classified.
    pub manifest_digest_sha256: String,
    /// Exactly [`FOUNDATION_FAMILY_CLOSURES`], in that order.
    pub families: Vec<FoundationFamilyDigestRow>,
    pub classification_sha256: String,
}

impl FoundationFamilyClassification {
    pub fn computed_classification_sha256(&self) -> Result<String, String> {
        let mut unsigned = self.clone();
        unsigned.classification_sha256 = String::new();
        let bytes = serde_json::to_vec(&unsigned).map_err(|error| {
            format!("encode foundation family classification identity: {error}")
        })?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn with_computed_classification_sha256(mut self) -> Result<Self, String> {
        self.classification_sha256 = self.computed_classification_sha256()?;
        self.validate()?;
        Ok(self)
    }

    pub fn family(&self, family_id: &str) -> Option<&FoundationFamilyDigestRow> {
        FOUNDATION_FAMILY_CLOSURES
            .iter()
            .zip(&self.families)
            .find(|(expected, _)| expected.family_id == family_id)
            .map(|(_, row)| row)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != FOUNDATION_FAMILY_CONTENT_SET_SCHEMA {
            return Err(format!(
                "unsupported foundation family classification schema {}",
                self.schema
            ));
        }
        valid_sha256(&self.manifest_digest_sha256, "manifest digest")?;
        if self.families.len() != FOUNDATION_FAMILY_CLOSURES.len() {
            return Err(format!(
                "foundation family classification covers {} of {} families",
                self.families.len(),
                FOUNDATION_FAMILY_CLOSURES.len()
            ));
        }
        for (expected, family) in FOUNDATION_FAMILY_CLOSURES.iter().zip(&self.families) {
            if family.family_id != expected.family_id {
                return Err(format!(
                    "foundation family position mismatch: expected {}, got {}",
                    expected.family_id, family.family_id
                ));
            }
            valid_sha256(&family.bound_paths_sha256, "bound-path witness")?;
            valid_sha256(&family.missing_paths_sha256, "missing-path witness")?;
        }
        valid_sha256(&self.classification_sha256, "classification")?;
        if self.classification_sha256 != self.computed_classification_sha256()? {
            return Err("foundation family classification identity does not match its payload"
                .into());
        }
        Ok(())
    }
}

/// THE XMAP02 RECORD on the wire: which manifest was classified, and the seal
/// of that classification. Consumers re-derive with shared code and compare -
/// nothing derivable rides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationFamilyContentSet {
    pub schema: String,
    /// The finalized manifest digest the producer classified.
    pub manifest_digest_sha256: String,
    /// Seal over [`FoundationFamilyClassification`] as derived from that
    /// manifest.
    pub classification_sha256: String,
}

impl FoundationFamilyContentSet {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != FOUNDATION_FAMILY_CONTENT_SET_SCHEMA {
            return Err(format!(
                "unsupported foundation family content set schema {}",
                self.schema
            ));
        }
        valid_sha256(&self.manifest_digest_sha256, "manifest digest")?;
        valid_sha256(&self.classification_sha256, "classification")?;
        Ok(())
    }

    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self =
            serde_json::from_slice(bytes).map_err(|error| format!("decode: {error}"))?;
        value.validate()?;
        Ok(value)
    }

    /// THE JOIN PRIMITIVE: this record holds exactly the seal of `derived`,
    /// and `derived` names the same manifest.
    pub fn verify_against(&self, derived: &FoundationFamilyClassification) -> Result<(), String> {
        self.validate()?;
        derived.validate()?;
        if self.manifest_digest_sha256 != derived.manifest_digest_sha256 {
            return Err(format!(
                "foundation content record names manifest {}, but the joined classification is \
                 of manifest {}",
                self.manifest_digest_sha256, derived.manifest_digest_sha256
            ));
        }
        if self.classification_sha256 != derived.classification_sha256 {
            return Err(
                "foundation content record does not match the classification derived from its \
                 own manifest"
                    .into(),
            );
        }
        Ok(())
    }
}

/// Derive ONE family's bound closure paths against a manifest's
/// required-content rows: fixed paths present are bound; namespace members
/// are exactly whatever rows exist under the prefix - measured, never
/// assumed. The result is ascending and unique; out-of-closure names cannot
/// enter by construction.
pub fn derive_family_bound_paths(
    closure: &FoundationFamilyClosure,
    mut present: impl FnMut(&str) -> bool,
    mut names_under_prefix: impl FnMut(&str) -> Vec<String>,
) -> Vec<String> {
    let mut used: Vec<String> = closure
        .fixed_paths
        .iter()
        .filter(|path| present(path))
        .map(|path| (*path).to_string())
        .collect();
    used.extend(
        closure
            .namespace_prefixes
            .iter()
            .flat_map(|prefix| names_under_prefix(prefix)),
    );
    // Namespace prefixes may be written with or without the trailing slash;
    // every hit must genuinely sit under the prefix directory so a caller bug
    // cannot smuggle foreign rows into the seal.
    used.retain(|path| {
        closure.fixed_paths.contains(&path.as_str())
            || closure.namespace_prefixes.iter().any(|prefix| {
                let dir = format!("{}/", prefix.trim_end_matches('/'));
                path.starts_with(&dir)
            })
    });
    used.sort();
    used.dedup();
    used
}

/// Derive ONE family's NAMED GAP: the ascending list of FIXED closure paths
/// admission could not bind. Namespaces have no gap state here - their
/// members are measured, never assumed.
pub fn derive_family_missing_paths(
    closure: &FoundationFamilyClosure,
    mut present: impl FnMut(&str) -> bool,
) -> Vec<String> {
    closure
        .fixed_paths
        .iter()
        .filter(|path| !present(path))
        .map(|path| (*path).to_string())
        .collect()
}

pub fn bound_paths_sha256(paths: &[String]) -> String {
    path_list_sha256(paths)
}

pub fn missing_paths_sha256(paths: &[String]) -> String {
    path_list_sha256(paths)
}

fn path_list_sha256(paths: &[String]) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(paths).expect("string array serializes"))
    )
}

/// Derive the full XMAP02 classification from a finalized manifest. Producer
/// and consumer run THIS function; equality of its seal is the entire join.
/// Also returns each family's named-missing list so the producer can surface
/// the NAMES on its summary line.
pub fn derive_foundation_family_classification(
    manifest: &ContentManifest,
) -> Result<FoundationFamilyClassification, String> {
    let manifest_digest_sha256 = manifest
        .manifest_digest_sha256
        .as_ref()
        .ok_or("XMAP02 foundation content requires a finalized manifest digest")?
        .clone();
    manifest.validate().map_err(|error| {
        format!("XMAP02 foundation content refused its manifest: {error}")
    })?;
    let required_paths: std::collections::BTreeSet<&str> = manifest
        .required_content
        .iter()
        .map(|row| row.content.normalized_game_relative_path.as_str())
        .collect();
    let present = |path: &str| required_paths.contains(path);
    let families = FOUNDATION_FAMILY_CLOSURES
        .iter()
        .map(|closure| {
            let bound = derive_family_bound_paths(closure, present, |prefix| {
                let prefixed = format!("{}/", prefix.trim_end_matches('/'));
                required_paths
                    .range(prefixed.as_str()..)
                    .take_while(|path| path.starts_with(prefixed.as_str()))
                    .map(|path| (*path).to_string())
                    .collect::<Vec<String>>()
            });
            let missing = derive_family_missing_paths(closure, present);
            FoundationFamilyDigestRow {
                family_id: closure.family_id.to_string(),
                bound_paths_sha256: bound_paths_sha256(&bound),
                missing_paths_sha256: missing_paths_sha256(&missing),
            }
        })
        .collect();
    FoundationFamilyClassification {
        schema: FOUNDATION_FAMILY_CONTENT_SET_SCHEMA.into(),
        manifest_digest_sha256,
        families,
        classification_sha256: String::new(),
    }
    .with_computed_classification_sha256()
    .map_err(|error| format!("seal XMAP02 foundation family classification: {error}"))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// THE HARNESS-AUTHORITY DRIFT GUARD. `FOUNDATION_FAMILY_CLOSURES` claims
    /// to pin its structural input lists to the QJ effective-VFS dependency
    /// ledger; nothing else checked that the two agree, so the next ledger
    /// edit would have silently diverged from the code. This test reads THAT
    /// ledger at test time, derives each of the five families' fixed paths and
    /// namespace prefixes from it, and requires exact equality with the table,
    /// order included (both sides are byte-ascending).
    // kills: harness-authority drift between FOUNDATION_FAMILY_CLOSURES and
    // replay_interface/re_evidence/rendering_laws/XMAP00/
    // effective_vfs_dependency_ledger_20260730.tsv.
    #[test]
    fn closures_match_the_effective_vfs_dependency_ledger_exactly() {
        const LEDGER: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../replay_interface/re_evidence/rendering_laws/XMAP00/effective_vfs_dependency_ledger_20260730.tsv"
        ));

        fn push_unique(list: &mut Vec<String>, value: String) {
            if !list.contains(&value) {
                list.push(value);
            }
        }

        let mut derived_fixed: std::collections::BTreeMap<&str, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut derived_namespaces: std::collections::BTreeMap<&str, Vec<String>> =
            std::collections::BTreeMap::new();
        // The PINNED-namespace justification row must exist and still name its
        // directory in its evidence anchor, or the pin below refuses.
        let mut pinned_countries_anchor: Option<&str> = None;
        // Named binding: row strings are referenced by the maps below, so the
        // normalized document must outlive the parse loop.
        let ledger_document = LEDGER.replace("\r\n", "\n");
        for line in ledger_document.lines().skip(1) {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 3 {
                continue;
            }
            let family_id = fields[0].trim();
            let dependency_id = fields[1].trim();
            if family_id == "content.political_fow"
                && dependency_id == "country_definitions"
            {
                pinned_countries_anchor = Some(fields.get(7).copied().unwrap_or("").trim());
            }
            if canonical_closure(family_id).is_none() {
                continue;
            }
            let input = fields[2].trim().replace('\\', "/").to_ascii_lowercase();
            if input.is_empty() || input.contains("->") {
                // Prose-derived dependencies cannot be parsed here; every one
                // is pinned below with its justifying row.
                continue;
            }
            if input.contains('*') || input.contains('?') {
                let cut = input.find('*').or_else(|| input.find('?')).expect("wildcard");
                let prefix = input[..cut].trim_end_matches('/').to_string();
                assert!(
                    !prefix.is_empty(),
                    "ledger row {family_id}/{:?} has a wildcard with no directory prefix",
                    fields[1]
                );
                push_unique(
                    derived_namespaces.entry(family_id).or_default(),
                    prefix,
                );
            } else {
                push_unique(derived_fixed.entry(family_id).or_default(), input);
            }
        }

        // PINNED NAMESPACE: the ledger row `content.political_fow` /
        // `country_definitions` expresses its dependency as prose ("country
        // tag declarations -> referenced country definition paths"), not as a
        // glob, so the derivation above cannot see it. The pin is only valid
        // while that exact row still exists AND its evidence-anchor column
        // still names `common/countries` - deleting, renaming or re-anchoring
        // the row fails this test.
        assert_eq!(
            pinned_countries_anchor,
            Some("common/countries"),
            "the pinned common/countries namespace justification row (content.political_fow / \
             country_definitions) is missing, changed shape, or no longer names common/countries \
             in its evidence anchor"
        );
        push_unique(
            derived_namespaces.entry("content.political_fow").or_default(),
            "common/countries".to_string(),
        );

        for closure in FOUNDATION_FAMILY_CLOSURES.iter() {
            let mut fixed = derived_fixed.remove(closure.family_id).unwrap_or_default();
            fixed.sort();
            let fixed_refs: Vec<&str> = fixed.iter().map(String::as_str).collect();
            assert_eq!(
                fixed_refs, closure.fixed_paths,
                "{} fixed closure diverges from the effective-VFS dependency ledger",
                closure.family_id
            );
            let mut namespaces = derived_namespaces.remove(closure.family_id).unwrap_or_default();
            namespaces.sort();
            let namespace_refs: Vec<&str> = namespaces.iter().map(String::as_str).collect();
            assert_eq!(
                namespace_refs, closure.namespace_prefixes,
                "{} namespace prefixes diverge from the effective-VFS dependency ledger",
                closure.family_id
            );
        }
    }

    fn sha(value: char) -> String {
        std::iter::repeat(value).take(64).collect()
    }

    #[test]
    fn derivation_is_deterministic_and_namespace_sensitive() {
        // kills: content.borders / content.political_fow reopen condition -
        // history/states belongs to both families; a mod adding state files
        // must change what was classified for them.
        let closure = canonical_closure("content.borders").unwrap();
        let without_mod = ["map/provinces.bmp"];
        let with_mod = ["history/states/zz.txt", "map/provinces.bmp"];
        let derive_for = |rows: &[&str]| {
            let present = |path: &str| rows.contains(&path);
            let names_under_prefix = |prefix: &str| -> Vec<String> {
                let dir = format!("{}/", prefix.trim_end_matches('/'));
                rows.iter()
                    .filter(|path| path.starts_with(dir.as_str()))
                    .map(|path| (*path).to_string())
                    .collect()
            };
            let bound = derive_family_bound_paths(closure, present, names_under_prefix);
            bound_paths_sha256(&bound)
        };
        assert_ne!(derive_for(&without_mod), derive_for(&with_mod));
        assert_eq!(derive_for(&with_mod), derive_for(&with_mod));
    }

    #[test]
    fn missing_fixed_input_changes_its_named_gap_witness() {
        // kills: R4-1 - an incomplete capture must not seal identically to a
        // complete one; the unbound fixed inputs carry their own witness.
        let closure = canonical_closure("content.lighting_shadow").unwrap();
        let complete = [
            "common/defines/00_graphics.lua",
            "gfx/fx/constants.fxh",
            "gfx/fx/pdxmap.shader",
            "gfx/fx/shadow.fxh",
            "gfx/fx/shadowblur.shader",
        ];
        let incomplete = ["gfx/fx/shadow.fxh"];
        let missing_witness = |rows: &[&str]| {
            let present = |path: &str| rows.contains(&path);
            missing_paths_sha256(&derive_family_missing_paths(closure, present))
        };
        // A complete family witnesses exactly the empty list.
        assert_eq!(missing_witness(&complete), missing_paths_sha256(&[]));
        assert_ne!(missing_witness(&incomplete), missing_witness(&complete));
    }

    #[test]
    fn out_of_closure_names_cannot_enter_the_seal() {
        let closure = canonical_closure("content.borders").unwrap();
        let polluted: Vec<String> = vec![
            "map/provinces.bmp".to_string(),
            "gfx/models/tank.mesh".to_string(),
            "history/statesfoo.txt".to_string(),
        ];
        let bound = derive_family_bound_paths(
            closure,
            |path: &str| polluted.iter().any(|candidate| candidate == path),
            // A caller whose prefix scan is boundary-broken would return
            // near-prefix names like this one; the filter must drop them.
            |prefix| {
                let _ = prefix;
                vec!["history/statesfoo.txt".to_string()]
            },
        );
        assert!(bound.contains(&"map/provinces.bmp".to_string()));
        assert!(!bound.contains(&"gfx/models/tank.mesh".to_string()));
        assert!(!bound.contains(&"history/statesfoo.txt".to_string()));
    }

    fn sample_classification() -> FoundationFamilyClassification {
        let rows = [
            "map/provinces.bmp",
            "gfx/fx/border.shader",
            "history/states/1.txt",
        ];
        let present = |path: &str| rows.contains(&path);
        let names_under_prefix = |prefix: &str| -> Vec<String> {
            let dir = format!("{}/", prefix.trim_end_matches('/'));
            rows.iter()
                .filter(|path| path.starts_with(dir.as_str()))
                .map(|path| (*path).to_string())
                .collect()
        };
        FoundationFamilyClassification {
            schema: FOUNDATION_FAMILY_CONTENT_SET_SCHEMA.into(),
            manifest_digest_sha256: sha('d'),
            families: FOUNDATION_FAMILY_CLOSURES
                .iter()
                .map(|closure| FoundationFamilyDigestRow {
                    family_id: closure.family_id.into(),
                    bound_paths_sha256: bound_paths_sha256(&derive_family_bound_paths(
                        closure,
                        present,
                        names_under_prefix,
                    )),
                    missing_paths_sha256: missing_paths_sha256(&derive_family_missing_paths(
                        closure, present,
                    )),
                })
                .collect(),
            classification_sha256: String::new(),
        }
        .with_computed_classification_sha256()
        .expect("seal")
    }

    #[test]
    fn classification_round_trips_and_attests() {
        let value = sample_classification();
        value.validate().expect("valid");
        let decoded: FoundationFamilyClassification =
            serde_json::from_value(serde_json::to_value(&value).expect("value")).expect("decode");
        assert_eq!(decoded, value);
        let record = FoundationFamilyContentSet {
            schema: FOUNDATION_FAMILY_CONTENT_SET_SCHEMA.into(),
            manifest_digest_sha256: value.manifest_digest_sha256.clone(),
            classification_sha256: value.classification_sha256.clone(),
        };
        record.verify_against(&value).expect("attests");
        let encoded = record.encode_json().expect("encode");
        assert_eq!(
            FoundationFamilyContentSet::decode_json(&encoded).expect("decode"),
            record
        );
    }

    #[test]
    fn rejects_wrong_schema_position_swap_stale_seal_and_tampered_record() {
        let wrong_schema = FoundationFamilyClassification {
            schema: "hoi4.replay.xmap02-nope.v9".into(),
            ..sample_classification()
        };
        assert!(wrong_schema.validate().is_err());

        let mut swapped_values = sample_classification();
        swapped_values.families.swap(0, 1);
        assert!(swapped_values.validate().is_err());

        let mut stale = sample_classification();
        stale.families[0].missing_paths_sha256 = sha('9');
        assert!(stale.validate().is_err(), "re-seal mismatch refuses");

        let mut short = sample_classification();
        short.manifest_digest_sha256 = "ab".into();
        assert!(short.validate().is_err());

        let mut partial = sample_classification();
        partial.families.pop();
        assert!(partial.validate().is_err());

        // A mutated wire record still validates on its own but cannot attest.
        let value = sample_classification();
        let tampered = FoundationFamilyContentSet {
            schema: FOUNDATION_FAMILY_CONTENT_SET_SCHEMA.into(),
            manifest_digest_sha256: value.manifest_digest_sha256.clone(),
            classification_sha256: sha('f'),
        };
        tampered.validate().expect("shape-only validation");
        let error = tampered.verify_against(&value).err().expect("refused");
        assert!(error.contains("does not match"), "{error}");

        let foreign_manifest = FoundationFamilyContentSet {
            schema: FOUNDATION_FAMILY_CONTENT_SET_SCHEMA.into(),
            manifest_digest_sha256: sha('a'),
            classification_sha256: value.classification_sha256.clone(),
        };
        let error = foreign_manifest
            .verify_against(&value)
            .err()
            .expect("foreign refused");
        assert!(error.contains("names manifest"), "{error}");
    }
}
