//! Division equipment's served history: every country's whole history, and some countries'
//! by host and over each span; compared by what it says ([`canonical`]).

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, io};

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        for tag in &countries {
            ask.ask(
                super::QUERY,
                json!({ "branchId": id, "tag": tag, "countFor": "owner" }),
            )?;
        }
        for tag in served::some(&countries) {
            ask.ask(
                super::QUERY,
                json!({ "branchId": id, "tag": tag, "countFor": "host" }),
            )?;
            for (from, to) in branch.spans() {
                ask.ask(
                    super::QUERY,
                    json!({ "branchId": id, "tag": tag, "countFor": "owner", "firstHour": from, "lastHour": to }),
                )?;
            }
        }
    }
    Ok(())
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("A division equipment history reply is malformed: {what}"),
    )
}

/// A history reply as it is compared: each series by its name and each operand it names by
/// what that operand is, so two preparations numbering the history's types, notes, namings,
/// artworks and designs apart compare by the replies the UI reads from them. Types are named
/// by their archetype, a naming, artwork or design value by its name and refusal, and a
/// note's counts by the refusals they make in each hour's warnings, which are compared whole.
/// A run going on with the same named value is one run. A pending reply is itself.
pub fn canonical(reply: &Value) -> io::Result<Value> {
    let Value::Object(reply) = reply else {
        return Ok(reply.clone());
    };
    let get = |name: &str| reply.get(name).ok_or_else(|| invalid(name));
    let window = get("window")?;
    let by_id = |name: &str| -> io::Result<&Map<String, Value>> {
        get(name)?.as_object().ok_or_else(|| invalid(name))
    };
    let (namings, artworks, designs) = (by_id("namings")?, by_id("artworks")?, by_id("designs")?);
    let mut archetypes = BTreeMap::new();
    let mut kinds = Vec::new();
    for kind in get("kinds")?.as_array().ok_or_else(|| invalid("kinds"))? {
        let id = kind
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("kind id"))?;
        let archetype = kind.get("archetype").cloned().unwrap_or(Value::Null);
        archetypes.insert(id.to_owned(), archetype.to_string());
        kinds.push(json!({
            "archetype": archetype,
            "name": kind.get("name"),
            "nameMissing": kind.get("nameMissing"),
        }));
    }
    let mut series = Map::new();
    for held in window
        .get("series")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("series"))?
    {
        let name = held
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("series name"))?;
        let parts: Vec<&str> = name.split('|').collect();
        let named = match parts.as_slice() {
            ["note", _] => continue,
            ["type", kind, rest @ ..] => format!(
                "type|{}|{}",
                archetypes
                    .get(*kind)
                    .ok_or_else(|| invalid("series type"))?,
                rest.join("|")
            ),
            _ => name.to_owned(),
        };
        let operands = match parts.as_slice() {
            ["template", _, "naming"] => Some(namings),
            ["template", _, "artwork"] => Some(artworks),
            ["type", _, "design", _] | ["stockDesign", _] => Some(designs),
            _ => None,
        };
        let mut runs: Vec<Value> = Vec::new();
        for run in held
            .get("runs")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("runs"))?
        {
            let [first, last, value, starts] = run.as_array().map(Vec::as_slice).unwrap_or(&[])
            else {
                return Err(invalid("run"));
            };
            let value = match operands {
                Some(operands) => {
                    let id = value.as_str().ok_or_else(|| invalid("run value"))?;
                    operands
                        .get(id)
                        .cloned()
                        .ok_or_else(|| invalid("run operand"))?
                }
                None => value.clone(),
            };
            let (first, last) = (
                first.as_i64().ok_or_else(|| invalid("run hour"))?,
                last.as_i64().ok_or_else(|| invalid("run hour"))?,
            );
            if let Some(Value::Array(before)) = runs.last_mut()
                && before[1].as_i64().is_some_and(|end| end + 1 == first)
                && before[2] == value
                && starts == &Value::Bool(false)
            {
                before[1] = json!(last);
                continue;
            }
            runs.push(json!([first, last, value, starts]));
        }
        if series.insert(named, Value::Array(runs)).is_some() {
            return Err(invalid("a series named twice"));
        }
    }
    let mut formed = reply.clone();
    for numbered in ["namings", "artworks", "designs"] {
        formed.remove(numbered);
    }
    formed.insert("kinds".into(), Value::Array(kinds));
    formed.insert(
        "window".into(),
        json!({
            "firstHour": window.get("firstHour"),
            "lastHour": window.get("lastHour"),
            "known": window.get("known"),
            "reads": window.get("reads"),
            "series": series,
        }),
    );
    Ok(Value::Object(formed))
}
