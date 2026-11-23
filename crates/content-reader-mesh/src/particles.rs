
use sha2::Digest;
use std::{collections::BTreeMap, io};

use content_reader_layers::{ResolvedLayers, script_files::ScriptPath};
use content_reader_parser::{crt, fields::{all, block, invalid, last, number, require, text}, parser};

const STARTUP_GFX: [&str; 3] = [
    "interface/load_screen.gfx",
    "interface/load_screen_font.gfx",
    "interface/core_bare_minimum.gfx",
];

pub(crate) fn object_file_order(
    interface: &[ScriptPath],
    gfx: &[ScriptPath],
) -> Vec<ScriptPath> {
    let mut files: Vec<_> = STARTUP_GFX
        .iter()
        .filter(|path| interface.iter().any(|file| file == *path))
        .map(|path| ScriptPath::from(*path))
        .collect();
    files.extend(
        content_reader_defines::defines::load_order("interface/", "gfx", interface)
            .into_iter()
            .filter(|path| !STARTUP_GFX.iter().any(|startup| path == startup)),
    );
    files.extend(content_reader_defines::defines::load_order("gfx/", "gfx", gfx));
    files
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderDescriptor {
    pub texture: String,
    pub shader: String,
    pub tiles: [i32; 2],
    pub trail: bool,
    pub billboard: bool,
    pub local_space: bool,
    pub sort: Sort,
    pub invert: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    None,
    Depth,
    Distance,
    Age,
}

impl Sort {
    fn read(fields: &[parser::Item]) -> Self {
        fields
            .iter()
            .filter(|item| item.keyword_is("sort"))
            .fold(Self::None, |state, item| {
                match item
                    .value
                    .read_string()
                    .map(<[u8]>::to_ascii_lowercase)
                    .as_deref()
                {
                    Some(b"depth") => Self::Depth,
                    Some(b"distance") => Self::Distance,
                    Some(b"age") => Self::Age,
                    _ => state,
                }
            })
    }
}

impl RenderDescriptor {
    pub fn read(fields: &[parser::Item]) -> io::Result<Self> {
        let mut out = Self {
            texture: String::new(),
            shader: String::new(),
            tiles: [1; 2],
            trail: parser::read_bool_field(fields, "trail", false),
            local_space: parser::read_bool_field(fields, "local_space", false),
            billboard: fields
                .iter()
                .filter(|item| item.keyword_is("billboard"))
                .fold(true, |_, item| {
                    item.value.scalar().map(parser::c_string) == Some(b"yes")
                }),
            sort: Sort::read(fields),
            invert: parser::read_bool_field(fields, "invert", false),
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
                        out.tiles[index] =
                            crt::scan_i32(text(&item.value, "particle texture tiles")?.as_bytes())
                                .stored()
                                .ok_or_else(|| {
                                    invalid("particle texture tiles require signed int32")
                                })?;
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

    pub fn technique(&self) -> String {
        format!(
            "{}{}{}",
            self.shader,
            if self.trail { "Trail" } else { "" },
            if self.billboard { "" } else { "NoBillboard" }
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub path: ScriptPath,
    pub name: String,
    pub scale: f32,
    pub registration: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    objects: BTreeMap<String, Result<(String, f32), String>>,
    ordinals: BTreeMap<String, usize>,
    registrations: usize,
    files: BTreeMap<String, Result<ScriptPath, String>>,
    assets: Vec<ScriptPath>,
    refused: Vec<String>,
}

impl Catalog {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut out = Self::default();
        let interface = layers.descriptor_paths("interface", &[".gfx"])?;
        let gfx = layers.descriptor_paths("gfx", &[".gfx", ".asset"])?;
        out.assets = gfx
            .iter()
            .filter(|path| path.as_bytes().to_ascii_lowercase().ends_with(b".asset"))
            .cloned()
            .collect();
        let gfx = object_file_order(&interface, &gfx);
        for path in gfx {
            let bytes = match path.read_uncovered(layers) {
                Ok(bytes) => bytes,
                Err(error) => {
                    out.refused.push(format!("{path}: {error}"));
                    continue;
                }
            };
            if bytes
                .windows(11)
                .any(|word| word.eq_ignore_ascii_case(b"pdxparticle"))
            {
                out.add_objects(&bytes, &path);
            }
        }
        let mut opened = std::collections::BTreeSet::new();
        for path in &out.assets.clone() {
            let lower = path.as_bytes().to_ascii_lowercase();
            if lower.starts_with(b"gfx/particles/") {
                let bytes = match path.read_uncovered(layers) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        out.refused.push(format!("{path}: {error}"));
                        continue;
                    }
                };
                let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
                if !opened.insert((lower, digest)) {
                    continue;
                }
                out.add_particles(&bytes, path);
            }
        }
        Ok(out)
    }

    pub fn refused(&self) -> &[String] {
        &self.refused
    }

    pub fn assets(&self) -> &[ScriptPath] {
        &self.assets
    }

    fn add_objects(&mut self, bytes: &[u8], path: &ScriptPath) {
        let document = match content_reader_parser::parser::parse_unclosed(bytes) {
            Ok((document, _)) => document,
            Err(error) => return self.refused.push(format!("{path}: {error}")),
        };
        for objects in keyword_values(&document.items, "objectTypes") {
            let objects = match block(objects, "objectTypes") {
                Ok(objects) => objects,
                Err(error) => {
                    self.refused.push(format!("{path}: {error}"));
                    continue;
                }
            };
            for value in keyword_values(objects, "pdxparticle") {
                let ordinal = self.registrations;
                self.registrations += 1;
                let Ok(fields) = block(value, "pdxparticle") else {
                    self.refused
                        .push(format!("{path}: pdxparticle {ordinal} is not a block"));
                    continue;
                };
                let field = |key: &'static str| {
                    keyword_values(fields, key)
                        .next_back()
                        .ok_or_else(|| invalid(format!("pdxparticle has no {key}")))
                };
                let name = match field("name").and_then(|name| text(name, "pdxparticle name")) {
                    Ok(name) => name,
                    Err(error) => {
                        self.refused.push(format!("{path}: {error}"));
                        continue;
                    }
                };
                let read = || -> io::Result<(String, f32)> {
                    let kind = text(field("type")?, "pdxparticle type")?;
                    let scale = match keyword_values(fields, "scale").next_back() {
                        Some(value) => number(value, "pdxparticle scale")?,
                        None => 1.0,
                    };
                    if !scale.is_finite() || scale <= 0.0 {
                        return Err(invalid(format!("pdxparticle {name} has scale {scale}")));
                    }
                    Ok((kind.to_owned(), scale))
                };
                let read = read().map_err(|error| format!("{path}: pdxparticle {name}: {error}"));
                self.objects.insert(name.to_owned(), read);
                self.ordinals.insert(name.to_owned(), ordinal);
            }
        }
    }

    fn add_particles(&mut self, bytes: &[u8], path: &ScriptPath) {
        let document = match parser::parse(bytes) {
            Ok(document) => document,
            Err(error) => return self.refused.push(format!("{path}: {error}")),
        };
        for value in all(&document.items, b"particle") {
            let name = match block(value, "particle")
                .and_then(|fields| text(require(fields, "name", "particle")?, "particle name"))
            {
                Ok(name) => name,
                Err(error) => {
                    self.refused.push(format!("{path}: {error}"));
                    continue;
                }
            };
            let fields = block(value, "particle").unwrap_or_default();
            let read = match capacity(fields) {
                Ok(_) => Ok(path.clone()),
                Err(error) => Err(format!("{path}: particle {name}: {error}")),
            };
            match self.files.get(name) {
                None => {
                    self.files.insert(name.to_owned(), read);
                }
                Some(earlier) => {
                    let earlier = earlier.as_ref().map_or_else(Clone::clone, |p| p.to_string());
                    self.files.insert(
                        name.to_owned(),
                        Err(format!(
                            "particle {name} is declared twice ({earlier}; {path}), with unsettled precedence"
                        )),
                    );
                }
            }
        }
    }

    pub fn resolve(&self, object: &str) -> io::Result<Particle> {
        let (kind, scale) = self
            .objects
            .get(object)
            .ok_or_else(|| invalid(format!("no pdxparticle {object}")))?
            .as_ref()
            .map_err(|why| invalid(why.clone()))?;
        let path = self
            .files
            .get(kind)
            .ok_or_else(|| {
                invalid(format!(
                    "pdxparticle {object} names missing particle {kind}"
                ))
            })?
            .as_ref()
            .map_err(|why| invalid(format!("pdxparticle {object}: {why}")))?;
        Ok(Particle {
            path: path.clone(),
            name: kind.clone(),
            scale: *scale,
            registration: self.ordinals[object],
        })
    }

    #[cfg(any(test, feature = "layer-fixtures"))]
    pub fn add_test_particle(&mut self, object: &str, kind: &str, scale: f32, path: &str) {
        let ordinal = self.registrations;
        self.registrations += 1;
        self.ordinals.insert(object.into(), ordinal);
        self.objects.insert(object.into(), Ok((kind.into(), scale)));
        self.files.insert(kind.into(), Ok(ScriptPath::from(path)));
    }

    pub fn objects(&self) -> impl Iterator<Item = &str> {
        self.objects.keys().map(String::as_str)
    }
}

fn keyword_values<'a>(
    items: &'a [parser::Item],
    key: &'a str,
) -> impl DoubleEndedIterator<Item = &'a parser::Value> {
    items
        .iter()
        .filter(move |item| item.keyword_is(key))
        .map(|item| &item.value)
}

