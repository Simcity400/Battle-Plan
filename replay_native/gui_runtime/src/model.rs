//! Raw parsed `.gui`-JSON model (the on-disk shape emitted by the offline
//! Python `.gui` parser, e.g. `game_assets/gui/topbar.json`).
//!
//! This is a *lenient* mirror of the JSON: fields the runtime does not use are
//! ignored (serde default), and the polymorphic spots the real data exhibits are
//! modelled explicitly:
//!   * `text.text` / `text.buttonText` may be a JSON string **or** a number
//!     (placeholder values like `999`, `0`).
//!   * a `size` axis may be an int (px), a string (`"100%%"` percent, or `"-1"`),
//!     and the whole `size` may use `{width,height}` or `{x,y}` keys.
//!   * a `position` axis ([`Coord`]) may be a number (int/decimal px), a `@named`
//!     anchor (`"@fixed_btn_mod_col_0"`), a pixel-from-edge suffix (`"1s"`), a
//!     percent (`"50%"`/`"50%%"`), or an unknown raw token — the typed parser must
//!     ACCEPT all of them (a bare `f32` field rejected `@`/`s`/`%`, killing the
//!     whole file parse). `@named` anchors are RESOLVED against the file's
//!     `@var = N` define table (see [`GuiFile::resolve_anchors`]); an unresolved
//!     anchor keeps its raw token + `resolved: None` (never fabricated to a value).
//!   * `text.maxWidth` / `text.maxHeight` may be a number OR a percent string
//!     (`"100%%"`) — captured as a [`Scalar`], resolved by [`TextBlock::max_w`] /
//!     [`TextBlock::max_h`] (a plain number -> px; a percent -> None, parse-tolerant).
//!   * `orientation`/`origo` casing varies (`UPPER_RIGHT`, `upper_right`,
//!     `UPPER_right`) — normalised at parse-into-scene time, not here.

use std::collections::HashMap;

use serde::Deserialize;

/// The top-level parsed `.gui` view file.
#[derive(Debug, Clone, Deserialize)]
pub struct GuiFile {
    #[serde(default)]
    pub view: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub windows: Vec<RawNode>,
    /// The file's `@var = N` DEFINE table for `@named` position anchors. Clausewitz
    /// `.gui` files declare these constants in the file header (or a shared defs
    /// file); the offline JSON extractor may emit them under `defines` or
    /// `constants` (forward-compatible — absent today). After parse,
    /// [`GuiFile::resolve_anchors`] folds these (plus any externally-supplied shared
    /// defs) into every [`Coord::Anchor`] so a `@`-anchor resolves to its REAL
    /// coordinate; an unresolved anchor honestly keeps `resolved: None`.
    #[serde(default, alias = "constants")]
    pub defines: HashMap<String, f64>,
}

/// One raw element as it appears in JSON. `type` is the Clausewitz element kind.
#[derive(Debug, Clone, Deserialize)]
pub struct RawNode {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub position: Option<Position>,
    /// Present only on containers/icons that declare an explicit box.
    #[serde(default)]
    pub size: Option<Size>,

    #[serde(default)]
    pub orientation: Option<String>,
    #[serde(default)]
    pub origo: Option<String>,

    #[serde(default)]
    pub sprite: Option<String>,
    /// Frame index within a multi-frame strip (1-based in Clausewitz; default 1).
    #[serde(default)]
    pub frame: Option<i64>,

    #[serde(default)]
    pub text: Option<TextBlock>,

    #[serde(default)]
    pub tooltip: Option<Scalar>,

    /// `clipping = yes` -> establishes a scissor rect for descendants.
    #[serde(default)]
    pub clipping: Option<YesNo>,
    /// `alwaystransparent = yes` -> hit-test pass-through.
    #[serde(default, alias = "alwaysTransparent")]
    pub alwaystransparent: Option<YesNo>,

