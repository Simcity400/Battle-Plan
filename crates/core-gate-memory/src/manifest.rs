use replay::{columns, campaign::TableFields};
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
    #[serde(default)]
    pub readset: Vec<crate::readset::Table>,
    #[serde(default)]
    pub references: Vec<crate::readset::Reference>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamedTable {
    pub name: String,
    pub fields: Fields,
    pub subscription: String,
    pub source: String,
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
    #[serde(default)]
    pub fallback: Vec<String>,
    #[serde(default)]
    pub recorded_only: Vec<String>,
    #[serde(default)]
    pub recorded_sources: BTreeMap<String, String>,
    #[serde(default)]
    pub query_sources: BTreeMap<String, String>,
    #[serde(default)]
    pub all_reason: Option<String>,
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
    pub recorded_only: BTreeSet<usize>,
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
    pub shared_without_trigger: Vec<Selection>,
    pub shared_relations: Vec<usize>,
    pub tables: Vec<TableId>,
    pub union: Vec<Selection>,
    pub declared: Vec<Selection>,
    pub primary: Vec<Vec<Selection>>,
    pub private: Vec<Vec<Selection>>,
    pub relations: Vec<Relation>,
    pub family_relations: Vec<Vec<usize>>,
    pub readsets: Vec<Option<crate::readset::ReadSet>>,
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

    pub fn resolve(mut self) -> io::Result<Resolved> {
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
        let mut primary = Vec::new();
        let mut family_union: Option<Vec<Selection>> = None;
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
            let mut primary_selected = Vec::new();
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
                let mut fields = match &named.fields {
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
                    Fields::All(all)
                        if all == "family-union" && families.len() >= self.families.len() =>
                    {
                        family_union
                            .get_or_insert_with(|| {
                                union(families[..self.families.len()].iter().flatten())
                            })
                            .iter()
                            .find(|s| s.table == id)
                            .ok_or_else(|| {
                                invalid(format!("Shared {id:?} has no family declaration"))
                            })?
                            .fields
                            .clone()
                    }
                    _ => {
                        return Err(invalid(format!(
                            "{}: fields must be all or a list",
                            family.name
                        )));
                    }
                };
                let main_fields = fields.clone();
                let inherited_recorded = if matches!(&named.fields, Fields::All(all) if all == "family-union")
                {
                    family_union
                        .as_ref()
                        .unwrap()
                        .iter()
                        .find(|s| s.table == id)
                        .unwrap()
                        .recorded_only
                        .clone()
                } else {
                    BTreeSet::new()
                };
                for name in &named.fallback {
                    let field = columns::field(id, name)?;
                    if fields.contains(&field) {
                        return Err(invalid(format!(
                            "{} repeats fallback {id:?}.{name}",
                            family.name
                        )));
                    }
                    fields.push(field);
                }
                fields.sort_unstable();
                let mut recorded_only = inherited_recorded;
                for name in &named.recorded_only {
                    let field = columns::field(id, name)?;
                    if !fields.contains(&field) || !recorded_only.insert(field) {
                        return Err(invalid(format!(
                            "{}: invalid recorded-only {id:?}.{name}",
                            family.name
                        )));
                    }
                    let source = named.recorded_sources.get(name).ok_or_else(|| {
                        invalid(format!(
                            "{}: {id:?}.{name} needs its served-value refusal source",
                            family.name
                        ))
                    })?;
                    validate_source(source)?;
                }
                if named.recorded_sources.len() != named.recorded_only.len() {
                    return Err(invalid(format!(
                        "{}: {id:?} has stray recorded-bit sources",
                        family.name
                    )));
                }
                for (name, source) in &named.query_sources {
                    let field = columns::field(id, name)?;
                    if !fields.contains(&field) || recorded_only.contains(&field) {
                        return Err(invalid(format!(
                            "{}: query {id:?}.{name} needs its value",
                            family.name
                        )));
                    }
                    validate_source(source)?;
                }
                if !named.sources.is_empty() {
                    if matches!(&named.fields, Fields::All(all) if all == "all")
                        && named.all_reason.as_deref().is_none_or(str::is_empty)
                    {
                        return Err(invalid(format!(
                            "{}: whole {id:?} needs a reason",
                            family.name
                        )));
                    }
                    if named.sources.len() != fields.len() {
                        return Err(invalid(format!(
                            "{}: {id:?} needs a source for every field",
                            family.name
                        )));
                    }
                    for &field in &fields {
                        let name = id
                            .field_name(field)
                            .ok_or_else(|| invalid("Field outside schema"))?;
                        let source =
                            named.sources.get(&name.replace(' ', "")).ok_or_else(|| {
                                invalid(format!("{} lacks {id:?}.{name}'s source", family.name))
                            })?;
                        validate_source(source)?;
                    }
                }
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
                    recorded_only,
                    undeclared: named.source == "undeclared",
                    subscription: named.subscription.clone(),
                    previous: named.previous,
                    read_rule: named.read_rule.clone(),
                });
                primary_selected.push(Selection {
                    recorded_only: selected
                        .last()
                        .unwrap()
                        .recorded_only
                        .iter()
                        .copied()
                        .filter(|f| main_fields.contains(f))
                        .collect(),
                    fields: main_fields,
                    ..selected.last().unwrap().clone()
                });
            }
            primary.push(primary_selected);
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
        primary.truncate(self.families.len());
        for selections in &mut primary {
            selections.sort_by_key(|s| s.table);
        }
        family_relations.truncate(self.families.len());
        let shared_union = union(shared.iter().flatten());
        let shared_without_trigger = union(
            self.shared
                .iter()
                .zip(&shared)
                .filter(|(owner, _)| owner.name != "trigger-context")
                .flat_map(|(_, selections)| selections),
        );
        let family_specs = &self.families;
        for (owner, selections) in self.shared.iter_mut().zip(&shared) {
            for named in &mut owner.tables {
                if matches!(&named.fields, Fields::All(all) if all == "family-union") {
                    let id = table(&named.name)?;
                    let selected = selections.iter().find(|s| s.table == id).unwrap();
                    named.fields = Fields::Named(
                        selected
                            .fields
                            .iter()
                            .map(|&field| id.field_name(field).unwrap().to_owned())
                            .collect(),
                    );
                    named.recorded_only = selected
                        .recorded_only
                        .iter()
                        .map(|&f| id.field_name(f).unwrap().to_owned())
                        .collect();
                    named.recorded_sources = named
                        .recorded_only
                        .iter()
                        .map(|name| {
                            let source = family_specs
                                .iter()
                                .flat_map(|f| &f.tables)
                                .filter(|t| t.name == named.name)
                                .find_map(|t| t.recorded_sources.get(name))
                                .unwrap();
                            (name.clone(), source.clone())
                        })
                        .collect();
                }
            }
        }
        let shared_tables: BTreeSet<_> = shared_union.iter().map(|s| s.table).collect();
        let shared_relations: Vec<_> = relations
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.steps
                    .iter()
                    .any(|(table, _)| shared_tables.contains(table))
            })
            .map(|(at, _)| at)
            .collect();
        for indexes in &mut family_relations {
            indexes.retain(|at| !shared_relations.contains(at));
        }
        let private = families
            .iter()
            .map(|selections| {
                selections
                    .iter()
                    .filter(|s| !shared_tables.contains(&s.table))
                    .cloned()
                    .collect()
            })
            .collect();
        let combined = union(families.iter().chain(&shared).flatten());
        let declared = union(
            families
                .iter()
                .chain(&shared)
                .flatten()
                .filter(|s| !s.undeclared),
        );
        let tables = combined.iter().map(|s| s.table).collect();
        let readsets = self
            .families
            .iter()
            .map(|family| crate::readset::resolve(family, &shared_union))
            .collect::<io::Result<_>>()?;
        Ok(Resolved {
            manifest: self,
            families,
            shared,
            shared_union,
            shared_without_trigger,
            shared_relations,
            tables,
            union: combined,
            declared,
            primary,
            private,
            relations,
            family_relations,
            readsets,
        })
    }
}

