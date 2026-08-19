use crate::{
    AdmittedAuthorityPayloads, ExactAuthorityPayloadRequest, RawEffectiveVfsWinnerIndexBinding,
    RawVfsContentIdentity,
};
use portable_content_manifest::normalize_game_relative_path;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterpretedParticleContent {
    pub subsystem_selector_u32: u32,
    pub definition_content: RawVfsContentIdentity,
    pub texture_content: RawVfsContentIdentity,
    pub authored_shader_token: String,
    pub authored_trail_token: Option<String>,
    pub authored_billboard_token: Option<String>,
    pub explicit_engine_effect_token_crosscheck: Option<String>,
    pub atlas_columns: u32,
    pub atlas_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterpretedEntityContent {
    pub resolved_entity_asset: RawVfsContentIdentity,
    pub mesh: RawVfsContentIdentity,
}

#[derive(Clone, Debug, Default)]
pub struct ProductionRuntimeContentInterpreter {
    particle_registrations: BTreeMap<String, Vec<String>>,
    particle_definitions: BTreeMap<String, Vec<InterpretedParticleDefinition>>,
    entities: BTreeMap<String, Vec<(RawVfsContentIdentity, String)>>,
    meshes: BTreeMap<String, Vec<RawVfsContentIdentity>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InterpretedParticleDefinition {
    definition_content: RawVfsContentIdentity,
    ordered_subsystems: Vec<InterpretedParticleContent>,
}

impl ProductionRuntimeContentInterpreter {
    pub fn required_payloads(
        winner_index: &RawEffectiveVfsWinnerIndexBinding,
    ) -> Result<Vec<ExactAuthorityPayloadRequest>, String> {
        winner_index.validate()?;
        Ok(winner_index
            .ordered_rows
            .iter()
            .filter(|row| relevant_definition_path(&row.content.logical_path))
            .map(|row| ExactAuthorityPayloadRequest {
                content: row.content.clone(),
                byte_offset: 0,
                byte_count: row.content.payload_bytes,
                range_sha256: row.content.payload_sha256.clone(),
            })
            .collect())
    }

    pub fn build(
        winner_index: &RawEffectiveVfsWinnerIndexBinding,
        payloads: &impl AdmittedAuthorityPayloads,
    ) -> Result<Self, String> {
        winner_index.validate()?;
        let mut output = Self::default();
        for row in &winner_index.ordered_rows {
            if !relevant_definition_path(&row.content.logical_path) {
                continue;
            }
            let bytes = payloads
                .payload(&row.content)
                .ok_or("runtime content interpreter payload was not admitted over H4CT")?;
            if bytes.len() as u64 != row.content.payload_bytes {
                return Err("runtime content interpreter payload length drifted".into());
            }
            let text = std::str::from_utf8(bytes)
                .map_err(|_| "runtime content definition is not UTF-8")?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(text);
            let root = Parser::parse(text)
                .map_err(|error| format!("parse {}: {error}", row.content.logical_path))?;
            collect_blocks(&root, "pdxparticle", &mut |block| {
                if let (Some(name), Some(source)) = (
                    optional_unique_scalar(block, "name")?,
                    optional_unique_scalar(block, "type")?,
                ) {
                    output
                        .particle_registrations
                        .entry(name)
                        .or_default()
                        .push(source);
                }
                Ok(())
            })?;
            collect_blocks(&root, "particle", &mut |block| {
                let Some(name) = optional_unique_scalar(block, "name")? else {
                    return Ok(());
                };
                let mut ordered_subsystems = Vec::new();
                collect_subsystem_textures(block, &mut |subsystem, texture| {
                    let file = normalize_game_relative_path(&unique_scalar(texture, "file")?)?;
                    let texture_content = exact_winner(winner_index, &file)?.clone();
                    let rows = parse_nonzero_u32(&unique_scalar(texture, "y")?)?;
                    let columns = parse_nonzero_u32(&unique_scalar(texture, "x")?)?;
                    let authored_shader_token = unique_scalar(texture, "shader")?;
                    if authored_shader_token.trim().is_empty() {
                        return Err("particle subsystem has an empty shader token".into());
                    }
                    let authored_trail_token = optional_unique_scalar(subsystem, "trail")?;
                    let authored_billboard_token = optional_unique_scalar(subsystem, "billboard")?;
                    let explicit_engine_effect_token_crosscheck = explicit_engine_effect_token(
                        &authored_shader_token,
                        authored_trail_token.as_deref(),
                        authored_billboard_token.as_deref(),
                    );
                    let subsystem_selector_u32 = u32::try_from(ordered_subsystems.len())
                        .map_err(|_| "particle subsystem ordinal overflowed u32")?;
                    ordered_subsystems.push(InterpretedParticleContent {
                        subsystem_selector_u32,
                        definition_content: row.content.clone(),
                        texture_content,
                        authored_shader_token,
                        authored_trail_token,
                        authored_billboard_token,
                        explicit_engine_effect_token_crosscheck,
                        atlas_columns: columns,
                        atlas_rows: rows,
                    });
                    Ok(())
                })?;
                output.particle_definitions.entry(name).or_default().push(
                    InterpretedParticleDefinition {
                        definition_content: row.content.clone(),
                        ordered_subsystems,
                    },
                );
                Ok(())
            })?;
            collect_blocks(&root, "entity", &mut |block| {
                let Some(name) = optional_unique_scalar(block, "name")? else {
                    return Ok(());
                };
                let Some(mesh) = optional_unique_scalar(block, "pdxmesh")? else {
                    return Ok(());
                };
                output
                    .entities
                    .entry(name)
                    .or_default()
                    .push((row.content.clone(), mesh));
                Ok(())
            })?;
            collect_blocks(&root, "pdxmesh", &mut |block| {
                let (Some(name), Some(file)) = (
                    optional_unique_scalar(block, "name")?,
                    optional_unique_scalar(block, "file")?,
                ) else {
                    return Ok(());
                };
                let Ok(path) = normalize_game_relative_path(&file) else {
                    return Ok(());
                };
                if let Ok(mesh) = exact_winner(winner_index, &path) {
                    output.meshes.entry(name).or_default().push(mesh.clone());
                }
                Ok(())
            })?;
        }
        Ok(output)
    }

    pub fn resolve_particle(
        &self,
        registration_token: &str,
        definition_source_token: &str,
        flattened_subsystem_ordinal: u32,
    ) -> Result<InterpretedParticleContent, String> {
        let registered_sources = self
            .particle_registrations
            .get(registration_token)
            .ok_or("particle engine registration is absent from admitted VFS definitions")?;
        if registered_sources.len() != 1
            || registered_sources[0] != definition_source_token
            || registration_token.is_empty()
            || definition_source_token.is_empty()
        {
            return Err("particle registration/definition join is ambiguous".into());
        }
        let definition = unique_ref(
            self.particle_definitions.get(definition_source_token),
            "particle definition source token",
        )?;
        if definition.definition_content.logical_path.is_empty() {
            return Err("particle definition winner is incomplete".into());
        }
        definition
            .ordered_subsystems
            .get(flattened_subsystem_ordinal as usize)
            .cloned()
            .ok_or_else(|| "particle flattened subsystem ordinal is out of range".into())
    }

    pub fn resolve_entity(&self, entity_name: &str) -> Result<InterpretedEntityContent, String> {
        if entity_name.is_empty() {
            return Err("entity engine registration token is empty".into());
        }
        let (asset, mesh_token) =
            unique_ref(self.entities.get(entity_name), "entity registration token")?;
        let mesh = unique_ref(self.meshes.get(mesh_token), "entity pdxmesh token")?;
        Ok(InterpretedEntityContent {
            resolved_entity_asset: asset.clone(),
            mesh: mesh.clone(),
        })
    }
}

fn relevant_definition_path(path: &str) -> bool {
    path.starts_with("gfx/") && (path.ends_with(".asset") || path.ends_with(".gfx"))
}

fn exact_winner<'a>(
    winner_index: &'a RawEffectiveVfsWinnerIndexBinding,
    path: &str,
) -> Result<&'a RawVfsContentIdentity, String> {
    let index = winner_index
        .ordered_rows
        .binary_search_by(|row| row.content.logical_path.as_str().cmp(path))
        .map_err(|_| "definition interpreter references a missing effective-VFS winner")?;
    Ok(&winner_index.ordered_rows[index].content)
}

