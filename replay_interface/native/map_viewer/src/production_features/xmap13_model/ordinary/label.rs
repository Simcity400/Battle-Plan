use crate::Result;
use snapshot::design_catalog::DesignByteObservation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LabelIssue {
    TopLevelMiss(replay_core::LocalisationMiss),
    NestedMiss(String),
    DepthLimit,
    ByteLimit,
    UnmatchedDollar,
}

pub(super) struct LocalizedLabel {
    pub text: String,
    pub issues: Vec<LabelIssue>,
}

pub(super) struct ModelLabelCatalog {
    lease: crate::ContentAdmissionLease,
    sources: Vec<portable_content_manifest::UsedContentKey>,
    selected_language: String,
    language: String,
    table: std::collections::BTreeMap<String, String>,
}

impl ModelLabelCatalog {
    // The frame owns the OnceLock so unobserved label branches do not load content.
    pub(super) fn load(
        resolver: &crate::GameFileResolver,
        admission: &portable_content_manifest::ValidatedContentAdmission,
    ) -> Result<Self> {
        use portable_content_manifest::authored_ui::{canonical_localisation_language, safe_language_directory};
        let lease = resolver.content_admission_lease()
            .ok_or("XMAP13 model labels require content admission")?;
        if lease.manifest_digest_sha256() != Some(admission.manifest_digest_sha256()) {
            return Err("XMAP13 model label resolver and admission disagree".into());
        }
        let selected_language = copy_label_text(admission.localisation_language())?;
        let language = canonical_localisation_language(&selected_language)?;
        let directory = safe_language_directory(&selected_language)?;
        let sources = lease.require_complete_admitted_dir(
            &format!("localisation/{directory}"), Some("yml"), true,
            "XMAP13 model label selected-language localisation namespace",
        )?;
        let mut table = std::collections::BTreeMap::new();
        let header = format!("{language}:");
        for source in &sources {
            if admission.require_used_content(&source.normalized_game_relative_path)? != source {
                return Err("XMAP13 model label source disagrees with content admission".into());
            }
            let bytes = resolver.read_admitted_used_content(source)?;
            let text = std::str::from_utf8(&bytes).map_err(|error| format!(
                "XMAP13 model label localisation {} is not valid UTF-8: {error}",
                source.normalized_game_relative_path,
            ))?;
            crate::map_text::merge_localisation_source_exact(
                &mut table, text, &source.normalized_game_relative_path, &header,
            )?;
        }
        // A witnessed empty namespace is an empty native table: lookups still name misses.
        Ok(Self { lease, sources, selected_language, language, table })
    }

    pub(super) fn checked(
        &self, resolver: &crate::GameFileResolver,
        admission: &portable_content_manifest::ValidatedContentAdmission,
    ) -> Result<&Self> {
        if resolver.content_admission_lease().as_ref() != Some(&self.lease)
            || self.lease.manifest_digest_sha256() != Some(admission.manifest_digest_sha256())
            || self.selected_language != admission.localisation_language()
        {
            return Err("XMAP13 model labels belong to another content admission or language".into());
        }
        for source in &self.sources {
            if admission.require_used_content(&source.normalized_game_relative_path)? != source {
                return Err("XMAP13 model label source identity changed".into());
            }
        }
        Ok(self)
    }

    pub(super) fn plain(
        &self, resolver: &crate::GameFileResolver,
        admission: &portable_content_manifest::ValidatedContentAdmission, key: &str,
    ) -> Result<LocalizedLabel> {
        self.checked(resolver, admission)?;
        localize_zero_arguments(&self.table, &self.language, key)
    }

    // The caller proves this exact raw TAG spelling. Viewer and helper-country operands
    // are separate inputs; no country-object or A27 registry-index conversion occurs here.
    pub(super) fn country(
        &self, resolver: &crate::GameFileResolver,
        admission: &portable_content_manifest::ValidatedContentAdmission, exact_raw_tag: &str, key: &str,
    ) -> Result<LocalizedLabel> {
        self.checked(resolver, admission)?;
        country_localize_zero_arguments(&self.table, &self.language, exact_raw_tag, key)
    }
}

fn copy_label_text(value: &str) -> Result<String> {
    let mut copy = String::new();
    copy.try_reserve(value.len()).map_err(|_| "XMAP13 model label text allocation refused")?;
    copy.push_str(value);
    Ok(copy)
}

