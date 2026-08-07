//! Lane EE stage-1 instrument: a per-section MUTATION DENSITY census over every
//! top-level member of the world document.
//!
//! # What it decides
//!
//! The v3 prototype's upper band is `v3_upper / coverage_by_bytes`: it charges
//! the uncovered sections the covered set's cost per byte. That extrapolation is
//! only conservative if the uncovered sections are no denser than the covered
//! ones. Where they are denser, the band understates, and converting the densest
//! first is what collapses the uncertainty soonest. The order therefore has to
//! be MEASURED, not guessed -- which is what this does.
//!
//! For every top-level member, at each requested grain depth, it decomposes the
//! member the way [`fold_v3`] would and counts how many grains were ADDED,
//! REMOVED and CHANGED against the previous hour, plus the canonical bytes of
//! the grains that had to be written. Those are exactly the quantities the v3
//! API is charged for per hour: one hash and one install per changed grain, one
//! journal record per mutation, and `grain_bytes_written` bytes copied.
//!
//! # Why it is a separate binary
//!
//! It shares no code path with `dq_v3_prototype`'s timed regions and cannot
//! perturb them. Nothing here is timed at all: a census that had to be fast
//! would have to approximate, and an approximate density would pick the wrong
//! conversion order.
//!
//! # Keyed arrays
//!
//! An array-spelled member decomposed by ORDINAL re-emits every element after an
//! insertion, so its ordinal density is an artefact of the addressing and not a
//! fact about the section. Where a member is an array of objects carrying a
//! string identity that is unique and already ascending -- the property
//! `divisions` has and which cut its delta to 0.41-0.61x -- this also reports the
//! KEYED density beside the ordinal one. The gap between the two is the value of
//! a keyed grain spec, measured rather than assumed.

use std::collections::BTreeMap;
use std::path::Path as FsPath;

use fold_v3::{decompose_ref, Path, Shape};
use snapshot::canon_value::{key_cmp, CanonValue};
use snapshot::sha256::Sha256;

use crate::dq_v3_prototype::COVERED_SECTIONS;
use crate::{HourCursor, IndexSourceDescriptor, StreamIdentityMode, WorldHour};

/// One grain's identity for census purposes: what it hashes to and how many
/// bytes it would cost to write. The BYTES are deliberately not retained -- a
/// census that held a copy of the document at three depths would be measuring
/// the machine's swap behaviour.
type GrainMap = BTreeMap<Path, ([u8; 32], u32)>;

#[derive(Default, Clone)]
struct DepthStat {
    grains_last: u64,
    grain_bytes_last: u64,
    added: u64,
    removed: u64,
    changed: u64,
    /// Canonical bytes of every grain that had to be WRITTEN (added or changed).
    /// A removal writes only its path, so it is not counted here.
    written_bytes: u64,
    /// Hours folded in: every hour but the member's first, matching the
    /// prototype's own convention that a first hour is an initialization and not
    /// a comparison.
    hours_measured: u64,
    /// Hours where the member's JSON KIND changed, so every grain was replaced.
    /// Counted separately because a kind flip is a fact about the fold, not a
    /// mutation density.
    kind_flip_hours: u64,
    /// Root-kind journal records: one per hour the section's ROOT went between
    /// scalar, object and array. It is a mutation the journal carries and the
    /// grain counts cannot see, so it is counted here and folded into
    /// [`Self::mutations`].
    ///
    /// A JSON-kind change is not always a root-kind change: `null` becoming a
    /// number is two scalars, and `fold_v3` journals no root record for it.
    root_changes: u64,
}

impl DepthStat {
    fn mutations(&self) -> u64 {
        self.added + self.removed + self.changed + self.root_changes
    }
}

struct MemberCensus {
    name: String,
    covered: bool,
    hours_present: u64,
    bytes_last: u64,
    kind_last: &'static str,
    kind_flips: u64,
    depths: Vec<usize>,
    prev: Vec<Option<GrainMap>>,
    stats: Vec<DepthStat>,
    /// Every member name that is still a viable element identity, in canonical
    /// order, or `None` before the member has been seen as a non-empty array.
    ///
    /// Kept as a SET rather than a single choice: an identity that holds on hour
    /// one can fail on hour forty while another holds throughout, and a census
    /// that had committed to the first would report the member as having no
    /// keyed decomposition at all.
    key_candidates: Option<Vec<String>>,
    /// The candidate currently being censused -- always the first surviving one.
    key_member: Option<String>,
    /// How many times the surviving candidate changed. Published so a keyed row
    /// measured over fewer hours than the member has is visible as such.
    keyed_restarts: u64,
    /// What the KEYED root was last hour: `array` while the member is spelled as
    /// one, `scalar` on an hour it is not. A `KeyedArray` section handed a
    /// non-array becomes a scalar root, and that is a journal record.
    keyed_root_last: &'static str,
    keyed_prev: Vec<Option<GrainMap>>,
    keyed_stats: Vec<DepthStat>,
    /// Why the keyed variant stopped, if it did. A member whose identities stop
    /// ascending cannot use a keyed grain, and saying so is the point.
    keyed_refused: Option<String>,
    /// Set once the member has been seen as an array, so a member that is never
    /// an array does not report an empty keyed row.
    ever_array: bool,
}

