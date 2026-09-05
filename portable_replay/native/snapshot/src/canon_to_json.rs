use std::cmp::Ordering;
use std::sync::Arc;

use serde_json::{Map, Number, Value};

use crate::canon_value::{decode_json_string_body, ArrayNode, CanonValue, ObjectNode};

pub const SERDE_JSON_RECURSION_LIMIT: usize = 127;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonToValueError {
    message: String,
}

impl CanonToValueError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CanonToValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CanonToValueError {}

pub fn to_value(value: &CanonValue) -> Result<Value, CanonToValueError> {
    convert(value, 0)
}

fn convert(value: &CanonValue, depth: usize) -> Result<Value, CanonToValueError> {
    match value {
        CanonValue::Null => Ok(Value::Null),
        CanonValue::Bool(flag) => Ok(Value::Bool(*flag)),
        CanonValue::Number(token) => number_from_token(token).map(Value::Number),
        CanonValue::String(body) => {
            decode_json_string_body(body)
                .map(Value::String)
                .ok_or_else(|| {
                    CanonToValueError::new(
                        "world JSON string body does not decode to text (lone surrogate escape)",
                    )
                })
        }
        CanonValue::Array(items) => {
            let depth = enter(depth)?;
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                out.push(convert(item, depth)?);
            }
            Ok(Value::Array(out))
        }
        CanonValue::Object(members) => {
            let depth = enter(depth)?;
            let mut out = Map::new();
            for (name, member) in members.iter() {
                let key = decode_json_string_body(name).ok_or_else(|| {
                    CanonToValueError::new("world JSON member name does not decode to text")
                })?;
                let converted = convert(member, depth)?;
                if out.insert(key, converted).is_some() {
                    return Err(CanonToValueError::new(
                        "world JSON object has a duplicate member name",
                    ));
                }
            }
            Ok(Value::Object(out))
        }
    }
}

fn enter(depth: usize) -> Result<usize, CanonToValueError> {
    let depth = depth + 1;
    if depth > SERDE_JSON_RECURSION_LIMIT {
        return Err(CanonToValueError::new(
            "world JSON exceeds the JSON reader's nesting limit",
        ));
    }
    Ok(depth)
}

/// The SHARED projected world — the same shape as `serde_json::Value`, except
/// that containers hold their children through `Arc`, so two hours' projections
/// can share every unchanged subtree by pointer. `Value` itself cannot do this
/// at any cost: `Value::Array(Vec<Value>)` and `Value::Object(Map<..>)` OWN
/// their children, so building a parent requires owning the children, which is
/// the 220 ms-per-hour wall batch A measured from the other side.
///
/// Everything in here is already decoded and validated: keys are decoded text,
/// strings are decoded text, numbers are `serde_json::Number`. Materialising it
/// into a `Value` ([`projected_to_value`]) therefore cannot fail.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectedValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Arc<Vec<ProjectedValue>>),
    Object(Arc<Vec<(String, ProjectedValue)>>),
}

/// What one container node knows about its own projection, stored in the
/// node's one-time cell ([`crate::canon_value::ArrayNode`]).
///
/// DEPTH IS CONTEXTUAL; THE CELL IS INTRINSIC. `to_value` refuses past
/// [`SERDE_JSON_RECURSION_LIMIT`] containers, so the same `[[]]` fails at depth
/// 126 and succeeds at depth 0 — a depth refusal must therefore NEVER be
/// stored. The cell stores only what is true of the node wherever it sits: its
/// projection built with no absolute-depth limit applied, its subtree HEIGHT so
/// the limit can be applied at the use site, or a NODE-LOCAL defect (lone
/// surrogate, invalid number token, undecodable or duplicate member name),
/// which is permanent at every depth.
#[derive(Debug, Clone)]
pub enum ProjectionMemo {
    Projected {
        value: ProjectedValue,
        /// The maximum number of container nodes on any path from this node
        /// through its deepest descendant, INCLUSIVE — scalars contribute
        /// zero, so `[]` has height 1. Placement then refuses exactly when
        /// `containers_above + height` exceeds the limit, which is the same
        /// set of documents the depth-first converter refuses.
        height: usize,
    },
    Defect(String),
}

/// Project one value, memoised on container-node identity: a node that is
/// pointer-identical to one projected before — the unchanged subtrees of the
/// previous hour, after batch A's path copying — is reused by pointer and
/// never visited. The absolute-depth limit is applied here, at the use site,
/// against the memoised subtree height; nothing depth-contextual is stored.
pub fn project(value: &CanonValue) -> Result<ProjectedValue, CanonToValueError> {
    project_at(value, 0)
}

/// [`project`] for a value sitting below `containers_above` enclosing
/// containers — the use-site depth check the memo design moves out of the
/// traversal. Reuse is sound iff `containers_above + height` stays within
/// [`SERDE_JSON_RECURSION_LIMIT`].
pub fn project_at(
    value: &CanonValue,
    containers_above: usize,
) -> Result<ProjectedValue, CanonToValueError> {
    match project_intrinsic(value, SERDE_JSON_RECURSION_LIMIT) {
        Err(Halt::Defect(defect)) => Err(CanonToValueError::new(defect)),
        // Running out of budget AT THE ENTRY NODE means this value's height
        // exceeds the limit, which refuses at every absolute depth — the same
        // outcome the depth-first converter reached by counting on the way
        // down. See [`Halt`].
        Err(Halt::TooDeep) => Err(CanonToValueError::new(
            "world JSON exceeds the JSON reader's nesting limit",
        )),
        Ok((projected, height)) => {
            let deepest = containers_above.checked_add(height).ok_or_else(|| {
                CanonToValueError::new("world JSON exceeds the JSON reader's nesting limit")
            })?;
            if deepest > SERDE_JSON_RECURSION_LIMIT {
                return Err(CanonToValueError::new(
                    "world JSON exceeds the JSON reader's nesting limit",
                ));
            }
            Ok(projected)
        }
    }
}

