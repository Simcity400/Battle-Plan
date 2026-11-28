
use std::collections::{BTreeMap, BTreeSet};

use content_reader_key_names::equipment;
use content_reader_mesh::mesh;

use super::Content;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Entity,
    Mesh,
    Unregistered,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reference {
    Pool {
        bucket: String,
        equipment: String,
        file: String,
    },
    Ladder { key: String, prefix: Option<String> },
    Design { equipment: String },
    CloneOf { of: String },
    ClonedBy { by: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefusalKind {
    Unregistered,
    Declaration,
    NoGeometry,
}

impl RefusalKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unregistered => "not registered by any descriptor",
            Self::Declaration => "a declaration it uses cannot be played",
            Self::NoGeometry => "no geometry to draw",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub reason: String,
}

pub fn refusal(roster: &mesh::Roster, name: &str) -> Option<Refusal> {
    refusal_in(&roster.descriptor_job(), name)
}

pub fn refusal_in(job: &mesh::DescriptorJob<'_>, name: &str) -> Option<Refusal> {
    if let Some(reason) = job.refused_declaration(name) {
        return Some(Refusal {
            kind: RefusalKind::Declaration,
            reason,
        });
    }
    let descriptors = job.view();
    let no_geometry = |reason: String| {
        Some(Refusal {
            kind: RefusalKind::NoGeometry,
            reason,
        })
    };
    match descriptors.inherit(name) {
        Some(_) if draws(descriptors, name, &mut BTreeSet::new()) => None,
        Some(entity) => {
            let root = match (entity.mesh, entity.unbound_mesh) {
                (Some(mesh), _) | (None, Some(mesh)) => {
                    format!("no pdxmesh {mesh} is registered")
                }
                (None, None) => "it names no pdxmesh".to_owned(),
            };
            no_geometry(if entity.attachments.is_empty() {
                root
            } else {
                format!("{root}, and nothing attached to it draws one")
            })
        }
        None if descriptors.meshes.contains_key(name) => None,
        None => Some(Refusal {
            kind: RefusalKind::Unregistered,
            reason: "no descriptor registers it".into(),
        }),
    }
}

fn draws<'a>(descriptors: mesh::DescriptorView<'a>, name: &'a str, seen: &mut BTreeSet<&'a str>) -> bool {
    if !seen.insert(name) {
        return false;
    }
    let Some(entity) = descriptors.inherit(name) else {
        return false;
    };
    entity
        .mesh
        .is_some_and(|mesh| descriptors.meshes.contains_key(mesh))
        || entity
            .attachments
            .iter()
            .any(|attachment| draws(descriptors, &attachment.entity, seen))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub references: BTreeSet<Reference>,
    pub refusal: Option<Refusal>,
}

impl Entry {
    pub fn aircraft(&self) -> bool {
        !self.references.is_empty()
    }
}

pub struct Inputs<'a> {
    pub roster: &'a mesh::Roster,
    pub equipment: &'a equipment::Database,
    pub pools: &'a Content,
    pub designs: &'a [(String, String)],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    pub entries: Vec<Entry>,
    pub diagnostics: Vec<String>,
}

impl Catalogue {
    pub fn build(inputs: &Inputs<'_>) -> Self {
        let job = inputs.roster.descriptor_job();
        Self::build_entries(inputs, true, |name| refusal_in(&job, name))
    }

    pub fn build_for_picker(inputs: &Inputs<'_>, job: &mesh::DescriptorJob<'_>) -> Self {
        Self::build_entries(inputs, false, |name| refusal_in(job, name))
    }