    #[serde(default)]
    pub children: Vec<RawNode>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Position {
    #[serde(default)]
    pub x: Coord,
    #[serde(default)]
    pub y: Coord,
}

/// One POSITION axis value, accepting EVERY Clausewitz `.gui` layout-coordinate form
/// the typed parser used to reject (a bare `f32` field rejected `@`/`s`/`%`, failing
/// the whole file parse). The variants:
///   * [`Coord::Px`] — a literal pixel offset (a JSON number, OR a numeric string,
///     INCLUDING decimals + negatives).
///   * [`Coord::Anchor`] — a `@named` anchor (`"@fixed_btn_mod_col_0"`). The `name`
///     + `raw` token are kept; `resolved` is filled by [`GuiFile::resolve_anchors`]
///     from the file's `@var = N` define table — or stays `None` (HONEST: never a
///     fabricated value) when no define exists. A consumer renders an unresolved
///     anchor at the `0.0` fallback ([`Coord::px`]) as an explicit CHROME fallback,
///     not a fabricated datum.
///   * [`Coord::EdgeOffset`] — a `"Ns"` pixel-from-edge suffix (e.g. `"1s"`). Carried
///     as a pixel value; the suffix marks "measured from the opposite edge" but in
///     this passive layout it resolves to the same pixel offset ([`Coord::px`]).
///   * [`Coord::Percent`] — a `"N%"`/`"N%%"` percent of the parent extent. Kept as a
///     fraction; [`Coord::px`] returns `0.0` for it (a position-percent has no
///     parent base at scene-build time) but the value is retained for a future
///     parent-relative resolve — never rejected, never fabricated.
///   * [`Coord::UnknownRaw`] — any other token (e.g. a malformed `"275k"`); retained
///     verbatim so the file still parses + the token is inspectable, rather than a
///     hard parse failure.
#[derive(Debug, Clone, PartialEq)]
pub enum Coord {
    Px(f32),
    Anchor {
        name: String,
        raw: String,
        resolved: Option<f32>,
    },
    EdgeOffset(f32),
    Percent(f32),
    UnknownRaw(String),
}

impl Default for Coord {
    fn default() -> Self {
        Coord::Px(0.0)
    }
}

impl Coord {
    /// Parse a coordinate token (the STRING form). Numbers are handled by serde
    /// directly; this is the string path (`@name` / `Ns` / `N%`/`N%%` / numeric
    /// string / unknown).
    pub fn parse_str(s: &str) -> Coord {
        let t = s.trim();
        if let Some(name) = t.strip_prefix('@') {
            return Coord::Anchor {
                name: name.to_string(),
                raw: t.to_string(),
                resolved: None,
            };
        }
        // percent: "N%" or "N%%" (Clausewitz doubles the % so a single literal
        // survives string interpolation; both spellings mean the same fraction).
        if t.ends_with('%') {
            let num = t.trim_end_matches('%').trim();
            if let Some(p) = parse_finite_f32(num) {
                return Coord::Percent(p);
            }
            return Coord::UnknownRaw(t.to_string());
        }
        // pixel-from-edge suffix: "Ns" (e.g. "1s").
        if let Some(num) = t.strip_suffix('s').or_else(|| t.strip_suffix('S')) {
            if let Some(v) = parse_finite_f32(num.trim()) {
                return Coord::EdgeOffset(v);
            }
        }
        // a plain numeric string (int OR decimal OR negative).
        if let Some(v) = parse_finite_f32(t) {
            return Coord::Px(v);
        }
        Coord::UnknownRaw(t.to_string())
    }

    /// The resolved PIXEL value for layout. A literal/edge-offset is its number; a
    /// resolved anchor is its resolved coordinate; an UNRESOLVED anchor, a position-
    /// percent, or an unknown token falls back to `0.0` (an explicit chrome fallback,
    /// NOT a fabricated datum — the raw token is preserved on the variant).
    pub fn px(&self) -> f32 {
        match self {
            Coord::Px(v) | Coord::EdgeOffset(v) => *v,
            Coord::Anchor { resolved, .. } => resolved.unwrap_or(0.0),
            Coord::Percent(_) | Coord::UnknownRaw(_) => 0.0,
        }
    }