/// Why a build stopped, and — decisively — whether the reason may be CACHED.
///
/// The memo may only ever hold facts that are true of the node WHEREVER it
/// sits. A node-local defect is such a fact. "I ran out of recursion budget
/// here" is not: it depends on how far below the entry node this node happened
/// to be, so caching it would let one placement's refusal be served to another.
enum Halt {
    /// Node-local and permanent: lone surrogate, invalid number token,
    /// undecodable or duplicate member name. Cacheable.
    Defect(String),
    /// The build's recursion budget ran out below this point. NEVER cached,
    /// and never stored in any cell — it only ever propagates to the caller.
    TooDeep,
}

/// The build ran out of budget. Distinct from [`Halt`] so a builder cannot
/// accidentally turn it into a cacheable `ProjectionMemo`.
struct TooDeep;

/// The node's intrinsic outcome and height, from its cell when filled, built
/// and filled otherwise. Scalars are converted inline: they are leaves whose
/// conversion is paid only when an enclosing container is (re)built, and a
/// scalar's defect is returned directly rather than cached anywhere.
///
/// WHY THE BUILD CARRIES A BUDGET. The cell is filled by RECURSION over the
/// subtree, and the depth check happens at the use site AFTERWARDS. The old
/// depth-first converter refused on the way DOWN, at container 128, so it never
/// recursed further. Without a budget here, a value nested deeper than the
/// stack can hold — the canonical decoder caps at
/// [`crate::canon_value::MAX_RECURSION_DEPTH`], but `CanonValue::array` is a
/// plain constructor and caps nothing — would exhaust the thread stack and
/// CRASH where the old route returned a refusal (Codex batch-B review, round 1,
/// finding 1).
///
/// The budget is [`SERDE_JSON_RECURSION_LIMIT`] measured from the ENTRY node,
/// which is exactly the reach that can matter: a node whose height exceeds the
/// limit cannot be placed at any absolute depth, since `containers_above` is
/// never negative. So exhausting the budget at the entry is a true refusal,
/// while exhausting it deeper is contextual and is why [`Halt::TooDeep`] is
/// never cached.
fn project_intrinsic(
    value: &CanonValue,
    remaining: usize,
) -> Result<(ProjectedValue, usize), Halt> {
    match value {
        CanonValue::Null => Ok((ProjectedValue::Null, 0)),
        CanonValue::Bool(flag) => Ok((ProjectedValue::Bool(*flag), 0)),
        CanonValue::Number(token) => number_from_token(token)
            .map(|number| (ProjectedValue::Number(number), 0))
            .map_err(|error| Halt::Defect(error.message)),
        CanonValue::String(body) => decode_json_string_body(body)
            .map(|text| (ProjectedValue::String(text), 0))
            .ok_or_else(|| {
                Halt::Defect(
                    "world JSON string body does not decode to text (lone surrogate escape)"
                        .to_string(),
                )
            }),
        CanonValue::Array(node) => {
            let cell = node.projection_cell();
            #[cfg(feature = "my-projection-census")]
            projection_census::note(cell.get().is_some());
            if let Some(memo) = cell.get() {
                return memo_outcome(memo);
            }
            let Some(remaining) = remaining.checked_sub(1) else {
                return Err(Halt::TooDeep);
            };
            // `set` rather than `get_or_init`, so that a budget exhaustion can
            // leave the cell EMPTY. A racing thread may have filled it first;
            // both outcomes are the same intrinsic fact, so the loser discards.
            match build_array_memo(node, remaining) {
                Err(TooDeep) => Err(Halt::TooDeep),
                Ok(memo) => {
                    let _ = cell.set(Box::new(memo));
                    memo_outcome(cell.get().expect("the cell was just filled"))
                }
            }
        }
        CanonValue::Object(node) => {
            let cell = node.projection_cell();
            #[cfg(feature = "my-projection-census")]
            projection_census::note(cell.get().is_some());
            if let Some(memo) = cell.get() {
                return memo_outcome(memo);
            }
            let Some(remaining) = remaining.checked_sub(1) else {
                return Err(Halt::TooDeep);
            };
            match build_object_memo(node, remaining) {
                Err(TooDeep) => Err(Halt::TooDeep),
                Ok(memo) => {
                    let _ = cell.set(Box::new(memo));
                    memo_outcome(cell.get().expect("the cell was just filled"))
                }
            }
        }
    }
}

fn memo_outcome(memo: &ProjectionMemo) -> Result<(ProjectedValue, usize), Halt> {
    match memo {
        ProjectionMemo::Projected { value, height } => Ok((value.clone(), *height)),
        ProjectionMemo::Defect(message) => Err(Halt::Defect(message.clone())),
    }
}

fn build_array_memo(node: &ArrayNode, remaining: usize) -> Result<ProjectionMemo, TooDeep> {
    let mut out = Vec::with_capacity(node.len());
    let mut deepest_child = 0usize;
    for item in node.iter() {
        match project_intrinsic(item, remaining) {
            Ok((projected, height)) => {
                deepest_child = deepest_child.max(height);
                out.push(projected);
            }
            Err(Halt::Defect(defect)) => return Ok(ProjectionMemo::Defect(defect)),
            Err(Halt::TooDeep) => return Err(TooDeep),
        }
    }
    Ok(ProjectionMemo::Projected {
        value: ProjectedValue::Array(Arc::new(out)),
        height: deepest_child.saturating_add(1),
    })
}

