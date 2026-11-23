use dashboard_text::scripted_text::stored_text;
use content_reader_localisation::{
    localisation::Argument, localisation::Argument as Arg, localisation::Localisation,
    localisation::Piece, localisation::pieces,
};
use replay_data::rows::CountryRow;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tooltip {
    pub title: String,
    pub body: String,
    pub refused: BTreeMap<String, usize>,
}

impl Tooltip {
    pub fn refuse(&mut self, line: &str, reason: String) {
        *self.refused.entry(format!("{line}: {reason}")).or_default() += 1;
    }
    pub fn line(&mut self, body: bool, gap: usize, name: &str, value: Result<String, String>) {
        match value {
            Ok(value) => {
                let out = if body {
                    &mut self.body
                } else {
                    &mut self.title
                };
                if !value.is_empty() {
                    ensure_newlines(out, gap);
                    out.push_str(&value);
                }
            }
            Err(reason) => self.refuse(name, reason),
        }
    }
}

pub trait TextIcons {
    fn icon(&self, name: &str) -> Option<(u32, i64)>;
}
#[cfg(any(test, feature = "test-support"))]
pub struct NoIcons;
#[cfg(any(test, feature = "test-support"))]
impl TextIcons for NoIcons {
    fn icon(&self, _: &str) -> Option<(u32, i64)> {
        None
    }
}
pub fn displayable(icons: &(impl TextIcons + ?Sized), text: String) -> Result<String, String> {
    validate(icons, &text)?;
    Ok(text)
}

pub fn validate(icons: &(impl TextIcons + ?Sized), text: &str) -> Result<(), String> {
    validate_inner(icons, text, false)
}

pub fn displayable_flags(
    icons: &(impl TextIcons + ?Sized),
    text: String,
) -> Result<String, String> {
    validate_inner(icons, &text, true)?;
    Ok(text)
}

fn validate_inner(
    icons: &(impl TextIcons + ?Sized),
    text: &str,
    flags: bool,
) -> Result<(), String> {
    for piece in pieces(text) {
        match piece {
            Piece::Text(_) | Piece::Colour(_) => (),
            Piece::Icon(name) => {
                if let Some((frames, frame)) = icons.icon(name)
                    && !(0..i64::from(frames)).contains(&frame)
                {
                    return Err(format!(
                        "{}, which selects frame index {frame} of a {frames}-frame sprite, outside it",
                        piece.describe()
                    ));
                }
            }
            Piece::Flag(_) if flags => (),
            Piece::Flag(_) | Piece::Control14 | Piece::Control1A(_) => {
                return Err(format!(
                    "{}, which the viewer does not draw",
                    piece.describe()
                ));
            }
        }
    }
    Ok(())
}
pub fn builtin(
    icons: &(impl TextIcons + ?Sized),
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    let text = localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|refusal| refusal.to_string())?;
    displayable(icons, text).map_err(|reason| format!("localisation {key} holds {reason}"))
}
pub fn token(
    icons: &(impl TextIcons + ?Sized),
    localisation: &Localisation,
    token: &[u8],
) -> Result<String, String> {
    token_with(icons, localisation, token, &[])
}
pub fn token_with(
    icons: &(impl TextIcons + ?Sized),
    localisation: &Localisation,
    token: &[u8],
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    let text = localisation
        .localise_with(token, arguments)
        .map_err(|refusal| refusal.to_string())?;
    displayable(icons, text).map_err(|reason| {
        format!(
            "localisation {} holds {reason}",
            String::from_utf8_lossy(content_reader_parser::parser::c_string(token))
        )
    })
}
#[macro_export]
macro_rules! tooltip_keys {
    ($($key:ident = $text:literal),* $(,)?) => {
        $(const $key: &str = $text;)*
    };
}
pub use crate::tooltip_keys;

pub fn state_name(
    loc: &Localisation,
    recorded: &str,
    id: u32,
    custom: &[u8],
    owner: &[u8],
    template: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    for (bytes, what) in [(custom, "the custom name"), (owner, "the owner name")] {
        if !bytes.is_empty() {
            return stored_text(loc, recorded, bytes, &format!("state {id}: {what}"));
        }
    }
    template()
}

pub fn localise(loc: &Localisation, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
    loc.localise_with(key.as_bytes(), args)
        .map_err(|e| e.to_string())
}

pub fn ensure_newlines(text: &mut String, count: usize) {
    if !text.is_empty() {
        let have = text.bytes().rev().take_while(|&b| b == b'\n').count();
        for _ in have..count {
            text.push('\n');
        }
    }
}