    /// True iff this coordinate could not be resolved to a real pixel value (an
    /// unresolved `@named` anchor or an unknown token) — used by diagnostics/tests
    /// to assert a known file has zero unresolved anchors once defines are present.
    pub fn is_unresolved(&self) -> bool {
        matches!(
            self,
            Coord::Anchor { resolved: None, .. } | Coord::UnknownRaw(_)
        )
    }

    /// Resolve a `@named` anchor against a define table, in place. A non-anchor (or
    /// an already-resolved anchor) is untouched; an anchor whose name is absent from
    /// the table keeps `resolved: None` (honest — never fabricated).
    fn resolve_against(&mut self, defs: &HashMap<String, f64>) {
        if let Coord::Anchor { name, resolved, .. } = self {
            if resolved.is_none() {
                if let Some(v) = defs.get(name) {
                    *resolved = Some(*v as f32);
                }
            }
        }
    }
}

impl<'de> Deserialize<'de> for Coord {
    fn deserialize<D>(de: D) -> Result<Coord, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // A coordinate axis is a number (int/float) OR a string form. Reuse the
        // lenient [`Scalar`] untagged enum to accept any JSON scalar, then classify.
        let s = Scalar::deserialize(de)?;
        Ok(match s {
            Scalar::Int(i) => Coord::Px(i as f32),
            // A JSON float that is FINITE as f64 can OVERFLOW to f32::INFINITY on the
            // `as f32` cast (e.g. `1e40`). A non-finite coordinate poisons layout math
            // and is a fabricated value — retain the raw token instead (Codex), so it
            // falls back to a finite 0.0 only at the explicit chrome seam.
            Scalar::Float(f) => {
                let v = f as f32;
                if v.is_finite() {
                    Coord::Px(v)
                } else {
                    Coord::UnknownRaw(Scalar::Float(f).as_text())
                }
            }
            Scalar::Str(text) => Coord::parse_str(&text),
            // A bool in a position axis is meaningless (the old f32 field REJECTED it).
            // Don't silently fabricate a `0.0` pixel (Codex 2): retain it as an unknown
            // raw token so it stays parse-tolerant yet visible to diagnostics + falls
            // back to 0.0 only at the explicit chrome seam, never as a measured coord.
            Scalar::Bool(b) => Coord::UnknownRaw(b.to_string()),
        })
    }
}

/// A `size` block. Accepts either `{width,height}` or `{x,y}` key spellings, and
/// each axis may be an int (px) or a string (`"100%%"`, `"-1"`). Captured as raw
/// scalars and interpreted by [`Dimension::from_scalar`] during layout.
#[derive(Debug, Clone, Deserialize)]
pub struct Size {
    #[serde(default)]
    pub width: Option<Scalar>,
    #[serde(default)]
    pub height: Option<Scalar>,
    #[serde(default)]
    pub x: Option<Scalar>,
    #[serde(default)]
    pub y: Option<Scalar>,
}

