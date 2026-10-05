//! Intel presentation shared by the operations log and agency comparison.
use super::ContentOptions;
use crate::{
    Artwork, Source,
    catalogue::{self, Catalogue, View},
};
use installed_content::localisation::Argument;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, OnceLock},
};

pub struct IntelligenceContent {
    pub catalogue: Arc<Catalogue>,
    pub definitions: installed_content::intelligence::Intelligence,
    pub warnings: Vec<String>,
    defense: OnceLock<Option<DefenseTemplate>>,
    artwork: Mutex<ArtworkCache>,
}

struct DefenseTemplate {
    prefix: String,
    suffix: String,
    label: String,
}

impl DefenseTemplate {
    /// 140F172A0 passes LEVEL through 1402E2DA0; 142232AB0's Text branch
    /// copies this marker but does not apply numeric precision, sign or suffix.
    /// Boundaries therefore isolate the Fixed5 branch's complete native number.
    fn load(catalogue: &Catalogue) -> Option<Self> {
        const KEY: &str = "AGENCY_DEFENSE_LEVEL";
        const MARKER: &str = "\u{e000}";
        catalogue.localisation.get(KEY)?;
        let text =
            catalogue.localised_with(KEY, &[("LEVEL", Argument::Text(MARKER))], View::Plain)?;
        let count = text.matches(MARKER).count();
        if count != 1 {
            catalogue.refuse(
                &format!("defence template must render LEVEL once to separate its label and number; rendered {count} occurrences"),
                KEY,
            );
            return None;
        }
        let (prefix, suffix) = text.split_once(MARKER).expect("one marker");
        let label = format!("{prefix}{suffix}")
            .trim_matches(|character: char| {
                character.is_whitespace() || matches!(character, ':' | '：')
            })
            .to_owned();
        if label.is_empty() {
            catalogue.refuse("defence template has no label apart from LEVEL", KEY);
            return None;
        }
        Some(Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
            label,
        })
    }
}

#[derive(Default)]
struct ArtworkCache {
    entries: BTreeMap<String, (Artwork, Option<String>)>,
    bytes: usize,
}

pub fn intelligence_content(
    source: &Source,
    options: &ContentOptions,
) -> io::Result<Arc<IntelligenceContent>> {
    type Cached = Option<Arc<IntelligenceContent>>;
    let catalogue = catalogue::catalogue(source, options)?;
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(cached) = state.as_ref()
        && Arc::ptr_eq(&cached.catalogue, &catalogue)
    {
        return Ok(Arc::clone(cached));
    }
    let loaded = Arc::new(IntelligenceContent::load(catalogue)?);
    *state = Some(Arc::clone(&loaded));
    Ok(loaded)
}