// These calls have zero supplied formatting variables. Only dollars are interpreted;
// runtime bracket functions, color/icon bytes and pipe characters remain native input.
pub(super) fn localize_zero_arguments(
    table: &std::collections::BTreeMap<String, String>, language: &str, key: &str,
) -> Result<LocalizedLabel> {
    const LIMIT: usize = 0x1800;
    struct Formatter<'a> {
        table: &'a std::collections::BTreeMap<String, String>,
        language: &'a str,
        output: Vec<u8>,
        issues: Vec<LabelIssue>,
    }
    impl Formatter<'_> {
        fn issue(&mut self, issue: LabelIssue) -> Result<()> {
            if !self.issues.contains(&issue) {
                self.issues.try_reserve(1).map_err(|_| "XMAP13 label diagnostic allocation refused")?;
                self.issues.push(issue);
            }
            Ok(())
        }
        fn append(&mut self, bytes: &[u8]) -> Result<()> {
            let count = bytes.len().min(LIMIT - self.output.len());
            self.output.extend_from_slice(&bytes[..count]);
            if count < bytes.len() { self.issue(LabelIssue::ByteLimit)?; }
            Ok(())
        }
        fn expand(&mut self, key: &str, root: bool, depth: u32) -> Result<()> {
            if depth >= 32 { return self.issue(LabelIssue::DepthLimit); }
            let table = self.table;
            let Some(value) = table.get(key) else {
                if root {
                    self.issue(LabelIssue::TopLevelMiss(replay_core::LocalisationMiss::effective_vfs(key, self.language)))?;
                    self.append(key.as_bytes())?;
                } else {
                    let missing = copy_label_text(key)?;
                    self.issue(LabelIssue::NestedMiss(missing))?;
                }
                return Ok(());
            };
            let mut tail = value.split('\0').next().expect("split always has one field");
            while !tail.is_empty() {
                let Some(open) = tail.find('$') else { return self.append(tail.as_bytes()); };
                let Some(relative_close) = tail[open + 1..].find('$') else {
                    self.issue(LabelIssue::UnmatchedDollar)?;
                    return self.append(tail.as_bytes());
                };
                let close = open + 1 + relative_close;
                if close == open + 1 {
                    self.append(&tail.as_bytes()[..close])?;
                } else {
                    self.append(&tail.as_bytes()[..open])?;
                    if self.output.len() == LIMIT { return self.issue(LabelIssue::ByteLimit); }
                    self.expand(&tail[open + 1..close], false, depth + 1)?;
                }
                if self.output.len() == LIMIT { return self.issue(LabelIssue::ByteLimit); }
                tail = &tail[close + 1..];
            }
            Ok(())
        }
    }
    let mut formatter = Formatter { table, language, output: Vec::new(), issues: Vec::new() };
    formatter.output.try_reserve(LIMIT).map_err(|_| "XMAP13 localized label output allocation refused")?;
    let key = key.split('\0').next().expect("split always has one field");
    if !key.is_empty() { formatter.expand(key, true, 0)?; }
    let text = String::from_utf8(formatter.output)
        .map_err(|_| "XMAP13 native localized label byte boundary is not valid UTF-8")?;
    Ok(LocalizedLabel { text, issues: formatter.issues })
}

pub(super) fn country_localize_zero_arguments(
    table: &std::collections::BTreeMap<String, String>, language: &str, tag: &str, key: &str,
) -> Result<LocalizedLabel> {
    let mut qualified = String::new();
    qualified.try_reserve(tag.len().checked_add(key.len()).and_then(|n| n.checked_add(1))
        .ok_or("XMAP13 country label key length overflow")?)
        .map_err(|_| "XMAP13 country label key allocation refused")?;
    qualified.push_str(tag);
    qualified.push('_');
    qualified.push_str(key);
    let probe = qualified.split('\0').next().expect("split always has one field");
    localize_zero_arguments(table, language, if table.contains_key(probe) { probe } else { key })
}