impl Size {
    /// The width axis under either key spelling.
    pub fn w(&self) -> Option<&Scalar> {
        self.width.as_ref().or(self.x.as_ref())
    }
    /// The height axis under either key spelling.
    pub fn h(&self) -> Option<&Scalar> {
        self.height.as_ref().or(self.y.as_ref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextBlock {
    #[serde(default)]
    pub text: Option<Scalar>,
    #[serde(default)]
    pub button_text: Option<Scalar>,
    #[serde(default, rename = "buttonText")]
    pub button_text_alias: Option<Scalar>,
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    /// `maxWidth`/`maxHeight` may be a number OR a percent string (`"100%%"`) — a
    /// bare `f32` field rejected the percent form, failing the whole file parse.
    /// Captured as a [`Scalar`] and resolved by [`TextBlock::max_w`]/[`max_h`].
    #[serde(default)]
    pub max_width: Option<Scalar>,
    #[serde(default, rename = "maxWidth")]
    pub max_width_alias: Option<Scalar>,
    #[serde(default)]
    pub max_height: Option<Scalar>,
    #[serde(default, rename = "maxHeight")]
    pub max_height_alias: Option<Scalar>,
}

impl TextBlock {
    pub fn raw_text(&self) -> Option<&Scalar> {
        self.text
            .as_ref()
            .or(self.button_text.as_ref())
            .or(self.button_text_alias.as_ref())
    }
    /// The `maxWidth` as a PIXEL value: a plain number resolves to px; a percent
    /// string (`"100%%"`) has no pixel base here so it resolves to `None` (honest —
    /// parse-tolerant, never fabricated). The raw scalar stays available.
    pub fn max_w(&self) -> Option<f32> {
        scalar_px(self.max_width.as_ref().or(self.max_width_alias.as_ref()))
    }
    pub fn max_h(&self) -> Option<f32> {
        scalar_px(self.max_height.as_ref().or(self.max_height_alias.as_ref()))
    }
}

/// Parse a numeric string to an f32 ONLY if it is a FINITE real number. Rust's
/// `f32::from_str` accepts `"inf"`/`"-inf"`/`"infinity"`/`"nan"` (ASCII
/// case-insensitive); those must NOT become measured coordinates/lengths (a NaN/Inf
/// pixel value poisons layout math and is a fabricated datum, not a real measure). A
/// non-finite parse returns `None` so the caller retains the raw token / honestly
/// reports "no pixel value", never fabricates one.
fn parse_finite_f32(s: &str) -> Option<f32> {
    match s.parse::<f32>() {
        Ok(v) if v.is_finite() => Some(v),
        _ => None,
    }
}

/// A `Scalar` length as a pixel value: a number -> px; a numeric string -> px; a
/// percent / non-numeric / non-finite string -> `None` (no pixel base at this layer).
fn scalar_px(s: Option<&Scalar>) -> Option<f32> {
    match s? {
        Scalar::Int(i) => Some(*i as f32),
        // a finite f64 can overflow to f32 inf on the cast — reject non-finite (Codex).
        Scalar::Float(f) => {
            let v = *f as f32;
            v.is_finite().then_some(v)
        }
        Scalar::Str(text) => {
            let t = text.trim();
            if t.ends_with('%') {
                None
            } else {
                parse_finite_f32(t)
            }
        }
        Scalar::Bool(_) => None,
    }
}

/// A JSON scalar that may be a string, integer, float, or bool — used wherever
/// the parser emits "either text or number" (placeholder values, tooltips).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Scalar {
    /// The scalar as a display string (the literal the `.gui` carried).
    pub fn as_text(&self) -> String {
        match self {
            Scalar::Str(s) => s.clone(),
            Scalar::Int(i) => i.to_string(),
            Scalar::Float(f) => {
                // Render integral floats without a trailing `.0`.
                if f.fract() == 0.0 {
                    format!("{}", *f as i64)
                } else {
                    format!("{f}")
                }
            }
            Scalar::Bool(b) => b.to_string(),
        }
    }
}

/// `yes`/`no` (also accepts JSON bools).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum YesNo {
    Bool(bool),
    Word(YesNoWord),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum YesNoWord {
    Yes,
    No,
}

impl YesNo {
    pub fn is_yes(&self) -> bool {
        matches!(self, YesNo::Bool(true) | YesNo::Word(YesNoWord::Yes))
    }
}

impl GuiFile {
    pub fn parse(text: &str) -> Result<GuiFile, String> {
        let mut file: GuiFile = serde_json::from_str(text).map_err(|e| format!("gui json: {e}"))?;
        // Fold the file's own `@var = N` define table into every `@named` anchor so
        // it resolves to a real coordinate. Externally-shared defs can be applied
        // afterward via [`GuiFile::resolve_anchors_with`].
        file.resolve_anchors();
        Ok(file)
    }
    pub fn load(path: &std::path::Path) -> Result<GuiFile, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    /// Resolve every `@named` POSITION anchor in the tree against the file's OWN
    /// `defines`/`constants` table. Idempotent; an anchor whose name is absent keeps
    /// `resolved: None`. Called automatically by [`GuiFile::parse`].
    pub fn resolve_anchors(&mut self) {
        let defs = self.defines.clone();
        for w in &mut self.windows {
            w.resolve_anchors(&defs);
        }
    }

    /// Resolve any STILL-unresolved `@named` anchors against an EXTERNAL shared
    /// define table (a `defs/constants` `.gui` shared across files). File-local
    /// defines (already applied by [`parse`](Self::parse)) take precedence — an
    /// anchor already resolved is untouched; only the remainder is filled here.
    pub fn resolve_anchors_with(&mut self, shared: &HashMap<String, f64>) {
        for w in &mut self.windows {
            w.resolve_anchors(shared);
        }
    }

    /// Count anchors/tokens that remain UNRESOLVED across the whole tree — a
    /// diagnostic for "does this file parse with every anchor resolved?".
    pub fn unresolved_anchor_count(&self) -> usize {
        self.windows
            .iter()
            .map(RawNode::unresolved_anchor_count)
            .sum()
    }
}

impl RawNode {
    /// Resolve this node's + descendants' `@named` POSITION anchors against `defs`.
    fn resolve_anchors(&mut self, defs: &HashMap<String, f64>) {
        if let Some(pos) = self.position.as_mut() {
            pos.x.resolve_against(defs);
            pos.y.resolve_against(defs);
        }
        for c in &mut self.children {
            c.resolve_anchors(defs);
        }
    }

    /// Count unresolved position coordinates in this subtree (inclusive).
    fn unresolved_anchor_count(&self) -> usize {
        let here = self
            .position
            .as_ref()
            .map(|p| p.x.is_unresolved() as usize + p.y.is_unresolved() as usize)
            .unwrap_or(0);
        here + self
            .children
            .iter()
            .map(RawNode::unresolved_anchor_count)
            .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalar_text_as_string_or_number() {
        let j = r#"{"windows":[{"type":"instantTextBoxType","name":"a",
            "text":{"text":999,"font":"f","format":"center","maxWidth":50,"maxHeight":24}}]}"#;
        let g = GuiFile::parse(j).unwrap();
        let n = &g.windows[0];
        assert_eq!(n.kind, "instantTextBoxType");
        assert_eq!(
            n.text.as_ref().unwrap().raw_text().unwrap().as_text(),
            "999"
        );
        assert_eq!(n.text.as_ref().unwrap().max_w(), Some(50.0));
    }

