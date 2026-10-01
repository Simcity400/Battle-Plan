//! Particle descriptors resolved by name, as the game resolves them.
//!
//! An entity event names a `pdxparticle` object; the object's `type` names a
//! `particle` block. Gameplay registers startup interface files, then the remaining
//! `interface` and `gfx` descriptors (140184620/1401826E0/140180C00), and particle
//! blocks by their `name` from
//! every `gfx/particles/**/*.asset`; no file name is derived from an
//! identifier. The last registered object of an exact name wins; duplicate
//! particle block names still have unproven precedence and are refused.
//! Descriptors outside the coverage roots are read unverified.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{all, block, invalid, last, number, require, text},
    parser,
};

///140184620 reads these first;1401826E0 excludes them from the ordinary walk.
///Literal bytes1427064A0/1427064C0/1427064E0, not content-selected defaults.
const STARTUP_GFX: [&str; 3] = [
    "interface/load_screen.gfx",
    "interface/load_screen_font.gfx",
    "interface/core_bare_minimum.gfx",
];

pub(crate) fn object_file_order(interface: &[String], gfx: &[String]) -> Vec<String> {
    let mut files: Vec<_> = STARTUP_GFX
        .iter()
        .filter(|path| interface.iter().any(|file| file == **path))
        .map(|path| (*path).to_owned())
        .collect();
    files.extend(
        crate::defines::load_order("interface/", "gfx", interface.iter().map(String::as_str))
            .into_iter()
            .filter(|path| !STARTUP_GFX.contains(&path.as_str())),
    );
    //140180C00 loads these after GUI initialization.1424C9F40 preserves
    //1424C7630's files-before-subdirectories, bytewise traversal.
    files.extend(crate::defines::load_order(
        "gfx/",
        "gfx",
        gfx.iter().map(String::as_str),
    ));
    files
}

/// Shader-facing subsystem metadata. Runtime particles retain this identity;
/// sharing a texture does not imply sharing a technique or constant packet.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderDescriptor {
    pub texture: String,
    pub shader: String,
    pub tiles: [i32; 2],
    pub trail: bool,
    pub billboard: bool,
    pub local_space: bool,
}

impl RenderDescriptor {
    pub fn read(fields: &[parser::Item]) -> io::Result<Self> {
        let boolean = |key: &[u8], default| match last(fields, key) {
            None => Ok(default),
            Some(value) => match text(value, "particle flag")? {
                "yes" => Ok(true),
                "no" => Ok(false),
                value => Err(invalid(format!(
                    "particle flag {}={value}",
                    String::from_utf8_lossy(key)
                ))),
            },
        };
        //1423D7990 clears both strings and initializes x/y to1.14231E470
        //writes signed x/y and truncates file/shader to255/63 bytes.
        let mut out = Self {
            texture: String::new(),
            shader: String::new(),
            tiles: [1; 2],
            trail: boolean(b"trail", false)?,
            billboard: boolean(b"billboard", true)?,
            local_space: boolean(b"local_space", false)?,
        };
        if let Some(texture) = last(fields, b"texture") {
            for item in block(texture, "particle texture")? {
                match item.key.as_deref() {
                    Some(b"file" | b"shader") => {
                        let file = item.key.as_deref() == Some(b"file");
                        let bytes = text(&item.value, "particle texture string")?.as_bytes();
                        let bytes = &bytes[..bytes.len().min(if file { 255 } else { 63 })];
                        let value = std::str::from_utf8(bytes)
                            .map_err(|_| invalid("particle texture truncation splits UTF-8"))?
                            .to_owned();
                        if file {
                            out.texture = value;
                        } else {
                            out.shader = value;
                        }
                    }
                    Some(b"x" | b"y") => {
                        let index = usize::from(item.key.as_deref() == Some(b"y"));
                        out.tiles[index] = text(&item.value, "particle texture tiles")?
                            .parse()
                            .map_err(|_| invalid("particle texture tiles require signed int32"))?;
                    }
                    _ => {}
                }
            }
        }
        if out.tiles.contains(&0) {
            return Err(invalid(format!(
                "particle texture {} has zero atlas dimension; native birth divides by it",
                out.texture
            )));
        }
        Ok(out)
    }

