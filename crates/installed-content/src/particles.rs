//! Particle descriptors resolved by name, as the game resolves them.
//!
//! An entity event names a `pdxparticle` object; the object's `type` names a
//! `particle` block. The engine loads every `.gfx` below `assetviewer_interface`,
//! `interface` and `gfx` (`14208ca80`), and particle blocks by their `name` from
//! every `gfx/particles/**/*.asset`; no file name is derived from an
//! identifier. A name declared twice has no known precedence and is refused.
//! Descriptors outside the coverage roots are read unverified.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{all, block, invalid, last, number, require, text},
    parser,
};

/// `.gfx` roots in the engine's enumeration order.
const GFX_ROOTS: [&str; 3] = ["assetviewer_interface", "interface", "gfx"];

/// One `particle` block an entity event resolves to.
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    /// Effective `.asset` path holding the block.
    pub path: String,
    /// The block's `name`, which selects it inside that file.
    pub name: String,
    /// The `pdxparticle` object's `scale`.
    pub scale: f32,
}

/// Every `pdxparticle` object and every named `particle` block of the stack,
/// and the `gfx/**/*.asset` entity files, listed in one walk of `gfx`.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// Object name to (`type`, `scale`).
    objects: BTreeMap<String, (String, f32)>,
    /// Particle block name to its file.
    files: BTreeMap<String, String>,
    assets: Vec<String>,
}

impl Catalog {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut out = Self::default();
        let mut gfx = Vec::new();
        for root in GFX_ROOTS {
            let extensions: &[&str] = if root == "gfx" {
                &[".gfx", ".asset"]
            } else {
                &[".gfx"]
            };
            for path in layers.descriptor_paths(root, extensions)? {
                if path.to_ascii_lowercase().ends_with(".asset") {
                    out.assets.push(path);
                } else {
                    gfx.push(path);
                }
            }
        }
        for path in gfx {
            let bytes = layers.read_uncovered(&path)?;
            // Sprite files never declare objects; skip parsing them.
            if bytes.windows(11).any(|word| word == b"pdxparticle") {
                out.add_objects(&bytes)
                    .map_err(|error| invalid(format!("{path}: {error}")))?;
            }
        }
        for path in &out.assets.clone() {
            if path.to_ascii_lowercase().starts_with("gfx/particles/") {
                let bytes = layers.read_uncovered(path)?;
                out.add_particles(&bytes, path)
                    .map_err(|error| invalid(format!("{path}: {error}")))?;
            }
        }
        Ok(out)
    }

    /// Every effective `gfx/**/*.asset`, in (layer, path) order.
    pub fn assets(&self) -> &[String] {
        &self.assets
    }

    fn add_objects(&mut self, bytes: &[u8]) -> io::Result<()> {
        let (document, _) = crate::parser::parse_unclosed(bytes)?;
        for objects in all(&document.items, b"objectTypes") {
            for value in all(block(objects, "objectTypes")?, b"pdxparticle") {
                let fields = block(value, "pdxparticle")?;
                let name = text(require(fields, "name", "pdxparticle")?, "pdxparticle name")?;
                let kind = text(require(fields, "type", "pdxparticle")?, "pdxparticle type")?;
                let scale = match last(fields, b"scale") {
                    Some(value) => number(value, "pdxparticle scale")? as f32,
                    None => 1.0,
                };
                if !scale.is_finite() || scale <= 0.0 {
                    return Err(invalid(format!("pdxparticle {name} has scale {scale}")));
                }
                if self
                    .objects
                    .insert(name.to_owned(), (kind.to_owned(), scale))
                    .is_some()
                {
                    return Err(invalid(format!("pdxparticle {name} is declared twice")));
                }
            }
        }
        Ok(())
    }

    fn add_particles(&mut self, bytes: &[u8], path: &str) -> io::Result<()> {
        let document = parser::parse(bytes)?;
        for value in all(&document.items, b"particle") {
            let fields = block(value, "particle")?;
            let name = text(require(fields, "name", "particle")?, "particle name")?;
            if self
                .files
                .insert(name.to_owned(), path.to_owned())
                .is_some()
            {
                return Err(invalid(format!("particle {name} is declared twice")));
            }
        }
        Ok(())
    }

    /// The particle block a `pdxparticle` object names, refused when either
    /// link is missing.
    pub fn resolve(&self, object: &str) -> io::Result<Particle> {
        let (kind, scale) = self
            .objects
            .get(object)
            .ok_or_else(|| invalid(format!("no pdxparticle {object}")))?;
        let path = self.files.get(kind).ok_or_else(|| {
            invalid(format!(
                "pdxparticle {object} names missing particle {kind}"
            ))
        })?;
        Ok(Particle {
            path: path.clone(),
            name: kind.clone(),
            scale: *scale,
        })
    }

    #[cfg(test)]
    pub(crate) fn add_test_particle(&mut self, object: &str, kind: &str, scale: f32, path: &str) {
        self.objects.insert(object.into(), (kind.into(), scale));
        self.files.insert(kind.into(), path.into());
    }

    /// Every object name, for loaders that resolve all of them.
    pub fn objects(&self) -> impl Iterator<Item = &str> {
        self.objects.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_resolve_through_type_to_the_named_block_and_duplicates_are_refused() {
        let mut catalog = Catalog::default();
        catalog
            .add_objects(
                br#"objectTypes = {
                    pdxparticle = { name = "drops" type = "unrelated_id" scale = 0.5 }
                    pdxparticle = { name = "bare" type = "absent" }
                }"#,
            )
            .unwrap();
        catalog
            .add_particles(
                b"particle = { name = \"unrelated_id\" subsystem = { } }",
                "gfx/particles/custom/any_file.asset",
            )
            .unwrap();
        assert_eq!(
            catalog.resolve("drops").unwrap(),
            Particle {
                path: "gfx/particles/custom/any_file.asset".into(),
                name: "unrelated_id".into(),
                scale: 0.5,
            }
        );
        assert!(catalog.resolve("bare").is_err());
        assert!(catalog.resolve("absent").is_err());
        assert!(
            catalog
                .add_objects(br#"objectTypes = { pdxparticle = { name = drops type = x } }"#)
                .is_err()
        );
        assert!(
            catalog
                .add_particles(
                    b"particle = { name = unrelated_id }",
                    "gfx/particles/b.asset"
                )
                .is_err()
        );
    }
}
