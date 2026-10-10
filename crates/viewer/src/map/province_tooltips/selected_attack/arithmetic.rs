//! Native selected-attack arithmetic over projected recorded operands.
//! Packet projection, gate checks and strait queries belong to the caller.
//! Preview results remain viewer-local.

pub const ONE: i64 = 100_000;
pub type Field<T> = Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: u32,
    pub value: i64,
}

pub fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

/// 14067C770 coastal=false; no-state factor is known0, null controller returns define.
pub fn fort_base(country: Option<i64>, state: i64, define: i64) -> i64 {
    match country {
        None => define,
        Some(country) => mul(country.wrapping_add(ONE).wrapping_add(state), define),
    }
}

pub fn river(unit: [i64; 2], leader: i64, define: i64) -> [i64; 2] {
    let adjustment = mul(define.wrapping_add(leader), ONE);
    unit.map(|value| value.wrapping_add(adjustment))
}

/// Native preview directions=1, both multipliers=ONE, coastal=false.
/// base_factor comes from EXACT14067C770 via fort_base below.
pub fn fort(unit: [i64; 2], leader: i64, healthy: i32, base_factor: i64) -> Option<[i64; 2]> {
    if healthy <= 0 {
        return None;
    }
    let levels = i64::from(healthy).wrapping_mul(ONE).max(ONE);
    let factor = mul(mul(levels, base_factor), ONE).wrapping_add(leader);
    Some(unit.map(|value| mul(value.wrapping_add(factor).min(0), ONE)))
}

pub fn weather(pair: [i64; 2], acclimatization: i64) -> [i64; 2] {
    pair.map(|value| mul(value, acclimatization))
}

pub fn night(stat: i64, define: i64, country: i64, leader: i64) -> i64 {
    stat.wrapping_add(define)
        .wrapping_add(country)
        .wrapping_add(leader)
}

pub fn has_snow(snow: i64, permanent: i64, ceiling: i64) -> bool {
    snow.wrapping_add(permanent).min(ceiling) > 0
}

/// Unknown retained route is not empty. One node uses actual current source.
pub fn retained_edge(current: u32, route: Field<&[u32]>) -> Field<Option<(u32, u32)>> {
    let route = route?;
    Ok(route.last().copied().map(|target| {
        let source = if route.len() > 1 {
            route[route.len() - 2]
        } else {
            current
        };
        (source, target)
    }))
}

/// Known disagreement proves preview unreachable despite other missing locations.
pub fn same_current(locations: &[Field<u32>]) -> Field<Option<u32>> {
    let mut first = None;
    let mut missing = None;
    for location in locations {
        match location {
            Ok(current) => match first {
                Some(first) if first != *current => return Ok(None),
                None => first = Some(*current),
                _ => {}
            },
            Err(reason) => {
                missing.get_or_insert_with(|| reason.clone());
            }
        }
    }
    if let Some(reason) = missing {
        Err(reason)
    } else {
        Ok(first)
    }
}
/// PRIVATE view-local projection, NOT a recording packet. All earlier
/// native outer gates must pass before the caller demands these fields.
/// The fort and river second values, and weather breakthrough, affect row
/// admission. Enemy-air B does not affect admission or rendered bytes.
pub struct Projection {
    pub terrain: Field<i64>,
    pub river: Field<Option<[i64; 2]>>,
    pub fort: Field<Option<[i64; 2]>>,
    pub snow: Field<Option<i64>>,
    pub weather: Field<Option<[i64; 2]>>,
    pub night: Field<Option<i64>>,
    pub enemy_air_positive: Field<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Preview {
    pub rows: Vec<Row>,
    pub refusals: Vec<(&'static str, String)>,
}

/// Exact observable projection of14129F040→140DE95A0. Seven independent
/// row slots preserve order and failure locality. No sort or dedup by kind.
pub fn rows(input: Projection) -> Preview {
    let mut out = Preview {
        rows: Vec::new(),
        refusals: Vec::new(),
    };
    let ordered = [
        (
            "terrain",
            input
                .terrain
                .map(|value| (value != 0).then_some(Row { kind: 27, value })),
        ),
        (
            "river",
            input.river.map(|pair| {
                pair.filter(|p| p[0] != 0 || p[1] != 0).map(|p| Row {
                    kind: 10,
                    value: p[0],
                })
            }),
        ),
        (
            "fort",
            input.fort.map(|pair| {
                pair.filter(|p| p[0] != 0 || p[1] != 0).map(|p| Row {
                    kind: 9,
                    value: p[0],
                })
            }),
        ),
        (
            "snow",
            input.snow.map(|value| {
                value
                    .filter(|v| *v != 0)
                    .map(|value| Row { kind: 28, value })
            }),
        ),
        (
            "region weather",
            input.weather.map(|pair| {
                pair.filter(|p| p[0] != 0 || p[1] != 0).map(|p| Row {
                    kind: 28,
                    value: p[0],
                })
            }),
        ),
        (
            "night",
            input
                .night
                .map(|value| value.map(|value| Row { kind: 29, value })),
        ),
        (
            "enemy air",
            input
                .enemy_air_positive
                .map(|positive| positive.then_some(Row { kind: 14, value: 0 })),
        ),
    ];
    for (name, row) in ordered {
        match row {
            Ok(Some(row)) => out.rows.push(row),
            Ok(None) => {}
            Err(reason) => out.refusals.push((name, reason)),
        }
    }
    out
}

// The caller resolves `name` through 141440750's shared key helper and
// recorded-content localisation; the native formatter supplies the percentage.

pub fn modifier_text(name: &str, value: i64) -> String {
    let percent = installed_content::fixed5::fixed5_percent(value, 2, true);
    if value > 0 {
        format!("{name}: \u{11}G+{percent}\u{11}!")
    } else {
        format!("{name}: \u{11}R{percent}\u{11}!")
    }
}
