use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Upper bound on individual identifier gaps enumerated in a refusal message.
/// The count is always exact; only the enumeration is bounded.
pub(crate) const MAX_REPORTED_ID_GAPS: usize = 20;

/// Fold a gap list into a single typed refusal, or `Ok(())` when there are none.
pub(crate) fn refuse_on_id_gaps(what: &str, gaps: &[String]) -> Result<()> {
    if gaps.is_empty() {
        return Ok(());
    }
    let shown = gaps.len().min(MAX_REPORTED_ID_GAPS);
    let elided = gaps.len() - shown;
    let suffix = if elided == 0 {
        String::new()
    } else {
        format!("; and {elided} more")
    };
    Err(format!(
        "{what} refused: {} identifier gap(s) — {}{suffix}",
        gaps.len(),
        gaps[..shown].join("; ")
    )
    .into())
}

/// FW7-F6: `history/states` identifiers are loaded fail-closed.
///
/// The previous loop dropped any state file with no parseable `id` with a bare
/// `continue`, and let a duplicate state id overwrite the earlier entry — but
/// *non-atomically*: `state_owners` and `province_to_state` were mutated before
/// `states.insert`, so the overwritten state's owner row and its province
/// mappings survived pointing at a state whose `StateMeta` no longer listed
/// them. A province claimed by two distinct states was likewise resolved
/// last-writer-wins in `province_to_state` while both `StateMeta.provinces`
/// lists kept it. None of this reached the caller; the return type carried only
/// the maps.
///
/// Each of those is now a typed gap, collected across the whole directory (so
/// one run names every defect rather than only the first) and returned as a
/// single refusal. The mutation order is also fixed: nothing is written until
/// the file is known to be admissible, so no partial state can survive.
/// This mirrors the gap discipline already used by `rail_files::rail_polylines`.
pub(crate) fn load_states(
    state_files: &[(String, PathBuf)],
) -> Result<(
    BTreeMap<u32, StateMeta>,
    BTreeMap<u32, u32>,
    BTreeMap<u32, String>,
)> {
    let sources = state_files
        .iter()
        .map(|(name, path)| Ok((name.clone(), path.display().to_string(), read_latin1(path)?)))
        .collect::<Result<Vec<_>>>()?;
    decode_states(&sources)
}

/// Parse already-admitted state bytes without reopening the effective files.
/// Each tuple is `(effective name, diagnostic source, Latin-1 source)`.
pub(crate) fn decode_states(
    state_sources: &[(String, String, String)],
) -> Result<(
    BTreeMap<u32, StateMeta>,
    BTreeMap<u32, u32>,
    BTreeMap<u32, String>,
)> {
    let mut states = BTreeMap::new();
    let mut province_to_state = BTreeMap::new();
    let mut state_owners = BTreeMap::new();
    let mut gaps: Vec<String> = Vec::new();
    let mut state_source: BTreeMap<u32, String> = BTreeMap::new();
    let mut province_source: BTreeMap<u32, (u32, String)> = BTreeMap::new();

    for (name, source, admitted_text) in state_sources {
        let text = strip_comments(admitted_text);
        let Some(state_id) = parse_u32_value(&text, "id") else {
            gaps.push(format!(
                "{name} ({}) has no parseable `id =` and would be dropped",
                source
            ));
            continue;
        };
        if let Some(previous) = state_source.get(&state_id) {
            gaps.push(format!(
                "{name} redefines state id {state_id} already defined by {previous}"
            ));
            continue;
        }
        let name_key =
            parse_quoted_value(&text, "name").unwrap_or_else(|| format!("STATE_{state_id}"));
        let state_category = parse_word_value(&text, "state_category");
        let manpower = parse_u64_value(&text, "manpower").unwrap_or_default();
        let impassable = parse_word_value(&text, "impassable")
            .map(|value| value.eq_ignore_ascii_case("yes"))
            .unwrap_or(false);
        let provinces = extract_block_after(&text, "provinces", 0)
            .map(|(inner, _)| parse_u32s(&inner))
            .unwrap_or_default();
        let mut victory_points = BTreeMap::new();
        let mut search = 0usize;
        while let Some((inner, next)) = extract_block_after(&text, "victory_points", search) {
            if !inner.contains('=') {
                let nums = parse_i32s(&inner);
                for pair in nums.chunks(2) {
                    if pair.len() == 2 {
                        if let Ok(province) = u32::try_from(pair[0]) {
                            *victory_points.entry(province).or_insert(0) += pair[1];
                        }
                    }
                }
            }
            search = next;
        }
        let vp_total = victory_points.values().sum();
        let mut conflicts = Vec::new();
        for province in &provinces {
            if let Some((owner_state, owner_file)) = province_source.get(province) {
                conflicts.push(format!(
                    "province {province} is already claimed by state {owner_state} ({owner_file})"
                ));
            }
        }
        if !conflicts.is_empty() {
            gaps.push(format!(
                "{name} (state {state_id}) conflicts: {}",
                conflicts.join(", ")
            ));
            continue;
        }
        // Admissible: only now is anything written, so no map can retain a row
        // belonging to a state that was superseded or refused.
        state_source.insert(state_id, name.clone());
        if let Some(owner) = parse_word_value(&text, "owner") {
            state_owners.insert(state_id, owner);
        }
        for province in &provinces {
            province_to_state.insert(*province, state_id);
            province_source.insert(*province, (state_id, name.clone()));
        }
        states.insert(
            state_id,
            StateMeta {
                name_key,
                provinces,
                victory_points,
                vp_total,
                state_category,
                manpower,
                impassable,
            },
        );
    }

    refuse_on_id_gaps("history/states", &gaps)?;
    Ok((states, province_to_state, state_owners))
}

/// FW7-F6: `map/strategicregions` identifiers are loaded fail-closed, on the
/// same terms as [`load_states`]. Previously a region file with no parseable
/// `id` was dropped silently, a duplicate region id was dropped silently
/// first-writer-wins (the opposite of `load_states`' last-writer-wins, for no
/// stated reason), and a province listed by two regions was resolved
/// last-writer-wins in `province_to_region` while both regions kept it in their
/// province list and their `province_count`.
pub(crate) fn load_strategic_regions(
    region_files: &[(String, PathBuf)],
) -> Result<(
    BTreeMap<u32, Vec<u32>>,
    BTreeMap<u32, StrategicRegionMeta>,
    BTreeMap<u32, u32>,
)> {
    let mut region_to_provinces = BTreeMap::new();
    let mut region_meta = BTreeMap::new();
    let mut province_to_region = BTreeMap::new();
    let mut gaps: Vec<String> = Vec::new();
    let mut region_source: BTreeMap<u32, String> = BTreeMap::new();
    let mut province_source: BTreeMap<u32, (u32, String)> = BTreeMap::new();

    for (name_hint, path) in region_files {
        let text = strip_comments(&read_latin1(&path)?);
        let Some(region_id) = parse_u32_value(&text, "id") else {
            gaps.push(format!(
                "{name_hint} ({}) has no parseable `id =` and would be dropped",
                path.display()
            ));
            continue;
        };
        if let Some(previous) = region_source.get(&region_id) {
            gaps.push(format!(
                "{name_hint} redefines strategic region id {region_id} already defined by {previous}"
            ));
            continue;
        }
        let name = parse_quoted_value(&text, "name")
            .unwrap_or_else(|| format!("STRATEGICREGION_{region_id}"));
        let provinces = extract_block_after(&text, "provinces", 0)
            .map(|(inner, _)| parse_u32s(&inner))
            .unwrap_or_default();
        let mut conflicts = Vec::new();
        for province in &provinces {
            if let Some((owner_region, owner_file)) = province_source.get(province) {
                conflicts.push(format!(
                    "province {province} is already claimed by strategic region {owner_region} ({owner_file})"
                ));
            }
        }
        if !conflicts.is_empty() {
            gaps.push(format!(
                "{name_hint} (strategic region {region_id}) conflicts: {}",
                conflicts.join(", ")
            ));
            continue;
        }
        region_source.insert(region_id, name_hint.clone());
        for province in &provinces {
            province_to_region.insert(*province, region_id);
            province_source.insert(*province, (region_id, name_hint.clone()));
        }
        region_meta.insert(
            region_id,
            StrategicRegionMeta {
                name,
                province_count: provinces.len(),
            },
        );
        region_to_provinces.insert(region_id, provinces);
    }

    refuse_on_id_gaps("map/strategicregions", &gaps)?;
    Ok((region_to_provinces, region_meta, province_to_region))
}

pub(crate) fn load_country_colors(resolver: &FileResolver) -> Result<BTreeMap<String, [u8; 3]>> {
    let mut colors = BTreeMap::new();
    let tag_paths = resolver.resolve_dir_files_recursive("common/country_tags", Some("txt"))?;
    let tag_files = load_country_tag_files(&tag_paths)?;
    for (tag, rel) in &tag_files {
        if let Some(path) = resolver.resolve_optional_file(rel)? {
            if let Some(color) = load_country_file_color(&path)? {
                colors.insert(tag.clone(), color);
            }
        }
    }
    if let Some(colors_path) = resolver.resolve_optional_file("common/countries/colors.txt")? {
        for (tag, color) in load_colors_txt_overrides(&colors_path)? {
            colors.insert(tag.clone(), color);
        }
    }
    // NAME1: the 1936 cosmetic-tag color overrides (see
    // apply_cosmetic_color_overrides). India must render as the BRITISH RAJ
    // red, not the RAJ base orange.
    // FW7 (Codex addition 4): this was `.unwrap_or_default()`, which turned any
    // resolver enumeration failure into an empty file list. Under a modded set
    // that silently discarded the whole 1936 cosmetic-tag history and fell back
    // to base country colors — the exact class of mod loss forbidden by the
    // `PROJECT.md` heading "Content and mod contract", and invisible in the
    // output. Carry the resolver error instead.
    let history_files = resolver
        .resolve_dir_files_recursive("history/countries", Some("txt"))
        .map_err(|error| -> Box<dyn std::error::Error> {
            format!("history/countries could not be enumerated for cosmetic-tag colors: {error}")
                .into()
        })?;
    let mut cosmetic_tags = cosmetic_tags_1936_from_history_files(&history_files)?;
    // NAME1b: DLC-conditional cosmetics. The bundle is generated without a
    // recording, so it assumes all DLC present (the common install + the
    // frame4700 byte target): Canada -> CAN_UK, Congo -> COG_belgian_congo.
    merge_dlc_conditional_cosmetics(&mut cosmetic_tags, &history_files, |_| true)?;
    let cosmetic_colors =
        if let Some(path) = resolver.resolve_optional_file("common/countries/cosmetic.txt")? {
            load_cosmetic_colors(&path)?
        } else {
            BTreeMap::new()
        };
    apply_cosmetic_color_overrides(&mut colors, &cosmetic_tags, &cosmetic_colors);
    Ok(colors)
}