fn compose_model_variant_label<'a>(display: &str,
    localized_equipment_name: impl FnOnce() -> Result<String>,
    append: &DesignByteObservation,
    suffix: impl FnOnce() -> Result<&'a str>,
) -> Result<String> {
    let mut label = if display.is_empty() {
        localized_equipment_name()?
    } else {
        copy_label_text(display)?
    };
    // Native140bcb1ab tests the full byte before140bcb1b1 tests suffix length.
    let append = match append {
        DesignByteObservation::Measured { raw_u8 } => *raw_u8,
        DesignByteObservation::Unavailable { reason } =>
            return Err(format!("XMAP13 variant append-name byte is unavailable: {reason:?}").into()),
    };
    if append == 0 { return Ok(label); }
    let suffix = suffix()?;
    if !suffix.is_empty() {
        label.try_reserve(suffix.len().checked_add(1).ok_or("XMAP13 variant label length overflow")?)
            .map_err(|_| "XMAP13 variant label suffix allocation refused")?;
        label.push(' ');
        label.push_str(suffix);
    }
    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapshot::design_catalog::DesignFieldRefusal;

    #[test]
    fn raw_label_whitespace_and_nonboolean_append_bytes_preserve_native_order() {
        let yes = DesignByteObservation::Measured { raw_u8: 0xff };
        assert_eq!(compose_model_variant_label("  ", || panic!("nonempty display must not localize"),
            &yes, || Ok(" S ")).unwrap(), "    S ");
        assert_eq!(compose_model_variant_label("B", || panic!("unused"), &yes, || Ok("S")).unwrap(), "B S");
        assert_eq!(compose_model_variant_label("B", || panic!("unused"), &yes, || Ok("")).unwrap(), "B");
        assert_eq!(compose_model_variant_label("", || Ok("Localized equipment".into()),
            &yes, || Ok("S")).unwrap(), "Localized equipment S");
        assert_eq!(compose_model_variant_label("B", || panic!("unused"),
            &DesignByteObservation::Measured { raw_u8: 0 }, || panic!("disabled suffix is not read")).unwrap(), "B");
    }

    #[test]
    fn label_refusals_follow_base_then_byte_then_suffix_without_record_name_fallback() {
        let unavailable = DesignByteObservation::Unavailable {
            reason: DesignFieldRefusal { kind: "unreadable".into(), field: "append_variant_name".into() },
        };
        let error = compose_model_variant_label("", || Err("localized-name-unavailable".into()),
            &unavailable, || panic!("earlier base failure")).unwrap_err();
        assert_eq!(error.to_string(), "localized-name-unavailable");
        let error = compose_model_variant_label("B", || panic!("unused"),
            &unavailable, || panic!("byte failure precedes even witnessed empty suffix")).unwrap_err();
        assert!(error.to_string().contains("append-name byte"));
        let error = compose_model_variant_label("B", || panic!("unused"),
            &DesignByteObservation::Measured { raw_u8: 2 }, || Err("suffix-observation-unavailable".into())).unwrap_err();
        assert_eq!(error.to_string(), "suffix-observation-unavailable");
    }
}


// The caller supplies its existing admitted equipment localization at the reached branch.
pub(super) fn variant_model_label(
    variant: &snapshot::design_catalog::NativeDesignerVariantInputs,
    localized_equipment_name: impl FnOnce() -> Result<String>,
) -> Result<String> {
    use snapshot::design_catalog::DesignTextObservation;
    let display = match &variant.display_name {
        DesignTextObservation::Measured { value, .. } => value.as_str(),
        DesignTextObservation::Unavailable { reason } =>
            return Err(format!("XMAP13 variant display-name observation is unavailable: {reason:?}").into()),
    };
    compose_model_variant_label(display, localized_equipment_name, &variant.append_variant_name, || {
        match &variant.variant_name {
            DesignTextObservation::Measured { value, .. } => Ok(value.as_str()),
            DesignTextObservation::Unavailable { reason } =>
                Err(format!("XMAP13 variant suffix observation is unavailable: {reason:?}").into()),
        }
    })
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use snapshot::design_catalog::{DesignFieldRefusal, DesignTextObservation};
    use super::super::tests::variant;

    #[test]
    fn exact_variant_text_projection_is_used_instead_of_merged_record_name() {
        let mut input = variant(7, "rifle", "GER", 0);
        input.record.name = Some("Merged record text must never enter model grouping".into());
        input.display_name = DesignTextObservation::Measured { value:"  ".into(), utf8_hex:"2020".into() };
        input.variant_name = DesignTextObservation::Measured { value:" S ".into(), utf8_hex:"205320".into() };
        input.append_variant_name = DesignByteObservation::Measured { raw_u8:255 };
        assert_eq!(variant_model_label(&input, || panic!("raw whitespace is nonempty")).unwrap(), "    S ");
        input.variant_name = DesignTextObservation::Unavailable {
            reason:DesignFieldRefusal { kind:"unreadable".into(), field:"variant_name".into() },
        };
        input.append_variant_name = DesignByteObservation::Measured { raw_u8:0 };
        assert_eq!(variant_model_label(&input, || panic!("unused")).unwrap(), "  ");
        input.append_variant_name = DesignByteObservation::Measured { raw_u8:2 };
        assert!(variant_model_label(&input, || panic!("unused")).unwrap_err().to_string().contains("suffix observation"));
        input.display_name = DesignTextObservation::Measured { value:"".into(), utf8_hex:"".into() };
        input.append_variant_name = DesignByteObservation::Measured { raw_u8:0 };
        assert_eq!(variant_model_label(&input, || Ok("actual equipment localization".into())).unwrap(), "actual equipment localization");
    }
}