impl MemberCensus {
    fn new(name: &str, depths: &[usize]) -> Self {
        Self {
            name: name.to_string(),
            covered: COVERED_SECTIONS.iter().any(|(c, _)| *c == name),
            hours_present: 0,
            bytes_last: 0,
            kind_last: "absent",
            kind_flips: 0,
            depths: depths.to_vec(),
            prev: vec![None; depths.len()],
            stats: vec![DepthStat::default(); depths.len()],
            key_candidates: None,
            key_member: None,
            keyed_restarts: 0,
            keyed_root_last: "absent",
            keyed_prev: vec![None; depths.len()],
            keyed_stats: vec![DepthStat::default(); depths.len()],
            keyed_refused: None,
            ever_array: false,
        }
    }
}

fn kind_name(value: &CanonValue) -> &'static str {
    match value {
        CanonValue::Object(_) => "object",
        CanonValue::Array(_) => "array",
        CanonValue::Null => "null",
        CanonValue::Bool(_) => "bool",
        CanonValue::Number(_) => "number",
        CanonValue::String(_) => "string",
    }
}

/// The ROOT KIND `fold_v3` would hold, which is coarser than the JSON kind:
/// every scalar spelling is one root kind, so `null` becoming a number moves the
/// JSON kind and not the root.
fn root_kind_name(kind: &str) -> &'static str {
    match kind {
        "object" => "object",
        "array" => "array",
        _ => "scalar",
    }
}

/// The shape [`decompose_ref`] would be given for this value, or `None` for a
/// value that has no container root and is therefore one whole-section grain.
fn shape_of(value: &CanonValue) -> Option<Shape> {
    match value {
        CanonValue::Object(_) => Some(Shape::Object),
        CanonValue::Array(_) => Some(Shape::Array),
        _ => None,
    }
}

/// Decompose into a grain map, hashing each grain.
///
/// An empty container decomposes to nothing, which is correct: its root kind is
/// the whole of its content.
fn grain_map(value: &CanonValue, depth: usize, shape: Option<&Shape>) -> Result<GrainMap, String> {
    let mut map = GrainMap::new();
    match shape {
        None => {
            let bytes = value.to_json_bytes();
            map.insert(Path::new(), (Sha256::digest(&bytes), bytes.len() as u32));
        }
        Some(shape) => {
            for (path, grain) in decompose_ref(value, depth, shape).map_err(|e| e.to_string())? {
                let bytes = grain.to_json_bytes();
                map.insert(path, (Sha256::digest(&bytes), bytes.len() as u32));
            }
        }
    }
    Ok(map)
}

/// Fold one hour's grain map against the previous one.
///
/// `previous` is `None` on the member's first hour, which is recorded as state
/// and contributes nothing: comparing "everything arrived" against a v2 lane
/// that pays no delta on its first hour would book an initialization as a
/// density.
fn fold_hour(
    stat: &mut DepthStat,
    previous: Option<&GrainMap>,
    current: &GrainMap,
    flipped: bool,
    root_changed: bool,
) {
    stat.grains_last = current.len() as u64;
    stat.grain_bytes_last = current.values().map(|(_, len)| u64::from(*len)).sum();
    let Some(previous) = previous else { return };
    stat.hours_measured += 1;
    if flipped {
        stat.kind_flip_hours += 1;
    }
    if root_changed {
        stat.root_changes += 1;
    }
    for (path, (digest, len)) in current {
        match previous.get(path) {
            Some((before, _)) if before == digest => {}
            Some(_) => {
                stat.changed += 1;
                stat.written_bytes += u64::from(*len);
            }
            None => {
                stat.added += 1;
                stat.written_bytes += u64::from(*len);
            }
        }
    }
    for path in previous.keys() {
        if !current.contains_key(path) {
            stat.removed += 1;
        }
    }
}