/// NAME1b: merge DLC-conditional `set_cosmetic_tag`s into a base cosmetic map
/// under a DLC predicate (a conditional overrides the base for its tag when its
/// `has_dlc`/`NOT has_dlc` condition holds). Shared by both color-bake paths.
pub(crate) fn merge_dlc_conditional_cosmetics(
    cosmetic_tags: &mut BTreeMap<String, String>,
    history_files: &[(String, PathBuf)],
    has_dlc: impl Fn(&str) -> bool,
) -> Result<()> {
    let conditional = dlc_conditional_cosmetics_1936_from_history_files(history_files)?;
    for (tag, entries) in conditional {
        for entry in entries {
            let holds = if entry.negated {
                !has_dlc(&entry.dlc)
            } else {
                has_dlc(&entry.dlc)
            };
            if holds {
                cosmetic_tags.insert(tag.clone(), entry.cosmetic);
            }
        }
    }
    Ok(())
}

/// Base tag colors WITHOUT any cosmetic-tag override: `common/country_tags`
/// per-country files plus `common/countries/colors.txt` overrides. The A5
/// timeline path re-applies cosmetic overrides on top of this base using the
/// recording's per-moment cosmetic tags (see
/// [`country_colors_from_game_resolver_with_cosmetics`]).
pub(crate) fn load_base_country_colors_from_game_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, [u8; 3]>> {
    let mut colors = BTreeMap::new();
    let tag_paths = resolver.resolve_dir_files_recursive("common/country_tags", Some("txt"))?;
    let tag_files = load_country_tag_files(&tag_paths)?;
    for (tag, rel) in &tag_files {
        if let Some(path) = resolver.resolve(&country_tag_rel_for_game_resolver(rel))? {
            if let Some(color) = load_country_file_color(&path)? {
                colors.insert(tag.clone(), color);
            }
        }
    }
    if let Some(colors_path) = resolver.resolve("common/countries/colors.txt")? {
        for (tag, color) in load_colors_txt_overrides(&colors_path)? {
            colors.insert(tag.clone(), color);
        }
    }
    Ok(colors)
}

/// Load `common/countries/cosmetic.txt` (cosmetic tag -> map color), or an
/// empty map when the file is absent.
pub(crate) fn load_cosmetic_colors_from_game_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, [u8; 3]>> {
    if let Some(path) = resolver.resolve("common/countries/cosmetic.txt")? {
        load_cosmetic_colors(&path)
    } else {
        Ok(BTreeMap::new())
    }
}

/// A5 timeline color path: base tag colors with the *recording's* per-moment
/// cosmetic tags applied (NAME1 color override), instead of the 1936-history
/// cosmetic parse. `cosmetic_by_tag` comes from the `WorldTimeline`
/// `country_identities` cosmetic view (see [`crate::keying::KeyingTables`]).
pub(crate) fn country_colors_from_game_resolver_with_cosmetics(
    resolver: &GameFileResolver,
    cosmetic_by_tag: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, [u8; 3]>> {
    let mut colors = load_base_country_colors_from_game_resolver(resolver)?;
    let cosmetic_colors = load_cosmetic_colors_from_game_resolver(resolver)?;
    apply_cosmetic_color_overrides(&mut colors, cosmetic_by_tag, &cosmetic_colors);
    Ok(colors)
}

pub(crate) fn load_country_colors_from_game_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, [u8; 3]>> {
    // NAME1: the 1936 cosmetic-tag color overrides (static-file path).
    // FW7 (Codex addition 4): this was `.unwrap_or_default()`, which turned any
    // resolver enumeration failure into an empty file list. Under a modded set
    // that silently discarded the whole 1936 cosmetic-tag history and fell back
    // to base country colors — the exact class of mod loss forbidden by the
    // `PROJECT.md` heading "Content and mod contract", and invisible in the
    // output. Carry the resolver error instead.
    let history_files = resolver
        .resolve_dir_files_recursive("history/countries", Some("txt"))
        .map_err(|error| -> Box<dyn std::error::Error> {
            format!("history/countries could not be enumerated for cosmetic-tag colors: {error}")
                .into()
        })?;
    let mut cosmetic_tags = cosmetic_tags_1936_from_history_files(&history_files)?;
    // NAME1b: DLC-conditional cosmetics, all-DLC assumed for the bundle.
    merge_dlc_conditional_cosmetics(&mut cosmetic_tags, &history_files, |_| true)?;
    country_colors_from_game_resolver_with_cosmetics(resolver, &cosmetic_tags)
}

/// NAME1: tag -> 1936 cosmetic tag from `history/countries/<TAG> - *.txt`.
///
/// The engine's `set_cosmetic_tag = X` effect overrides a country's displayed
/// NAME (localisation keys `X_<ideology>` / `X`, shipped in
/// `countries_cosmetic_l_english.yml`) and its map COLOR
/// (`common/countries/cosmetic.txt` `X = { color = ... }`). Vanilla 1936 sets
/// it unconditionally for 13 countries (e.g. `RAJ - British Raj.txt:244`
/// `set_cosmetic_tag = RAJ_UK` — "British Raj", rgb 170/10/10).
///
/// This parses ONLY the unconditional (brace depth 0) assignments. The
/// DLC-conditional ones (`if = { limit = { has_dlc ... } set_cosmetic_tag }`,
/// vanilla CAN/COG) are handled by
/// [`dlc_conditional_cosmetics_from_text`] + [`merge_dlc_conditional_cosmetics`]
/// (NAME1b). The remainder (CZE/MEN/SPR: focus/civil-war/event effects with
/// non-DLC limits) are NOT 1936 start state and are correctly skipped by both.
pub(crate) fn cosmetic_tags_1936_from_history_files(
    files: &[(String, PathBuf)],
) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (rel, path) in files {
        let Some(tag) = crate::map_furniture::country_history_tag_from_rel(rel) else {
            continue;
        };
        let text = strip_comments(&read_latin1(path)?);
        if let Some(cosmetic) = parse_top_level_word_value(&text, "set_cosmetic_tag") {
            out.entry(tag).or_insert(cosmetic);
        }
    }
    Ok(out)
}

/// NAME1b: a `set_cosmetic_tag` that a country's 1936 history applies only
/// when a DLC condition holds — `if = { limit = { has_dlc = "X" } ...
/// set_cosmetic_tag = Y }` (or its `NOT = { has_dlc = "X" }` fallback). The
/// engine evaluates `has_dlc` against the ACTIVE DLC set at game start, so the
/// displayed name + map colour differ with/without the DLC (vanilla:
/// `CAN - Canada.txt` → CAN_UK "Canada"/rgb 119,48,39 WITH Together for
/// Victory, CAN_ALY WITHOUT; `COG - Congo.txt` → COG_belgian_congo WITH
/// Gotterdammerung). This is the files-grounded answer to the user's
/// no-DLC-compatibility requirement. Production replay resolution is bound to
/// `SessionMetadataView.active_dlcs` by `map_furniture`; static bundle baking
/// passes an explicit all-DLC predicate for the frame4700 byte target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionalCosmetic {
    /// The cosmetic tag applied when the condition holds.
    pub cosmetic: String,
    /// The DLC name the `has_dlc` check names.
    pub dlc: String,
    /// True iff the limit is `NOT = { has_dlc = ... }` (the fallback branch,
    /// applied when the DLC is ABSENT).
    pub negated: bool,
}

/// Parse the DLC-conditional `set_cosmetic_tag` blocks from one country history
/// file (comments already stripped by the caller). Only TOP-LEVEL
/// `if = { limit = { <pure has_dlc> } ... set_cosmetic_tag = Y }` blocks
/// qualify — the `limit` must contain a `has_dlc` (optionally wrapped in a
/// single `NOT`) and NOTHING ELSE, so ideology/flag/civil-war-gated cosmetics
/// (SPR nationalist Spain, CZE fall-of-the-republic) are NOT misread as start
/// state. Deterministic, in file order.
pub(crate) fn dlc_conditional_cosmetics_from_text(text: &str) -> Vec<ConditionalCosmetic> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    // Walk top-level `if`/`IF` tokens at depth 0.
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
            }
            _ => {
                let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                let at_word_start = i == 0 || !is_word(bytes[i - 1]);
                let is_if = at_word_start
                    && depth == 0
                    && (bytes[i..].starts_with(b"if") || bytes[i..].starts_with(b"IF"))
                    && bytes.get(i + 2).map(|b| !is_word(*b)).unwrap_or(true);
                if !is_if {
                    i += 1;
                    continue;
                }
                // Find the block after `if =`.
                let mut j = i + 2;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if bytes.get(j) != Some(&b'=') {
                    i += 1;
                    continue;
                }
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if bytes.get(j) != Some(&b'{') {
                    i += 1;
                    continue;
                }
                // Balance the if-block.
                let body_start = j + 1;
                let mut d = 1i32;
                let mut k = body_start;
                while k < bytes.len() && d > 0 {
                    match bytes[k] {
                        b'{' => d += 1,
                        b'}' => d -= 1,
                        _ => {}
                    }
                    k += 1;
                }
                if d != 0 {
                    // Unterminated block: the closing `}` was never found, so
                    // `k - 1` may land inside a multi-byte (latin1-decoded)
                    // char. Nothing after this point is parseable.
                    break;
                }
                // d == 0 guarantees bytes[k - 1] is the ASCII '}'.
                let body = &text[body_start..k - 1];
                if let Some(entry) = conditional_cosmetic_from_if_body(body) {
                    out.push(entry);
                }
                i = k;
            }
        }
    }
    out
}

/// Given the body of a top-level `if { }`, return its DLC-conditional cosmetic
/// iff the `limit` is a pure `has_dlc` (optionally single-`NOT`) check and a
/// `set_cosmetic_tag` is present at the if-body top level.
fn conditional_cosmetic_from_if_body(body: &str) -> Option<ConditionalCosmetic> {
    let cosmetic = parse_top_level_word_value(body, "set_cosmetic_tag")?;
    // Extract the `limit = { ... }` sub-block.
    let limit = extract_braced_block(body, "limit")?;
    let (dlc, negated) = parse_pure_has_dlc(&limit)?;
    Some(ConditionalCosmetic {
        cosmetic,
        dlc,
        negated,
    })
}

