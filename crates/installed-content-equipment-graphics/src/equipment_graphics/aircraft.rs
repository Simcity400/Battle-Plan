//! Every aircraft model a content stack offers to fly, and every other entity
//! and mesh it registers.
//!
//! No descriptor says an entity is an aircraft, and a name proves nothing:
//! references do. A name is an aircraft's when
//! - a graphics pool lists it as a model under the key of a plane equipment
//!   type, that type's archetype or family, or the family's archetype, in any
//!   bucket, so other countries' pools count too;
//! - the airborne model ladder makes it for a plane type's own, archetype,
//!   family or sprite key: `<key>_entity`, or `<prefix>_<key>_entity` for a
//!   cosmetic or country tag;
//! - a recorded design of a plane type saved it as its model; or
//! - it is a registered clone of such an entity, or such an entity is a clone
//!   of it.
//!
//! A plane type is one whose effective categories the engine's plane
//! classifier admits ([`is_plane`]). Every registered entity and
//! bare `pdxmesh` is listed as well, for planes nothing references, and a name
//! a reference makes that no descriptor registers is listed, refused. Names
//! come from the registered roster ([`mesh::Roster`]), so layer precedence,
//! `replace_path`, clones and registration diagnostics are the stack's own.
//! [`refusal`] says why a name cannot be flown from that roster alone; the
//! files a name loads are read only when it is chosen.

use std::collections::{BTreeMap, BTreeSet};

use installed_content_core::{equipment, mesh};

use super::Content;

/// What a catalogue name is in the registered roster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A registered entity.
    Entity,
    /// A registered `pdxmesh` that no entity of its name shadows. It resolves
    /// with its own scale and no states, animation or attachments.
    Mesh,
    /// A name a reference makes that no descriptor registers.
    Unregistered,
}

/// Why a name is an aircraft's.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reference {
    /// A graphics pool lists it under a plane key (folded, as pool lookups
    /// compare keys).
    Pool {
        bucket: String,
        equipment: String,
        file: String,
    },
    /// The airborne model ladder's name for a plane key, with the prefix
    /// before it; `None` is the ladder's bare last level.
    Ladder { key: String, prefix: Option<String> },
    /// A recorded design of this plane type saved it as its model.
    Design { equipment: String },
    /// It is a registered clone of the aircraft entity `of`.
    CloneOf { of: String },
    /// The aircraft entity `by` is a registered clone of it.
    ClonedBy { by: String },
}

/// What kind of reason refuses a name, for counting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefusalKind {
    /// No descriptor registers it.
    Unregistered,
    /// A declaration it uses cannot be played as registered
    /// ([`mesh::Roster::refused_declaration`]).
    Declaration,
    /// It binds no registered mesh, so nothing would be drawn.
    NoGeometry,
}