    ///142281B00 appends Trail before NoBillboard; source is always particle.lua.
    pub fn technique(&self) -> String {
        format!(
            "{}{}{}",
            self.shader,
            if self.trail { "Trail" } else { "" },
            if self.billboard { "" } else { "NoBillboard" }
        )
    }
}

/// One `particle` block an entity event resolves to.
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    /// Effective `.asset` path holding the block.
    pub path: String,
    /// The block's `name`, which selects it inside that file.
    pub name: String,
    /// The `pdxparticle` object's `scale`.
    pub scale: f32,
    ///142281570 stores the registration ordinal before the descriptor is parsed.
    pub registration: usize,
}

/// Every `pdxparticle` object and every named `particle` block of the stack,
/// and the `gfx/**/*.asset` entity files, listed in one walk of `gfx`.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// Object name to (`type`, `scale`).
    objects: BTreeMap<String, (String, f32)>,
    ordinals: BTreeMap<String, usize>,
    registrations: usize,
    /// Particle block name to its file.
    files: BTreeMap<String, String>,
    assets: Vec<String>,
    max_particles: usize,
}

impl Catalog {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut out = Self::default();
        let interface = layers.descriptor_paths("interface", &[".gfx"])?;
        let gfx = layers.descriptor_paths("gfx", &[".gfx", ".asset"])?;
        out.assets = gfx
            .iter()
            .filter(|path| path.to_ascii_lowercase().ends_with(".asset"))
            .cloned()
            .collect();
        let gfx = object_file_order(&interface, &gfx);
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
                //1422264C0 ->1406B1110 prepends every object to its name bucket.
                //14226B330 ->142223D70 returns the first exact match. Constructor
                //142281570/14227EDF0 allocates an ordinal even for a duplicate.
                let ordinal = self.registrations;
                self.registrations += 1;
                self.objects
                    .insert(name.to_owned(), (kind.to_owned(), scale));
                self.ordinals.insert(name.to_owned(), ordinal);
            }
        }
        Ok(())
    }

    fn add_particles(&mut self, bytes: &[u8], path: &str) -> io::Result<()> {
        let document = parser::parse(bytes)?;
        for value in all(&document.items, b"particle") {
            let fields = block(value, "particle")?;
            let name = text(require(fields, "name", "particle")?, "particle name")?;
            self.max_particles = self.max_particles.max(
                capacity(fields).map_err(|error| invalid(format!("{path}: {name}: {error}")))?,
            );
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
            registration: self.ordinals[object],
        })
    }

    #[cfg(test)]
    pub(crate) fn add_test_particle(&mut self, object: &str, kind: &str, scale: f32, path: &str) {
        let ordinal = self.registrations;
        self.registrations += 1;
        self.ordinals.insert(object.into(), ordinal);
        self.objects.insert(object.into(), (kind.into(), scale));
        self.files.insert(kind.into(), path.into());
    }

    /// Every object name, for loaders that resolve all of them.
    pub fn objects(&self) -> impl Iterator<Item = &str> {
        self.objects.keys().map(String::as_str)
    }

    ///1423E88B0 takes the maximum system capacity, including replicated children.
    pub fn max_particles(&self) -> usize {
        self.max_particles
    }
}