fn build_object_memo(node: &ObjectNode, remaining: usize) -> Result<ProjectionMemo, TooDeep> {
    let mut out: Vec<(String, ProjectedValue)> = Vec::with_capacity(node.len());
    let mut deepest_child = 0usize;
    // Canonical objects are strictly increasing by decoded name, so adjacent
    // comparison catches every duplicate; a programmatically built unsorted
    // object falls back to the full check below, preserving the depth-first
    // converter's refusal outcome without charging every canonical rebuild a
    // hash set.
    let mut sorted = true;
    for (name, member) in node.iter() {
        let Some(key) = decode_json_string_body(name) else {
            return Ok(ProjectionMemo::Defect(
                "world JSON member name does not decode to text".to_string(),
            ));
        };
        let (projected, height) = match project_intrinsic(member, remaining) {
            Ok(child) => child,
            Err(Halt::Defect(defect)) => return Ok(ProjectionMemo::Defect(defect)),
            Err(Halt::TooDeep) => return Err(TooDeep),
        };
        if let Some((previous, _)) = out.last() {
            match previous.as_str().cmp(key.as_str()) {
                Ordering::Less => {}
                Ordering::Equal => {
                    return Ok(ProjectionMemo::Defect(
                        "world JSON object has a duplicate member name".to_string(),
                    ))
                }
                Ordering::Greater => sorted = false,
            }
        }
        deepest_child = deepest_child.max(height);
        out.push((key, projected));
    }
    if !sorted {
        let mut seen = std::collections::HashSet::with_capacity(out.len());
        for (key, _) in &out {
            if !seen.insert(key.as_str()) {
                return Ok(ProjectionMemo::Defect(
                    "world JSON object has a duplicate member name".to_string(),
                ));
            }
        }
    }
    Ok(ProjectionMemo::Projected {
        value: ProjectedValue::Object(Arc::new(out)),
        height: deepest_child.saturating_add(1),
    })
}

/// Materialise a projected value into the owning `serde_json::Value` the
/// not-yet-migrated boundary still hands out. Infallible: everything was
/// decoded and validated when the projection was built, and the use-site depth
/// check has already bounded the recursion. This allocation is the
/// 220 ms-class cost that only banks away as `raw()`/`domain()` consumers
/// migrate onto the shared type — until then no latency is claimed.
pub fn projected_to_value(value: &ProjectedValue) -> Value {
    match value {
        ProjectedValue::Null => Value::Null,
        ProjectedValue::Bool(flag) => Value::Bool(*flag),
        ProjectedValue::Number(number) => Value::Number(number.clone()),
        ProjectedValue::String(text) => Value::String(text.clone()),
        ProjectedValue::Array(items) => Value::Array(items.iter().map(projected_to_value).collect()),
        ProjectedValue::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), projected_to_value(member)))
                .collect(),
        ),
    }
}

/// `projected_to_value(&project(value)?)` with the root's members projected
/// and materialised on `workers` threads. The root-level checks are the ones
/// [`build_object_memo`] makes -- decodable, non-duplicate member names -- and
/// each member is projected at depth one, which is where the root places it.
pub fn materialize_parallel(value: &CanonValue, workers: usize) -> Result<Value, CanonToValueError> {
    let CanonValue::Object(members) = value else {
        return project(value).map(|projected| projected_to_value(&projected));
    };
    if workers <= 1 || members.len() < 2 {
        return project(value).map(|projected| projected_to_value(&projected));
    }
    let slots: Vec<std::sync::Mutex<Option<Result<Value, CanonToValueError>>>> =
        (0..members.len()).map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(members.len()) {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((_, member)) = members.get(at) else { break };
                let converted = project_at(member, 1).map(|projected| projected_to_value(&projected));
                *slots[at].lock().expect("slot lock") = Some(converted);
            });
        }
    });
    let mut out = Map::new();
    for ((name, _), slot) in members.iter().zip(slots) {
        let key = decode_json_string_body(name).ok_or_else(|| {
            CanonToValueError::new("world JSON member name does not decode to text")
        })?;
        let converted = slot.into_inner().expect("slot lock").expect("every member ran")?;
        if out.insert(key, converted).is_some() {
            return Err(CanonToValueError::new(
                "world JSON object has a duplicate member name",
            ));
        }
    }
    Ok(Value::Object(out))
}