fn unique_ref<'a, T>(rows: Option<&'a Vec<T>>, label: &str) -> Result<&'a T, String> {
    let rows = rows.ok_or_else(|| format!("{label} is missing"))?;
    if rows.len() != 1 {
        return Err(format!("{label} is ambiguous"));
    }
    Ok(&rows[0])
}

fn parse_nonzero_u32(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| "particle atlas dimension is not a nonzero integer".into())
}

#[derive(Clone, Debug)]
enum Value {
    Atom(String),
    Block(Vec<Entry>),
}

#[derive(Clone, Debug)]
enum Entry {
    Pair(String, Value),
    Atom,
}

fn unique_scalar(entries: &[Entry], key: &str) -> Result<String, String> {
    optional_unique_scalar(entries, key)?
        .ok_or_else(|| format!("definition block has no scalar {key:?}"))
}

fn optional_unique_scalar(entries: &[Entry], key: &str) -> Result<Option<String>, String> {
    let rows = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Pair(found, Value::Atom(value)) if found.eq_ignore_ascii_case(key) => {
                Some(value.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].is_empty() {
        return Err(format!("definition block has ambiguous scalar {key:?}"));
    }
    Ok(Some(rows[0].to_owned()))
}

fn collect_blocks(
    entries: &[Entry],
    key: &str,
    visitor: &mut impl FnMut(&[Entry]) -> Result<(), String>,
) -> Result<(), String> {
    for entry in entries {
        if let Entry::Pair(found, Value::Block(block)) = entry {
            if found.eq_ignore_ascii_case(key) {
                visitor(block)?;
            }
            collect_blocks(block, key, visitor)?;
        }
    }
    Ok(())
}

fn collect_subsystem_textures(
    entries: &[Entry],
    visitor: &mut impl FnMut(&[Entry], &[Entry]) -> Result<(), String>,
) -> Result<(), String> {
    for entry in entries {
        if let Entry::Pair(found, Value::Block(block)) = entry {
            if found.eq_ignore_ascii_case("subsystem") {
                collect_blocks(block, "texture", &mut |texture| visitor(block, texture))?;
            } else {
                collect_subsystem_textures(block, visitor)?;
            }
        }
    }
    Ok(())
}

fn explicit_engine_effect_token(
    authored_shader_token: &str,
    authored_trail_token: Option<&str>,
    authored_billboard_token: Option<&str>,
) -> Option<String> {
    let (Some(trail), Some(billboard)) = (authored_trail_token, authored_billboard_token) else {
        return None;
    };
    if !matches!(trail, "yes" | "no") || !matches!(billboard, "yes" | "no") {
        return None;
    }
    let mut token = authored_shader_token.to_owned();
    if trail == "yes" {
        token.push_str("Trail");
    }
    if billboard == "no" {
        token.push_str("NoBillboard");
    }
    Some(token)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Atom(String),
    Equals,
    Open,
    Close,
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn parse(text: &str) -> Result<Vec<Entry>, String> {
        let mut parser = Self {
            tokens: tokenize(text)?,
            at: 0,
        };
        let rows = parser.entries(false)?;
        if parser.at != parser.tokens.len() {
            return Err("definition parser left trailing tokens".into());
        }
        Ok(rows)
    }

    fn entries(&mut self, nested: bool) -> Result<Vec<Entry>, String> {
        let mut rows = Vec::new();
        loop {
            match self.tokens.get(self.at) {
                None if nested => return Err("definition block is missing a closing brace".into()),
                None => return Ok(rows),
                Some(Token::Close) if nested => {
                    self.at += 1;
                    return Ok(rows);
                }
                Some(Token::Close) => {
                    return Err("definition has an unmatched closing brace".into())
                }
                Some(Token::Open | Token::Equals) => {
                    return Err("definition has an unexpected structural token".into())
                }
                Some(Token::Atom(key)) => {
                    let key = key.clone();
                    self.at += 1;
                    if self.tokens.get(self.at) != Some(&Token::Equals) {
                        rows.push(Entry::Atom);
                        continue;
                    }
                    self.at += 1;
                    let value = match self.tokens.get(self.at).cloned() {
                        Some(Token::Atom(value)) => {
                            self.at += 1;
                            Value::Atom(value)
                        }
                        Some(Token::Open) => {
                            self.at += 1;
                            Value::Block(self.entries(true)?)
                        }
                        _ => return Err(format!("definition key {key:?} has no value")),
                    };
                    rows.push(Entry::Pair(key, value));
                }
            }
        }
    }
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut output = Vec::new();
    let bytes = text.as_bytes();
    let mut at = 0usize;
    while at < bytes.len() {
        match bytes[at] {
            b'#' => {
                at += 1;
                while at < bytes.len() && !matches!(bytes[at], b'\r' | b'\n') {
                    at += 1;
                }
            }
            byte if byte.is_ascii_whitespace() || byte == b',' => at += 1,
            b'=' => {
                output.push(Token::Equals);
                at += 1;
            }
            b'{' => {
                output.push(Token::Open);
                at += 1;
            }
            b'}' => {
                output.push(Token::Close);
                at += 1;
            }
            b'"' => {
                at += 1;
                let mut value = Vec::new();
                let mut closed = false;
                while at < bytes.len() {
                    match bytes[at] {
                        b'"' => {
                            at += 1;
                            closed = true;
                            break;
                        }
                        b'\\' if at + 1 < bytes.len() => {
                            at += 1;
                            value.push(bytes[at]);
                            at += 1;
                        }
                        byte => {
                            value.push(byte);
                            at += 1;
                        }
                    }
                }
                if !closed {
                    return Err("definition contains an unterminated string".into());
                }
                output.push(Token::Atom(
                    String::from_utf8(value).map_err(|_| "definition string is not UTF-8")?,
                ));
            }
            _ => {
                let start = at;
                while at < bytes.len()
                    && !bytes[at].is_ascii_whitespace()
                    && !matches!(bytes[at], b'#' | b',' | b'=' | b'{' | b'}' | b'"')
                {
                    at += 1;
                }
                if start == at {
                    return Err("definition tokenizer made no progress".into());
                }
                output.push(Token::Atom(
                    std::str::from_utf8(&bytes[start..at])
                        .map_err(|_| "definition token is not UTF-8")?
                        .to_owned(),
                ));
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawEffectiveVfsWinnerRow;
    use sha2::{Digest, Sha256};

    #[derive(Default)]
    struct Payloads(Vec<(RawVfsContentIdentity, Vec<u8>)>);

    impl AdmittedAuthorityPayloads for Payloads {
        fn payload_range(
            &self,
            content: &RawVfsContentIdentity,
            byte_offset: u64,
            byte_count: u64,
            range_sha256: &str,
        ) -> Option<&[u8]> {
            let bytes = &self.0.iter().find(|(key, _)| key == content)?.1;
            let start = usize::try_from(byte_offset).ok()?;
            let end = start.checked_add(usize::try_from(byte_count).ok()?)?;
            let range = bytes.get(start..end)?;
            (sha(range) == range_sha256).then_some(range)
        }
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn row(path: &str, payload: &[u8]) -> RawEffectiveVfsWinnerRow {
        let digest = sha(payload);
        RawEffectiveVfsWinnerRow {
            content: RawVfsContentIdentity {
                logical_path: path.into(),
                winning_layer_ordinal: 3,
                layer_kind: "mod".into(),
                layer_id: "mod/runtime_override.mod".into(),
                source_sha256: digest.clone(),
                payload_sha256: digest.clone(),
                payload_bytes: payload.len() as u64,
            },
            source_container_sha256: None,
            source_byte_offset: 0,
            source_byte_count: payload.len() as u64,
            source_range_sha256: digest,
        }
    }

    fn fixture(files: &[(&str, &[u8])]) -> (RawEffectiveVfsWinnerIndexBinding, Payloads) {
        let mut rows = files
            .iter()
            .map(|(path, payload)| row(path, payload))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.content.logical_path.cmp(&right.content.logical_path));
        let payloads = Payloads(
            rows.iter()
                .map(|row| {
                    let payload = files
                        .iter()
                        .find(|(path, _)| *path == row.content.logical_path)
                        .unwrap()
                        .1
                        .to_vec();
                    (row.content.clone(), payload)
                })
                .collect(),
        );
        let binding = RawEffectiveVfsWinnerIndexBinding {
            process_start_nonce: 9,
            authority_manifest_sha256: "a".repeat(64),
            declared_row_count: rows.len() as u64,
            ordered_rows_sha256: RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(&rows)
                .unwrap(),
            ordered_rows: rows,
        };
        (binding, payloads)
    }

    #[test]
    fn particle_interpreter_uses_effective_mod_winners_and_exact_atlas() {
        let registration = br#"
            objectTypes = {
              pdxparticle = { name = "mod_effect" type = "mod_effect_file" }
            }
        "#;
        let definition = br#"
            particle = {
              name = "mod_effect_file"
              subsystem = { trail=no billboard=yes texture = {
                file="gfx/particles/mod.dds" x=4 y=2 shader="ParticleAlphaBlend"
              } }
            }
        "#;
        let (binding, payloads) = fixture(&[
            ("gfx/entities/particles.gfx", registration),
            ("gfx/particles/mod_effect.asset", definition),
            ("gfx/particles/mod.dds", b"mod texture"),
        ]);

        assert_eq!(
            ProductionRuntimeContentInterpreter::required_payloads(&binding)
                .unwrap()
                .len(),
            2
        );
        let interpreter = ProductionRuntimeContentInterpreter::build(&binding, &payloads).unwrap();
        let resolved = interpreter
            .resolve_particle("mod_effect", "mod_effect_file", 0)
            .unwrap();
        assert_eq!(resolved.definition_content.winning_layer_ordinal, 3);
        assert_eq!(resolved.definition_content.layer_kind, "mod");
        assert_eq!(resolved.subsystem_selector_u32, 0);
        assert_eq!(
            resolved.texture_content.logical_path,
            "gfx/particles/mod.dds"
        );
        assert_eq!(resolved.authored_shader_token, "ParticleAlphaBlend");
        assert_eq!(
            resolved.explicit_engine_effect_token_crosscheck.as_deref(),
            Some("ParticleAlphaBlend")
        );
        assert_eq!(resolved.atlas_rows, 2);
        assert_eq!(resolved.atlas_columns, 4);
        assert!(interpreter
            .resolve_particle("vanilla_effect", "mod_effect_file", 0)
            .is_err());
    }

    #[test]
    fn particle_interpreter_preserves_duplicate_subsystems_and_selects_packet_ordinal() {
        let registration = br#"pdxparticle={name="multi" type="multi_file"}"#;
        let definition = br#"
          particle={name="multi_file"
            subsystem={trail=yes billboard=yes texture={file="gfx/particles/a.dds" x=4 y=2 shader="ParticleAlpha"}}
            subsystem={trail=yes billboard=no texture={file="gfx/particles/b.dds" x=4 y=2 shader="ParticleAdd"}}
          }
        "#;
        let (binding, payloads) = fixture(&[
            ("gfx/entities/multi.gfx", registration),
            ("gfx/particles/multi.asset", definition),
            ("gfx/particles/a.dds", b"a texture"),
            ("gfx/particles/b.dds", b"b texture"),
        ]);
        let interpreter = ProductionRuntimeContentInterpreter::build(&binding, &payloads).unwrap();
        let first = interpreter
            .resolve_particle("multi", "multi_file", 0)
            .unwrap();
        let second = interpreter
            .resolve_particle("multi", "multi_file", 1)
            .unwrap();
        assert_eq!((first.atlas_rows, first.atlas_columns), (2, 4));
        assert_eq!((second.atlas_rows, second.atlas_columns), (2, 4));
        assert_eq!(first.texture_content.logical_path, "gfx/particles/a.dds");
        assert_eq!(second.texture_content.logical_path, "gfx/particles/b.dds");
        assert_eq!(first.authored_shader_token, "ParticleAlpha");
        assert_eq!(second.authored_shader_token, "ParticleAdd");
        assert_eq!(
            first.explicit_engine_effect_token_crosscheck.as_deref(),
            Some("ParticleAlphaTrail")
        );
        assert_eq!(
            second.explicit_engine_effect_token_crosscheck.as_deref(),
            Some("ParticleAddTrailNoBillboard")
        );
        assert!(interpreter
            .resolve_particle("multi", "multi_file", 2)
            .is_err());
    }

    #[test]
    fn particle_interpreter_rejects_duplicate_registration_definition_join() {
        let registration = br#"
          pdxparticle={name="duplicate" type="duplicate_file"}
          pdxparticle={name="duplicate" type="duplicate_file"}
        "#;
        let definition = br#"
          particle={name="duplicate_file"
            subsystem={texture={file="gfx/particles/a.dds" x=1 y=1 shader="ParticleAlpha"}}
          }
        "#;
        let (binding, payloads) = fixture(&[
            ("gfx/entities/duplicate.gfx", registration),
            ("gfx/particles/duplicate.asset", definition),
            ("gfx/particles/a.dds", b"a texture"),
        ]);
        let interpreter = ProductionRuntimeContentInterpreter::build(&binding, &payloads).unwrap();
        assert!(interpreter
            .resolve_particle("duplicate", "duplicate_file", 0)
            .unwrap_err()
            .contains("ambiguous"));
    }

    #[test]
    fn particle_interpreter_does_not_invent_omitted_suffix_defaults() {
        let registration = br#"pdxparticle={name="defaults" type="defaults_file"}"#;
        let definition = br#"
          particle={name="defaults_file"
            subsystem={texture={file="gfx/particles/a.dds" x=1 y=1 shader="ParticleAlpha"}}
          }
        "#;
        let (binding, payloads) = fixture(&[
            ("gfx/entities/defaults.gfx", registration),
            ("gfx/particles/defaults.asset", definition),
            ("gfx/particles/a.dds", b"a texture"),
        ]);
        let interpreter = ProductionRuntimeContentInterpreter::build(&binding, &payloads).unwrap();
        let row = interpreter
            .resolve_particle("defaults", "defaults_file", 0)
            .unwrap();
        assert_eq!(row.authored_trail_token, None);
        assert_eq!(row.authored_billboard_token, None);
        assert_eq!(row.explicit_engine_effect_token_crosscheck, None);
    }

    #[test]
    fn entity_interpreter_follows_exact_name_asset_pdxmesh_and_mod_mesh_winner() {
        let asset = br#"
            entity = { name="mod_tank_entity" pdxmesh="mod_tank_mesh" }
            entity = { name="weather_only_entity" state={ name="idle" } }
        "#;
        let gfx = br#"
            objectTypes = {
              pdxmesh = { name="mod_tank_mesh" file="gfx/models/mod_tank.mesh" }
            }
        "#;
        let mesh = b"exact mod mesh bytes";
        let (binding, payloads) = fixture(&[
            ("gfx/entities/mod_tank.asset", asset),
            ("gfx/entities/mod_tank.gfx", gfx),
            ("gfx/models/mod_tank.mesh", mesh),
        ]);

        let interpreter = ProductionRuntimeContentInterpreter::build(&binding, &payloads).unwrap();
        let resolved = interpreter.resolve_entity("mod_tank_entity").unwrap();
        assert_eq!(
            resolved.resolved_entity_asset.logical_path,
            "gfx/entities/mod_tank.asset"
        );
        assert_eq!(resolved.mesh.logical_path, "gfx/models/mod_tank.mesh");
        assert_eq!(resolved.mesh.payload_sha256, sha(mesh));
        assert!(interpreter.resolve_entity("weather_only_entity").is_err());
    }

    #[test]
    fn entity_interpreter_rejects_cross_file_definition_ambiguity() {
        let first = br#"entity={name="duplicate" pdxmesh="mesh_a"}"#;
        let second = br#"entity={name="duplicate" pdxmesh="mesh_b"}"#;
        let meshes = br#"
          pdxmesh={name="mesh_a" file="gfx/models/a.mesh"}
          pdxmesh={name="mesh_b" file="gfx/models/b.mesh"}
        "#;
        let (binding, payloads) = fixture(&[
            ("gfx/entities/a.asset", first),
            ("gfx/entities/b.asset", second),
            ("gfx/entities/meshes.gfx", meshes),
            ("gfx/models/a.mesh", b"a"),
            ("gfx/models/b.mesh", b"b"),
        ]);
        let interpreter = ProductionRuntimeContentInterpreter::build(&binding, &payloads).unwrap();
        assert!(interpreter
            .resolve_entity("duplicate")
            .unwrap_err()
            .contains("ambiguous"));
    }
}
