//! Authored CNameGroup text, read by 1409B9A20. Repeated groups merge;
//! ordered entries replace by ID, unordered entries append. Names are
//! literal authored text (ReadStringList), not localisation keys.
use crate::{ResolvedLayers, fields, parser, triggers};
use std::{collections::BTreeMap, io};

#[derive(Default)]
pub struct Groups(BTreeMap<String, Group>);
#[derive(Default)]
struct Group {
    ordered: BTreeMap<i32, String>,
    unordered: Vec<String>,
    fallback: String,
    refused: Option<String>,
}
impl Groups {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut out = Self::default();
        // 140191044..140191083 loads all four into the same database.
        for folder in [
            "names_divisions",
            "names_ships",
            "codenames_operatives",
            "names_railway_guns",
        ] {
            for path in triggers::folder_files(layers, &format!("common/units/{folder}"))? {
                let doc = parser::parse(&layers.read(&path)?)
                    .map_err(|e| fields::invalid(format!("{path}: {e}")))?;
                out.add(&doc.items)?;
            }
        }
        Ok(out)
    }
    fn add(&mut self, items: &[parser::Item]) -> io::Result<()> {
        for item in items {
            let key = fields::key_str(item, "name group")?;
            let group = self.0.entry(key.into()).or_default();
            if let Err(e) = group.add(fields::block(&item.value, key)?) {
                group.refused = Some(format!("name group {key}: {e}"));
            }
        }
        Ok(())
    }
    pub fn name(&self, key: &str, order: i32, ordered: bool) -> Result<String, String> {
        let group = self
            .0
            .get(key)
            .ok_or_else(|| format!("name group {key} absent from content"))?;
        if let Some(reason) = &group.refused {
            return Err(reason.clone());
        }
        if ordered
            && order > 0
            && let Some(text) = group.ordered.get(&order)
        {
            return Ok(number(text, order));
        }
        if !ordered
            && let Some(text) = usize::try_from(order)
                .ok()
                .and_then(|i| group.unordered.get(i))
        {
            return Ok(text.clone());
        }
        if group.fallback.is_empty() {
            return Err(format!(
                "name group {key} uses random fallback 1409B4CE0; choice provenance is not retained"
            ));
        }
        // 1409B4CE0 increments unordered ordinals before fallback formatting.
        let order = if ordered {
            order
        } else {
            order.wrapping_add(1)
        };
        Ok(number(&group.fallback, order.max(1)))
    }
}
impl Group {
    fn add(&mut self, items: &[parser::Item]) -> io::Result<()> {
        for item in items {
            match item.key.as_deref() {
                Some(b"fallback_name") => {
                    self.fallback = fields::text(&item.value, "fallback_name")?.into()
                }
                Some(b"ordered") => {
                    for entry in fields::block(&item.value, "ordered")? {
                        let id: i32 = fields::key_str(entry, "ordered name ID")?
                            .parse()
                            .map_err(|_| fields::invalid("invalid ordered name ID"))?;
                        if id < 1 {
                            return Err(fields::invalid("ordered name ID is not positive"));
                        }
                        if let Some(text) = first(&entry.value)? {
                            self.ordered.insert(id, text);
                        }
                    }
                }
                Some(b"unordered") => {
                    for entry in fields::block(&item.value, "unordered")? {
                        if entry.key.is_some() {
                            return Err(fields::invalid("keyed unordered name"));
                        }
                        if let Some(text) = first(&entry.value)? {
                            self.unordered.push(text);
                        }
                    }
                }
                // 1409B9A20 token 30FA appends literal strings to the
                // same unordered entries as the extended `unordered` arm.
                Some(b"unique") => {
                    for entry in fields::block(&item.value, "unique")? {
                        if entry.key.is_some() {
                            return Err(fields::invalid("keyed name list entry"));
                        }
                        self.unordered
                            .push(fields::text(&entry.value, "name")?.into());
                    }
                }
                // These select a group. The recorded key already witnesses
                // that choice; they do not change its stored display text.
                Some(
                    b"name"
                    | b"type"
                    | b"for_countries"
                    | b"can_use"
                    | b"division_types"
                    | b"ship_types"
                    // 1409B5640 selects member text without the group prefix.
                    | b"prefix"
                    | b"link_numbering_with",
                ) => {}
                _ => {
                    return Err(fields::invalid(format!(
                        "unsupported name group field {:?}",
                        item.key
                    )));
                }
            }
        }
        Ok(())
    }
}
fn first(value: &parser::Value) -> io::Result<Option<String>> {
    let entries = fields::block(value, "extended name")?;
    if entries
        .iter()
        .any(|e| e.key.is_some() || e.value.scalar_str().is_none())
    {
        return Err(fields::invalid("invalid extended name string list"));
    }
    Ok(entries
        .first()
        .map(|e| e.value.scalar_str().unwrap().to_owned()))
}
/// 1409B57C0 chooses %s before %d, replacing every occurrence of only that
/// directive. Roman numerals 1422B8DD0 use the subtractive decimal pairs.
fn number(text: &str, order: i32) -> String {
    if !text.contains("%s") {
        return text.replace("%d", &order.to_string());
    }
    let mut n = order;
    let mut roman = String::new();
    for (value, digit) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while n >= value {
            roman.push_str(digit);
            n -= value;
        }
    }
    text.replace("%s", &roman)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merged_groups_replace_ordered_append_unordered_and_keep_literal_text() {
        let mut g = Groups::default();
        let doc = parser::parse(
            br#"x={ordered={4={"old"}} unordered={{"A"}} fallback_name="%d. X"}
            x={ordered={4={"%s/%d" "description" "url"}} unordered={{"B"}}}"#,
        )
        .unwrap();
        g.add(&doc.items).unwrap();
        assert_eq!(g.name("x", 4, true).unwrap(), "IV/%d");
        assert_eq!(g.name("x", 1, false).unwrap(), "B");
        assert_eq!(g.name("x", 2, false).unwrap(), "3. X");
        assert_eq!(g.name("x", 0, true).unwrap(), "1. X");
    }
    #[test]
    fn irrecoverable_random_fallback_and_unsupported_fields_are_named() {
        let mut g = Groups::default();
        g.add(&parser::parse(b"a={} b={unknown=yes}").unwrap().items)
            .unwrap();
        assert!(
            g.name("a", 1, true)
                .unwrap_err()
                .contains("random fallback")
        );
        assert!(
            g.name("b", 1, true)
                .unwrap_err()
                .contains("unsupported name group field")
        );
    }
    #[test]
    fn ship_unique_entries_append_and_member_text_does_not_apply_prefix() {
        let mut groups = Groups::default();
        groups.add(&parser::parse(br#"ship={prefix="PRE " ship_types={test_hull} unique={"First" "Second"} unordered={{"Third"}}} invalid={names={"wrong"}}"#).unwrap().items).unwrap();
        for (i, name) in ["First", "Second", "Third"].into_iter().enumerate() {
            assert_eq!(groups.name("ship", i as i32, false).unwrap(), name);
        }
        assert!(
            groups
                .name("invalid", 0, false)
                .unwrap_err()
                .contains("unsupported")
        );
    }
}