/// The root's members as shared values. A member named in `reuse.changed`, or
/// every member when `reuse` carries no change set, is projected at depth one
/// and materialised on `workers` threads; every other member is the previous
/// hour's `Arc`, which the walk proved untouched. Member by member the same
/// values as [`materialize_parallel`], with the same root-level checks.
pub fn materialize_root_members(
    value: &CanonValue,
    workers: usize,
    reuse: crate::replay_cache::RootReuse<'_>,
) -> Result<crate::replay_cache::MaterializedRoot, CanonToValueError> {
    let CanonValue::Object(members) = value else {
        return Err(CanonToValueError::new("a world body is a JSON object"));
    };
    let mut out: crate::replay_cache::MaterializedRoot = Vec::with_capacity(members.len());
    let mut todo: Vec<usize> = Vec::new();
    for (index, (name, _)) in members.iter().enumerate() {
        let key = decode_json_string_body(name).ok_or_else(|| {
            CanonToValueError::new("world JSON member name does not decode to text")
        })?;
        if out.iter().any(|(seen, _)| *seen == key) {
            return Err(CanonToValueError::new(
                "world JSON object has a duplicate member name",
            ));
        }
        let kept = match (reuse.previous, reuse.changed) {
            (Some(previous), Some(changed)) if !changed.contains(name.as_str()) => previous
                .iter()
                .find(|(seen, _)| *seen == key)
                .map(|(_, member)| member.clone()),
            _ => None,
        };
        match kept {
            Some(member) => out.push((key, member)),
            None => {
                todo.push(index);
                out.push((key, Arc::new(Value::Null)));
            }
        }
    }
    // The changed members are projected first (memoised per node, so only
    // their changed subtrees are visited), then converted with every wide
    // member split one level down so one large domain does not serialise the
    // conversion on a single worker.
    let projected: Vec<std::sync::Mutex<Option<Result<ProjectedValue, CanonToValueError>>>> =
        (0..todo.len()).map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, todo.len().max(1)) {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&index) = todo.get(at) else { break };
                *projected[at].lock().expect("slot lock") = Some(project_at(&members[index].1, 1));
            });
        }
    });
    let mut trees: Vec<ProjectedValue> = Vec::with_capacity(todo.len());
    for slot in projected {
        trees.push(slot.into_inner().expect("slot lock").expect("every member ran")?);
    }
    let converted = convert_trees_parallel(&trees, workers);
    for (&index, value) in todo.iter().zip(converted) {
        out[index].1 = Arc::new(value);
    }
    Ok(out)
}

/// [`projected_to_value`] of several trees on `workers` threads, each wide
/// container split one level down into tasks of its own.
fn convert_trees_parallel(trees: &[ProjectedValue], workers: usize) -> Vec<Value> {
    const SPLIT_AT: usize = 32;
    enum Task<'a> {
        Whole(usize, &'a ProjectedValue),
        Child(usize, usize, &'a ProjectedValue),
    }
    let mut tasks: Vec<Task<'_>> = Vec::new();
    let mut child_slots: Vec<Vec<std::sync::Mutex<Option<Value>>>> = Vec::with_capacity(trees.len());
    for (index, tree) in trees.iter().enumerate() {
        match tree {
            ProjectedValue::Object(children) if children.len() >= SPLIT_AT => {
                child_slots.push((0..children.len()).map(|_| std::sync::Mutex::new(None)).collect());
                for (child, (_, value)) in children.iter().enumerate() {
                    tasks.push(Task::Child(index, child, value));
                }
            }
            ProjectedValue::Array(children) if children.len() >= SPLIT_AT => {
                child_slots.push((0..children.len()).map(|_| std::sync::Mutex::new(None)).collect());
                for (child, value) in children.iter().enumerate() {
                    tasks.push(Task::Child(index, child, value));
                }
            }
            _ => {
                child_slots.push(Vec::new());
                tasks.push(Task::Whole(index, tree));
            }
        }
    }
    let whole_slots: Vec<std::sync::Mutex<Option<Value>>> =
        (0..trees.len()).map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, tasks.len().max(1)) {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(task) = tasks.get(at) else { break };
                match task {
                    Task::Whole(index, value) => {
                        *whole_slots[*index].lock().expect("slot lock") =
                            Some(projected_to_value(value));
                    }
                    Task::Child(index, child, value) => {
                        *child_slots[*index][*child].lock().expect("slot lock") =
                            Some(projected_to_value(value));
                    }
                }
            });
        }
    });
    let take = |slot: &std::sync::Mutex<Option<Value>>| {
        slot.lock().expect("slot lock").take().expect("every task ran")
    };
    trees
        .iter()
        .enumerate()
        .map(|(index, tree)| {
            if child_slots[index].is_empty() {
                return take(&whole_slots[index]);
            }
            match tree {
                ProjectedValue::Object(children) => Value::Object(
                    children
                        .iter()
                        .zip(&child_slots[index])
                        .map(|((name, _), slot)| (name.clone(), take(slot)))
                        .collect(),
                ),
                ProjectedValue::Array(_) => {
                    Value::Array(child_slots[index].iter().map(take).collect())
                }
                _ => unreachable!("only containers are split"),
            }
        })
        .collect()
}

