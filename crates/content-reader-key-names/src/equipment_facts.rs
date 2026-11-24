use std::io;

use crate::{equipment::Database, unit_leader::needs::SubUnitNeeds};
use content_reader_layers::ResolvedLayers;
use content_reader_layers::content::facts::{self, Input, put_bytes, put_u32};

pub fn gather(layers: &ResolvedLayers) -> Result<Vec<u8>, String> {
    let database = Database::load_complete(layers)
        .map_err(|error| format!("the equipment database cannot be read: {error}"))?;
    let needs = SubUnitNeeds::read(layers, &database)
        .map_err(|error| format!("the sub-unit needs cannot be read: {error}"))?;
    encode_needs(&needs)
}

pub fn sub_unit_needs(pack: &facts::Pack) -> Result<SubUnitNeeds, String> {
    decode_needs(&pack.equipment_section()?)
        .map_err(|error| format!("the recording's sub-unit needs cannot be read: {error}"))
}

pub fn encode_needs(needs: &SubUnitNeeds) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut write = || -> io::Result<()> {
        put_u32(&mut out, facts::count(needs.needs.len())?);
        for (key, need) in &needs.needs {
            put_bytes(&mut out, key.as_bytes())?;
            put_u32(&mut out, facts::count(need.len())?);
            for (archetype, amount) in need {
                put_bytes(&mut out, archetype.as_bytes())?;
                out.extend_from_slice(&amount.to_le_bytes());
            }
        }
        put_u32(&mut out, facts::count(needs.dropped.len())?);
        for dropped in &needs.dropped {
            put_bytes(&mut out, dropped.as_bytes())?;
        }
        Ok(())
    };
    write().map_err(|error| format!("the sub-unit needs cannot be packed: {error}"))?;
    Ok(out)
}

pub fn decode_needs(bytes: &[u8]) -> io::Result<SubUnitNeeds> {
    let mut input = Input(bytes);
    let mut out = SubUnitNeeds::default();
    for _ in 0..counted(&mut input)? {
        let key = input.string()?;
        let mut need = Vec::new();
        for _ in 0..counted(&mut input)? {
            let archetype = input.string()?;
            need.push((
                archetype,
                i64::from_le_bytes(input.take(8)?.try_into().expect("eight bytes")),
            ));
        }
        out.needs.insert(key, need);
    }
    for _ in 0..counted(&mut input)? {
        out.dropped.push(input.string()?);
    }
    finished(&input)?;
    Ok(out)
}

fn counted(input: &mut Input<'_>) -> io::Result<usize> {
    let count = input.u32()? as usize;
    if count > input.0.len() / 4 + 1 {
        return Err(content_reader_parser::fields::invalid(
            "an equipment fact counts more entries than it holds",
        ));
    }
    Ok(count)
}