/// Whether `candidate` names a STRING member of every element of `items` whose
/// values are unique and already in strictly ascending canonical order.
///
/// That is the exact precondition [`Shape::KeyedArray`] imposes, so a name that
/// passes here is one a keyed grain spec could actually use.
fn key_is_viable(candidate: &str, items: &[CanonValue]) -> bool {
    let mut previous: Option<&String> = None;
    for item in items {
        let CanonValue::Object(members) = item else {
            return false;
        };
        let Some(body) = members.iter().find_map(|(name, value)| match value {
            CanonValue::String(body) if name == candidate => Some(body),
            _ => None,
        }) else {
            return false;
        };
        if let Some(previous) = previous {
            if key_cmp(previous, body) != std::cmp::Ordering::Less {
                return false;
            }
        }
        previous = Some(body);
    }
    true
}

/// EVERY member name that could address these elements, in canonical order.
///
/// All of them, not the first: an identity that holds on one hour can fail on
/// another while a different one holds throughout, and a member ranked on its
/// ORDINAL density because the first candidate failed would be ranked on an
/// artefact of the addressing rather than on anything about the member.
fn viable_key_members(items: &[CanonValue]) -> Vec<String> {
    let Some(CanonValue::Object(first)) = items.first() else {
        return Vec::new();
    };
    first
        .iter()
        .filter(|(_, value)| matches!(value, CanonValue::String(_)))
        .map(|(name, _)| name.clone())
        .filter(|candidate| key_is_viable(candidate, items))
        .collect()
}

fn json_string(body: &str) -> String {
    format!("\"{}\"", body.replace('\\', "/").replace('"', "'"))
}

fn depth_rows(depths: &[usize], stats: &[DepthStat]) -> String {
    let rows: Vec<String> = depths
        .iter()
        .zip(stats)
        .map(|(depth, stat)| {
            let hours = if stat.hours_measured > 0 {
                stat.hours_measured as f64
            } else {
                1.0
            };
            format!(
                "{{\"depth\":{depth},\"grains_last\":{},\"grain_bytes_last\":{},\
                 \"hours_measured\":{},\"kind_flip_hours\":{},\
                 \"root_changes\":{},\
                 \"mutations_per_hour\":{:.2},\"added_per_hour\":{:.2},\
                 \"removed_per_hour\":{:.2},\"changed_per_hour\":{:.2},\
                 \"written_bytes_per_hour\":{:.1},\"mean_grain_bytes\":{:.1}}}",
                stat.grains_last,
                stat.grain_bytes_last,
                stat.hours_measured,
                stat.kind_flip_hours,
                stat.root_changes,
                stat.mutations() as f64 / hours,
                stat.added as f64 / hours,
                stat.removed as f64 / hours,
                stat.changed as f64 / hours,
                stat.written_bytes as f64 / hours,
                if stat.grains_last > 0 {
                    stat.grain_bytes_last as f64 / stat.grains_last as f64
                } else {
                    0.0
                },
            )
        })
        .collect();
    rows.join(",")
}

