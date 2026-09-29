//! Small adapter for main's `Localisation`. L4 replaces this substitution,
//! numeric formatting and markup adapter with the shared text implementation.
//! Callers retain typed native arguments: tag 1 strings, tag 2 integers and
//! tag 9 fixed5 (`localization_substitution_record::consumer@142230F20`).

use installed_content::localisation::Localisation;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Text(String),
    Integer(i32),
    Fixed(i64),
}

pub fn localise(loc: &Localisation, key: &str, args: &[(&str, Arg)]) -> Result<String, String> {
    expand(loc, key, args, &mut BTreeSet::new())
}

fn expand(
    loc: &Localisation,
    key: &str,
    args: &[(&str, Arg)],
    active: &mut BTreeSet<String>,
) -> Result<String, String> {
    let key = key.split('\0').next().unwrap_or_default();
    if !active.insert(key.to_owned()) {
        return Err(format!("localisation cycle at {key}"));
    }
    let result = (|| {
        // `142230940`: an absent top-level key is displayed literally.
        let text = loc.get(key).unwrap_or(key);
        if text.contains('[') {
            return Err(format!(
                "scripted localisation in {key} requires L4's scoped text"
            ));
        }
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('$') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after
                .find('$')
                .ok_or_else(|| format!("unclosed substitution in {key}"))?;
            let token = &after[..end];
            let (name, format) = token.split_once('|').unwrap_or((token, ""));
            let value = match args.iter().find(|(key, _)| *key == name) {
                Some((_, value)) => format_arg(value, format)?,
                None if loc.get(name).is_some() => {
                    let text = expand(loc, name, args, active)?;
                    format_arg(&Arg::Text(text), format)?
                }
                None => return Err(format!("unbound localisation argument {name} in {key}")),
            };
            out.push_str(&value);
            // The native localisation scratch buffer is 0x1800 bytes
            // (`142230F20`). Refuse overflow rather than truncate a line.
            if out.len() >= 0x1800 {
                return Err(format!("localisation {key} exceeds native buffer"));
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        if out.len() >= 0x1800 {
            return Err(format!("localisation {key} exceeds native buffer"));
        }
        Ok(out)
    })();
    active.remove(key);
    result
}

/// `142232AB0`, tag 9 -> `1424B89F0`: decimal digits truncate, default
/// precision 2; `%` scales before formatting, `%%` only appends the glyph.
/// `+`/`-` choose colour, independently of `=`'s strictly-positive plus.
fn format_arg(value: &Arg, format: &str) -> Result<String, String> {
    let mut precision = 2;
    let mut colour = None;
    let mut sign_colour = None;
    let mut plus = false;
    let mut percent = 0;
    for ch in format.chars() {
        match ch {
            '0'..='5' => precision = ch as u32 - '0' as u32,
            '.' => {}
            '%' => percent += 1,
            '=' => plus = true,
            '+' | '-' => sign_colour = Some(ch),
            // U and K select conversions in the native formatter, rather
            // than text colours. Leave them to the shared L4 formatter.
            'U' | 'K' | 'k' => {
                return Err(format!("unsupported native number directive {format:?}"));
            }
            'A'..='Z' | 'a'..='z' => colour = Some(ch),
            _ => return Err(format!("unsupported native number directive {format:?}")),
        }
    }
    if percent > 2 {
        return Err(format!("unsupported native percent directive {format:?}"));
    }
    let positive = match value {
        Arg::Text(_) => false,
        Arg::Integer(v) => *v > 0,
        Arg::Fixed(v) => *v > 0,
    };
    let zero = match value {
        Arg::Text(_) => false,
        Arg::Integer(v) => *v == 0,
        Arg::Fixed(v) => *v == 0,
    };
    let scale = if percent == 1 { 100 } else { 1 };
    let mut text = match value {
        Arg::Text(text) if percent == 0 && !plus && sign_colour.is_none() => text.clone(),
        Arg::Text(_) => return Err("numeric directive applied to a string".into()),
        Arg::Integer(v) => v.wrapping_mul(scale).to_string(),
        Arg::Fixed(v) => fixed(v.wrapping_mul(i64::from(scale)), precision),
    };
    if plus && positive {
        text.insert(0, '+');
    }
    if percent != 0 {
        text.push('%');
    }
    if let Some(sign) = sign_colour {
        colour = Some(if zero {
            'Y'
        } else if positive == (sign == '+') {
            'G'
        } else {
            'R'
        });
    }
    Ok(match colour {
        Some(code) => format!("§{code}{text}§!"),
        None => text,
    })
}

fn fixed(raw: i64, precision: u32) -> String {
    let magnitude = raw.unsigned_abs();
    let sign = if raw < 0 { "-" } else { "" };
    // `1424B89F0` narrows the whole part to i32 before writing it.
    let whole = (magnitude / 100_000) as i32;
    if precision == 0 {
        format!("{sign}{whole}")
    } else {
        let fraction = magnitude % 100_000 / 10u64.pow(5 - precision);
        format!(
            "{sign}{whole}.{fraction:0width$}",
            width = precision as usize
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_arguments_keep_native_truncation_and_directives() {
        for (raw, format, expected) in [
            (199_999, "", "1.99"),
            (-1, "0", "-0"),
            (9_999, "1%", "9.9%"),
            (9_999, "1%%", "0.0%"),
            (100_000, "0+=", "§G+1§!"),
            (-100_000, "0-=", "§G-1§!"),
            (0, "0+", "§Y0§!"),
            (12_345, ".3H", "§H0.123§!"),
        ] {
            assert_eq!(format_arg(&Arg::Fixed(raw), format).unwrap(), expected);
        }
        assert_eq!(format_arg(&Arg::Integer(3), "2").unwrap(), "3");
        assert!(format_arg(&Arg::Fixed(1), "^2").is_err());
    }

    #[test]
    fn expansion_uses_content_and_names_unsupported_forms() {
        let loc: Localisation = [("line", "$nested$: $V|0%$"), ("nested", "Chance")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        assert_eq!(
            localise(&loc, "line", &[("V", Arg::Fixed(75_999))]).unwrap(),
            "Chance: 75%"
        );
        assert_eq!(localise(&loc, "absent", &[]).unwrap(), "absent");
        assert!(localise(&loc, "[ROOT.GetName]", &[]).is_err());
        assert!(localise(&loc, "line", &[]).unwrap_err().contains("V"));
    }
}