// 14191ab30 assigns first-seen unsigned byte ranks before its stable ordering pass.
// Equal ranks after wrap remain distinct labels; sorting by strings or usize changes row one.
pub(super) fn order_by_native_label_rank<T>(rows: &mut [T], label: impl Fn(&T) -> &str) -> Result<()> {
    let mut ranks = Vec::new();
    ranks.try_reserve(rows.len()).map_err(|_| "XMAP13 label rank allocation refused")?;
    let mut next = 0u8;
    for (index, row) in rows.iter().enumerate() {
        let rank = match rows[..index].iter().position(|prior| label(prior) == label(row)) {
            Some(previous) => ranks[previous],
            None => { let value = next; next = next.wrapping_add(1); value },
        };
        ranks.push(rank);
    }
    // Stable insertion uses the already computed native keys without another allocation.
    for index in 1..rows.len() {
        let mut current = index;
        while current != 0 && ranks[current] < ranks[current - 1] {
            ranks.swap(current, current - 1);
            rows.swap(current, current - 1);
            current -= 1;
        }
    }
    Ok(())
}

// 1419173d0 counts each row's input label before mutation. Newly decorated labels
// do not reserve a key; the inner pass includes the default row in the same sequence.
pub(super) fn decorate_model_labels<T>(
    rows: &mut [T], mut label: impl FnMut(&mut T) -> &mut String,
) -> Result<()> {
    let mut counts = std::collections::HashMap::<String, i32>::new();
    counts.try_reserve(rows.len()).map_err(|_| "XMAP13 label decoration count allocation refused")?;
    for row in rows {
        let text = label(row);
        let count = counts.entry(copy_label_text(text)?).or_insert(0);
        if let Some(occurrence) = next_decorated_occurrence(count) {
            let number = native_decoration_number(occurrence)?;
            let additional = number.len().checked_add(1)
                .ok_or("XMAP13 label decoration length overflow")?;
            text.try_reserve(additional).map_err(|_| "XMAP13 label decoration text allocation refused")?;
            text.push(' ');
            text.push_str(&number);
        }
    }
    Ok(())
}

fn next_decorated_occurrence(count: &mut i32) -> Option<u32> {
    *count = count.wrapping_add(1);
    (*count > 1).then_some(*count as u32)
}

