
use content_reader_layers::{ResolvedLayers, script_files::ScriptPath};
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Debug)]
struct ScriptError(String);

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScriptError {}

fn script_error(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, ScriptError(message))
}

pub fn is_script_error(error: &io::Error) -> bool {
    let mut inner: Option<&(dyn std::error::Error + 'static)> = error
        .get_ref()
        .map(|error| error as &(dyn std::error::Error + 'static));
    while let Some(current) = inner {
        if current.is::<ScriptError>() {
            return true;
        }
        inner = if let Some(wrapped) = current.downcast_ref::<io::Error>() {
            wrapped
                .get_ref()
                .map(|error| error as &(dyn std::error::Error + 'static))
        } else if let Some(wrapped) = current.downcast_ref::<Arc<io::Error>>() {
            wrapped
                .get_ref()
                .map(|error| error as &(dyn std::error::Error + 'static))
        } else {
            current.source()
        };
    }
    false
}

const HOOK_INTERVAL: u32 = 1_000_000;
const HOOK_LIMIT: u32 = 100;
const BUDGET_SPENT: &str = "define script exceeded its instruction limit";

const BUDGET_GUARD: &str = r#"
local spent, message = ...
local raise, catch, handled = error, pcall, xpcall
local function guard(...)
    if spent() then
        raise(message, 0)
    end
    return ...
end
pcall = function(...) return guard(catch(...)) end
xpcall = function(...) return guard(handled(...)) end
if type(coroutine) == "table" and coroutine.resume then
    local resume = coroutine.resume
    coroutine.resume = function(...) return guard(resume(...)) end
end
"#;

const MEMORY_LIMIT: usize = 256 << 20;

#[derive(Clone)]
pub struct Defines {
    lua: Lua,
    sealed: bool,
    spent: Arc<AtomicBool>,
}

impl std::fmt::Debug for Defines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn value(item: Value, active: &mut Vec<usize>) -> io::Result<String> {
            Ok(match item {
                Value::Nil => "nil".into(),
                Value::Boolean(v) => format!("{v:?}"),
                Value::Integer(v) => format!("{v:?}"),
                Value::Number(v) => format!("{v:?}"),
                Value::String(v) => format!("string:{:?}", v.as_bytes()),
                Value::Table(table) => {
                    let id = table.to_pointer() as usize;
                    if active.contains(&id) {
                        return Err(invalid("cyclic NDefines table"));
                    }
                    active.push(id);
                    let mut entries = Vec::new();
                    for pair in table.pairs::<Value, Value>() {
                        let (key, item) = pair.map_err(lua_error)?;
                        entries.push((value(key, active)?, value(item, active)?));
                    }
                    active.pop();
                    entries.sort();
                    format!("table:{entries:?}")
                }
                other => {
                    return Err(invalid(format!(
                        "unsupported NDefines {}",
                        other.type_name()
                    )));
                }
            })
        }
        let values = self
            .lua
            .globals()
            .raw_get::<Value>("NDefines")
            .map_err(lua_error)
            .and_then(|v| value(v, &mut Vec::new()));
        f.debug_tuple("Defines").field(&values).finish()
    }
}

#[derive(Clone)]
struct CachedDefines {
    defines: Defines,
    scripts: Vec<(ScriptPath, [u8; 32])>,
}

#[derive(Default)]
struct UnscopedDefines(std::sync::Mutex<Option<CachedDefines>>);

pub fn read(layers: &ResolvedLayers) -> io::Result<Defines> {
    if layers.is_generation_scoped() {
        let build = |layers: &ResolvedLayers| {
            let mut defines = Defines::new()?;
            for path in script_paths(layers)? {
                defines.load(&path.text(), &path.read(layers)?)?;
            }
            defines.sealed = true;
            Ok(defines)
        };
        let defines = if layers.reads_visual_defines() {
            layers.retained_product("defines Lua 5.1", 1, &[], build)?
        } else {
            layers.retained_recorded_product("defines Lua 5.1", 1, &[], build)?
        };
        return Ok(defines.as_ref().clone());
    }
    let target = if layers.reads_visual_defines() {
        layers.clone()
    } else {
        layers.recorded_or_self()?
    };
    let paths = script_paths(&target)?;
    let cache = target.derived::<UnscopedDefines>();
    let cached = cache
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let same_paths = cached.as_ref().is_some_and(|cached| {
        cached.scripts.len() == paths.len()
            && cached
                .scripts
                .iter()
                .zip(&paths)
                .all(|((cached_path, _), path)| cached_path == path)
    });
    let (defines, digests) = if let Some(cached) = cached.filter(|_| same_paths) {
        let mut verified = Vec::with_capacity(paths.len());
        let mut changed_at = None;
        for (index, path) in paths.iter().enumerate() {
            let bytes = path.read(&target)?;
            let digest = bytes.digest();
            if digest != cached.scripts[index].1 {
                changed_at = Some((index, bytes, digest));
                break;
            }
            verified.push((path.clone(), bytes, digest));
        }
        let Some((index, changed_bytes, changed_digest)) = changed_at else {
            return Ok(cached.defines);
        };
        let mut defines = Defines::new()?;
        let mut digests = Vec::with_capacity(paths.len());
        for (path, bytes, digest) in verified {
            defines.load(&path.text(), &bytes)?;
            digests.push((path, digest));
        }
        let changed_path = &paths[index];
        defines.load(&changed_path.text(), &changed_bytes)?;
        digests.push((changed_path.clone(), changed_digest));
        for path in &paths[index + 1..] {
            let bytes = path.read(&target)?;
            defines.load(&path.text(), &bytes)?;
            digests.push((path.clone(), bytes.digest()));
        }
        defines.sealed = true;
        (defines, digests)
    } else {
        let mut defines = Defines::new()?;
        let mut digests = Vec::with_capacity(paths.len());
        for path in paths {
            let bytes = path.read(&target)?;
            defines.load(&path.text(), &bytes)?;
            digests.push((path, bytes.digest()));
        }
        defines.sealed = true;
        (defines, digests)
    };
    let mut guard = cache.0.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(cached) = guard.as_ref()
        && cached.scripts == digests
    {
        return Ok(cached.defines.clone());
    }
    *guard = Some(CachedDefines {
        defines: defines.clone(),
        scripts: digests,
    });
    Ok(defines)
}

pub(crate) fn script_paths(layers: &ResolvedLayers) -> io::Result<Vec<ScriptPath>> {
    let listed = layers.listed_layer_paths(b"common/defines/")?;
    Ok(
        load_order_paths(b"common/defines/", b"lua", listed.iter().map(Vec::as_slice))
            .into_iter()
            .map(ScriptPath::new)
            .collect(),
    )
}

pub fn load_order<'a>(
    root: &str,
    filter: &str,
    paths: impl IntoIterator<Item = &'a ScriptPath>,
) -> Vec<ScriptPath> {
    load_order_paths(
        root.as_bytes(),
        filter.as_bytes(),
        paths.into_iter().map(ScriptPath::as_bytes),
    )
    .into_iter()
    .map(ScriptPath::new)
    .collect()
}

pub fn load_order_paths<'a>(
    root: &[u8],
    filter: &[u8],
    paths: impl Iterator<Item = &'a [u8]>,
) -> Vec<Vec<u8>> {
    let mut paths: Vec<&[u8]> = paths
        .filter(|path| {
            path.len() > root.len()
                && path[..root.len()].eq_ignore_ascii_case(root)
                && path.ends_with(filter)
        })
        .map(|path| &path[root.len()..])
        .collect();
    paths.sort_by(|a, b| {
        let (a, b) = (
            a.split(|byte| *byte == b'/').collect::<Vec<_>>(),
            b.split(|byte| *byte == b'/').collect::<Vec<_>>(),
        );
        for (index, (x, y)) in a.iter().zip(&b).enumerate() {
            let (x_file, y_file) = (index + 1 == a.len(), index + 1 == b.len());
            if x != y || x_file != y_file {
                return y_file.cmp(&x_file).then_with(|| x.cmp(y));
            }
        }
        a.len().cmp(&b.len())
    });
    paths
        .into_iter()
        .map(|path| [root, path].concat())
        .collect()
}

impl Defines {
    pub fn new() -> io::Result<Self> {
        let lua = Lua::new_with(
            StdLib::STRING | StdLib::TABLE | StdLib::MATH,
            LuaOptions::default(),
        )
        .map_err(lua_error)?;
        lua.set_memory_limit(MEMORY_LIMIT).map_err(lua_error)?;
        let globals = lua.globals();
        for name in ["dofile", "loadfile", "print"] {
            globals.raw_remove(name).map_err(lua_error)?;
        }
        let hooks = Arc::new(AtomicU32::new(0));
        let spent = Arc::new(AtomicBool::new(false));
        let spending = Arc::clone(&spent);
        lua.set_global_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INTERVAL),
            move |_, _| {
                if hooks.fetch_add(1, Ordering::Relaxed) >= HOOK_LIMIT {
                    spending.store(true, Ordering::Relaxed);
                    Err(mlua::Error::runtime(BUDGET_SPENT))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )
        .map_err(lua_error)?;
        let checked = Arc::clone(&spent);
        let check = lua
            .create_function(move |_, ()| Ok(checked.load(Ordering::Relaxed)))
            .map_err(lua_error)?;
        lua.load(BUDGET_GUARD)
            .set_name("define instruction limit")
            .call::<()>((check, BUDGET_SPENT))
            .map_err(lua_error)?;
        Ok(Self {
            lua,
            sealed: false,
            spent,
        })
    }

    pub fn load(&self, path: &str, source: &[u8]) -> io::Result<()> {
        if self.sealed {
            return Err(invalid(
                "resolved defines are immutable; use Defines::new for a new script load",
            ));
        }
        self.run(path, source)
            .map_err(|error| script_error(format!("defines unavailable in {path}: {error}")))
    }

    fn run(&self, name: &str, source: &[u8]) -> io::Result<()> {
        self.lua
            .load(source)
            .set_name(name)
            .exec()
            .map_err(lua_error)?;
        if self.spent.load(Ordering::Relaxed) {
            return Err(invalid(BUDGET_SPENT));
        }
        Ok(())
    }

    fn value(&self, section: &str, name: &str) -> io::Result<Value> {
        let missing = || invalid(format!("missing NDefines.{section}.{name}"));
        let table = |value: Value| match value {
            Value::Table(table) => Ok(table),
            _ => Err(missing()),
        };
        let defines: Table = table(self.lua.globals().get("NDefines").map_err(lua_error)?)?;
        let section_table = table(defines.get(section).map_err(lua_error)?)?;
        section_table.get(name).map_err(lua_error)
    }

    pub fn number(&self, section: &str, name: &str) -> io::Result<f64> {
        number(self.value(section, name)?)
            .ok_or_else(|| invalid(format!("NDefines.{section}.{name} is not a number")))
    }

    pub fn int32(&self, section: &str, name: &str) -> io::Result<i32> {
        let Some(value) = number(self.value(section, name)?) else {
            return Ok(0);
        };
        let value = value.trunc();
        let wide = if (-(2.0_f64.powi(63))..2.0_f64.powi(63)).contains(&value) {
            value as i64
        } else {
            i64::MIN
        };
        Ok(wide as i32)
    }

    pub fn boolean(&self, section: &str, name: &str) -> io::Result<bool> {
        match self.value(section, name)? {
            Value::Boolean(value) => Ok(value),
            _ => Err(invalid(format!(
                "NDefines.{section}.{name} is not a boolean"
            ))),
        }
    }

    pub fn text(&self, section: &str, name: &str) -> io::Result<String> {
        match self.value(section, name)? {
            Value::String(text) => Ok(text.to_str().map_err(lua_error)?.to_owned()),
            _ => Err(invalid(format!(
                "NDefines.{section}.{name} is not a string"
            ))),
        }
    }

    pub fn numbers(&self, section: &str, name: &str) -> io::Result<Vec<f64>> {
        let refuse = |what: &str| invalid(format!("NDefines.{section}.{name} {what}"));
        let Value::Table(table) = self.value(section, name)? else {
            return Err(refuse("is not a table"));
        };
        table
            .pairs::<Value, Value>()
            .map(|pair| {
                number(pair.map_err(lua_error)?.1).ok_or_else(|| refuse("holds a non-number"))
            })
            .collect()
    }

    pub fn number_lists(&self, section: &str, name: &str) -> io::Result<Vec<Vec<f64>>> {
        let refuse = |what: &str| invalid(format!("NDefines.{section}.{name} {what}"));
        let Value::Table(table) = self.value(section, name)? else {
            return Err(refuse("is not a table"));
        };
        table
            .pairs::<Value, Value>()
            .map(|pair| {
                let Value::Table(inner) = pair.map_err(lua_error)?.1 else {
                    return Err(refuse("holds a non-table"));
                };
                inner
                    .pairs::<Value, Value>()
                    .map(|pair| {
                        number(pair.map_err(lua_error)?.1)
                            .ok_or_else(|| refuse("holds a non-number"))
                    })
                    .collect()
            })
            .collect()
    }

    pub fn strings(&self, section: &str, name: &str) -> io::Result<Vec<String>> {
        let refuse = |what: &str| invalid(format!("NDefines.{section}.{name} {what}"));
        let Value::Table(table) = self.value(section, name)? else {
            return Err(refuse("is not a table"));
        };
        table
            .pairs::<Value, Value>()
            .map(|pair| match pair.map_err(lua_error)?.1 {
                Value::String(text) => Ok(text.to_str().map_err(lua_error)?.to_owned()),
                _ => Err(refuse("holds a non-string")),
            })
            .collect()
    }

    pub fn fixed5_array(&self, section: &str, name: &str) -> io::Result<Vec<i64>> {
        self.numbers(section, name)?
            .into_iter()
            .map(|value| {
                fixed5(value)
                    .map_err(|error| invalid(format!("NDefines.{section}.{name}: {error}")))
            })
            .collect()
    }

    pub fn fixed5(&self, section: &str, name: &str) -> io::Result<i64> {
        fixed5(self.number(section, name)?)
            .map_err(|error| invalid(format!("NDefines.{section}.{name}: {error}")))
    }
}

fn number(value: Value) -> Option<f64> {
    match value {
        Value::Number(value) => Some(value),
        Value::Integer(value) => Some(value as f64),
        _ => None,
    }
}

fn lua_error(error: mlua::Error) -> io::Error {
    invalid(error.to_string())
}

pub const GRAPHICS_SECTIONS: [&str; 6] = [
    "NGraphics",
    "NAirGfx",
    "NInterface",
    "NFrontend",
    "NMapMode",
    "NMapIcons",
];

impl Defines {
    pub fn graphics_for(&self, layers: &ResolvedLayers) -> Defines {
        let Some(stack) = layers.presentation_defines() else {
            return self.clone();
        };
        match read(stack).and_then(|shown| self.with_graphics_of(&shown)) {
            Ok((graphics, kept)) => {
                for why in kept {
                    layers.note_overlay_refusal(format!(
                        "graphics define {why}, from {}",
                        stack.visual_mod_names().join(", ")
                    ));
                }
                graphics
            }
            Err(error) => {
                layers.note_overlay_refusal(format!(
                    "graphics defines of {} ({error})",
                    stack.visual_mod_names().join(", ")
                ));
                self.clone()
            }
        }
    }

    pub fn with_graphics_of(&self, shown: &Defines) -> io::Result<(Defines, Vec<String>)> {
        let mut graphics = Defines::new()?;
        let lua = &graphics.lua;
        let root = |defines: &Defines| -> io::Result<Option<Table>> {
            match defines
                .lua
                .globals()
                .raw_get::<Value>("NDefines")
                .map_err(lua_error)?
            {
                Value::Table(table) => Ok(Some(table)),
                _ => Ok(None),
            }
        };
        let section = |table: &Option<Table>, name: &str| -> io::Result<Option<Table>> {
            match table {
                Some(table) => match table.raw_get::<Value>(name).map_err(lua_error)? {
                    Value::Table(section) => Ok(Some(section)),
                    _ => Ok(None),
                },
                None => Ok(None),
            }
        };
        let field = |table: &Option<Table>, name: &str| -> io::Result<Value> {
            match table {
                Some(table) => table.raw_get::<Value>(name).map_err(lua_error),
                None => Ok(Value::Nil),
            }
        };
        let (recorded, mods) = (root(self)?, root(shown)?);
        let into = lua.create_table().map_err(lua_error)?;
        let mut from_recording = Copier::new(lua);
        let mut graphics_from_recording = Copier::new(lua);
        let mut from_mods = Copier::new(lua);
        let mut in_recording = Copier::new(&self.lua);
        let mut in_mods = Copier::new(&shown.lua);
        if let Some(recorded) = &recorded {
            for pair in recorded.pairs::<Value, Value>() {
                let (key, value) = pair.map_err(lua_error)?;
                let copier = if graphics_section(&key) {
                    &mut graphics_from_recording
                } else {
                    &mut from_recording
                };
                into.raw_set(copier.copy(key)?, copier.copy(value)?)
                    .map_err(lua_error)?;
            }
        }
        let mut kept = Vec::new();
        for name in GRAPHICS_SECTIONS {
            let ours = section(&recorded, name)?;
            let theirs = match field(&mods, name)? {
                Value::Table(theirs) => theirs,
                Value::Nil if ours.is_none() => continue,
                other => {
                    kept.push(format!(
                        "NDefines.{name} ({})",
                        change(&other, &field(&recorded, name)?)
                    ));
                    continue;
                }
            };
            let target = match into.raw_get::<Value>(name).map_err(lua_error)? {
                Value::Table(table) => table,
                _ => {
                    let table = lua.create_table().map_err(lua_error)?;
                    into.raw_set(name, table.clone()).map_err(lua_error)?;
                    table
                }
            };
            for pair in theirs.pairs::<Value, Value>() {
                let (key, value) = pair.map_err(lua_error)?;
                let recorded_value = match &ours {
                    Some(ours) => ours
                        .raw_get::<Value>(in_recording.copy(key.clone())?)
                        .map_err(lua_error)?,
                    None => Value::Nil,
                };
                if !recorded_value.is_nil() && kind(&recorded_value) != kind(&value) {
                    kept.push(format!(
                        "NDefines.{name}.{} ({})",
                        key_text(&key),
                        change(&value, &recorded_value)
                    ));
                    continue;
                }
                target
                    .raw_set(from_mods.copy(key)?, from_mods.copy(value)?)
                    .map_err(lua_error)?;
            }
            if let Some(ours) = &ours {
                for pair in ours.pairs::<Value, Value>() {
                    let (key, value) = pair.map_err(lua_error)?;
                    let theirs_value = theirs
                        .raw_get::<Value>(in_mods.copy(key.clone())?)
                        .map_err(lua_error)?;
                    if theirs_value.is_nil() {
                        kept.push(format!(
                            "NDefines.{name}.{} ({})",
                            key_text(&key),
                            change(&Value::Nil, &value)
                        ));
                    }
                }
            }
        }
        lua.globals().raw_set("NDefines", into).map_err(lua_error)?;
        graphics.sealed = true;
        kept.sort();
        Ok((graphics, kept))
    }
}

fn graphics_section(key: &Value) -> bool {
    matches!(key, Value::String(name) if GRAPHICS_SECTIONS
        .iter()
        .any(|section| &*name.as_bytes() == section.as_bytes()))
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Integer(_) | Value::Number(_) => "number",
        other => other.type_name(),
    }
}

fn key_text(key: &Value) -> String {
    match key {
        Value::String(text) => String::from_utf8_lossy(&text.as_bytes()).into_owned(),
        Value::Integer(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        other => other.type_name().to_owned(),
    }
}

fn change(theirs: &Value, ours: &Value) -> String {
    if theirs.is_nil() {
        format!("deleted where the recording has {}", kind(ours))
    } else {
        format!("{} where the recording has {}", kind(theirs), kind(ours))
    }
}

struct Copier<'a> {
    into: &'a Lua,
    copied: std::collections::HashMap<*const std::ffi::c_void, Table>,
}

impl<'a> Copier<'a> {
    fn new(into: &'a Lua) -> Self {
        Self {
            into,
            copied: std::collections::HashMap::new(),
        }
    }

    fn copy(&mut self, value: Value) -> io::Result<Value> {
        Ok(match value {
            Value::Nil | Value::Boolean(_) | Value::Integer(_) | Value::Number(_) => value,
            Value::String(text) => Value::String(
                self.into
                    .create_string(&*text.as_bytes())
                    .map_err(lua_error)?,
            ),
            Value::Table(table) => {
                let id = table.to_pointer();
                if let Some(copied) = self.copied.get(&id) {
                    return Ok(Value::Table(copied.clone()));
                }
                let copied = self.into.create_table().map_err(lua_error)?;
                self.copied.insert(id, copied.clone());
                for pair in table.pairs::<Value, Value>() {
                    let (key, item) = pair.map_err(lua_error)?;
                    let (key, item) = (self.copy(key)?, self.copy(item)?);
                    copied.raw_set(key, item).map_err(lua_error)?;
                }
                Value::Table(copied)
            }
            other => {
                return Err(invalid(format!(
                    "unsupported NDefines {}",
                    other.type_name()
                )));
            }
        })
    }
}

pub fn fixed5(value: f64) -> io::Result<i64> {
    let value = value as f32;
    let sign = if value >= 0.0 { 1.0_f32 } else { 0.0 };
    let rounded = (sign + value * 100_000.0_f32) - 0.5_f32;
    if !rounded.is_finite() || rounded < i64::MIN as f32 || rounded >= -(i64::MIN as f32) {
        return Err(invalid("numeric define is outside signed fixed5 range"));
    }
    Ok(rounded as i64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn graphics_defines_take_the_mods_presentation_values_only() {
        let recorded = super::Defines::new().unwrap();
        recorded
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { UNITS_ICONS_DISTANCE_CUTOFF = 1200, \
                  FRIEND_COLOR = { 0, 1, 0 } }, NMilitary = { BASE_SPEED = 4 } }",
            )
            .unwrap();
        let shown = super::Defines::new().unwrap();
        shown
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { UNITS_ICONS_DISTANCE_CUTOFF = 600, \
                  FRIEND_COLOR = \"green\", MOD_ONLY = 3 }, NMilitary = { BASE_SPEED = 9 }, \
                  NMapIcons = { TOP_MAP_ICON = 7 } }",
            )
            .unwrap();
        let (graphics, kept) = recorded.with_graphics_of(&shown).unwrap();
        assert_eq!(
            graphics
                .number("NGraphics", "UNITS_ICONS_DISTANCE_CUTOFF")
                .unwrap(),
            600.0
        );
        assert_eq!(graphics.number("NGraphics", "MOD_ONLY").unwrap(), 3.0);
        assert_eq!(graphics.number("NMapIcons", "TOP_MAP_ICON").unwrap(), 7.0);
        assert_eq!(graphics.number("NMilitary", "BASE_SPEED").unwrap(), 4.0);
        assert_eq!(
            graphics.numbers("NGraphics", "FRIEND_COLOR").unwrap(),
            [0.0, 1.0, 0.0]
        );
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(
            kept[0].starts_with(
                "NDefines.NGraphics.FRIEND_COLOR (string where the recording has table"
            ),
            "{kept:?}"
        );
        assert!(graphics.load("test", b"NDefines.NGraphics.X = 1").is_err());
    }

    #[test]
    fn retyped_and_deleted_graphics_defines_keep_the_recorded_values_by_name() {
        let recorded = super::Defines::new().unwrap();
        recorded
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { A = 1, B = 2 }, NMapIcons = { C = 3 }, \
                  NFrontend = { D = 4 } }",
            )
            .unwrap();
        let shown = super::Defines::new().unwrap();
        shown
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { A = 1, B = 2 }, NMapIcons = { C = 3 }, \
                  NFrontend = { D = 4 } } \
                  NDefines.NGraphics.A = 5 NDefines.NGraphics.B = nil \
                  NDefines.NMapIcons = 6 NDefines.NFrontend = nil",
            )
            .unwrap();
        let (graphics, kept) = recorded.with_graphics_of(&shown).unwrap();
        assert_eq!(graphics.number("NGraphics", "A").unwrap(), 5.0);
        assert_eq!(graphics.number("NGraphics", "B").unwrap(), 2.0);
        assert_eq!(graphics.number("NMapIcons", "C").unwrap(), 3.0);
        assert_eq!(graphics.number("NFrontend", "D").unwrap(), 4.0);
        assert_eq!(
            kept,
            [
                "NDefines.NFrontend (deleted where the recording has table)",
                "NDefines.NGraphics.B (deleted where the recording has number)",
                "NDefines.NMapIcons (number where the recording has table)",
            ]
        );
    }

    #[test]
    fn a_define_table_that_holds_itself_is_copied_as_a_cycle() {
        let recorded = super::Defines::new().unwrap();
        recorded
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { A = 1 }, NMilitary = { SPEED = 4 } } \
                  NDefines.NMilitary.again = NDefines.NMilitary",
            )
            .unwrap();
        let shown = super::Defines::new().unwrap();
        shown
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { A = 2 } } \
                  NDefines.NGraphics.extra = NDefines.NGraphics",
            )
            .unwrap();
        let (graphics, kept) = recorded.with_graphics_of(&shown).unwrap();
        assert!(kept.is_empty(), "{kept:?}");
        assert_eq!(graphics.number("NGraphics", "A").unwrap(), 2.0);
        assert_eq!(graphics.number("NMilitary", "SPEED").unwrap(), 4.0);
        assert!(graphics.number("NGraphics", "extra").is_err());
    }

    #[test]
    fn a_simulation_section_sharing_a_graphics_table_stays_recorded() {
        let recorded = super::Defines::new().unwrap();
        recorded
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { A = 1 } } NDefines.NMilitary = NDefines.NGraphics",
            )
            .unwrap();
        let shown = super::Defines::new().unwrap();
        shown
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = { NGraphics = { A = 2 } }",
            )
            .unwrap();
        let (graphics, kept) = recorded.with_graphics_of(&shown).unwrap();
        assert!(kept.is_empty(), "{kept:?}");
        assert_eq!(graphics.number("NGraphics", "A").unwrap(), 2.0);
        assert_eq!(graphics.number("NMilitary", "A").unwrap(), 1.0);
    }

    #[test]
    fn a_caught_instruction_limit_still_ends_the_load() {
        let defines = super::Defines::new().unwrap();
        let error = defines
            .load(
                "common/defines/probe.lua",
                b"NDefines = { Probe = { Done = 0 } } \
                  while true do pcall(function() while true do end end) end",
            )
            .unwrap_err();
        assert!(error.to_string().contains("instruction limit"), "{error}");
    }

    #[test]
    fn number_lists_read_each_inner_table_and_refuse_a_flat_one() {
        let defines = super::Defines::new().unwrap();
        defines
            .load(
                "test",
                b"NDefines={S={Colors={{0.1,0.9,0.2,1.0},{1,0.5,0,1}},Flat={1,2}}}",
            )
            .unwrap();
        assert_eq!(
            defines.number_lists("S", "Colors").unwrap(),
            [vec![0.1, 0.9, 0.2, 1.0], vec![1.0, 0.5, 0.0, 1.0]]
        );
        assert!(defines.numbers("S", "Colors").is_err());
        assert!(defines.number_lists("S", "Flat").is_err());
    }

    #[test]
    fn resolved_defines_execute_once_and_share_an_immutable_state() {
        let root = std::env::temp_dir().join(format!("shared-defines-{}", std::process::id()));
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        let path = root.join("common/defines/test.lua");
        std::fs::write(&path, "NDefines={Synthetic={Value=7}}").unwrap();
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let first = super::read(&layers).unwrap();
        let second = super::read(&layers).unwrap();
        assert_eq!(first.number("Synthetic", "Value").unwrap(), 7.0);
        assert_eq!(second.number("Synthetic", "Value").unwrap(), 7.0);
        assert!(second.load("test", b"NDefines.Synthetic.Value=9").is_err());
        assert_eq!(first.number("Synthetic", "Value").unwrap(), 7.0);
        assert!(
            super::read(&content_reader_layers::ResolvedLayers::default())
                .unwrap()
                .number("Synthetic", "Value")
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_unscoped_defines_reads_share_the_winning_lua_state() {
        use std::sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let root = std::env::temp_dir().join(format!(
            "concurrent-unscoped-defines-{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        std::fs::write(
            root.join("common/defines/test.lua"),
            b"NDefines = {} NDefines.Synthetic = { Id = tostring(NDefines) }",
        )
        .unwrap();
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let arrivals = Arc::new(AtomicUsize::new(0));
        let wait_barrier = Arc::clone(&barrier);
        let wait_arrivals = Arc::clone(&arrivals);
        let traced = layers.with_read_trace(Arc::new(move |_| {
            if wait_arrivals.fetch_add(1, Ordering::AcqRel) < 2 {
                wait_barrier.wait();
            }
        }));
        let (first, second) = std::thread::scope(|scope| {
            let a = scope.spawn(|| super::read(&traced).unwrap());
            let b = scope.spawn(|| super::read(&traced).unwrap());
            (a.join().unwrap(), b.join().unwrap())
        });
        let first_id = first.text("Synthetic", "Id").unwrap();
        assert!(first_id.starts_with("table:"), "{first_id}");
        assert_eq!(first_id, second.text("Synthetic", "Id").unwrap());
        let third = super::read(&traced).unwrap();
        assert_eq!(first_id, third.text("Synthetic", "Id").unwrap());
        assert_eq!(arrivals.load(Ordering::Acquire), 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn d16_shared_defines_repeat_main_verified_reads_before_reusing_the_state() {
        use std::fs;
        let root =
            std::env::temp_dir().join(format!("shared-defines-reads-{}", std::process::id()));
        fs::create_dir_all(root.join("common/defines/a")).unwrap();
        let first = root.join("common/defines/z.lua");
        let later = root.join("common/defines/a/a.lua");
        let source = b"NDefines={Synthetic={Value=7}}";
        fs::write(&first, source).unwrap();
        fs::write(&later, b"NDefines.Synthetic.Value=8").unwrap();
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        assert_eq!(
            super::read(&layers)
                .unwrap()
                .number("Synthetic", "Value")
                .unwrap(),
            8.0
        );
        fs::write(&first, b"NDefines={Synthetic={Value=9}}").unwrap();
        fs::write(&later, b"NDefines.Synthetic.Value=0").unwrap();
        let error = super::read(&layers).unwrap_err().to_string();
        assert!(
            error.contains("changed since verification") && error.contains("common/defines/z.lua"),
            "{error}"
        );
        fs::write(&first, source).unwrap();
        fs::remove_file(&later).unwrap();
        let error = super::read(&layers).unwrap_err().to_string();
        assert!(
            error.contains("changed since verification")
                && error.contains("common/defines/a/a.lua"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lua_value_dump_sorts_nested_tables_without_addresses() {
        let a = super::Defines::new().unwrap();
        let b = super::Defines::new().unwrap();
        a.load(
            "test",
            b"NDefines = { S = { z = 1.25, a = { [2] = -0.0, [1] = 'x' } } }",
        )
        .unwrap();
        b.load(
            "test",
            b"NDefines = { S = { a = { [1] = 'x', [2] = -0.0 }, z = 1.25 } }",
        )
        .unwrap();
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        a.load("test", b"NDefines.S.empty = ''").unwrap();
        b.load("test", b"NDefines.S.empty = {}").unwrap();
        assert_ne!(format!("{a:?}"), format!("{b:?}"));
    }

    use super::*;

    fn defines(sources: &[&str]) -> io::Result<Defines> {
        let defines = Defines::new()?;
        for source in sources {
            defines.run("test", source.as_bytes())?;
        }
        Ok(defines)
    }

    const SCRIPT_BASE: &str = r#"
        NDefines = { NSession = { QUANTA = 4 } }
        ScenarioGroups = { NVisualTest = {
            THRESHOLD = 72, -- trailing comment
            ASSET = "gfx/fixtures/stripe#sample.tga",
            TINT = {3.0/8, 5.0/16, 7.0/32, 0.5},
        } }
        local function install_groups(groups)
            for group_name, group_values in pairs(groups) do
                NDefines[group_name] = group_values
            end
        end
        install_groups(ScenarioGroups)"#;

    #[test]
    fn later_scripts_override_through_shared_tables_and_evaluate_lua() {
        let values = defines(&[
            SCRIPT_BASE,
            "NDefines.NVisualTest.THRESHOLD = 96;",
            "ScenarioGroups.NVisualTest.THRESHOLD = 1.25e2",
            "local unit = NDefines.NSession.QUANTA\nNDefines.NSession.SPAN = (unit + 5) * 7",
            "NDefines.NSession.SAMPLES = { 0.25, 0.8, 2.000015, -3.5 }",
            "for increment = 1, 4 do NDefines.NSession.QUANTA = NDefines.NSession.QUANTA + increment end",
        ])
        .unwrap();
        assert_eq!(values.number("NVisualTest", "THRESHOLD").unwrap(), 125.0);
        assert_eq!(values.number("NSession", "SPAN").unwrap(), 63.0);
        assert_eq!(values.number("NSession", "QUANTA").unwrap(), 14.0);
        assert!(values.number("NVisualTest", "ASSET").is_err());
        assert_eq!(values.int32("NSession", "SPAN").unwrap(), 63);
        let wide =
            defines(&["NDefines = { N = { A = -2.9, B = 4294967297, C = 1e300 } }"]).unwrap();
        assert_eq!(
            ["A", "B", "C"].map(|name| wide.int32("N", name).unwrap()),
            [-2, 1, 0]
        );
        assert_eq!(
            values.text("NVisualTest", "ASSET").unwrap(),
            "gfx/fixtures/stripe#sample.tga"
        );
        assert!(values.text("NVisualTest", "THRESHOLD").is_err());
        assert_eq!(
            values.fixed5_array("NSession", "SAMPLES").unwrap(),
            [25_000, 80_000, 200_002, -350_000]
        );
        assert!(values.fixed5_array("NVisualTest", "THRESHOLD").is_err());
        assert!(values.number("NVisualTest", "ABSENT").is_err());
        assert!(values.number("NAbsent", "THRESHOLD").is_err());
    }

    #[test]
    fn script_errors_host_access_and_runaway_loops_refuse_the_reader() {
        for source in [
            "NDefines.NVisualTest.THRESHOLD = 1 + missing",
            "NDefines.NVisualTest.THRESHOLD = os.time()",
            "NDefines.NVisualTest.THRESHOLD = io.read()",
            "dofile('x.lua')",
            "require('x')",
            "while true do end",
            "local s = string.rep('x', 2^30)",
        ] {
            assert!(defines(&[SCRIPT_BASE, source]).is_err(), "{source}");
        }
    }

    #[test]
    fn scripts_load_by_byte_name_with_files_before_subdirectories() {
        let listed = [
            "common/defines/zz_mod.lua",
            "common/defines/sub/a.lua",
            "common/defines/00_defines.lua",
            "common/defines/Z_upper.lua",
            "common/defines/readme.txt",
            "common/defines/sub/deeper/b.lua",
            "common/defines/sub/B.lua",
            "common/other/x.lua",
        ]
        .map(ScriptPath::from);
        let order = load_order("common/defines/", "lua", &listed);
        assert_eq!(
            order,
            [
                "common/defines/00_defines.lua",
                "common/defines/Z_upper.lua",
                "common/defines/zz_mod.lua",
                "common/defines/sub/B.lua",
                "common/defines/sub/a.lua",
                "common/defines/sub/deeper/b.lua",
            ]
        );
    }

    #[test]
    fn fixed5_keeps_float_steps() {
        assert_eq!(fixed5(0.000005).unwrap(), 1);
        assert_eq!(fixed5(-0.000005).unwrap(), -1);
        assert_eq!(fixed5(0.0000049).unwrap(), 0);
        assert_eq!(fixed5(1000.00001).unwrap(), 100_000_000);
        assert!(fixed5(f64::NAN).is_err());
        assert!(fixed5(f64::MAX).is_err());
    }

    #[test]
    fn script_error_marker_marks_script_loads_and_not_unreadable_or_changed_files() {
        use std::fs;

        let root = std::env::temp_dir().join(format!(
            "define-script-marker-{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("common/defines")).unwrap();
        let script = root.join("common/defines/bad.lua");
        fs::write(&script, b"error('marker probe')").unwrap();
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let unscoped = super::read(&layers).unwrap_err();
        assert_eq!(unscoped.kind(), io::ErrorKind::InvalidData);
        assert!(unscoped.to_string().contains("defines unavailable in common/defines/bad.lua:"), "{unscoped}");
        assert!(super::is_script_error(&unscoped));
        let scoped_layers =
            content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let _generation = scoped_layers.enter_generation();
        let scoped = super::read(&scoped_layers).unwrap_err();
        assert_eq!(scoped.kind(), io::ErrorKind::InvalidData);
        assert_eq!(scoped.to_string(), unscoped.to_string());
        assert!(super::is_script_error(&scoped));

        fs::write(&script, b"NDefines={Synthetic={Value=1}}").unwrap();
        let changed = super::read(&layers).unwrap_err();
        assert_eq!(changed.kind(), io::ErrorKind::InvalidData);
        assert!(changed.to_string().contains("changed since verification"), "{changed}");
        assert!(!super::is_script_error(&changed));

        fs::write(&script, b"error('marker probe')").unwrap();
        let unreadable = {
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                let _locked = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&script)
                    .unwrap();
                super::read(&layers).unwrap_err()
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&script, fs::Permissions::from_mode(0o000)).unwrap();
                let error = super::read(&layers).unwrap_err();
                fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
                error
            }
        };
        assert!(!super::is_script_error(&unreadable), "{unreadable}");
        assert_ne!(unreadable.kind(), io::ErrorKind::InvalidData, "{unreadable}");

        fs::remove_dir_all(root).unwrap();
    }
}