/// [`projected_to_value`] with the root's members converted on `workers`
/// threads. Same value: the tree is immutable and every member is converted by
/// exactly one task, so the only difference from the serial route is which
/// thread paid for which allocation.
pub fn projected_to_value_parallel(value: &ProjectedValue, workers: usize) -> Value {
    let ProjectedValue::Object(members) = value else {
        return projected_to_value(value);
    };
    if workers <= 1 || members.len() < 2 {
        return projected_to_value(value);
    }
    // A member wide enough is split one level down so the largest domains do
    // not serialise the whole conversion on one worker.
    const SPLIT_AT: usize = 32;
    enum Task<'a> {
        Whole(usize, &'a ProjectedValue),
        Child(usize, usize, &'a ProjectedValue),
    }
    let mut tasks: Vec<Task<'_>> = Vec::new();
    let mut child_slots: Vec<Vec<std::sync::Mutex<Option<Value>>>> = Vec::with_capacity(members.len());
    for (index, (_, member)) in members.iter().enumerate() {
        match member {
            ProjectedValue::Object(children) if children.len() >= SPLIT_AT => {
                child_slots.push((0..children.len()).map(|_| std::sync::Mutex::new(None)).collect());
                for (child, (_, value)) in children.iter().enumerate() {
                    tasks.push(Task::Child(index, child, value));
                }
            }
            ProjectedValue::Array(children) if children.len() >= SPLIT_AT => {
                child_slots.push((0..children.len()).map(|_| std::sync::Mutex::new(None)).collect());
                for (child, value) in children.iter().enumerate() {
                    tasks.push(Task::Child(index, child, value));
                }
            }
            _ => {
                child_slots.push(Vec::new());
                tasks.push(Task::Whole(index, member));
            }
        }
    }
    let whole_slots: Vec<std::sync::Mutex<Option<Value>>> =
        (0..members.len()).map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(tasks.len()) {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(task) = tasks.get(at) else { break };
                match task {
                    Task::Whole(index, value) => {
                        *whole_slots[*index].lock().expect("slot lock") =
                            Some(projected_to_value(value));
                    }
                    Task::Child(index, child, value) => {
                        *child_slots[*index][*child].lock().expect("slot lock") =
                            Some(projected_to_value(value));
                    }
                }
            });
        }
    });
    let take = |slot: &std::sync::Mutex<Option<Value>>| {
        slot.lock().expect("slot lock").take().expect("every task ran")
    };
    Value::Object(
        members
            .iter()
            .enumerate()
            .map(|(index, (key, member))| {
                let converted = if child_slots[index].is_empty() {
                    take(&whole_slots[index])
                } else {
                    match member {
                        ProjectedValue::Object(children) => Value::Object(
                            children
                                .iter()
                                .zip(&child_slots[index])
                                .map(|((name, _), slot)| (name.clone(), take(slot)))
                                .collect(),
                        ),
                        ProjectedValue::Array(_) => {
                            Value::Array(child_slots[index].iter().map(take).collect())
                        }
                        _ => unreachable!("only containers are split"),
                    }
                };
                (key.clone(), converted)
            })
            .collect(),
    )
}

/// The filled projected storage reachable from `root`, each allocation charged
/// ONCE however many cells share it. A recursive per-cell sum would
/// double-count: a parent's cached projection holds its children's through the
/// same `Arc`s the children's own cells hold. This is the second of the two
/// accountings the design requires — canonical `heap_bytes` charges the empty
/// inline cell and stays independent of fill state; this charges what filling
/// actually allocated. Diagnostic: nothing budgets on it.
pub fn projected_cache_bytes(root: &CanonValue) -> u64 {
    projected_cache_bytes_over([root])
}

/// [`projected_cache_bytes`] over SEVERAL roots at once, sharing one identity
/// set so a node reachable from more than one of them is charged once.
///
/// This exists because the cursor's world is NOT the only thing holding filled
/// cells: `Shadow` remembers whole subtrees so a revived member can be restored
/// without re-emitting it, and a remembered subtree keeps whatever projection
/// its nodes had. Counting only the cursor's root reports a LOWER BOUND and
/// silently omits exactly the retention that a fold-heavy recording makes
/// largest (Codex batch-B review, round 1, finding 2).
pub fn projected_cache_bytes_over<'a>(
    roots: impl IntoIterator<Item = &'a CanonValue>,
) -> u64 {
    let mut canon_seen: std::collections::HashSet<*const ()> = std::collections::HashSet::new();
    let mut projected_seen: std::collections::HashSet<*const ()> = std::collections::HashSet::new();
    let mut total = 0u64;
    let mut stack: Vec<&CanonValue> = roots.into_iter().collect();
    while let Some(value) = stack.pop() {
        let memo = match value {
            CanonValue::Array(node) => {
                if !canon_seen.insert(Arc::as_ptr(node) as *const ()) {
                    continue;
                }
                stack.extend(node.iter());
                node.projection_cell().get()
            }
            CanonValue::Object(node) => {
                if !canon_seen.insert(Arc::as_ptr(node) as *const ()) {
                    continue;
                }
                stack.extend(node.iter().map(|(_, member)| member));
                node.projection_cell().get()
            }
            _ => continue,
        };
        let Some(memo) = memo else { continue };
        // The boxed memo itself is a filled-only allocation.
        total = total.saturating_add(std::mem::size_of::<ProjectionMemo>() as u64);
        match memo.as_ref() {
            ProjectionMemo::Defect(message) => {
                total = total.saturating_add(message.capacity() as u64);
            }
            ProjectionMemo::Projected { value, .. } => {
                total = total.saturating_add(projected_bytes(value, &mut projected_seen));
            }
        }
    }
    total
}

fn projected_bytes(
    root: &ProjectedValue,
    seen: &mut std::collections::HashSet<*const ()>,
) -> u64 {
    let arc_header = (std::mem::size_of::<usize>() * 2) as u64;
    let mut total = 0u64;
    let mut stack: Vec<&ProjectedValue> = vec![root];
    while let Some(value) = stack.pop() {
        total = total.saturating_add(match value {
            ProjectedValue::Null | ProjectedValue::Bool(_) => 0,
            // `serde_json::Number` stores its value inline (no
            // arbitrary-precision feature in this tree), so it owns nothing.
            ProjectedValue::Number(_) => 0,
            ProjectedValue::String(text) => text.capacity() as u64,
            ProjectedValue::Array(items) => {
                if !seen.insert(Arc::as_ptr(items) as *const ()) {
                    continue;
                }
                stack.extend(items.iter());
                arc_header
                    .saturating_add(std::mem::size_of::<Vec<ProjectedValue>>() as u64)
                    .saturating_add(
                        (items.capacity() * std::mem::size_of::<ProjectedValue>()) as u64,
                    )
            }
            ProjectedValue::Object(members) => {
                if !seen.insert(Arc::as_ptr(members) as *const ()) {
                    continue;
                }
                let mut keys = 0u64;
                for (key, member) in members.iter() {
                    keys = keys.saturating_add(key.capacity() as u64);
                    stack.push(member);
                }
                keys.saturating_add(arc_header)
                    .saturating_add(std::mem::size_of::<Vec<(String, ProjectedValue)>>() as u64)
                    .saturating_add(
                        (members.capacity() * std::mem::size_of::<(String, ProjectedValue)>())
                            as u64,
                    )
            }
        });
    }
    total
}

