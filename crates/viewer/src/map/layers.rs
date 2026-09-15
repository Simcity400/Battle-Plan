//! Dump-only pass omissions for paired screenshot attribution.

use std::io;

/// Every flag omits the named layer. The default preserves the full frame.
/// Omitted world objects are never uploaded, so they cast no shadows either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Omissions {
    /// Unit, base and combat sprites; victory-point sprites belong to labels.
    pub counters: bool,
    /// Country/state names and victory-point/capital text and sprites.
    pub labels: bool,
    pub trees: bool,
    /// Ordinary and demilitarised strips, including the strait ribbons.
    pub borders: bool,
    /// Political gradient and interior fill, independently of border strips.
    pub gradient: bool,
    pub rivers: bool,
    /// Weather particles only; terrain snow/mud and its winter grade remain.
    pub weather: bool,
    pub orders: bool,
    /// Unit figures; buildings, city sprawl and railway objects are separate.
    pub models: bool,
    pub sprawl: bool,
    pub buildings: bool,
    /// Rail strips, supply trains and railway bridges.
    pub rails: bool,
}

impl Omissions {
    /// Parse the comma-separated list of passes to omit, rejecting typos.
    pub fn parse(value: &str) -> io::Result<Self> {
        let mut out = Self::default();
        for name in value.split(',').map(str::trim) {
            *match name {
                "counters" => &mut out.counters,
                "labels" => &mut out.labels,
                "trees" => &mut out.trees,
                "borders" => &mut out.borders,
                "gradient" => &mut out.gradient,
                "rivers" => &mut out.rivers,
                "weather" => &mut out.weather,
                "orders" => &mut out.orders,
                "models" => &mut out.models,
                "sprawl" => &mut out.sprawl,
                "buildings" => &mut out.buildings,
                "rails" => &mut out.rails,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown omitted layer `{name}`"),
                    ));
                }
            } = true;
        }
        Ok(out)
    }
}
