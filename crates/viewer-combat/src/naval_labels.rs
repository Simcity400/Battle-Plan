//! Naval source labels in the display language.
//!
//! The naval source trees store each source's label as the text the game wrote in the
//! recording's language, without its localisation key. In that language the label shows
//! as stored. In another language it is recovered: the recording language's
//! localisation from the recorded content stack, indexed when the document loads, is
//! searched for the keys whose own text renders exactly to the label, and the label shows
//! as those keys render in the display language. A label that opens with an inline icon
//! (0x13, its name and the character ending it) is also matched with the icon kept as
//! written and the text after it recovered, since the game writes such an icon before a
//! doctrine's, technology's or idea's name; an authored text may hold the icon itself, so
//! both readings are tried and must agree. A label no key renders to (a composed or
//! substituted text, or one whose token the game's modifier copy discarded) and a label
//! whose readings render differently in the display language keep the stored-text
//! refusal by name, each counted with the hover's other refusals.
use installed_content::{ResolvedLayers, localisation::Localisation};
use std::collections::HashMap;

/// Why a stored label cannot show in the display language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The recording language's localisation failed to load.
    Unavailable(String),
    /// No key's text renders to the label.
    Unmatched,
    /// Several keys render to the label and differ in the display language.
    Ambiguous(usize),
}

impl Refusal {
    fn reason(&self, recorded: &str, display: &str, what: &str) -> String {
        match self {
            Self::Unavailable(reason) => format!(
                "{what} is text the game stored in {recorded} without its key, and the {recorded} localisation that would recover its key for {display} is unavailable: {reason}"
            ),
            Self::Unmatched => format!(
                "{what} is text the game stored in {recorded} without its key, and no {recorded} localisation key renders to it, so it cannot show in {display}"
            ),
            Self::Ambiguous(count) => format!(
                "{what} is text the game stored in {recorded} without its key, and {count} {recorded} localisation readings render to it with different {display} texts"
            ),
        }
    }
}

/// The keys of one language by the text each renders to with no arguments.
#[derive(Debug, Default)]
pub struct LabelIndex {
    by_text: HashMap<String, Vec<String>>,
}

impl LabelIndex {
    /// Every key whose text renders, keyed by that text; empty texts are no label.
    pub fn new(recorded: &Localisation) -> Self {
        let mut by_text: HashMap<String, Vec<String>> = HashMap::new();
        for (key, _) in recorded.iter() {
            if let Ok(text) = recorded.localise(key.as_bytes())
                && !text.is_empty()
            {
                by_text.entry(text).or_default().push(key.to_owned());
            }
        }
        Self { by_text }
    }

    /// The label as its keys render in `display`, or why it cannot be recovered. A label
    /// opening with an inline icon is read both whole and as the icon followed by a
    /// recovered text; readings that recover must agree.
    pub fn recover(&self, label: &str, display: &Localisation) -> Result<String, Refusal> {
        let whole = self.recover_text(label, display);
        let (icon, text) = label.split_at(icon_prefix(label));
        if icon.is_empty() {
            return whole;
        }
        let prefixed = if text.is_empty() {
            Ok(label.to_owned())
        } else {
            self.recover_text(text, display)
                .map(|text| format!("{icon}{text}"))
        };
        match (whole, prefixed) {
            (Ok(whole), Ok(prefixed)) if whole == prefixed => Ok(whole),
            (Ok(_), Ok(_)) => Err(Refusal::Ambiguous(2)),
            (Err(Refusal::Ambiguous(count)), _) | (_, Err(Refusal::Ambiguous(count))) => {
                Err(Refusal::Ambiguous(count))
            }
            (Ok(text), Err(_)) | (Err(_), Ok(text)) => Ok(text),
            (Err(refusal), Err(_)) => Err(refusal),
        }
    }