// 1422b8dd0's reached literals, closed by CMB_DECORATION_LITERAL_CUSTODY.json
// (6207 bytes, SHA256 9f2b6d843643b55ef348e549a761945db23c4e4af7562199c467c05631e1059a).
// Greedy unsigned subtraction has no 3999 cap or special overflow representation.
fn native_decoration_number(mut value: u32) -> Result<String> {
    const TOKENS: [(u32, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"),
        (90, "XC"), (50, "L"), (40, "XL"), (10, "X"), (9, "IX"),
        (5, "V"), (4, "IV"), (1, "I"),
    ];
    let mut remaining = value;
    let mut length = 0usize;
    for (denomination, token) in TOKENS {
        let repetitions = usize::try_from(remaining / denomination)?;
        length = repetitions.checked_mul(token.len()).and_then(|n| length.checked_add(n))
            .ok_or("XMAP13 decoration numeral length overflow")?;
        remaining %= denomination;
    }
    let mut number = String::new();
    number.try_reserve(length).map_err(|_| "XMAP13 decoration numeral allocation refused")?;
    for (denomination, token) in TOKENS {
        while value >= denomination {
            number.push_str(token);
            value -= denomination;
        }
    }
    Ok(number)
}

#[cfg(test)]
mod decoration_tests {
    use super::*;

    #[test]
    fn decoration_counts_original_exact_labels_and_preserves_entity_payload_order() {
        let mut rows = ["A", "A", "A II", "A", "a", " A", "", ""]
            .into_iter().enumerate().map(|(index, label)| (label.to_owned(), index, -(index as i64)))
            .collect::<Vec<_>>();
        decorate_model_labels(&mut rows, |row| &mut row.0).unwrap();
        assert_eq!(rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
            ["A", "A II", "A II", "A III", "a", " A", "", " II"]);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!((row.1, row.2), (index, -(index as i64)));
        }
    }

    #[test]
    fn inner_default_and_outer_passes_observe_prior_decoration() {
        let mut rows = ["Default", "Default", "Default II"].map(str::to_owned);
        decorate_model_labels(&mut rows, |row| row).unwrap();
        assert_eq!(rows, ["Default", "Default II", "Default II"]);
        order_by_native_label_rank(&mut rows, String::as_str).unwrap();
        decorate_model_labels(&mut rows, |row| row).unwrap();
        assert_eq!(rows, ["Default", "Default II", "Default II II"]);
    }

    #[test]
    fn all_thirteen_tokens_and_unbounded_thousands_follow_unsigned_greedy_subtraction() {
        for (number, expected) in [
            (0, ""), (1, "I"), (4, "IV"), (5, "V"), (9, "IX"), (10, "X"),
            (40, "XL"), (50, "L"), (90, "XC"), (100, "C"), (400, "CD"),
            (500, "D"), (900, "CM"), (1000, "M"), (1984, "MCMLXXXIV"),
            (3999, "MMMCMXCIX"), (4000, "MMMM"), (4999, "MMMMCMXCIX"),
        ] {
            assert_eq!(native_decoration_number(number).unwrap(), expected);
        }
        let mut rows = vec![String::from("A"); 4000];
        decorate_model_labels(&mut rows, |row| row).unwrap();
        assert_eq!(rows[3999], "A MMMM");
    }

    #[test]
    fn count_wrap_uses_signed_comparison_before_unsigned_number_conversion() {
        let mut count = i32::MAX - 1;
        assert_eq!(next_decorated_occurrence(&mut count), Some(i32::MAX as u32));
        assert_eq!(next_decorated_occurrence(&mut count), None);
        assert_eq!(count, i32::MIN);
        count = -1;
        assert_eq!(next_decorated_occurrence(&mut count), None);
        assert_eq!(next_decorated_occurrence(&mut count), None);
        assert_eq!(next_decorated_occurrence(&mut count), Some(2));
    }
}

#[cfg(test)]
mod grouping_tests {
    use super::*;

    #[test]
    fn distinct_labels_wrap_without_collapsing_or_lexical_sorting() {
        let mut rows = (0..257).map(|ordinal| (format!("label-{ordinal:03}"), ordinal)).collect::<Vec<_>>();
        // A repeated label must reuse its original rank even after next rank wrapped.
        rows.push(("label-001".into(), 257));
        order_by_native_label_rank(&mut rows, |row| row.0.as_str()).unwrap();
        assert_eq!(rows.iter().take(5).map(|row| row.1).collect::<Vec<_>>(), [0,256,1,257,2]);
        assert_eq!(rows.len(), 258);
        assert_eq!(rows.last().unwrap().1, 255);
    }

    #[test]
    fn exact_append_operand_changes_row_one_only_through_actual_label_identity() {
        let mut plain = (0..256).map(|ordinal| (format!("label-{ordinal}"), ordinal)).collect::<Vec<_>>();
        plain[200].0 = "B S".into();
        let build = |append| compose_model_variant_label("B", || panic!("display is nonempty"),
            &DesignByteObservation::Measured { raw_u8: append }, || Ok("S")).unwrap();
        let mut appended = plain.clone();
        plain.push((build(0),256));
        appended.push((build(0xff),256));
        order_by_native_label_rank(&mut plain, |row| row.0.as_str()).unwrap();
        order_by_native_label_rank(&mut appended, |row| row.0.as_str()).unwrap();
        assert_eq!(plain[1].1,256);
        assert_eq!(appended[1].1,1);
        assert_eq!(appended.iter().filter(|row| row.0 == "B S").map(|row| row.1).collect::<Vec<_>>(),[200,256]);
    }
}

