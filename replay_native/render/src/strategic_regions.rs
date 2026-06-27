//! `strategic_regions` — the province→strategic-region JOIN the FROZEN-SEAS dynamic
//! branch needs. The runtime weather snapshot carries `weather.regions.<rid>.phenomenon`
//! (a per-REGION flag list that may contain `arctic_water` = frozen seas), but the map's
//! dynamic LUT is per-PROVINCE. To set `frozen` on the right provinces we need:
//!
//!   * province → strategic region   (which region a sea province belongs to), and
//!   * which provinces are SEA        (frozen seas must NOT freeze land in the same region).
//!
//! Both are parsed from the Installed content via the mod-over-vanilla [`assets::resolver`]
//! (the mod ships its OWN `map/strategicregions/` + `map/definition.csv`, which override
//! vanilla). Snow/mud are per-province DIRECT and never need this join — ONLY frozen does.
//!
//! ## Parsing discipline (Codex-hardened)
//! A `strategicregions/*.txt` file is a Clausewitz block:
//! ```text
//! strategic_region={
//!     id=100
//!     name="STRATEGICREGION_100"
//!     provinces={ 2800 2825 5388 8742 8766 }
//!     weather={ period={ between={ 0.0 30.0 } temperature={ 5.0 30.0 } ... } ... }
//! }
//! ```
//! A naive "grab every integer" would wrongly pull `temperature`/`between` numbers from the
//! `weather` block. We therefore brace-track and extract ONLY the integers inside the
//! TOP-LEVEL `provinces={ ... }` block, and the TOP-LEVEL `id=N`. The region id comes from
//! the FILE CONTENTS (`id=`), never the filename. `#` comments are stripped. Float tokens
//! (those containing a `.`) are ignored even inside provinces (province ids are integers).

use std::collections::{HashMap, HashSet};

use assets::resolver::GameResolver;

/// The parsed province↔strategic-region join + the set of SEA provinces.
#[derive(Debug, Clone, Default)]
pub struct StrategicRegions {
    /// province id → strategic-region id (every province listed in any region file).
    province_to_region: HashMap<u32, u32>,
    /// strategic-region id → its province ids (the inverse, for the frozen-seas join).
    region_to_provinces: HashMap<u32, Vec<u32>>,
    /// the set of SEA province ids (from `definition.csv` type column == "sea"). LAKE is
    /// deliberately EXCLUDED (frozen SEAS): the game's `arctic_water` is a sea phenomenon.
    sea_provinces: HashSet<u32>,
}