fn capacity(fields: &[parser::Item]) -> io::Result<usize> {
    let mut total = 0usize;
    for value in all(fields, b"subsystem") {
        let fields = block(value, "particle subsystem")?;
        let hidden = parser::read_bool_field(fields, "hide", false);
        let count = if hidden {
            0
        } else {
            subsystem_capacity(fields)?
        };
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

pub fn subsystem_capacity(fields: &[parser::Item]) -> io::Result<usize> {
    let value = text(
        require(fields, "max_amount", "particle capacity")?,
        "max_amount",
    )?;
    let value = crt::scan_i32(value.as_bytes())
        .stored()
        .ok_or_else(|| invalid("particle max_amount requires int32"))?;
    usize::try_from(value).map_err(|_| invalid("negative particle max_amount"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_flags_keep_previous_values_on_failed_reads() {
        for (source, expected) in [
            ("trail = YES local_space = NO", false),
            (
                "trail = yes trail = YES local_space = yes local_space = NO",
                true,
            ),
            (
                "trail = yes trail = no local_space = yes local_space = no",
                false,
            ),
        ] {
            let document = parser::parse(source.as_bytes()).unwrap();
            let descriptor = RenderDescriptor::read(&document.items).unwrap();
            assert_eq!(
                (descriptor.trail, descriptor.local_space),
                (expected, expected)
            );
        }
    }

    #[test]
    fn sort_folds_case_and_keeps_its_value_past_an_unknown_one() {
        for (source, sort, invert) in [
            ("", Sort::None, false),
            ("sort = \"depth\"", Sort::Depth, false),
            ("sort = DISTANCE invert = yes", Sort::Distance, true),
            (
                "sort = age sort = nearest invert = yes invert = YES",
                Sort::Age,
                true,
            ),
            (
                "sort = depth sort = { age } invert = yes invert = no",
                Sort::Depth,
                false,
            ),
        ] {
            let document = parser::parse(source.as_bytes()).unwrap();
            let descriptor = RenderDescriptor::read(&document.items).unwrap();
            assert_eq!(
                (descriptor.sort, descriptor.invert),
                (sort, invert),
                "{source}"
            );
        }
    }

    #[test]
    fn billboard_assignments_are_yes_only_c_strings() {
        for (source, expected) in [
            (b"billboard = no".as_slice(), false),
            (b"billboard = NO", false),
            (b"billboard = \"yes\"", true),
            (b"billboard = yes billboard = NO", false),
            (b"billboard = no billboard = yes", true),
        ] {
            let document = parser::parse(source).unwrap();
            assert_eq!(
                RenderDescriptor::read(&document.items).unwrap().billboard,
                expected
            );
        }
        let mut document = parser::parse(b"billboard = no").unwrap();
        document.items[0].value = parser::Value::Scalar(b"yes\0x".to_vec());
        assert!(RenderDescriptor::read(&document.items).unwrap().billboard);
    }

    #[test]
    fn capacity_replicates_children_and_refuses_a_block_without_one() {
        let mut catalog = Catalog::default();
        catalog.add_particles(br#"
            particle={name=large subsystem={max_amount=3 subsystem={max_amount=4 subsystem={max_amount=2}}} subsystem={max_amount=5}}
            particle={name=hidden subsystem={hide=yes max_amount=900}}
            particle={name=small subsystem={max_amount=6}}
        "#, &"fixture".into());
        let blocks = parser::parse(br#"
            particle={name=large subsystem={max_amount=3 subsystem={max_amount=4 subsystem={max_amount=2}}} subsystem={max_amount=5}}
            particle={name=hidden subsystem={hide=yes max_amount=900}}
        "#).unwrap();
        let capacities: Vec<_> = all(&blocks.items, b"particle")
            .map(|block| capacity(block.items().unwrap()).unwrap())
            .collect();
        assert_eq!(capacities, [44, 0]);
        catalog.add_objects(
            b"objectTypes={pdxparticle={name=uses_bad type=bad} pdxparticle={name=uses_small type=small} pdxparticle={name=zero type=small scale=0}}",
            &"fixture".into(),
        );
        catalog.add_particles(b"particle={name=bad subsystem={}}", &"fixture".into());
        let error = catalog.resolve("uses_bad").unwrap_err().to_string();
        assert!(
            error.contains("pdxparticle uses_bad: fixture: particle bad") && error.contains("max_amount"),
            "{error}"
        );
        let error = catalog.resolve("zero").unwrap_err().to_string();
        assert!(error.contains("pdxparticle zero has scale 0"), "{error}");
        assert_eq!(catalog.resolve("uses_small").unwrap().registration, 1);
    }

    #[test]
    fn particle_numbers_follow_the_crt_prefix_scan() {
        let document = parser::parse(br#"texture = { x = "2tail" y = "3tail" }"#).unwrap();
        assert_eq!(
            RenderDescriptor::read(&document.items).unwrap().tiles,
            [2, 3]
        );
        let mut catalog = Catalog::default();
        catalog
            .add_particles(
                br#"particle = { name = p subsystem = { max_amount = "4tail" } }"#,
                &"fixture".into(),
            );
        let block = parser::parse(br#"subsystem = { max_amount = "4tail" }"#).unwrap();
        assert_eq!(capacity(&block.items).unwrap(), 4);
    }

    #[test]
    fn a_subsystem_capacity_is_the_catalogue_s_int32_scan() {
        for (source, expected) in [
            (br#"max_amount = "4tail""#.as_slice(), 4),
            (b"max_amount = 0x10", 16),
            (b"max_amount = 010", 8),
            (b"max_amount = 1e99", 1),
        ] {
            let document = parser::parse(source).unwrap();
            assert_eq!(subsystem_capacity(&document.items).unwrap(), expected);
        }
        for source in [b"max_amount = -1".as_slice(), b"hide = no"] {
            let document = parser::parse(source).unwrap();
            assert!(subsystem_capacity(&document.items).is_err());
        }
    }

    #[test]
    fn gameplay_registration_uses_startup_then_directory_order() {
        let interface = [
            "interface/a/nested.gfx",
            "interface/z.gfx",
            "interface/core_bare_minimum.gfx",
            "interface/load_screen.gfx",
        ]
        .map(ScriptPath::from);
        let gfx = [
            "gfx/a/nested.gfx",
            "gfx/z.gfx",
            "gfx/ignored.asset",
            "gfx/A.gfx",
        ]
        .map(ScriptPath::from);
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
                b"objectTypes={pdxparticle={name=z type=shared} pdxparticle={name=a type=shared}}", &"fixture".into());
        catalog
            .add_particles(b"particle={name=shared}", &"gfx/particles/shared.asset".into());
        assert_eq!(catalog.resolve("z").unwrap().registration, 0);
        assert_eq!(catalog.resolve("a").unwrap().registration, 1);
    }

    #[test]
    fn render_descriptor_preserves_strings_tiles_and_suffix_order() {
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
    fn objects_dispatch_by_keyword_and_keep_exact_identifiers() {
        let mut catalog = Catalog::default();
        catalog
            .add_objects(
                br#"OBJECTTYPES = {
                    PDXPARTICLE = { NAME = Rain TYPE = Rain_Particle SCALE = 2 }
                    PdxParticle = { Name = snow Type = snow_particle }
                    "pdxparticle" = { name = quoted type = snow_particle }
                    pdxparticle = { "name" = keyed name = keyed_name type = snow_particle }
                }"#, &"fixture".into());
        catalog
            .add_particles(
                b"particle = { name = Rain_Particle } particle = { name = snow_particle }",
                &"gfx/particles/weather.asset".into(),
            );
        let rain = catalog.resolve("Rain").unwrap();
        assert_eq!((rain.name.as_str(), rain.scale), ("Rain_Particle", 2.0));
        assert!(catalog.resolve("rain").is_err());
        assert_eq!(catalog.resolve("snow").unwrap().registration, 1);
        assert!(catalog.resolve("quoted").is_err());
        assert!(catalog.resolve("keyed").is_err());
        assert_eq!(catalog.resolve("keyed_name").unwrap().registration, 2);
        catalog.add_objects(
            b"objectTypes = { pdxparticle = { NAME = typeless } pdxparticle = { type = x } }",
            &"fixture".into(),
        );
        let error = catalog.resolve("typeless").unwrap_err().to_string();
        assert!(error.contains("pdxparticle has no type"), "{error}");
        assert_eq!(catalog.refused(), ["fixture: pdxparticle has no name"]);
    }

    #[test]
    fn latest_object_wins_without_reusing_its_registration_ordinal() {
        let mut catalog = Catalog::default();
        catalog
            .add_objects(
                br#"objectTypes = {
                    pdxparticle = { name = "drops" type = "unrelated_id" scale = 0.5 }
                    pdxparticle = { name = "bare" type = "absent" }
                }"#, &"fixture".into());
        catalog
            .add_particles(
                b"particle = { name = \"unrelated_id\" subsystem = { max_amount=1 } }",
                &"gfx/particles/custom/any_file.asset".into(),
            );
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
        }"#, &"fixture".into());
        let replaced = catalog.resolve("drops").unwrap();
        assert_eq!(replaced.scale, 2.);
        assert_eq!(replaced.registration, 2);
        assert_eq!(catalog.resolve("other").unwrap().registration, 3);
        assert_eq!(catalog.resolve("Drops").unwrap().registration, 4);
        catalog.add_particles(
            b"particle = { name = unrelated_id }",
            &"gfx/particles/b.asset".into(),
        );
        let error = catalog.resolve("drops").unwrap_err().to_string();
        assert!(error.contains("particle unrelated_id is declared twice"), "{error}");
    }
}