#[cfg(test)]
mod localization_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn zero_argument_expansion_keeps_native_misses_and_literal_runtime_markup() {
        let table = BTreeMap::from([
            ("ROOT".into(), "A $MISSING$ B $$ $K|0$ [X.GetName] §Y£icon£".into()),
            ("K|0".into(), "exact pipe key".into()),
        ]);
        let label = localize_zero_arguments(&table,"l_english","ROOT").unwrap();
        assert_eq!(label.text,"A  B $ exact pipe key [X.GetName] §Y£icon£");
        assert_eq!(label.issues,[LabelIssue::NestedMiss("MISSING".into())]);
        let missing = localize_zero_arguments(&table,"l_english","ABSENT").unwrap();
        assert_eq!(missing.text,"ABSENT");
        assert!(matches!(&missing.issues[..], [LabelIssue::TopLevelMiss(miss)] if miss.key == "ABSENT"));
        assert_eq!(localize_zero_arguments(&table,"l_english","").unwrap().text, "");
    }

    #[test]
    fn country_key_presence_wins_even_when_its_measured_text_is_empty() {
        let table = BTreeMap::from([
            ("UNIT".into(), "unqualified".into()), ("GER_UNIT".into(), "".into()),
            ("FRA_UNIT".into(), "$UNIT$ [unchanged]".into()),
        ]);
        let german = country_localize_zero_arguments(&table,"l_english","GER","UNIT").unwrap();
        assert_eq!(german.text, "");
        assert!(german.issues.is_empty());
        assert_eq!(country_localize_zero_arguments(&table,"l_english","FRA","UNIT").unwrap().text,
            "unqualified [unchanged]");
        assert_eq!(country_localize_zero_arguments(&table,"l_english","ENG","UNIT").unwrap().text,
            "unqualified");
    }

    #[test]
    fn depth_byte_and_c_string_boundaries_preserve_exact_output_or_named_refusal() {
        let mut table = BTreeMap::from([
            ("LOOP".into(), "x$LOOP$".into()),
            ("UNMATCHED".into(), "literal $tail".into()),
            ("NUL".into(), "before\0$MISSING$after".into()),
            ("BOUND".into(), "a".repeat(0x1801)),
        ]);
        let recursive = localize_zero_arguments(&table,"l_english","LOOP").unwrap();
        assert_eq!(recursive.text,"x".repeat(32));
        assert_eq!(recursive.issues,[LabelIssue::DepthLimit]);
        let unmatched = localize_zero_arguments(&table,"l_english","UNMATCHED").unwrap();
        assert_eq!(unmatched.text,"literal $tail");
        assert_eq!(unmatched.issues,[LabelIssue::UnmatchedDollar]);
        let nul = localize_zero_arguments(&table,"l_english","NUL\0ignored").unwrap();
        assert_eq!(nul.text,"before");
        assert!(nul.issues.is_empty());
        let bounded = localize_zero_arguments(&table,"l_english","BOUND").unwrap();
        assert_eq!(bounded.text.len(),0x1800);
        assert_eq!(bounded.issues,[LabelIssue::ByteLimit]);
        table.insert("SPLIT".into(),format!("{}é","a".repeat(0x17ff)));
        assert!(localize_zero_arguments(&table,"l_english","SPLIT").err().unwrap().to_string()
            .contains("byte boundary is not valid UTF-8"));
    }
}

#[cfg(test)]
mod admitted_catalog_tests {
    use super::*;
    use crate::test_support::{admitted_fixture_read_set, temp_test_dir, write_modvfs_fixture};

    fn scope() -> portable_content_manifest::ContentNamespaceScope {
        portable_content_manifest::ContentNamespaceScope {
            directory: "localisation/english".into(), extension: Some("yml".into()), recursive: true,
        }
    }