    fn build_entries(
        inputs: &Inputs<'_>,
        include_non_aircraft: bool,
        mut refuse: impl FnMut(&str) -> Option<Refusal>,
    ) -> Self {
        let Inputs {
            roster,
            equipment,
            pools,
            designs,
        } = *inputs;
        let mut diagnostics = Vec::new();
        let keys = PlaneKeys::read(equipment, &mut diagnostics);
        let mut references: BTreeMap<String, BTreeSet<Reference>> = BTreeMap::new();
        let refer = |references: &mut BTreeMap<String, BTreeSet<Reference>>,
                     name: &str,
                     reference: Reference| {
            references
                .entry(name.to_owned())
                .or_default()
                .insert(reference);
        };
        for model in pools.models() {
            if keys.pools.contains(model.equipment) {
                refer(
                    &mut references,
                    model.model,
                    Reference::Pool {
                        bucket: model.bucket.to_string(),
                        equipment: model.equipment.to_owned(),
                        file: model.file.to_owned(),
                    },
                );
            }
        }
        for name in roster
            .entity_names()
            .chain(roster.unreadable_names.keys().map(String::as_str))
        {
            for (key, prefix) in ladder_keys(name, &keys.ladder) {
                refer(
                    &mut references,
                    name,
                    Reference::Ladder {
                        key: key.to_owned(),
                        prefix: prefix.map(str::to_owned),
                    },
                );
            }
        }
        let mut unknown_designs = BTreeSet::new();
        for (kind, model) in designs {
            if model.is_empty() {
                continue;
            }
            match equipment.named(kind) {
                Some(found) if is_plane(equipment.category(found)) => refer(
                    &mut references,
                    model,
                    Reference::Design {
                        equipment: kind.clone(),
                    },
                ),
                Some(_) => {}
                None => {
                    unknown_designs.insert(kind.as_str());
                }
            }
        }
        if !unknown_designs.is_empty() {
            diagnostics.push(format!(
                "{} recorded design types are not in the content, so whether their saved models are aircraft is unknown: {}",
                unknown_designs.len(),
                unknown_designs.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        let referenced: Vec<String> = references
            .keys()
            .filter(|name| roster.has_entity(name))
            .cloned()
            .collect();
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (child, parent) in &roster.clones {
            children
                .entry(parent.as_str())
                .or_default()
                .push(child.as_str());
        }
        for aircraft in &referenced {
            let mut seen = BTreeSet::from([aircraft.as_str()]);
            let mut parent = roster.clones.get(aircraft.as_str());
            while let Some(name) = parent {
                if !seen.insert(name.as_str()) {
                    break;
                }
                refer(
                    &mut references,
                    name,
                    Reference::ClonedBy {
                        by: aircraft.clone(),
                    },
                );
                parent = roster.clones.get(name.as_str());
            }
            let mut pending = vec![aircraft.as_str()];
            while let Some(name) = pending.pop() {
                for &child in children.get(name).into_iter().flatten() {
                    if seen.insert(child) {
                        refer(
                            &mut references,
                            child,
                            Reference::CloneOf {
                                of: aircraft.clone(),
                            },
                        );
                        pending.push(child);
                    }
                }
            }
        }
        let mut names: BTreeMap<&str, Kind> = roster
            .mesh_definitions()
            .keys()
            .map(|name| (name.as_str(), Kind::Mesh))
            .collect();
        names.extend(
            roster.entity_names().map(|name| (name, Kind::Entity)),
        );
        for name in references.keys() {
            names.entry(name).or_insert(Kind::Unregistered);
        }
        let entries = names
            .into_iter()
            .filter(|&(name, _)| include_non_aircraft || references.contains_key(name))
            .map(|(name, kind)| Entry {
                name: name.to_owned(),
                kind,
                references: references.get(name).cloned().unwrap_or_default(),
                refusal: refuse(name),
            })
            .collect();
        if !roster.unreadable.is_empty() {
            diagnostics.push(format!(
                "{} descriptor files do not parse even when closed; a name they may declare is refused where a reference makes it and not otherwise listed: {}",
                roster.unreadable.len(),
                roster.unreadable.join(", ")
            ));
        }
        if !roster.logged.is_empty() {
            diagnostics.push(format!(
                "{} entity registration diagnostics: {}",
                roster.logged.len(),
                roster.logged.join("; ")
            ));
        }
        Self {
            entries,
            diagnostics,
        }
    }

    pub fn refusal_counts<'a>(
        entries: impl IntoIterator<Item = &'a Entry>,
    ) -> BTreeMap<RefusalKind, usize> {
        let mut counts = BTreeMap::new();
        for entry in entries {
            if let Some(refusal) = &entry.refusal {
                *counts.entry(refusal.kind).or_default() += 1;
            }
        }
        counts
    }
}

fn is_plane(category: u64) -> bool {
    category & engine_executable::native_values::equipment_type::PLANE_MASK != 0
}

struct PlaneKeys {
    ladder: BTreeSet<String>,
    pools: BTreeSet<String>,
}

impl PlaneKeys {
    fn read(database: &equipment::Database, diagnostics: &mut Vec<String>) -> Self {
        let mut keys = Self {
            ladder: BTreeSet::new(),
            pools: BTreeSet::new(),
        };
        let mut untext = BTreeSet::new();
        let mut text = |bytes: &[u8]| match content_reader_parser::parser::text(bytes) {
            Ok(text) => Some(text.to_owned()),
            Err(_) => {
                untext.insert(String::from_utf8_lossy(bytes).into_owned());
                None
            }
        };
        for kind in &database.equipment {
            if !is_plane(database.category(kind)) {
                continue;
            }
            let archetype = kind.archetype.map(|at| &database.equipment[at]);
            let family = &database.equipment[kind.family];
            let family_archetype = family.archetype.map(|at| &database.equipment[at]);
            let named = [Some(kind), archetype, Some(family), family_archetype];
            for name in named
                .into_iter()
                .flatten()
                .filter_map(|kind| text(&kind.name))
            {
                keys.pools.insert(name.to_ascii_lowercase());
            }
            let ladder = [Some(kind), archetype, Some(family)]
                .into_iter()
                .flatten()
                .map(|kind| kind.name.as_slice())
                .chain([kind.sprite.as_slice()]);
            for key in ladder.filter(|key| !key.is_empty()).filter_map(&mut text) {
                keys.ladder.insert(key);
            }
        }
        if !untext.is_empty() {
            diagnostics.push(format!(
                "{} plane equipment keys are not text, so no entity name is made from them: {}",
                untext.len(),
                untext.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        keys
    }
}

fn ladder_keys<'a>(name: &'a str, keys: &BTreeSet<String>) -> Vec<(&'a str, Option<&'a str>)> {
    let Some(stem) = name.strip_suffix("_entity") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    if keys.contains(stem) {
        found.push((stem, None));
    }
    for (at, _) in stem.match_indices('_').filter(|(at, _)| *at > 0) {
        let key = &stem[at + 1..];
        if keys.contains(key) {
            found.push((key, Some(&stem[..at])));
        }
    }
    found
}

#[cfg(test)]
mod tests;
