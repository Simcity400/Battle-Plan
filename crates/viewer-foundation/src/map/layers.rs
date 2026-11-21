
use std::io;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Omissions {
    pub counters: bool,
    pub labels: bool,
    pub trees: bool,
    pub borders: bool,
    pub gradient: bool,
    pub rivers: bool,
    pub weather: bool,
    pub orders: bool,
    pub models: bool,
    pub sprawl: bool,
    pub buildings: bool,
    pub rails: bool,
}

impl Omissions {
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