impl StrategicRegions {
    /// Load + parse the strategic-region province lists and the sea-province set from the
    /// resolved installed content (mod overrides vanilla). Returns `Err` with a human message if
    /// neither a mod nor vanilla `map/strategicregions/` directory can be found — the caller
    /// then runs WITHOUT a frozen-seas layer (snow/mud still work) rather than fabricate.
    pub fn load(resolver: &GameResolver) -> Result<StrategicRegions, String> {
        let mut sr = StrategicRegions::default();

        // 1) strategic regions: parse every `*.txt` in the EFFECTIVE strategicregions dir.
        //
        //    `map/strategicregions` is REPLACE_PATH'd by this mod (descriptor.mod /
        //    test.mod): when the mod ships a strategicregions/ directory it FULLY REPLACES
        //    vanilla's — vanilla files are not loaded at all (no per-file overlay). This
        //    matches the offline asset builder (build_strategic_regions.py) that produced
        //    `province_to_strategic_region.json`, so the parsed result is byte-faithful to
        //    the precomputed asset. Only when NO mod strategicregions dir exists do we fall
        //    back to vanilla. (Codex-confirmed: the prior per-file overlay was a latent
        //    replace_path bug that could leak a vanilla-only region file's provinces.)
        let dir = match effective_strategicregion_dir(resolver) {
            Some(d) => d,
            None => return Err("map/strategicregions not found in mod or vanilla".to_string()),
        };
        // Deterministic SORTED-filename order so a province claimed by >1 region resolves
        // identically every run AND matches the Python builder's sorted/last-writer-wins
        // rule. `std::fs::read_dir` is OS-unordered; relying on it (as the old code did) made
        // the result non-deterministic — a NO-FABRICATION hazard if collisions ever appear.
        let mut files: Vec<std::path::PathBuf> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("txt"))
                .collect(),
            Err(e) => return Err(format!("read {}: {e}", dir.display())),
        };
        files.sort();

        // Track duplicate REGION ids (same id in two files) and province COLLISIONS (a
        // province claimed by two distinct regions) so a malformed/edited region set surfaces
        // a diagnostic instead of silently fabricating an assignment.
        let mut seen_region_file: HashMap<u32, std::path::PathBuf> = HashMap::new();
        for path in &files {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Some((rid, provs)) = parse_region_file(&text) else {
                continue;
            };
            // A duplicate region id: first sorted file wins (matches the Python builder),
            // the later one is skipped entirely (its provinces are NOT merged in).
            if let Some(prev) = seen_region_file.get(&rid) {
                eprintln!(
                    "[strategic_regions] WARN duplicate region id {rid} in {} and {} — keeping the first",
                    prev.display(),
                    path.display()
                );
                continue;
            }
            seen_region_file.insert(rid, path.clone());
            for &p in &provs {
                // A province should belong to exactly one region. If two distinct regions
                // claim it, the LAST sorted file wins (overwrite) — matching the Python
                // builder so the parser agrees with the precomputed asset — and we log it.
                if let Some(&prev_rid) = sr.province_to_region.get(&p) {
                    if prev_rid != rid {
                        eprintln!(
                            "[strategic_regions] WARN province {p} claimed by regions {prev_rid} and {rid} — using {rid} ({})",
                            path.display()
                        );
                    }
                }
                sr.province_to_region.insert(p, rid);
            }
            sr.region_to_provinces.entry(rid).or_default().extend(provs);
        }
        if sr.province_to_region.is_empty() {
            return Err("no strategic-region province lists parsed".to_string());
        }

        // 2) sea provinces from definition.csv (mod overrides vanilla as one whole file).
        if let Some(def) = resolver.resolve("map/definition.csv") {
            if let Ok(text) = std::fs::read_to_string(&def) {
                sr.sea_provinces = parse_sea_provinces(&text);
            }
        }
        Ok(sr)
    }

    /// Build directly from already-parsed maps (test / alternate-source constructor).
    pub fn from_parts(
        province_to_region: HashMap<u32, u32>,
        region_to_provinces: HashMap<u32, Vec<u32>>,
        sea_provinces: HashSet<u32>,
    ) -> StrategicRegions {
        StrategicRegions {
            province_to_region,
            region_to_provinces,
            sea_provinces,
        }
    }

    pub fn region_of_province(&self, province_id: u32) -> Option<u32> {
        self.province_to_region.get(&province_id).copied()
    }

    pub fn provinces_in_region(&self, region_id: u32) -> &[u32] {
        self.region_to_provinces
            .get(&region_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn is_sea(&self, province_id: u32) -> bool {
        self.sea_provinces.contains(&province_id)
    }

    pub fn province_count(&self) -> usize {
        self.province_to_region.len()
    }

    pub fn region_count(&self) -> usize {
        self.region_to_provinces.len()
    }

    pub fn sea_count(&self) -> usize {
        self.sea_provinces.len()
    }

    /// The FROZEN-SEAS join (Codex (b)): given the snapshot's per-region phenomenon map
    /// (`weather.regions.<rid>.phenomenon` arrays), return the set of SEA province ids that
    /// should be drawn frozen — every sea province of any region whose phenomenon list
    /// contains `arctic_water`. NO fabrication: a region with no `arctic_water` (or no
    /// phenomenon entry) freezes nothing; LAND provinces in an arctic region are NEVER
    /// frozen (sea-only filter); a region id we don't know is skipped.
    pub fn frozen_sea_provinces(&self, weather_regions: &serde_json::Value) -> HashSet<u32> {
        let mut frozen = HashSet::new();
        let Some(regions) = weather_regions.as_object() else {
            return frozen;
        };
        for (rid_key, rval) in regions {
            let Ok(rid) = rid_key.parse::<u32>() else {
                continue;
            };
            if !phenomenon_has_arctic_water(rval) {
                continue;
            }
            for &pid in self.provinces_in_region(rid) {
                if self.is_sea(pid) {
                    frozen.insert(pid);
                }
            }
        }
        frozen
    }
}

/// True iff a region's snapshot value carries a `phenomenon` array containing the literal
/// `"arctic_water"`. Absent / non-array / not-containing → false (no fabrication).
fn phenomenon_has_arctic_water(region_value: &serde_json::Value) -> bool {
    region_value
        .get("phenomenon")
        .and_then(serde_json::Value::as_array)
        .map(|arr| arr.iter().any(|v| v.as_str() == Some("arctic_water")))
        .unwrap_or(false)
}

/// The single EFFECTIVE strategicregions directory under `replace_path` semantics: the mod's
/// dir if it exists (it FULLY replaces vanilla — no merge), otherwise vanilla's. Returns `None`
/// if neither is present. (`map/strategicregions` is in the mod's `replace_path` list, so a
/// mod dir present means vanilla is not loaded at all.)
fn effective_strategicregion_dir(resolver: &GameResolver) -> Option<std::path::PathBuf> {
    if let Some(m) = &resolver.mod_root {
        let d = m.join("map").join("strategicregions");
        if d.is_dir() {
            return Some(d);
        }
    }
    let v = resolver.vanilla.join("map").join("strategicregions");
    if v.is_dir() {
        return Some(v);
    }
    None
}

/// Strip an end-of-line `#` comment from a line (the game comment syntax). A `#` inside a quoted
/// string would be mishandled, but strategicregions province/id data is never quoted, so a
/// plain first-`#` cut is correct here.
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Parse ONE strategicregions file's text → `(region_id, province_ids)`. Brace-tracked so
/// only the TOP-LEVEL `provinces={ ... }` integers (and the top-level `id=N`) are taken; the
/// `weather={ period={ temperature={...} } }` numbers are ignored. Returns `None` if the
/// file has no `id=` / no provinces. (Codex (d): id from contents, comments stripped, the
/// weather block's floats/integers excluded by depth + the float-token guard.)
pub fn parse_region_file(text: &str) -> Option<(u32, Vec<u32>)> {
    // Tokenize on whitespace and braces/equals after stripping comments. We track the brace
    // DEPTH and whether we are currently inside the `provinces` block. The outermost block is
    // `strategic_region={ ... }` (depth 1 once entered); `id` and `provinces` live at depth 1.
    let mut region_id: Option<u32> = None;
    let mut provinces: Vec<u32> = Vec::new();

    // depth 0 = before/at the strategic_region={ ; once we pass the first `{` we are at
    // depth 1 (the region body), where `id` and `provinces` are keys.
    let mut depth: i32 = 0;
    // when Some(d), we are inside the provinces block whose `{` opened at depth d; we read
    // integer tokens until the matching `}` closes it.
    let mut in_provinces_at: Option<i32> = None;
    // when we just saw the `id` key at depth 1 and an `=`, the next number token is the id.
    let mut expect_id_value = false;
    // when we saw `provinces` at depth 1, the next `{` opens the provinces block.
    let mut pending_provinces = false;

    for raw_line in text.lines() {
        let line = strip_comment(raw_line);
        // split into tokens, treating { } = as their own tokens.
        for tok in tokenize(line) {
            match tok {
                Tok::Open => {
                    depth += 1;
                    if pending_provinces {
                        in_provinces_at = Some(depth);
                        pending_provinces = false;
                    }
                }
                Tok::Close => {
                    if let Some(d) = in_provinces_at {
                        if depth == d {
                            in_provinces_at = None;
                        }
                    }
                    depth -= 1;
                    // a stray `=`/key state cannot survive a block close.
                    expect_id_value = false;
                    pending_provinces = false;
                }
                Tok::Eq => {
                    // an `=` after the `id` key arms the id-value capture; for `provinces`
                    // the `{` (not the `=`) opens the block, handled by pending_provinces.
                }
                Tok::Word(w) => {
                    // inside the provinces block: collect integer province ids only.
                    if in_provinces_at.is_some() {
                        if let Some(pid) = parse_province_int(w) {
                            provinces.push(pid);
                        }
                        continue;
                    }
                    // capture the id VALUE if armed (the token right after `id` `=`).
                    if expect_id_value {
                        expect_id_value = false;
                        if let Ok(v) = w.parse::<u32>() {
                            region_id = Some(v);
                        }
                        continue;
                    }
                    // keys at the region-body depth (1). Be tolerant of `id=100` packed by
                    // tokenize into the word "id" then Eq then the number.
                    if depth == 1 {
                        match w {
                            "id" => expect_id_value = true,
                            "provinces" => pending_provinces = true,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    match region_id {
        Some(rid) if !provinces.is_empty() => Some((rid, provinces)),
        _ => None,
    }
}

/// A province token must be a pure non-negative integer; a token containing `.` (a float,
/// e.g. a `temperature` value that somehow leaked) is rejected. Returns None for non-ints.
fn parse_province_int(w: &str) -> Option<u32> {
    if w.contains('.') {
        return None;
    }
    w.parse::<u32>().ok()
}

/// Minimal Clausewitz tokens for the region parser.
enum Tok<'a> {
    Open,
    Close,
    Eq,
    Word(&'a str),
}

/// Split a (comment-stripped) line into `{ } =` punctuation tokens and whitespace-delimited
/// words. Handles `id=100`, `provinces={`, `5388 }` etc. by separating the punctuation.
/// ASCII-only by byte index (the game region files are ASCII; `{ } =` are 1-byte tokens).
fn tokenize(line: &str) -> Vec<Tok<'_>> {
    let mut out: Vec<Tok<'_>> = Vec::new();
    let bytes = line.as_bytes();
    let mut start: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let punct = matches!(b, b'{' | b'}' | b'=');
        let ws = (b as char).is_whitespace();
        if punct || ws {
            if let Some(s) = start.take() {
                if i > s {
                    out.push(Tok::Word(&line[s..i]));
                }
            }
            if punct {
                out.push(match b {
                    b'{' => Tok::Open,
                    b'}' => Tok::Close,
                    _ => Tok::Eq,
                });
            }
        } else if start.is_none() {
            start = Some(i);
        }
        i += 1;
    }
    if let Some(s) = start.take() {
        if bytes.len() > s {
            out.push(Tok::Word(&line[s..]));
        }
    }
    out
}

/// Parse `map/definition.csv` → the set of SEA province ids. The CSV is `;`-delimited:
/// `id;R;G;B;type;coastal;terrain;continent`. The `type` is field index 4 (0-based) — using
/// index 5 would read `coastal` (Codex (b) off-by-one). Only `type == "sea"` counts; `land`
/// and `lake` are excluded (frozen SEAS, not lakes).
pub fn parse_sea_provinces(csv: &str) -> HashSet<u32> {
    let mut sea = HashSet::new();
    for line in csv.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(';').collect();
        if cols.len() < 5 {
            continue;
        }
        let Ok(pid) = cols[0].trim().parse::<u32>() else {
            continue;
        };
        if cols[4].trim().eq_ignore_ascii_case("sea") {
            sea.insert(pid);
        }
    }
    sea
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_REGION: &str = r#"
strategic_region={
	id=100
	name="STRATEGICREGION_100"
	provinces={
		2800 2825 5388 8742 8766
	}
	weather={
		period={
			between={ 0.0 30.0 }
			temperature={ 5.0 30.0 }
			no_phenomenon=0.500
			rain_light=0.400
			snow=0.000
			arctic_water=0.000
			mud=1.000
			min_snow_level=0.000
		}
	}
}
"#;

    /// The parser pulls ONLY the top-level `id` and `provinces={}` integers; the weather
    /// block's `temperature={ 5.0 30.0 }` / `between={ 0.0 30.0 }` floats AND the `period`
    /// integers must NOT appear as provinces (Codex (d): the core fabrication trap).
    #[test]
    fn parse_region_extracts_only_provinces_block() {
        let (rid, provs) = parse_region_file(SAMPLE_REGION).expect("parses");
        assert_eq!(rid, 100, "region id from contents, not filename");
        assert_eq!(provs, vec![2800, 2825, 5388, 8742, 8766]);
        // none of the weather floats (5.0, 30.0, 0.0) leaked in as a province.
        assert!(
            !provs.contains(&5),
            "temperature 5.0 must not become province 5"
        );
        assert!(!provs.contains(&30), "30.0 must not become province 30");
        assert!(!provs.contains(&0), "0.0 must not become province 0");
    }

    /// `#` comments are stripped; a commented province id is not collected.
    #[test]
    fn parse_region_strips_comments() {
        let text = "strategic_region={\n id=7\n provinces={ 11 22 # 33 commented\n 44 }\n}";
        let (rid, provs) = parse_region_file(text).expect("parses");
        assert_eq!(rid, 7);
        assert_eq!(provs, vec![11, 22, 44], "commented 33 excluded");
    }

    /// A file with no `id` or no provinces yields None (never a fabricated 0-region).
    #[test]
    fn parse_region_rejects_incomplete() {
        assert!(parse_region_file("name=\"X\"\nprovinces={ 1 2 }").is_none());
        assert!(parse_region_file("strategic_region={ id=3 }").is_none());
    }

    /// definition.csv sea parse: type is column index 4; sea kept, land/lake excluded; the
    /// off-by-one (reading `coastal` at index 5) is guarded by the test's `coastal=true`
    /// land row that must NOT be classified as sea.
    #[test]
    fn parse_sea_uses_type_column_4() {
        let csv = "0;0;0;0;land;false;unknown;0\n\
                   1;230;81;119;lake;false;lakes;7\n\
                   4;0;0;232;sea;true;ocean;0\n\
                   5;0;2;240;sea;false;ocean;0\n\
                   2;0;0;55;land;true;forest;1\n"; // coastal=true LAND: must stay non-sea.
        let sea = parse_sea_provinces(csv);
        assert!(sea.contains(&4) && sea.contains(&5), "sea rows kept");
        assert!(!sea.contains(&0) && !sea.contains(&2), "land rows excluded");
        assert!(!sea.contains(&1), "lake excluded (frozen SEAS only)");
        // exactly the two sea provinces.
        assert_eq!(sea.len(), 2);
    }

    /// The frozen-seas join: a region whose phenomenon contains `arctic_water` freezes ONLY
    /// its SEA provinces; LAND provinces in the same region are never frozen; a region without
    /// arctic_water freezes nothing (Codex (b) + (c)).
    #[test]
    fn frozen_join_seas_only() {
        // region 50 = sea provinces {100,101} + a land province 102; region 51 = sea {200}.
        let mut p2r = HashMap::new();
        for (p, r) in [(100, 50), (101, 50), (102, 50), (200, 51)] {
            p2r.insert(p as u32, r as u32);
        }
        let mut r2p: HashMap<u32, Vec<u32>> = HashMap::new();
        r2p.insert(50, vec![100, 101, 102]);
        r2p.insert(51, vec![200]);
        let sea: HashSet<u32> = [100u32, 101, 200].into_iter().collect(); // 102 is LAND.
        let sr = StrategicRegions::from_parts(p2r, r2p, sea);

        // region 50 has arctic_water; region 51 does not.
        let weather_regions = serde_json::json!({
            "50": { "phenomenon": ["arctic_water"] },
            "51": { "phenomenon": ["snow"] }
        });
        let frozen = sr.frozen_sea_provinces(&weather_regions);
        assert!(
            frozen.contains(&100) && frozen.contains(&101),
            "sea of arctic region frozen"
        );
        assert!(!frozen.contains(&102), "LAND in arctic region NOT frozen");
        assert!(!frozen.contains(&200), "non-arctic region's sea NOT frozen");
        assert_eq!(frozen.len(), 2);
    }

    /// No arctic_water anywhere (or an empty/missing phenomenon) → no frozen provinces.
    #[test]
    fn frozen_join_no_arctic_is_empty() {
        let sr = StrategicRegions::from_parts(
            [(100u32, 50u32)].into_iter().collect(),
            [(50u32, vec![100u32])].into_iter().collect(),
            [100u32].into_iter().collect(),
        );
        // empty phenomenon, and a missing phenomenon, and a non-arctic one.
        let w = serde_json::json!({
            "50": { "phenomenon": [] },
            "99": { "temperature": -20.0 }
        });
        assert!(sr.frozen_sea_provinces(&w).is_empty());
    }

    /// CROSS-CHECK (Codex (d)): the strategicregions/*.txt parser must agree with the
    /// pipeline-precomputed `map_pipeline/assets/province_to_strategic_region.json`. That asset
    /// is built by `build_strategic_regions.py` from the MOD's strategicregions dir ONLY (the
    /// mod replace_path's `map/strategicregions`, so vanilla is not mixed in). The cross-check
    /// must therefore load the SAME effective set — the MOD's files — or it would compare a
    /// vanilla parse against a mod-built asset and report spurious mismatches (the bug this test
    /// previously exhibited: 277 false mismatches with `mod_root=None` → vanilla loaded).
    ///
    /// Gated: only runs when BOTH the precomputed asset AND the mod's strategicregions dir are
    /// present (so it is a no-op in a bare CI checkout, never a false failure). The mod dir is
    /// derived from `CARGO_MANIFEST_DIR` (…/replay_native/render → …/mod/test), overridable via
    /// `BATTLE_PLAN_MOD` for a different mod checkout.
    #[test]
    fn parser_matches_precomputed_asset_when_available() {
        // locate the precomputed asset relative to the crate (…/replay_native/render → repo).
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")); // …/replay_native/render
        let asset = crate_dir.join("../../map_pipeline/assets/province_to_strategic_region.json");
        // The mod root the asset was built from: BATTLE_PLAN_MOD if set, else the sibling
        // …/mod/test (…/replay_native/render → up 3 → mod, then /test).
        let mod_root: Option<std::path::PathBuf> = std::env::var("BATTLE_PLAN_MOD").or_else(|_| std::env::var("HOI4_MOD"))
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                let m = crate_dir.join("../../../test");
                m.join("map").join("strategicregions").is_dir().then_some(m)
            });
        let Some(mod_root) = mod_root else {
            eprintln!("[skip] cross-check: mod strategicregions dir not found");
            return;
        };
        if !asset.is_file() {
            eprintln!("[skip] cross-check: precomputed asset not present");
            return;
        }
        // Load using the MOD as the effective set (replace_path) so the parse matches the
        // mod-built asset; vanilla is irrelevant here (and not required to exist).
        let resolver = GameResolver::new(None, mod_root.to_str());
        let sr = match StrategicRegions::load(&resolver) {
            Ok(sr) => sr,
            Err(e) => {
                eprintln!("[skip] cross-check: strategicregions load failed: {e}");
                return;
            }
        };
        let txt = std::fs::read_to_string(&asset).expect("read precomputed asset");
        let expected: HashMap<String, serde_json::Value> =
            serde_json::from_str(&txt).expect("parse precomputed asset");

        // every province in the precomputed map must resolve to the SAME region via our parser.
        let mut checked = 0usize;
        let mut mismatches = 0usize;
        for (pid_s, rid_v) in &expected {
            let (Ok(pid), Some(rid)) = (pid_s.parse::<u32>(), rid_v.as_u64()) else {
                continue;
            };
            match sr.region_of_province(pid) {
                Some(got) if got as u64 == rid => checked += 1,
                Some(got) => {
                    mismatches += 1;
                    if mismatches <= 5 {
                        eprintln!("mismatch: province {pid} parser→{got} asset→{rid}");
                    }
                }
                None => {
                    mismatches += 1;
                    if mismatches <= 5 {
                        eprintln!("missing: province {pid} not in parser (asset→{rid})");
                    }
                }
            }
        }
        assert_eq!(
            mismatches, 0,
            "{mismatches} province→region mismatches vs precomputed asset"
        );
        assert!(
            checked > 1000,
            "expected to cross-check many provinces, got {checked}"
        );

        // A known ARCTIC SEA region (46 = "Barents Sea") with a measured `arctic_water`
        // phenomenon must freeze a NON-EMPTY set, and EVERY frozen province must be sea (the
        // sea-only filter), proving the real-data join (not just the synthetic unit test).
        let barents = sr.frozen_sea_provinces(&serde_json::json!({
            "46": { "phenomenon": ["arctic_water"] }
        }));
        eprintln!(
            "[info] region 46 (Barents Sea) arctic_water → {} sea provinces frozen",
            barents.len()
        );
        assert!(
            !barents.is_empty(),
            "Barents Sea arctic_water should freeze its sea provinces"
        );
        assert!(
            barents.iter().all(|&p| sr.is_sea(p)),
            "every frozen province must be sea"
        );
        // a LAND-heavy region (a region with land provinces) must NOT freeze its land: pick
        // any province known to be land from the region join and confirm it is never frozen.
        assert!(
            barents
                .iter()
                .all(|&p| sr.region_of_province(p) == Some(46)),
            "frozen provinces must belong to the arctic region"
        );
    }

    // ---- temp-dir loader tests (run in bare CI; no game install needed) ----

    /// Write a minimal strategicregions file under `dir`.
    fn write_region(dir: &std::path::Path, fname: &str, id: u32, provs: &[u32]) {
        let prov_list = provs
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let text = format!(
            "strategic_region={{\n id={id}\n name=\"R{id}\"\n provinces={{ {prov_list} }}\n}}\n"
        );
        std::fs::write(dir.join(fname), text).unwrap();
    }

    /// A unique temp dir for a test (avoids colliding parallel runs).
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "sr_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// REPLACE_PATH semantics (the production bug Codex confirmed): when the MOD ships a
    /// strategicregions dir, vanilla's is NOT loaded at all. A vanilla-only region file (a
    /// filename absent from the mod) must NOT contribute provinces, and a province the mod
    /// assigns differently than vanilla must take the MOD's region — never vanilla's.
    #[test]
    fn load_mod_replaces_vanilla_fully() {
        let root = temp_dir("replace");
        let mod_dir = root.join("mod/map/strategicregions");
        let van_dir = root.join("van/map/strategicregions");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::create_dir_all(&van_dir).unwrap();

        // Mod: region 1 owns province 100. Region 2 owns 200.
        write_region(&mod_dir, "1.txt", 1, &[100]);
        write_region(&mod_dir, "2.txt", 2, &[200]);
        // Vanilla DISAGREES: it puts province 100 in region 1 too BUT also has a vanilla-only
        // region 9 owning province 999, and reassigns 200 to region 1.
        write_region(&van_dir, "1.txt", 1, &[100, 200]);
        write_region(&van_dir, "9.txt", 9, &[999]);

        let resolver = GameResolver::new(
            Some(root.join("van").to_str().unwrap()),
            Some(root.join("mod").to_str().unwrap()),
        );
        let sr = StrategicRegions::load(&resolver).expect("load");
        // province 200 is region 2 per the MOD (vanilla's region-1 claim is NOT loaded).
        assert_eq!(sr.region_of_province(200), Some(2), "mod replaces vanilla");
        // the vanilla-only province 999 is absent entirely (vanilla dir not loaded).
        assert_eq!(
            sr.region_of_province(999),
            None,
            "vanilla-only region not loaded"
        );
        // exactly the mod's two regions / two provinces.
        assert_eq!(sr.region_count(), 2);
        assert_eq!(sr.province_count(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Falls back to vanilla ONLY when no mod strategicregions dir exists.
    #[test]
    fn load_falls_back_to_vanilla_when_no_mod_dir() {
        let root = temp_dir("fallback");
        let van_dir = root.join("van/map/strategicregions");
        std::fs::create_dir_all(&van_dir).unwrap();
        write_region(&van_dir, "9.txt", 9, &[999]);

        // mod_root points somewhere WITHOUT a strategicregions dir.
        let resolver = GameResolver::new(
            Some(root.join("van").to_str().unwrap()),
            Some(root.join("nomod").to_str().unwrap()),
        );
        let sr = StrategicRegions::load(&resolver).expect("load falls back to vanilla");
        assert_eq!(sr.region_of_province(999), Some(9));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// DETERMINISM: a province claimed by two DISTINCT regions resolves to the SAME region
    /// every run (sorted-filename, last-writer-wins — matching the Python asset builder), not
    /// whatever order the OS returned from `read_dir`. (The old first-writer + unordered
    /// `read_dir` made this non-deterministic — a NO-FABRICATION hazard.)
    #[test]
    fn load_collision_is_deterministic_last_sorted_wins() {
        let root = temp_dir("collide");
        let mod_dir = root.join("mod/map/strategicregions");
        std::fs::create_dir_all(&mod_dir).unwrap();
        // Province 500 is (wrongly) claimed by BOTH region 10 (file "a") and region 20
        // (file "z"). Sorted order is a<z, so the LAST sorted file (z → region 20) wins.
        write_region(&mod_dir, "a_region10.txt", 10, &[500, 501]);
        write_region(&mod_dir, "z_region20.txt", 20, &[500, 502]);

        let resolver = GameResolver::new(
            Some("/nonexistent_vanilla"),
            Some(root.join("mod").to_str().unwrap()),
        );
        // Run twice; the assignment must be identical and equal to the last-sorted writer (20).
        let a = StrategicRegions::load(&resolver).unwrap();
        let b = StrategicRegions::load(&resolver).unwrap();
        assert_eq!(
            a.region_of_province(500),
            Some(20),
            "last sorted file wins the collision"
        );
        assert_eq!(
            a.region_of_province(500),
            b.region_of_province(500),
            "deterministic"
        );
        // the non-colliding provinces keep their own region.
        assert_eq!(a.region_of_province(501), Some(10));
        assert_eq!(a.region_of_province(502), Some(20));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A duplicate REGION ID across two files: the FIRST sorted file wins and the second's
    /// provinces are NOT merged (matches the Python builder's first-by-name rule).
    #[test]
    fn load_duplicate_region_id_keeps_first_sorted() {
        let root = temp_dir("dupe");
        let mod_dir = root.join("mod/map/strategicregions");
        std::fs::create_dir_all(&mod_dir).unwrap();
        write_region(&mod_dir, "a.txt", 7, &[10, 11]);
        write_region(&mod_dir, "b.txt", 7, &[20, 21]); // same region id 7

        let resolver = GameResolver::new(
            Some("/nonexistent_vanilla"),
            Some(root.join("mod").to_str().unwrap()),
        );
        let sr = StrategicRegions::load(&resolver).unwrap();
        // first file (a.txt) won: its provinces are present, b.txt's are dropped.
        assert_eq!(sr.region_of_province(10), Some(7));
        assert_eq!(
            sr.region_of_province(20),
            None,
            "duplicate region id's later file dropped"
        );
        assert_eq!(sr.provinces_in_region(7), &[10, 11]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
