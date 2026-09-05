
use replay_core::{
    RuntimeTextCatalog, RuntimeTextLayout, RuntimeTextRecord, RuntimeTextSource,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_EXPANSION_DEPTH: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeTextContext {
    pub variables: BTreeMap<String, String>,
    pub data_functions: BTreeMap<String, String>,
    pub dynamic_colors: BTreeMap<String, char>,
    pub exact: Option<ExactRuntimeText>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactRuntimeText {
    pub view_id: String,
    pub element_id: String,
    pub source: RuntimeTextSource,
    pub engine_utf8: String,
    pub layout: RuntimeTextLayout,
}

impl From<&RuntimeTextRecord> for RuntimeTextContext {
    fn from(record: &RuntimeTextRecord) -> Self {
        Self {
            variables: record.variables.clone(),
            data_functions: record.data_functions.clone(),
            dynamic_colors: record.dynamic_colors.clone(),
            exact: Some(ExactRuntimeText {
                view_id: record.view_id.clone(),
                element_id: record.element_id.clone(),
                source: record.source.clone(),
                engine_utf8: record.final_text_utf8.clone(),
                layout: record.layout.clone(),
            }),
        }
    }
}

impl RuntimeTextContext {
    pub fn exact_for_element(&self, view_id: Option<&str>, element_id: &str) -> bool {
        self.exact.as_ref().is_some_and(|exact| {
            exact.element_id == element_id && view_id.map_or(true, |view| exact.view_id == view)
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeTextViewJoin {
    pub by_element: BTreeMap<String, RuntimeTextContext>,
    pub ambiguous_elements: BTreeSet<String>,
}

pub fn join_exact_runtime_text_view(
    catalog: &RuntimeTextCatalog,
    frame_hour: i64,
    pov_country_or_global: &str,
    view_id: &str,
) -> Option<RuntimeTextViewJoin> {
    if !catalog.contexts_complete {
        return None;
    }
    let mut joined = RuntimeTextViewJoin::default();
    for record in catalog.contexts.values().filter(|record| {
        record.frame_hour == frame_hour
            && record.pov_country_or_global == pov_country_or_global
            && record.view_id == view_id
    }) {
        if joined
            .by_element
            .insert(
                record.element_id.clone(),
                RuntimeTextContext::from(record),
            )
            .is_some()
        {
            joined.ambiguous_elements.insert(record.element_id.clone());
        }
    }
    for element in &joined.ambiguous_elements {
        joined.by_element.remove(element);
    }
    Some(joined)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextIssueKind {
    CountryNameInputsUnavailable,
    MissingLocalisationKey,
    MissingRuntimeVariable,
    FormattedVariableRequiresRuntimeValue,
    MissingDataFunction,
    MissingDynamicColor,
    InlineIconRequiresSpriteLayout,
    RuntimeTextIdentityMismatch,
    MissingLocalisationProvenance,
    AmbiguousLocalisationProvenance,
    EngineTextLayoutRefused,
    LocalisationCycle,
    ExpansionDepthExceeded,
    UnterminatedToken,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextIssue {
    pub kind: TextIssueKind,
    pub token: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedText {
    pub text: String,
    pub issues: Vec<TextIssue>,
    pub source_keys: BTreeSet<String>,
}

impl ResolvedText {
    pub fn admitted(&self) -> bool {
        self.issues.is_empty()
    }
}

pub fn resolve_localisation(
    entries: &BTreeMap<String, String>,
    key: &str,
    runtime: &RuntimeTextContext,
) -> ResolvedText {
    resolve_localisation_inner(entries, key, runtime, false)
}

pub fn resolve_localisation_preserving_inline_icons(
    entries: &BTreeMap<String, String>,
    key: &str,
    runtime: &RuntimeTextContext,
) -> ResolvedText {
    resolve_localisation_inner(entries, key, runtime, true)
}

fn resolve_localisation_inner(
    entries: &BTreeMap<String, String>,
    key: &str,
    runtime: &RuntimeTextContext,
    preserve_inline_icons: bool,
) -> ResolvedText {
    if let Some(exact) = &runtime.exact {
        return exact_result(
            exact,
            matches!(
                &exact.source,
                RuntimeTextSource::LocalisationKey(identity)
                    | RuntimeTextSource::AuthoredToken(identity)
                    if identity == key
            ),
            key,
            true,
        );
    }
    let mut state = ExpansionState::new(entries, runtime, preserve_inline_icons);
    state.expand_key(key, 0);
    state.finish()
}

pub fn resolve_template(
    entries: &BTreeMap<String, String>,
    template: &str,
    runtime: &RuntimeTextContext,
) -> ResolvedText {
    if let Some(exact) = &runtime.exact {
        return exact_result(
            exact,
            matches!(
                &exact.source,
                RuntimeTextSource::Literal(identity)
                    | RuntimeTextSource::AuthoredToken(identity)
                    if identity == template
            ),
            template,
            false,
        );
    }
    let mut state = ExpansionState::new(entries, runtime, false);
    state.expand(template, 0);
    state.finish()
}

pub fn resolve_template_preserving_inline_icons(
    entries: &BTreeMap<String, String>,
    template: &str,
    runtime: &RuntimeTextContext,
) -> ResolvedText {
    if let Some(exact) = &runtime.exact {
        return exact_result(
            exact,
            matches!(
                &exact.source,
                RuntimeTextSource::Literal(identity)
                    | RuntimeTextSource::AuthoredToken(identity)
                    if identity == template
            ),
            template,
            false,
        );
    }
    let mut state = ExpansionState::new(entries, runtime, true);
    state.expand(template, 0);
    state.finish()
}

pub fn resolve_element_bound_exact_runtime(runtime: &RuntimeTextContext) -> ResolvedText {
    let Some(exact) = runtime.exact.as_ref() else {
        return ResolvedText {
            text: String::new(),
            issues: vec![TextIssue {
                kind: TextIssueKind::RuntimeTextIdentityMismatch,
                token: "missing exact element-bound runtime text".to_string(),
            }],
            source_keys: BTreeSet::new(),
        };
    };
    if !matches!(&exact.source, RuntimeTextSource::SetterCallerRva(_)) {
        return ResolvedText {
            text: String::new(),
            issues: vec![TextIssue {
                kind: TextIssueKind::RuntimeTextIdentityMismatch,
                token: "element-bound bypass requires a setter-caller source".to_string(),
            }],
            source_keys: BTreeSet::new(),
        };
    }
    ResolvedText {
        text: normalize_engine_markup(&exact.engine_utf8),
        issues: Vec::new(),
        source_keys: BTreeSet::new(),
    }
}

fn exact_result(
    exact: &ExactRuntimeText,
    identity_matches: bool,
    token: &str,
    tracks_localisation_key: bool,
) -> ResolvedText {
    if !identity_matches {
        return ResolvedText {
            text: String::new(),
            issues: vec![TextIssue {
                kind: TextIssueKind::RuntimeTextIdentityMismatch,
                token: token.to_string(),
            }],
            source_keys: BTreeSet::new(),
        };
    }
    let mut source_keys = BTreeSet::new();
    if tracks_localisation_key {
        match &exact.source {
            RuntimeTextSource::LocalisationKey(key) | RuntimeTextSource::AuthoredToken(key) => {
                source_keys.insert(key.clone());
            }
            RuntimeTextSource::Literal(_) | RuntimeTextSource::SetterCallerRva(_) => {}
        }
    }
    ResolvedText {
        text: normalize_engine_markup(&exact.engine_utf8),
        issues: Vec::new(),
        source_keys,
    }
}

pub fn normalize_engine_markup(text: &str) -> String {
    text.chars()
        .map(|ch| if ch == '\u{0011}' { '\u{00a7}' } else { ch })
        .collect()
}

struct ExpansionState<'a> {
    entries: &'a BTreeMap<String, String>,
    runtime: &'a RuntimeTextContext,
    output: String,
    issues: Vec<TextIssue>,
    source_keys: BTreeSet<String>,
    key_stack: Vec<String>,
    preserve_inline_icons: bool,
}

impl<'a> ExpansionState<'a> {
    fn new(
        entries: &'a BTreeMap<String, String>,
        runtime: &'a RuntimeTextContext,
        preserve_inline_icons: bool,
    ) -> Self {
        Self {
            entries,
            runtime,
            output: String::new(),
            issues: Vec::new(),
            source_keys: BTreeSet::new(),
            key_stack: Vec::new(),
            preserve_inline_icons,
        }
    }

    fn finish(mut self) -> ResolvedText {
        self.issues.sort();
        self.issues.dedup();
        ResolvedText {
            text: self.output,
            issues: self.issues,
            source_keys: self.source_keys,
        }
    }

    fn issue(&mut self, kind: TextIssueKind, token: impl Into<String>) {
        self.issues.push(TextIssue {
            kind,
            token: token.into(),
        });
    }

    fn expand_key(&mut self, key: &str, depth: usize) {
        if key.is_empty() {
            return;
        }
        if depth > MAX_EXPANSION_DEPTH {
            self.issue(TextIssueKind::ExpansionDepthExceeded, key);
            return;
        }
        if self.key_stack.iter().any(|active| active == key) {
            self.issue(TextIssueKind::LocalisationCycle, key);
            return;
        }
        let Some(value) = self.entries.get(key) else {
            self.output.push_str(key);
            self.issue(TextIssueKind::MissingLocalisationKey, key);
            return;
        };
        self.source_keys.insert(key.to_string());
        self.key_stack.push(key.to_string());
        self.expand(value, depth + 1);
        self.key_stack.pop();
    }

    fn expand(&mut self, template: &str, depth: usize) {
        if depth > MAX_EXPANSION_DEPTH {
            self.issue(TextIssueKind::ExpansionDepthExceeded, "<template>");
            return;
        }
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\u{00a7}' => self.expand_color(&mut chars),
                '\u{00a3}' => self.expand_icon(&mut chars),
                '$' => self.expand_variable(&mut chars, depth),
                '[' => self.expand_data_function(&mut chars, depth),
                _ => self.output.push(ch),
            }
        }
    }

    fn expand_color<I>(&mut self, chars: &mut std::iter::Peekable<I>)
    where
        I: Iterator<Item = char>,
    {
        if chars.peek() == Some(&'[') {
            chars.next();
            let (expr, closed) = collect_until(chars, ']');
            if !closed {
                self.issue(
                    TextIssueKind::UnterminatedToken,
                    format!("section-sign[{expr}"),
                );
                return;
            }
            if let Some(code) = self.runtime.dynamic_colors.get(&expr) {
                self.output.push('\u{00a7}');
                self.output.push(*code);
            } else {
                self.issue(TextIssueKind::MissingDynamicColor, expr);
            }
            return;
        }
        self.output.push('\u{00a7}');
        if let Some(code) = chars.next() {
            self.output.push(code);
        } else {
            self.issue(TextIssueKind::UnterminatedToken, "section-sign");
        }
    }

    fn expand_icon<I>(&mut self, chars: &mut std::iter::Peekable<I>)
    where
        I: Iterator<Item = char>,
    {
        let mut key = String::new();
        while let Some(next) = chars.peek().copied() {
            if next.is_ascii_alphanumeric() || next == '_' {
                key.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if chars.peek() == Some(&'|') {
            key.push(chars.next().expect("peeked icon separator"));
            while let Some(next) = chars.peek().copied() {
                if next.is_ascii_digit() {
                    key.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
        }
        if self.preserve_inline_icons {
            self.output.push('\u{00a3}');
            self.output.push_str(&key);
        } else {
            self.issue(TextIssueKind::InlineIconRequiresSpriteLayout, key);
        }
    }

    fn expand_variable<I>(&mut self, chars: &mut std::iter::Peekable<I>, depth: usize)
    where
        I: Iterator<Item = char>,
    {
        let (token, closed) = collect_until(chars, '$');
        if !closed {
            self.issue(TextIssueKind::UnterminatedToken, format!("${token}"));
            return;
        }
        if let Some(value) = self.runtime.variables.get(&token) {
            self.expand(value, depth + 1);
        } else if token.contains('|') {
            self.issue(TextIssueKind::FormattedVariableRequiresRuntimeValue, token);
        } else if self.entries.contains_key(&token) {
            self.expand_key(&token, depth + 1);
        } else {
            self.issue(TextIssueKind::MissingRuntimeVariable, token);
        }
    }

    fn expand_data_function<I>(&mut self, chars: &mut std::iter::Peekable<I>, depth: usize)
    where
        I: Iterator<Item = char>,
    {
        let (expr, closed) = collect_until(chars, ']');
        if !closed {
            self.issue(TextIssueKind::UnterminatedToken, format!("[{expr}"));
            return;
        }
        if let Some(value) = self.runtime.data_functions.get(&expr) {
            self.expand(value, depth + 1);
        } else {
            self.issue(TextIssueKind::MissingDataFunction, expr);
        }
    }
}

fn collect_until<I>(chars: &mut std::iter::Peekable<I>, end: char) -> (String, bool)
where
    I: Iterator<Item = char>,
{
    let mut out = String::new();
    for ch in chars.by_ref() {
        if ch == end {
            return (out, true);
        }
        out.push(ch);
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_static_and_exact_runtime_values_expand() {
        let entries = BTreeMap::from([
            ("ROOT".to_string(), "§Y$NESTED$§! [?days|0]".to_string()),
            ("NESTED".to_string(), "Ready".to_string()),
        ]);
        let runtime = RuntimeTextContext {
            data_functions: BTreeMap::from([("?days|0".to_string(), "12".to_string())]),
            ..RuntimeTextContext::default()
        };
        let result = resolve_localisation(&entries, "ROOT", &runtime);
        assert!(result.admitted(), "{:?}", result.issues);
        assert_eq!(result.text, "§YReady§! 12");
        assert_eq!(
            result.source_keys,
            BTreeSet::from(["NESTED".into(), "ROOT".into()])
        );
    }

    #[test]
    fn formatted_value_is_never_guessed() {
        let entries = BTreeMap::from([("PERCENT".to_string(), "$VAL|%0$".to_string())]);
        let missing = resolve_localisation(&entries, "PERCENT", &RuntimeTextContext::default());
        assert_eq!(
            missing.issues,
            vec![TextIssue {
                kind: TextIssueKind::FormattedVariableRequiresRuntimeValue,
                token: "VAL|%0".to_string(),
            }]
        );
        let runtime = RuntimeTextContext {
            variables: BTreeMap::from([("VAL|%0".to_string(), "74%".to_string())]),
            ..RuntimeTextContext::default()
        };
        assert_eq!(
            resolve_localisation(&entries, "PERCENT", &runtime).text,
            "74%"
        );
    }

    #[test]
    fn cycles_dynamic_colors_and_inline_icons_fail_closed() {
        let entries = BTreeMap::from([
            ("A".to_string(), "$B$".to_string()),
            ("B".to_string(), "$A$".to_string()),
            (
                "DYNAMIC".to_string(),
                "§[GetColor]x £political_power".to_string(),
            ),
        ]);
        let cycle = resolve_localisation(&entries, "A", &RuntimeTextContext::default());
        assert!(cycle
            .issues
            .iter()
            .any(|issue| issue.kind == TextIssueKind::LocalisationCycle));
        let dynamic = resolve_localisation(&entries, "DYNAMIC", &RuntimeTextContext::default());
        assert_eq!(
            dynamic
                .issues
                .iter()
                .map(|issue| issue.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                TextIssueKind::MissingDynamicColor,
                TextIssueKind::InlineIconRequiresSpriteLayout,
            ]
        );
    }

    #[test]
    fn layout_specific_resolution_preserves_icons_but_not_other_runtime_gaps() {
        let entries = BTreeMap::from([
            (
                "ICON".to_string(),
                "Before \u{00a3}political_power|2 after".to_string(),
            ),
            (
                "MIXED".to_string(),
                "\u{00a3}army_experience [GetValue]".to_string(),
            ),
        ]);
        let ordinary = resolve_localisation(&entries, "ICON", &RuntimeTextContext::default());
        assert_eq!(
            ordinary.issues,
            vec![TextIssue {
                kind: TextIssueKind::InlineIconRequiresSpriteLayout,
                token: "political_power|2".into(),
            }]
        );
        let layout = resolve_localisation_preserving_inline_icons(
            &entries,
            "ICON",
            &RuntimeTextContext::default(),
        );
        assert!(layout.admitted(), "{:?}", layout.issues);
        assert_eq!(layout.text, "Before \u{00a3}political_power|2 after");

        let mixed = resolve_localisation_preserving_inline_icons(
            &entries,
            "MIXED",
            &RuntimeTextContext::default(),
        );
        assert_eq!(
            mixed.issues,
            vec![TextIssue {
                kind: TextIssueKind::MissingDataFunction,
                token: "GetValue".into(),
            }]
        );
        assert_eq!(mixed.text, "\u{00a3}army_experience ");
    }

    #[test]
    fn exact_authored_token_matches_key_or_template_but_caller_identity_fails_closed() {
        let entries = BTreeMap::new();
        let authored_key = RuntimeTextContext {
            exact: Some(ExactRuntimeText {
                view_id: "topbar".into(),
                element_id: "stability".into(),
                source: RuntimeTextSource::AuthoredToken("STABILITY".into()),
                engine_utf8: "52%".into(),
                layout: RuntimeTextLayout::AuthoredReproducible,
            }),
            ..RuntimeTextContext::default()
        };
        let resolved = resolve_localisation(&entries, "STABILITY", &authored_key);
        assert!(resolved.admitted());
        assert_eq!(resolved.text, "52%");
        assert_eq!(resolved.source_keys, BTreeSet::from(["STABILITY".into()]));

        let authored_literal = RuntimeTextContext {
            exact: Some(ExactRuntimeText {
                view_id: "fixed_window".into(),
                element_id: "value".into(),
                source: RuntimeTextSource::AuthoredToken("$VAL|%0$".into()),
                engine_utf8: "74%".into(),
                layout: RuntimeTextLayout::AuthoredReproducible,
            }),
            ..RuntimeTextContext::default()
        };
        let resolved = resolve_template(&entries, "$VAL|%0$", &authored_literal);
        assert!(resolved.admitted());
        assert_eq!(resolved.text, "74%");
        assert!(resolved.source_keys.is_empty());

        let caller_only = RuntimeTextContext {
            exact: Some(ExactRuntimeText {
                view_id: "fixed_window".into(),
                element_id: "value".into(),
                source: RuntimeTextSource::SetterCallerRva(0x0188_A0DE),
                engine_utf8: "74%".into(),
                layout: RuntimeTextLayout::AuthoredReproducible,
            }),
            ..RuntimeTextContext::default()
        };
        assert_eq!(
            resolve_template(&entries, "$VAL|%0$", &caller_only).issues,
            vec![TextIssue {
                kind: TextIssueKind::RuntimeTextIdentityMismatch,
                token: "$VAL|%0$".into(),
            }]
        );
        let element_bound = resolve_element_bound_exact_runtime(&caller_only);
        assert!(element_bound.admitted());
        assert_eq!(element_bound.text, "74%");
        assert!(element_bound.source_keys.is_empty());
    }

    #[test]
    fn a_missed_top_level_key_renders_as_itself_and_is_named() {
        let entries = BTreeMap::from([("ROOT".to_string(), "$GONE$ here".to_string())]);
        let missed = resolve_localisation(&entries, "NOT_THERE", &RuntimeTextContext::default());
        assert_eq!(missed.text, "NOT_THERE");
        assert_eq!(
            missed.issues,
            vec![TextIssue {
                kind: TextIssueKind::MissingLocalisationKey,
                token: "NOT_THERE".to_string(),
            }]
        );
        assert!(missed.source_keys.is_empty());
        // a nested token that is neither a key nor a bound variable stays unbound
        let nested = resolve_localisation(&entries, "ROOT", &RuntimeTextContext::default());
        assert_eq!(nested.text, " here");
        assert_eq!(
            nested.issues,
            vec![TextIssue {
                kind: TextIssueKind::MissingRuntimeVariable,
                token: "GONE".to_string(),
            }]
        );
    }
}

#[cfg(test)]
mod empty_lookup_tests {
    use super::*;
    #[test]
    fn empty_key_bypasses_the_table_without_a_miss() {
        let entries = BTreeMap::from([(String::new(), "not an engine lookup".into())]);
        for table in [BTreeMap::new(), entries] {
            let result = resolve_localisation(&table, "", &RuntimeTextContext::default());
            assert_eq!(result, ResolvedText::default());
        }
    }
}