    fn recover_text(&self, label: &str, display: &Localisation) -> Result<String, Refusal> {
        let keys = self.by_text.get(label).ok_or(Refusal::Unmatched)?;
        let mut shown: Option<String> = None;
        for key in keys {
            let text = display.localise_bytes(key.as_bytes(), &[]);
            let text = String::from_utf8(text).map_err(|_| Refusal::Unmatched)?;
            match &shown {
                Some(first) if *first != text => return Err(Refusal::Ambiguous(keys.len())),
                Some(_) => {}
                None => shown = Some(text),
            }
        }
        shown.ok_or(Refusal::Unmatched)
    }
}

/// The byte length of a leading inline icon: 0x13, a name of up to 128 characters from
/// `0-9`, `A-Z`, `a-z`, `_` and `|`, and the character that ends a shorter name, which
/// the text layout consumes; zero without one.
fn icon_prefix(label: &str) -> usize {
    let Some(body) = label.strip_prefix('\u{13}') else {
        return 0;
    };
    let mut end = 1;
    for (count, character) in body.chars().enumerate() {
        if count == 128 {
            return end;
        }
        end += character.len_utf8();
        if !(character.is_ascii_alphanumeric() || character == '_' || character == '|') {
            return end;
        }
    }
    end
}

/// The recording's language and, when the display language differs, its label index,
/// prepared with the document's content so drawing never reads content.
pub struct RecordedLabels {
    language: String,
    /// None when the display language is the recording's and no label needs recovery.
    index: Result<Option<LabelIndex>, String>,
}

impl RecordedLabels {
    /// Labels of a recording in `language` shown in `display`: the recording language's
    /// localisation is read from `layers` and indexed now when the languages differ.
    pub fn prepare(language: String, display: &str, layers: &ResolvedLayers) -> Self {
        let index = if display == language {
            Ok(None)
        } else {
            installed_content::localisation::read(layers, &language)
                .map(|recorded| Some(LabelIndex::new(&recorded)))
                .map_err(|error| error.to_string())
        };
        Self { language, index }
    }

    /// Labels whose recording-language localisation is already in hand.
    pub fn from_localisation(recorded: &Localisation) -> Self {
        Self {
            language: recorded.language.clone(),
            index: Ok(Some(LabelIndex::new(recorded))),
        }
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// A stored label as it shows in `display`'s language: as stored in the recording's
    /// language, otherwise recovered through its keys ([`LabelIndex::recover`]).
    pub fn text(&self, display: &Localisation, bytes: &[u8], what: &str) -> Result<String, String> {
        if bytes.is_empty() || display.language == self.language {
            return dashboard_text::scripted_text::stored_text(
                display,
                &self.language,
                bytes,
                what,
            );
        }
        let label = std::str::from_utf8(bytes).map_err(|_| format!("{what} is not UTF-8"))?;
        match &self.index {
            Ok(Some(index)) => index.recover(label, display),
            Ok(None) => Err(Refusal::Unavailable(format!(
                "it was prepared for {} only",
                self.language
            ))),
            Err(reason) => Err(Refusal::Unavailable(reason.clone())),
        }
        .map_err(|refusal| refusal.reason(&self.language, &display.language, what))
    }
}

/// Labels of an English recording with no localisation keys, for tests.
#[cfg(test)]
pub(crate) fn english() -> &'static RecordedLabels {
    static ENGLISH: std::sync::LazyLock<RecordedLabels> = std::sync::LazyLock::new(|| {
        let mut english = Localisation::default();
        english.language = "english".into();
        RecordedLabels::from_localisation(&english)
    });
    &ENGLISH
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(language: &str, entries: &[(&str, &str)]) -> Localisation {
        let mut loc: Localisation = entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        loc.language = language.into();
        loc
    }

