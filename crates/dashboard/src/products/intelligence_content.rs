//! Intel presentation shared by the operations log and agency comparison.
use super::ContentOptions;
use crate::{
    Artwork, Source,
    catalogue::{self, Catalogue, Refusals, View},
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
    pub upgrades: installed_content::intelligence::AgencyUpgrades,
    upgrade_content_error: Option<String>,
    pub warnings: Vec<String>,
    agency_refusals: Mutex<Refusals>,
    defense: OnceLock<Option<DefenseTemplate>>,
    artwork: Mutex<ArtworkCache>,
}

struct DefenseTemplate {
    prefix: String,
    suffix: String,
    label: String,
}

impl DefenseTemplate {
    /// The LEVEL argument's Text branch copies this marker without numeric
    /// precision, sign or suffix. Boundaries therefore isolate the fixed5 branch's
    /// complete number.
    fn load(content: &IntelligenceContent) -> Option<Self> {
        const KEY: &str = "AGENCY_DEFENSE_LEVEL";
        const MARKER: &str = "\u{e000}";
        if content.catalogue.localisation.get(KEY).is_none() {
            content.refuse_agency(
                "required agency defence localisation key missing from the recorded content stack",
                KEY,
            );
            return None;
        }
        let text = content.agency_text(KEY, &[("LEVEL", Argument::Text(MARKER))])?;
        let count = text.matches(MARKER).count();
        if count != 1 {
            content.refuse_agency(
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
            content.refuse_agency("defence template has no label apart from LEVEL", KEY);
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
}

pub fn intelligence_content(
    source: &Source,
    options: &ContentOptions,
) -> io::Result<Arc<IntelligenceContent>> {
    prepared_content(source, options, ContentScope::Operations)
}

/// Agency comparison neither shows nor reads operation definitions.
pub fn agency_content(
    source: &Source,
    options: &ContentOptions,
) -> io::Result<Arc<IntelligenceContent>> {
    prepared_content(source, options, ContentScope::Agency)
}

#[derive(Clone, Copy)]
enum ContentScope {
    Agency,
    Operations,
}

#[derive(Default)]
struct Cached {
    agency: Option<Arc<IntelligenceContent>>,
    operations: Option<Arc<IntelligenceContent>>,
}

fn prepared_content(
    source: &Source,
    options: &ContentOptions,
    scope: ContentScope,
) -> io::Result<Arc<IntelligenceContent>> {
    let catalogue = catalogue::catalogue(source, options)?;
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let cached = match scope {
        ContentScope::Agency => &mut state.agency,
        ContentScope::Operations => &mut state.operations,
    };
    if let Some(cached) = cached.as_ref()
        && Arc::ptr_eq(&cached.catalogue, &catalogue)
    {
        return Ok(Arc::clone(cached));
    }
    let loaded = Arc::new(match scope {
        ContentScope::Agency => IntelligenceContent::load_acquired_agency(catalogue),
        ContentScope::Operations => IntelligenceContent::load(catalogue)?,
    });
    *cached = Some(Arc::clone(&loaded));
    Ok(loaded)
}

impl IntelligenceContent {
    pub(crate) fn check_reader_notes(&self) -> Vec<crate::content_check::Finding> {
        self.definitions
            .notes
            .iter()
            .cloned()
            .map(|note| crate::content_check::Finding::reader("operation definitions", note))
            .collect()
    }

    pub(crate) fn check_operations(&self) -> Vec<crate::content_check::Finding> {
        use crate::content_check::{Finding, Kind};
        let mut findings = Vec::new();
        if self.definitions.operation_refusals.is_empty() {
            for definition in self.definitions.operations.values() {
                let subject = format!("operation {}", definition.token);
                let (artwork, _) = self.operation_artwork(&definition.token);
                let notes = self.catalogue.take_findings();
                if artwork.name.is_none()
                    && !notes.iter().any(|note| note.subject == definition.name)
                {
                    findings.push(Finding::new(
                        Kind::Fallback,
                        &subject,
                        "operation has no usable authored name; the reply name is unknown",
                    ));
                }
                // operation_artwork records its own text and image refusals.
                findings.extend(notes);
            }
        }
        findings
    }

    pub(crate) fn check_agency(&self) -> Vec<crate::content_check::Finding> {
        use crate::content_check::{Finding, Kind};
        let mut findings = Vec::new();
        self.default_agency_name();
        findings.extend(self.catalogue.take_findings());
        let label = self.defense_label();
        let notes = self.catalogue.take_findings();
        if label.is_none() && notes.is_empty() {
            findings.push(Finding::new(
                Kind::Fallback,
                "AGENCY_DEFENSE_LEVEL",
                "agency defense template is missing; its label and number are unknown",
            ));
        }
        findings.extend(notes);
        findings.push(Finding::new(Kind::Offline, "AGENCY_DEFENSE_LEVEL",
            "agency defense numeric text needs the recorded IntelligenceAgencies value; unavailable offline"));
        findings
    }

    pub(crate) fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        let definitions = installed_content::intelligence::read(&catalogue.content.layers)?;
        let mut content = Self::load_agency(catalogue);
        content
            .warnings
            .extend(definitions.warnings.iter().cloned());
        content.definitions = definitions;
        Ok(content)
    }

    fn load_acquired_agency(catalogue: Arc<Catalogue>) -> Self {
        let mut content = Self::load_agency(catalogue);
        match installed_content::intelligence::read_agency_upgrades(&content.catalogue.content.layers) {
            Ok(upgrades) => {
                content.warnings.extend(upgrades.warnings.iter().cloned());
                content.upgrades = upgrades;
            }
            Err(error) => {
                let reason = format!("Acquired upgrade content unavailable: {error}");
                content.warnings.push(reason.clone());
                content.upgrade_content_error = Some(reason);
            }
        }
        content
    }

    /// An agency upgrade localises its authored identifier.
    pub fn upgrade_name(&self, branch: &str, token: &str) -> Result<String, String> {
        if let Some(reason) = &self.upgrade_content_error {
            return Err(reason.clone());
        }
        let definition = self
            .upgrades
            .branch_upgrade(branch, token)?
            .ok_or_else(|| {
                format!("agency upgrade {branch}/{token}: absent from the recorded content stack")
            })?;
        self.catalogue
            .localised(&definition.name, View::Plain)
            .ok_or_else(|| format!("agency upgrade {branch}/{token}: localisation refused"))
    }

    /// An operation stores its authored name and localises it.
    pub fn operation_name(&self, token: &str) -> Result<String, String> {
        let definition = self
            .definitions
            .operation(token)?
            .ok_or_else(|| format!("operation {token}: absent from the recorded content stack"))?;
        self.catalogue
            .localised(&definition.name, View::Plain)
            .ok_or_else(|| format!("operation {token}: name localisation refused"))
    }

    pub(crate) fn load_agency(catalogue: Arc<Catalogue>) -> Self {
        let mut warnings = catalogue.verification.clone();
        warnings.extend(catalogue.warnings.iter().cloned());
        Self {
            catalogue,
            definitions: installed_content::intelligence::Intelligence::default(),
            upgrades: installed_content::intelligence::AgencyUpgrades::default(),
            upgrade_content_error: None,
            warnings,
            agency_refusals: Mutex::default(),
            defense: OnceLock::default(),
            artwork: Mutex::default(),
        }
    }

    /// Agency comparison neither shows nor reads operation definitions. Track
    /// agency text at its producer: operation tokens and sprites can use the
    /// same identifiers as the native agency localisation keys.
    pub fn agency_refusal_lines(&self) -> Vec<String> {
        let mut lines = self
            .agency_refusals
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lines("texts are");
        lines.extend(self.catalogue.reader_warning_lines());
        lines
    }

    fn agency_text(&self, key: &str, arguments: &[(&str, Argument<'_>)]) -> Option<String> {
        self.catalogue
            .localised_with_refusals(key, arguments, View::Plain, &self.agency_refusals)
    }

    fn refuse_agency(&self, reason: &str, key: &str) {
        self.agency_refusals
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .note(reason, key);
        self.catalogue.refuse(reason, key);
    }

    /// The uncreated agency's default-name key. Displayed bytes always come
    /// from recorded content.
    pub fn default_agency_name(&self) -> Option<String> {
        self.agency_text("AGENCY_NAME", &[])
    }

    /// The agency screen's fixed5 LEVEL argument: content controls its
    /// precision and suffix.
    pub fn defense_text(&self, raw: i64) -> Option<String> {
        self.agency_text("AGENCY_DEFENSE_LEVEL", &[("LEVEL", Argument::Fixed5(raw))])
    }

    /// The same content template's label, without its LEVEL number or separator.
    pub fn defense_label(&self) -> Option<String> {
        self.defense_template()
            .map(|template| template.label.clone())
    }

    /// Keep the full LEVEL output, including authored precision and numeric
    /// suffixes; never infer it by parsing rendered digits.
    pub fn defense_value(&self, text: &str) -> Option<String> {
        let template = self.defense_template()?;
        let value = text
            .strip_prefix(&template.prefix)
            .and_then(|text| text.strip_suffix(&template.suffix));
        if value.is_none() {
            self.refuse_agency(
                "defence template's rendered LEVEL boundaries differ from its fixed-point output",
                "AGENCY_DEFENSE_LEVEL",
            );
        }
        value.map(str::to_owned)
    }

    fn defense_template(&self) -> Option<&DefenseTemplate> {
        self.defense
            .get_or_init(|| DefenseTemplate::load(self))
            .as_ref()
    }

    /// An operation stores its name/icon. The stored name localises without
    /// event-scope expansion. Missing or refused content remains unknown.
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
        artwork.name = self.operation_name(token).ok();
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
        let result = (artwork, reason);
        // Cache authored definitions once each; the product budgets its full reply.
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
    fn intelligence_reader_notes_are_shared_and_do_not_enter_product_checks() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/operations/mod.txt",
                "mod_operation = { operation_target = { target_array = unsupported } }",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n AGENCY_NAME:0 \"Custom agency\"\n AGENCY_DEFENSE_LEVEL:0 \"Protection: $LEVEL|.1$\"\n",
            ),
        ]);
        let catalogue = Arc::new(catalogue);
        let operations = IntelligenceContent::load(Arc::clone(&catalogue)).unwrap();
        assert_eq!(operations.definitions.operation_refusals.len(), 1);
        let notes = operations.check_reader_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, crate::content_check::Kind::Unsettled);
        assert!(notes[0].message.contains("target_array"));
        assert!(operations.check_operations().is_empty());
        let agency_notes = operations.check_agency();
        assert_eq!(agency_notes.len(), 1);
        assert_eq!(agency_notes[0].kind, crate::content_check::Kind::Offline);
        assert_eq!(agency_notes[0].subject, "AGENCY_DEFENSE_LEVEL");