/// The brace-balanced content of the first `key = { ... }` block at the top
/// level of `text` (comments stripped). Returns the inner text (no braces).
fn extract_braced_block(text: &str, key: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let key_bytes = key.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {
                if depth == 0
                    && bytes[i..].starts_with(key_bytes)
                    && (i == 0 || !is_word(bytes[i - 1]))
                    && bytes
                        .get(i + key_bytes.len())
                        .map(|b| !is_word(*b))
                        .unwrap_or(true)
                {
                    let mut j = i + key_bytes.len();
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'=') {
                        j += 1;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if bytes.get(j) == Some(&b'{') {
                            let start = j + 1;
                            let mut d = 1i32;
                            let mut k = start;
                            while k < bytes.len() && d > 0 {
                                match bytes[k] {
                                    b'{' => d += 1,
                                    b'}' => d -= 1,
                                    _ => {}
                                }
                                k += 1;
                            }
                            if d != 0 {
                                // Unterminated block: `k - 1` may land inside
                                // a multi-byte (latin1-decoded) char.
                                return None;
                            }
                            // d == 0 guarantees bytes[k - 1] is the ASCII '}'.
                            return Some(text[start..k - 1].to_string());
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Parse a `limit` body that must be a PURE has_dlc test: either
/// `has_dlc = "X"` or `NOT = { has_dlc = "X" }`, with no other tokens. Returns
/// `(dlc_name, negated)`; `None` if the limit has any other condition (so
/// non-DLC gates are rejected). The DLC name is a quoted string.
fn parse_pure_has_dlc(limit: &str) -> Option<(String, bool)> {
    let trimmed = limit.trim();
    // Strip a single NOT = { ... } wrapper.
    let (inner, negated) = if let Some(rest) = trimmed.strip_prefix("NOT") {
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('=')?.trim_start();
        let rest = rest.strip_prefix('{')?;
        let rest = rest.strip_suffix('}')?;
        (rest.trim().to_string(), true)
    } else {
        (trimmed.to_string(), false)
    };
    // The remaining must be exactly `has_dlc = "X"`.
    let rest = inner.strip_prefix("has_dlc")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let dlc = &rest[..end];
    // Nothing but whitespace may follow the closing quote.
    if !rest[end + 1..].trim().is_empty() {
        return None;
    }
    Some((dlc.to_string(), negated))
}

/// Collect DLC-conditional cosmetics per tag across all history files.
pub(crate) fn dlc_conditional_cosmetics_1936_from_history_files(
    files: &[(String, PathBuf)],
) -> Result<BTreeMap<String, Vec<ConditionalCosmetic>>> {
    let mut out: BTreeMap<String, Vec<ConditionalCosmetic>> = BTreeMap::new();
    for (rel, path) in files {
        let Some(tag) = crate::map_furniture::country_history_tag_from_rel(rel) else {
            continue;
        };
        let text = strip_comments(&read_latin1(path)?);
        let entries = dlc_conditional_cosmetics_from_text(&text);
        if !entries.is_empty() {
            out.entry(tag).or_default().extend(entries);
        }
    }
    Ok(out)
}

/// First `key = <word>` assignment at BRACE DEPTH 0 (comments already
/// stripped). Unlike `parse_word_value`, assignments nested inside any `{}`
/// block (if/limit, scoped effects, date blocks) are ignored.
pub(crate) fn parse_top_level_word_value(text: &str, key: &str) -> Option<String> {
    // Byte-wise scan: history files are latin1-decoded and may contain
    // multi-byte UTF-8 chars, so no `&text[i..]` slicing at arbitrary offsets.
    let bytes = text.as_bytes();
    let key_bytes = key.as_bytes();
    let is_word_byte = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {
                if depth == 0
                    && bytes[i..].starts_with(key_bytes)
                    && (i == 0 || !is_word_byte(bytes[i - 1]))
                {
                    let after = i + key_bytes.len();
                    if bytes.get(after).map(|b| !is_word_byte(*b)).unwrap_or(true) {
                        let mut j = after;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if bytes.get(j) == Some(&b'=') {
                            j += 1;
                            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                                j += 1;
                            }
                            let start = j;
                            while j < bytes.len() && is_word_byte(bytes[j]) {
                                j += 1;
                            }
                            if j > start {
                                // Word bytes are pure ASCII by construction.
                                return Some(
                                    String::from_utf8_lossy(&bytes[start..j]).into_owned(),
                                );
                            }
                        }
                        i = after;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Parse `common/countries/cosmetic.txt`: `<cosmetic_tag> = { color = ... }`
/// blocks. Cosmetic identifiers are long mixed-case words (`RAJ_UK`,
/// `URG_is_unaligned_...`), unlike the 2-3 char tags in `colors.txt`, so this
/// scans generic identifiers. Both `rgb {}` and `hsv {}` colors ship in the
/// vanilla file; `parse_color_assignment` handles both.
pub(crate) fn load_cosmetic_colors(path: &Path) -> Result<BTreeMap<String, [u8; 3]>> {
    let mut out = BTreeMap::new();
    if !path.is_file() {
        return Ok(out);
    }
    let text = strip_comments(&read_latin1(path)?);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && !(bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if start == i {
            break;
        }
        let name = &text[start..i];
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'=' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                let inner = extract_brace_at(&text, j).unwrap_or_default();
                if let Some(color) = parse_color_assignment(&inner, "color") {
                    out.insert(name.to_string(), color);
                }
                i = j + inner.len() + 2;
                continue;
            }
        }
    }
    Ok(out)
}

/// Overlay the 1936 cosmetic colors onto the base tag colors: a country whose
/// history sets a cosmetic tag takes that cosmetic's `cosmetic.txt` color. A
/// cosmetic tag without a color entry keeps the base color (the cosmetic then
/// only affects the display name).
pub(crate) fn apply_cosmetic_color_overrides(
    colors: &mut BTreeMap<String, [u8; 3]>,
    cosmetic_tags: &BTreeMap<String, String>,
    cosmetic_colors: &BTreeMap<String, [u8; 3]>,
) {
    for (tag, cosmetic) in cosmetic_tags {
        if let Some(color) = cosmetic_colors.get(cosmetic) {
            colors.insert(tag.clone(), *color);
        }
    }
}

fn country_tag_rel_for_game_resolver(rel: &str) -> String {
    let rel = normalize_rel_path(rel);
    if rel.starts_with("common/") {
        rel
    } else {
        format!("common/{rel}")
    }
}

pub(crate) fn load_terrain_texture_map(path: &Path) -> Result<BTreeMap<u8, u8>> {
    let text = strip_comments(&read_latin1(path)?);
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if !line.contains("color") || !line.contains("texture") {
            continue;
        }
        let Some(color_pos) = line.find("color") else {
            continue;
        };
        let Some(texture_pos) = line.find("texture") else {
            continue;
        };
        let Some(color_brace) = line[color_pos..].find('{').map(|idx| color_pos + idx) else {
            continue;
        };
        let Some(color_inner) = extract_brace_at(line, color_brace) else {
            continue;
        };
        let Some(index) = parse_u32s(&color_inner).first().copied() else {
            continue;
        };
        let texture_tail = &line[texture_pos + "texture".len()..];
        let Some(eq_pos) = texture_tail.find('=') else {
            continue;
        };
        let Some(texture) = parse_u32_at(&texture_tail[eq_pos + 1..]) else {
            continue;
        };
        if let (Ok(index), Ok(texture)) = (u8::try_from(index), u8::try_from(texture)) {
            out.insert(index, texture);
        }
    }
    Ok(out)
}

pub(crate) fn map_terrain_texture_ids(
    terrain_indices: &IndexImage,
    terrain_texture_map: &BTreeMap<u8, u8>,
) -> Vec<u8> {
    terrain_indices
        .pixels
        .iter()
        .map(|index| terrain_texture_map.get(index).copied().unwrap_or(255))
        .collect()
}

pub(crate) fn load_map_object_meta(
    default_map: &Path,
    cities_txt: &Path,
    cities: &IndexImage,
    trees: &IndexImage,
    terrain_indices: &IndexImage,
    terrain_texture_map: &BTreeMap<u8, u8>,
    seasons_txt: &Path,
    buildings_txt: &Path,
    railways_txt: &Path,
    supply_nodes_txt: &Path,
    unitstacks_txt: &Path,
    weatherpositions_txt: &Path,
    ambient_object_txt: &Path,
    common_building_paths: &[(String, PathBuf)],
    render_source_paths: &[(String, PathBuf)],
) -> Result<MapObjectMeta> {
    let mut indexed_maps = BTreeMap::new();
    indexed_maps.insert("map/cities.bmp".to_string(), indexed_map_meta(cities));
    indexed_maps.insert("map/trees.bmp".to_string(), indexed_map_meta(trees));

    let mut static_sources = BTreeMap::new();
    static_sources.insert(
        "map/buildings.txt".to_string(),
        delimited_source_meta(buildings_txt, Delimiter::Semicolon, Some(1))?,
    );
    static_sources.insert(
        "map/railways.txt".to_string(),
        delimited_source_meta(railways_txt, Delimiter::Whitespace, Some(0))?,
    );
    static_sources.insert(
        "map/supply_nodes.txt".to_string(),
        delimited_source_meta(supply_nodes_txt, Delimiter::Whitespace, Some(0))?,
    );
    static_sources.insert(
        "map/unitstacks.txt".to_string(),
        delimited_source_meta(unitstacks_txt, Delimiter::Semicolon, Some(1))?,
    );
    static_sources.insert(
        "map/weatherpositions.txt".to_string(),
        delimited_source_meta(weatherpositions_txt, Delimiter::Semicolon, Some(4))?,
    );

    Ok(MapObjectMeta {
        default_map: load_default_map_meta(default_map)?,
        terrain: terrain_source_meta(terrain_indices, terrain_texture_map),
        cities: load_city_source_meta(cities_txt)?,
        indexed_maps,
        seasons: load_season_source_meta(seasons_txt)?,
        static_sources,
        ambient_objects: load_ambient_source_meta(ambient_object_txt)?,
        common_building_rules: load_common_building_rules(common_building_paths)?,
        render_source_catalogs: load_text_source_catalogs(
            common_building_paths
                .iter()
                .chain(render_source_paths.iter())
                .map(|(source, path)| (source.as_str(), path)),
        )?,
    })
}

pub(crate) fn terrain_source_meta(
    terrain_indices: &IndexImage,
    terrain_texture_map: &BTreeMap<u8, u8>,
) -> TerrainSourceMeta {
    let terrain_index_counts = indexed_counts(&terrain_indices.pixels);
    let unmapped_index_counts = terrain_index_counts
        .iter()
        .filter_map(|(index, count)| {
            (!terrain_texture_map.contains_key(index)).then_some((*index, *count))
        })
        .collect();
    TerrainSourceMeta {
        index_to_texture_id: terrain_texture_map.clone(),
        terrain_index_counts,
        unmapped_index_counts,
        implemented_parity_level:
            "terrain_idmap_four_id_atlas_mip_law_capture_gated_runtime_lighting".to_string(),
        generated_runtime_gaps: vec![
            "SnowMudData runtime texture".to_string(),
            "LightIndexMap/LightDataMap runtime textures".to_string(),
            "ShadowMap runtime texture".to_string(),
        ],
    }
}

pub(crate) fn collect_source_files(
    resolver: &FileResolver,
    source_dir: &str,
    extension: Option<&str>,
) -> Result<Vec<(String, PathBuf)>> {
    resolver.resolve_dir_files(source_dir, extension)
}

pub(crate) fn source_spec(source: &str, path: PathBuf) -> (String, String, PathBuf) {
    (source.to_string(), source_output_name(source), path)
}

pub(crate) fn sanitize_source_name(source: &str) -> String {
    let mut out = String::new();
    for ch in source.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

pub(crate) fn source_output_name(source: &str) -> String {
    format!("source_{}", sanitize_source_name(source))
}

pub(crate) fn shader_rgba_output_name(source: &str) -> String {
    format!("shader_{}.rgba", sanitize_source_name(source))
}

pub(crate) fn shader_rgba_mip_output_name(source: &str) -> String {
    format!("shader_{}_mips.rgba", sanitize_source_name(source))
}

pub(crate) fn material_mip_output_name(source: &str, mip_count: u32) -> Option<String> {
    if mip_count > 1 {
        Some(shader_rgba_mip_output_name(source))
    } else {
        None
    }
}

pub(crate) fn material_rgba_preservation(mip_count: u32) -> String {
    if mip_count > 1 {
        "decoded_rgba_exact_2d_with_mip_chain".to_string()
    } else {
        "decoded_rgba_exact_2d".to_string()
    }
}

pub(crate) fn material_rgba_specs(resolver: &FileResolver) -> Result<Vec<RgbaSourceSpec>> {
    let mut specs = Vec::new();
    push_material_spec(
        &mut specs,
        resolver,
        "map/terrain/atlas0.dds",
        "terrain_diffuse_atlas",
        &["TerrainDiffuse"],
    )?;
    push_material_spec(
        &mut specs,
        resolver,
        "map/terrain/atlas_normal0.dds",
        "terrain_normal_atlas",
        &["TerrainNormal"],
    )?;
    push_material_spec(
        &mut specs,
        resolver,
        "map/terrain/colormap_rgb_cityemissivemask_a.dds",
        "terrain_color_tint_city_emissive_mask",
        &["TerrainColorTint"],
    )?;
    for idx in 0..=2 {
        for (pattern, role, samplers) in [
            ("colormap_water", "water_color", &["WaterColor"][..]),
            ("underwater_terrain", "underwater_terrain", &[][..]),
            ("RiverSurface_diffuse", "river_diffuse", &["Diffuse"][..]),
            ("RiverSurface_normal", "river_normal", &["NormalMap"][..]),
            (
                "citylights_rgb_snowmask_a",
                "citylights_snow_mask",
                &["CityLightsAndSnowNoise"][..],
            ),
            (
                "mud_diffuse_rgb_gloss_a",
                "mud_diffuse_gloss",
                &["MudDiffuseGloss"][..],
            ),
            (
                "mud_normal_rgb_spec_a",
                "mud_normal_spec",
                &["MudNormalSpec"][..],
            ),
            ("ice_noise", "ice_noise", &["IceNoise"][..]),
        ] {
            push_material_spec(
                &mut specs,
                resolver,
                &format!("map/terrain/{pattern}_{idx}.dds"),
                role,
                samplers,
            )?;
        }
        for border in [
            "border_country",
            "border_impassable",
            "border_province",
            "border_sea",
            "border_sea_region",
            "border_state",
        ] {
            push_material_spec(
                &mut specs,
                resolver,
                &format!("map/terrain/{border}_{idx}.dds"),
                border,
                &["BorderDiffuse"],
            )?;
        }
    }
    for (source, role, samplers) in [
        (
            "map/terrain/fow_rgb_waterspec_a.dds",
            "water_specular_fow",
            &["SpecularMap"][..],
        ),
        (
            "map/terrain/lean1.dds",
            "water_lean_1",
            &["LeanTexture1"][..],
        ),
        (
            "map/terrain/lean2.dds",
            "water_lean_2",
            &["LeanTexture2"][..],
        ),
        (
            "map/terrain/RiverSurface_masks.dds",
            "river_surface_masks",
            &[][..],
        ),
        (
            "map/terrain/snow_normal_rgb_diffuse_a.dds",
            "snow_normal_diffuse",
            &["SnowTexture"][..],
        ),
        (
            "map/terrain/ice_diffuse.dds",
            "ice_diffuse",
            &["IceDiffuse"][..],
        ),
    ] {
        push_material_spec(&mut specs, resolver, source, role, samplers)?;
    }
    Ok(specs)
}

pub(crate) fn push_material_spec(
    specs: &mut Vec<RgbaSourceSpec>,
    resolver: &FileResolver,
    source: &str,
    role: &str,
    shader_samplers: &[&str],
) -> Result<()> {
    let path = resolver.resolve_file(source, source)?;
    specs.push(RgbaSourceSpec {
        source: source.to_string(),
        output: shader_rgba_output_name(source),
        path,
        role: role.to_string(),
        shader_samplers: shader_samplers
            .iter()
            .map(|sampler| (*sampler).to_string())
            .collect(),
    });
    Ok(())
}

pub(crate) fn build_map_material_contract(
    rgba_specs: &[RgbaSourceSpec],
    raw_specs: &[RawMaterialSpec],
    shader_sources: &BTreeMap<String, String>,
    shader_constants: BTreeMap<String, ShaderConstantMeta>,
    shader_vector3_constants: BTreeMap<String, ShaderVector3Meta>,
) -> Result<MapMaterialContract> {
    let mut shipped_sources = BTreeMap::new();
    for spec in rgba_specs {
        let info = dds_info(&spec.path)?;
        shipped_sources.insert(
            spec.source.clone(),
            MaterialSourceMeta {
                output: shader_sources
                    .get(&spec.source)
                    .cloned()
                    .unwrap_or_else(|| spec.output.clone()),
                mip_output: material_mip_output_name(&spec.source, info.mip_count),
                role: spec.role.clone(),
                shader_samplers: spec.shader_samplers.clone(),
                dimensions: info.dimensions,
                dds_format: info.format,
                mip_count: info.mip_count,
                preservation: material_rgba_preservation(info.mip_count),
            },
        );
    }
    for spec in raw_specs {
        let info = dds_info(&spec.path)?;
        shipped_sources.insert(
            spec.source.clone(),
            MaterialSourceMeta {
                output: spec.output.clone(),
                mip_output: None,
                role: spec.role.clone(),
                shader_samplers: spec.shader_samplers.clone(),
                dimensions: info.dimensions,
                dds_format: info.format,
                mip_count: info.mip_count,
                preservation: "raw_dds_preserved".to_string(),
            },
        );
    }
    Ok(MapMaterialContract {
        implemented_visible_level: "production_terrain_material_contract_runtime_inputs_required"
            .to_string(),
        shader_constants,
        shader_vector3_constants,
        shipped_sources,
        generated_runtime_inputs: generated_runtime_material_inputs(),
    })
}

pub(crate) fn load_shader_constant_meta_from_text(
    text: &str,
) -> BTreeMap<String, ShaderConstantMeta> {
    let text = strip_comments(text);
    let mut out = BTreeMap::new();
    for key in [
        "TERRAIN_TILE_FREQ",
        "TEXELS_PER_TILE",
        "MAP_NUM_TILES",
        "ATLAS_TEXEL_POW2_EXPONENT",
        "COLORMAP_OVERLAY_STRENGTH",
        "COLORMAP_MUD_OVERLAY_STRENGTH",
        "MUD_TILING",
        "MUD_NORMAL_CUTOFF",
        "MUD_STRENGHTEN",
        "MUD_CAM_MIN",
        "MUD_CAM_MAX",
        "SNOW_OPACITY_MIN",
        "SNOW_OPACITY_MAX",
        "SNOW_CAM_MIN",
        "SNOW_CAM_MAX",
        "SNOW_START_HEIGHT",
        "SNOW_RIDGE_START_HEIGHT",
        "SNOW_NORMAL_START",
        "SNOW_CLIFFS",
        "SNOW_SPEC_GLOSS_MULT",
        "SNOW_TILING",
        "SNOW_NOISE_TILING",
        "SNOW_FROST_MIN_EFFECT",
        "CITY_LIGHTS_TILING",
        "CITY_LIGHTS_INTENSITY",
        "FOG_BEGIN",
        "FOG_END",
        "FOG_MAX",
        "GB_CAM_MIN",
        "GB_CAM_MAX",
        "GB_CAM_MAX_FILLING_CLAMP",
        "GB_THRESHOLD",
        "GB_THRESHOLD2",
        "GB_OPACITY_NEAR",
        "GB_OPACITY_FAR",
        "GB_STRENGTH_CH1",
        "GB_STRENGTH_CH2",
        "GB_FIRST_LAYER_PRIORITY",
        "SEC_MAP_TILE",
        "BORDER_NIGHT_DESATURATION_MAX",
        "BORDER_FOW_REMOVAL_FACTOR",
        "BORDER_LIGHT_REMOVAL_FACTOR",
        "MAP_SPECULAR_WIDTH",
        "WATER_COLOR_LIGHTNESS",
    ] {
        if let Some(value) = parse_shader_float_constant(&text, key) {
            out.insert(
                key.to_string(),
                ShaderConstantMeta {
                    value,
                    source: "gfx/FX/constants.fxh".to_string(),
                },
            );
        }
    }
    out
}

pub(crate) fn load_shader_vector3_constant_meta_from_text(
    text: &str,
) -> BTreeMap<String, ShaderVector3Meta> {
    let text = strip_comments(text);
    let mut out = BTreeMap::new();
    for key in ["SNOW_COLOR", "FOG_COLOR"] {
        if let Some(value) = parse_shader_float3_constant(&text, key) {
            out.insert(
                key.to_string(),
                ShaderVector3Meta {
                    value,
                    source: "gfx/FX/constants.fxh".to_string(),
                },
            );
        }
    }
    out
}

pub(crate) fn parse_shader_float_constant(text: &str, key: &str) -> Option<f32> {
    for line in text.lines() {
        if !line.contains(key) {
            continue;
        }
        // A line can mention the key without assigning it (e.g. a `//` doc
        // comment); skip it rather than aborting the whole scan.
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        if !left.split_whitespace().any(|part| part == key) {
            continue;
        }
        let value = right
            .split_once(';')
            .map(|(value, _)| value)
            .unwrap_or(right);
        let token = value.trim().trim_end_matches('f').trim();
        if let Ok(parsed) = token.parse::<f32>() {
            return Some(parsed);
        }
    }
    None
}

pub(crate) fn parse_shader_float3_constant(text: &str, key: &str) -> Option<[f32; 3]> {
    for line in text.lines() {
        if !line.contains(key) {
            continue;
        }
        // A line can mention the key without a parseable assignment (e.g. a
        // `//` doc comment); skip it so a later real definition still parses.
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        if !left.split_whitespace().any(|part| part == key) {
            continue;
        }
        let value = right
            .split_once(';')
            .map(|(value, _)| value)
            .unwrap_or(right);
        let Some(f3) = value.find("float3") else {
            continue;
        };
        let after = &value[f3 + "float3".len()..];
        let Some(open) = after.find('(') else {
            continue;
        };
        let start = open + 1;
        let Some(close) = after[start..].find(')') else {
            continue;
        };
        let end = close + start;
        let Ok(parts) = after[start..end]
            .split(',')
            .map(|part| part.trim().trim_end_matches('f').trim().parse::<f32>())
            .collect::<std::result::Result<Vec<_>, _>>()
        else {
            continue;
        };
        if parts.len() == 3 {
            return Some([parts[0], parts[1], parts[2]]);
        }
    }
    None
}

pub(crate) fn generated_runtime_material_inputs() -> BTreeMap<String, GeneratedRuntimeInput> {
    let mut inputs = BTreeMap::new();
    inputs.insert(
        "terrain_id_map".to_string(),
        GeneratedRuntimeInput {
            shader_symbols: vec!["TerrainIDMap".to_string()],
            consumers: vec!["gfx/FX/pdxmap.shader".to_string()],
            status: "resolver_derived_capture_validated_input".to_string(),
            reason: "TerrainIDMap RGBA packing is derived from resolver-read map/terrain.bmp plus common/terrain texture IDs and byte-validates against the bound captured resource; exact shader point/clamp sampling and four-ID material blending are implemented.".to_string(),
        },
    );
    for (name, symbols, consumers, reason) in [
        (
            "river_data",
            &["RiverData"][..],
            &["gfx/FX/river.shader"][..],
            "River mesh/data generation from rivers.bmp is engine/runtime generated.",
        ),
        (
            "gradient_border_channels",
            &[
                "GradientBorderChannel1",
                "GradientBorderChannel2",
                "GradientBorderChannel3",
            ][..],
            &[
                "gfx/FX/pdxmap.shader",
                "gfx/FX/pdxwater.shader",
                "gfx/FX/river.shader",
            ][..],
            "Gradient border channel generation is not stored as a shipped map file.",
        ),
        (
            "water_refraction",
            &["WaterRefraction"][..],
            &["gfx/FX/pdxwater.shader"][..],
            "Water refraction is a runtime render target, not a source DDS.",
        ),
        (
            "province_secondary_color_map",
            &["ProvinceSecondaryColorMap"][..],
            &["gfx/FX/pdxmap.shader", "gfx/FX/pdxwater.shader"][..],
            "Province secondary color overlays are runtime buffers.",
        ),
        (
            "snow_mud_data",
            &["SnowMudData", "SnowMudTexture"][..],
            &[
                "gfx/FX/pdxmap.shader",
                "gfx/FX/pdxwater.shader",
                "gfx/FX/river.shader",
                "gfx/FX/tree.shader",
            ][..],
            "Snow/mud coverage data is runtime/generated; shipped DDS files are only materials.",
        ),
        (
            "light_maps",
            &["LightIndexMap", "LightDataMap"][..],
            &[
                "gfx/FX/pdxmap.shader",
                "gfx/FX/pdxwater.shader",
                "gfx/FX/river.shader",
            ][..],
            "Tiled point-light textures are runtime/generated.",
        ),
        (
            "shadow_map",
            &["ShadowMap", "ShadowMapTextureMatrix"][..],
            &[
                "gfx/FX/pdxmap.shader",
                "gfx/FX/pdxwater.shader",
                "gfx/FX/river.shader",
                "gfx/FX/border.shader",
            ][..],
            "Shadow maps and matrices are runtime scene state.",
        ),
        (
            "terrain_vertex_height",
            &["VertexIn.height", "Input.prepos.y"][..],
            &["gfx/FX/pdxmap.shader"][..],
            "Terrain pixel height is shader vertex input/prepos state; prove engine heightmap-to-vertex mapping or capture runtime mesh height before applying height-dependent snow/mud.",
        ),
        (
            "terrain_input_layout",
            &["POSITION0", "TEXCOORD0", "D3D11_INPUT_ELEMENT_DESC"][..],
            &["gfx/FX/pdxmap.shader"][..],
            "Terrain vertex semantics require the runtime D3D11 input layout descriptor; ID3D11InputLayout does not expose descriptors after creation.",
        ),
        (
            "terrain_scene_state",
            &[
                "vCamPos",
                "vGBCamDistOverride_GBOutlineCutoff",
                "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed",
                "DayNight_Hour_SunDir",
                "AmbientPosX",
                "AmbientNegX",
                "AmbientPosY",
                "AmbientNegY",
                "AmbientPosZ",
                "AmbientNegZ",
                "SunDiffuseIntensity",
                "MoonDiffuseIntensity",
                "SunSpecularIntensity",
                "ShadowFadeFactor",
                "FOWFadeFactor",
            ][..],
            &[
                "gfx/FX/standardfuncsgfx.fxh",
                "gfx/FX/pdxmap.shader",
                "gfx/FX/pdxwater.shader",
            ][..],
            "Terrain lighting, distance fog, day/night, snow/mud fade, and border blend use runtime constant-buffer state, not shipped map assets.",
        ),
        (
            "tree_mask_texture",
            &["TreeMaskTexture"][..],
            &["gfx/FX/tree.shader"][..],
            "Tree mask/scatter texture is generated from source maps and runtime state.",
        ),
        (
            "fow_intel_textures",
            &["FoWTexture", "IntelMap"][..],
            &[
                "gfx/FX/pdxmap.shader",
                "gfx/FX/pdxwater.shader",
                "gfx/FX/border.shader",
            ][..],
            "Fog-of-war and intel textures are runtime gameplay buffers.",
        ),
    ] {
        inputs.insert(
            name.to_string(),
            GeneratedRuntimeInput {
                shader_symbols: symbols.iter().map(|value| (*value).to_string()).collect(),
                consumers: consumers.iter().map(|value| (*value).to_string()).collect(),
                status: "blocked_generated_runtime_input".to_string(),
                reason: reason.to_string(),
            },
        );
    }
    inputs
}

pub(crate) fn load_default_map_meta(path: &Path) -> Result<DefaultMapMeta> {
    let text = strip_comments(&read_latin1(path)?);
    let mut file_references = BTreeMap::new();
    for key in [
        "definitions",
        "provinces",
        "positions",
        "terrain",
        "rivers",
        "heightmap",
        "tree_definition",
        "continent",
        "adjacency_rules",
        "adjacencies",
        "ambient_object",
        "seasons",
    ] {
        if let Some(value) = parse_quoted_value(&text, key) {
            file_references.insert(key.to_string(), value);
        }
    }
    let tree_indices = extract_block_after(&text, "tree", 0)
        .map(|(inner, _)| parse_u32s(&inner))
        .unwrap_or_default();
    Ok(DefaultMapMeta {
        file_references,
        tree_indices,
    })
}

pub(crate) fn load_city_source_meta(path: &Path) -> Result<CitySourceMeta> {
    let text = strip_comments(&read_latin1(path)?);
    let mut city_groups = Vec::new();
    let mut search = 0usize;
    while let Some((inner, next)) = extract_block_after(&text, "city_group", search) {
        let mut buildings = Vec::new();
        let mut building_search = 0usize;
        while let Some((building_inner, building_next)) =
            extract_block_after(&inner, "building", building_search)
        {
            let meshes = extract_block_after(&building_inner, "mesh", 0)
                .map(|(mesh_inner, _)| parse_quoted_items(&mesh_inner))
                .unwrap_or_default();
            buildings.push(CityBuildingMeta {
                distance: parse_u32_value(&building_inner, "distance"),
                meshes,
            });
            building_search = building_next;
        }
        city_groups.push(CityGroupMeta {
            color_index: parse_u32_value(&inner, "color_index"),
            density: parse_f32_value(&inner, "density"),
            buildings,
        });
        search = next;
    }
    Ok(CitySourceMeta {
        types_source: parse_quoted_value(&text, "types_source"),
        pixel_step_x: parse_u32_value(&text, "pixel_step_x"),
        pixel_step_y: parse_u32_value(&text, "pixel_step_y"),
        city_groups,
    })
}

pub(crate) fn parse_quoted_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('"') else {
            break;
        };
        items.push(after_start[..end].to_string());
        rest = &after_start[end + 1..];
    }
    items
}

pub(crate) fn indexed_map_meta(image: &IndexImage) -> IndexedMapMeta {
    let index_counts = indexed_counts(&image.pixels);
    let nonzero_pixels = image.pixels.iter().filter(|index| **index != 0).count();
    IndexedMapMeta {
        width: image.width,
        height: image.height,
        distinct_indices: index_counts.len(),
        nonzero_pixels,
        index_counts,
    }
}

pub(crate) fn indexed_counts(pixels: &[u8]) -> BTreeMap<u8, usize> {
    let mut index_counts = BTreeMap::new();
    for index in pixels {
        *index_counts.entry(*index).or_insert(0) += 1;
    }
    index_counts
}

pub(crate) fn load_season_source_meta(path: &Path) -> Result<SeasonSourceMeta> {
    let text = strip_comments(&read_latin1(path)?);
    let mut blocks = Vec::new();
    for name in ["winter", "spring", "summer", "autumn"] {
        if let Some((inner, _)) = extract_block_after(&text, name, 0) {
            blocks.push(SeasonBlockMeta {
                name: name.to_string(),
                start_date: parse_token_value(&inner, "start_date"),
                end_date: parse_token_value(&inner, "end_date"),
            });
        }
    }
    Ok(SeasonSourceMeta { blocks })
}

#[derive(Clone, Copy)]
pub(crate) enum Delimiter {
    Semicolon,
    Whitespace,
}

pub(crate) fn delimited_source_meta(
    path: &Path,
    delimiter: Delimiter,
    category_column: Option<usize>,
) -> Result<TabularSourceMeta> {
    let text = strip_comments(&read_latin1(path)?);
    let mut rows = 0usize;
    let mut malformed_rows = 0usize;
    let mut field_counts = BTreeMap::new();
    let mut category_counts = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let fields = match delimiter {
            Delimiter::Semicolon => line
                .split(';')
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .collect::<Vec<_>>(),
            Delimiter::Whitespace => line.split_whitespace().collect::<Vec<_>>(),
        };
        if fields.is_empty() {
            malformed_rows += 1;
            continue;
        }
        rows += 1;
        *field_counts.entry(fields.len()).or_insert(0) += 1;
        if let Some(column) = category_column {
            if let Some(value) = fields.get(column) {
                *category_counts.entry((*value).to_string()).or_insert(0) += 1;
            } else {
                malformed_rows += 1;
            }
        }
    }
    Ok(TabularSourceMeta {
        rows,
        malformed_rows,
        field_counts,
        category_column,
        category_counts,
    })
}

pub(crate) fn load_ambient_source_meta(path: &Path) -> Result<AmbientSourceMeta> {
    let text = strip_comments(&read_latin1(path)?);
    let mut type_blocks = 0usize;
    let mut object_blocks = 0usize;
    let mut type_counts = BTreeMap::new();
    let mut object_name_counts = BTreeMap::new();
    let mut search = 0usize;
    while let Some((inner, next)) = extract_block_after(&text, "type", search) {
        type_blocks += 1;
        if let Some(type_name) = parse_quoted_value(&inner, "type") {
            *type_counts.entry(type_name).or_insert(0) += 1;
        }
        let mut object_search = 0usize;
        while let Some((object_inner, object_next)) =
            extract_block_after(&inner, "object", object_search)
        {
            object_blocks += 1;
            if let Some(name) = parse_quoted_value(&object_inner, "name") {
                *object_name_counts.entry(name).or_insert(0) += 1;
            }
            object_search = object_next;
        }
        search = next;
    }
    Ok(AmbientSourceMeta {
        type_blocks,
        object_blocks,
        type_counts,
        object_name_counts,
    })
}

pub(crate) fn load_common_building_rules(
    paths: &[(String, PathBuf)],
) -> Result<BTreeMap<String, CommonBuildingRuleMeta>> {
    let mut rules = BTreeMap::new();
    for (source, path) in paths {
        let text = strip_comments(&read_latin1(path)?);
        let Some((inner, _)) = extract_block_after(&text, "buildings", 0) else {
            continue;
        };
        for (name, block) in top_level_assignment_blocks(&inner) {
            let level_cap = extract_block_after(&block, "level_cap", 0).map(|(inner, _)| inner);
            let tags = extract_block_after(&block, "tags", 0)
                .map(|(inner, _)| parse_word_items(&inner))
                .unwrap_or_default();
            let key = if rules.contains_key(&name) {
                format!("{source}:{name}")
            } else {
                name
            };
            rules.insert(
                key,
                CommonBuildingRuleMeta {
                    source: source.clone(),
                    show_on_map: parse_u32_value(&block, "show_on_map"),
                    icon_frame: parse_u32_value(&block, "icon_frame"),
                    value: parse_u32_value(&block, "value"),
                    spawn_point: parse_word_value(&block, "spawn_point"),
                    province_max: level_cap
                        .as_deref()
                        .and_then(|inner| parse_u32_value(inner, "province_max")),
                    state_max: level_cap
                        .as_deref()
                        .and_then(|inner| parse_u32_value(inner, "state_max")),
                    centered: parse_bool_value(&block, "centered"),
                    always_shown: parse_bool_value(&block, "always_shown"),
                    only_display_if_exists: parse_bool_value(&block, "only_display_if_exists"),
                    tags,
                },
            );
        }
    }
    Ok(rules)
}

pub(crate) fn top_level_assignment_blocks(text: &str) -> Vec<(String, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && is_word_byte(bytes[i]) {
            i += 1;
        }
        if key_start == i {
            i = i.saturating_add(1);
            continue;
        }
        let key = &text[key_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'{' {
            continue;
        }
        if let Some(block) = extract_brace_at(text, i) {
            let next = i + block.len() + 2;
            out.push((key.to_string(), block));
            i = next;
        } else {
            break;
        }
    }
    out
}

pub(crate) fn parse_bool_value(text: &str, key: &str) -> Option<bool> {
    parse_word_value(text, key).and_then(|value| match value.as_str() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    })
}

pub(crate) fn parse_word_items(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub(crate) fn load_text_source_catalogs<'a>(
    paths: impl Iterator<Item = (&'a str, &'a PathBuf)>,
) -> Result<BTreeMap<String, TextSourceCatalog>> {
    let mut catalogs = BTreeMap::new();
    for (source, path) in paths {
        catalogs.insert(source.to_string(), load_text_source_catalog(path)?);
    }
    Ok(catalogs)
}

pub(crate) fn load_text_source_catalog(path: &Path) -> Result<TextSourceCatalog> {
    let text = strip_comments(&read_latin1(path)?);
    Ok(TextSourceCatalog {
        bytes: fs::metadata(path)?.len(),
        lines: text.lines().filter(|line| !line.trim().is_empty()).count(),
        key_counts: assignment_key_counts(&text),
        quoted_values: quoted_value_catalog(&text),
    })
}

pub(crate) fn assignment_key_counts(text: &str) -> BTreeMap<String, usize> {
    let bytes = text.as_bytes();
    let mut counts = BTreeMap::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && is_word_byte(bytes[i]) {
            i += 1;
        }
        if start != i {
            let key = &text[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                *counts.entry(key.to_ascii_lowercase()).or_insert(0) += 1;
            }
        }
        i = i.saturating_add(1);
    }
    counts
}

