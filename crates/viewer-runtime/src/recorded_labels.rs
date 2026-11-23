use content_reader_layers::ResolvedLayers;
use content_reader_localisation::localisation::Localisation;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Unavailable(String),
    Unmatched,
    Ambiguous(usize),
    Localisation(String),
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
            Self::Localisation(reason) => {
                format!("{what} cannot show its recovered {display} localisation: {reason}")
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct LabelIndex {
    by_text: HashMap<String, Vec<String>>,
}

impl LabelIndex {
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
            let text = display
                .localise_bytes(key.as_bytes(), &[])
                .map_err(|error| Refusal::Localisation(error.to_string()))?;
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

pub struct RecordedLabels {
    language: String,
    index: Result<Option<LabelIndex>, String>,
}

impl RecordedLabels {
    pub fn prepare(language: String, display: &str, layers: &ResolvedLayers) -> Self {
        let index = if display == language {
            Ok(None)
        } else {
            content_reader_localisation::localisation::read(layers, &language)
                .map(|recorded| Some(LabelIndex::new(&recorded)))
                .map_err(|error| error.to_string())
        };
        Self { language, index }
    }

    pub fn from_localisation(recorded: &Localisation) -> Self {
        Self {
            language: recorded.language.clone(),
            index: Ok(Some(LabelIndex::new(recorded))),
        }
    }

    pub fn language(&self) -> &str {
        &self.language
    }

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

#[cfg(any(test, feature = "test-support"))]
pub fn english() -> &'static RecordedLabels {
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
        assert_eq!(text("Rain").as_deref(), Ok("Pluie"));
        assert!(
            text("Coast")
                .unwrap_err()
                .contains("2 english localisation readings")
        );
        assert_eq!(
            text("\u{13}tech_mod Smoke Generators").as_deref(),
            Ok("\u{13}tech_mod Générateurs de fumée")
        );
        assert_eq!(text("\u{13}tech_mod ").as_deref(), Ok("\u{13}tech_mod "));
        assert!(text("\u{13}tech_mod German Navy").is_err());
        assert_eq!(
            text("\u{13}weather Stormfront").as_deref(),
            Ok("\u{13}weather Front orageux")
        );
        assert!(
            text("\u{13}weather Squall")
                .unwrap_err()
                .contains("2 english localisation readings")
        );
        assert_eq!(
            text("\u{13}weather Drizzle").as_deref(),
            Ok("\u{13}weather Bruine")
        );
        assert!(
            text("German Navy")
                .unwrap_err()
                .contains("no english localisation key")
        );
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
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let labels = RecordedLabels::prepare("english".into(), "french", &layers);
        std::fs::remove_dir_all(&root).unwrap();
        let french = loc("french", &[("NIGHT", "Nuit")]);
        assert_eq!(
            labels
                .text(&french, b"Night", "naval modifier source")
                .as_deref(),
            Ok("Nuit")
        );
        let same = RecordedLabels::prepare("english".into(), "english", &layers);
        let english = loc("english", &[]);
        assert_eq!(
            same.text(&english, b"Night", "naval modifier source")
                .as_deref(),
            Ok("Night")
        );
    }
}