pub fn run(
    raw_h4cr_path: &FsPath,
    game_id: &str,
    identity_mode: StreamIdentityMode,
    expected_source_len: u64,
    depth_spec: &str,
    max_hours: u64,
) -> Result<String, String> {
    let depths: Vec<usize> = depth_spec
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<usize>()
                .map_err(|_| format!("depth spec {depth_spec} is not a comma-separated depth list"))
        })
        .collect::<Result<_, _>>()?;
    if depths.is_empty() || depths.iter().any(|d| *d == 0) {
        return Err("grain depth 0 would make the whole section one grain".to_string());
    }

    let raw_h4cr_path = std::fs::canonicalize(raw_h4cr_path).map_err(|e| e.to_string())?;
    let source_file = std::fs::File::open(&raw_h4cr_path).map_err(|e| e.to_string())?;
    let mut cursor = HourCursor::from_reader_with_source(
        Box::new(source_file),
        expected_source_len,
        game_id.to_string(),
        identity_mode,
        Some(IndexSourceDescriptor {
            kind: "stream_file".to_string(),
            canonical_path: raw_h4cr_path.to_string_lossy().into_owned(),
        }),
    );

    // Insertion order is not stable across hours -- a member can be absent on
    // one hour and present on the next -- so members are held by name.
    let mut members: BTreeMap<String, MemberCensus> = BTreeMap::new();
    let mut hours = 0u64;
    let mut document_member_bytes_last = 0u64;
    let mut coverage_hour = 0i64;

    while hours < max_hours {
        let Some(hour) = cursor.next_hour() else { break };
        let hour: WorldHour = hour.map_err(|e| format!("decode: {e:?}"))?;
        hours += 1;
        let document = CanonValue::parse(&hour.snapshot)
            .map_err(|e| format!("hour {} is not canonical: {e}", hour.hour_index))?;
        let CanonValue::Object(top) = &document else {
            return Err(format!("hour {} is not an object", hour.hour_index));
        };
        coverage_hour = hour.hour_index;
        document_member_bytes_last = 0;

        for (name, value) in top {
            let bytes = value.to_json_bytes();
            document_member_bytes_last += bytes.len() as u64;
            let entry = members
                .entry(name.clone())
                .or_insert_with(|| MemberCensus::new(name, &depths));
            entry.hours_present += 1;
            entry.bytes_last = bytes.len() as u64;
            let kind = kind_name(value);
            // A member absent last hour is not a flip: it has no previous kind
            // to have moved from, and the depth stats skip its first hour too.
            let flipped = entry.kind_last != "absent" && entry.kind_last != kind;
            // Whether the SECTION ROOT moved between scalar, object and array --
            // which is what `fold_v3` journals a root record for. `null`
            // becoming a number is a JSON-kind flip and not a root change.
            let root_changed =
                entry.kind_last != "absent" && root_kind_name(entry.kind_last) != root_kind_name(kind);
            if flipped {
                entry.kind_flips += 1;
            }
            entry.kind_last = kind;
            let shape = shape_of(value);

            for (index, depth) in depths.iter().enumerate() {
                let current = grain_map(value, *depth, shape.as_ref()).map_err(|e| {
                    format!("{name} at depth {depth} on hour {}: {e}", hour.hour_index)
                })?;
                // A kind flip is folded against the REAL previous map, not
                // against an empty one. The two address spaces cannot overlap --
                // an object's grains are name-addressed, an array's are
                // ordinal-addressed, and a scalar's is the empty path -- so
                // comparing them naturally counts every old grain as REMOVED and
                // every new one as ADDED, which is what the journal would carry.
                // Substituting an empty map counted the additions and silently
                // dropped every removal.
                fold_hour(
                    &mut entry.stats[index],
                    entry.prev[index].as_ref(),
                    &current,
                    flipped,
                    root_changed,
                );
                entry.prev[index] = Some(current);
            }

            // ---- the keyed variant ------------------------------------------
            //
            // CANDIDATE elimination needs elements to read, so it only runs on a
            // non-empty array. The keyed CENSUS runs on every hour the member is
            // present once an identity exists, which is not the same condition:
            // an empty array removes every grain, and a non-array turns a
            // `KeyedArray` section into a scalar root. Both are records the
            // journal would carry, and skipping them compared the hours on
            // either side of them to each other -- the same defect the ordinary
            // path had across an absence.
            if let CanonValue::Array(items) = value {
                entry.ever_array = true;
                if entry.keyed_refused.is_none() && !items.is_empty() {
                    // EVERY viable identity is kept, not just the first.
                    // Committing to one candidate and refusing the member when
                    // it later fails would report the ORDINAL density for a
                    // member that has a perfectly good keyed decomposition under
                    // a different name -- and the ordinal density of an array is
                    // an artefact of the addressing, so that would rank the
                    // member on a number about nothing.
                    match &mut entry.key_candidates {
                        None => entry.key_candidates = Some(viable_key_members(items)),
                        Some(candidates) => {
                            candidates.retain(|candidate| key_is_viable(candidate, items))
                        }
                    }
                }
            }
            if entry.keyed_refused.is_none() && entry.key_candidates.is_some() {
                let surviving = entry
                    .key_candidates
                    .as_ref()
                    .and_then(|candidates| candidates.first())
                    .cloned();
                match surviving {
                    None => {
                        entry.keyed_refused = Some(format!(
                            "hour {}: no member is a string identity that is unique and \
                             already ascending across every element",
                            hour.hour_index
                        ));
                        entry.key_member = None;
                        entry.keyed_root_last = "absent";
                        entry.keyed_prev.iter_mut().for_each(|slot| *slot = None);
                        entry
                            .keyed_stats
                            .iter_mut()
                            .for_each(|slot| *slot = DepthStat::default());
                    }
                    Some(key) => {
                        // Whatever key is finally reported, the numbers reported
                        // beside it must be that key's. Switching therefore
                        // restarts its census rather than continuing another
                        // key's totals under a new name.
                        if entry.key_member.as_deref() != Some(key.as_str()) {
                            if entry.key_member.is_some() {
                                entry.keyed_restarts += 1;
                            }
                            entry.key_member = Some(key.clone());
                            entry.keyed_root_last = "absent";
                            entry.keyed_prev.iter_mut().for_each(|slot| *slot = None);
                            entry
                                .keyed_stats
                                .iter_mut()
                                .for_each(|slot| *slot = DepthStat::default());
                        }
                        // A `KeyedArray` section handed something that is not an
                        // array becomes a SCALAR root holding the whole value,
                        // which is what `assign_section` does with it. Censusing
                        // it as one grain at the empty path is that hour, not an
                        // absence of one.
                        let is_array = matches!(value, CanonValue::Array(_));
                        let shape = is_array.then(|| Shape::KeyedArray { key_member: key });
                        let keyed_root = if is_array { "array" } else { "scalar" };
                        let keyed_root_changed =
                            entry.keyed_root_last != "absent" && entry.keyed_root_last != keyed_root;
                        entry.keyed_root_last = keyed_root;
                        for (index, depth) in depths.iter().enumerate() {
                            // The surviving key was validated against THIS
                            // hour's elements above -- `key_is_viable` tests the
                            // same object / string / strictly-ascending
                            // conditions `Shape::KeyedArray` imposes -- so a
                            // failure here is a disagreement between the two and
                            // must not be swallowed.
                            let current =
                                grain_map(value, *depth, shape.as_ref()).map_err(|e| {
                                    format!(
                                        "{name} keyed at depth {depth} on hour {}: {e}",
                                        hour.hour_index
                                    )
                                })?;
                            fold_hour(
                                &mut entry.keyed_stats[index],
                                entry.keyed_prev[index].as_ref(),
                                &current,
                                !is_array,
                                keyed_root_changed,
                            );
                            entry.keyed_prev[index] = Some(current);
                        }
                    }
                }
            }
        }

        // ---- members the document did NOT carry this hour --------------------
        //
        // Only present members are visited above, so an absent one would keep
        // its previous grain maps and be compared against them when it came
        // back -- collapsing however many hours the gap lasted into one
        // transition, and dividing that transition by a denominator that never
        // counted the missing hours. Its state is restarted instead, which is
        // what "the member's first hour is an initialization" already means.
        for (name, entry) in members.iter_mut() {
            if top.iter().any(|(present, _)| present == name) {
                continue;
            }
            entry.kind_last = "absent";
            entry.keyed_root_last = "absent";
            entry.prev.iter_mut().for_each(|slot| *slot = None);
            entry.keyed_prev.iter_mut().for_each(|slot| *slot = None);
        }
    }

    if hours == 0 {
        return Err("stream emitted no hours".to_string());
    }
    if document_member_bytes_last == 0 {
        return Err("the last hour's document has no members".to_string());
    }

    let rows: Vec<String> = members
        .values()
        .map(|member| {
            let keyed = if member.ever_array && member.key_member.is_some() {
                format!(
                    ",\"keyed_by\":{},\"keyed_alternatives\":{},\"keyed_restarts\":{},\
                     \"keyed_depths\":[{}]",
                    json_string(member.key_member.as_deref().unwrap_or("")),
                    member.key_candidates.as_ref().map_or(0, Vec::len),
                    member.keyed_restarts,
                    depth_rows(&member.depths, &member.keyed_stats)
                )
            } else if member.ever_array {
                // A member that was an array but never a NON-EMPTY one has no
                // elements to read an identity from. That is a different fact
                // from an identity having been looked for and not found, and
                // reporting both as "unknown" said neither.
                format!(
                    ",\"keyed_refused\":{}",
                    json_string(member.keyed_refused.as_deref().unwrap_or(
                        "the member was never a non-empty array, so it carries no element \
                         identity to address by"
                    ))
                )
            } else {
                String::new()
            };
            format!(
                "{{\"section\":{},\"covered\":{},\"hours_present\":{},\
                 \"bytes_last\":{},\"byte_share\":{:.6},\"kind_last\":{},\
                 \"kind_flips\":{},\"depths\":[{}]{keyed}}}",
                json_string(&member.name),
                member.covered,
                member.hours_present,
                member.bytes_last,
                member.bytes_last as f64 / document_member_bytes_last as f64,
                json_string(member.kind_last),
                member.kind_flips,
                depth_rows(&member.depths, &member.stats),
            )
        })
        .collect();

    Ok(format!(
        "{{\"schema\":\"dq-v3-census-v1\",\"hours\":{hours},\
         \"coverage_hour\":{coverage_hour},\
         \"document_member_bytes_last\":{document_member_bytes_last},\
         \"depth_spec\":{},\"sections\":[{}]}}",
        json_string(depth_spec),
        rows.join(","),
    ))
}
