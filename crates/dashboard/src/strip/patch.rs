use serde::Serialize;
use serde_json::Value;
use std::io;

mod array;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(untagged)]
pub enum Part {
    Key(String),
    Index(usize),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    Replace(Vec<Part>, Value),
    Remove(Vec<Part>),
    Splice(Vec<Part>, usize, usize, Vec<Value>),
}

pub fn diff(before: &Value, after: &Value) -> Vec<Operation> {
    let mut out = Vec::new();
    walk(before, after, &mut Vec::new(), &mut out);
    out
}

fn walk(before: &Value, after: &Value, path: &mut Vec<Part>, out: &mut Vec<Operation>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().filter(|key| !b.contains_key(*key)) {
                path.push(Part::Key(key.clone()));
                out.push(Operation::Remove(path.clone()));
                path.pop();
            }
            for (key, value) in b {
                path.push(Part::Key(key.clone()));
                match a.get(key) {
                    Some(previous) => walk(previous, value, path, out),
                    None => out.push(Operation::Replace(path.clone(), value.clone())),
                }
                path.pop();
            }
        }
        (Value::Array(a), Value::Array(b)) => array::diff(a, b, path, out),
        _ => out.push(Operation::Replace(path.clone(), after.clone())),
    }
}

pub fn encode(patch: &[Operation]) -> io::Result<Vec<u8>> {
    rmp_serde::to_vec(patch).map_err(io::Error::other)
}

#[cfg(test)]
pub mod apply;

#[cfg(test)]
mod tests;