/// Lane MY projection census: how often the incremental projection finds a
/// filled cell. Measurement only — with the feature off, the note sites
/// compile away and the projection is byte for byte what it was.
#[cfg(feature = "my-projection-census")]
pub mod projection_census {
    use std::cell::Cell;

    thread_local! {
        static HITS: Cell<u64> = const { Cell::new(0) };
        static MISSES: Cell<u64> = const { Cell::new(0) };
    }

    #[inline]
    pub(crate) fn note(hit: bool) {
        if hit {
            HITS.with(|count| count.set(count.get().saturating_add(1)));
        } else {
            MISSES.with(|count| count.set(count.get().saturating_add(1)));
        }
    }

    /// Take and clear this thread's census: (filled cells reused, cells built).
    pub fn take() -> (u64, u64) {
        (
            HITS.with(|count| count.replace(0)),
            MISSES.with(|count| count.replace(0)),
        )
    }
}

fn number_from_token(token: &str) -> Result<Number, CanonToValueError> {
    if !token.contains(['.', 'e', 'E']) {
        if token.starts_with('-') {
            if let Ok(value) = token.parse::<i64>() {
                if value < 0 {
                    return Ok(Number::from(value));
                }
            }
        } else if let Ok(value) = token.parse::<u64>() {
            return Ok(Number::from(value));
        }
    }
    serde_json::from_str::<Number>(token).map_err(|error| {
        CanonToValueError::new(format!(
            "world JSON number token {token} is not a number: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_number_token_matches_the_text_round_trip() {
        let mut tokens: Vec<String> = crate::canon_value::NON_IDENTITY_TOKENS
            .iter()
            .map(|token| (*token).to_string())
            .collect();
        for extra in [
            "0",
            "-0",
            "1",
            "-1",
            "9223372036854775807",
            "-9223372036854775808",
            "9223372036854775808",
            "18446744073709551615",
            "18446744073709551616",
            "-9223372036854775809",
            "1.0",
            "-1.0",
            "0.1",
            "1e-3",
            "-0.0",
            "1.7976931348623157e308",
            "5e-324",
        ] {
            tokens.push(extra.to_string());
        }
        for token in tokens {
            let document = format!(r#"{{"v":{token}}}"#);
            let canon = CanonValue::parse(document.as_bytes()).expect("token parses");
            let direct = to_value(&canon).expect("token converts");
            let through_text =
                crate::parse_json_strict(&canon.to_json_bytes()).expect("token round trips");
            assert_eq!(direct, through_text, "token {token} converted differently");
        }
    }

    #[test]
    fn strings_names_and_containers_match_the_text_round_trip() {
        for document in [
            r#"{}"#,
            r#"[]"#,
            r#"null"#,
            r#"{"a":[1,2],"bb":"xyz"}"#,
            r#"{"a\"b":1,"aXb":2}"#,
            r#"{"v":"quote \" backslash \\ slash \/ \b\f\n\r\t"}"#,
            r#"{"v":"café 中文 😀"}"#,
            r#"{"a":{"b":{"c":[true,false,null,-0.0,1e3]}},"z":""}"#,
            r#"[{"k":"A\\"},{"k":"plain"}]"#,
        ] {
            let canon = CanonValue::parse(document.as_bytes()).expect("document parses");
            let direct = to_value(&canon).expect("document converts");
            let through_text =
                crate::parse_json_strict(&canon.to_json_bytes()).expect("document round trips");
            assert_eq!(
                direct, through_text,
                "document {document} converted differently"
            );
        }
    }

    #[test]
    fn a_body_serde_json_refuses_is_refused_here_too() {
        let canon = CanonValue::parse(br#"{"v":"\ud800"}"#).expect("the cache stores it");
        assert!(to_value(&canon).is_err(), "direct route must refuse");
        assert!(
            crate::parse_json_strict(&canon.to_json_bytes()).is_err(),
            "text route must refuse"
        );
    }

    /// THE PROJECTION PARITY GATE: the memoised route must produce exactly the
    /// `Value` the depth-first converter produces, and refuse exactly what it
    /// refuses, over every fixture the converter's own tests use.
    ///
    /// kills: `build_object_memo` inserting members in a different order than
    /// `convert`'s map inserts; `number_from_token` being bypassed on the
    /// projected route; the duplicate-name or lone-surrogate refusal dropping
    /// out of `build_*_memo`.
    #[test]
    fn the_projected_route_agrees_with_the_depth_first_converter() {
        let mut documents: Vec<String> = vec![
            r#"{}"#,
            r#"[]"#,
            r#"null"#,
            r#"true"#,
            r#""text""#,
            r#"12.50"#,
            r#"{"a":[1,2],"bb":"xyz"}"#,
            r#"{"a\"b":1,"aXb":2}"#,
            r#"{"v":"quote \" backslash \\ slash \/ \b\f\n\r\t"}"#,
            r#"{"v":"café 中文 😀"}"#,
            r#"{"a":{"b":{"c":[true,false,null,-0.0,1e3]}},"z":""}"#,
            r#"[{"k":"A\\"},{"k":"plain"}]"#,
            r#"{"v":"\ud800"}"#,
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        for token in crate::canon_value::NON_IDENTITY_TOKENS {
            documents.push(format!(r#"{{"v":{token}}}"#));
        }
        for document in &documents {
            let canon = CanonValue::parse(document.as_bytes()).expect("document parses");
            let direct = to_value(&canon);
            let projected = project(&canon);
            match (direct, projected) {
                (Ok(want), Ok(projected)) => {
                    assert_eq!(
                        projected_to_value(&projected),
                        want,
                        "document {document} projects differently"
                    );
                }
                (Err(_), Err(_)) => {}
                (want, got) => panic!(
                    "{document}: the two routes disagree about acceptance: \
                     depth-first ok={} projected ok={}",
                    want.is_ok(),
                    got.is_ok()
                ),
            }
        }
    }

    /// A node reachable at TWO absolute depths in one tree is projected once
    /// and reused by pointer at both, and the whole tree still equals the
    /// depth-first conversion.
    #[test]
    fn a_shared_node_is_projected_once_and_reused_at_two_depths() {
        let shared = CanonValue::parse(br#"{"k":[1,2,3]}"#).unwrap();
        // The same allocation at depth 1 (member "a") and depth 2 (inside the
        // array under "b"): a canonical clone is an `Arc` bump, so both
        // occurrences ARE the node.
        let world = CanonValue::object(vec![
            ("a".to_string(), shared.clone()),
            ("b".to_string(), CanonValue::array(vec![shared.clone()])),
        ]);
        let projected = project(&world).expect("projects");
        assert_eq!(
            projected_to_value(&projected),
            to_value(&world).expect("converts"),
            "the shared-node tree projects differently"
        );
        let ProjectedValue::Object(members) = &projected else {
            panic!("the root projects as an object");
        };
        let at_depth_1 = &members[0].1;
        let ProjectedValue::Array(b_items) = &members[1].1 else {
            panic!("member b projects as an array");
        };
        let at_depth_2 = &b_items[0];
        let (ProjectedValue::Object(left), ProjectedValue::Object(right)) =
            (at_depth_1, at_depth_2)
        else {
            panic!("the shared node projects as an object");
        };
        assert!(
            Arc::ptr_eq(left, right),
            "the two occurrences must reuse ONE projection, not build two"
        );
        // And the cell holds it: the canonical node's memo is filled.
        let CanonValue::Object(node) = &shared else {
            unreachable!("parsed as an object");
        };
        assert!(
            node.projection_cell().get().is_some(),
            "the shared node's cell must be filled after projection"
        );
    }

    /// DEPTH REFUSAL IS CONTEXTUAL AND NEVER STORED (design point 1): the same
    /// node succeeds at depth 0, fails past the budget, and afterwards still
    /// answers at depth 0 from the SAME filled cell — a stored depth failure
    /// would poison it, a cleared cell would rebuild it.
    #[test]
    fn a_depth_refusal_is_produced_at_the_use_site_and_never_stored() {
        let mut nested = String::new();
        for _ in 0..SERDE_JSON_RECURSION_LIMIT {
            nested.push('[');
        }
        for _ in 0..SERDE_JSON_RECURSION_LIMIT {
            nested.push(']');
        }
        let canon = CanonValue::parse(nested.as_bytes()).unwrap();

        let at_root = project_at(&canon, 0).expect("height 127 fits at depth 0");
        assert!(
            project_at(&canon, 1).is_err(),
            "height 127 under one container must refuse"
        );
        let again = project_at(&canon, 0).expect("the refusal must not poison the cell");
        let (ProjectedValue::Array(first), ProjectedValue::Array(second)) = (&at_root, &again)
        else {
            panic!("the nested document projects as an array");
        };
        assert!(
            Arc::ptr_eq(first, second),
            "both successes must be the ONE stored projection"
        );

        // The whole-document equivalence at the boundary: one deeper on both
        // routes refuses.
        let mut deeper = String::from("[");
        deeper.push_str(&nested);
        deeper.push(']');
        let canon_deeper = CanonValue::parse(deeper.as_bytes()).unwrap();
        assert!(to_value(&canon_deeper).is_err(), "depth-first route refuses");
        assert!(project(&canon_deeper).is_err(), "projected route refuses");
    }

    /// An INTRINSIC refusal — here a lone surrogate — is permanent at every
    /// depth, cached once, and does not depend on where the node sits.
    #[test]
    fn an_intrinsic_refusal_is_permanent_at_every_depth() {
        let canon = CanonValue::parse(br#"{"v":"\ud800"}"#).unwrap();
        for depth in [0usize, 5, 100, 127] {
            assert!(
                project_at(&canon, depth).is_err(),
                "a lone surrogate must refuse at depth {depth}"
            );
        }
        let CanonValue::Object(node) = &canon else {
            unreachable!("parsed as an object");
        };
        assert!(
            matches!(
                node.projection_cell().get().map(|memo| memo.as_ref()),
                Some(ProjectionMemo::Defect(_))
            ),
            "the intrinsic refusal must be cached as the node's own outcome"
        );
    }

    /// MINOR (a), PINNED: when one tree holds BOTH an intrinsic defect and a
    /// branch past the depth budget, only the refusal OUTCOME is contractual —
    /// the projected route may report either reason first, and this test
    /// deliberately compares acceptance, never message text.
    #[test]
    fn a_tree_with_both_defect_kinds_refuses_on_both_routes() {
        let mut nested = String::new();
        for _ in 0..(SERDE_JSON_RECURSION_LIMIT + 1) {
            nested.push('[');
        }
        for _ in 0..(SERDE_JSON_RECURSION_LIMIT + 1) {
            nested.push(']');
        }
        // The deep branch sits under "a"; the intrinsic defect under "b". The
        // depth-first converter hits the depth wall first (member order);
        // reversing the members would reverse that, which is exactly why the
        // message is not contractual.
        for document in [
            format!(r#"{{"a":{nested},"b":"\ud800"}}"#),
            format!(r#"{{"a":"\ud800","b":{nested}}}"#),
        ] {
            let canon = CanonValue::parse(document.as_bytes()).unwrap();
            assert!(to_value(&canon).is_err(), "depth-first route refuses");
            assert!(project(&canon).is_err(), "projected route refuses");
        }
    }

    /// A VALUE TOO DEEP TO RECURSE REFUSES; IT DOES NOT CRASH (Codex batch-B
    /// review, round 1, finding 1). The memo is filled by recursion over the
    /// subtree and the absolute-depth check happens at the use site
    /// AFTERWARDS, so without the build's own budget this exhausts the thread
    /// stack where the depth-first converter — which counts on the way DOWN
    /// and stops at container 128 — returns a refusal. The canonical decoder
    /// caps nesting, but `CanonValue::array` is a plain constructor and caps
    /// nothing, so a programmatically built value reaches this.
    ///
    /// Both routes must refuse, because the projected route's contract is
    /// equality of OUTCOME with `to_value` (the message is not contractual;
    /// see the test above).
    ///
    /// kills: reverting `project_intrinsic` to an unbudgeted recursion — every
    /// other projection test stays inside 128 containers and passes, while
    /// this one terminates the process instead of failing. It is a depth the
    /// existing 200,000-deep `heap_bytes` value in `canon_value` already
    /// builds, and that test never projects it.
    #[test]
    fn a_value_deeper_than_the_stack_refuses_instead_of_overflowing() {
        const DEPTH: usize = 200_000;
        let mut value = CanonValue::String("leaf".to_string());
        for _ in 0..DEPTH {
            value = CanonValue::array(vec![value]);
        }

        assert!(
            project(&value).is_err(),
            "the projected route must refuse a value it cannot recurse"
        );
        assert!(
            project_at(&value, 0).is_err(),
            "the refusal holds at every absolute depth: a height past the \
             limit cannot be placed anywhere"
        );
        assert!(
            to_value(&value).is_err(),
            "the depth-first route refuses it too, which is the outcome the \
             projected route has to match"
        );

        // Take the tree apart iteratively: `CanonValue`'s own `Drop` is
        // recursive, so leaving 200,000 containers to fall out of scope would
        // overflow the stack in the harness and say nothing about the code
        // under test. Same wind-down as
        // `canon_value::heap_bytes_survives_a_value_too_deep_to_recurse`.
        let mut unwind = value;
        loop {
            let next = match &mut unwind {
                CanonValue::Array(items) if !items.is_empty() => {
                    Arc::make_mut(items).contents_mut().pop()
                }
                _ => None,
            };
            match next {
                Some(inner) => unwind = inner,
                None => break,
            }
        }
    }

    /// The projected accounting is IDENTITY-AWARE: a subtree shared by two
    /// paths is charged once, so the shared tree accounts strictly less than
    /// the same document built without sharing.
    #[test]
    fn projected_accounting_charges_each_allocation_once() {
        let shared = CanonValue::parse(br#"{"k":["abcdefgh",2,3]}"#).unwrap();
        let sharing = CanonValue::object(vec![
            ("a".to_string(), shared.clone()),
            ("b".to_string(), shared.clone()),
        ]);
        let flat = CanonValue::parse(
            br#"{"a":{"k":["abcdefgh",2,3]},"b":{"k":["abcdefgh",2,3]}}"#,
        )
        .unwrap();
        assert_eq!(
            to_value(&sharing).unwrap(),
            to_value(&flat).unwrap(),
            "the two spellings are one document"
        );
        assert_eq!(
            projected_cache_bytes(&sharing),
            0,
            "nothing is charged before anything is projected"
        );
        project(&sharing).expect("projects");
        project(&flat).expect("projects");
        let shared_bytes = projected_cache_bytes(&sharing);
        let flat_bytes = projected_cache_bytes(&flat);
        assert!(
            shared_bytes < flat_bytes,
            "sharing must be charged once: shared {shared_bytes} >= flat {flat_bytes}"
        );
    }

    #[test]
    fn the_recursion_limit_is_the_one_serde_json_enforces() {
        let nested = |depth: usize| {
            let mut document = String::new();
            for _ in 0..depth {
                document.push('[');
            }
            for _ in 0..depth {
                document.push(']');
            }
            document
        };
        let accepts = |depth: usize| {
            let document = nested(depth);
            let canon = CanonValue::parse(document.as_bytes()).expect("the value model parses it");
            (
                crate::parse_json_strict(document.as_bytes()).is_ok(),
                to_value(&canon).is_ok(),
            )
        };
        let mut deepest_accepted = 0;
        for depth in 1..=(SERDE_JSON_RECURSION_LIMIT + 8) {
            if accepts(depth).0 {
                deepest_accepted = depth;
            } else {
                break;
            }
        }
        assert_eq!(
            deepest_accepted, SERDE_JSON_RECURSION_LIMIT,
            "serde_json's nesting limit moved; SERDE_JSON_RECURSION_LIMIT must follow it"
        );
        assert_eq!(
            accepts(deepest_accepted),
            (true, true),
            "the two routes disagree at the deepest accepted document"
        );
        assert_eq!(
            accepts(deepest_accepted + 1),
            (false, false),
            "the two routes disagree one past the deepest accepted document"
        );
    }
}
