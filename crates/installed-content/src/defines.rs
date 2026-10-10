//! `NDefines` from `common/defines`, evaluated in stack load order.
//!
//! All scripts in the merged tree run in one Lua 5.1 state, whose numbers
//! are doubles. Typed readers select `NDefines.<section>.<name>`. Vendored
//! PUC Lua 5.1 evaluates mod arithmetic, locals, functions and loops. A
//! script error refuses the reader, as the game abandons the load.
//!
//! The sandbox opens `base`, `string`, `table` and `math` only, removes the
//! file loaders from `base` and bounds the instruction count. Scripts that
//! need `io`, `os`, `package` or `debug` are refused instead of receiving
//! access to this machine.

use crate::ResolvedLayers;
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Hook calls of [`HOOK_INTERVAL`] instructions before the scripts are
/// stopped: 10^8 instructions, far past the shipped scripts' few thousand.
const HOOK_INTERVAL: u32 = 1_000_000;
const HOOK_LIMIT: u32 = 100;

/// Memory the define state may allocate; a runaway string or table fails
/// the load instead of exhausting the process.
const MEMORY_LIMIT: usize = 256 << 20;

/// The final `NDefines` of the recorded content stack.
pub struct Defines {
    lua: Lua,
}

// Dump the Lua values, never Lua handles (whose Debug contains addresses).
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

/// Runs every define script of the stack in load order.
pub fn read(layers: &ResolvedLayers) -> io::Result<Defines> {
    let defines = Defines::new()?;
    for path in script_paths(layers) {
        defines.load(&path, &layers.read(&path)?)?;
    }
    Ok(defines)
}

/// Define scripts in recursive load order: each directory's names ending
/// in `lua`, case-sensitive and in byte order, then its subdirectories in
/// byte order, depth first. A path repeated across layers uses the winning
/// layer; paths differing only in case remain separate tree entries.
pub(crate) fn script_paths(layers: &ResolvedLayers) -> Vec<String> {
    load_order(
        "common/defines/",
        "lua",
        layers
            .listed_paths("common/defines/")
            .iter()
            .map(String::as_str),
    )
}

/// Names under `root` ending in `filter`, with files before subdirectories
/// and each level in byte order, depth first.
pub fn load_order<'a>(
    root: &str,
    filter: &str,
    paths: impl Iterator<Item = &'a str>,
) -> Vec<String> {
    load_order_paths(root.as_bytes(), filter.as_bytes(), paths.map(str::as_bytes))
        .into_iter()
        .map(|path| String::from_utf8(path).expect("text paths order as text"))
        .collect()
}

/// [`load_order`] over layer paths' bytes, which the enumeration compares and
/// returns as they are ([`crate::ResolvedLayers::listed_layer_paths`]).
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
    // Files sort before the subdirectories of the same directory, and each
    // level compares names bytewise.
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
    /// The sandboxed define state before any script has run; [`read`] then
    /// [`Defines::load`]s the stack's scripts into it.
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
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INTERVAL),
            move |_, _| {
                if hooks.fetch_add(1, Ordering::Relaxed) >= HOOK_LIMIT {
                    Err(mlua::Error::runtime(
                        "define script exceeded its instruction limit",
                    ))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )
        .map_err(lua_error)?;
        Ok(Self { lua })
    }

    /// Runs `source` as the next script of the load order; an error refuses the reader.
    pub fn load(&self, path: &str, source: &[u8]) -> io::Result<()> {
        self.run(path, source)
            .map_err(|error| invalid(format!("defines unavailable in {path}: {error}")))
    }

    fn run(&self, name: &str, source: &[u8]) -> io::Result<()> {
        self.lua
            .load(source)
            .set_name(name)
            .exec()
            .map_err(lua_error)
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

    /// `NDefines.<section>.<name>`, which must be a Lua number.
    pub fn number(&self, section: &str, name: &str) -> io::Result<f64> {
        number(self.value(section, name)?)
            .ok_or_else(|| invalid(format!("NDefines.{section}.{name} is not a number")))
    }

    /// `NDefines.<section>.<name>` as a signed integer: truncate a Lua number
    /// to 64 bits, using `i64::MIN` for out-of-range values, then take the low
    /// 32 bits. Missing or non-number values store 0.
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

    /// `NDefines.<section>.<name>`, which must be a Lua boolean.
    pub fn boolean(&self, section: &str, name: &str) -> io::Result<bool> {
        match self.value(section, name)? {
            Value::Boolean(value) => Ok(value),
            _ => Err(invalid(format!(
                "NDefines.{section}.{name} is not a boolean"
            ))),
        }
    }

    /// `NDefines.<section>.<name>`, which must be a Lua string.
    pub fn text(&self, section: &str, name: &str) -> io::Result<String> {
        match self.value(section, name)? {
            Value::String(text) => Ok(text.to_str().map_err(lua_error)?.to_owned()),
            _ => Err(invalid(format!(
                "NDefines.{section}.{name} is not a string"
            ))),
        }
    }

    /// Table define numbers in Lua iteration order.
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

    /// String-vector define entries in Lua iteration order.
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

    /// A table define converted entry by entry to fixed5.
    pub fn fixed5_array(&self, section: &str, name: &str) -> io::Result<Vec<i64>> {
        self.numbers(section, name)?
            .into_iter()
            .map(|value| {
                fixed5(value)
                    .map_err(|error| invalid(format!("NDefines.{section}.{name}: {error}")))
            })
            .collect()
    }

    /// `NDefines.<section>.<name>` through the native fixed-point getter.
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

/// Numeric define conversion narrows the Lua number to f32 before separate
/// multiply, add, subtract and truncating conversion steps. Rounding in
/// f64 or with `round()` differs at half steps and large values.
pub fn fixed5(value: f64) -> io::Result<i64> {
    let value = value as f32;
    let sign = if value >= 0.0 { 1.0_f32 } else { 0.0 };
    let rounded = (sign + value * 100_000.0_f32) - 0.5_f32;
    // Truncating conversion would return its indefinite sentinel here.
    if !rounded.is_finite() || rounded < i64::MIN as f32 || rounded >= -(i64::MIN as f32) {
        return Err(invalid("numeric define is outside signed fixed5 range"));
    }
    Ok(rounded as i64)
}

#[cfg(test)]
mod tests {
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
        // The copy shares NVisualTest, so the second alias's write is visible.
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
        let order = load_order(
            "common/defines/",
            "lua",
            [
                "common/defines/zz_mod.lua",
                "common/defines/sub/a.lua",
                "common/defines/00_defines.lua",
                "common/defines/Z_upper.lua",
                "common/defines/readme.txt",
                "common/defines/sub/deeper/b.lua",
                "common/defines/sub/B.lua",
                "common/other/x.lua",
            ]
            .into_iter(),
        );
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
        // The f32 narrowing loses this fraction, as the native getter does.
        assert_eq!(fixed5(1000.00001).unwrap(), 100_000_000);
        assert!(fixed5(f64::NAN).is_err());
        assert!(fixed5(f64::MAX).is_err());
    }
}
