//! Effect parsing dispatches registered classes, scopes and scripted effects in
//! their accepted order. Reader and class identities come from the verified
//! executable; parsing retains effects as data.

use engine_executable::executable_tables::RuntimeTable;

use crate::{
    key_names::KeyNames,
    parser::{
        Item, c_string,
        native::{Field, Kind, Shape},
    },
    trigger_keys::ScopeStep,
};

mod classes;
pub(crate) mod key_readers;
pub use key_readers::is_own_field;
#[cfg(test)]
mod tests;
pub(crate) mod value_readers;
pub(crate) const READ_SCRIPT: u64 = 0x140587F40;
pub(crate) const READ_EFFECT: u64 = 0x140588290;
pub(crate) const READ_VALUE: u64 = 0x141436660;
const IF_TOKEN: u32 = 0x27C3;
pub static REGISTERED_EFFECTS: RuntimeTable<&'static [(u32, &'static str)]> =
    RuntimeTable::new(|| {
        &engine_executable::executable_tables::get()
            .registries
            .effects
    });
pub static EFFECT_CLASSES: RuntimeTable<&'static [(u32, u64, u64)]> = RuntimeTable::new(|| {
    &engine_executable::executable_tables::get()
        .registries
        .effect_classes
});
pub fn registered(keyword: &[u8]) -> Option<u32> {
    REGISTERED_EFFECTS
        .iter()
        .find(|(_, name)| name.as_bytes().eq_ignore_ascii_case(keyword))
        .map(|(token, _)| *token)
}
pub fn class(token: u32) -> Option<(u64, u64)> {
    EFFECT_CLASSES
        .binary_search_by_key(&token, |(id, _, _)| *id)
        .ok()
        .map(|index| (EFFECT_CLASSES[index].1, EFFECT_CLASSES[index].2))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeClass {
    pub name: &'static str,
    pub vtable: u64,
}
pub(crate) fn class_vtable(token: u32) -> Option<u64> {
    classes::CLASS_IDENTITIES
        .binary_search_by_key(&token, |&(id, _, _)| id)
        .ok()
        .map(|index| classes::CLASS_IDENTITIES[index].2)
}

pub fn native_class(keyword: &[u8]) -> Option<NativeClass> {
    let token = if keyword.eq_ignore_ascii_case(b"else") || keyword.eq_ignore_ascii_case(b"else_if")
    {
        IF_TOKEN
    } else {
        registered(keyword)?
    };
    classes::CLASS_IDENTITIES
        .binary_search_by_key(&token, |(id, _, _)| *id)
        .ok()
        .map(|index| {
            let (_, name, vtable) = classes::CLASS_IDENTITIES[index];
            NativeClass { name, vtable }
        })
}
pub fn read_body(bytes: &[u8], names: &KeyNames) -> Result<Vec<Item>, String> {
    let read = crate::parser::native::read(
        bytes,
        crate::parser::native::Mode::KeepMark,
        crate::parser::native::Top::Persistent(()),
        Some(names),
        &mut |(), field| shape(Some(names), field),
    );
    if let Some(why) = read.unsettled {
        return Err(why.to_string());
    }
    if !read.misreads.is_empty() {
        return Err(read
            .misreads
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "));
    }
    Ok(read.items)
}
pub fn adds_child(item: &Item, names: &KeyNames) -> Result<bool, String> {
    let keyword = item.keyword().map(|keyword| keyword.folded());
    if matches!(keyword.as_deref(), Some(b"else" | b"else_if")) {
        return Ok(false);
    }
    if keyword.as_deref().and_then(registered).is_some() {
        return Ok(true);
    }
    let key = c_string(item.key.as_deref().unwrap_or_default());
    if item.value.items().is_some() {
        match names.scope(key, item.key_quoted) {
            ScopeStep::Scope(_) => return Ok(true),
            ScopeStep::NotScope => {}
            ScopeStep::Unknown(why) => return Err(why),
        }
    }
    Ok(names.scripted_effects.contains(&key.to_ascii_lowercase()))
}
pub fn description_fields(item: &Item) -> Result<Vec<Item>, String> {
    let items = item.value.items().unwrap_or_default();
    let token = if item.keyword_is("else") {
        Some(key_readers::ELSE_TOKEN)
    } else if item.keyword_is("else_if") {
        Some(key_readers::ELSE_IF_TOKEN)
    } else {
        item.keyword().and_then(|key| registered(&key.folded()))
    };
    let Some(token) = token else {
        return Ok(items.to_vec());
    };
    let class_token = if matches!(token, key_readers::ELSE_TOKEN | key_readers::ELSE_IF_TOKEN) {
        IF_TOKEN
    } else {
        token
    };
    let (value, key) = class(class_token).expect("registered effect class");
    if value == READ_SCRIPT || value == 0x14056F280 {
        if key == READ_EFFECT && value == READ_SCRIPT {
            return Ok(items.to_vec());
        }
        return key_readers::description_fields(token, items);
    }
    if value == READ_VALUE || items.is_empty() {
        return Ok(Vec::new());
    }
    value_readers::description_fields(value, key, token, item)
}
pub fn random_list_members(item: &Item) -> Result<Vec<Item>, String> {
    let token = item
        .keyword()
        .and_then(|key| registered(&key.folded()))
        .ok_or_else(|| "random-list own-key projection needs a registered class".to_string())?;
    key_readers::random_list_members(token, item.value.items().unwrap_or_default())
}
pub fn random_member_fields(item: &Item) -> Vec<Item> {
    key_readers::random_member_fields(item.value.items().unwrap_or_default())
}
pub(crate) fn bound_localization_shape(field: &Field<'_>) -> Shape<()> {
    if field.opens() {
        Shape::EffectValueKeys(0x142426550)
    } else {
        Shape::One
    }
}

pub(crate) fn body_error(key: &str, value: &str) -> String {
    format!(
        "field {key} is {value} where the effect reader requires a block: it is reported and nothing more is read"
    )
}

fn class_shape(readers: (u64, u64), field: &Field<'_>) -> Shape<()> {
    let (value_reader, key_reader) = readers;
    if let Some(shape) = key_readers::value_shape(value_reader, field) {
        return shape;
    }
    match readers {
        (READ_SCRIPT, key_reader) => key_readers::shape(key_reader, field),
        (READ_VALUE, _) => Shape::One,
        _ => {
            if let Some(read) = value_readers::shape(value_reader, key_reader, field) {
                return read;
            }
            let name = field.keyword().unwrap_or_default();
            Shape::Reported {
                kind: crate::DiagnosticKind::Unsupported,
                report: format!(
                    "Deferred effect `{}`: its value reader is unsupported; by-key reading retained",
                    String::from_utf8_lossy(&name)
                ),
                read: Box::new(Shape::ByKey),
            }
        }
    }
}
pub fn shape(names: Option<&KeyNames>, field: &Field<'_>) -> Shape<()> {
    let keyword = field.keyword();
    if matches!(keyword.as_deref(), Some(b"else" | b"else_if")) {
        return class_shape(
            class(IF_TOKEN).expect("registered conditional effect"),
            field,
        );
    }
    if let Some(readers) = keyword.as_deref().and_then(registered).and_then(class) {
        return class_shape(readers, field);
    }
    let Some(names) = names else {
        return Shape::Unsettled(
            "Effect parsing has no content names for its scope and scripted-effect lookups".into(),
        );
    };
    if field.opens() {
        match names.scope(c_string(&field.key.text), field.key.kind == Kind::Quoted) {
            ScopeStep::Scope(_) => return Shape::Effect,
            ScopeStep::NotScope => {}
            ScopeStep::Unknown(why) => return Shape::Unsettled(why),
        }
    }
    if names
        .scripted_effects
        .contains(&c_string(&field.key.text).to_ascii_lowercase())
    {
        return Shape::One;
    }
    Shape::Skip
}
