use super::*;
use serde::ser::{SerializeMap, SerializeSeq};
use std::collections::HashMap;

pub(super) struct Served<'a> {
    data: &'a Value,
    patches: HashMap<*const Value, Option<Value>>,
    served: Option<Value>,
}
impl<'a> Served<'a> {
    pub(super) fn new(
        source: &Source,
        product: &str,
        name: &str,
        data: &'a Value,
        content: Option<&ContentOptions>,
        fallbacks: &std::collections::BTreeSet<String>,
    ) -> Self {
        let paths = content_warning_paths(product, name);
        let mut reply = Self::decorate(data, paths, Vec::new(), |warnings| {
            source.append_stack_warnings(warnings);
            source.append_changed_warnings(warnings, content);
        });
        reply.add_lines(paths, served_lines(source, content, fallbacks));
        if product == crate::products::division_equipment::history::QUERY && data.is_object() {
            reply.served = Some(Value::from(
                crate::products::division_equipment::history::served_lines(source, content),
            ));
            reply.patches.entry(data).or_insert(None);
        }
        reply
    }

    fn decorate(
        data: &'a Value,
        paths: &[&[&str]],
        lines: Vec<String>,
        mut refresh: impl FnMut(&mut Vec<String>),
    ) -> Self {
        let mut patches = HashMap::new();
        for path in paths {
            patch_at(data, path, &mut patches, &mut |value| {
                let Value::Array(original) = value else {
                    return None;
                };
                let strings: Option<Vec<String>> = original
                    .iter()
                    .map(|line| line.as_str().map(str::to_owned))
                    .collect();
                let warnings: Vec<Value> = match strings {
                    Some(mut strings) => {
                        refresh(&mut strings);
                        strings.into_iter().map(Value::String).collect()
                    }
                    None => original.clone(),
                };
                Some(Value::Array(warnings))
            });
        }
        let mut reply = Self {
            data,
            patches,
            served: None,
        };
        reply.add_lines(paths, lines);
        reply
    }

    fn add_lines(&mut self, paths: &[&[&str]], lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        for path in paths {
            if let Some(first) = first_target(self.data, path) {
                let Some(Some(Value::Array(warnings))) =
                    self.patches.get_mut(&(first as *const Value))
                else {
                    break;
                };
                for line in lines {
                    if !warnings.iter().any(|kept| kept.as_str() == Some(&line)) {
                        warnings.push(Value::String(line));
                    }
                }
                break;
            }
        }
    }
}
fn first_target<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    match path.split_first() {
        None => Some(value),
        Some((&"*", rest)) => value
            .as_array()?
            .iter()
            .find_map(|item| first_target(item, rest)),
        Some((key, rest)) => first_target(value.get(*key)?, rest),
    }
}
fn patch_at(
    value: &Value,
    path: &[&str],
    patches: &mut HashMap<*const Value, Option<Value>>,
    replace: &mut impl FnMut(&Value) -> Option<Value>,
) -> bool {
    let changed = match path.split_first() {
        None => match replace(value) {
            Some(replacement) => {
                patches.insert(value, Some(replacement));
                return true;
            }
            None => return false,
        },
        Some((&"*", rest)) => {
            let mut changed = false;
            for item in value.as_array().into_iter().flatten() {
                changed |= patch_at(item, rest, patches, replace);
            }
            changed
        }
        Some((key, rest)) => value
            .get(*key)
            .is_some_and(|next| patch_at(next, rest, patches, replace)),
    };
    if changed {
        patches.entry(value).or_insert(None);
    }
    changed
}
struct View<'a, 'b> {
    data: &'a Value,
    reply: &'b Served<'a>,
}
impl Serialize for Served<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        View {
            data: self.data,
            reply: self,
        }
        .serialize(serializer)
    }
}
impl Serialize for View<'_, '_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Unchanged subtrees use Value's serializer directly, including large row/point arrays.
        match self.reply.patches.get(&(self.data as *const Value)) {
            None => return self.data.serialize(serializer),
            Some(Some(value)) => return value.serialize(serializer),
            Some(None) => {}
        }
        match self.data {
            Value::Array(rows) => {
                let mut output = serializer.serialize_seq(Some(rows.len()))?;
                for data in rows {
                    output.serialize_element(&View {
                        data,
                        reply: self.reply,
                    })?;
                }
                output.end()
            }
            Value::Object(fields) => {
                let mut served = std::ptr::eq(self.data, self.reply.data)
                    .then_some(self.reply.served.as_ref())
                    .flatten();
                let mut output = serializer.serialize_map(Some(
                    fields.len() + usize::from(served.is_some() && !fields.contains_key("served")),
                ))?;
                for (key, data) in fields {
                    if served.is_some() && key.as_str() >= "served" {
                        output.serialize_entry("served", served.take().unwrap())?;
                    }
                    if key == "served"
                        && std::ptr::eq(self.data, self.reply.data)
                        && self.reply.served.is_some()
                    {
                        continue;
                    }
                    output.serialize_entry(
                        key,
                        &View {
                            data,
                            reply: self.reply,
                        },
                    )?;
                }
                if let Some(served) = served {
                    output.serialize_entry("served", served)?;
                }
                output.end()
            }
            value => value.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests;