    #[test]
    fn admitted_catalog_uses_complete_language_tree_and_exact_lease() {
        let root = temp_test_dir("xmap13-model-label-catalog");
        write_modvfs_fixture(&root, "localisation/english/main.yml",
            "\u{feff}l_english:\n UNIT:0 \"plain $TAIL$\"\n GER_UNIT:0 \"\"\n");
        write_modvfs_fixture(&root, "localisation/english/nested/extra.yml",
            "l_english:\n TAIL:0 \"[GetName] $ABSENT$\"\n");
        write_modvfs_fixture(&root, "localisation/french/other.yml",
            "l_french:\n FOREIGN:0 \"must not enter the selected table\"\n");
        let raw = crate::GameFileResolver::vanilla(&root).unwrap();
        let (resolver, admission) = admitted_fixture_read_set(&raw, &Default::default(),
            &[scope()], "xmap13-model-label-catalog");
        let catalog = ModelLabelCatalog::load(&resolver, &admission).unwrap();
        assert_eq!(catalog.sources.len(), 2);
        let label = catalog.plain(&resolver, &admission, "UNIT").unwrap();
        assert_eq!(label.text, "plain [GetName] ");
        assert_eq!(label.issues, [LabelIssue::NestedMiss("ABSENT".into())]);
        assert_eq!(catalog.country(&resolver, &admission, "GER", "UNIT").unwrap().text, "");
        let foreign = catalog.plain(&resolver, &admission, "FOREIGN").unwrap();
        assert_eq!(foreign.text, "FOREIGN");
        assert!(matches!(&foreign.issues[..], [LabelIssue::TopLevelMiss(miss)]
            if miss.key == "FOREIGN"));
        let (replacement, _) = raw.clone().with_content_admission(&admission).unwrap()
            .isolated_content_snapshot().unwrap();
        assert_eq!(replacement.content_admission_lease().unwrap().manifest_digest_sha256(),
            resolver.content_admission_lease().unwrap().manifest_digest_sha256());
        assert!(catalog.plain(&replacement, &admission, "UNIT").err().unwrap().to_string()
            .contains("another content admission"));
        assert!(ModelLabelCatalog::load(&raw, &admission).err().unwrap().to_string()
            .contains("require content admission"));
        drop((catalog, resolver, replacement, raw));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_namespace_witness_is_unavailable_but_witnessed_empty_names_native_miss() {
        let root = temp_test_dir("xmap13-model-label-empty");
        std::fs::create_dir_all(&root).unwrap();
        let raw = crate::GameFileResolver::vanilla(&root).unwrap();
        let (unwitnessed, no_scope) = admitted_fixture_read_set(&raw, &Default::default(),
            &[], "xmap13-model-label-unwitnessed");
        assert!(ModelLabelCatalog::load(&unwitnessed, &no_scope).is_err());
        let (empty, admitted) = admitted_fixture_read_set(&raw, &Default::default(),
            &[scope()], "xmap13-model-label-empty");
        let catalog = ModelLabelCatalog::load(&empty, &admitted).unwrap();
        let missing = catalog.plain(&empty, &admitted, "USE_DEFAULT_MODEL").unwrap();
        assert_eq!(missing.text, "USE_DEFAULT_MODEL");
        assert!(matches!(&missing.issues[..], [LabelIssue::TopLevelMiss(miss)]
            if miss.key == "USE_DEFAULT_MODEL"));
        assert!(catalog.checked(&empty, &no_scope).is_err());
        drop((catalog, unwitnessed, empty, raw));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_qualified_key_bad_language_and_non_utf8_content_are_named_refusals() {
        for (label, first, second, expected) in [
            ("duplicate", b"l_english:\n GER_UNIT:0 \"first\"\n UNIT:0 \"fallback\"\n".as_slice(),
                "l_english:\n GER_UNIT:0 \"second\"\n", "multiple effective definitions"),
            ("language", b"l_french:\n UNIT:0 \"wrong language\"\n".as_slice(),
                "l_english:\n", "header"),
            ("utf8", b"l_english:\n UNIT:0 \"\xff\"\n".as_slice(),
                "l_english:\n", "not valid UTF-8"),
        ] {
            let root = temp_test_dir(&format!("xmap13-model-label-{label}"));
            write_modvfs_fixture(&root, "localisation/english/first.yml", "");
            std::fs::write(root.join("localisation/english/first.yml"), first).unwrap();
            write_modvfs_fixture(&root, "localisation/english/second.yml", second);
            let raw = crate::GameFileResolver::vanilla(&root).unwrap();
            let (resolver, admission) = admitted_fixture_read_set(&raw, &Default::default(),
                &[scope()], &format!("xmap13-model-label-{label}"));
            assert!(ModelLabelCatalog::load(&resolver, &admission).err().unwrap().to_string()
                .contains(expected));
            drop((resolver, raw));
            portable_test_fs_retry::remove_dir_all(root).unwrap();
        }
    }
}