pub fn union<'a>(selections: impl Iterator<Item = &'a Selection>) -> Vec<Selection> {
    let mut tables = BTreeMap::<TableId, BTreeMap<usize, bool>>::new();
    for selection in selections {
        let fields = tables.entry(selection.table).or_default();
        for &field in &selection.fields {
            let bits = selection.recorded_only.contains(&field);
            fields
                .entry(field)
                .and_modify(|only| *only &= bits)
                .or_insert(bits);
        }
    }
    tables
        .into_iter()
        .map(|(table, fields)| Selection {
            table,
            recorded_only: fields
                .iter()
                .filter_map(|(&f, &bits)| bits.then_some(f))
                .collect(),
            fields: fields.into_keys().collect(),
            undeclared: false,
            subscription: "stream".into(),
            previous: false,
            read_rule: None,
        })
        .collect()
}

fn validate_source(source: &str) -> io::Result<()> {
    if !source
        .rsplit_once(':')
        .is_some_and(|(file, line)| !file.is_empty() && line.parse::<usize>().is_ok_and(|n| n > 0))
    {
        return Err(invalid(format!("Invalid field source {source}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_union_keeps_family_fallbacks_and_previous_but_no_private_copy() {
        let text = r#"
version = 1
families_source = "synthetic"
[[families]]
name = "first"
origin = "code"
ordered = false
[[families.tables]]
name = "Countries"
fields = ["tag"]
fallback = ["canonical_slot"]
sources = { tag = "synthetic.rs:1", canonical_slot = "synthetic.rs:2" }
subscription = "lookup"
source = "synthetic.rs:1"
previous = true
[[families]]
name = "second"
origin = "code"
ordered = false
[[families.tables]]
name = "Countries"
fields = ["manpower_residual"]
subscription = "lookup"
source = "synthetic"
[[shared]]
name = "roster"
origin = "code"
ordered = false
[[shared.tables]]
name = "Countries"
fields = "family-union"
subscription = "lookup"
source = "synthetic"
[[shared]]
name = "trigger-context"
origin = "code"
ordered = false
[[shared.tables]]
name = "Countries"
fields = "all"
subscription = "lookup"
source = "synthetic"
"#;
        let resolved = toml::from_str::<Manifest>(text).unwrap().resolve().unwrap();
        assert_eq!(
            resolved.primary[0][0].fields,
            [columns::field(TableId::Countries, "tag").unwrap()]
        );
        assert_eq!(resolved.families[0][0].fields.len(), 2);
        assert!(resolved.families[0][0].previous);
        assert!(resolved.private.iter().all(Vec::is_empty));
        assert_eq!(resolved.shared[0][0].fields.len(), 3);
        assert_eq!(
            resolved.shared_union[0].fields.len(),
            TableId::Countries.schema().len()
        );
        let Fields::Named(names) = &resolved.manifest.shared[0].tables[0].fields else {
            panic!("shared fields were not expanded")
        };
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn source_coverage_and_fallback_overlap_are_validated() {
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='code'\nordered=false\n[[families.tables]]\nname='Countries'\nfields=['tag']\nfallback=['canonical_slot']\nsources={tag='synthetic.rs:1'}\nsubscription='lookup'\nsource='synthetic'";
        assert!(toml::from_str::<Manifest>(text).unwrap().resolve().is_err());
        assert!(
            toml::from_str::<Manifest>(
                &text.replace("fallback=['canonical_slot']", "fallback=['tag']")
            )
            .unwrap()
            .resolve()
            .is_err()
        );
        assert!(
            toml::from_str::<Manifest>(
                &text
                    .replace("fallback=['canonical_slot']", "fallback=[]")
                    .replace("synthetic.rs:1", "synthetic.rs:0")
            )
            .unwrap()
            .resolve()
            .is_err()
        );
    }

    #[test]
    fn shipped_fields_have_sources_and_every_repeated_table_has_an_owner() {
        let resolved = toml::from_str::<Manifest>(include_str!("../manifest.toml"))
            .unwrap()
            .resolve()
            .unwrap();
        let mut readers = BTreeMap::<TableId, usize>::new();
        for family in &resolved.manifest.families {
            for named in &family.tables {
                *readers.entry(table(&named.name).unwrap()).or_default() += 1;
                if family.origin != "note" {
                    let main = match &named.fields {
                        Fields::All(_) => table(&named.name).unwrap().schema().len(),
                        Fields::Named(fields) => fields.len(),
                    };
                    assert_eq!(
                        named.sources.len(),
                        main + named.fallback.len(),
                        "{} {}",
                        family.name,
                        named.name
                    );
                }
            }
        }
        for (table, count) in readers {
            if count >= 3 {
                assert!(
                    resolved.shared_union.iter().any(|s| s.table == table),
                    "{table:?}"
                );
            }
        }
        for (owner, selections) in resolved.manifest.shared.iter().zip(&resolved.shared) {
            if owner.name != "trigger-context" {
                for selection in selections {
                    let declared = union(resolved.families.iter().flatten());
                    assert_eq!(
                        selection.fields,
                        declared
                            .iter()
                            .find(|s| s.table == selection.table)
                            .unwrap()
                            .fields
                    );
                }
            }
        }
    }

    #[test]
    fn unknown_fields_and_tables_are_errors() {
        assert!(table("SyntheticMissingTable").is_err());
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='note'\nordered=false\n[[families.tables]]\nname='Countries'\nfields=['synthetic_missing_field']\nsubscription='stream'\nsource='synthetic'";
        assert!(toml::from_str::<Manifest>(text).unwrap().resolve().is_err());
    }

    #[test]
    fn shared_context_does_not_expand_family_or_previous_subscriptions() {
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='note'\nordered=true\n[[families.tables]]\nname='Countries'\nfields=['tag']\nsubscription='stream'\nsource='synthetic'\n[[families.readset]]\nname='Countries'\nfields=['tag']\nspool='stream'\nsource='synthetic'\n[[shared]]\nname='trigger-context'\norigin='code'\nordered=false\n[[shared.tables]]\nname='Countries'\nfields='all'\nsubscription='lookup'\nsource='synthetic'";
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
            recorded_only: Default::default(),
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

    #[test]
    fn value_read_dominates_recorded_bits_in_the_shared_union() {
        let bits = Selection {
            table: TableId::Countries,
            fields: vec![0, 1],
            recorded_only: [0, 1].into(),
            undeclared: false,
            subscription: "lookup".into(),
            previous: false,
            read_rule: None,
        };
        let values = Selection {
            fields: vec![1],
            recorded_only: BTreeSet::new(),
            ..bits.clone()
        };
        let selected = union([&bits, &values].into_iter());
        assert_eq!(selected[0].fields, [0, 1]);
        assert_eq!(selected[0].recorded_only, [0].into());
    }

    #[test]
    fn a_shared_reverse_index_leaves_every_family_group() {
        let text = r#"
version = 1
families_source = "synthetic"
[[families]]
name = "first"
origin = "code"
ordered = false
tables = [{name="CountryLinks", fields=["canonical_slot"], subscription="reverse", source="synthetic"}]
reverse = [{chain=["CountryLinks.canonical_slot"]}]
[[families]]
name = "second"
origin = "code"
ordered = false
tables = [{name="CountryLinks", fields=["canonical_slot"], subscription="reverse", source="synthetic"}]
reverse = [{chain=["CountryLinks.canonical_slot"]}]
[[shared]]
name = "roster"
origin = "code"
ordered = false
tables = [{name="CountryLinks", fields="family-union", subscription="lookup", source="synthetic"}]
"#;
        let resolved = toml::from_str::<Manifest>(text).unwrap().resolve().unwrap();
        assert_eq!(resolved.relations.len(), 1);
        assert_eq!(resolved.shared_relations, [0]);
        assert!(resolved.family_relations.iter().all(Vec::is_empty));
    }
}
