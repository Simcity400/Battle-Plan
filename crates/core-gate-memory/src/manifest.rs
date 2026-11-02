use replay::{columns, container::TableFields};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub families_source: String,
    pub families: Vec<Family>,
    #[serde(default)]
    pub shared: Vec<Family>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Family {
    pub name: String,
    pub origin: String,
    pub ordered: bool,
    #[serde(default)]
    pub declaration: Option<String>,
    pub tables: Vec<NamedTable>,
    #[serde(default)]
    pub reverse: Vec<Reverse>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamedTable {
    pub name: String,
    pub fields: Fields,
    pub subscription: String,
    pub source: String,
    #[serde(default)]
    pub previous: bool,
    #[serde(default)]
    pub read_rule: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Fields {
    All(String),
    Named(Vec<String>),
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reverse {
    pub chain: Vec<String>,
}

#[derive(Clone)]
pub struct Selection {
    pub table: TableId,
    pub fields: Vec<usize>,
    pub undeclared: bool,
    pub subscription: String,
    pub previous: bool,
    pub read_rule: Option<String>,
}

#[derive(Clone)]
pub struct Relation {
    pub name: String,
    pub steps: Vec<(TableId, Operand)>,
}

#[derive(Clone)]
pub enum Operand {
    Field(usize),
    Key(u32),
}

pub struct Resolved {
    pub manifest: Manifest,
    pub families: Vec<Vec<Selection>>,
    pub shared: Vec<Vec<Selection>>,
    pub shared_union: Vec<Selection>,
    pub tables: Vec<TableId>,
    pub union: Vec<Selection>,
    pub declared: Vec<Selection>,
    pub relations: Vec<Relation>,
    pub family_relations: Vec<Vec<usize>>,
}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn table(name: &str) -> io::Result<TableId> {
    TableId::ALL
        .into_iter()
        .find(|t| format!("{t:?}") == name)
        .ok_or_else(|| invalid(format!("No replay-data TableId named {name}")))
}

impl Manifest {
    pub fn read(path: &Path) -> io::Result<Resolved> {
        let manifest: Self = toml::from_str(&std::fs::read_to_string(path)?)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        manifest.resolve()
    }

    pub fn resolve(self) -> io::Result<Resolved> {
        if self.version != 1 {
            return Err(invalid("Unsupported gate manifest version"));
        }
        for owner in &self.shared {
            if owner.ordered || !owner.reverse.is_empty() || owner.tables.iter().any(|t| t.previous)
            {
                return Err(invalid(
                    "Shared upper bounds cannot own ordered, reverse or previous state",
                ));
            }
        }
        let mut names = BTreeSet::new();
        let mut families = Vec::new();
        let mut relations = Vec::<Relation>::new();
        let mut family_relations = Vec::new();
        for family in self.families.iter().chain(&self.shared) {
            if !names.insert(&family.name) || family.tables.is_empty() {
                return Err(invalid(format!(
                    "Duplicate or empty family {}",
                    family.name
                )));
            }
            if !matches!(family.origin.as_str(), "note" | "code" | "undeclared") {
                return Err(invalid(format!("{} has an invalid origin", family.name)));
            }
            let mut selected = Vec::new();
            let mut seen = BTreeSet::new();
            for named in &family.tables {
                let id = table(&named.name)?;
                if named
                    .read_rule
                    .as_deref()
                    .is_some_and(|r| !matches!(r, "supplied" | "acquired"))
                {
                    return Err(invalid(format!(
                        "{}: invalid read rule for {}",
                        family.name, named.name
                    )));
                }
                if !seen.insert(id) {
                    return Err(invalid(format!("{} repeats {}", family.name, named.name)));
                }
                for kind in named.subscription.split(',') {
                    if !matches!(
                        kind,
                        "stream" | "watch" | "wake-on-read" | "lookup" | "reverse" | "ordered"
                    ) {
                        return Err(invalid(format!(
                            "{}: unknown subscription {kind}",
                            family.name
                        )));
                    }
                }
                let fields = match &named.fields {
                    Fields::All(all) if all == "all" => {
                        let _ = TableFields::all(id);
                        (0..id.schema().len()).collect()
                    }
                    Fields::Named(names) => {
                        let names: Vec<_> = names.iter().map(String::as_str).collect();
                        TableFields::named(id, &names)?;
                        let mut fields: Vec<_> = names
                            .iter()
                            .map(|name| columns::field(id, name))
                            .collect::<io::Result<_>>()?;
                        fields.sort_unstable();
                        if fields.windows(2).any(|pair| pair[0] == pair[1]) {
                            return Err(invalid(format!(
                                "{} repeats a field of {id:?}",
                                family.name
                            )));
                        }
                        fields
                    }
                    _ => {
                        return Err(invalid(format!(
                            "{}: fields must be all or a list",
                            family.name
                        )));
                    }
                };
                if named.source == "undeclared"
                    && !matches!(&named.fields, Fields::All(s) if s == "all")
                {
                    return Err(invalid(format!(
                        "{}: undeclared {} must be whole",
                        family.name, named.name
                    )));
                }
                selected.push(Selection {
                    table: id,
                    fields,
                    undeclared: named.source == "undeclared",
                    subscription: named.subscription.clone(),
                    previous: named.previous,
                    read_rule: named.read_rule.clone(),
                });
            }
            selected.sort_by_key(|s| s.table);
            let mut indexes = Vec::new();
            for reverse in &family.reverse {
                if reverse.chain.is_empty() {
                    return Err(invalid("An empty reverse chain"));
                }
                let name = reverse.chain.join(" -> ");
                let at = if let Some(at) = relations.iter().position(|r| r.name == name) {
                    at
                } else {
                    let mut steps = Vec::new();
                    for step in &reverse.chain {
                        let (name, member) = step
                            .split_once('.')
                            .ok_or_else(|| invalid(format!("Invalid reverse step {step}")))?;
                        let id = table(name)?;
                        let operand = if member == "$key" {
                            Operand::Key(0)
                        } else if let Some(shift) = member.strip_prefix("$key:") {
                            let shift: u32 = shift
                                .parse()
                                .map_err(|_| invalid(format!("Invalid key shift {step}")))?;
                            if shift >= id.key_bits() {
                                return Err(invalid(format!("Key shift past {name}'s key")));
                            }
                            Operand::Key(shift)
                        } else {
                            let field = columns::field(id, member)?;
                            if !selected
                                .iter()
                                .any(|s| s.table == id && s.fields.contains(&field))
                            {
                                return Err(invalid(format!(
                                    "{} does not subscribe to reverse input {step}",
                                    family.name
                                )));
                            }
                            Operand::Field(field)
                        };
                        steps.push((id, operand));
                    }
                    relations.push(Relation { name, steps });
                    relations.len() - 1
                };
                indexes.push(at);
            }
            families.push(selected);
            family_relations.push(indexes);
        }
        let shared = families.split_off(self.families.len());
        family_relations.truncate(self.families.len());
        let shared_union = union(shared.iter().flatten());
        let combined = union(families.iter().chain(&shared).flatten());
        let declared = union(
            families
                .iter()
                .chain(&shared)
                .flatten()
                .filter(|s| !s.undeclared),
        );
        let tables = combined.iter().map(|s| s.table).collect();
        Ok(Resolved {
            manifest: self,
            families,
            shared,
            shared_union,
            tables,
            union: combined,
            declared,
            relations,
            family_relations,
        })
    }
}

pub fn union<'a>(selections: impl Iterator<Item = &'a Selection>) -> Vec<Selection> {
    let mut tables = BTreeMap::<TableId, BTreeSet<usize>>::new();
    for selection in selections {
        tables
            .entry(selection.table)
            .or_default()
            .extend(&selection.fields);
    }
    tables
        .into_iter()
        .map(|(table, fields)| Selection {
            table,
            fields: fields.into_iter().collect(),
            undeclared: false,
            subscription: "stream".into(),
            previous: false,
            read_rule: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_and_tables_are_errors() {
        assert!(table("SyntheticMissingTable").is_err());
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='note'\nordered=false\n[[families.tables]]\nname='Countries'\nfields=['synthetic_missing_field']\nsubscription='stream'\nsource='synthetic'";
        assert!(toml::from_str::<Manifest>(text).unwrap().resolve().is_err());
    }

    #[test]
    fn shared_context_does_not_expand_family_or_previous_subscriptions() {
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='note'\nordered=true\n[[families.tables]]\nname='Countries'\nfields=['tag']\nsubscription='stream'\nsource='synthetic'\n[[shared]]\nname='trigger-context'\norigin='code'\nordered=false\n[[shared.tables]]\nname='Countries'\nfields='all'\nsubscription='lookup'\nsource='synthetic'";
        let resolved = toml::from_str::<Manifest>(text).unwrap().resolve().unwrap();
        assert_eq!(resolved.families.len(), 1);
        assert_eq!(resolved.families[0][0].fields.len(), 1);
        assert!(!resolved.families[0][0].previous);
        assert_eq!(
            resolved.shared_union[0].fields.len(),
            TableId::Countries.schema().len()
        );
        assert_eq!(resolved.union[0].fields, resolved.shared_union[0].fields);
    }

    #[test]
    fn union_deduplicates_overlapping_fields() {
        let one = Selection {
            table: TableId::Countries,
            fields: vec![0, 1],
            undeclared: false,
            subscription: "stream".into(),
            previous: false,
            read_rule: None,
        };
        let two = Selection {
            fields: vec![1, 3],
            ..one.clone()
        };
        assert_eq!(union([&one, &two].into_iter())[0].fields, [0, 1, 3]);
    }
}