pub fn country_localised(
    loc: &Localisation,
    tag: Result<&str, &str>,
    key: &str,
) -> Result<String, String> {
    let tag = tag.map_err(|reason| {
        format!("{key}'s name needs its country's TAG_{key} key, but {reason}")
    })?;
    let qualified = format!("{tag}_{key}");
    localise(
        loc,
        if loc.get(&qualified).is_some() {
            &qualified
        } else {
            key
        },
        &[],
    )
}

pub fn viewing_tag<'a>(
    countries: &'a [CountryRow],
    vision: &crate::perspective::Vision,
) -> Result<&'a str, &'static str> {
    countries
        .iter()
        .find(|country| country.canonical_slot == vision.ours())
        .ok_or("the viewing country has no Countries row")
        .and_then(|country| {
            std::str::from_utf8(&country.tag).map_err(|_| "the viewing country's tag is not UTF-8")
        })
}

pub fn value_limit(
    loc: &Localisation,
    min: Option<i32>,
    max: Option<i32>,
) -> Result<String, String> {
    let key = match (min, max) {
        (None, None) => return Ok(String::new()),
        (Some(_), None) => "value_limit_min",
        (None, Some(_)) => "value_limit_max",
        (Some(a), Some(b)) if b < a => "value_limit_invalid",
        (Some(a), Some(b)) if a == b => "value_limit_exact",
        _ => "value_limit_min_max",
    };
    let mut args = Vec::new();
    if let Some(n) = min {
        args.push(("MIN", Arg::Int32(n)));
    }
    if let Some(n) = max {
        args.push(("MAX", Arg::Int32(n)));
    }
    localise(loc, key, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_lines_are_named_and_counted_without_hiding_other_lines() {
        let mut tooltip = Tooltip::default();
        tooltip.line(false, 2, "one", Ok("first\n".into()));
        for _ in 0..2 {
            tooltip.line(false, 2, "two", Err("absent operand".into()));
        }
        tooltip.line(false, 2, "three", Ok("last".into()));
        assert_eq!(tooltip.title, "first\n\nlast");
        assert_eq!(tooltip.refused["two: absent operand"], 2);
    }

    #[test]
    fn stored_state_names_precede_the_template_in_the_recording_language_only() {
        let mut loc = Localisation::default();
        loc.language = "english".into();
        let name = |custom: &[u8], owner: &[u8], recorded| {
            state_name(&loc, recorded, 7, custom, owner, || Ok("template".into()))
        };
        assert_eq!(
            name(b"$literal$", b"owner", "english").as_deref(),
            Ok("$literal$")
        );
        assert_eq!(name(b"", b"owner", "english").as_deref(), Ok("owner"));
        assert_eq!(name(b"", b"", "english").as_deref(), Ok("template"));
        assert_eq!(name(b"", b"", "german").as_deref(), Ok("template"));
        assert_eq!(
            name(b"", b"Bohmen", "german"),
            Err("state 7: the owner name is text the game stored in german without its key, and the viewer's language is english".into())
        );
        assert!(
            name(b"Sudetenland", b"", "german")
                .unwrap_err()
                .contains("custom name")
        );
        assert_eq!(
            name(&[255], b"owner", "english"),
            Err("state 7: the custom name is not UTF-8".into())
        );
    }
}


pub fn wing_short_name(loc: &Localisation, tag: &str, key: &str) -> Result<String, String> {
    let short = format!("{key}_short");
    let qualified = format!("{tag}_{short}");
    let selected = if loc.get(&qualified).is_some() {
        qualified
    } else if loc.get(&short).is_some() {
        short
    } else {
        return country_localised(loc, Ok(tag), key);
    };
    localise(loc, &selected, &[])
}

#[cfg(test)]
mod wing_name_tests {
    use super::*;
    #[test]
    fn wing_short_name_prefers_country_then_generic_then_ordinary_key() {
        let fixture = |country, generic| {
            let mut entries = vec![("fighter", "Ordinary")];
            if country {
                entries.push(("TAG_fighter_short", "Country short"));
            }
            if generic {
                entries.push(("fighter_short", "Generic short"));
            }
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect::<Localisation>()
        };
        assert_eq!(
            wing_short_name(&fixture(true, true), "TAG", "fighter").unwrap(),
            "Country short"
        );
        assert_eq!(
            wing_short_name(&fixture(false, true), "TAG", "fighter").unwrap(),
            "Generic short"
        );
        assert_eq!(
            wing_short_name(&fixture(false, false), "TAG", "fighter").unwrap(),
            "Ordinary"
        );
    }
}