    #[test]
    fn parses_size_in_both_spellings_and_percent() {
        let j = r#"{"windows":[
            {"type":"containerWindowType","name":"root","size":{"width":"100%%","height":"100%%"}},
            {"type":"containerWindowType","name":"b","size":{"x":228,"y":42}}
        ]}"#;
        let g = GuiFile::parse(j).unwrap();
        assert!(
            matches!(g.windows[0].size.as_ref().unwrap().w().unwrap(), Scalar::Str(s) if s=="100%%")
        );
        assert!(matches!(
            g.windows[1].size.as_ref().unwrap().w().unwrap(),
            Scalar::Int(228)
        ));
    }

    // -------------------------------------------------------------------------------
    // FIX 2 — Coord (position) value-form coverage: every previously-rejected form
    // must now PARSE through the shared parser (a bare `f32` field failed the whole
    // file on `@`/`s`/`%`/decimal-string).
    // -------------------------------------------------------------------------------

    #[test]
    fn coord_parses_plain_int_and_decimal_and_negative() {
        // numbers come through serde directly; numeric strings + decimals + negatives.
        assert_eq!(Coord::parse_str("42"), Coord::Px(42.0));
        assert_eq!(Coord::parse_str("-399"), Coord::Px(-399.0));
        assert_eq!(Coord::parse_str("12.5"), Coord::Px(12.5));
        assert_eq!(Coord::parse_str("-0.25"), Coord::Px(-0.25));
    }

    #[test]
    fn coord_parses_named_anchor_keeps_raw_unresolved_by_default() {
        match Coord::parse_str("@fixed_btn_mod_col_0") {
            Coord::Anchor {
                name,
                raw,
                resolved,
            } => {
                assert_eq!(name, "fixed_btn_mod_col_0");
                assert_eq!(raw, "@fixed_btn_mod_col_0");
                assert_eq!(
                    resolved, None,
                    "no define table -> honestly unresolved, NOT zeroed-as-resolved"
                );
            }
            other => panic!("expected Anchor, got {other:?}"),
        }
    }

    #[test]
    fn coord_parses_edge_offset_suffix() {
        assert_eq!(Coord::parse_str("1s"), Coord::EdgeOffset(1.0));
        assert_eq!(Coord::parse_str("-3s"), Coord::EdgeOffset(-3.0));
        assert_eq!(Coord::parse_str("1s").px(), 1.0);
    }

    #[test]
    fn coord_parses_percent_single_and_double() {
        assert_eq!(Coord::parse_str("50%"), Coord::Percent(50.0));
        assert_eq!(Coord::parse_str("100%%"), Coord::Percent(100.0));
    }

    #[test]
    fn coord_unknown_token_is_retained_not_a_parse_failure() {
        assert_eq!(Coord::parse_str("275k"), Coord::UnknownRaw("275k".into()));
        // and an unknown / percent / unresolved-anchor falls back to 0.0 px (chrome).
        assert_eq!(Coord::parse_str("275k").px(), 0.0);
        assert_eq!(Coord::parse_str("100%%").px(), 0.0);
    }

    // -------------------------------------------------------------------------------
    // BUG (Codex H1) — Rust's `f32::from_str` ACCEPTS "inf"/"-inf"/"infinity"/"nan"
    // (ASCII case-insensitive). A non-finite token must NOT become a MEASURED
    // Px/EdgeOffset/Percent coordinate (it would poison layout math with NaN/Inf and
    // is a fabricated value, violating "never fabricated to a value"). It must be
    // retained as `UnknownRaw` — visible, falling back to a finite 0.0 only at the
    // explicit chrome seam, exactly like "275k".
    // -------------------------------------------------------------------------------
    #[test]
    fn coord_non_finite_tokens_are_unknown_not_fabricated_coords() {
        for tok in [
            "inf",
            "-inf",
            "Inf",
            "infinity",
            "-infinity",
            "nan",
            "NaN",
            "NAN",
        ] {
            let c = Coord::parse_str(tok);
            assert!(
                matches!(c, Coord::UnknownRaw(_)),
                "non-finite px token {tok:?} must be UnknownRaw, got {c:?}"
            );
            // the chrome-seam fallback is a FINITE 0.0 (never NaN/Inf reaching layout).
            assert!(
                c.px().is_finite(),
                "{tok:?} px() must be finite, got {}",
                c.px()
            );
            assert_eq!(c.px(), 0.0);
        }
        // edge-offset suffix on a non-finite number ("nans"/"infs") must NOT become a
        // measured EdgeOffset(NaN/Inf) either.
        for tok in ["infs", "nans", "infS", "-infs"] {
            let c = Coord::parse_str(tok);
            assert!(matches!(c, Coord::UnknownRaw(_)), "{tok:?} -> {c:?}");
            assert!(c.px().is_finite());
        }
        // percent of a non-finite number ("inf%%") must NOT become Percent(Inf).
        for tok in ["inf%", "nan%%", "-infinity%"] {
            let c = Coord::parse_str(tok);
            assert!(matches!(c, Coord::UnknownRaw(_)), "{tok:?} -> {c:?}");
            assert!(c.px().is_finite());
        }
        // a numeric string that OVERFLOWS f32 (e.g. "1e40") parses to INFINITY — it must
        // also be rejected as a fabricated coordinate, not become Px(inf).
        for tok in ["1e40", "-1e40", "3.5e38", "1e40s", "1e40%"] {
            let c = Coord::parse_str(tok);
            assert!(
                matches!(c, Coord::UnknownRaw(_)),
                "overflow token {tok:?} -> {c:?}"
            );
            assert!(c.px().is_finite());
        }
        // a normal finite number is unaffected.
        assert_eq!(Coord::parse_str("12.5"), Coord::Px(12.5));
        assert_eq!(Coord::parse_str("3s"), Coord::EdgeOffset(3.0));
        assert_eq!(Coord::parse_str("50%"), Coord::Percent(50.0));
        assert_eq!(Coord::parse_str("1e3"), Coord::Px(1000.0)); // a large-but-finite exp
    }

    #[test]
    fn json_numeric_coord_overflow_is_not_a_fabricated_infinity() {
        // a JSON NUMBER (not a string) that is finite as f64 but overflows the `as f32`
        // cast (e.g. 1e40) must NOT deserialize to Coord::Px(inf) — the serde numeric
        // path must reject it too, not just the string path (Codex follow-up).
        let j = r#"{"windows":[{"type":"containerWindowType","name":"w",
            "position":{"x":1e40,"y":-1e40},
            "size":{"width":1e40,"height":24},
            "text":{"text":"x","maxWidth":1e40,"maxHeight":12}}]}"#;
        let g = GuiFile::parse(j).expect("an overflowing numeric coord stays parse-tolerant");
        let p = g.windows[0].position.as_ref().unwrap();
        assert!(matches!(&p.x, Coord::UnknownRaw(_)), "1e40 x -> {:?}", p.x);
        assert!(matches!(&p.y, Coord::UnknownRaw(_)), "-1e40 y -> {:?}", p.y);
        assert!(p.x.px().is_finite() && p.y.px().is_finite());
        // a finite JSON float is unaffected.
        let j2 = r#"{"windows":[{"type":"containerWindowType","name":"w","position":{"x":12.5,"y":0}}]}"#;
        let g2 = GuiFile::parse(j2).unwrap();
        assert_eq!(g2.windows[0].position.as_ref().unwrap().x, Coord::Px(12.5));
        // and the overflowing maxWidth resolves to None (honest), not an inf px base.
        let tb = g.windows[0].text.as_ref().unwrap();
        assert_eq!(tb.max_w(), None);
        // a finite numeric maxHeight still resolves.
        assert_eq!(tb.max_h(), Some(12.0));
    }

    #[test]
    fn scalar_px_rejects_non_finite_size_and_maxdim() {
        // text.maxWidth/maxHeight + size axes resolve through `scalar_px`; a non-finite
        // numeric string must resolve to None (honest), never a NaN/Inf pixel base.
        assert_eq!(scalar_px(Some(&Scalar::Str("inf".into()))), None);
        assert_eq!(scalar_px(Some(&Scalar::Str("NaN".into()))), None);
        assert_eq!(scalar_px(Some(&Scalar::Str("-infinity".into()))), None);
        // a finite numeric string is unaffected.
        assert_eq!(scalar_px(Some(&Scalar::Str("57".into()))), Some(57.0));
        assert_eq!(scalar_px(Some(&Scalar::Int(24))), Some(24.0));
    }

    #[test]
    fn position_with_string_axes_no_longer_fails_the_file() {
        // BEFORE FIX: position.x="@.." / position.y="1s" rejected (f32), KILLING the
        // whole-file parse. Now the file parses and the axes classify correctly.
        let j = r#"{"windows":[
            {"type":"positionType","name":"slot","position":{"x":"@fixed_btn_mod_col_0","y":"@fixed_btn_mod_row_1"}},
            {"type":"containerWindowType","name":"edge","position":{"x":0,"y":"1s"}}
        ]}"#;
        let g = GuiFile::parse(j).expect("string-form position axes must parse");
        let slot = &g.windows[0].position.as_ref().unwrap();
        assert!(matches!(slot.x, Coord::Anchor { .. }));
        assert!(matches!(slot.y, Coord::Anchor { .. }));
        assert_eq!(
            g.windows[1].position.as_ref().unwrap().y,
            Coord::EdgeOffset(1.0)
        );
    }

    #[test]
    fn named_anchors_resolve_against_a_defines_table() {
        // when the file carries the `@var = N` defines, anchors RESOLVE to the real
        // coordinate (not zeroed) and `unresolved_anchor_count` drops to 0.
        let j = r#"{
            "defines": { "fixed_btn_mod_col_0": 12, "fixed_btn_mod_row_1": 88 },
            "windows":[
              {"type":"positionType","name":"slot","position":{"x":"@fixed_btn_mod_col_0","y":"@fixed_btn_mod_row_1"}}
            ]}"#;
        let g = GuiFile::parse(j).unwrap();
        assert_eq!(
            g.unresolved_anchor_count(),
            0,
            "all anchors resolved against the defines"
        );
        let slot = g.windows[0].position.as_ref().unwrap();
        assert!(matches!(slot.x, Coord::Anchor { resolved: Some(v), .. } if v == 12.0));
        assert!(matches!(slot.y, Coord::Anchor { resolved: Some(v), .. } if v == 88.0));
        // and they resolve to REAL pixel coordinates, not the 0.0 fallback.
        assert_eq!(slot.x.px(), 12.0);
        assert_eq!(slot.y.px(), 88.0);
    }

    #[test]
    fn shared_defs_resolve_remaining_anchors_after_parse() {
        // an anchor unresolved by the file's own (empty) defines is filled by an
        // externally-shared defs table (a shared constants `.gui`).
        let j = r#"{"windows":[
            {"type":"positionType","name":"slot","position":{"x":"@shared_col","y":0}}
        ]}"#;
        let mut g = GuiFile::parse(j).unwrap();
        assert_eq!(g.unresolved_anchor_count(), 1);
        let shared: HashMap<String, f64> = [("shared_col".to_string(), 33.0)].into_iter().collect();
        g.resolve_anchors_with(&shared);
        assert_eq!(g.unresolved_anchor_count(), 0);
        assert_eq!(g.windows[0].position.as_ref().unwrap().x.px(), 33.0);
    }

    #[test]
    fn maxheight_percent_string_parses_no_longer_rejected() {
        // BEFORE FIX: text.maxHeight="100%%" rejected (f32) -> whole-file parse fail.
        let j = r#"{"windows":[
            {"type":"instantTextBoxType","name":"t","text":{"text":"x","maxWidth":"100%%","maxHeight":"100%%"}}
        ]}"#;
        let g = GuiFile::parse(j).expect("percent maxWidth/maxHeight must parse");
        let tb = g.windows[0].text.as_ref().unwrap();
        // a percent has no pixel base here -> None (honest), but it PARSED.
        assert_eq!(tb.max_h(), None);
        assert_eq!(tb.max_w(), None);
    }

    #[test]
    fn bool_coordinate_is_unknown_not_a_fabricated_zero() {
        // a JSON bool in a position axis is meaningless; it must NOT silently become a
        // measured 0px (the old f32 path rejected it). Retained as UnknownRaw.
        let j = r#"{"windows":[{"type":"containerWindowType","name":"b","position":{"x":true,"y":0}}]}"#;
        let g = GuiFile::parse(j).expect("a bool axis stays parse-tolerant");
        let p = g.windows[0].position.as_ref().unwrap();
        assert!(matches!(&p.x, Coord::UnknownRaw(s) if s == "true"));
        assert!(
            p.x.is_unresolved(),
            "a bool coordinate is unresolved, not a measured 0"
        );
        assert_eq!(p.x.px(), 0.0, "still falls back to 0.0 at the chrome seam");
    }

    #[test]
    fn maxheight_numeric_still_resolves_to_px() {
        let j = r#"{"windows":[
            {"type":"instantTextBoxType","name":"t","text":{"text":"x","maxWidth":57,"maxHeight":24}}
        ]}"#;
        let g = GuiFile::parse(j).unwrap();
        let tb = g.windows[0].text.as_ref().unwrap();
        assert_eq!(tb.max_w(), Some(57.0));
        assert_eq!(tb.max_h(), Some(24.0));
    }
}
