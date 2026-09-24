//! `NDefines` from `common/defines`, evaluated as the game evaluates it.
//!
//! `LoadGameDefines` (`14073dda0`) runs every define script of the merged
//! `common/defines` tree in one PUC Lua 5.1.1 state (`14205a590`, banner at
//! `142b1b87e`; numbers are doubles), then its readers take
//! `NDefines.<section>.<name>`, requiring a Lua number (`14205d560`). This
//! runs the same scripts in the same order on vendored PUC Lua 5.1, so mod
//! arithmetic, locals, functions and loops evaluate natively. A script error
//! refuses the whole reader, as the game abandons the load.
//!
//! The sandbox opens `base`, `string`, `table` and `math` only, removes the
//! file loaders from `base` and bounds the instruction count. The game also
//! opens `io`, `os`, `package` and `debug`; a script that needs them is refused
//! rather than run with access to this machine.

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

/// Runs every define script of the stack in load order.
pub fn read(layers: &ResolvedLayers) -> io::Result<Defines> {
    let defines = Defines::new()?;
    for path in script_paths(layers) {
        defines
            .run(&path, &layers.read(&path)?)
            .map_err(|error| invalid(format!("defines unavailable in {path}: {error}")))?;
    }
    Ok(defines)
}

/// The define scripts in native load order (`1424c5170`, `1424c5ef0`): per
/// directory, the file names ending in `lua` in byte order (case-sensitive),
/// then each subdirectory in byte order, depth first. A name repeated across
/// layers is one entry read from the winning layer; names that differ only in
/// case are separate entries, as the tree keys them.
pub(crate) fn script_paths(layers: &ResolvedLayers) -> Vec<String> {
    load_order(
        layers
            .listed_paths("common/defines/")
            .iter()
            .map(String::as_str),
    )
}

fn load_order<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<String> {
    const ROOT: &str = "common/defines/";
    let mut paths: Vec<String> = paths
        .filter(|path| {
            path.len() > ROOT.len()
                && path
                    .get(..ROOT.len())
                    .is_some_and(|root| root.eq_ignore_ascii_case(ROOT))
                && path.ends_with("lua")
        })
        .map(|path| path[ROOT.len()..].to_owned())
        .collect();
    // Files sort before the subdirectories of the same directory, and each
    // level compares names bytewise.
    paths.sort_by(|a, b| {
        let (a, b) = (
            a.split('/').collect::<Vec<_>>(),
            b.split('/').collect::<Vec<_>>(),
        );
        for (index, (x, y)) in a.iter().zip(&b).enumerate() {
            let (x_file, y_file) = (index + 1 == a.len(), index + 1 == b.len());
            if x != y || x_file != y_file {
                return y_file
                    .cmp(&x_file)
                    .then_with(|| x.as_bytes().cmp(y.as_bytes()));
            }
        }
        a.len().cmp(&b.len())
    });
    paths
        .into_iter()
        .map(|path| format!("{ROOT}{path}"))
        .collect()
}

impl Defines {
    fn new() -> io::Result<Self> {
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

    /// A table define's numbers in `lua_next` order, as the native table
    /// loaders (`142059cf0`) walk it.
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

    /// A table define through the native fixed-point array loader (`142059cf0`).
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

/// Native numeric define conversion (`14205d560`): Lua number to f32, then
/// separate MULSS, ADDSS, SUBSS and CVTTSS2SI. Rounding in f64 or with
/// `round()` differs at half steps and large values.
pub fn fixed5(value: f64) -> io::Result<i64> {
    let value = value as f32;
    let sign = if value >= 0.0 { 1.0_f32 } else { 0.0 };
    let rounded = (sign + value * 100_000.0_f32) - 0.5_f32;
    // CVTTSS2SI would return its indefinite sentinel here.
    if !rounded.is_finite() || rounded < i64::MIN as f32 || rounded >= -(i64::MIN as f32) {
        return Err(invalid("numeric define is outside signed fixed5 range"));
    }
    Ok(rounded as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defines(sources: &[&str]) -> io::Result<Defines> {
        let defines = Defines::new()?;
        for source in sources {
            defines.run("test", source.as_bytes())?;
        }
        Ok(defines)
    }

    const SHIPPED: &str = r#"
        NDefines = { NGame = { DAYS = 1 } }
        NDefines_Graphics = { NGraphics = {
            CUTOFF = 1500, -- trailing comment
            NAME = "gfx/a#b.tga",
            COLOR = {140.0/255, 131.0/255, 119.0/255, 1},
        } }
        for k,v in pairs( NDefines_Graphics ) do NDefines[k] = v end"#;

    #[test]
    fn later_scripts_override_through_shared_tables_and_evaluate_lua() {
        let values = defines(&[
            SHIPPED,
            "NDefines.NGraphics.CUTOFF = 1800;",
            "NDefines_Graphics.NGraphics.CUTOFF = 2e3",
            "local base = NDefines.NGame.DAYS\nNDefines.NGame.HOURS = (base + 6) * 24",
            "NDefines.NGame.LEVELS = { 0, 0.6, 1.000015, -2 }",
            "for i = 1, 3 do NDefines.NGame.DAYS = NDefines.NGame.DAYS + i end",
        ])
        .unwrap();
        // The copy shares NGraphics, so the second alias's write is visible.
        assert_eq!(values.number("NGraphics", "CUTOFF").unwrap(), 2000.0);
        assert_eq!(values.number("NGame", "HOURS").unwrap(), 168.0);
        assert_eq!(values.number("NGame", "DAYS").unwrap(), 7.0);
        assert!(values.number("NGraphics", "NAME").is_err());
        assert_eq!(
            values.fixed5_array("NGame", "LEVELS").unwrap(),
            [0, 60_000, 100_002, -200_000]
        );
        assert!(values.fixed5_array("NGraphics", "CUTOFF").is_err());
        assert!(values.number("NGraphics", "ABSENT").is_err());
        assert!(values.number("NAbsent", "CUTOFF").is_err());
    }

    #[test]
    fn script_errors_host_access_and_runaway_loops_refuse_the_reader() {
        for source in [
            "NDefines.NGraphics.CUTOFF = 1 + missing",
            "NDefines.NGraphics.CUTOFF = os.time()",
            "NDefines.NGraphics.CUTOFF = io.read()",
            "dofile('x.lua')",
            "require('x')",
            "while true do end",
            "local s = string.rep('x', 2^30)",
        ] {
            assert!(defines(&[SHIPPED, source]).is_err(), "{source}");
        }
    }

    #[test]
    fn scripts_load_by_byte_name_with_files_before_subdirectories() {
        let order = load_order(
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
    fn fixed5_keeps_native_float_steps() {
        assert_eq!(fixed5(0.000005).unwrap(), 1);
        assert_eq!(fixed5(-0.000005).unwrap(), -1);
        assert_eq!(fixed5(0.0000049).unwrap(), 0);
        // The f32 narrowing loses this fraction, as the native getter does.
        assert_eq!(fixed5(1000.00001).unwrap(), 100_000_000);
        assert!(fixed5(f64::NAN).is_err());
        assert!(fixed5(f64::MAX).is_err());
    }
}