    #[test]
    fn a_label_shows_through_the_keys_that_render_to_it() {
        let english = loc(
            "english",
            &[
                ("FLEET_DOCTRINE", "Fleet in Being"),
                ("WEATHER_RAIN", "Rain"),
                ("WEATHER_RAIN_LIGHT", "Rain"),
                ("TERRAIN_A", "Coast"),
                ("TERRAIN_B", "Coast"),
                ("NAMED", "$NAME$ Navy"),
                ("SMOKE", "Smoke Generators"),
                ("STORMFRONT", "\u{13}weather Stormfront"),
                ("SQUALL_ICON", "\u{13}weather Squall"),
                ("SQUALL", "Squall"),
                ("DRIZZLE_ICON", "\u{13}weather Drizzle"),
                ("DRIZZLE", "Drizzle"),
            ],
        );
        let french = loc(
            "french",
            &[
                ("FLEET_DOCTRINE", "Flotte de dissuasion"),
                ("WEATHER_RAIN", "Pluie"),
                ("WEATHER_RAIN_LIGHT", "Pluie"),
                ("TERRAIN_A", "Côte"),
                ("TERRAIN_B", "Littoral"),
                ("SMOKE", "Générateurs de fumée"),
                ("STORMFRONT", "\u{13}weather Front orageux"),
                ("SQUALL_ICON", "\u{13}weather Grain"),
                ("SQUALL", "Rafale"),
                ("DRIZZLE_ICON", "\u{13}weather Bruine"),
                ("DRIZZLE", "Bruine"),
            ],
        );
        let labels = RecordedLabels::from_localisation(&english);
        let text = |label: &str| labels.text(&french, label.as_bytes(), "naval modifier source");
        assert_eq!(
            text("Fleet in Being").as_deref(),
            Ok("Flotte de dissuasion")
        );
        // Two keys that agree in the display language recover.
        assert_eq!(text("Rain").as_deref(), Ok("Pluie"));
        // Two that differ refuse by name.
        assert!(
            text("Coast")
                .unwrap_err()
                .contains("2 english localisation readings")
        );
        // An inline icon before the name stays as written.
        assert_eq!(
            text("\u{13}tech_mod Smoke Generators").as_deref(),
            Ok("\u{13}tech_mod Générateurs de fumée")
        );
        assert_eq!(text("\u{13}tech_mod ").as_deref(), Ok("\u{13}tech_mod "));
        assert!(text("\u{13}tech_mod German Navy").is_err());
        // An icon the authored text holds itself is matched whole.
        assert_eq!(
            text("\u{13}weather Stormfront").as_deref(),
            Ok("\u{13}weather Front orageux")
        );
        // A whole match and an icon-kept match that render differently refuse.
        assert!(
            text("\u{13}weather Squall")
                .unwrap_err()
                .contains("2 english localisation readings")
        );
        // Readings that agree recover.
        assert_eq!(
            text("\u{13}weather Drizzle").as_deref(),
            Ok("\u{13}weather Bruine")
        );
        // A composed text matches no key.
        assert!(
            text("German Navy")
                .unwrap_err()
                .contains("no english localisation key")
        );
        // The recording's own language shows the stored text.
        assert_eq!(
            labels
                .text(&english, b"German Navy", "naval modifier source")
                .as_deref(),
            Ok("German Navy")
        );
        assert_eq!(
            labels
                .text(&french, b"", "naval modifier source")
                .as_deref(),
            Ok("")
        );
    }

    /// The recording language's index is read with the document's content: a label is
    /// recovered after the content folder is gone, so drawing reads no content.
    #[test]
    fn the_index_is_prepared_with_the_content_and_drawing_reads_none() {
        let root = std::env::temp_dir().join(format!(
            "naval-labels-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("localisation/english")).unwrap();
        std::fs::write(
            root.join("localisation/english/naval_l_english.yml"),
            "\u{feff}l_english:\n NIGHT:0 \"Night\"\n",
        )
        .unwrap();
        let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let labels = RecordedLabels::prepare("english".into(), "french", &layers);
        std::fs::remove_dir_all(&root).unwrap();
        let french = loc("french", &[("NIGHT", "Nuit")]);
        assert_eq!(
            labels
                .text(&french, b"Night", "naval modifier source")
                .as_deref(),
            Ok("Nuit")
        );
        // The recording's own language needs no index and shows the stored text.
        let same = RecordedLabels::prepare("english".into(), "english", &layers);
        let english = loc("english", &[]);
        assert_eq!(
            same.text(&english, b"Night", "naval modifier source")
                .as_deref(),
            Ok("Night")
        );
    }
}