impl RefusalKind {
    /// The kind as a counted reason names it.
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

/// Why `name` cannot be flown, read from the registered roster alone; `None`
/// when what it declares can be baked, though its files load only when it is
/// chosen and can still fail then.
pub fn refusal(roster: &mesh::Roster, name: &str) -> Option<Refusal> {
    if let Some(reason) = roster.refused_declaration(name) {
        return Some(Refusal {
            kind: RefusalKind::Declaration,
            reason,
        });
    }
    let descriptors = &roster.descriptors;
    let no_geometry = |reason: String| {
        Some(Refusal {
            kind: RefusalKind::NoGeometry,
            reason,
        })
    };
    match descriptors.inherit(name) {
        // An entity whose own binding draws nothing still draws what is attached to it, as the
        // model composer places attachments on a meshless root by its locators.
        Some(_) if draws(roster, name, &mut BTreeSet::new()) => None,
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

/// Whether the registered entity `name` draws anything: its own registered mesh, or that of an
/// entity attached to it at any depth. The composer skips an attachment child that no entity
/// registers, a bare mesh included, so such a child draws nothing here either, though a bare mesh
/// can be chosen itself. A cycle draws nothing more.
fn draws<'a>(roster: &'a mesh::Roster, name: &'a str, seen: &mut BTreeSet<&'a str>) -> bool {
    if !seen.insert(name) {
        return false;
    }
    let descriptors = &roster.descriptors;
    let Some(entity) = descriptors.inherit(name) else {
        return false;
    };
    entity
        .mesh
        .is_some_and(|mesh| descriptors.meshes.contains_key(mesh))
        || entity
            .attachments
            .iter()
            .any(|attachment| draws(roster, &attachment.entity, seen))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    /// Why it is an aircraft's, sorted; empty for a name only the complete
    /// listing offers.
    pub references: BTreeSet<Reference>,
    pub refusal: Option<Refusal>,
}

impl Entry {
    pub fn aircraft(&self) -> bool {
        !self.references.is_empty()
    }
}

/// The catalogue's inputs.
pub struct Inputs<'a> {
    pub roster: &'a mesh::Roster,
    pub equipment: &'a equipment::Database,
    pub pools: &'a Content,
    /// Recorded designs: each design's equipment type as recorded, and the
    /// model it saved. An empty model saved none.
    pub designs: &'a [(String, String)],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    /// Every name, by name bytes.
    pub entries: Vec<Entry>,
    /// What the inputs could not settle, named and counted.
    pub diagnostics: Vec<String>,
}

impl Catalogue {
    pub fn build(inputs: &Inputs<'_>) -> Self {
        let Inputs {
            roster,
            equipment,
            pools,
            designs,
        } = *inputs;
        let descriptors = &roster.descriptors;
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
        // The ladder looks names up among registered entities, which include
        // those a descriptor that does not parse may declare.
        for name in descriptors
            .entities
            .keys()
            .chain(roster.unreadable_names.keys())
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
        // Clones relate only to what the references above name, so an
        // aircraft's siblings through a shared parent are not drawn in.
        let referenced: Vec<String> = references
            .keys()
            .filter(|name| descriptors.entities.contains_key(*name))
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
            // Registration can leave a clone cycle (a parent later redeclared
            // as a clone of its own clone), so each walk stops at a repeat.
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
        let mut names: BTreeMap<&str, Kind> = descriptors
            .meshes
            .keys()
            .map(|name| (name.as_str(), Kind::Mesh))
            .collect();
        names.extend(
            descriptors
                .entities
                .keys()
                .map(|name| (name.as_str(), Kind::Entity)),
        );
        for name in references.keys() {
            names.entry(name).or_insert(Kind::Unregistered);
        }
        let entries = names
            .into_iter()
            .map(|(name, kind)| Entry {
                name: name.to_owned(),
                kind,
                references: references.get(name).cloned().unwrap_or_default(),
                refusal: refusal(roster, name),
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

    /// Refused entries by kind of reason, over `entries`.
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

/// Whether effective category flags ([`equipment::Database::category`]) are
/// the engine's plane family.
fn is_plane(category: u64) -> bool {
    category & engine::cequipment_type::PLANE_MASK != 0
}

/// The keys of every plane equipment type.
struct PlaneKeys {
    /// Exact keys the airborne ladder joins into entity names.
    ladder: BTreeSet<String>,
    /// Folded keys pool lookups visit.
    pools: BTreeSet<String>,
}

impl PlaneKeys {
    fn read(database: &equipment::Database, diagnostics: &mut Vec<String>) -> Self {
        let mut keys = Self {
            ladder: BTreeSet::new(),
            pools: BTreeSet::new(),
        };
        let mut untext = BTreeSet::new();
        let mut text = |bytes: &[u8]| match installed_content_core::parser::text(bytes) {
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
            for name in named.into_iter().flatten().filter_map(|kind| text(&kind.name)) {
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

/// Each `(key, prefix)` an entity name is the ladder's name for:
/// `<key>_entity` with no prefix, or `<prefix>_<key>_entity` with a nonempty
/// prefix.
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