fn capacity(fields: &[parser::Item]) -> io::Result<usize> {
    let mut total = 0usize;
    for value in all(fields, b"subsystem") {
        let fields = block(value, "particle subsystem")?;
        //14232ECC0 clears hidden subsystem capacity before runtime construction.
        let count = if last(fields, b"hide")
            .map(|v| text(v, "particle hide"))
            .transpose()?
            == Some("yes")
        {
            0
        } else {
            //SSubsystemReader constructor14231BB90 leaves +6E8 uninitialized.
            let value: i32 = text(
                require(fields, "max_amount", "particle capacity")?,
                "max_amount",
            )?
            .parse()
            .map_err(|_| invalid("particle max_amount requires int32"))?;
            usize::try_from(value).map_err(|_| invalid("negative particle max_amount"))?
        };
        //1423DD4E0 adds parent_count*child_count at every recursive level.
        let contribution = capacity(fields)?
            .checked_add(1)
            .and_then(|children| count.checked_mul(children))
            .ok_or_else(|| invalid("particle capacity overflow"))?;
        total = total
            .checked_add(contribution)
            .filter(|value| *value <= i32::MAX as usize)
            .ok_or_else(|| invalid("particle capacity exceeds native int32"))?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_capacity_replicates_children_and_takes_maximum_system() {
        let mut catalog = Catalog::default();
        catalog.add_particles(br#"
            particle={name=large subsystem={max_amount=3 subsystem={max_amount=4 subsystem={max_amount=2}}} subsystem={max_amount=5}}
            particle={name=hidden subsystem={hide=yes max_amount=900}}
            particle={name=small subsystem={max_amount=6}}
        "#, "fixture").unwrap();
        assert_eq!(catalog.max_particles(), 44);
        assert!(
            catalog
                .add_particles(b"particle={name=bad subsystem={}}", "fixture")
                .unwrap_err()
                .to_string()
                .contains("max_amount")
        );
    }

    #[test]
    fn gameplay_registration_uses_startup_then_native_directory_order() {
        let interface = [
            "interface/a/nested.gfx",
            "interface/z.gfx",
            "interface/core_bare_minimum.gfx",
            "interface/load_screen.gfx",
        ]
        .map(String::from);
        let gfx = [
            "gfx/a/nested.gfx",
            "gfx/z.gfx",
            "gfx/ignored.asset",
            "gfx/A.gfx",
        ]
        .map(String::from);
        assert_eq!(
            object_file_order(&interface, &gfx),
            [
                "interface/load_screen.gfx",
                "interface/core_bare_minimum.gfx",
                "interface/z.gfx",
                "interface/a/nested.gfx",
                "gfx/A.gfx",
                "gfx/z.gfx",
                "gfx/a/nested.gfx"
            ]
        );
        let mut catalog = Catalog::default();
        catalog
            .add_objects(
                b"objectTypes={pdxparticle={name=z type=shared} pdxparticle={name=a type=shared}}",
            )
            .unwrap();
        catalog
            .add_particles(b"particle={name=shared}", "gfx/particles/shared.asset")
            .unwrap();
        assert_eq!(catalog.resolve("z").unwrap().registration, 0);
        assert_eq!(catalog.resolve("a").unwrap().registration, 1);
    }

    #[test]
    fn render_descriptor_preserves_native_strings_tiles_and_suffix_order() {
        let doc = parser::parse(br#"texture={file="one" file="two" shader="ModEffect" x=3 y=-2} trail=yes billboard=no local_space=yes"#).unwrap();
        let descriptor = RenderDescriptor::read(&doc.items).unwrap();
        assert_eq!(descriptor.texture, "two");
        assert_eq!(descriptor.tiles, [3, -2]);
        assert!(descriptor.local_space);
        assert_eq!(descriptor.technique(), "ModEffectTrailNoBillboard");
        let empty = RenderDescriptor::read(&[]).unwrap();
        assert_eq!(empty.tiles, [1, 1]);
        assert_eq!(empty.technique(), "");
        assert!(empty.billboard);
        let zero = parser::parse(b"texture={file=bad.dds x=0}").unwrap();
        assert!(
            RenderDescriptor::read(&zero.items)
                .unwrap_err()
                .to_string()
                .contains("bad.dds has zero atlas dimension")
        );
    }

    #[test]
    fn latest_object_wins_without_reusing_its_registration_ordinal() {
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
                b"particle = { name = \"unrelated_id\" subsystem = { max_amount=1 } }",
                "gfx/particles/custom/any_file.asset",
            )
            .unwrap();
        assert_eq!(
            catalog.resolve("drops").unwrap(),
            Particle {
                path: "gfx/particles/custom/any_file.asset".into(),
                name: "unrelated_id".into(),
                scale: 0.5,
                registration: 0,
            }
        );
        assert!(catalog.resolve("bare").is_err());
        assert!(catalog.resolve("absent").is_err());
        catalog
            .add_objects(
                br#"objectTypes = {
            pdxparticle = { name = drops type = unrelated_id scale = 2 }
            pdxparticle = { name = other type = unrelated_id }
            pdxparticle = { name = Drops type = unrelated_id }
        }"#,
            )
            .unwrap();
        let replaced = catalog.resolve("drops").unwrap();
        assert_eq!(replaced.scale, 2.);
        assert_eq!(replaced.registration, 2);
        assert_eq!(catalog.resolve("other").unwrap().registration, 3);
        assert_eq!(catalog.resolve("Drops").unwrap().registration, 4);
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
