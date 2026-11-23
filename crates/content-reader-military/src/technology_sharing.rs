use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{fields::invalid, parser::Item};
use content_reader_script::{key_names::KeyNames, native::{self, Shape}};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub token: String,
    pub name: String,
}

#[derive(Default)]
pub struct TechnologySharing {
    pub groups: BTreeMap<String, Group>,
    pub refusals: Vec<String>,
    pub warnings: Vec<String>,
}

impl TechnologySharing {
    pub fn group(&self, token: &str) -> Result<Option<&Group>, String> {
        if self.refusals.is_empty() {
            Ok(self.groups.get(&token.to_ascii_lowercase()))
        } else {
            Err(format!(
                "technology-sharing group {token}: {}",
                self.refusals.join("; ")
            ))
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Group,
}

fn string(item: &Item) -> io::Result<String> {
    item.value.read_str().map(str::to_owned).map_err(|why| {
        invalid(format!(
            "technology-sharing {}: {why} (count: 1)",
            String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
        ))
    })
}

pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<(Vec<Group>, Vec<String>)> {
    let mut available = false;
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File if field.keyword_is("technology_sharing_group") => {
                available = false;
                Shape::Object(Node::Group)
            }
            Node::File => Shape::Skip,
            Node::Group => match field.keyword().as_deref() {
                Some(
                    b"id"
                    | b"name"
                    | b"desc"
                    | b"picture"
                    | b"research_sharing_per_country_bonus"
                    | b"is_faction_sharing",
                ) => Shape::One,
                Some(b"categories") => Shape::RawList,
                Some(b"available") if available => Shape::Skip,
                Some(b"available") => {
                    available = true;
                    Shape::Trigger
                }
                _ => Shape::Skip,
            },
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!(
            "technology-sharing definitions: {cause} (count: 1)"
        )));
    }
    let mut result = Vec::new();
    for item in read.items {
        if !item.keyword_is("technology_sharing_group") {
            continue;
        }
        let mut group = Group::default();
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("id") {
                group.token = string(field)?;
            } else if field.keyword_is("name") {
                group.name = string(field)?;
            }
        }
        if group.token.is_empty() {
            return Err(invalid(
                "technology-sharing group has no representable id (count: 1)",
            ));
        }
        result.push(group);
    }
    Ok((
        result,
        read.misreads
            .into_iter()
            .map(|why| why.to_string())
            .collect(),
    ))
}

pub fn read(layers: &ResolvedLayers) -> io::Result<TechnologySharing> {
    let names = layers.key_names()?;
    let mut result = TechnologySharing::default();
    for path in script_files::paths(layers, "common/technology_sharing/", Some(".txt"))? {
        match parse(&path.read(layers)?, &names) {
            Ok((groups, warnings)) => {
                result
                    .warnings
                    .extend(warnings.into_iter().map(|why| format!("{path}: {why}")));
                for group in groups {
                    result
                        .groups
                        .insert(group.token.to_ascii_lowercase(), group);
                }
            }
            Err(why) => result.refusals.push(format!("{path}: {why}")),
        }
    }
    result.warnings.extend(result.refusals.iter().cloned());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn technology_sharing_names_raw_categories_and_repeated_available() {
        content_reader_script::test_game::enter();
        let (groups, warnings) = parse(
            br#"
            @name=MOD_NAME
            technology_sharing_group={id=custom name=@name categories={alpha beta}
                available={always=yes}
                available={@leaked=BAD}
            }
            technology_sharing_group={id=second name=@leaked}
        "#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            groups,
            [
                Group {
                    token: "custom".into(),
                    name: "MOD_NAME".into()
                },
                Group {
                    token: "second".into(),
                    name: "@leaked".into()
                }
            ]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let (groups, _) = parse(
            b"technology_sharing_group={id=empty} } technology_sharing_group={id=ignored}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            groups,
            [Group {
                token: "empty".into(),
                name: String::new()
            }]
        );
        let why = parse(
            b"technology_sharing_group={name=missing}",
            &KeyNames::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(why.contains("no representable id") && why.contains("count: 1"));
    }

    #[test]
    fn technology_sharing_layers_replace_complete_template_and_refuse_unknown_suffix() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_technology_sharing_{}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let base = root.join("base");
        let over = root.join("over");
        for folder in [&base, &over] {
            std::fs::create_dir_all(folder.join("common/technology_sharing")).unwrap();
        }
        std::fs::write(base.join("common/technology_sharing/a.txt"), b"technology_sharing_group={id=custom name=BASE} technology_sharing_group={id=removed name=OLD}").unwrap();
        std::fs::write(over.join("common/technology_sharing/b.txt"), b"technology_sharing_group={id=CUSTOM name=MOD} technology_sharing_group={id=empty name=OLD} technology_sharing_group={id=empty}").unwrap();
        let stack =
            ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&over, &[])]).unwrap();
        let definitions = read(&stack).unwrap();
        assert_eq!(definitions.group("CUSTOM").unwrap().unwrap().name, "MOD");
        assert_eq!(definitions.group("empty").unwrap().unwrap().name, "");
        let replaced = ResolvedLayers::from_test_common_folders(&[
            (&base, &[]),
            (&over, &["common/technology_sharing"]),
        ])
        .unwrap();
        assert!(read(&replaced).unwrap().group("removed").unwrap().is_none());
        std::fs::write(
            over.join("common/technology_sharing/c.txt"),
            b"technology_sharing_group={name=missing}",
        )
        .unwrap();
        let refused =
            ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&over, &[])]).unwrap();
        let why = read(&refused).unwrap().group("custom").unwrap_err();
        assert!(why.contains("c.txt") && why.contains("count: 1"));
        let cleanup_root = std::fs::canonicalize(root).unwrap();
        let cleanup_parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(cleanup_root.parent(), Some(cleanup_parent.as_path()));
        std::fs::remove_dir_all(cleanup_root).unwrap();
    }
}