fn finished(input: &Input<'_>) -> io::Result<()> {
    if input.0.is_empty() {
        Ok(())
    } else {
        Err(content_reader_parser::fields::invalid(
            "an equipment fact runs past its entries",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_layers::content::facts::{
        EQUIPMENT_VERSION, MAGIC_FOR_FIXTURES as MAGIC, Pack, Source,
        UNEQUIPPED_FOR_FIXTURES as UNEQUIPPED, VERSION_FOR_FIXTURES as VERSION, write,
    };
    use replay_identity::Identity;
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

    const EQUIPMENT: &str = "equipments = { infantry_equipment = { is_archetype = yes } infantry_equipment_1 = { archetype = infantry_equipment } \
        support_equipment = { is_archetype = yes } support_equipment_1 = { archetype = support_equipment } }";
    const UNITS: &str = "sub_units = { infantry = { need = { infantry_equipment_1 = 100 support_equipment_1 = 0.5 ghost = 1 } } engineer = { need = { support_equipment_1 = 10 } } }";

    struct Fixture {
        root: PathBuf,
        cache: PathBuf,
        layers: ResolvedLayers,
        identity: Identity,
    }

    impl Fixture {
        fn new(name: &str, exe_hash: [u8; 32]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hoi4_facts_equipment_{name}_{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            for (path, text) in [
                ("content/common/units/equipment/infantry.txt", EQUIPMENT),
                ("content/common/units/infantry.txt", UNITS),
            ] {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            let layers = ResolvedLayers::from_folders(&[(&root.join("content"), &[][..])]).unwrap();
            let identity = Identity {
                game_version: "synthetic".into(),
                exe_hash,
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: vec![replay_identity::Mod {
                    id: "test0".into(),
                    display_name: "test".into(),
                    declared_version: String::new(),
                    hash: [3; 32],
                }],
                coverage_policy: replay_identity::COVERAGE_POLICY_VERSION,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [0; 32],
                verification: replay_identity::Verification::Unchecked,
            };
            Self {
                cache: root.join("cache"),
                root,
                layers,
                identity,
            }
        }

        fn write(&self, executable: Result<Vec<u8>, String>) -> Vec<u8> {
            let source = Source {
                layers: &self.layers,
                withheld: BTreeMap::new(),
            };
            write(
                &source,
                &self.identity,
                executable,
                &self.cache,
                &|| false,
                gather,
            )
            .unwrap()
            .0
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn opened(bytes: Vec<u8>) -> Pack {
        let length = bytes.len() as u64;
        Pack::open(Arc::new(bytes), 0, length).unwrap()
    }

    fn reworked(pack: &[u8], cut: usize, section: &[u8], version: u16) -> Vec<u8> {
        let compressed = u32::from_le_bytes(pack[6..10].try_into().unwrap()) as usize;
        let raw = u32::from_le_bytes(pack[10..14].try_into().unwrap()) as usize;
        let mut manifest = zstd::bulk::decompress(&pack[14..14 + compressed], raw).unwrap();
        manifest.truncate(manifest.len() - cut);
        manifest.extend_from_slice(section);
        let packed = zstd::bulk::compress(&manifest, 1).unwrap();
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        out.extend_from_slice(&pack[14 + compressed..]);
        out
    }

    #[test]
    fn a_pack_built_twice_is_the_same_bytes_and_one_without_equipment_facts_says_why() {
        let fixture = Fixture::new("withheld", [1; 32]);
        let first = fixture.write(Err("no executable here".to_owned()));
        assert_eq!(
            first,
            fixture.write(Err("no executable here".to_owned())),
            "a pack built twice from the same content"
        );
        let pack = opened(first.clone());
        let why = sub_unit_needs(&pack).unwrap_err();
        assert_eq!(
            why,
            "the equipment database needs the executable's tables: no executable here"
        );
        let cut = 1 + 4 + why.len();
        let older = opened(reworked(&first, cut, &[], UNEQUIPPED));
        assert!(
            sub_unit_needs(&older)
                .unwrap_err()
                .contains("packed before they carried equipment facts")
        );
        let mut other = vec![0];
        for word in [99_u32, 0] {
            other.extend_from_slice(&word.to_le_bytes());
        }
        let refused = sub_unit_needs(&opened(reworked(&first, cut, &other, VERSION))).unwrap_err();
        assert!(
            refused.contains("version 99")
                && refused.contains(&format!("version {}", EQUIPMENT_VERSION)),
            "{refused}"
        );
    }

    #[test]
    fn a_packs_equipment_facts_are_what_the_equipment_database_gives() {
        let tables = content_reader_script::test_game::enter();
        let executable = engine_executable::executable_tables::scope(
            Some(tables),
            engine_executable::executable_tables::recorded_facts,
        )
        .unwrap();
        let fixture = Fixture::new("database", tables.identity().sha256);
        let bytes = fixture.write(Ok(executable.clone()));
        assert_eq!(
            bytes,
            fixture.write(Ok(executable)),
            "a pack built twice from the same content"
        );
        let pack = opened(bytes);
        let needs = engine_executable::executable_tables::scope(Some(tables), || {
            let database = crate::equipment::Database::load_complete(&fixture.layers).unwrap();
            crate::unit_leader::needs::SubUnitNeeds::read(&fixture.layers, &database).unwrap()
        });
        assert_eq!(sub_unit_needs(&pack).unwrap(), needs);
        assert_eq!(
            needs.get("infantry").unwrap(),
            [
                ("infantry_equipment".to_owned(), 10_000_000),
                ("support_equipment".to_owned(), 50_000)
            ]
        );
        assert_eq!(
            needs.dropped,
            ["sub-unit infantry needs ghost, which is no equipment"]
        );
    }

    fn needs() -> SubUnitNeeds {
        SubUnitNeeds {
            needs: [
                (
                    "infantry".to_owned(),
                    vec![
                        ("infantry_equipment".to_owned(), 10_000_000),
                        ("support_equipment".to_owned(), 50_000),
                    ],
                ),
                ("engineer".to_owned(), Vec::new()),
            ]
            .into(),
            dropped: vec!["sub-unit A needs ghost, which is no equipment".to_owned()],
        }
    }

    #[test]
    fn sub_unit_needs_decode_to_what_was_encoded_and_encode_the_same_bytes() {
        let needs = needs();
        let bytes = encode_needs(&needs).unwrap();
        assert_eq!(bytes, encode_needs(&needs).unwrap());
        assert_eq!(decode_needs(&bytes).unwrap(), needs);
        assert!(decode_needs(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes;
        longer.push(0);
        assert!(decode_needs(&longer).is_err());
    }
}