pub(crate) fn quoted_value_catalog(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut collected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (key, canonical) in [
        ("name", "name"),
        ("file", "file"),
        ("mesh", "mesh"),
        ("texturefile", "texturefile"),
        ("textureFile", "texturefile"),
        ("shader", "shader"),
    ] {
        let mut search = 0usize;
        while let Some(idx) = find_assignment(text, key, search) {
            let value = &text[idx..];
            if let Some(first) = value.find('"') {
                let rest = &value[first + 1..];
                if let Some(end) = rest.find('"') {
                    collected
                        .entry(canonical.to_string())
                        .or_default()
                        .insert(rest[..end].to_string());
                    search = idx + first + end + 2;
                    continue;
                }
            }
            search = idx.saturating_add(1);
        }
    }
    collected
        .into_iter()
        .map(|(key, values)| (key, values.into_iter().collect()))
        .collect()
}

pub(crate) fn load_country_tag_files(
    tag_paths: &[(String, PathBuf)],
) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (_, path) in tag_paths {
        let text = strip_comments(&read_latin1(&path)?);
        for line in text.lines() {
            let Some((left, right)) = line.split_once('=') else {
                continue;
            };
            let tag = left.trim();
            if !is_tag(tag) {
                continue;
            }
            let rel = right.trim().trim_matches('"').trim().to_string();
            if !rel.is_empty() {
                out.entry(tag.to_string()).or_insert(rel);
            }
        }
    }
    Ok(out)
}