impl IntelligenceContent {
    pub(crate) fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        let definitions = installed_content::intelligence::read(&catalogue.content.layers)?;
        let mut warnings = catalogue.verification.clone();
        warnings.extend(catalogue.warnings.iter().cloned());
        warnings.extend(definitions.warnings.iter().cloned());
        Ok(Self {
            catalogue,
            definitions,
            warnings,
            defense: OnceLock::default(),
            artwork: Mutex::default(),
        })
    }

    /// GetAgency's uncreated branch, 141A7B350 case 0x19. The literal is an
    /// executable fact; the displayed bytes always come from recorded content.
    pub fn default_agency_name(&self) -> Option<String> {
        self.catalogue.localised("AGENCY_NAME", View::Plain)
    }

    /// Agency screen 140F172A0 -> LocalizeWithFixedPointArgument 1402E2DA0:
    /// the content controls the LEVEL argument's precision and suffix.
    pub fn defense_text(&self, raw: i64) -> Option<String> {
        self.catalogue.localised_with(
            "AGENCY_DEFENSE_LEVEL",
            &[("LEVEL", Argument::Fixed5(raw))],
            View::Plain,
        )
    }

    /// The same content template's label, without its LEVEL number or separator.
    pub fn defense_label(&self) -> Option<String> {
        self.defense_template()
            .map(|template| template.label.clone())
    }

    /// Keep the full native LEVEL output, including authored precision and
    /// numeric suffixes (142232AB0); never infer it by parsing rendered digits.
    pub fn defense_value(&self, text: &str) -> Option<String> {
        let template = self.defense_template()?;
        let value = text
            .strip_prefix(&template.prefix)
            .and_then(|text| text.strip_suffix(&template.suffix));
        if value.is_none() {
            self.catalogue.refuse(
                "defence template's rendered LEVEL boundaries differ from its fixed-point output",
                "AGENCY_DEFENSE_LEVEL",
            );
        }
        value.map(str::to_owned)
    }

    fn defense_template(&self) -> Option<&DefenseTemplate> {
        self.defense
            .get_or_init(|| DefenseTemplate::load(&self.catalogue))
            .as_ref()
    }

    /// COperation::vf4 140A70140 stores name/icon (+4D0/+490).
    /// LocalizeOperationName 140A6DF00 localises the stored name without event
    /// scope expansion. Missing or refused content remains unknown.
    pub fn operation_artwork(&self, token: &str) -> (Artwork, Option<String>) {
        let mut cached = self
            .artwork
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(artwork) = cached.entries.get(token) {
            return artwork.clone();
        }
        let mut artwork = Artwork {
            token: token.into(),
            name: None,
            description: None,
            base_duration_days: None,
            icon: None,
        };
        let definition = match self.definitions.operation(token) {
            Ok(Some(definition)) => definition,
            Ok(None) => {
                self.catalogue.refuse(
                    "operation definition absent from the recorded content stack",
                    token,
                );
                return (
                    artwork,
                    Some(format!(
                        "operation {token}: no definition in the recorded content stack"
                    )),
                );
            }
            Err(why) => {
                self.catalogue.refuse(&why, token);
                return (artwork, Some(why));
            }
        };
        artwork.name = (!definition.name.is_empty())
            .then(|| self.catalogue.localised(&definition.name, View::Plain))
            .flatten();
        let icon = self.sprite_png(&definition.sprite);
        let reason = match icon {
            Ok(icon) => {
                artwork.icon = Some(icon);
                None
            }
            Err(why) => {
                self.catalogue.refuse(&why.to_string(), &definition.sprite);
                Some(format!(
                    "operation {token}: artwork {} unavailable: {why}",
                    definition.sprite
                ))
            }
        };
        let bytes = super::artwork::artwork_bytes(&artwork)
            .saturating_add(token.len())
            .saturating_add(reason.as_ref().map_or(0, |reason| reason.len() + 32));
        if cached.bytes.saturating_add(bytes) > super::artwork::MAX_RETAINED_ARTWORK_BYTES {
            let why = "operation artwork exceeds the shared retained artwork byte budget";
            self.catalogue.refuse(why, token);
            artwork.icon = None;
            return (artwork, Some(format!("operation {token}: {why}")));
        }
        let result = (artwork, reason);
        // Cache only authored operation definitions, once each; raw unknown
        // dictionary tokens cannot enlarge it across a campaign.
        cached.bytes += bytes;
        cached.entries.insert(token.to_owned(), result.clone());
        result
    }

    fn sprite_png(&self, sprite: &str) -> io::Result<String> {
        if sprite.is_empty() {
            return Err(crate::invalid("no authored sprite"));
        }
        let sprite = self
            .catalogue
            .sprites
            .lookup(sprite)
            .map_err(crate::invalid)?;
        let decode = self.catalogue.gui_decode.as_ref().map_err(crate::invalid)?;
        catalogue::sprite_frame_png(
            &self.catalogue.content.layers,
            sprite,
            *decode,
            0,
            super::artwork::ICON_EDGE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intelligence_content_ignores_uncaptured_agency_upgrade_definitions() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/operations/mod.txt",
                "mod_operation = { name = MOD_OPERATION icon = mod_sprite }",
            ),
            (
                "common/intelligence_agency_upgrades/mod.txt",
                "mod_branch = { mod_upgrade = { picture = { unsupported = yes } } }",
            ),
        ]);
        let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
        assert!(
            content
                .definitions
                .operation("mod_operation")
                .unwrap()
                .is_some()
        );
        assert!(content.definitions.warnings.is_empty());
        assert!(
            content
                .warnings
                .iter()
                .all(|warning| !warning.contains("agency upgrade"))
        );
    }

    #[test]
    fn intelligence_content_modded_operation_name_art_and_uncreated_agency_are_content_resolved() {
        let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/operations/mod.txt",
                "a_custom_operation = { name = MODDED_OPERATION icon = mod_sprite desc = MODDED_DESC }",
            ),
            (
                "interface/mod.gfx",
                "spriteTypes = { spriteType = { name = mod_sprite texturefile = gfx/mod.tga } }",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n MODDED_OPERATION:0 \"Custom operation\"\n MODDED_DESC:0 \"Custom description\"\n AGENCY_NAME:0 \"Custom agency\"\n AGENCY_DEFENSE_LEVEL:0 \"$LEVEL|.1$ protection\"\n",
            ),
        ]);
        let mut tga = vec![0; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 32;
        tga[17] = 0x28;
        tga.extend_from_slice(&[40, 80, 120, 255]);
        std::fs::create_dir_all(folder.0.join("gfx")).unwrap();
        std::fs::write(folder.0.join("gfx/mod.tga"), tga).unwrap();
        let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
        let (artwork, reason) = content.operation_artwork("a_custom_operation");
        assert_eq!(artwork.name.as_deref(), Some("Custom operation"));
        assert!(
            artwork
                .icon
                .as_deref()
                .is_some_and(|icon| icon.starts_with("data:image/png;base64,")),
            "{reason:?}"
        );
        assert_eq!(
            content.default_agency_name().as_deref(),
            Some("Custom agency")
        );
        assert_eq!(
            content.defense_text(125_000).as_deref(),
            Some("1.2 protection")
        );
        assert_eq!(content.defense_label().as_deref(), Some("protection"));
        assert_eq!(
            content.defense_value("1.2 protection").as_deref(),
            Some("1.2")
        );
        let (missing, reason) = content.operation_artwork("missing_type");
        assert_eq!(missing.name, None);
        assert_eq!(missing.icon, None);
        assert!(reason.unwrap().contains("missing_type"));
    }

    #[test]
    fn intelligence_content_defense_parts_keep_native_precision_sign_suffix_and_references() {
        for (template, raw, label, number) in [
            ("$LABEL$: $LEVEL|H$", 575_999, "Custom protection", "5.75"),
            ("$LEVEL|.1$ $LABEL$", 125_000, "Custom protection", "1.2"),
            (
                "$LABEL$：$LEVEL|.3%=+$",
                12_345,
                "Custom protection",
                "+12.345%",
            ),
            (
                "$LABEL$: $LEVEL|^1$",
                150_000_000,
                "Custom protection",
                "1.5K",
            ),
            ("$LABEL$: $NESTED$", -125_000, "Custom protection", "-1.2"),
            ("$LABEL$: $LEVEL|_1$", 0, "Custom protection", ""),
        ] {
            let localisation = format!(
                "\u{feff}l_english:\n LABEL:0 \"Custom protection\"\n NESTED:0 \"$LEVEL|.1$\"\n AGENCY_DEFENSE_LEVEL:0 \"{template}\"\n"
            );
            let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
                "localisation/test_l_english.yml",
                &localisation,
            )]);
            let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
            let text = content.defense_text(raw).unwrap();
            assert_eq!(
                content.defense_label().as_deref(),
                Some(label),
                "{template}"
            );
            assert_eq!(
                content.defense_value(&text).as_deref(),
                Some(number),
                "{template}"
            );
            assert!(content.catalogue.refusal_lines().is_empty(), "{template}");
        }
    }

    #[test]
    fn intelligence_content_defense_parts_refuse_ambiguous_or_truncated_templates_by_name() {
        for (template, reason) in [
            ("Custom protection", "rendered 0 occurrences"),
            (
                "Custom protection: $LEVEL$ / $LEVEL$",
                "rendered 2 occurrences",
            ),
            ("$LEVEL$", "no label apart from LEVEL"),
        ] {
            let localisation =
                format!("\u{feff}l_english:\n AGENCY_DEFENSE_LEVEL:0 \"{template}\"\n");
            let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
                "localisation/test_l_english.yml",
                &localisation,
            )]);
            let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
            assert_eq!(content.defense_label(), None);
            assert_eq!(
                content.defense_value(&content.defense_text(100_000).unwrap()),
                None
            );
            let refusals = content.catalogue.refusal_lines();
            assert_eq!(refusals.len(), 1, "{template}");
            assert!(refusals[0].starts_with("1 texts are not shown:"));
            assert!(refusals[0].contains("AGENCY_DEFENSE_LEVEL"));
            assert!(refusals[0].contains(reason), "{}", refusals[0]);
        }
        // The localiser's bounded buffer can cut a longer numeric output at a
        // different place from the marker; never invent the cut-off number.
        let template = format!("{}: $LEVEL$ end", "x".repeat(0x1800 - 10));
        let localisation = format!("\u{feff}l_english:\n AGENCY_DEFENSE_LEVEL:0 \"{template}\"\n");
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
            "localisation/test_l_english.yml",
            &localisation,
        )]);
        let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
        assert!(content.defense_label().is_some());
        assert_eq!(
            content.defense_value(&content.defense_text(i64::MAX).unwrap()),
            None
        );
        assert!(content.catalogue.refusal_lines()[0].contains("rendered LEVEL boundaries differ"));
    }

    #[test]
    fn intelligence_content_defense_missing_content_stays_unknown() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[]);
        let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
        assert_eq!(content.defense_label(), None);
        assert_eq!(content.defense_value("AGENCY_DEFENSE_LEVEL"), None);
        assert!(content.catalogue.refusal_lines().is_empty());
    }
}