        let agency = IntelligenceContent::load_agency(catalogue);
        assert!(agency.definitions.operation_refusals.is_empty());
        assert!(agency.definitions.operations.is_empty());
        assert_eq!(
            agency.default_agency_name().as_deref(),
            Some("Custom agency")
        );
        assert_eq!(agency.defense_label().as_deref(), Some("Protection"));
        assert_eq!(agency.check_agency().len(), 1);
    }

    #[test]
    fn agency_preparation_does_not_read_operation_files() {
        let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/operations/mod.txt",
                "mod_operation = { name = MOD_OPERATION icon = mod_sprite }",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n AGENCY_NAME:0 \"Custom agency\"\n AGENCY_DEFENSE_LEVEL:0 \"Protection: $LEVEL|.1$\"\n",
            ),
        ]);
        std::fs::remove_file(folder.0.join("common/operations/mod.txt")).unwrap();
        let catalogue = Arc::new(catalogue);
        assert!(IntelligenceContent::load(Arc::clone(&catalogue)).is_err());
        let agency = IntelligenceContent::load_agency(catalogue);
        assert_eq!(
            agency.default_agency_name().as_deref(),
            Some("Custom agency")
        );
        assert_eq!(agency.defense_label().as_deref(), Some("Protection"));
        let notes = agency.check_agency();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, crate::content_check::Kind::Offline);
    }

    #[test]
    fn intelligence_shared_notes_keep_native_deferred_kind() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
            "common/operations/mod.txt",
            "custom = { on_start = { free_operative = { operative = custom_name } } }",
        )]);
        let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
        let notes = content.check_reader_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].kind, crate::content_check::Kind::Unsupported);
        assert!(
            notes[0]
                .message
                .contains("Deferred effect `free_operative`")
        );
        let operation_notes = content.check_operations();
        assert!(
            operation_notes
                .iter()
                .all(|note| note.message != notes[0].message)
        );
        let agency_notes = content.check_agency();
        assert!(
            agency_notes
                .iter()
                .all(|note| note.message != notes[0].message)
        );
    }

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
    fn intelligence_content_operation_artwork_keeps_authored_and_localised_empty_names() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/operations/mod.txt",
                "authored_empty = { name = \"\" } localised_empty = { name = EMPTY_OPERATION } refused_name = { name = REFUSED_OPERATION }",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n EMPTY_OPERATION:0 \"\"\n REFUSED_OPERATION:0 \"\u{a4}\"\n",
            ),
        ]);
        let content = IntelligenceContent::load(Arc::new(catalogue)).unwrap();
        for token in ["authored_empty", "localised_empty"] {
            assert_eq!(content.operation_name(token).unwrap(), "");
            assert_eq!(content.operation_artwork(token).0.name.as_deref(), Some(""));
        }
        assert_eq!(content.operation_artwork("refused_name").0.name, None);
        assert_eq!(content.operation_artwork("absent_definition").0.name, None);
    }

    #[test]
    fn intelligence_content_modded_operation_name_art_and_uncreated_agency_are_content_resolved() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        let refusals = content.catalogue.refusal_lines();
        assert_eq!(refusals.len(), 1);
        assert!(refusals[0].contains("1 texts are not shown"));
        assert!(refusals[0].contains("AGENCY_DEFENSE_LEVEL"));
        assert!(refusals[0].contains("required agency defence localisation key missing"));
        assert_eq!(content.agency_refusal_lines(), refusals);
    }

    #[test]
    fn intelligence_content_agency_name_refusal_keeps_its_live_wording() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n AGENCY_NAME:0 \"Custom\u{00a4}agency\"\n",
        )]);
        let content = IntelligenceContent::load_agency(Arc::new(catalogue));
        assert_eq!(content.default_agency_name(), None);
        let refusals = content.catalogue.refusal_lines();
        assert_eq!(refusals.len(), 1);
        assert!(refusals[0].contains("AGENCY_NAME"));
        assert!(refusals[0].contains("control byte 0x14"));
        assert_eq!(content.agency_refusal_lines(), refusals);
    }
}