pub(crate) fn is_tag(value: &str) -> bool {
    (2..=3).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

pub(crate) fn load_country_file_color(path: &Path) -> Result<Option<[u8; 3]>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = strip_comments(&read_latin1(path)?);
    Ok(parse_color_assignment(&text, "color"))
}

pub(crate) fn load_colors_txt_overrides(path: &Path) -> Result<BTreeMap<String, [u8; 3]>> {
    let mut out = BTreeMap::new();
    if !path.is_file() {
        return Ok(out);
    }
    let text = strip_comments(&read_latin1(path)?);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_uppercase() || bytes[i].is_ascii_digit()) {
            i += 1;
        }
        let tag = &text[start..i];
        if is_tag(tag) {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'{' {
                    let inner = extract_brace_at(&text, j).unwrap_or_default();
                    if let Some(color) = parse_color_assignment(&inner, "color") {
                        out.insert(tag.to_string(), color);
                    }
                    i = j + inner.len() + 2;
                    continue;
                }
            }
        }
        i = i.saturating_add(1);
    }
    Ok(out)
}

pub(crate) fn extract_brace_at(text: &str, brace: usize) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.get(brace) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let inner_start = brace + 1;
    for i in brace..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[inner_start..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn parse_color_assignment(text: &str, key: &str) -> Option<[u8; 3]> {
    let idx = find_assignment(text, key, 0)?;
    let rest = &text[idx..];
    let brace = rest.find('{')?;
    let kind = rest[..brace].trim().to_ascii_lowercase();
    let inner = extract_brace_at(rest, brace)?;
    let nums = inner
        .split_whitespace()
        .filter_map(|part| part.parse::<f32>().ok())
        .collect::<Vec<_>>();
    if nums.len() < 3 {
        return None;
    }
    if kind == "hsv" && nums[0] <= 1.0 {
        Some(hsv_to_rgb(
            nums[0],
            nums[1].clamp(0.0, 1.0),
            nums[2].clamp(0.0, 1.0),
        ))
    } else {
        Some([
            nums[0].round().clamp(0.0, 255.0) as u8,
            nums[1].round().clamp(0.0, 255.0) as u8,
            nums[2].round().clamp(0.0, 255.0) as u8,
        ])
    }
}

pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = (h.fract() * 6.0).max(0.0);
    let i = h.floor() as i32;
    let f = h - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    #[test]
    fn top_level_word_value_skips_nested_blocks() {
        // RAJ-shaped fixture: a gated set_cosmetic_tag inside an if block must
        // NOT win; the later top-level assignment must.
        let text = "\
if = {\n\tlimit = { has_dlc = \"Together for Victory\" }\n\tset_cosmetic_tag = RAJ_GATED\n}\n\
set_convoys = 20\n\
set_cosmetic_tag = RAJ_UK\n";
        assert_eq!(
            parse_top_level_word_value(text, "set_cosmetic_tag"),
            Some("RAJ_UK".to_string())
        );
        // Only-nested: no top-level assignment exists.
        let nested_only = "if = {\n\tset_cosmetic_tag = SPR_nationalist_spain\n}\n";
        assert_eq!(
            parse_top_level_word_value(nested_only, "set_cosmetic_tag"),
            None
        );
        // Key-suffix words must not match (e.g. another_set_cosmetic_tag).
        let suffixed = "another_set_cosmetic_tag = NOPE\nset_cosmetic_tag = YES_1\n";
        assert_eq!(
            parse_top_level_word_value(suffixed, "set_cosmetic_tag"),
            Some("YES_1".to_string())
        );
    }

    #[test]
    fn cosmetic_colors_parse_rgb_and_hsv_with_long_identifiers() {
        let dir = crate::test_support::temp_test_dir("cosmetic_colors");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("cosmetic.txt");
        std::fs::write(
            &path,
            "RAJ_UK = {\n\tcolor = rgb { 170  10  10 }\n\tcolor_ui = rgb { 200 10 10 }\n}\n\
             URG_is_unaligned_URG_but_has_normal_URG_flag = {\n\tcolor = rgb { 1 2 3 }\n}\n\
             SOME_HSV = {\n\tcolor = HSV { 0.0 1.0 1.0 }\n}\n",
        )
        .expect("write cosmetic.txt");
        let colors = load_cosmetic_colors(&path).expect("parse");
        assert_eq!(colors.get("RAJ_UK"), Some(&[170, 10, 10]));
        assert_eq!(
            colors.get("URG_is_unaligned_URG_but_has_normal_URG_flag"),
            Some(&[1, 2, 3])
        );
        assert_eq!(colors.get("SOME_HSV"), Some(&[255, 0, 0]));
    }

    #[test]
    fn cosmetic_color_overrides_replace_base_tag_colors() {
        let mut colors = BTreeMap::from([
            ("RAJ".to_string(), [226u8, 135, 40]),
            ("GER".to_string(), [50u8, 60, 70]),
        ]);
        let cosmetic_tags = BTreeMap::from([
            ("RAJ".to_string(), "RAJ_UK".to_string()),
            // Cosmetic without a color entry: base color must survive.
            ("GER".to_string(), "GER_NO_COLOR".to_string()),
        ]);
        let cosmetic_colors = BTreeMap::from([("RAJ_UK".to_string(), [170u8, 10, 10])]);
        apply_cosmetic_color_overrides(&mut colors, &cosmetic_tags, &cosmetic_colors);
        assert_eq!(colors.get("RAJ"), Some(&[170, 10, 10]));
        assert_eq!(colors.get("GER"), Some(&[50, 60, 70]));
    }

    /// NAME1 evidence gate against the real vanilla files: India (RAJ) must
    /// resolve the BRITISH RAJ cosmetic name and color, and the DLC/event-
    /// gated cosmetic tags must NOT leak in (they live inside if/limit
    /// blocks the replay does not evaluate).
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_cosmetic_tag_law() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let files = resolver
            .resolve_dir_files("history/countries", Some("txt"))
            .expect("history/countries");
        let cosmetic_tags = cosmetic_tags_1936_from_history_files(&files).expect("cosmetic tags");
        assert_eq!(
            cosmetic_tags.get("RAJ").map(String::as_str),
            Some("RAJ_UK"),
            "RAJ - British Raj.txt sets RAJ_UK at top level"
        );
        // The unconditional 1936 set (verified by brace-depth scan of the
        // shipped files).
        for tag in [
            "BRM", "GRE", "ICE", "INS", "JOR", "KUW", "LEB", "MAL", "PAL", "RAJ", "SYR", "TUR",
            "URG",
        ] {
            assert!(cosmetic_tags.contains_key(tag), "missing top-level {tag}");
        }
        // Gated-only countries must be absent (their set_cosmetic_tag is
        // inside if/limit blocks).
        for tag in ["BUL", "CAN", "COG", "CZE", "MEN", "SPR", "XSM"] {
            assert!(
                !cosmetic_tags.contains_key(tag),
                "{tag} cosmetic tag is DLC/event-gated and must not parse as 1936 state"
            );
        }
        // Color law end-to-end: RAJ takes the RAJ_UK cosmetic red.
        let colors = load_country_colors_from_game_resolver(&resolver).expect("colors");
        assert_eq!(
            colors.get("RAJ"),
            Some(&[170, 10, 10]),
            "cosmetic.txt RAJ_UK color must override the RAJ base color"
        );
    }

    #[test]
    fn country_tag_paths_are_resolved_from_common_root_for_game_resolver() {
        assert_eq!(
            country_tag_rel_for_game_resolver("countries/Kuwait.txt"),
            "common/countries/Kuwait.txt"
        );
        assert_eq!(
            country_tag_rel_for_game_resolver("common/countries/Germany.txt"),
            "common/countries/Germany.txt"
        );
    }

    #[test]
    fn unmapped_terrain_indices_remain_explicit_after_terrain_law_promotion() {
        let terrain_indices = IndexImage {
            width: 3,
            height: 2,
            pixels: vec![1, 2, 48, 2, 48, 99],
        };
        let terrain_texture_map = BTreeMap::from([(1, 4), (2, 7)]);

        let texture_ids = map_terrain_texture_ids(&terrain_indices, &terrain_texture_map);
        assert_eq!(texture_ids, vec![4, 7, 255, 7, 255, 255]);

        let meta = terrain_source_meta(&terrain_indices, &terrain_texture_map);
        assert_eq!(meta.unmapped_index_counts.get(&48), Some(&2));
        assert_eq!(meta.unmapped_index_counts.get(&99), Some(&1));
        assert_eq!(
            meta.implemented_parity_level,
            "terrain_idmap_four_id_atlas_mip_law_capture_gated_runtime_lighting"
        );
        assert!(!meta
            .generated_runtime_gaps
            .iter()
            .any(|gap| gap.contains("TerrainIDMap")));
    }

    #[test]
    fn generated_shader_inputs_are_blocked_in_contract() {
        let inputs = generated_runtime_material_inputs();
        assert_eq!(
            inputs["terrain_id_map"].status,
            "resolver_derived_capture_validated_input"
        );
        for key in [
            "river_data",
            "gradient_border_channels",
            "water_refraction",
            "province_secondary_color_map",
            "snow_mud_data",
            "light_maps",
            "shadow_map",
            "terrain_vertex_height",
            "terrain_input_layout",
            "terrain_scene_state",
        ] {
            let input = inputs.get(key).expect("required generated input");
            assert_eq!(input.status, "blocked_generated_runtime_input");
            assert!(!input.shader_symbols.is_empty());
        }
        assert!(inputs["river_data"]
            .shader_symbols
            .contains(&"RiverData".to_string()));
        assert!(inputs["terrain_id_map"]
            .shader_symbols
            .contains(&"TerrainIDMap".to_string()));
        assert!(inputs["gradient_border_channels"]
            .shader_symbols
            .contains(&"GradientBorderChannel1".to_string()));
        assert!(inputs["province_secondary_color_map"]
            .consumers
            .contains(&"gfx/FX/pdxmap.shader".to_string()));
        assert!(inputs["terrain_vertex_height"]
            .shader_symbols
            .contains(&"Input.prepos.y".to_string()));
        assert!(inputs["terrain_input_layout"]
            .shader_symbols
            .contains(&"D3D11_INPUT_ELEMENT_DESC".to_string()));
        assert!(inputs["terrain_scene_state"]
            .shader_symbols
            .contains(&"vCamPos".to_string()));
        assert!(inputs["terrain_scene_state"]
            .shader_symbols
            .contains(&"DayNight_Hour_SunDir".to_string()));
    }

    #[test]
    fn shader_float_constants_parse_source_syntax() {
        let text = r#"
            // TERRAIN_TILE_FREQ tweak note
            static const float TERRAIN_TILE_FREQ = 128.0f;
            // SNOW_COLOR doc comment without an assignment
            static const float MAP_NUM_TILES = 4.0f;
            static const float WATER_COLOR_LIGHTNESS = 0.5;
            static const float NIGHT_AMBIENT_BOOST = 3.0f;
            static const float GMT_OFFSET = 2793.0f;
            static const float FEATHER_MIN = -0.01f;
            static const float MOON_FEATHER_MAX = 0.01;
            static const float WATER_HEIGHT_RECP = 1.0f / WATER_HEIGHT;
            static const float3 SNOW_COLOR = float3( 0.46, 0.48, 0.69 );
        "#;
        assert_eq!(
            parse_shader_float_constant(text, "TERRAIN_TILE_FREQ"),
            Some(128.0)
        );
        assert_eq!(
            parse_shader_float_constant(text, "MAP_NUM_TILES"),
            Some(4.0)
        );
        assert_eq!(
            parse_shader_float_constant(text, "WATER_COLOR_LIGHTNESS"),
            Some(0.5)
        );
        assert_eq!(
            parse_shader_float_constant(text, "NIGHT_AMBIENT_BOOST"),
            Some(NIGHT_AMBIENT_BOOST)
        );
        assert_eq!(
            parse_shader_float_constant(text, "GMT_OFFSET"),
            Some(GMT_OFFSET)
        );
        assert_eq!(
            parse_shader_float_constant(text, "FEATHER_MIN"),
            Some(FEATHER_MIN)
        );
        assert_eq!(
            parse_shader_float_constant(text, "MOON_FEATHER_MAX"),
            Some(MOON_FEATHER_MAX)
        );
        assert_eq!(parse_shader_float_constant(text, "WATER_HEIGHT"), None);
        assert_eq!(
            parse_shader_float3_constant(text, "SNOW_COLOR"),
            Some([0.46, 0.48, 0.69])
        );
        assert_eq!(parse_shader_float3_constant(text, "MISSING_COLOR"), None);
    }

    #[test]
    fn material_mip_output_name_is_stable() {
        assert_eq!(
            shader_rgba_mip_output_name("map/terrain/snow_normal_rgb_diffuse_a.dds"),
            "shader_map_terrain_snow_normal_rgb_diffuse_a_dds_mips.rgba"
        );
        assert_eq!(
            material_mip_output_name("map/terrain/colormap_rgb_cityemissivemask_a.dds", 1),
            None
        );
        assert_eq!(
            material_rgba_preservation(2),
            "decoded_rgba_exact_2d_with_mip_chain"
        );
    }

    #[test]
    fn dlc_conditional_cosmetics_parse_canada_both_branches() {
        // NAME1b: the exact shape of `CAN - Canada.txt` (vanilla): two
        // top-level if-blocks, mutually exclusive on Together for Victory.
        let text = "\
capital = 468\n\
if = {\n\
\tlimit = { NOT = { has_dlc = \"Together for Victory\" } }\n\
\tset_cosmetic_tag = CAN_ALY\n\
}\n\
if = {\n\
\tlimit = { has_dlc = \"Together for Victory\" }\n\
\tadd_ideas = CAN_great_depression_1\n\
\tset_cosmetic_tag = CAN_UK\n\
}\n";
        let got = dlc_conditional_cosmetics_from_text(text);
        assert_eq!(
            got,
            vec![
                ConditionalCosmetic {
                    cosmetic: "CAN_ALY".to_string(),
                    dlc: "Together for Victory".to_string(),
                    negated: true,
                },
                ConditionalCosmetic {
                    cosmetic: "CAN_UK".to_string(),
                    dlc: "Together for Victory".to_string(),
                    negated: false,
                },
            ]
        );
    }

    #[test]
    fn dlc_conditional_cosmetics_parse_congo_single_positive() {
        // `COG - Congo.txt`: one positive-only `IF`/`has_dlc` block.
        let text = "\
IF = {\n\
\tlimit = {\n\t\thas_dlc = \"Gotterdammerung\"\n\t}\n\
\tset_cosmetic_tag = COG_belgian_congo\n\
}\n";
        assert_eq!(
            dlc_conditional_cosmetics_from_text(text),
            vec![ConditionalCosmetic {
                cosmetic: "COG_belgian_congo".to_string(),
                dlc: "Gotterdammerung".to_string(),
                negated: false,
            }]
        );
    }

    #[test]
    fn dlc_conditional_cosmetics_reject_non_dlc_and_nested_gates() {
        // SPR civil-war cosmetics sit inside deeper political/event blocks with
        // NON-has_dlc limits -> must NOT be read as start-state DLC gates.
        let spain = "\
country_event = {\n\
\tif = {\n\t\tlimit = { has_country_flag = SPR_civil_war }\n\t\tset_cosmetic_tag = SPR_nationalist_spain\n\t}\n\
}\n";
        assert!(dlc_conditional_cosmetics_from_text(spain).is_empty());
        // A top-level if whose limit mixes has_dlc with another condition is not
        // a pure DLC gate (could carry ideology/date state we do not evaluate).
        let mixed = "\
if = {\n\tlimit = { has_dlc = \"X\" ruling_party = fascism }\n\tset_cosmetic_tag = FOO\n}\n";
        assert!(dlc_conditional_cosmetics_from_text(mixed).is_empty());
    }

    #[test]
    fn pure_has_dlc_accepts_plain_and_not_rejects_extra() {
        assert_eq!(
            parse_pure_has_dlc(" has_dlc = \"Waking the Tiger\" "),
            Some(("Waking the Tiger".to_string(), false))
        );
        assert_eq!(
            parse_pure_has_dlc(" NOT = { has_dlc = \"Together for Victory\" } "),
            Some(("Together for Victory".to_string(), true))
        );
        assert_eq!(parse_pure_has_dlc("has_dlc = \"X\" date > 1937.1.1"), None);
        assert_eq!(parse_pure_has_dlc("owns_state = 1"), None);
    }

    #[test]
    fn unterminated_brace_blocks_do_not_panic_on_non_ascii_tails() {
        // An unterminated top-level if-block whose text ends in a non-ASCII
        // char (latin1-decoded history files) must not panic on a byte-slice
        // boundary; it yields no cosmetics.
        let unterminated_non_ascii = "if = { limit = { has_dlc = \"X\" } é";
        assert!(dlc_conditional_cosmetics_from_text(unterminated_non_ascii).is_empty());
        // Text ending exactly at the opening '{'.
        let ends_at_open = "if = {";
        assert!(dlc_conditional_cosmetics_from_text(ends_at_open).is_empty());
        // Same for extract_braced_block: an unterminated `limit = {` must
        // return None, not panic.
        assert_eq!(
            extract_braced_block("limit = { has_dlc = \"X\" é", "limit"),
            None
        );
        assert_eq!(extract_braced_block("limit = {", "limit"), None);
        // A well-formed block still parses.
        assert_eq!(
            extract_braced_block("limit = { has_dlc = \"X\" }", "limit"),
            Some(" has_dlc = \"X\" ".to_string())
        );
    }

    fn state_fixture_writer(
        dir: &std::path::Path,
    ) -> impl Fn(&str, &str) -> (String, PathBuf) + '_ {
        std::fs::create_dir_all(dir).expect("create temp dir");
        move |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).expect("write fixture");
            (name.to_string(), path)
        }
    }

    /// FW7-F6: well-formed state files still load, and the parsing behavior the
    /// old test pinned alongside the silent drops (repeated `victory_points`
    /// accumulation, `impassable = yes`, absent `owner`) is unchanged.
    #[test]
    fn load_states_parses_well_formed_files() {
        let dir = crate::test_support::temp_test_dir("load_states_wellformed");
        let write = state_fixture_writer(&dir);
        let owned = write(
            "1-owned.txt",
            "state = {\n\tid = 5\n\tname = \"STATE_5\"\n\towner = GER\n\tprovinces = { 10 11 }\n}\n",
        );
        let vp = write(
            "3-vp.txt",
            "state = {\n\tid = 7\n\timpassable = yes\n\tmanpower = 1234\n\tprovinces = { 20 21 }\n\
             \tvictory_points = { 20 3 }\n\tvictory_points = { 20 2 21 5 }\n}\n",
        );
        let (states, province_to_state, state_owners) =
            load_states(&[owned, vp]).expect("load_states");
        assert_eq!(states.len(), 2);
        let s5 = states.get(&5).expect("state 5");
        assert_eq!(s5.name_key, "STATE_5");
        assert_eq!(s5.provinces, vec![10, 11]);
        assert!(!s5.impassable);
        assert_eq!(state_owners.get(&5).map(String::as_str), Some("GER"));
        assert_eq!(province_to_state.get(&10), Some(&5));
        assert_eq!(province_to_state.get(&11), Some(&5));
        let s7 = states.get(&7).expect("state 7");
        assert_eq!(s7.victory_points.get(&20), Some(&5));
        assert_eq!(s7.victory_points.get(&21), Some(&5));
        assert_eq!(s7.vp_total, 10);
        assert!(s7.impassable);
        assert_eq!(s7.manpower, 1234);
        assert!(!state_owners.contains_key(&7));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FW7-F6 regression: a state file with no parseable `id` used to vanish
    /// with a bare `continue`. It is now a named gap in a refusal.
    #[test]
    fn load_states_refuses_file_without_id() {
        let dir = crate::test_support::temp_test_dir("load_states_no_id");
        let write = state_fixture_writer(&dir);
        let no_id = write("0-no_id.txt", "state = { name = \"STATE_NO_ID\" }\n");
        let ok = write("1-ok.txt", "state = {\n\tid = 5\n\tprovinces = { 10 }\n}\n");
        let err = load_states(&[no_id, ok]).expect_err("missing id must refuse");
        let message = err.to_string();
        assert!(message.contains("history/states refused"), "{message}");
        assert!(message.contains("0-no_id.txt"), "{message}");
        assert!(message.contains("no parseable `id ="), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FW7-F6 regression: duplicate state ids used to overwrite last-writer-wins
    /// and — worse — non-atomically, so the superseded state's owner row and its
    /// province mappings survived pointing at a `StateMeta` that no longer
    /// listed them (the old test asserted `province_to_state[10] == 5` while
    /// `states[5].provinces == [11, 12]`).
    #[test]
    fn load_states_refuses_duplicate_state_ids_without_partial_mutation() {
        let dir = crate::test_support::temp_test_dir("load_states_dup_id");
        let write = state_fixture_writer(&dir);
        let first = write(
            "1-first.txt",
            "state = {\n\tid = 5\n\tname = \"STATE_5_FIRST\"\n\towner = GER\n\tprovinces = { 10 11 }\n}\n",
        );
        let second = write(
            "2-second.txt",
            "state = {\n\tid = 5\n\tname = \"STATE_5_SECOND\"\n\towner = FRA\n\tprovinces = { 12 }\n}\n",
        );
        let err = load_states(&[first, second]).expect_err("duplicate id must refuse");
        let message = err.to_string();
        assert!(message.contains("redefines state id 5"), "{message}");
        assert!(message.contains("1-first.txt"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FW7-F6 regression: one province listed by two distinct states used to be
    /// resolved last-writer-wins in `province_to_state` while both
    /// `StateMeta.provinces` lists kept it.
    #[test]
    fn load_states_refuses_province_claimed_by_two_states() {
        let dir = crate::test_support::temp_test_dir("load_states_dup_province");
        let write = state_fixture_writer(&dir);
        let first = write(
            "1-first.txt",
            "state = {\n\tid = 5\n\tprovinces = { 10 11 }\n}\n",
        );
        let second = write(
            "2-second.txt",
            "state = {\n\tid = 6\n\tprovinces = { 11 12 }\n}\n",
        );
        let err = load_states(&[first, second]).expect_err("double claim must refuse");
        let message = err.to_string();
        assert!(
            message.contains("province 11 is already claimed by state 5"),
            "{message}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FW7-F6: every gap in the directory is named in one refusal, not just the
    /// first — one run tells the operator the whole story.
    #[test]
    fn load_states_reports_every_gap_in_one_refusal() {
        let dir = crate::test_support::temp_test_dir("load_states_all_gaps");
        let write = state_fixture_writer(&dir);
        let no_id = write("0-no_id.txt", "state = { name = \"NO_ID\" }\n");
        let first = write(
            "1-first.txt",
            "state = {\n\tid = 5\n\tprovinces = { 10 }\n}\n",
        );
        let dup = write(
            "2-dup.txt",
            "state = {\n\tid = 5\n\tprovinces = { 11 }\n}\n",
        );
        let err = load_states(&[no_id, first, dup]).expect_err("must refuse");
        let message = err.to_string();
        assert!(message.contains("2 identifier gap(s)"), "{message}");
        assert!(message.contains("0-no_id.txt"), "{message}");
        assert!(message.contains("redefines state id 5"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_strategic_regions_parses_well_formed_files() {
        let dir = crate::test_support::temp_test_dir("load_regions_wellformed");
        let write = state_fixture_writer(&dir);
        let first = write(
            "1-first.txt",
            "strategic_region = {\n\tid = 9\n\tname = \"REGION_FIRST\"\n\tprovinces = { 100 101 }\n}\n",
        );
        let (region_to_provinces, region_meta, province_to_region) =
            load_strategic_regions(&[first]).expect("load_strategic_regions");
        assert_eq!(region_to_provinces.get(&9), Some(&vec![100, 101]));
        let meta = region_meta.get(&9).expect("region 9");
        assert_eq!(meta.name, "REGION_FIRST");
        assert_eq!(meta.province_count, 2);
        assert_eq!(province_to_region.get(&100), Some(&9));
        assert_eq!(province_to_region.get(&101), Some(&9));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FW7-F6 regression: the region loader dropped a missing id silently and
    /// resolved a duplicate id first-writer-wins — the opposite of `load_states`'
    /// last-writer-wins, for no stated reason. Both are refusals now, and both
    /// loaders behave identically.
    #[test]
    fn load_strategic_regions_refuses_missing_and_duplicate_ids() {
        let dir = crate::test_support::temp_test_dir("load_regions_gaps");
        let write = state_fixture_writer(&dir);
        let no_id = write(
            "0-no_id.txt",
            "strategic_region = { name = \"NAMELESS\" }\n",
        );
        let first = write(
            "1-first.txt",
            "strategic_region = {\n\tid = 9\n\tprovinces = { 100 101 }\n}\n",
        );
        let second = write(
            "2-second.txt",
            "strategic_region = {\n\tid = 9\n\tprovinces = { 102 }\n}\n",
        );
        let err = load_strategic_regions(&[no_id, first, second]).expect_err("gaps must refuse");
        let message = err.to_string();
        assert!(
            message.contains("map/strategicregions refused"),
            "{message}"
        );
        assert!(message.contains("0-no_id.txt"), "{message}");
        assert!(
            message.contains("redefines strategic region id 9"),
            "{message}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FW7-F6 regression: a province listed by two strategic regions.
    #[test]
    fn load_strategic_regions_refuses_province_claimed_twice() {
        let dir = crate::test_support::temp_test_dir("load_regions_dup_province");
        let write = state_fixture_writer(&dir);
        let first = write(
            "1-first.txt",
            "strategic_region = {\n\tid = 9\n\tprovinces = { 100 101 }\n}\n",
        );
        let second = write(
            "2-second.txt",
            "strategic_region = {\n\tid = 10\n\tprovinces = { 101 }\n}\n",
        );
        let err = load_strategic_regions(&[first, second]).expect_err("double claim must refuse");
        assert!(
            err.to_string()
                .contains("province 101 is already claimed by strategic region 9"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FW7-F6: the enumeration in a refusal is bounded but the count is exact.
    #[test]
    fn id_gap_refusal_bounds_the_enumeration_but_not_the_count() {
        let gaps = (0..MAX_REPORTED_ID_GAPS + 5)
            .map(|index| format!("gap-{index}"))
            .collect::<Vec<_>>();
        let err = refuse_on_id_gaps("history/states", &gaps).expect_err("must refuse");
        let message = err.to_string();
        assert!(
            message.contains(&format!("{} identifier gap(s)", MAX_REPORTED_ID_GAPS + 5)),
            "{message}"
        );
        assert!(message.contains("and 5 more"), "{message}");
        assert!(
            !message.contains(&format!("gap-{}", MAX_REPORTED_ID_GAPS)),
            "{message}"
        );
        assert!(refuse_on_id_gaps("history/states", &[]).is_ok());
    }
}
