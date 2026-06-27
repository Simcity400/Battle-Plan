"""registry.py -- the modular READER-PLUG registry.

Each cracked memory domain is an optional *plug*. A plug owns:

  * a stable ``name`` (the provenance domain id it contributes),
  * an import-guarded ``available`` flag (the underlying reader module imported
    cleanly -> True; an ImportError -> the plug is permanently unavailable this
    run, NO fabrication),
  * a ``read(ctx) -> dict`` that returns a small, plain-data result for the
    assembler to fold into a snapshot.

The capture loop (capture.py) calls ``read_all(ctx)``; the assembler
(assembler.py) consumes the returned ``DomainResult``s. The contract is strict
and honest:

  * A plug whose module did not import -> NOT run; its domain is absent from
    ``provenance.memory.domains``; every field it would have filled stays null.
  * A plug that RAISES at read time -> its domain is absent from provenance and
    its (partial) data is DISCARDED (transactional: the assembler only ever sees
    a result that was produced without raising). The error is recorded in the
    result for diagnostics but never reaches the snapshot as data.
  * A plug that returns successfully -> its domain IS listed in provenance, even
    if the data is empty (measured-absent: e.g. "no active combats" is a real
    observation, distinct from "combats not measured").

This is the single choke point that enforces DESIGN no-fabrication: a domain is
"captured" iff a plug ran to completion without raising.

The naval/air COMBAT reader and the weather reader are being cracked by other
agents right now; their plugs import-guard their modules and simply report
unavailable until those land -- no edits to memory_reader/ are required here.

Pure orchestration: this module reads nothing from memory itself. It imports the
reader modules lazily inside each factory so a missing/broken reader can never
break the import of live_capture.
"""
from __future__ import annotations

import hashlib as _hashlib
import os
import sys
import traceback
from dataclasses import dataclass, field
from typing import Any, Callable, Dict, List, Optional

# Make the repo root importable so `import winmem`, `from live_layers import ...`,
# etc. resolve the same way the readers expect when run in-tree.
_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir))
_MR = os.path.join(_REPO, "memory_reader")
for _p in (_REPO, _MR):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from replay import schema  # noqa: E402  (stdlib-only module; single source of valid_tag)


@dataclass
class CaptureContext:
    """Everything a plug needs to read once. Built fresh per capture by the loop.

    ``proc`` is an open winmem.Process (shared by the winmem-based plugs).
    ``tag_table`` / ``bridge`` / ``province_control`` are the already-resolved
    live-layer objects (resolved once per capture and reused). Any may be None if
    its resolve() failed -- a plug that needs one and finds it None must raise (so
    its domain is honestly omitted), NOT fabricate.
    """

    proc: Any = None
    identity: Any = None              # ProcessIdentity
    tag_table: Any = None
    bridge: Any = None
    province_control: Any = None
    # The SINGLE canonical country IDENTITY (diplomacy_live.CountryIdentity) for this capture,
    # resolved at most ONCE and reused by every country-identity reader (diplomacy / diplo_
    # relations / opinion-modifiers). Distinct from ``identity`` (the ProcessIdentity). It is
    # resolved LAZILY via ``country_identity()`` (so a subset capture that needs no identity
    # never pays for it) and is fail-closed: it is cached ONLY when it resolves AND agrees with
    # ctx.bridge (same TAG + same country object for every shared tag-db-id); a disagreement /
    # partial / failed resolve caches None, so the readers self-resolve and a BAD identity is
    # NEVER threaded. ``_identity_full`` holds the resolved object; ``_identity_full_done``
    # marks "we tried" so a None result is not re-attempted every walk within one process epoch.
    _identity_full: Any = None
    _identity_full_done: bool = False
    extras: Dict[str, Any] = field(default_factory=dict)

    # ---- STEADY-STATE resolve cache (perf) ----------------------------------
    # The 3 live layers above are resolved ONCE per capture and reused. The other
    # heap-scanning readers (state_detail / state_vp / buildings_live / mio /
    # designs_variants / division_equipment / supply / the shared CountryIdentity)
    # each cost a FULL ~7.6GB heap scan (iter_regions + AOB vtable search) PER walk.
    # ``pins`` caches each one's LOCATED ROOTS across walks so a warmed walk re-reads
    # ONLY the cheap per-root fields (no scan). It is keyed off ctx.extras["pins"]
    # (a dict the long-lived sampler owns so the cache survives across capture_once
    # calls); if the caller did not supply one we fall back to a ctx-local dict (the
    # perf probe / validate harness build ONE ctx and reuse it, so even the fallback
    # caches correctly). Invalidation is fail-CLOSED + no-fabrication (see _pin_get):
    # a moved/freed/realloc'd root is RE-SCANNED, never served stale.
    @property
    def pins(self) -> Dict[str, Any]:
        store = self.extras.get("pins") if isinstance(self.extras, dict) else None
        if not isinstance(store, dict):
            store = {}
            if isinstance(self.extras, dict):
                self.extras["pins"] = store
        return store

    def proc_fp(self):
        """The process-instance fingerprint a pinned root is bound to. A pin is
        ONLY reused while this is unchanged: (pid, creation-time, exe-sha, ASLR
        base). Any change -> the whole process/build/mapping changed -> every pin
        is invalidated (full re-scan). create_time defeats PID reuse; module_base
        is included because the cached vtable ABSOLUTE addresses are base-relative
        (a re-mapped image would shift every RVA -- Codex Q4)."""
        ident = self.identity
        p = self.proc
        return (
            getattr(ident, "pid", None) if ident is not None else getattr(p, "pid", None),
            getattr(ident, "create_time_100ns", None) if ident is not None else None,
            getattr(ident, "exe_sha256", None) if ident is not None else None,
            getattr(p, "module_base", None) if p is not None else None,
        )

    def resolve_tag(self, tag_db_id: Optional[int]) -> Optional[str]:
        """tag_db_id -> TAG via the live tag table, or None. NEVER returns a
        numeric id (schema forbids a numeric id in a *_tag field)."""
        if self.tag_table is None or tag_db_id is None:
            return None
        try:
            t = self.tag_table.tag(int(tag_db_id))
        except Exception:
            return None
        if not t or t == "---":
            return None
        return t

    # ---- STAGE 3: arena-restrict scan range (config flag, default ON) -----------------------
    # Full-heap scans use fastscan.discover_arena's tight ~7.58GB bounds (the densest committed
    # cluster), skipping the vast empty 47-bit address space (0x10000 .. 0x7FFFFFFFFFFF). This is
    # ON by DEFAULT as of Stage 3: the live coverage-diff tool proved -- on a paused heap -- that
    # the SAME accepted roots are found under the tight arena vs the wide range (rec0, all 143
    # bridge entries, all 179 divisions matched; no root lives outside the arena). discover_arena
    # failure falls back to the wide range (fail-OPEN to correctness; see scan_range).
    def arena_restrict_enabled(self) -> bool:
        """True iff arena-restrict is configured ON. DEFAULT ON (Stage 3, live-proof gated); OFF
        only when explicitly disabled via ctx.extras['arena_restrict'] falsey OR the
        CAPTURE_ARENA_RESTRICT env var set to a disable token. Read once per call (cheap)."""
        import os as _os
        v = self.extras.get("arena_restrict") if isinstance(self.extras, dict) else None
        if v is None:
            ev = _os.environ.get("CAPTURE_ARENA_RESTRICT")
            if ev is None:
                return True                          # Stage 3: DEFAULT ON
            return ev.strip().lower() not in ("", "0", "false", "no", "off")
        return bool(v)

    def scan_range(self, default_lo: int = 0x10000,
                   default_hi: int = 0x7FFFFFFFFFFF):
        """Return the (lo, hi) address window a full heap scan should use THIS capture.

        Default (flag OFF): the wide 47-bit window (default_lo, default_hi) -- IDENTICAL to the
        pre-Stage-3 behaviour (so OFF is a guaranteed no-op). Flag ON: fastscan.discover_arena's
        tight arena bounds, discovered ONCE per capture and cached on ctx.extras['_arena_bounds'].
        If discover_arena fails (returns None) we FALL BACK to the wide window -- fail-OPEN to
        CORRECTNESS (a wider scan can only find MORE roots, never fewer; it is never a fabrication
        risk, only a perf regression), never silently narrowing to a bad arena."""
        if not self.arena_restrict_enabled():
            return default_lo, default_hi
        store = self.extras if isinstance(self.extras, dict) else {}
        cached = store.get("_arena_bounds")
        if cached is None:
            cached = False                       # sentinel: "tried, failed" (don't retry every call)
            try:
                import fastscan as _fs
                pid = getattr(self.proc, "pid", None)
                if pid is not None:
                    ar = _fs.discover_arena(pid)
                    if ar is not None:
                        cached = (int(ar[0]), int(ar[1]))
            except Exception:
                cached = False
            if isinstance(self.extras, dict):
                self.extras["_arena_bounds"] = cached
        if isinstance(cached, tuple):
            return cached
        return default_lo, default_hi            # discover_arena failed -> wide (fail-open)

    def country_identity(self) -> Any:
        """The SINGLE canonical country CountryIdentity for this capture (resolved at most
        once, lazily), or None. Fail-closed + no-fabrication:

          * resolved ONCE and cached on ctx; a None result is remembered (``_identity_full_done``)
            so a failed resolve is not retried every walk within the same process epoch;
          * the resolve REUSES ctx.tag_table + ctx.bridge (no fresh scans for those layers) and,
            crucially, attaches ctx.bridge so the identity resolves country-object POINTERS LIVE
            through the bridge (only the stable TAG<->id maps are cached -- Codex-vetted);
          * it is cached ONLY when it resolves AND AGREES with ctx.bridge: for every tag-db-id in
            BOTH, the TAG and the live country-object pointer must match. Any disagreement, a
            partial resolve, or an exception -> cache None (the readers then self-resolve, and a
            BAD/INCONSISTENT identity is never threaded -- one canonical identity owner or none).

        A change of process/build is handled by the loop rebuilding the ctx (and the pins'
        ``__fp__`` epoch); this accessor's cache lives on the per-process ctx.

        ctx.bridge is FIXED for the ctx epoch (resolved once at attach in _resolve_live_layers and
        never re-resolved within one capture), so the bridge-availability used by the equivalence
        gate is terminal for this ctx (Codex finding 2): if the bridge was None at attach it stays
        None for the whole walk, and the identity (resolved with bridge=None) is self-consistent;
        if the bridge resolved, every reuse re-gates against it via _pin_country_reader's validate
        (which clears this cache on a liveness miss so the next call re-resolves + re-gates)."""
        if self._identity_full_done:
            return self._identity_full
        self._identity_full_done = True
        self._identity_full = None
        if self.proc is None:
            return None
        try:
            import diplomacy_live as _dl
        except Exception:
            return None
        try:
            ident = _dl.CountryIdentity.resolve(self.proc, bridge=self.bridge)
        except Exception:
            ident = None
        if ident is None:
            return None
        # Equivalence gate vs ctx.bridge (the canonical owner): every tag-db-id present in BOTH
        # must map to the SAME TAG and the SAME live country object. A bridge that did not resolve
        # (None) leaves the identity self-consistent (it was resolved with bridge=None internally
        # then), which is acceptable -- but if a bridge DOES exist it must agree, else fail closed.
        if self.bridge is not None:
            try:
                if not _identity_agrees_with_bridge(self.proc, ident, self.bridge):
                    return None     # two authoritative maps disagree -> use neither (fail closed)
            except Exception:
                return None
        self._identity_full = ident
        return ident


# ---------------------------------------------------------------------------
# STEADY-STATE resolve cache (perf): pin a heap-scanned reader's LOCATED ROOTS in
# ctx.pins and re-read fields from them on later walks WITHOUT re-scanning, with a
# fail-closed, no-fabrication invalidation gate. See CaptureContext.pins.
#
# Invalidation set (Codex-reviewed -- the union closes the ABA-reuse + completeness
# holes for a PAUSED, build-gated, single-process, read-only capture):
#   1. PROCESS fingerprint: (pid, create_time, exe_sha, module_base) -- a different
#      process / build / image mapping invalidates EVERY pin (the cached absolute
#      vtable/object addresses are base-relative).
#   2. Per-root SEMANTIC identity (validate_fn): re-read the vtable AND the object's
#      own id at a/each pinned root and confirm vtable==expected AND id==cached_key.
#      The id-match defeats ABA reuse (a freed slot re-used by a same-class object
#      would still match the vtable but NOT the original key). On ANY mismatch the
#      pin is DISCARDED and the reader RE-SCANS (never serves a stale/moved root).
#   3. POPULATION fingerprint: a cheap count/shape token recorded at pin time and
#      re-checked from the SAME cached roots on reuse. The game STAYS PAUSED for the
#      whole capture session (a hard system invariant -> no object alloc/free between
#      walks), so the located-root SET cannot grow/shrink; the population token is a
#      belt-and-braces tripwire that forces a full re-scan if it ever does.
# A failed gate ALWAYS falls back to a full re-scan + re-pin -- the slow-but-correct
# path -- so the cache can only ever make a walk FASTER, never WRONG.
# ---------------------------------------------------------------------------
@dataclass
class _PinEntry:
    fp: Any                       # ctx.proc_fp() at pin time
    roots: Any                    # the located-roots object (a layer or an addr map)
    pop: Any = None               # population token (count/shape) at pin time


# The exhaustive set of mutually-exclusive _pin_get branches a single key can take on one
# walk. The READ-ONLY branch trace (ctx.extras["pin_trace"]) records exactly one of these per
# pinned read, so the perf harness can see -- per warm walk -- which pinned reader took the warm
# fast-path ("hit") vs why it fell back to a full re-scan. Diagnostic-only: it changes NO read
# behaviour, allocates a tiny dict, and is OFF unless the caller opts in.
_PIN_BRANCHES = (
    "hit",                 # validate passed -> warm reread, NO scan (the goal)
    "miss_no_entry",       # no pin cached for this key yet (cold / first warm after epoch reset)
    "miss_validate_false", # validate returned a falsey value -> re-scan (fail-closed)
    "miss_validate_raise", # validate RAISED -> re-scan (fail-closed; e.g. an undefined name)
    "miss_reread_raise",   # validate passed but reread raised mid-walk -> re-scan (fail-closed)
    "resolve_pinned",      # full scan ran and the result WAS pinned (roots is not None)
    "resolve_unpinned",    # full scan ran but roots is None -> nothing pinned (honest absence)
)


def _pin_trace_record(ctx: "CaptureContext", key: str, branch: str) -> None:
    """Append (key, branch) to the per-walk pin trace IFF the caller enabled it. READ-ONLY:
    never touches game memory, never raises into the read path, and is a no-op unless
    ctx.extras['pin_trace'] is a list (the harness/tests set this; production leaves it unset).

    The trace records EVERY branch a key hits, including the resolve_* outcomes that follow a
    miss_* on the SAME walk -- so a warm walk that re-scans shows e.g. ['miss_validate_raise',
    'resolve_pinned'] for that key, pinpointing the fail-closed cause. (branch is one of
    _PIN_BRANCHES.)"""
    extras = getattr(ctx, "extras", None)
    if not isinstance(extras, dict):
        return
    trace = extras.get("pin_trace")
    if isinstance(trace, list):
        trace.append((key, branch))


def _pin_get(ctx: "CaptureContext", key: str,
             resolve_fn, validate_fn, reread_fn) -> Any:
    """Return this domain's read RESULT, reusing pinned located roots when valid.

    ``resolve_fn() -> (roots, data, pop)`` : the FULL scan. Returns the located roots
        (to pin), the FRESH read result for THIS walk (used directly -- no double work
        on the pinning walk), and a population token. ``roots is None`` -> the caller
        treats the domain as unresolved (raises / honest absence).
    ``validate_fn(roots, pop) -> bool`` : a CHEAP re-read confirming the pinned roots
        are the SAME live objects (vtable + semantic id) AND the population is unchanged.
        Any False (or an exception) -> the pin is dropped and we RE-SCAN (fail-closed).
    ``reread_fn(roots) -> data`` : the WARMED steady-state read -- re-read the fields
        from the pinned roots WITHOUT scanning.

    No-fabrication invariant: a root is reused ONLY while the process fingerprint AND
    validate_fn both pass; otherwise the slow full scan replaces it. So the cache can
    only make a walk faster, never serve a moved/freed/stale root as live.

    INSTRUMENTATION (read-only): each call records exactly one warm-path outcome
    (hit / miss_no_entry / miss_validate_false / miss_validate_raise / miss_reread_raise)
    and, when it falls through to the full scan, one resolve outcome (resolve_pinned /
    resolve_unpinned) via ctx.extras['pin_trace'] when the caller opts in. This is the
    branch-trace the perf harness uses to see why a warm walk re-scans; it never alters reads."""
    pins = ctx.pins
    fp = ctx.proc_fp()
    # GLOBAL epoch: if the process/build/mapping changed, EVERY pin is stale (the cached
    # absolute addresses are base-relative) -> clear the whole cache, not just this key.
    if pins.get("__fp__") != fp:
        pins.clear()
        pins["__fp__"] = fp
    ent = pins.get(key)
    if isinstance(ent, _PinEntry) and ent.fp == fp and ent.roots is not None:
        ok = False
        try:
            ok = bool(validate_fn(ent.roots, ent.pop))
        except Exception:
            ok = False
            _pin_trace_record(ctx, key, "miss_validate_raise")
        if ok:
            try:
                data = reread_fn(ent.roots)  # FAST PATH: re-read pinned roots, NO scan
                _pin_trace_record(ctx, key, "hit")
                return data
            except Exception:
                # a reread that raised AFTER validation passed (e.g. a root became
                # unreadable mid-walk) must NOT omit the domain on a possibly-stale pin:
                # drop the pin and fall through to a full re-scan (slow-but-correct).
                _pin_trace_record(ctx, key, "miss_reread_raise")
        elif not _pin_trace_last_is(ctx, key, "miss_validate_raise"):
            # validate returned falsey (NOT raised; the raise case is already recorded above).
            _pin_trace_record(ctx, key, "miss_validate_false")
        pins.pop(key, None)               # stale/moved/freed/repopulated -> re-scan
    else:
        _pin_trace_record(ctx, key, "miss_no_entry")
    roots, data, pop = resolve_fn()
    if roots is None:
        _pin_trace_record(ctx, key, "resolve_unpinned")
        return data                       # unresolved: caller raises / honest absence
    pins[key] = _PinEntry(fp=fp, roots=roots, pop=pop)
    _pin_trace_record(ctx, key, "resolve_pinned")
    return data                           # fresh scan result for THIS (pinning) walk


def _pin_trace_last_is(ctx: "CaptureContext", key: str, branch: str) -> bool:
    """True iff the most recent pin-trace entry is (key, branch) -- lets _pin_get avoid double-
    recording a validate outcome (it records miss_validate_raise inside the except, so the
    falsey-branch must not ALSO record miss_validate_false for the same call). No-op-safe when
    tracing is off."""
    extras = getattr(ctx, "extras", None)
    if not isinstance(extras, dict):
        return False
    trace = extras.get("pin_trace")
    if isinstance(trace, list) and trace:
        return trace[-1] == (key, branch)
    return False


def _vt_at(p, addr: int) -> Optional[int]:
    """Read the vtable qword at a heap object base, or None (unreadable)."""
    try:
        return p.u64(addr)
    except OSError:
        return None


def _identity_agrees_with_bridge(p, ident, bridge) -> bool:
    """True iff the CountryIdentity and the CountryIndexBridge are the SAME canonical map:
    for every tag-db-id present in BOTH, the TAG matches AND the live country-object pointer
    matches. Fail-closed: any disagreement, or fewer than a small quorum of shared ids, -> False
    (so the caller threads NEITHER and the readers self-resolve). This prevents two independently
    resolved 'authoritative' country maps (one possibly a different mirror copy) from producing a
    mixed snapshot no single per-reader resolution would have produced (Codex Q1b)."""
    try:
        bmap = bridge.as_map()                 # {tag_db_id: running_index}
    except Exception:
        return False
    if not bmap:
        return False
    shared = 0
    for tid in bmap:
        itag = ident.tag(tid)
        if itag is None:
            continue                            # id not in the identity -> not a shared id
        btag = None
        try:
            btag = bridge.tag(tid)
        except Exception:
            btag = None
        if btag is None:
            continue
        if itag != btag:
            return False                        # same id, different TAG -> disagree
        # live country-object pointer must match (identity.object_of resolves through the
        # bridge when attached, so this is normally trivially equal; the explicit compare
        # also catches a bridge attached AFTER a non-bridge resolve).
        iobj = ident.object_of(itag)
        try:
            bobj = bridge.country_object(tid)
        except Exception:
            bobj = None
        if iobj and bobj and iobj != bobj:
            return False                        # same id+TAG, different object -> disagree
        shared += 1
    return shared >= 6                          # need a real overlap to call them equivalent


def _bridge_idx_tag(ctx: "CaptureContext"):
    """Derive, from the SHARED ctx.bridge (CountryIndexBridge resolved once at attach), the
    triple the per-country PP-bridge readers (focus / politics / tech / leadership / politics_
    residuals) need: ({running_index: TAG} dict, rec0, count). Returns (None, None, None) when
    no live bridge is available -> the reader self-locates via its save-PP Rosetta (honest).

    CRITICAL (correctness, coordinator + Codex-confirmed): supplying rec0/count FROM THE BRIDGE
    lets the reader SKIP its own _locate_country_array, which otherwise runs a stale-save-PP
    anchor relocate that raises LocateError on a live frame and -- as a BaseException before the
    fix -- killed the WHOLE walk. The bridge base is the live-validated array (locate.locate's
    structural + name quorum), authoritative for live capture. rec0 AND count are returned
    together (a partial supply is a caller bug the readers reject)."""
    bridge = ctx.bridge
    if bridge is None:
        return None, None, None
    rec0 = getattr(bridge, "rec0", None)
    count = getattr(bridge, "count", None)
    idx_tag = {}
    try:
        amap = bridge.as_map()
    except Exception:
        amap = {}
    for tag_db_id, running_index in amap.items():
        tag = ctx.resolve_tag(tag_db_id)
        if tag is not None and running_index is not None:
            idx_tag[int(running_index)] = tag
    if not idx_tag:
        return None, None, None                 # un-nameable -> let the reader self-bridge
    # rec0/count must be a usable PAIR to be passed; if either is missing, pass NEITHER (the
    # reader then self-locates rec0/count but still uses the bridge idx_tag for naming).
    if rec0 is None or count is None:
        return idx_tag, None, None
    return idx_tag, int(rec0), int(count)


def _shared_state_objs(ctx: "CaptureContext") -> Dict[int, int]:
    """Return the live {state_id: obj_base} root map, SHARED across the state-object
    readers (state_detail / state_fields / country_containers) so the expensive whole-heap
    state-vtable scan (StateDetailLive.enumerate_states) runs at most ONCE per capture.

    Reuses the pinned state_detail roots when they are already in ctx.pins (the common
    case -- _pin_state_detail runs first in the registry order and caches the located
    {state_id: obj_base} under the 'state_detail' pin). Falls back to a single
    enumerate_states scan, cached on ctx.extras for the rest of THIS capture, when the
    pin is absent (e.g. a subset capture that skipped the state_detail plug). Fail-open:
    a scan failure returns {} (the caller honestly omits its domain)."""
    # 1) reuse the pinned state_detail roots if present (no scan).
    pins = ctx.pins
    ent = pins.get("state_detail")
    roots = getattr(ent, "roots", None)
    if isinstance(roots, dict):
        states = roots.get("states")
        if isinstance(states, dict) and states:
            return {int(k): int(v) for k, v in states.items()}
    # 2) per-capture cache on ctx.extras (so a second state reader this walk doesn't re-scan).
    store = ctx.extras if isinstance(ctx.extras, dict) else {}
    cached = store.get("_state_objs")
    if isinstance(cached, dict):
        return cached
    out: Dict[int, int] = {}
    try:
        from state_detail_live import StateDetailLive
        scanned = StateDetailLive.enumerate_states(ctx.proc)
        if isinstance(scanned, dict):
            out = {int(k): int(v) for k, v in scanned.items()}
    except Exception:
        out = {}
    if isinstance(ctx.extras, dict):
        ctx.extras["_state_objs"] = out
    return out


# ---------------------------------------------------------------------------
# SHARED COUNTRY-IDENTITY pin (the ONE canonical country root, Codex-vetted)
# ---------------------------------------------------------------------------
# Every reader whose only expensive root IS the country set (diplomacy, diplo_relations, fuel,
# construction, decisions) reuses the SAME canonical {bridge, identity} resolved once at attach
# (ctx.bridge + ctx.country_identity()). So none of them re-scans the heap for the country set --
# Stage 1 already eliminated that. To still give each a fail-closed WARM path through _pin_get
# (task requirement: "all 15 pinned, warm walks skip re-scanning"), each registers its OWN pin
# KEY whose ROOTS are the shared canonical {bridge, identity}, with a shared validate gate, and
# its own reread that re-runs the reader against those pinned roots WITHOUT any fresh identity/
# bridge scan. _pin_get caches ROOTS (not data), so distinct keys never collide on shape, and the
# wrapper's emptiness/validity checks still run every walk on the freshly-reread data.
#
# Robustness (coordinator + Codex): if the canonical bridge/identity did not resolve, roots is
# None -> _pin_get returns the reader's honest result (which self-resolves or raises). A BAD or
# partial canonical identity is never pinned (the equivalence gate in ctx.country_identity()
# already fail-closes it to None).
def _pin_country_reader(ctx: "CaptureContext", key: str, read_fn,
                        need_identity: bool = False):
    """Run a country-set reader against the shared canonical {bridge, identity} root, pinned.

    ``read_fn(roots) -> data`` does the actual read, consuming roots["bridge"] (a
    CountryIndexBridge or None) and roots["identity"] (a CountryIdentity or None). It MUST NOT
    perform its own identity/bridge heap scan -- it threads the shared objects (passing None when
    a shared object is absent makes the underlying reader self-resolve, which is the fail-open
    honest path, NOT fabrication).

    ``need_identity``: when True the canonical CountryIdentity is required to pin (its absence
    means there is no shared root to cache -> we still RUN the reader each walk, self-resolving).
    When False only the bridge is needed to pin.

    The pin's validate gate re-confirms the shared roots are still live: bridge.revalidate() and
    (when present) identity.revalidate(), plus a population token (exact tag-db-id set). Any miss
    -> re-resolve the canonical roots + re-run. Fail-closed; the cache only makes a walk faster.

    Codex finding 1: _resolve() must re-fetch FRESH canonical roots after a validate MISS (the
    previously-pinned roots were moved/freed), NOT close over a stale identity. The ctx identity
    cache is reset by _validate WHEN IT FAILS (the precise "roots are stale" signal), so the next
    _resolve genuinely re-resolves. The COLD path is unaffected: the first key's _resolve resolves+
    caches the identity ONCE and keys 2..5 reuse it (single resolution per cold walk preserved)."""
    p = ctx.proc

    def _resolve():
        # FRESH canonical roots each cold/post-miss resolve. ctx.country_identity() is cached on
        # ctx, so the FIRST country-set key resolves it once and the rest reuse it (single
        # resolution); a prior _validate failure cleared that cache -> a genuine re-resolve here.
        bridge = ctx.bridge                      # the bridge object reads pointers live each call
        identity = ctx.country_identity()        # resolved (or re-resolved) + re-gated vs the bridge
        # The canonical roots are the shared, already-resolved objects. We pin them only when a
        # real shared root exists; otherwise roots=None and the reader runs (self-resolving).
        have = (identity is not None) if need_identity else (bridge is not None)
        if not have:
            return None, read_fn({"bridge": bridge, "identity": identity}), None
        roots = {"bridge": bridge, "identity": identity}
        pop = identity.population_token() if identity is not None else None
        if pop is None and bridge is not None:
            try:
                pop = frozenset(bridge.as_map().items())
            except Exception:
                pop = None
        data = read_fn(roots)
        return roots, data, pop

    def _fail():
        # a stale-root signal: drop the ctx canonical-identity cache so the next _resolve
        # genuinely re-resolves it (Codex finding 1) rather than serving the moved/freed identity.
        ctx._identity_full = None
        ctx._identity_full_done = False
        return False

    def _validate(roots, pop):
        br = roots.get("bridge")
        idn = roots.get("identity")
        # bridge liveness (re-reads anchor slots)
        if br is not None:
            try:
                if not br.revalidate():
                    return _fail()
            except Exception:
                return _fail()
        elif need_identity is False:
            return _fail()                      # the pinned root WAS the bridge; it's gone
        # identity liveness (re-reads spread anchors' vtable + name)
        if idn is not None:
            try:
                if not idn.revalidate(p):
                    return _fail()
            except Exception:
                return _fail()
        elif need_identity:
            return _fail()
        # population token (exact id set) -- a TAG swap / released country / partial re-resolve
        # changes this even when a count would not.
        if pop is not None:
            cur = idn.population_token() if idn is not None else None
            if cur is None and br is not None:
                try:
                    cur = frozenset(br.as_map().items())
                except Exception:
                    cur = None
            if cur != pop:
                return _fail()
        return True

    def _reread(roots):
        # re-run the reader against the SAME pinned shared roots -- NO identity/bridge scan.
        return read_fn(roots)

    return _pin_get(ctx, key, _resolve, _validate, _reread)


@dataclass
class DomainResult:
    """The outcome of running one plug for one capture."""

    name: str
    available: bool                 # the plug's module imported
    captured: bool                  # read() completed without raising
    data: Optional[Dict[str, Any]] = None
    error: Optional[str] = None     # diagnostic only; never enters the snapshot

    @property
    def domain_captured(self) -> bool:
        return self.available and self.captured


class ReaderPlug:
    """One optional domain reader.

    ``factory`` lazily imports + returns the underlying reader callable (or raises
    ImportError). ``reader`` is ``(ctx, impl) -> dict`` doing the actual read.
    """

    def __init__(self, name: str,
                 factory: Callable[[], Any],
                 reader: Callable[[CaptureContext, Any], Dict[str, Any]]):
        self.name = name
        self._factory = factory
        self._reader = reader
        self._impl: Any = None
        self._import_error: Optional[str] = None
        self._available: Optional[bool] = None

    @property
    def available(self) -> bool:
        """True iff the underlying reader module imported. Cached; import is tried
        once. An ImportError (or any import-time error) -> permanently False."""
        if self._available is None:
            try:
                self._impl = self._factory()
                self._available = True
            except Exception as exc:  # ImportError + anything raised at import
                self._impl = None
                self._available = False
                self._import_error = "%s: %s" % (type(exc).__name__, exc)
        return self._available

    def read(self, ctx: CaptureContext) -> DomainResult:
        """Run the plug transactionally: build the domain's data in a LOCAL dict
        and only return it on success. Any exception -> captured=False, data
        discarded (the assembler never folds a partially-built domain)."""
        if not self.available:
            return DomainResult(self.name, available=False, captured=False,
                                error=self._import_error)
        try:
            data = self._reader(ctx, self._impl)
            if data is None:
                data = {}
            return DomainResult(self.name, available=True, captured=True,
                                data=data)
        except Exception:
            return DomainResult(self.name, available=True, captured=False,
                                data=None, error=traceback.format_exc(limit=6))


# ---------------------------------------------------------------------------
# Plug factories + readers. Each factory imports its module lazily; each reader
# returns plain JSON-able data keyed exactly as the assembler expects.
# ---------------------------------------------------------------------------
def _f_state_control():
    from live_layers import province_control as pc  # noqa
    return pc


def _pin_controllers_raw(ctx: CaptureContext, pc) -> Dict[int, int]:
    """Resolve-or-REUSE the {state_id: controller_tag_db_id} raw map WITHOUT re-running the
    per-state vtable heap scan on a warmed walk.

    The per-state runtime objects (vtable MODULE+0x2910920, state_id u32@+0x58, controller
    tag-db-id u32@+0xCC) are the SAME objects the state_detail pin located ({state_id:
    obj_base}); state_control reuses that shared map via _shared_state_objs(ctx) (no new scan)
    and RE-READS controller@+0xCC LIVE from each pinned obj each walk (Codex: re-reading the
    controller live closes the stale-controller hole -- nothing decoded is cached).

    Invalidation (fail-closed, Codex-vetted): validate re-reads, at SPREAD SENTINEL states,
    the per-state vtable (== MODULE+STATE_VTABLE_REL) AND the state_id (u32@+0x58 == the pinned
    key) -- so a moved/freed/ABA-reused state object forces a re-scan; plus the state COUNT is
    a tripwire. (Sentinel sampling is sufficient under the HARD PAUSED invariant -- no state
    object can enter/leave/change identity between reads -- per Codex.) On any miss, OR when no
    shared state-object map is available (state_detail pin absent + scan empty), we FALL BACK to
    pc.enumerate_state_controllers (the original full vtable scan) -- never serving a stale map.

    Returns {state_id(int): controller_tag_db_id(int)}; the caller folds tag-db-id -> TAG."""
    from live_layers.province_control import (STATE_VTABLE_REL, STATE_OFF_SID,
                                              STATE_OFF_CTRL)
    p = ctx.proc

    def _read_ctrl_map(states: Dict[int, int]) -> Dict[int, int]:
        # EXACT parity with enumerate_state_controllers: a state is emitted only when its
        # controller tag-db-id is in the sane band [0, 200) (the same gate the full scan uses
        # at province_control.py line 114), so the pinned warm map == the cold-scan map.
        out: Dict[int, int] = {}
        for sid, obj in states.items():
            try:
                ctrl = p.u32(obj + STATE_OFF_CTRL)
            except OSError:
                continue
            if 0 <= ctrl < 200:
                out[int(sid)] = ctrl
        return out

    def _resolve():
        states = _shared_state_objs(ctx)   # reuses the state_detail pin (no scan) when present
        if isinstance(states, dict) and states:
            roots = {"states": {int(k): int(v) for k, v in states.items()}}
            data = _read_ctrl_map(roots["states"])
            return roots, data, len(roots["states"])
        # no shared state-object map -> fall back to the original full vtable scan (not pinned;
        # honest: the next walk retries the shared map first).
        return None, pc.enumerate_state_controllers(p), None

    def _validate(roots, pop):
        states = roots.get("states") or {}
        if pop is not None and len(states) != pop:     # population tripwire
            return False
        vt_expected = p.module_base + STATE_VTABLE_REL
        keys = sorted(states)
        if not keys:
            return False
        for sid in _sentinel_keys(keys):               # spread sentinels (vtable + id ABA)
            obj = states[sid]
            if _vt_at(p, obj) != vt_expected:          # moved/freed/different class
                return False
            try:
                if p.u32(obj + STATE_OFF_SID) != sid:  # ABA: same vtable, wrong id
                    return False
            except OSError:
                return False
        return True

    def _reread(roots):
        return _read_ctrl_map(roots["states"])

    return _pin_get(ctx, "state_control", _resolve, _validate, _reread)


def _r_state_control(ctx: CaptureContext, pc) -> Dict[str, Any]:
    """{state_id: TAG} controller map from the live state-controller crack.

    Folds tag-db-id -> TAG via the tag table. An id that won't resolve is kept as
    a state with controller=None (measured, un-nameable) rather than dropped --
    the state still exists (Codex D). The raw count of unresolved ids is reported
    so provenance/diagnostics can see it without fabricating a TAG.

    STAGE B: the {state_id: controller_tag_db_id} raw map is PINNED so a warmed walk reuses the
    shared state-object roots (state_detail's pin) and re-reads controller@+0xCC LIVE instead
    of re-running the per-state vtable heap scan (fail-closed -- see _pin_controllers_raw).
    """
    if ctx.tag_table is None:
        raise RuntimeError("state_control needs the tag table (unavailable)")
    raw = _pin_controllers_raw(ctx, pc)              # {state_id: ctrl_tag_db_id}
    controllers: Dict[str, Optional[str]] = {}
    unresolved = 0
    for sid, ctrl_id in raw.items():
        tag = ctx.resolve_tag(ctrl_id) if ctrl_id else None
        if ctrl_id and tag is None:
            unresolved += 1
        controllers[str(int(sid))] = tag
    return {"controllers": controllers, "count": len(controllers),
            "unresolved_ids": unresolved}


def _f_province_control():
    from live_layers import province_control as pc  # noqa
    return pc


def _r_province_control(ctx: CaptureContext, pc) -> Dict[str, Any]:
    """{province_id: TAG|None} from the resolved ProvinceControl layer."""
    layer = ctx.province_control
    if layer is None:
        raise RuntimeError("province control layer not resolved")
    pmap = layer.as_map()    # {province_id: TAG_or_None}
    out = {str(int(pid)): tag for pid, tag in pmap.items()}
    nn = sum(1 for v in out.values() if v)
    return {"controllers": out, "count": len(out), "with_controller": nn}


def _f_divisions():
    from division_live.division_live_reader import DivisionLiveReader  # noqa
    return DivisionLiveReader


def _pin_divisions(ctx: CaptureContext, DivisionLiveReader):
    """Resolve-or-REUSE the located CDivision body map ({div_id: body_addr}). On a warmed walk
    the full arena CDivision-vtable scan is SKIPPED: read_divisions_from_bodies re-decodes each
    pinned body LIVE (values never frozen). Each walk opens its OWN DivisionLiveReader handle
    (verified same-process); only INTEGER VAs are cached (proc-fingerprint epoch clears all pins
    on any pid/base change). Invalidation (fail-closed, Codex Q2): validate re-reads EVERY pinned
    body's vtable (==div_vt), type (==4713) AND div_id (the body's stable self-id at +0x0C) --
    a true ABA identity proof; any mismatch or a count change -> drop the pin + re-scan."""
    def _with_reader(fn):
        reader = DivisionLiveReader()
        try:
            _verify_same_process(ctx, reader)        # Codex G: same-process check
            return fn(reader)
        finally:
            try:
                reader.close()
            except Exception:
                pass

    def _resolve():
        def _do(reader):
            bodies = reader.locate_division_bodies()  # ordered {div_id: addr} -- located roots
            if not bodies:
                return None, [], None                 # empty -> do NOT pin (no negative cache)
            data = reader.read_divisions_from_bodies(
                bodies, resolve_owner_tag=ctx.tag_table.tag)
            return bodies, data, len(bodies)
        return _with_reader(_do)

    def _validate(bodies, pop):
        def _do(reader):
            if pop is not None and len(bodies) != pop:
                return False
            import struct as _st
            for did, addr in bodies.items():
                b = reader._read_clamped(addr, 0x10)
                if len(b) < 0x10:
                    return False
                if _st.unpack_from("<Q", b, 0)[0] != reader.div_vt:    # moved/freed/diff class
                    return False
                if _st.unpack_from("<I", b, 0x08)[0] != 4713:          # not a division body
                    return False
                if _st.unpack_from("<I", b, 0x0C)[0] != did:           # ABA: wrong division id
                    return False
            return True
        return _with_reader(_do)

    def _reread(bodies):
        def _do(reader):
            return reader.read_divisions_from_bodies(
                bodies, resolve_owner_tag=ctx.tag_table.tag)
        return _with_reader(_do)

    return _pin_get(ctx, "divisions", _resolve, _validate, _reread)


def _r_divisions(ctx: CaptureContext, DivisionLiveReader) -> Dict[str, Any]:
    """List of division dicts. Owner resolved to TAG via the tag table; an
    unresolved owner keeps the division with owner=None (Codex D), never dropped.
    Opens + closes its own reader handle (the division reader has its own handle
    class).

    STAGE 2: the located CDivision body set is PINNED so a warmed walk skips the arena scan
    (fail-closed -- a moved/freed/realloc'd division body re-scans; see _pin_divisions)."""
    if ctx.tag_table is None:
        raise RuntimeError("divisions need the tag table for owner resolution")
    rows = _pin_divisions(ctx, DivisionLiveReader)
    out = []
    unresolved_owner = 0
    for r in rows:
        owner = r.get("owner")
        if owner == "---" or not owner:
            owner = None
        if owner is None and r.get("owner_tag_db_id"):
            unresolved_owner += 1
        out.append({
            "div_id": r.get("id"),
            "owner": owner,
            "location_province": r.get("location"),
            "moving": r.get("moving"),
            "move_target_province": r.get("move_target_province"),
            "move_progress": r.get("movement_progress"),
            "in_combat": r.get("in_combat"),
            "organisation": r.get("organisation"),
            "strength": r.get("strength"),
            "experience": r.get("experience"),
            "supply_status": r.get("army_current_supply_ratio"),
        })
    return {"divisions": out, "count": len(out),
            "unresolved_owner": unresolved_owner}


def _f_country_scalars():
    from live_layers import country_index_bridge as cib  # noqa
    import locate  # noqa
    return (cib, locate)


def _r_country_scalars(ctx: CaptureContext, impl) -> Dict[str, Any]:
    """Per-TAG politics scalars (stability, war_support, political_power,
    command_power) + num_ships + manpower_pool + convoys, read from the country
    object via the live bridge.

    Offsets within the country object are taken from the economy/owner offset
    drafts. Only fields whose offset is PINNED in the draft are read; anything
    else stays null (no fabrication). A field that reads out of its sane range is
    null, not clamped.

    manpower_pool (country_obj +0x338, u32 raw) and convoys (country_obj +0x1280,
    i32 /1e5 -> integer convoy stockpile count) are BOTH pinned in
    economy_offsets.draft.json and folded by the assembler into
    military.manpower.available and military.convoys_count respectively.
    """
    cib, locate = impl
    bridge = ctx.bridge
    if bridge is None:
        raise RuntimeError("country-index bridge not resolved")
    p = ctx.proc
    offs = ctx.extras.get("economy_offsets") or {}
    fields = offs.get("fields") or {}
    countries: Dict[str, Dict[str, Any]] = {}
    for tag_db_id in bridge.as_map():
        tag = ctx.resolve_tag(tag_db_id)
        if tag is None:
            continue   # un-nameable country: cannot key it by TAG, skip (not fab)
        obj = bridge.country_object(tag_db_id)
        if not obj:
            continue
        rec: Dict[str, Any] = {}
        for fname in ("stability", "war_support", "political_power",
                      "command_power", "num_ships", "manpower_pool", "convoys"):
            spec = fields.get(fname)
            val = _read_country_field(p, obj, spec)
            if val is not None:
                rec[fname] = val
        if rec:
            countries[tag] = rec
    return {"countries": countries, "count": len(countries)}


def _f_state_detail():
    from state_detail_live import StateDetailLive  # noqa
    return StateDetailLive


def _pin_state_detail(ctx: CaptureContext, StateDetailLive):
    """Resolve-or-REUSE the StateDetailLive root map ({state_id: obj_base}). On a warmed
    walk the full per-state vtable heap scan is skipped: we rebuild a live layer from the
    pinned obj_base map and re-read each state's fields LIVE (no frozen data). Invalidation
    (fail-closed): a moved/freed/ABA-reused root is caught by re-reading, at a SENTINEL
    pinned state, the per-state vtable AND the state_id at that obj_base -- both must still
    match (vtable == module_base+STATE_VTABLE_REL AND state_id == the pinned key); plus the
    state COUNT must be unchanged. Any mismatch -> full re-scan + re-pin."""
    import state_detail_live as _sd
    p = ctx.proc

    def _resolve():
        layer = StateDetailLive.resolve(p)
        if layer is None:
            return None, None, None
        # only pin the located roots if the resolved layer EXPOSES them (a real layer);
        # a mock/fake layer without .states is used directly + never pinned.
        roots = None
        pop = None
        states = getattr(layer, "states", None)
        if isinstance(states, dict):
            roots = {"states": dict(states),
                     "catname": dict(getattr(layer, "_catname", {}) or {})}
            pop = len(states)
        return roots, layer, pop

    def _validate(roots, pop):
        states = roots.get("states") or {}
        if pop is not None and len(states) != pop:     # population tripwire
            return False
        vt_expected = p.module_base + _sd.STATE_VTABLE_REL
        keys = sorted(states)
        if not keys:
            return False
        for sid in _sentinel_keys(keys):               # spread sentinels (Codex Q3)
            obj = states[sid]
            if _vt_at(p, obj) != vt_expected:          # moved/freed/different class
                return False
            try:
                if p.u32(obj + _sd.OFF_SID) != sid:    # ABA: same vtable, wrong id
                    return False
            except OSError:
                return False
        return True

    def _reread(roots):
        return StateDetailLive.from_states(p, roots["states"], roots["catname"])

    return _pin_get(ctx, "state_detail", _resolve, _validate, _reread)


def _sentinel_keys(sorted_keys, n=5):
    """Up to n keys spread across a sorted list (first, last, evenly between) for a
    cheap multi-point validation that a single sentinel would miss (Codex Q3)."""
    m = len(sorted_keys)
    if m <= n:
        return list(sorted_keys)
    idxs = sorted({int(round(i * (m - 1) / (n - 1))) for i in range(n)})
    return [sorted_keys[i] for i in idxs]


def _r_state_detail(ctx: CaptureContext, StateDetailLive) -> Dict[str, Any]:
    """Per-STATE live detail: manpower pool {available, locked, total},
    occupation {resistance, compliance}, and state_category. Reads ctx.proc
    READ-ONLY (1081/1081 validated live==save).

    StateDetailLive.resolve(p) fail-closes to None on a build mismatch or an
    incomplete state set; we RAISE on None so the domain is honestly omitted
    (no fabrication). Owner/controller are deliberately NOT emitted here -- the
    state_control plug owns controller (M/S) and owner is left for the save."""
    if ctx.proc is None:
        raise RuntimeError("state_detail needs ctx.proc (unavailable)")
    layer = _pin_state_detail(ctx, StateDetailLive)
    if layer is None:
        raise RuntimeError("state_detail layer not resolved (build mismatch or "
                           "partial state set) -- domain honestly omitted")
    states: Dict[str, Dict[str, Any]] = {}
    for sid, d in layer.as_map().items():
        if not d:
            continue
        states[str(int(sid))] = {
            "manpower_available": d.get("manpower_available"),
            "manpower_locked": d.get("manpower_locked"),
            "manpower_total": d.get("manpower_total"),
            "resistance": d.get("resistance"),
            "compliance": d.get("compliance"),
            "state_category": d.get("state_category"),
        }
    return {"states": states, "count": len(states)}


def _f_state_vp():
    from state_vp_live import StateVPLive  # noqa
    return StateVPLive


def _pin_state_vp(ctx: CaptureContext, StateVPLive):
    """Resolve-or-REUSE the StateVPLive province-record ARRAY root (begin, container). On a
    warmed walk the full container heap scan is skipped: from_roots re-decodes province VP
    LIVE from the pinned begin-array (values are NOT frozen). Invalidation (fail-closed): the
    pinned root must still be the SAME province-record array -- we re-check every VP_ANCHOR
    pointer in the begin-array still (a) resolves to a heap record whose vtable ==
    module_base+RECORD_VTABLE_RVA AND (b) reads its exact anchor VP. This is the SAME anchor
    gate resolve() uses to pin the array, so a moved/freed/rebuilt array fails it -> re-scan."""
    import state_vp_live as _vp
    p = ctx.proc

    def _resolve():
        layer = StateVPLive.resolve(p)
        if layer is None:
            return None, None, None
        # only pin if the resolved layer EXPOSES its array roots (a real layer); a
        # mock/fake layer (no .begin) is used directly + never pinned.
        roots = None
        pop = None
        begin = getattr(layer, "begin", None)
        if begin:
            roots = {"begin": begin, "container": getattr(layer, "container", None),
                     "p2s": getattr(layer, "_p2s", None)}
            pop = dict(_vp.VP_ANCHORS)        # anchor signature (re-verified on reuse)
        return roots, layer, pop

    def _validate(roots, pop):
        begin = roots.get("begin")
        if not begin:
            return False
        vt_expected = p.module_base + _vp.RECORD_VTABLE_RVA
        for pid, want in (pop or _vp.VP_ANCHORS).items():
            try:
                ptr = p.u64(begin + pid * 8)
            except OSError:
                return False
            if not _vp._is_heap(ptr):
                return False
            if _vt_at(p, ptr) != vt_expected:          # moved/freed/different class
                return False
            if _vp.StateVPLive._vp_at(p, ptr) != want:  # ABA / stale: wrong VP value
                return False
        return True

    def _reread(roots):
        # re-decode VP LIVE from the pinned array; None (unreadable array) -> raise via
        # the caller (a None layer is an honest failure, never a fabricated empty world).
        layer = StateVPLive.from_roots(p, roots["begin"], roots["container"],
                                       roots["p2s"])
        if layer is None:
            raise RuntimeError("state_vp: pinned VP array unreadable -- re-resolve")
        return layer

    return _pin_get(ctx, "state_vp", _resolve, _validate, _reread)


def _r_state_vp(ctx: CaptureContext, StateVPLive) -> Dict[str, Any]:
    """Per-STATE live VICTORY POINTS (state_vp_live.py: province VP 1365/1365 EXACT,
    per-state 865/865 EXACT vs the crack-time melt). Reads ctx.proc READ-ONLY.

    StateVPLive.resolve(p) fail-closes to None on a build mismatch or an ambiguous /
    failed relocation; we RAISE on None so the domain is honestly omitted (no
    fabrication) -- never emit a fabricated zero-VP world from a failed read.

    The reader's province/state maps are NONZERO-ONLY. To give the live channel a
    MEASURED ZERO it can protect against a stale save nonzero (Codex review A1), we
    enumerate the FULL state universe from the shipped map_pipeline/assets/state_meta.json
    and emit victory_points_total = live_state_map.get(sid, 0) for EVERY known state --
    a zero-VP state carries a measured 0, not an absence. victory_points_by_province is
    the live per-province VP restricted to that state's provinces (a measured-empty {}
    for a state with no VP-bearing province); it REPLACES (never key-merges) the save's
    map at the fold (the live map is the complete VP truth for a measured state).

    The state asset is REQUIRED for the per-state total enumeration; if it is absent
    (the reader then has no province->state map either, as_state_map() == {}), we RAISE
    so the domain is honestly omitted rather than emitting an unprotectable partial."""
    if ctx.proc is None:
        raise RuntimeError("state_vp needs ctx.proc (unavailable)")
    layer = _pin_state_vp(ctx, StateVPLive)
    if layer is None:
        raise RuntimeError("state_vp layer not resolved (build mismatch or ambiguous/"
                           "failed relocation) -- domain honestly omitted")
    # full state universe (the same asset the reader uses for its province->state map);
    # {state_id: [province_id,...]}. Absent/empty -> we cannot emit a measured zero per
    # state, so raise (no unprotectable partial).
    p2s = _load_state_provinces()
    if not p2s:
        raise RuntimeError("state_vp: state_meta asset unavailable (no full state "
                           "universe to emit measured zeros) -- domain honestly omitted")
    prov_vp = layer.as_province_map()   # {province_id: vp} nonzero-only
    state_vp = layer.as_state_map()     # {state_id: vp_total} nonzero-only
    states: Dict[str, Dict[str, Any]] = {}
    for sid, provs in p2s.items():
        total = int(state_vp.get(sid, 0))   # MEASURED 0 for a zero-VP state
        by_prov = {}
        for pr in provs:
            vp = prov_vp.get(int(pr))
            if vp:                          # nonzero-only; a 0-VP province is omitted
                by_prov[str(int(pr))] = int(vp)
        states[str(int(sid))] = {
            "victory_points_total": total,
            "victory_points_by_province": by_prov,
        }
    return {"states": states, "count": len(states),
            "n_with_vp": sum(1 for s in states.values()
                             if s["victory_points_total"])}


def _load_state_provinces() -> Dict[int, list]:
    """{state_id: [province_id,...]} from the shipped static state metadata (the same
    asset state_vp_live uses). Returns {} if the asset is absent/unreadable (the plug
    then raises -> domain honestly omitted)."""
    import json
    path = os.path.join(_REPO, "map_pipeline", "assets", "state_meta.json")
    if not os.path.isfile(path):
        return {}
    try:
        with open(path, "r", encoding="utf-8") as fh:
            sm = json.load(fh)
    except (OSError, ValueError):
        return {}
    out: Dict[int, list] = {}
    for sid, meta in (sm or {}).items():
        try:
            s = int(sid)
        except (TypeError, ValueError):
            continue
        if isinstance(meta, dict):
            out[s] = list(meta.get("provinces") or [])
    return out


def _f_country_misc():
    from country_misc_live import enumerate_country_misc  # noqa
    return enumerate_country_misc


def _r_country_misc(ctx: CaptureContext, enumerate_country_misc) -> Dict[str, Any]:
    """Per-country live MISC scalars: convoys_destroyed, accidents_score,
    ace_count, cosmetic_tag. REUSES the already-resolved country-index bridge
    (no re-scan, no save): the misc enumerator indexes the country array by
    RUNNING-INDEX and needs (rec0, count, idx_tag={running_index: TAG}).

    The bridge gives {tag_db_id: running_index} (.as_map) + .tag(tag_db_id), so we
    build idx_tag keyed by running_index. rec0/count come straight off the bridge.
    Raises if the bridge is unavailable (domain honestly omitted, no fabrication)."""
    bridge = ctx.bridge
    if bridge is None:
        raise RuntimeError("country_misc needs the country-index bridge (unavailable)")
    # idx_tag must be RUNNING-INDEX keyed (the enumerator reads rec0 + i*0x120).
    idx_tag: Dict[int, str] = {}
    for tag_db_id, running_index in bridge.as_map().items():
        tag = ctx.resolve_tag(tag_db_id)
        if tag is not None and running_index is not None:
            idx_tag[int(running_index)] = tag
    # An empty idx_tag means identity mapping FAILED (no tag resolved) on a
    # non-empty bridge -- that is a broken read, NOT a measured-empty domain.
    # Raise so the domain is honestly omitted rather than claimed captured-empty
    # (Codex review #3). A genuinely empty bridge already raised above.
    if not idx_tag:
        raise RuntimeError("country_misc: no running_index resolved to a TAG "
                           "(identity mapping failed) -- domain honestly omitted")
    countries: Dict[str, Dict[str, Any]] = {}
    for tag, _idx, misc in enumerate_country_misc(
            ctx.proc, bridge.rec0, bridge.count, idx_tag):
        if not tag:
            continue
        countries[tag] = {
            "convoys_destroyed": misc.get("convoys_destroyed"),
            "accidents_score": misc.get("accidents_score"),
            "ace_count": misc.get("ace_count"),
            "cosmetic_tag": misc.get("cosmetic_tag"),
        }
    return {"countries": countries, "count": len(countries)}


def _f_country_static():
    from country_static_live import read_country_static  # noqa
    return read_country_static


def _r_country_static(ctx: CaptureContext, read_country_static) -> Dict[str, Any]:
    """LIVE per-country STATIC scalars -> country.identity.{original_capital,
    original_tag, reserved_dynamic_country, templates_locked} (M; distinct
    live-only leaves). Reuses the resolved bridge's country_object pointer (no
    re-scan): for each tag, obj = bridge.country_object(tag_db_id); the reader
    reads object-relative offsets (cracked + validated 140/140 vs the
    content-matched melt). original_tag's tag-db-id is resolved -> TAG via the
    live tag table; absent (id 0) -> None (no fabrication). Raises if the bridge
    is unavailable (domain honestly omitted)."""
    bridge = ctx.bridge
    if bridge is None:
        raise RuntimeError("country_static needs the country-index bridge (unavailable)")
    p = ctx.proc
    countries: Dict[str, Dict[str, Any]] = {}
    for tag_db_id in bridge.as_map():
        tag = ctx.resolve_tag(tag_db_id)
        if tag is None:
            continue
        obj = bridge.country_object(tag_db_id)
        if not obj:
            continue
        rec = read_country_static(p, obj, tag_table=ctx.tag_table)
        countries[tag] = {
            "original_capital": rec.get("original_capital"),
            "original_tag": rec.get("original_tag"),
            "reserved_dynamic_country": rec.get("reserved_dynamic_country"),
            "templates_locked": rec.get("templates_locked"),
        }
    if not countries:
        raise RuntimeError("country_static: no country resolved -- domain honestly omitted")
    return {"countries": countries, "count": len(countries)}


def _f_country_variables():
    from variables_live import read_variables_tristate  # noqa
    return read_variables_tristate


def _r_country_variables(ctx: CaptureContext, read_variables_tristate) -> Dict[str, Any]:
    """LIVE per-country SCRIPT-VARIABLES bag -> country.script_variables (M;
    the mass closure for the nation scripted-GUI overlays). Reuses the resolved
    bridge's country_object pointer. The reader walks the CVariables container
    (country_obj+0x218) with TRI-STATE semantics (Codex wave-1 fix): a country is
    emitted ONLY when the container was actually MEASURED -- a real, complete bag
    (key-count == size header; validated 2855/2855 across 35 countries) OR a real
    measured-empty {} (container size == 0). A country whose container could NOT
    be verified (RTTI miss / torn read / ambiguous parse) is UNMEASURED and OMITTED
    (its leaf stays null -- never a fabricated {}). Raises if the bridge is
    unavailable."""
    bridge = ctx.bridge
    if bridge is None:
        raise RuntimeError("country_variables needs the country-index bridge (unavailable)")
    p = ctx.proc
    countries: Dict[str, Dict[str, Any]] = {}
    nonempty = 0
    for tag_db_id in bridge.as_map():
        tag = ctx.resolve_tag(tag_db_id)
        if tag is None:
            continue
        obj = bridge.country_object(tag_db_id)
        if not obj:
            continue
        bag, measured = read_variables_tristate(p, obj, strict=True)
        if not measured:
            continue                  # UNMEASURED -> omit (leaf stays null, no fabrication)
        countries[tag] = dict(bag)    # measured (a real {} when size == 0 is kept)
        if bag:
            nonempty += 1
    if not countries:
        raise RuntimeError("country_variables: no country measured -- domain honestly omitted")
    return {"countries": countries, "count": len(countries),
            "countries_with_variables": nonempty}


def _f_country_containers():
    import country_containers_live as ccl  # noqa
    return ccl


def _r_country_containers(ctx: CaptureContext, ccl) -> Dict[str, Any]:
    """LIVE per-country CONTAINER fields -> country.identity.{cores, claims} +
    country.decisions.status_present + country.identity.naval_headquarter_status +
    country.country_reports (M; validated 63/63 cores, 9/9 claims, 29/29
    decision-status set, 5/5 naval-HQ set, 140/140 report scalars).

    cores/claims are INVERTED from the per-state core/claim country-id vectors
    (a state lists every country that cores/claims it), so this plug enumerates the
    live STATE objects (state_detail_live relocator) once and inverts; the country
    submanagers are read via the resolved bridge's country_object pointer. All reads
    are fail-closed (a country/state that won't read is omitted, never fabricated).
    Raises if the bridge is unavailable."""
    bridge = ctx.bridge
    if bridge is None:
        raise RuntimeError("country_containers needs the country-index bridge (unavailable)")
    if ctx.tag_table is None:
        raise RuntimeError("country_containers needs the tag table (unavailable)")
    p = ctx.proc
    # cores/claims: invert from the live state objects. Reuse the SHARED capture-wide
    # state-object root map (pinned by state_detail) instead of re-scanning the heap.
    state_objs = _shared_state_objs(ctx)
    try:
        tmap = ctx.tag_table.as_map()
    except Exception:
        tmap = {}
    cc_map = ccl.enumerate_cores_claims(p, state_objs, tmap) if state_objs and tmap else {}
    # per-country submanagers
    countries: Dict[str, Dict[str, Any]] = {}
    for tag_db_id in bridge.as_map():
        tag = ctx.resolve_tag(tag_db_id)
        if tag is None:
            continue
        obj = bridge.country_object(tag_db_id)
        if not obj:
            continue
        sm = ccl.read_country_submanagers(p, obj)
        ccrec = cc_map.get(tag, {})
        countries[tag] = {
            "cores": list(ccrec.get("cores") or []),
            "claims": list(ccrec.get("claims") or []),
            "decision_status_present": sm.get("decision_status"),
            "naval_headquarter_status": sm.get("naval_headquarter_status"),
            "country_reports": sm.get("country_reports"),
        }
    if not countries:
        raise RuntimeError("country_containers: no country resolved -- domain honestly omitted")
    return {"countries": countries, "count": len(countries)}


def _f_state_fields():
    from state_fields_live import enumerate_state_fields  # noqa
    return enumerate_state_fields


def _r_state_fields(ctx: CaptureContext, enumerate_state_fields) -> Dict[str, Any]:
    """LIVE per-state extra fields -> states.*.{resources, dynamic_modifier,
    extra_shared_slots} (M; shares the save home for resources, distinct
    live-only leaves for the others). Uses the existing state-object relocator
    (state_detail_live). Each leaf is read from a REAL located container (an
    empty list/dict is a measured empty, never a hard-coded one). The
    is_border_conflict candidate byte is read into a diagnostic field only (it is
    UNCONFIRMED in this all-false peacetime frame -- not folded as a data value).
    Reuses ctx.proc + the SHARED capture-wide state-object root map (pinned by
    state_detail) so the state-vtable heap scan is not repeated."""
    p = ctx.proc
    sfields = enumerate_state_fields(p, state_objs=_shared_state_objs(ctx) or None)
    if not sfields:
        raise RuntimeError("state_fields: no state read -- domain honestly omitted")
    states: Dict[str, Dict[str, Any]] = {}
    for sid, rec in sfields.items():
        states[str(int(sid))] = {
            "resources": dict(rec.get("resources") or {}),
            "dynamic_modifier": list(rec.get("dynamic_modifier") or []),
            "extra_shared_slots": rec.get("extra_shared_slots"),
        }
    return {"states": states, "count": len(states)}


def _f_designs():
    from designs_live import DesignsLiveReader  # noqa
    return DesignsLiveReader


def _pin_designs(ctx: CaptureContext, DesignsLiveReader):
    """Resolve-or-REUSE the located template body list ([(body_addr, template_id)]). On a warmed
    walk the full arena template-vtable scan is SKIPPED: read_templates_from_bodies re-decodes each
    pinned body LIVE. Each walk opens its OWN DesignsLiveReader handle (verified same-process +
    build-gated); only INTEGER VAs are cached. Invalidation (fail-closed): validate re-reads EVERY
    pinned body's vtable (==tmpl_vt), type (==52) AND template id (stable self-id at +0x0C) -- a
    true ABA identity proof; any mismatch or count change -> drop the pin + re-scan."""
    import designs_live as _dl
    def _with_reader(fn):
        reader = DesignsLiveReader()
        try:
            _verify_same_process(ctx, reader)
            ok, _sha = reader.verify_build()
            if not ok:
                raise RuntimeError("designs reader build gate failed (unverified build)")
            return fn(reader)
        finally:
            try:
                reader.close()
            except Exception:
                pass

    def _resolve():
        def _do(reader):
            bodies = reader.locate_template_bodies()   # [(addr, id)] ordered -- located roots
            if not bodies:
                return None, [], None                  # empty -> do NOT pin (no negative cache)
            return bodies, reader.read_templates_from_bodies(bodies), len(bodies)
        return _with_reader(_do)

    def _validate(bodies, pop):
        def _do(reader):
            if pop is not None and len(bodies) != pop:
                return False
            import struct as _st
            for cand, tid in bodies:
                b = reader._read_clamped(cand, 0x10)
                if len(b) < 0x10:
                    return False
                if _st.unpack_from("<Q", b, 0)[0] != reader.tmpl_vt:    # moved/freed/diff class
                    return False
                if _st.unpack_from("<I", b, _dl.OFF_TYPE)[0] != _dl.TEMPLATE_TYPE:
                    return False
                if _st.unpack_from("<I", b, _dl.OFF_ID)[0] != tid:      # ABA: wrong template id
                    return False
            return True
        return _with_reader(_do)

    def _reread(bodies):
        return _with_reader(lambda reader: reader.read_templates_from_bodies(bodies))

    return _pin_get(ctx, "designs", _resolve, _validate, _reread)


def _r_designs(ctx: CaptureContext, DesignsLiveReader) -> Dict[str, Any]:
    """LIVE division TEMPLATES (649/649 validated). Opens its OWN ctypes handle
    (the designs reader is self-contained), verified to be the SAME process as the
    capture context before use (Codex G). The equipment-variant CATALOG is NOT yet
    exposed by this reader, so ONLY templates are emitted here (see the open ticket
    in the deliverable note); we never fabricate an empty variant catalog.

    STAGE 2: the located template body set is PINNED so a warmed walk skips the arena scan
    (fail-closed -- a moved/freed/realloc'd template body re-scans; see _pin_designs)."""
    templates = _pin_designs(ctx, DesignsLiveReader)
    return {"division_templates": list(templates or []),
            "count": len(templates or []),
            "variants_available": False}


def _f_mio():
    from mio_live import enumerate_by_tag, enumerate_mios  # noqa
    return (enumerate_by_tag, enumerate_mios)


def _pin_mio(ctx: CaptureContext, enumerate_mios):
    """Resolve-or-REUSE the MIO record-address pool ({org_id: record_base}). On a warmed walk
    the full record-vtable heap scan is skipped: read_mios_from_addrs re-reads each pinned
    record LIVE and re-gates it (record vtable at +VT_OFF, type==79, id==pinned key -> ABA
    defense). Invalidation (fail-closed): validate re-checks spread SENTINEL records' vtable +
    id and the pool COUNT; any mismatch -> full re-scan + re-pin. The per-record re-gate in
    read_mios_from_addrs is the second line of defense (a single stale record is dropped, not
    fabricated)."""
    import mio_live as _ml
    p = ctx.proc

    def _resolve():
        raw = enumerate_mios(p)               # {org_id: record} (record has 'addr')
        if not raw:
            return None, {}, None
        addrs = {oid: rec["addr"] for oid, rec in raw.items() if rec.get("addr")}
        if not addrs:
            # records with no addr (e.g. a mock enum) -> use them directly, do not pin.
            return None, raw, None
        return addrs, raw, len(addrs)

    def _validate(addrs, pop):
        if pop is not None and len(addrs) != pop:
            return False
        vt = p.module_base + _ml.VT_RVA
        for oid in _sentinel_keys(sorted(addrs)):
            base = addrs[oid]
            try:
                if p.u64(base + _ml.VT_OFF) != vt:     # moved/freed/different class
                    return False
                if p.u32(base + _ml.OFF_TYPE) != _ml.MIO_TYPE:
                    return False
                if p.u32(base + _ml.OFF_ID) != oid:    # ABA: same vtable, wrong id
                    return False
            except OSError:
                return False
        return True

    def _reread(addrs):
        return _ml.read_mios_from_addrs(p, addrs)

    return _pin_get(ctx, "mio", _resolve, _validate, _reread)


def _r_mio(ctx: CaptureContext, impl) -> Dict[str, Any]:
    """LIVE MIO catalog (1210/1210 validated). Reads ctx.proc READ-ONLY and groups
    by TAG via the live tag table. Requires the tag table to name owners (else org
    records would carry only a raw tag-db-id); raises if it is unavailable so the
    domain is honestly omitted (no fabrication). Each record also keeps its raw
    owner_tag_db_id alongside the resolved owner TAG."""
    enumerate_by_tag, enumerate_mios = impl
    if ctx.tag_table is None:
        raise RuntimeError("mio needs the tag table to name owners (unavailable)")
    raw = _pin_mio(ctx, enumerate_mios)       # {org_id: record} (warmed walk: no heap scan)
    # OPTIONAL allowed-policy NAME resolution (import-guarded, per-capture cached, fail-soft):
    # the MIO record carries only allowed_policies_count; the actual u32 policy HANDLE array
    # lives at the record's heap addr (MIO+0x168), so defnames_live.resolve_mio_policies_live
    # reads it and decodes each handle to its ICON token. We resolve here using the raw
    # record's `addr` BEFORE it is stripped, and ADD a `policies` list (icon tokens, in save
    # order) ALONGSIDE the count. A handle the registry does not know -> dropped (no fake).
    # Disabled by default; a missing/broken defnames leaves `policies` absent (count survives).
    _pol_reg = _defnames_registry(ctx, "mio_policy") if _defnames_enabled(ctx) else None
    _dn = _defnames_module() if _pol_reg else None
    by_id: Dict[str, Any] = {}
    for oid, rec in (raw or {}).items():
        owner_id = rec.get("owner_tag_db_id")
        owner = ctx.resolve_tag(owner_id)
        # resolve policy tokens from the raw record's heap addr BEFORE stripping it.
        policies = None
        if _dn is not None and rec.get("addr"):
            try:
                policies = _dn.resolve_mio_policies_live(ctx.proc, rec["addr"], _pol_reg)
            except Exception:
                policies = None
        rrec = dict(rec)
        # Strip the diagnostic-only fields that must NEVER reach persisted output:
        # the heap addr, and the RAW numeric tag-db-id (a numeric id must not
        # masquerade as identity, Codex review #5). The resolved TAG lives in
        # "owner" (or None for an un-nameable owner).
        rrec.pop("addr", None)
        rrec.pop("owner_tag_db_id", None)
        rrec["owner"] = owner                # TAG or None (un-nameable owner)
        # additive resolved policy icon tokens. allowed_policies_count is the AUTHORITATIVE
        # measured count (always kept); `policies` is a NAMES enrichment only. To avoid
        # understating the real policy set, emit the resolved list ONLY when it is COMPLETE:
        # a genuine measured-empty (count == 0) OR every allowed policy resolved
        # (len(policies) == allowed_policies_count). A count>0 that resolved to a SHORTER list
        # (some handles unknown this frame) is an INCOMPLETE resolution -> omit `policies` (the
        # count still conveys the real number; no fabricated/understated set -- Codex review #1).
        if isinstance(policies, list):
            cnt = rrec.get("allowed_policies_count")
            if cnt == 0 or (isinstance(cnt, int) and len(policies) == cnt):
                rrec["policies"] = list(policies)
        by_id[str(int(oid))] = rrec
    by_tag: Dict[str, Any] = {}
    for rec in by_id.values():
        owner = rec.get("owner")
        if owner is None:
            continue                          # un-nameable owner: cannot key by TAG
        by_tag.setdefault(owner, []).append(rec)
    for t in by_tag:
        by_tag[t].sort(key=lambda r: r.get("id", 0))
    return {"by_tag": by_tag, "by_id": by_id, "count": len(by_id)}


def _f_tech_live():
    import tech_live as _tl  # noqa
    return _tl


def _pin_tech(ctx: CaptureContext, tl, bridge_idx_tag, rec0, count):
    """Resolve-or-REUSE the located CTechnologyStatus + CResearchSlot object addresses. On a warmed
    walk the TWO full-arena vtable sweeps (status + slot -- the ~50s cold cost) are SKIPPED:
    read_country_tech_from_addrs re-reads every field LIVE off the pinned addrs (the country bridge
    is REBUILT cheaply each walk from the supplied rec0/count, so only the heap-swept object
    addresses are cached). Invalidation (fail-closed, Codex-vetted): validate_tech_addrs re-checks
    EVERY pinned status addr (vtable==STATUS_VT + stable country-link id + header-stability sentinel)
    and every slot addr (vtable==SLOT_VT + (owner_status_ptr, slot_index) stable id) == the locate-
    time snapshot; any mismatch / empty status pool -> drop the pin + full re-scan.

    The bridge/rec0/count differ per walk only in cheap recompute; they are passed straight to the
    decode (NOT part of the pinned roots), so a stale country bridge can never be served."""
    p = ctx.proc

    def _decode(addrs):
        return tl.read_country_tech_from_addrs(
            p, addrs["status"], addrs["slot"], bridge=bridge_idx_tag,
            strict=True, rec0=rec0, count=count)

    def _resolve():
        ta = tl.enumerate_tech_addrs(p)
        status_addrs = ta.get("status_addrs") or []
        slot_addrs = ta.get("slot_addrs") or []
        if not status_addrs:
            return None, {}, None                 # nothing located -> do NOT pin
        roots = {"status": status_addrs, "slot": slot_addrs}
        return roots, _decode(roots), len(status_addrs)

    def _validate(roots, pop):
        return bool(tl.validate_tech_addrs(p, roots["status"], roots["slot"]))

    def _reread(roots):
        return _decode(roots)

    return _pin_get(ctx, "tech", _resolve, _validate, _reread)


def _r_tech_live(ctx: CaptureContext, tl) -> Dict[str, Any]:
    """LIVE per-country research/tech state (tech_live.py: completed_tech_count
    43/43 EXACT, research_slot_count 45/46 EXACT vs the lockstep save). Reads
    ctx.proc READ-ONLY and reuses the already-resolved country-index bridge to
    name countries (its {running_index: TAG} form is what the reader expects).

    The reader's keys are already TAGs; we keep ONLY schema-valid TAG keys (a key
    that is not a real TAG is dropped, never written to a *_tag-adjacent home).
    The diagnostic heap addr (`_status_addr`) is stripped before the data enters
    the snapshot. RESISTED: tech NAMES (registry handle) + progress SCALE -- we
    carry the raw int handles + progress_raw only, never a fabricated name/value.

    STAGE 1.5: the located CTechnologyStatus + CResearchSlot object set is PINNED (see _pin_tech)
    so a warmed walk SKIPS the two arena vtable sweeps and re-reads tech LIVE off the pinned
    objects (fail-closed -- a moved/freed/realloc'd object re-scans). The country bridge is
    rebuilt cheaply each walk (not pinned), so a stale bridge is never served.

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("tech_live needs ctx.proc (unavailable)")
    # reuse the shared bridge for naming AND for the live country-array base (rec0/count) so the
    # reader SKIPS its own _locate_country_array (removes a redundant relocate AND the stale-PP
    # relocate that used to kill the walk -- see _bridge_idx_tag). None -> reader self-locates.
    bridge_idx_tag, _rec0, _count = _bridge_idx_tag(ctx)
    raw = _pin_tech(ctx, tl, bridge_idx_tag, _rec0, _count)
    # OPTIONAL name resolution (import-guarded, per-capture cached, fail-soft): resolve each
    # in-progress slot's raw tech handle/index to its tech TOKEN and ADD it as `tech_name`
    # ALONGSIDE the raw handles (additive; the raw handles always survive). A handle the
    # registry does not know -> NO tech_name (never a fabricated/synthetic name). Disabled
    # by default; the raw-handle shape is unchanged when resolution is off (Codex).
    _tech_reg = _defnames_registry(ctx, "tech") if _defnames_enabled(ctx) else None
    _dn = _defnames_module() if _tech_reg else None

    def _resolve_tech_name(slot):
        if not _tech_reg or _dn is None:
            return None
        # the registry is keyed by the tech-def handle; the in-progress slot's
        # tech_def_index_raw is that def index (matches defnames' researched-vector decode).
        # Fall back to tech_handle_raw if the def index is absent/unknown. Unknown -> None.
        for key in ("tech_def_index_raw", "tech_handle_raw"):
            h = slot.get(key)
            if h is None:
                continue
            try:
                tok = _dn.resolve_tech(ctx.proc, h, reg=_tech_reg)
            except Exception:
                tok = None
            if tok:
                return tok
        return None

    countries: Dict[str, Any] = {}
    for tag, rec in (raw or {}).items():
        if not schema.valid_tag(tag):
            continue            # not a real TAG -> drop (no numeric/name masquerade)
        # PRESERVE the None/measured distinction (Codex final review): a missing/None
        # active_research is UNMEASURED -> stays None (the assembler then leaves the
        # in_progress_live leaf null so the save can fill it); a measured [] (the
        # reader's default per country) is a measured-empty list and is kept as [].
        active_raw = rec.get("active_research")
        active = None
        if active_raw is not None:
            active = []
            for a in active_raw:
                if not isinstance(a, dict):
                    continue
                slot = {
                    "slot_index": a.get("slot_index"),
                    "tech_handle_raw": a.get("tech_handle_raw"),
                    "tech_def_index_raw": a.get("tech_def_index_raw"),
                    "progress_raw": a.get("progress_raw"),
                }
                # additive resolved token (only when known); never overwrites a raw handle.
                tn = _resolve_tech_name(a)
                if tn is not None:
                    slot["tech_name"] = tn
                active.append(slot)
        countries[tag] = {
            "completed_tech_count": rec.get("completed_tech_count"),
            "research_slot_count": rec.get("research_slot_count"),
            "active_research": active,   # raw-handle shape (NOT the save's in_progress)
        }
    return {"countries": countries, "count": len(countries)}


def _f_commanders_live():
    from characters_live import CharactersLiveReader  # noqa
    return CharactersLiveReader


def _pin_commanders(ctx: CaptureContext, CharactersLiveReader):
    """Resolve-or-REUSE the located leader/character body ADDRESSES. On a warmed walk the four
    full-arena vtable sweeps (find_instances x4 -- the ~96s cold locate) are SKIPPED: only the
    LOCATED body addresses + their ABA snapshot are cached, NEVER the decoded values. A warmed
    walk re-reads EVERY leader field LIVE off the pinned addrs via enumerate_from_addrs (the same
    address-only contract as _pin_divisions / _pin_division_equipment).

    Roots = {"addrs": addr_sets}.

    WHY ADDRESS-ONLY, NOT CACHED-DECODE (no-stale-data fix, Codex-confirmed): the per-leader
    decode emits MUTABLE inline scalars -- experience (+0xE68), the 4 category skills
    (+0xF58/+0xF68/+0xF78/+0xF88), and the per-character fields -- that the body-IDENTITY validate
    (validate_addrs: vtable + type + +0x0C id + +0x08 CCharacter ptr + the trait flat_map content
    fp) does NOT cover. Caching the decoded enumeration and serving it on a warm hit would return a
    STALE experience/skill snapshot the instant the paused invariant leaks (a commander gains XP on
    an unpaused tick, or an in-place edit while "paused"). validate_addrs proves "same body object",
    NOT "same decoded commander state" -- so we re-decode live every walk and never serve stale
    scalars. (The trait content fp in validate_addrs is now belt-and-braces for the address-only
    re-read; correctness no longer depends on it.) This restores the big win -- skipping the ~96s
    four-arena SWEEP -- while paying only the per-leader decode, exactly like every other address-
    only pin.

    Each walk opens its OWN CharactersLiveReader handle (verified same-process + build-gated); only
    INTEGER body VAs (in addr_sets) are cached (the proc-fingerprint epoch in _pin_get clears all
    pins on any pid/create-time/sha/base change). Invalidation (fail-closed): validate_addrs re-reads
    EVERY pinned body's vtable + type + stable id (+0x0C leader/char id; +0x08 CCharacter ptr for
    country_leader) and confirms each == the locate-time snapshot (a true ABA identity proof); any
    mismatch -> drop the pin + re-scan + re-decode. Returns the decoded enumerate() dict (the SAME
    shape the cold path produced)."""
    def _with_reader(fn):
        reader = CharactersLiveReader()
        try:
            _verify_same_process(ctx, reader)
            ok, _sha = reader.verify_build()
            if not ok:
                raise RuntimeError("commanders reader build gate failed (unverified build)")
            return fn(reader)
        finally:
            closer = getattr(reader, "close", None)
            if callable(closer):
                try:
                    closer()
                except Exception:
                    pass

    def _decode(addr_sets):
        def _do(reader):
            return reader.enumerate_from_addrs(addr_sets, resolve_country=True,
                                               read_traits=True)
        return _with_reader(_do)

    def _resolve():
        def _do(reader):
            addr_sets = reader.enumerate_addrs()
            n = (len(addr_sets.get("army") or []) + len(addr_sets.get("navy") or [])
                 + len(addr_sets.get("char") or []))
            if n == 0:
                return None, {}, None              # nothing located -> do NOT pin
            data = reader.enumerate_from_addrs(addr_sets, resolve_country=True,
                                               read_traits=True)
            return {"addrs": addr_sets}, data, n
        return _with_reader(_do)

    def _validate(roots, pop):
        def _do(reader):
            return bool(reader.validate_addrs(roots["addrs"]))
        return _with_reader(_do)

    def _reread(roots):
        # validate_addrs passed -> every pinned body is the SAME live object (no move/free/ABA);
        # re-decode the leader fields LIVE off those pinned addrs so a mutable scalar that drifted
        # (experience/skill) is read fresh -- never a stale cached snapshot. The ~96s arena sweep
        # is still skipped (addresses are pinned); only the per-leader decode is paid.
        return _decode(roots["addrs"])

    return _pin_get(ctx, "commanders", _resolve, _validate, _reread)


def _r_commanders_live(ctx: CaptureContext, CharactersLiveReader) -> Dict[str, Any]:
    """LIVE unit-leaders grouped by country TAG (characters_live.py: army leaders
    770/770, navy 183/183, per-leader skills + experience validated 100% vs save).
    Opens its OWN read-only handle (the reader owns a ctypes handle), verified to be
    the SAME process as the capture context before use (Codex G).

    The reader CANNOT split generals vs field-marshals live (the role split needs the
    RESISTED overall level), so army leaders are emitted UNSPLIT under army_leaders
    and admirals under navy_leaders -- never mapped onto the save's role-split leaves.
    Each leader's country is resolved to a TAG; a leader with no resolvable TAG is
    dropped (cannot be keyed by country). The diagnostic heap addr (`addr`) is
    stripped. RESISTED: overall LEVEL (left as the reader's null) + trait NAMES (only
    trait_xp ids carried) -- never fabricated.

    A country with NO leader rows in the global enumeration is simply absent from
    army_by_tag/navy_by_tag -> the assembler leaves that country's army_leaders /
    navy_leaders leaf NULL (unmeasured), NOT [] (Codex final review): the reader
    enumerates leaders globally, so "no rows for this tag" is genuinely ambiguous
    (vs a positively-measured zero), and asserting [] would be fabrication. The
    per-tag COUNT (commanders.counts.{army,navy}_leaders) does carry the measured 0.

    Raises if the reader's build gate fails or the read raises (domain honestly
    omitted, no partial fold).

    STAGE 1.5: the located leader/character body set is PINNED (see _pin_commanders) so a warmed
    walk SKIPS the four arena vtable sweeps and re-reads the leaders LIVE off the pinned bodies
    (fail-closed -- a moved/freed/realloc'd body re-scans)."""
    out = _pin_commanders(ctx, CharactersLiveReader)
    # OPT-IN leader DETAIL enrichment (import-guarded, fail-soft): join overall SKILL +
    # assigned-TRAIT token set onto each leader by unit_leader_id (Codex B1 -- the join lives
    # in the plug, where the raw unit_leader_id is available). The read is a SECOND full
    # leader-pool heap scan == EXPENSIVE, so it is OFF by default and runs ONLY when the caller
    # sets ctx.extras["resolve_leader_detail"] (mirrors the resolve_defnames opt-in cost gate).
    # A missing/broken reader or any read failure leaves the leaders WITHOUT skill/traits (the
    # overall LEVEL stays RESISTED) -- never a crash, never a fabricated 0 skill / [] traits.
    detail_map, detail_ran = _leader_detail_map(ctx)
    def _clean_leader(r):
        r = dict(r)
        r.pop("addr", None)       # diagnostic heap addr -> never persisted
        r.pop("_unpinned", None)  # reader-internal "which fields are RESISTED" marker
        # additive skill/traits join (only when the opt-in enrichment ran AND measured this
        # unit_leader_id). A measured-empty traits [] is a real "no traits" and is kept; an
        # unknown id simply gets no skill/traits keys (no fabrication).
        if detail_ran:
            d = detail_map.get(r.get("unit_leader_id"))
            if isinstance(d, dict):
                if d.get("skill") is not None:
                    r["skill"] = d["skill"]
                if isinstance(d.get("traits"), list):
                    r["traits"] = list(d["traits"])
        return r
    army_by_tag: Dict[str, list] = {}
    navy_by_tag: Dict[str, list] = {}
    counts_by_tag: Dict[str, Dict[str, int]] = {}
    char_counts: Dict[str, int] = {}
    # count characters per resolved TAG (for commanders.counts.characters)
    for c in (out.get("characters") or {}).values():
        tag = c.get("country")
        if schema.valid_tag(tag):
            char_counts[tag] = char_counts.get(tag, 0) + 1
    for lst, bucket in ((out.get("army_leaders") or [], army_by_tag),
                        (out.get("navy_leaders") or [], navy_by_tag)):
        for r in lst:
            tag = r.get("country")
            if not schema.valid_tag(tag):
                continue          # un-nameable country: cannot key by TAG -> drop
            bucket.setdefault(tag, []).append(_clean_leader(r))
    for tag in set(army_by_tag) | set(navy_by_tag) | set(char_counts):
        counts_by_tag[tag] = {
            "army_leaders": len(army_by_tag.get(tag, [])),
            "navy_leaders": len(navy_by_tag.get(tag, [])),
            "characters": char_counts.get(tag, 0),
        }
    return {"army_by_tag": army_by_tag, "navy_by_tag": navy_by_tag,
            "counts_by_tag": counts_by_tag,
            # honest flag: True only when the OPT-IN leader_detail enrichment ran to completion
            # (the assembler lists the "leader_detail" provenance id iff this is True). A failed/
            # disabled enrichment leaves it False -> the domain is NOT claimed (no fabrication).
            "leader_detail_enriched": bool(detail_ran),
            "count": len(set(army_by_tag) | set(navy_by_tag))}


def _f_intel_live():
    import intel_live  # noqa
    return intel_live


def _r_intel_live(ctx: CaptureContext, intel_live) -> Dict[str, Any]:
    """LIVE per-country intelligence-agency scalars (intel_live.py: name 13/14,
    upgrade_progress 14/14, max_operative_count 43/44, defense 44/44 EXACT vs save).
    Reads ctx.proc READ-ONLY.

    intel_live.read_all keys by TAG when its country bridge resolves, but FALLS BACK
    to localized country NAMES when the bridge fails -- those are NOT TAGs, so we
    REJECT any key that is not schema-valid (Codex: never let a localized name create
    a non-TAG country). The diagnostic heap addr (`agency_ptr`) is stripped. RESISTED:
    per-operative XP + operative enumeration -- only the agency scalars + capacity
    counts are carried, never fabricated per-operative data.

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("intel_live needs ctx.proc (unavailable)")
    # Cross-check keys against the bridge's KNOWN TAG set when available (Codex final
    # review): schema.valid_tag alone would pass a 3-letter localized country NAME
    # (the reader's fallback key when its own bridge fails). Requiring the key to be a
    # TAG the resolved bridge actually knows closes that no-fabrication hole. With no
    # bridge we fall back to valid_tag only (still rejects multi-word localized names).
    known_tags = None
    if ctx.bridge is not None:
        try:
            known_tags = set()
            for tag_db_id in ctx.bridge.as_map():
                t = ctx.resolve_tag(tag_db_id)
                if schema.valid_tag(t):
                    known_tags.add(t)
        except Exception:
            known_tags = None
    # reuse the already-resolved country-index bridge (ctx.bridge) so the reader skips its
    # own per-walk country-array re-resolve scan (steady-state perf; None -> reader self-resolves).
    raw = intel_live.read_all(ctx.proc, bridge=ctx.bridge)
    countries: Dict[str, Any] = {}
    for key, ag in (raw or {}).items():
        if not schema.valid_tag(key):
            continue            # localized-name fallback key (or non-TAG) -> drop
        if known_tags is not None and key not in known_tags:
            continue            # 3-letter string the bridge doesn't know -> drop
        if not isinstance(ag, dict):
            continue
        countries[key] = {
            "name": ag.get("name"),
            "icon": ag.get("icon"),
            "upgrade_progress": ag.get("upgrade_progress"),
            "max_operative_count": ag.get("max_operative_count"),
            "usable_operative_slots": ag.get("usable_operative_slots"),
            "defense": ag.get("defense"),
            # agency_ptr is a diagnostic heap addr -> deliberately NOT carried.
        }
    return {"countries": countries, "count": len(countries)}


def _f_focus_live():
    from focus_live import read_country_focus  # noqa
    return read_country_focus


def _r_focus_live(ctx: CaptureContext, read_country_focus) -> Dict[str, Any]:
    """LIVE per-country NATIONAL-FOCUS state (focus_live.py: focus_tree 20/20 EXACT,
    completed set+order 22/22 EXACT, completed_count 22/22, current 9/9,
    current_continuous 13/13, progress_days 19/19 EXACT vs the lockstep save). Reads
    ctx.proc READ-ONLY and reuses the already-resolved country-index bridge to name
    countries (its {running_index: TAG} form is what the reader expects), so it does
    not re-scan / re-bridge against the save PP Rosetta.

    Folds onto the SAME canonical save HOME (country.focus) under the DISTINCT
    "focus_live" provenance id. The reader's keys are already TAGs; we keep ONLY
    schema-valid TAG keys (a non-TAG key is dropped, never written near identity). The
    diagnostic submanager heap addr (`_submgr_addr`) is stripped before the data enters
    the snapshot (mirrors the projects_live/_addr convention). include_candidate=False:
    the UNVALIDATED available_focuses_candidate vector is deliberately NOT carried.

    RESISTED (carried as the reader's null, never fabricated): `paused` (no paused=yes
    ground truth this frame -> the live byte is unpinned); the completion FRACTION
    (serialize/UI-computed, not stored). progress is carried in DAYS (the stored input,
    same unit as the save `progress`). Raises if ctx.proc is unavailable (offline ->
    domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("focus_live needs ctx.proc (unavailable)")
    # reuse the shared bridge for naming AND for the live country-array base (rec0/count) so the
    # reader SKIPS its own _locate_country_array (removes a redundant relocate AND the stale-PP
    # relocate that used to kill the walk -- see _bridge_idx_tag). None -> reader self-locates.
    bridge_idx_tag, _rec0, _count = _bridge_idx_tag(ctx)
    raw = read_country_focus(ctx.proc, bridge=bridge_idx_tag,
                             include_candidate=False, strict=True,
                             rec0=_rec0, count=_count)
    countries: Dict[str, Any] = {}
    for tag, rec in (raw or {}).items():
        if not schema.valid_tag(tag):
            continue            # not a real TAG -> drop (no numeric/name masquerade)
        if not isinstance(rec, dict):
            continue
        rrec = dict(rec)
        # diagnostic submanager heap addr -> NEVER persisted. Also drop the UNVALIDATED
        # candidate list if a future caller ever turns it on (defensive; we ask for
        # include_candidate=False, so it is normally absent already).
        rrec.pop("_submgr_addr", None)
        rrec.pop("available_focuses_candidate", None)
        # FAIL-CLOSED invariant (Codex final review): country.focus.completed is a
        # PROTECTED, SAVE-SHARED leaf whose schema skeleton default is [] (not None). The
        # live reader always yields a LIST for completed on an emitted country (a country
        # whose submanager fails the strict gate is omitted upstream, never emitted with
        # completed=None). If completed is ever NOT a list (a malformed/partial read), we
        # DROP this country rather than emit a None that the assembler would leave as the
        # skeleton [] and the save backstop would then protect as if measured-empty -- a
        # silent fabrication of "zero completed focuses". Dropping keeps the channel
        # honest: focus.completed is a measured list IFF the country is present.
        if not isinstance(rrec.get("completed"), list):
            continue
        countries[tag] = {
            "focus_tree": rrec.get("focus_tree"),
            "completed": rrec.get("completed"),
            "completed_count": rrec.get("completed_count"),
            "current": rrec.get("current"),
            "current_continuous": rrec.get("current_continuous"),
            # progress_days is the days-into-the-active-focus value (same unit as the
            # save `progress`); it folds onto country.focus.progress (M/S).
            "progress_days": rrec.get("progress_days"),
            # paused is RESISTED -> the reader's None; carried so the assembler can see
            # it is unmeasured (it never writes a fabricated False).
            "paused": rrec.get("paused"),
        }
    return {"countries": countries, "count": len(countries)}


def _f_production_live():
    from production_live import read_all  # noqa
    return read_all


def _r_production_live(ctx: CaptureContext, read_all) -> Dict[str, Any]:
    """LIVE per-country PRODUCTION (production_live.py: equipment STOCKPILE bank
    459/461 ids EXACT fixed-point over 23 countries, 22/23 countries 100% exact; mil-
    line count + SUM(active_factories) EXACT for 7/7 anchors; naval-line count EXACT).
    Reads ctx.proc READ-ONLY.

    Folds into the NEW live-only home country.production under the DISTINCT
    "production_live" provenance id (the save "production" domain folds elsewhere --
    military.equipment_stockpile / military.convoys / economy._production_lines -- so
    there is no collision). production_live.read_all keys by TAG when the country-index
    bridge resolves, but FALLS BACK to localized country NAMES when the bridge fails;
    those are NOT TAGs, so we REJECT any key that is not schema-valid (mirrors
    intel_live -- never let a localized name create a non-TAG country). If rejecting the
    name-keyed fallback leaves NOTHING (the bridge failed and every key was a name), we
    RAISE so the domain is honestly omitted rather than claimed captured-empty (Codex).

    RESISTED (absent, never fabricated -- production_offsets.draft.json open tickets):
    per-line priority / requested_factories / equipment_variant_index (not offset-pinned,
    no distinguishing variance this frame); construction queue (state-derived). Raises if
    ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("production_live needs ctx.proc (unavailable)")
    # reuse the already-resolved country-index bridge (ctx.bridge) so the reader skips its
    # own per-walk country-array re-resolve scan (steady-state perf; None -> reader self-resolves).
    raw = read_all(ctx.proc, bridge=ctx.bridge)
    countries: Dict[str, Any] = {}
    saw_any_key = False
    for key, rec in (raw or {}).items():
        saw_any_key = True
        if not schema.valid_tag(key):
            continue            # localized-name fallback key (or non-TAG) -> drop
        if not isinstance(rec, dict):
            continue
        countries[key] = {
            # stockpile keys are equipment-variant ids; coerce to str so the persisted
            # JSON dict is string-keyed (no fabrication of the values). A measured-empty
            # {} is a real "no banked equipment" and is preserved.
            "stockpile": ({str(k): v for k, v in rec["stockpile"].items()}
                          if isinstance(rec.get("stockpile"), dict) else
                          rec.get("stockpile")),
            "stockpile_total": rec.get("stockpile_total"),
            "military_lines": rec.get("military_lines"),
            "naval_lines": rec.get("naval_lines"),
            "military_factories": rec.get("military_factories"),
            "naval_factories": rec.get("naval_factories"),
        }
    # If the reader returned countries but NONE survived the TAG filter, the bridge fell
    # back to name keys (identity un-nameable as TAGs) -> a broken read, NOT a measured-
    # empty world. Raise so the domain is honestly omitted (Codex; mirrors country_misc).
    if saw_any_key and not countries:
        raise RuntimeError("production_live: no key resolved to a TAG (bridge fell back "
                           "to localized names) -- domain honestly omitted")
    return {"countries": countries, "count": len(countries)}


def _f_diplomacy_live():
    from diplomacy_live import read_all  # noqa
    return read_all


def _r_diplomacy_live(ctx: CaptureContext, read_all) -> Dict[str, Any]:
    """LIVE per-country + world DIPLOMACY (diplomacy_live.py: at-war set 30/30 EXACT,
    casualty pairs 144/144 EXACT, per-country totals 30/30, faction membership 24/24 vs
    the lockstep save). Reads ctx.proc READ-ONLY. The reader resolves its OWN country
    identity (vtable scan + tag-db-id bridge), so it does NOT need ctx.bridge.

    Splits into TWO homes (Codex-blessed; diplomacy_world is a SAVE-shaped catalog whose
    war-id / save-faction-id keys are a DIFFERENT identity model than the live pairwise
    war_relations + numeric-group faction_members, so the live world data is NOT folded
    into it):
      * PER-COUNTRY -> country.diplomacy.{at_war, casualties, faction_group} (M).
      * WORLD -> the NEW top-level sidecar snap["diplomacy_live"] {war_relations,
        faction_members, subtype_warning} (M).

    EVERY TAG is validated via schema.valid_tag (country keys, enemy-TAG lists, faction
    member lists) -- a key/enemy that is not a real TAG is dropped, never written near
    identity. faction_group is the OPAQUE engine diplomatic-group id (an int, NOT a TAG),
    kept as an int and named honestly. RESISTED (left to the save): opinion/relation flags,
    subjects/autonomy, the formal faction OBJECT name/leader (see the reader's open tickets).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("diplomacy_live needs ctx.proc (unavailable)")
    # STAGE 1+2: reuse the shared canonical CountryIdentity (resolved once at attach) instead of
    # this reader's own vtable+bridge identity scan, and pin on the shared identity root so a
    # warmed walk skips the scan. A None identity (resolution failed / disagreed with the bridge)
    # -> read_all self-resolves (honest, no fabrication).
    #
    # STAGE 1.5 (war-relation pool pin): identity threading alone left the war_relation HEAP SWEEP
    # (read_war_relations) running EVERY walk -- the residual diplomacy cold/warm cost. We now also
    # pin the located war_relation RECORD ADDRESSES across walks (ctx.pins["diplomacy_war_addrs"]):
    # the cold walk records them (want_war_addrs=True) and a warm walk re-reads casualties from the
    # pinned records WITHOUT the sweep, gated by validate_war_relation_addrs (typeptr + tag-pair +
    # echo per record -> fail-closed). A validate miss inside read_all falls back to the full sweep.
    # faction_groups is already a cheap per-country read (no sweep), so it needs no pool pin.
    _pins = ctx.pins
    _fp = ctx.proc_fp()
    if _pins.get("__fp__") != _fp:
        _pins.clear()                       # mirror _pin_get's global epoch (proc/build change)
        _pins["__fp__"] = _fp
    _war_addrs_in = _pins.get("diplomacy_war_addrs")

    def _read_diplo(roots):
        raw = read_all(ctx.proc, ident=roots.get("identity"),
                       war_addrs=_war_addrs_in, want_war_addrs=True)
        # cache the (re-validated) record addresses for the next warm walk. A None means the warm
        # path was not taken and the sweep found none -> drop any stale cache (fail-closed).
        wa = raw.get("war_addrs") if isinstance(raw, dict) else None
        if wa:
            _pins["diplomacy_war_addrs"] = wa
        else:
            _pins.pop("diplomacy_war_addrs", None)
        return raw

    raw = _pin_country_reader(ctx, "diplomacy", _read_diplo, need_identity=True)
    if not isinstance(raw, dict):
        raise RuntimeError("diplomacy_live.read_all returned non-dict")
    raw.pop("war_addrs", None)              # diagnostic heap addrs: never enter the snapshot

    # --- per-country: at_war (enemy TAGs), casualties (int), faction_group (int) ---
    at_war_in = raw.get("at_war") or {}
    casualties_in = raw.get("casualties") or {}
    faction_group_in = raw.get("faction_group") or {}
    # union of TAGs the reader keyed; keep ONLY schema-valid TAGs (a non-TAG key is
    # dropped, never written near identity).
    tags = set()
    for src in (at_war_in, casualties_in, faction_group_in):
        for k in src:
            if schema.valid_tag(k):
                tags.add(k)
    countries: Dict[str, Any] = {}
    for tag in tags:
        rec: Dict[str, Any] = {}
        # at_war: a MEASURED list (incl. measured-empty [] = at peace). Filter each enemy
        # through valid_tag so a stray non-TAG can never enter the enemy list. Only emit
        # the leaf when the reader actually keyed this TAG (else it stays None/unmeasured).
        if tag in at_war_in:
            enemies = at_war_in.get(tag)
            if isinstance(enemies, list):
                rec["at_war"] = [e for e in enemies if schema.valid_tag(e)]
        if tag in casualties_in:
            cv = casualties_in.get(tag)
            if isinstance(cv, int):
                rec["casualties"] = cv
        if tag in faction_group_in:
            gv = faction_group_in.get(tag)
            if isinstance(gv, int):
                rec["faction_group"] = gv
        if rec:
            countries[tag] = rec

    # --- world: war_relations (pairwise, TAG-validated) + faction_members (group->TAGs) ---
    war_relations = []
    for w in (raw.get("war_relations") or []):
        if not isinstance(w, dict):
            continue
        first = w.get("first")
        second = w.get("second")
        # keep only fully-named pairs (both ends a real TAG); a half-resolved pair is
        # not a clean measurement of who-fought-whom (no numeric/None masquerade).
        if not (schema.valid_tag(first) and schema.valid_tag(second)):
            continue
        war_relations.append({
            "first": first, "second": second,
            "first_casualties": w.get("first_casualties"),
            "second_casualties": w.get("second_casualties"),
        })
    faction_members: Dict[str, list] = {}
    for gid, members in (raw.get("faction_members") or {}).items():
        if not isinstance(members, (list, tuple)):
            continue
        clean = [m for m in members if schema.valid_tag(m)]
        # key by the OPAQUE numeric group id, stringified for JSON. Keep a group even if
        # it ends up with one member (it is still a measured grouping); a group whose
        # members are all un-nameable becomes an empty list (measured-but-un-nameable).
        faction_members[str(gid)] = sorted(clean)
    subtype_warning = raw.get("war_relation_subtype_warning")
    if not isinstance(subtype_warning, dict):
        subtype_warning = {}

    return {
        "countries": countries,
        "war_relations": war_relations,
        "faction_members": faction_members,
        "subtype_warning": subtype_warning,
        "count": len(countries),
    }


def _f_diplo_relations():
    from diplo_relations_live import read_all  # noqa
    return read_all


def _r_diplo_relations(ctx: CaptureContext, read_all) -> Dict[str, Any]:
    """LIVE bilateral RELATIONS (diplo_relations_live.py: opinion 1113/1113 real ordered
    pairs EXACT, subjects/autonomy 17/17 EXACT, relation flags as undirected pairs all EXACT
    vs the lockstep save). Reads ctx.proc READ-ONLY. The reader resolves its OWN country
    identity (it reuses diplomacy_live's CountryIdentity vtable scan), so it does NOT need
    ctx.bridge.

    Folds PER-COUNTRY onto countries.*.diplomacy (Codex-blessed -- the bilateral matrix is a
    natural per-country {TGT: value} shape, the SAME shape the save opinion leaf already uses;
    a top-level sidecar would duplicate identity). The reader's "SRC|TGT" string keys are
    transposed into a per-SRC {TGT: ...} map. EVERY tag (SRC, TGT, overlord, subject) is
    validated via schema.valid_tag -- a non-TAG end is dropped, never written near identity
    (the reader already excludes the WST pseudo-tag pairs; this is defense in depth).

    Per-country leaves emitted:
      * opinion       {TGT: net_opinion(int)}  -- folds LIVE-FIRST onto the save-shared
                       diplomacy.opinion leaf (M/S; protected via _LIVE_COUNTRY_DOMAIN_LEAVES).
      * relation_flags {TGT: [flag,...]}        -- the 12 directed relation-flag types (live-only).
      * overlord / autonomy_state / autonomy_ratio -- when THIS country is a SUBJECT (from the
                       reader's `subjects` list, keyed by subject TAG).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted, no fabrication)."""
    if ctx.proc is None:
        raise RuntimeError("diplo_relations needs ctx.proc (unavailable)")
    # STAGE 1+2: reuse the shared canonical CountryIdentity (resolved once at attach) + pin on the
    # shared identity root so a warmed walk skips the scan. None identity -> self-resolve (no fab).
    raw = _pin_country_reader(
        ctx, "diplo_relations",
        lambda roots: read_all(ctx.proc, ident=roots.get("identity")),
        need_identity=True)
    if not isinstance(raw, dict):
        raise RuntimeError("diplo_relations_live.read_all returned non-dict")

    # WST (the Wasteland pseudo-tag) PASSES schema.valid_tag (3 uppercase alphanumerics) but is
    # NOT a real diplomatic actor (no country object) -- the reader already excludes WST pairs;
    # this is defense in depth so a stray WST end can never key a country / target / overlord.
    def _real_tag(t):
        return schema.valid_tag(t) and t != "WST"

    # --- transpose the "SRC|TGT"-keyed opinion + flags into per-SRC {TGT: ...} maps ---
    opinion_by_src: Dict[str, Dict[str, int]] = {}
    for key, val in (raw.get("opinion") or {}).items():
        if not isinstance(key, str) or "|" not in key or not isinstance(val, int):
            continue
        src, tgt = key.split("|", 1)
        if not (_real_tag(src) and _real_tag(tgt)):
            continue            # a non-TAG / WST end -> drop (no identity masquerade)
        opinion_by_src.setdefault(src, {})[tgt] = val

    flags_by_src: Dict[str, Dict[str, list]] = {}
    for key, fl in (raw.get("flags") or {}).items():
        if not isinstance(key, str) or "|" not in key or not isinstance(fl, list):
            continue
        src, tgt = key.split("|", 1)
        if not (_real_tag(src) and _real_tag(tgt)):
            continue
        # keep only string flag names (the reader emits sorted token strings); a measured-empty
        # list is dropped by the reader already (it only keys a pair when fl is truthy).
        clean = [f for f in fl if isinstance(f, str)]
        if clean:
            flags_by_src.setdefault(src, {})[tgt] = clean

    # --- subjects/autonomy: keyed by the SUBJECT TAG (this-country-is-a-subject) ---
    subject_of: Dict[str, Dict[str, Any]] = {}
    for s in (raw.get("subjects") or []):
        if not isinstance(s, dict):
            continue
        sub = s.get("subject")
        ov = s.get("overlord")
        # both ends must be real TAGs to be a clean measurement; the autonomy state/ratio ride
        # on the subject's per-country record.
        if not (_real_tag(sub) and _real_tag(ov)):
            continue
        subject_of[sub] = {
            "overlord": ov,
            "autonomy_state": (s.get("autonomy_state")
                               if isinstance(s.get("autonomy_state"), str) else None),
            "autonomy_ratio": (s.get("value")
                               if isinstance(s.get("value"), (int, float)) else None),
        }

    # --- assemble per-country records (only TAGs that carry at least one measured leaf) ---
    tags = set(opinion_by_src) | set(flags_by_src) | set(subject_of)
    countries: Dict[str, Any] = {}
    for tag in tags:
        if not schema.valid_tag(tag):
            continue
        rec: Dict[str, Any] = {}
        if tag in opinion_by_src:
            rec["opinion"] = opinion_by_src[tag]
        if tag in flags_by_src:
            rec["relation_flags"] = flags_by_src[tag]
        if tag in subject_of:
            rec["overlord"] = subject_of[tag]["overlord"]
            rec["autonomy_state"] = subject_of[tag]["autonomy_state"]
            rec["autonomy_ratio"] = subject_of[tag]["autonomy_ratio"]
        if rec:
            countries[tag] = rec
    return {"countries": countries, "count": len(countries)}


def _f_politics_live():
    from politics_live import read_country_politics  # noqa
    return read_country_politics


def _r_politics_live(ctx: CaptureContext, read_country_politics) -> Dict[str, Any]:
    """LIVE per-country POLITICS (politics_live.py: popularity 4-vector 46/46 EXACT,
    ruling_party 46/46 EXACT, ideas set+order 46/46 EXACT, election_frequency EXACT,
    political_power == the PP Rosetta vs the lockstep save). Reads ctx.proc READ-ONLY and
    reuses the already-resolved country-index bridge to name countries (its {running_index:
    TAG} form is what the reader expects), so it does not re-scan / re-bridge against the
    save PP Rosetta.

    Folds onto the SAME canonical save home (country.politics) under the DISTINCT
    "politics_live" provenance id. The reader's keys are already TAGs; we keep ONLY
    schema-valid TAG keys (a non-TAG key is dropped, never written near identity).

    PP-OVERLAP reconciliation (Codex-blessed): political_power is ALREADY owned by the
    country_scalars live channel (record+0x20, folded + protected). politics_live ALSO reads
    PP (ps+0xE0) and the two agree (same Rosetta), but we deliberately do NOT carry PP here
    -- a single live owner keeps the protected leaf unambiguous and prevents a silent clobber
    if the two readers ever disagree. We carry popularities/ruling_party/ideas/idea_count/
    election_frequency/party_count/low_confidence only.

    RESISTED (left to the save / null, never fabricated): LAW sub-typing (the economy/trade/
    mobilisation law ideas ARE in the `ideas` list by id, but not classified into a typed
    triple -- needs CIdea->category resolution); last_election + elections_allowed; advisors
    (see the reader's open tickets). The diagnostic _status_addr is stripped.

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("politics_live needs ctx.proc (unavailable)")
    # reuse the shared bridge for naming AND for the live country-array base (rec0/count) so the
    # reader SKIPS its own _locate_country_array (removes a redundant relocate AND the stale-PP
    # relocate that used to kill the walk -- see _bridge_idx_tag). None -> reader self-locates.
    bridge_idx_tag, _rec0, _count = _bridge_idx_tag(ctx)
    raw = read_country_politics(ctx.proc, bridge=bridge_idx_tag, strict=True,
                                rec0=_rec0, count=_count)
    # OPTIONAL LAW sub-typing (import-guarded, per-capture cached, fail-soft): the reader's
    # `ideas` list already contains the economy/trade/mobilisation LAW ideas BY TOKEN (the
    # reader measures the ideas set EXACT as token strings). defnames_live's idea_group_map
    # classifies each idea token into its law GROUP, so classify_laws(ideas) yields a typed
    # {economy, trade_laws, mobilization_laws} triple WITHOUT re-reading memory per idea.
    # This CLOSES the law-subtyping ticket. Derived from the MEASURED idea tokens (source M).
    # Disabled by default; a missing/broken defnames leaves laws absent (raw ideas survive).
    _idea_map = _defnames_registry(ctx, "idea_group") if _defnames_enabled(ctx) else None
    _dn = _defnames_module() if _idea_map else None
    countries: Dict[str, Any] = {}
    for tag, rec in (raw or {}).items():
        if not schema.valid_tag(tag):
            continue            # not a real TAG -> drop (no numeric/name masquerade)
        if not isinstance(rec, dict):
            continue
        # popularities: a MEASURED dict {democratic,communism,fascism,neutrality: pct}.
        # Pass it through only when it is a dict (the reader always returns the 4-key dict
        # on an emitted country); a measured 0.0 popularity is informative and kept.
        pops = rec.get("popularities")
        pops = dict(pops) if isinstance(pops, dict) else None
        # ideas: a MEASURED ordered list (incl. measured-empty []). The reader returns a
        # list on an emitted country; keep it as a list (a non-list is unmeasured -> None
        # so the leaf stays null rather than fabricating an empty ideas set).
        ideas = rec.get("ideas")
        ideas = list(ideas) if isinstance(ideas, list) else None
        out_rec = {
            "popularities": pops,
            "ruling_party": rec.get("ruling_party"),
            "ideas": ideas,
            "idea_count": rec.get("idea_count"),
            "election_frequency": rec.get("election_frequency"),
            "party_count": rec.get("party_count"),
            "low_confidence": rec.get("low_confidence"),
            # political_power is RESISTED here (country_scalars owns it). _status_addr is a
            # diagnostic heap addr -> deliberately NOT carried.
        }
        # additive typed law triple from the measured idea tokens (only law groups present
        # are keyed; an unknown/non-law idea simply never appears -> no fabrication).
        if _dn is not None and isinstance(ideas, list):
            try:
                laws = _dn.classify_laws(ctx.proc, ideas, _idea_map)
            except Exception:
                laws = None
            if laws:
                out_rec["laws"] = laws
        countries[tag] = out_rec
    return {"countries": countries, "count": len(countries)}


def _f_projects():
    import projects_live as _pl  # noqa
    return _pl


def _pin_projects(ctx: CaptureContext, pl):
    """Resolve-or-REUSE the located facility/project/scientist object-address pools. On a warmed
    walk the THREE full-arena vtable sweeps (read_facilities + read_projects + read_scientists
    each sweep committed heap -- the ~76s cold cost) are SKIPPED: read_*_from_addrs re-reads every
    field LIVE off the pinned addrs (values never frozen). Returns the decoded
    (facilities, projects, scientists_raw) triple -- scientists_raw KEEPS each row's `_addr` so
    the caller's opt-in trait resolution still works (the caller strips it before persisting).

    Roots = {"fac": [(addr,fp)...], "proj": [(addr,fp)...], "sci": [(addr,fp)...],
             "data": (facilities, projects, scientists_raw)}.
    Invalidation (fail-closed): validate runs pl.validate_{facility,project,scientist}_addrs over
    EVERY pinned addr -- vtable + type + a composite stable-id fingerprint (Codex-hardened ABA
    defense: facility=(id,province), project=(id,def_handle,owner_tag,owner_fid), scientist=
    (char_id)); any mismatch -> drop the pin + full re-scan. The pool COUNTS are implicit in the
    pinned addr lists (a paused game cannot grow/shrink the pools)."""
    p = ctx.proc

    def _resolve():
        fac_addrs = pl.enumerate_facility_addrs(p)
        proj_addrs = pl.enumerate_project_addrs(p)
        sci_addrs = pl.enumerate_scientist_addrs(p)
        n = len(fac_addrs) + len(proj_addrs) + len(sci_addrs)
        if n == 0:
            return None, (([], [], [])), None     # nothing located -> do NOT pin
        data = (pl.read_facilities_from_addrs(p, fac_addrs),
                pl.read_projects_from_addrs(p, proj_addrs),
                pl.read_scientists_from_addrs(p, sci_addrs))
        roots = {"fac": fac_addrs, "proj": proj_addrs, "sci": sci_addrs, "data": data}
        return roots, data, n

    def _validate(roots, pop):
        return bool(pl.validate_facility_addrs(p, roots["fac"])
                    and pl.validate_project_addrs(p, roots["proj"])
                    and pl.validate_scientist_addrs(p, roots["sci"]))

    def _reread(roots):
        # NOTE (task #2): projects is byte-bound (~515k tiny per-row RPM) but NOT a dominant
        # residual. We deliberately RE-READ off the validated addrs rather than serving the cached
        # decode: the project `iterations` counter and scientist skill levels are decode fields the
        # validate fingerprints do NOT cover (id/def/owner only), so a cached-decode return would
        # be a fail-closed gap the per-row reread doesn't have. The reread is bounded by the pinned
        # addr lists (no heap sweep) -- correctness over the byte-bound micro-cost.
        return (pl.read_facilities_from_addrs(p, roots["fac"]),
                pl.read_projects_from_addrs(p, roots["proj"]),
                pl.read_scientists_from_addrs(p, roots["sci"]))

    return _pin_get(ctx, "projects", _resolve, _validate, _reread)


def _r_projects(ctx: CaptureContext, impl) -> Dict[str, Any]:
    """LIVE special-projects / facilities / scientists pools (projects_live.py:
    facilities 25/25, projects 6721 == 6721, scientists 311/311 validated vs save).

    Reads ctx.proc READ-ONLY (the reader uses ctx.proc directly; it opens no own
    handle, like state_detail / supply). NO owner/country is exposed by the reader
    (facilities carry only a province id; scientists carry no country), so the pools
    are emitted as GLOBAL world-scoped lists -- NOT keyed by TAG and ctx.resolve_tag
    is deliberately NOT applied (per-country attribution is an open ticket, never
    fabricated). The raw static-def db-id HANDLES (active_project_handle /
    static_def_handle) are kept as ints named as handles -- the token-name registry
    is unlocated, so they are never resolved to a fabricated name. The diagnostic
    heap address (`_addr`) is stripped before the data enters the snapshot.

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted). Any
    sub-read raising propagates so the WHOLE domain is omitted rather than returning
    a partial special_projects block (Codex: no per-sublist measurement claim).

    STAGE 1.5: the three located object pools are PINNED (see _pin_projects) so a warmed walk
    SKIPS the three arena vtable sweeps and re-reads each pool LIVE off the pinned addrs
    (fail-closed -- a moved/freed/realloc'd object re-scans)."""
    pl = impl
    if ctx.proc is None:
        raise RuntimeError("projects_live needs ctx.proc (unavailable)")
    p = ctx.proc

    def _strip_addr(rows):
        out = []
        for r in (rows or []):
            if not isinstance(r, dict):
                continue
            r = dict(r)
            r.pop("_addr", None)        # heap addr is diagnostic-only; never persisted
            out.append(r)
        return out

    _fac_raw, _proj_raw, _sci_raw = _pin_projects(ctx, pl)   # warmed walk: no heap sweeps
    facilities = _strip_addr(_fac_raw)
    projects = _strip_addr(_proj_raw)
    # OPTIONAL scientist TRAIT NAME resolution (import-guarded, per-capture cached, fail-soft):
    # the scientist record exposes only trait_count, but its char `_addr` lets defnames_live
    # walk the DIRECT trait def-pointer elements (resolve_scientist_traits_live) and decode
    # each to its trait TOKEN. We resolve using the raw `_addr` BEFORE _strip_addr removes it,
    # and ADD a `traits` list (tokens) ALONGSIDE trait_count. A scientist whose trait walk
    # fails closed (resolver returns None) gets NO traits key (never a fabricated list).
    # Disabled by default; a missing/broken defnames leaves `traits` absent (count survives).
    _sci_raw = _sci_raw or []
    _resolve_traits = _defnames_enabled(ctx)
    _dn = _defnames_module() if _resolve_traits else None
    _sci_def_addrs = (_defnames_registry(ctx, "scientist_trait_def_addrs")
                      if _resolve_traits else None)
    scientists = []
    for r in _sci_raw:
        if not isinstance(r, dict):
            continue
        addr = r.get("_addr")
        traits = None
        if _dn is not None and _sci_def_addrs and addr:
            try:
                traits = _dn.resolve_scientist_traits_live(p, addr, _sci_def_addrs)
            except Exception:
                traits = None
        r = dict(r)
        r.pop("_addr", None)            # heap addr is diagnostic-only; never persisted
        # resolve_scientist_traits_live returns [token,...] (incl. a measured-empty [] for a
        # genuinely traitless scientist) or None (fail-closed). trait_count is the
        # AUTHORITATIVE measured count (always kept). To avoid FABRICATING "no traits" when
        # trait_count>0 but a token came back unreadable (None), we add `traits` ONLY when the
        # resolved list is COMPLETE -- every element is a real token AND its length matches the
        # measured trait_count (so no silent drop understates a real roster). A genuine
        # measured-empty [] (trait_count 0) is kept; an incomplete resolution -> omit `traits`
        # (the count still conveys the real number; no fabricated zero -- Codex final review #2).
        if isinstance(traits, list):
            tc = r.get("trait_count")
            if all(t for t in traits) and (tc is None or len(traits) == tc):
                r["traits"] = list(traits)
        scientists.append(r)
    return {
        "facilities": facilities,
        "projects": projects,
        "scientists": scientists,
        "count": {"facilities": len(facilities), "projects": len(projects),
                  "scientists": len(scientists)},
    }


def _f_ships():
    from combat.combat_enum import CombatReader  # noqa
    from oob_full_live import OobShipRegistry  # noqa
    return (CombatReader, OobShipRegistry)


def _pin_ships(ctx: CaptureContext, CombatReader, OobShipRegistry):
    """Resolve-or-REUSE the located CShip body addresses ({instance_id: addr}). On a warmed walk
    the full arena CShip-vtable scan (build_index) is SKIPPED: read_ship re-reads each pinned
    body's stats LIVE (values never frozen). Each walk opens its OWN CombatReader handle (verified
    same-process); we cache only INTEGER VAs (handle-independent; the proc-fingerprint epoch in
    _pin_get clears all pins on any pid/create-time/sha/base change -- Codex Q4). Invalidation
    (fail-closed, Codex Q2): validate re-reads EVERY pinned body's vtable (==cship_vt) AND its
    (type==51, instance_id==key) handle; any mismatch -> drop the pin + re-scan. A stable per-body
    id (the handle's high u32) makes this a true ABA identity proof, not a mere liveness heuristic.

    WARM-FIX (pin-miss root cause): _validate reads the CShip layout constants (CSHIP_VT_OFF /
    OFF_HANDLE / SHIP_TYPE_TAG) from oob_full_live. Bind the module HERE (lazy, like _f_ships)
    -- the previous code referenced an undefined ``_oob`` global, so EVERY warm validate raised
    NameError, _pin_get caught it as a miss, and the full CShip arena scan re-ran every walk. The
    constants are validated identity gates (NOT a weakened check); binding the module makes the
    intended vtable+type+id validation actually run -- a stale ship still fails closed + re-scans."""
    import oob_full_live as _oob            # CShip layout constants for the validate gate
    def _with_reader(fn):
        reader = CombatReader()
        try:
            _verify_same_process(ctx, reader)
            ok, _sha = reader.verify_build()
            if not ok:
                raise RuntimeError("ships reader build gate failed (unverified build)")
            return fn(reader)
        finally:
            try:
                reader.close()
            except Exception:
                pass

    def _resolve():
        def _do(reader):
            reg = OobShipRegistry(reader)
            index = dict(reg.build_index())        # {iid: addr} -- the located roots
            if not index:
                return None, {}, None              # empty -> do NOT pin (no negative cache)
            data = {iid: reg.read_ship(iid) for iid in index}
            return index, data, len(index)
        return _with_reader(_do)

    def _validate(index, pop):
        def _do(reader):
            if pop is not None and len(index) != pop:
                return False
            cship_vt = reader.module_base + _oob.CSHIP_VT_OFF
            # Validate ALL pinned bodies (vtable + type + id) -- still far cheaper than a heap
            # scan, and a true identity proof for every cached root (Codex Q2).
            for iid, addr in index.items():
                if reader.u64(addr) != cship_vt:           # moved/freed/different class
                    return False
                handle = reader.u64(addr + _oob.OFF_HANDLE)
                if handle is None:
                    return False
                if (handle & 0xFFFFFFFF) != _oob.SHIP_TYPE_TAG:
                    return False
                if ((handle >> 32) & 0xFFFFFFFF) != iid:   # ABA: same class, wrong instance
                    return False
            return True
        return _with_reader(_do)

    def _reread(index):
        def _do(reader):
            reg = OobShipRegistry(reader)
            reg._index = index                     # reuse pinned roots; NO scan
            return {iid: reg.read_ship(iid) for iid in index}
        return _with_reader(_do)

    return _pin_get(ctx, "ships", _resolve, _validate, _reread)


def _r_ships(ctx: CaptureContext, impl) -> Dict[str, Any]:
    """LIVE per-SHIP roster from the global CShip registry (1150/1150 validated):
    definition, category, strength_hp, organisation, experience, sunk_convoys.
    Opens its OWN handle (CombatReader), verified same-process before use. owner is
    NOT available live (it comes from the save OOB join), so live ships have none.

    STAGE 2: the located CShip body set is PINNED so a warmed walk skips the arena scan
    (fail-closed -- a moved/freed/realloc'd ship body re-scans; see _pin_ships)."""
    CombatReader, OobShipRegistry = impl
    ships_map = _pin_ships(ctx, CombatReader, OobShipRegistry)  # {instance_id: ship-dict}
    ships = []
    for iid, s in (ships_map or {}).items():
        if not s:
            continue
        ships.append({
            "instance_id": int(iid),
            "definition": s.get("definition"),
            "category": s.get("category"),
            "strength_hp": s.get("strength_hp"),
            "organisation": s.get("organisation"),
            "experience": s.get("experience"),
            "sunk_convoys": s.get("sunk_convoys"),
        })
    ships.sort(key=lambda r: r["instance_id"])
    return {"ships": ships, "count": len(ships)}


def _f_supply():
    from logistics_intel import supply_live  # noqa
    return supply_live


def _pin_supply(ctx: CaptureContext, supply_live):
    """Resolve-or-REUSE the CDivision body-address list backing the per-province supply proxy.
    On a warmed walk the full arena division-vtable scan (enumerate_supply_bodies) is skipped:
    enumerate_supply_from_bodies recomputes the proxy LIVE off the pinned bodies, re-gating each
    (vtable + type==4713) so a stale slot is dropped. Invalidation (fail-closed): validate
    re-checks spread SENTINEL bodies' vtable + type and the body COUNT; any mismatch -> re-scan."""
    p = ctx.proc

    def _resolve():
        bodies = supply_live.enumerate_supply_bodies(p)
        if not bodies:
            return None, {}, None
        bodies = list(bodies)
        return bodies, supply_live.enumerate_supply_from_bodies(p, bodies), len(bodies)

    def _validate(bodies, pop):
        if pop is not None and len(bodies) != pop:
            return False
        div_vt = p.module_base + supply_live.DIV_VT_OFF
        n = len(bodies)
        if n == 0:
            return False
        # sample spread indices (first, last, evenly between) -- cheap multi-point check.
        idxs = sorted({0, n - 1} | {int(round(i * (n - 1) / 4)) for i in range(5)})
        for i in idxs:
            body = bodies[i]
            try:
                if p.u64(body) != div_vt:
                    return False
                if p.u32(body + supply_live.OFF_DIV_TYPE) != supply_live.DIVISION_TYPE:
                    return False
            except OSError:
                return False
        return True

    def _reread(bodies):
        return supply_live.enumerate_supply_from_bodies(p, bodies)

    return _pin_get(ctx, "supply", _resolve, _validate, _reread)


def _r_supply(ctx: CaptureContext, supply_live) -> Dict[str, Any]:
    """{province_id: {current, capacity, gain, divisions}} live supply proxy."""
    raw = _pin_supply(ctx, supply_live)   # warmed walk: no arena scan
    out = {str(int(pid)): rec for pid, rec in raw.items()}
    return {"by_province": out, "count": len(out)}


def _f_fuel_live():
    from fuel_live import read_fuel_live  # noqa
    return read_fuel_live


def _r_fuel_live(ctx: CaptureContext, read_fuel_live) -> Dict[str, Any]:
    """LIVE per-country national FUEL (fuel_live.py: max_fuel 71/71 EXACT vs save via a
    portable Ghidra-derived pointer chain, CCountry+0x1580 fuel_status; i64/32768). Reads
    ctx.proc READ-ONLY. The reader resolves its OWN country identity (it uses the shared
    intel_live bridge internally), so it needs ONLY ctx.proc -- no ctx.bridge required.

    Folds onto the SAME canonical save home (country.economy.fuel.{current,max}) under the
    DISTINCT "fuel_live" provenance id (live-first; protected via _LIVE_COUNTRY_DOMAIN_LEAVES).
    The reader's keys are already TAGs; we keep ONLY schema-valid TAG keys (a non-TAG key is
    dropped, never written near identity). The diagnostic heap address (`fuel_status_addr`) is
    stripped before the data enters the snapshot (mirrors the projects_live/_addr convention).

    NOTE (NOT a bug): current `fuel` can read slightly BELOW the save value (the live store is
    post-CLAMP to max, and the live frame is a different game-moment than the save) -- this is
    EXPECTED drift, so we do NOT add a save-relative range gate that would null a measured
    value. max_fuel is the proof field (a slow cap; 71/71 EXACT). A measured 0.0 is a real
    "empty tank" and is preserved (the assembler uses a `is not None` guard, never truthiness).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted, no fabrication)."""
    if ctx.proc is None:
        raise RuntimeError("fuel_live needs ctx.proc (unavailable)")
    # STAGE 1: reuse the shared ctx.bridge (resolved once at attach) instead of a per-walk
    # country-array re-resolve. STAGE 2: pin the shared canonical country root so a warmed walk
    # skips the scan entirely (fail-closed -- a stale/freed root re-resolves). When the shared
    # bridge is None (resolution failed/partial), read_fuel_live self-resolves (honest, no fab).
    raw = _pin_country_reader(
        ctx, "fuel",
        lambda roots: read_fuel_live(ctx.proc, bridge=roots.get("bridge")))
    countries: Dict[str, Any] = {}
    for tag, rec in (raw or {}).items():
        if not schema.valid_tag(tag):
            continue            # not a real TAG -> drop (no numeric/name masquerade)
        if not isinstance(rec, dict):
            continue
        # fuel/max_fuel are MEASURED floats; keep both (incl. a measured 0.0). The
        # diagnostic heap addr (`fuel_status_addr`) is deliberately NOT carried.
        countries[tag] = {
            "fuel": rec.get("fuel"),
            "max_fuel": rec.get("max_fuel"),
        }
    # read_fuel_live can resolve country OBJECTS but have every per-country fuel_status read
    # fail (-> {}); that is a BROKEN read, NOT a measured-empty world. Raise so the domain is
    # honestly omitted rather than claimed captured with zero countries (Codex final review).
    if not countries:
        raise RuntimeError("fuel_live: no country resolved a fuel_status this frame "
                           "-- domain honestly omitted (not captured-empty)")
    return {"countries": countries, "count": len(countries)}


def _f_supply_inputs():
    from supply_solver_live import read_supply_inputs  # noqa
    return read_supply_inputs


def _pin_supply_inputs(ctx: CaptureContext, read_supply_inputs):
    """Resolve-or-REUSE the SINGLETON CSupplySystem instance address. On a warmed walk the full
    _find_csupplysystem heap scan is SKIPPED: read_supply_inputs re-reads the per-province arrays
    LIVE off the pinned css. Invalidation (fail-closed): validate re-confirms the pinned css still
    carries the DOUBLE-vtable signature (+0/+8 both in the CSS vtable set) AND both per-province
    arrays are still province-count-sized with an in-heap base -- the SAME gate _find_csupplysystem
    used to accept it; any mismatch -> drop the pin + re-scan. A singleton root (not a body pool).

    If the css cannot be located (e.g. a non-live process, or a mock that injects a plain
    read_supply_inputs) the pin is BYPASSED and the impl is called directly (honest, un-pinned)."""
    p = ctx.proc
    try:
        import supply_solver_live as _ss
    except Exception:
        _ss = None

    def _resolve():
        css = None
        if _ss is not None:
            try:
                css = _ss._find_csupplysystem(p)
            except Exception:
                css = None
        if not css:
            # not located (or mock impl) -> do NOT pin; call the impl directly (no css kwarg so a
            # plain fake `lambda p: {...}` still works). Honest, un-pinned (no negative cache).
            return None, read_supply_inputs(p), None
        return css, read_supply_inputs(p, css=css), None

    def _validate(css, pop):
        if _ss is None:
            return False
        vts = {p.module_base + r for r in _ss.CSS_VT_RVA}
        try:
            vt0 = p.u64(css)
            vt1 = p.u64(css + 8)
            if vt0 not in vts or vt1 not in vts:   # double-vtable signature (moved/freed/diff)
                return False
            eff_base = p.u64(css + _ss.CSS_PROV_EFF)
            eff_cnt = p.u32(css + _ss.CSS_PROV_EFF + 8)
            draw_cnt = p.u32(css + _ss.CSS_PROV_DRAW + 8)
        except OSError:
            return False
        # the SAME province-count gate _find_csupplysystem accepts the instance with.
        return bool(eff_base and _ss.HEAP_LO <= eff_base < _ss.HEAP_HI
                    and 13000 <= eff_cnt <= 14000 and 13000 <= draw_cnt <= 14000)

    def _reread(css):
        return read_supply_inputs(p, css=css)

    return _pin_get(ctx, "supply_inputs", _resolve, _validate, _reread)


def _r_supply_inputs(ctx: CaptureContext, read_supply_inputs) -> Dict[str, Any]:
    """LIVE per-province SUPPLY SOLVER INPUTS (supply_solver_live.py): the PERSISTENT
    inputs that ARE stored -- per-province supply EFFICIENCY/fulfilment [0..1] and supply
    DRAW/demand (CSupplySystem+0x10/+0x138, i32/1e5, verified live e.g. 3324=0.95). Reads
    ctx.proc READ-ONLY.

    These are BADGED SIGNALS, NOT the exact displayed per-province "supply available" number
    -- that value is provably NOT persisted on this build (supply_solver_live.province_supply_
    exact() returns {} honestly; it is a transient routing-solver output). So we surface the
    efficiency/draw INPUTS as additional confidence-badged fields on the supply block WITHOUT
    claiming exact per-province supply, and WITHOUT touching the existing division-province
    proxy (the "supply" domain). The keys are raw province ids (ints, NOT tags); they pass
    through unresolved (resolving them as country tags would be a category error).

    read_supply_inputs returns {} on a build mismatch / no live game; we RAISE on an empty
    result so the domain is honestly omitted rather than claimed captured-empty (the reader's
    own build gate already fail-closes, so {} here means it did not run). A measured-empty
    per-province dict inside a non-empty result IS preserved (sparse; only nonzero entries).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted, no fabrication)."""
    if ctx.proc is None:
        raise RuntimeError("supply_inputs needs ctx.proc (unavailable)")
    # STAGE 2: pin the SINGLETON CSupplySystem instance address so a warmed walk skips the
    # _find_csupplysystem heap scan; the per-province arrays are re-read LIVE off the pinned css.
    raw = _pin_supply_inputs(ctx, read_supply_inputs)
    if not raw:
        # {} == build mismatch / not-run (the reader fail-closes), NOT a measured-empty
        # world. Raise so the domain is honestly omitted (mirrors country_misc / production).
        raise RuntimeError("supply_inputs: read_supply_inputs returned empty "
                           "(build mismatch or not run) -- domain honestly omitted")
    eff = raw.get("efficiency")
    draw = raw.get("draw")
    # A truthy result with ONLY meta (neither efficiency NOR draw is a dict) means the css
    # instance resolved but BOTH per-province arrays were unreadable this frame -- that is a
    # broken read, NOT a measured world. Raise so the domain is honestly omitted rather than
    # badging a meta-only block as captured (Codex final review #3).
    if not isinstance(eff, dict) and not isinstance(draw, dict):
        raise RuntimeError("supply_inputs: neither efficiency nor draw resolved "
                           "(meta-only read) -- domain honestly omitted")
    # stringify the province-id keys for JSON (the values are not fabricated). A measured
    # sparse {} is preserved (a province with zero efficiency/draw is simply absent -- the
    # reader returns only nonzero entries; this is the reader's measured-sparse contract).
    eff_out = ({str(int(pid)): v for pid, v in eff.items()}
               if isinstance(eff, dict) else None)
    draw_out = ({str(int(pid)): v for pid, v in draw.items()}
                if isinstance(draw, dict) else None)
    # carry the reader's meta (counts + build) but STRIP the diagnostic heap addr
    # (`csupplysystem` is a "0x..." pointer string) -- a heap address must never reach the
    # persisted snapshot (mirrors fuel_status_addr / agency_ptr / _status_addr stripping).
    meta = raw.get("meta")
    if isinstance(meta, dict):
        meta = {k: v for k, v in meta.items() if k != "csupplysystem"}
    else:
        meta = None
    return {
        "efficiency_by_province": eff_out,
        "draw_by_province": draw_out,
        "meta": meta,
    }


def _f_sea_control():
    from naval_air.naval_air_reader import NavalAirReader  # noqa
    return NavalAirReader


def _r_sea_control(ctx: CaptureContext, NavalAirReader) -> Dict[str, Any]:
    """Naval sea-zone dominance + air superiority by strategic region (TAGs).

    REQUIRES the tag table: without it the reader would emit raw tag-db-ids /
    "?<id>" placeholders into controller/owner keys (Codex code-review #5). We
    sanitize any surviving "?<id>" placeholder to None so a numeric id can never
    masquerade as a TAG in the sidecar.
    """
    if ctx.tag_table is None:
        raise RuntimeError("naval_air needs the tag table to name controllers")
    reader = NavalAirReader()
    try:
        _verify_same_process(ctx, reader)
        sea = reader.enumerate_sea_zone_control(tag_table=ctx.tag_table)
        air = reader.enumerate_air_superiority(tag_table=ctx.tag_table)
    finally:
        try:
            reader.close()
        except Exception:
            pass
    sea_out = {str(int(rid)): _sanitize_naval_entries(ents)
               for rid, ents in (sea or {}).items()}
    air_out = {str(int(rid)): _sanitize_air_region(r)
               for rid, r in (air or {}).items()}
    return {"sea_zone_control": sea_out, "air_superiority": air_out}


def _clean_tag(v: Any) -> Optional[str]:
    """Return a real 3-letter TAG or None. The naval/air reader writes "?<id>" when it
    cannot name a controller; that, "---", and ANY non-TAG string (incl. a numeric
    string like "42") -> None so no id can masquerade as a TAG (Codex review #5).
    Single source of truth = schema.valid_tag."""
    return v if schema.valid_tag(v) else None


def _sanitize_naval_entries(ents):
    out = []
    for e in (ents or []):
        if isinstance(e, dict):
            e = dict(e)
            if "controller" in e:
                e["controller"] = _clean_tag(e.get("controller"))
            # Drop any RAW tag-db-id the reader left in the entry (adversarial
            # review #4): a numeric id must never reach the sidecar masquerading
            # as identity. The named TAG already lives in "controller" (or is
            # None); the raw id is diagnostic-only and is stripped here.
            e.pop("controller_tagid", None)
            e.pop("controller_id", None)
        out.append(e)
    return out


def _sanitize_air_region(r):
    if not isinstance(r, dict):
        return r
    r = dict(r)
    by = r.get("by_owner")
    if isinstance(by, dict):
        # keys that are "?<id>" placeholders are un-nameable owners: relabel the
        # bucket key to None-string is lossy, so we move them under an explicit
        # "_unresolved" bucket instead of pretending they are a TAG.
        clean = {}
        unresolved = {"effective_planes": 0.0, "wings": 0}
        for k, v in by.items():
            if _clean_tag(k) is None:
                if isinstance(v, dict):
                    unresolved["effective_planes"] += v.get("effective_planes", 0) or 0
                    unresolved["wings"] += v.get("wings", 0) or 0
            else:
                clean[k] = v
        if unresolved["wings"] or unresolved["effective_planes"]:
            clean["_unresolved"] = unresolved
        r["by_owner"] = clean
    # sanitize the leader UNCONDITIONALLY (Codex reconcile #5) -- not only when
    # by_owner happens to be a dict; an un-nameable leader becomes None.
    if _clean_tag(r.get("leader")) is None:
        r["leader"] = None
    return r


def _f_land_combats():
    from combat.combat_enum import CombatReader  # noqa
    return CombatReader


def _r_land_combats(ctx: CaptureContext, CombatReader) -> Dict[str, Any]:
    """Active LAND combats (the SOLVED, save-validated 13/13 land reader).

    Each record is {combat_id, location(province id), day, duration,
    attacker_units:[div_id], defender_units:[div_id]}. These carry NO tag-db-ids:
    div_ids and the province id are raw engine ints, NOT tags, so ctx.resolve_tag
    is deliberately NOT applied (Codex Q4 -- applying it would be a category error).
    A side that failed the reader's fail-closed validators is None (distinct from an
    empty []); we pass that through honestly. The assembler promotes these into the
    canonical typed combats[] (kind="land") AND keeps the raw rows in a sidecar.

    STAGE-4 (deliberately NOT pinned -- proven unpinnable; do NOT re-investigate):
    enumerate_land_combats locates active land combats by a full committed-heap vtable scan
    (CLandCombat vtable MODULE+0x2980CB8, type==62) -- there is NO base pointer to a land-
    combat container. At PEACETIME there are 0 active land combats: the scan finds NOTHING but
    still pays the full sweep, and there is no decoded ROOT to pin. Per the Codex B-round, a
    cached EMPTY scan cannot prove continued absence without a CHEAP AUTHORITATIVE zero/count
    validator, and none exists for land combats:
      * The combat-MANAGER handle array (holder vtable MODULE+0x305B170, stride 0x18) has NO
        fixed base pointer (its first record is session-dynamic, re-located every launch -> you
        would have to SCAN to find it, defeating the purpose), AND it mixes 67 type-62 records
        = land + naval/air + RECYCLED slots (each record carries a 'gen' generation field). An
        UNCHANGED total therefore does NOT prove "no new land combat" (a recycled slot can be
        reused without changing the total; a naval/air combat changes it without a land combat).
      * No per-type bucket count / land-combat registry size field exists in the cracked layout.
    So land_combats RE-SCANS every warm walk -- the honest peacetime floor for this domain.
    (At WARTIME it has real combat bodies to anchor on; an empty pin is only ever the peacetime
    problem, and it has no safe anchor. Pinning it unsafely would risk serving a stale "no
    combats" after a battle appears -- explicitly rejected.)"""
    reader = CombatReader()
    try:
        _verify_same_process(ctx, reader)
        rows = reader.enumerate_land_combats(with_defenders=True)
    finally:
        try:
            reader.close()
        except Exception:
            pass
    out = []
    for r in (rows or []):
        out.append({
            "combat_id": r.get("combat_id"),
            "location": r.get("location"),
            "day": r.get("day"),
            "duration": r.get("duration"),
            "attacker_units": r.get("attacker_units"),
            "defender_units": r.get("defender_units"),
        })
    return {"land": out, "count": len(out)}


def _f_oob_command():
    import oob_command_live as _ocl  # noqa
    return _ocl


def _oob_struct_fp(p, ocl, objs):
    """The MUTABLE-STRUCTURE fingerprint the decoded command tree depends on (Codex C MUST-FIX).
    validate_command_objs proves each command OBJECT's identity (vtable + handle), but the decoded
    tree is derived from INLINE fields that the engine can edit while the game is PAUSED (e.g. the
    user reassigning a division to another army, or changing an army's army-group): the cached
    membership/owner-consensus would go stale with the object identity unchanged. So -- mirroring the
    designs_variants per-object payload fingerprint -- we snapshot EVERY decode-determining inline
    edge and re-check it on warm validate:
      * div   : army-parent ptr (+DIV_ARMY_PARENT), owner tag-db-id (+DIV_OWNER)
      * army  : army-group parent (+ARMY_AG_PARENT), leader ptr (+ARMY_LEADER_PTR), member count
                (+ARMY_MEMBER_COUNT)
      * tf    : fleet parent (+TF_FLEET_PARENT), ship-vector begin (+TF_SHIP_BEGIN) + count
                (+TF_SHIP_COUNT)
    A field that can't be read is recorded as None (so an unreadable edge fingerprints differently
    and fails validation). Returns a hashable tuple (sorted by object addr) or None on any error
    (None != a real fp -> a re-decode, fail-closed)."""
    try:
        rows = []
        for a, _ in objs.get("div", ()):
            try:
                ap = p.u64(a + ocl.DIV_ARMY_PARENT)
            except OSError:
                ap = None
            try:
                ow = p.u32(a + ocl.DIV_OWNER)
            except OSError:
                ow = None
            rows.append(("d", a, ap, ow))
        for a, _ in objs.get("army", ()):
            try:
                agp = p.u64(a + ocl.ARMY_AG_PARENT)
            except OSError:
                agp = None
            try:
                lp = p.u64(a + ocl.ARMY_LEADER_PTR)
            except OSError:
                lp = None
            try:
                mc = p.u32(a + ocl.ARMY_MEMBER_COUNT)
            except OSError:
                mc = None
            rows.append(("a", a, agp, lp, mc))
        for a, _ in objs.get("tf", ()):
            try:
                fp = p.u64(a + ocl.TF_FLEET_PARENT)
            except OSError:
                fp = None
            try:
                sb = p.u64(a + ocl.TF_SHIP_BEGIN)
            except OSError:
                sb = None
            try:
                sc = p.u32(a + ocl.TF_SHIP_COUNT)
            except OSError:
                sc = None
            # the decode's task_force ship set is the ship-vector CONTENTS, not just (begin,count):
            # hash the element bytes so a SAME-COUNT ship swap/move while paused is caught (Codex C
            # caveat -- begin+count alone would let a stale cached TF->ship set through). Bounded by
            # the count cap; a bad header -> sh=None (begin+count still pin the structure).
            sh = None
            try:
                cap = getattr(ocl, "MAX_TF_SHIPS", 0) or 0
                if sb is not None and sc is not None and 0 < sc <= cap:
                    blob = p.try_read(sb, sc * 8)
                    if len(blob) >= sc * 8:
                        sh = _hashlib.md5(blob[:sc * 8]).hexdigest()
            except Exception:
                sh = None
            rows.append(("t", a, fp, sb, sc, sh))
        return tuple(sorted(rows))
    except Exception:
        return None


def _pin_oob_command(ctx: CaptureContext, ocl) -> Dict[str, Any]:
    """Resolve-or-REUSE the located OOB command-OBJECT sets AND the FULLY-DECODED command tree.
    On a warmed walk BOTH the arena LOCATE pass (discover_arena + enumerate_regions + the single
    _enum_vtables sweep) AND the per-candidate decode (read_command_tree_from_objs -- the division
    membership/owner-consensus loop over THOUSANDS of CDivision objects + the per-army/ag/fleet/tf
    parent-ptr reads, task #22) are SKIPPED: the cold walk decodes the tree once and pins it; a
    warmed walk returns the cached DECODED tree with NO heap access (after a cheap validate). This
    mirrors the designs_variants decoded-catalog pin.

    Roots = {"objs": <located object sets>, "tree": <decoded command tree>, "struct_fp": <edge fp>}.

    Invalidation (fail-closed, Codex-vetted): validate re-runs validate_command_objs (every pinned
    object's vtable + (id,type) handle == locate-time snapshot -- a true ABA defense for all 7
    classes, incl. DIV + SHIP) AND re-checks the MUTABLE-STRUCTURE fingerprint (_oob_struct_fp:
    every decode-determining inline edge -- div army-parent + owner, army ag-parent + leader +
    member-count, tf fleet-parent + ship-vector). Codex C MUST-FIX: object identity alone is NOT
    enough for a cached DECODED tree -- the engine can reassign a division/army (changing div+0xB0 /
    army+0x1B8) while PAUSED with the object handle unchanged, which would otherwise serve a stale
    cached membership. The struct fp catches exactly that (designs_variants payload-fp parity). Any
    mismatch -> drop the pin + re-locate + re-decode."""
    p = ctx.proc

    def _resolve():
        objs = ocl.enumerate_command_objs(p)
        n = sum(len(objs.get(k) or []) for k in
                ("army", "ag", "fleet", "tf", "theatre", "div", "ship"))
        if n == 0:
            return None, {}, None                 # nothing located -> do NOT pin
        tree = ocl.read_command_tree_from_objs(p, objs)
        struct_fp = _oob_struct_fp(p, ocl, objs)
        return {"objs": objs, "tree": tree, "struct_fp": struct_fp}, tree, n

    def _validate(roots, pop):
        if not ocl.validate_command_objs(p, roots["objs"]):
            return False
        # MUST-FIX C: the decoded membership/owner consensus is derived from MUTABLE inline edges
        # the object-identity check does NOT cover; re-fingerprint them. A None (re-read error) !=
        # the pinned fp -> fail-closed re-decode. The pinned fp itself being None means the cold
        # snapshot failed -> also re-decode (never trust a tree behind an unknown structure).
        cur = _oob_struct_fp(p, ocl, roots["objs"])
        return cur is not None and cur == roots.get("struct_fp")

    def _reread(roots):
        # validate passed -> every pinned object is the SAME live object AND every decode-determining
        # inline edge is byte-identical to the cold snapshot, so the cached decode is provably fresh:
        # serve it with NO re-read (the task-#22 per-candidate cost is eliminated). A validate MISS
        # already re-located + re-decoded.
        return roots["tree"]

    return _pin_get(ctx, "oob_command", _resolve, _validate, _reread)


def _r_oob_command(ctx: CaptureContext, ocl) -> Dict[str, Any]:
    """LIVE OOB COMMAND HIERARCHY (oob_command_live.py: armies 343/343, army-groups
    194/194, fleets 83/83, theatres 35/35; AG-member set 194/194 + leader 189/189 EXACT;
    army member-set 304/343 EXACT with the remaining gap a MEASURED live-vs-save frame gap
    surfaced as missing_count; fleet TF-set + TF->ship set + TF->fleet parent EXACT on live
    task forces, all validated vs the lockstep save). Reads ctx.proc READ-ONLY (the reader
    reuses the passed winmem.Process + fastscan; it opens NO own handle, like state_detail /
    projects_live -- so no _verify_same_process is needed).

    Folds into the SAME canonical save home (top-level snap["order_of_battle"].{armies,
    army_groups,fleets,theatres}) under the DISTINCT "oob_command" provenance id. Command
    structure is WORLD-scoped (cross-tag expeditionary divisions can sit in another tag's
    army), so the lists live top-level, NOT under countries.*.order_of_battle (Codex-blessed);
    each element carries its owner-CONSENSUS country TAG. Task forces nest under each fleet
    element (mirroring the save oob.json shape; there is no first-class top-level TF list).

    The per-SHIP order_of_battle.ships leaf is owned by the SEPARATE "ships" domain and is
    NEVER touched here (we add the command-group structure ALONGSIDE it).

    ID DISCIPLINE (Codex-blessed): member_div_ids / member_army_ids / ship_ids / task_force
    ids / army_group_id / leader_id are ALL raw engine ints in OTHER id namespaces -- they are
    passed through UNRESOLVED (resolving them as country tags would be a category error, exactly
    like land_combats div_ids). ONLY owner_tag_db_id is resolved to a country TAG via
    ctx.resolve_tag (-> None if un-nameable), written only when schema.valid_tag accepts it.

    missing_count is a MEASURED frame gap (some save member divisions have no live CDivision
    object this frame -- in-transit/embarked pool), surfaced honestly, NOT an error. The
    diagnostic heap address ("obj") is stripped before the data enters the snapshot.

    STAGE 1.5 + #22: the located command-object set AND the fully-decoded command tree are PINNED
    (see _pin_oob_command) so a warmed walk SKIPS BOTH the arena LOCATE pass AND the per-candidate
    decode (the division membership/owner-consensus loop) -- it returns the cached decoded tree
    after a cheap fail-closed validate (every object, incl. every division, identity-revalidated).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted, no partial fold)."""
    if ctx.proc is None:
        raise RuntimeError("oob_command needs ctx.proc (unavailable)")
    tree = _pin_oob_command(ctx, ocl)
    if not isinstance(tree, dict):
        raise RuntimeError("oob_command_live.read_command_tree returned non-dict")

    def _owner(rec):
        # owner_tag_db_id -> resolved country TAG (or None). schema.valid_tag is the single
        # gate: a numeric id can never masquerade as identity (mirrors every other plug).
        tag = ctx.resolve_tag(rec.get("owner_tag_db_id"))
        return tag if schema.valid_tag(tag) else None

    armies = []
    for aid, a in (tree.get("armies") or {}).items():
        armies.append({
            "id": a.get("id"),
            "owner": _owner(a),
            # raw engine division ids (NOT tags) -- pass through unresolved.
            "member_div_ids": list(a.get("member_div_ids") or []),
            "member_count_engine": a.get("member_count_engine"),
            "live_member_count": a.get("live_member_count"),
            "missing_count": a.get("missing_count"),
            "leader_id": a.get("leader_id"),          # unit_leader id (NOT a tag)
            "army_group_id": a.get("army_group_id"),  # raw army-group id (NOT a tag)
        })
    armies.sort(key=lambda r: (r["id"] is None, r["id"]))

    army_groups = []
    for agid, g in (tree.get("army_groups") or {}).items():
        army_groups.append({
            "id": g.get("id"),
            "owner": _owner(g),
            "member_army_ids": list(g.get("member_army_ids") or []),  # raw army ids
            "live_member_count": g.get("live_member_count"),
        })
    army_groups.sort(key=lambda r: (r["id"] is None, r["id"]))

    # task forces nest under their parent fleet (save oob.json shape). Group by fleet_id.
    tf_by_fleet: Dict[Any, list] = {}
    orphan_tfs = []
    for tid, t in (tree.get("task_forces") or {}).items():
        tf_rec = {
            "id": t.get("id"),
            "fleet_id": t.get("fleet_id"),
            "ship_ids": list(t.get("ship_ids") or []),  # raw ship instance ids (NOT tags)
            "ship_count": t.get("ship_count"),
        }
        fid = t.get("fleet_id")
        if fid is None:
            orphan_tfs.append(tf_rec)
        else:
            tf_by_fleet.setdefault(fid, []).append(tf_rec)
    for lst in tf_by_fleet.values():
        lst.sort(key=lambda r: (r["id"] is None, r["id"]))

    fleets = []
    for fid, f in (tree.get("fleets") or {}).items():
        fleets.append({
            "id": f.get("id"),
            "owner": _owner(f),
            "task_force_ids": list(f.get("task_force_ids") or []),  # raw TF ids
            "live_task_force_count": f.get("live_task_force_count"),
            # nested task-force objects (each with its own ship_ids); the parent-keyed
            # grouping above attributes every live TF to its fleet.
            "task_forces": tf_by_fleet.get(f.get("id"), []),
        })
    fleets.sort(key=lambda r: (r["id"] is None, r["id"]))

    theatres = []
    for thid, th in (tree.get("theatres") or {}).items():
        theatres.append({"id": th.get("id")})
    theatres.sort(key=lambda r: (r["id"] is None, r["id"]))

    return {
        "armies": armies,
        "army_groups": army_groups,
        "fleets": fleets,
        "theatres": theatres,
        # task forces whose live fleet parent missed this frame (measured-orphan, not an
        # error) are kept in a sidecar so they are not silently dropped.
        "orphan_task_forces": orphan_tfs,
        "counts": {
            "armies": len(armies), "army_groups": len(army_groups),
            "fleets": len(fleets), "theatres": len(theatres),
            "task_forces": sum(len(v) for v in tf_by_fleet.values()) + len(orphan_tfs),
        },
    }


def _f_combats():
    # The naval/air COMBAT reader is being cracked RIGHT NOW by another agent.
    # Import-guard it: if the methods aren't there yet, the plug is unavailable.
    from naval_air.naval_air_reader import NavalAirReader  # noqa
    if not (hasattr(NavalAirReader, "enumerate_naval_combats")
            and hasattr(NavalAirReader, "enumerate_air_combats")):
        raise ImportError("naval/air combat reader not yet available")
    return NavalAirReader


def _r_combats(ctx: CaptureContext, NavalAirReader) -> Dict[str, Any]:
    reader = NavalAirReader()
    try:
        _verify_same_process(ctx, reader)
        naval = reader.enumerate_naval_combats()
        air = reader.enumerate_air_combats()
    finally:
        try:
            reader.close()
        except Exception:
            pass
    return {"naval": naval or [], "air": air or []}


def _f_buildings_save():
    from logistics_intel import buildings_save as bs  # noqa
    return bs


def _r_buildings_save(ctx: CaptureContext, bs) -> Dict[str, Any]:
    """SAVE-anchored per-province rail/supply_node/naval_base + per-state building
    levels. This is NOT a live memory read (Codex Q2): it parses a melted save on
    disk and is OPT-IN -- it activates ONLY when the caller supplies a path via
    ctx.extras. With no path it RAISES, so the domain is honestly absent by default
    (no fabrication) rather than fabricating a freshness it does not have.

    extras keys (any one enables it):
        ``melt_path``         -> per-province building levels (BuildingsSave.from_melt)
        ``state_detail_path`` -> per-state building levels (StateBuildings.from_state_detail)
    The result is provenance-honest about its source (path + the fact it is a save,
    not a live read). It does NOT touch ctx.proc / PID / exe checks."""
    melt = ctx.extras.get("melt_path")
    state_detail = ctx.extras.get("state_detail_path")
    if not melt and not state_detail:
        raise RuntimeError("buildings (save-backstop) needs a melt_path or "
                           "state_detail_path in ctx.extras; none supplied "
                           "(domain honestly omitted, not fabricated)")
    out: Dict[str, Any] = {"source": "save", "from": "buildings_save"}
    if melt:
        layer = bs.BuildingsSave.from_melt(melt)
        # emit the full per-province set the reader exposes, not just rail
        # (adversarial review #6): rail_way, supply_node, naval_base.
        out["rail_by_province"] = {str(int(pid)): lvl
                                   for pid, lvl in layer.rail_map().items()}
        out["supply_node_by_province"] = {
            str(int(pid)): lvl
            for pid, lvl in (layer.prov.get("supply_node") or {}).items()}
        out["naval_base_by_province"] = {
            str(int(pid)): lvl
            for pid, lvl in (layer.prov.get("naval_base") or {}).items()}
        # HARD redaction rule (SCHEMA.md / global): store the BASENAME only -- never
        # an absolute / user-directory path in tracked snapshot output.
        out["melt_file"] = _basename(melt)
    if state_detail:
        sl = bs.StateBuildings.from_state_detail(state_detail)
        # full per-state building dict, not just infrastructure (review #6).
        out["state_buildings_by_state"] = {str(int(sid)): dict(blds)
                                           for sid, blds in sl.states.items()}
        out["infrastructure_by_state"] = {str(int(sid)): sl.infrastructure(sid)
                                          for sid in sl.states}
        out["state_detail_file"] = _basename(state_detail)
    return out


def _f_intel_save():
    from logistics_intel import intel_save as isv  # noqa
    return isv


def _r_intel_save(ctx: CaptureContext, isv) -> Dict[str, Any]:
    """SAVE-anchored per-(observer TAG, target INDEX) intel rows. NOT a live read
    (the live intel array is an unresolved residual). OPT-IN: activates ONLY when
    ctx.extras['melt_path'] is supplied; raises otherwise (absent by default).

    No-fabrication caveat (Codex Q3): the observer key IS a real save TAG, but the
    target INDEX -> TAG mapping is EXPLICITLY UNCONFIRMED in the reader's own
    docstring. We therefore keep target indices as raw integers and DO NOT relabel
    them as TAGs; a provenance note records that the index->TAG map is unvalidated."""
    melt = ctx.extras.get("melt_path")
    if not melt:
        raise RuntimeError("intel (save-backstop) needs ctx.extras['melt_path']; "
                           "none supplied (domain honestly omitted, not fabricated)")
    layer = isv.IntelSave.from_melt(melt)
    rows: Dict[str, Any] = {}
    for obs in layer.observers():
        # keys are target INDEXES (ints), kept raw -- the index->TAG map is
        # UNCONFIRMED, so we never fabricate a TAG here.
        rows[obs] = {str(int(idx)): rec
                     for idx, rec in layer.observer_map(obs).items()}
    return {"source": "save", "from": "intel_save",
            "melt_file": _basename(melt),  # basename only (HARD redaction rule)
            "by_observer": rows, "stats": layer.stats(),
            "target_index_is_tag": False,
            "note": ("target indices are raw engine indices; the index->TAG map "
                     "is UNCONFIRMED and intentionally NOT resolved")}


# NOTE on weather (Codex code-review #10): the weather reader is being cracked by
# another agent right now. Its only live entry point (read_live_month_value ->
# locate_clock) does the ~150s arena scan AND returns a DATE integer, not weather.
# That is neither cheap nor actually-weather, so there is NO honest weather plug to
# register yet -- registering one would either block every capture or surface clock
# data mislabeled as weather (a fabrication). The weather domain therefore stays
# entirely absent from the default registry until a cheap, true weather read lands;
# it is NOT in default_registry() below. (No stub, no fake "captured" -- absent.)


def _basename(path: Optional[str]) -> Optional[str]:
    """Return the BASENAME of a path (HARD redaction rule, SCHEMA.md §2): tracked
    snapshot output must never contain an absolute / user-directory path. Handles
    both separators so a Windows path is redacted on any OS."""
    if not path:
        return None
    return os.path.basename(str(path).replace("\\", "/").rstrip("/"))


# ---------------------------------------------------------------------------
# defnames_live HANDLE->NAME resolution (import-guarded, per-capture cached)
# ---------------------------------------------------------------------------
# Several already-wired live domains pin a static-def HANDLE/INDEX but leave the human
# token NAME unresolved. defnames_live cracks the handle->name registries live; we resolve
# IN THE PLUG (it has ctx.proc) and ADD the resolved tokens ALONGSIDE the raw handles
# (additive, never replacing them). Resolution is STRICTLY fail-soft + no-fabrication:
#   * import-guarded -- a missing/broken defnames_live leaves the plug returning ONLY its
#     raw handles (names absent), never crashing the plug.
#   * unknown id -> NO token (drop), never a synthetic "UNKNOWN_*"/raw-as-string fake name.
# Each registry build is a full heap vtable scan == EXPENSIVE, so we build each registry at
# most ONCE per capture and cache it on ctx.extras (keyed by the registry kind). The cache
# is per-capture only (a fresh CaptureContext per tick), so there is no stale-name risk.
# Name resolution is OPT-IN via ctx.extras["resolve_defnames"] (default OFF) so the base
# capture path never pays the scan cost unless a caller asks for resolved names.

def _defnames_enabled(ctx: CaptureContext) -> bool:
    """Resolve names only when the caller opted in AND ctx.proc is available. Default OFF
    so the base capture path never pays the per-registry heap-scan cost (Codex)."""
    return bool(ctx.extras.get("resolve_defnames")) and ctx.proc is not None


def _defnames_registry(ctx: CaptureContext, kind: str):
    """Return a defnames_live registry of `kind`, built at most ONCE per capture and cached
    on ctx.extras. Import-guarded + fail-soft: any import/build failure caches None so the
    plug silently falls back to its raw handles (no crash, no fabrication). `kind` is one of
    'tech' | 'idea_group' | 'scientist_trait' | 'mio_policy' (+ 'scientist_trait_def_addrs',
    the direct def-pointer set used by resolve_scientist_traits_live)."""
    cache = ctx.extras.setdefault("_defnames_cache", {})
    if kind in cache:
        return cache[kind]
    val = None
    try:
        import defnames_live as _dn  # noqa: import-guarded (missing -> raw handles only)
        if kind == "tech":
            val = _dn.build_tech_registry(ctx.proc)
        elif kind == "idea_group":
            val = _dn.build_idea_group_map(ctx.proc)
        elif kind == "scientist_trait":
            val = _dn.build_scientist_trait_registry(ctx.proc)
        elif kind == "scientist_trait_def_addrs":
            val = _dn._scientist_trait_def_addrs(ctx.proc)
        elif kind == "mio_policy":
            val = _dn.build_mio_policy_registry(ctx.proc)
    except Exception:
        val = None   # fail-soft: a broken/missing resolver never crashes the plug
    # An empty registry ({} / empty set) means the build gate failed or found nothing this
    # frame; cache it as-is (an empty map resolves nothing -> raw handles survive, no fake).
    cache[kind] = val
    return val


def _defnames_module():
    """Import defnames_live or return None (import-guarded). Cached implicitly by Python's
    module system; callers use this only for the stateless resolver functions."""
    try:
        import defnames_live as _dn  # noqa
        return _dn
    except Exception:
        return None


# ---------------------------------------------------------------------------
# leader_detail enrichment (OPT-IN; commanders_live joins skill+traits in-plug)
# ---------------------------------------------------------------------------
# leader_skill_traits_live cracks the per-unit-leader OVERALL SKILL + the assigned-TRAIT
# token set, keyed by unit_leader_id. The read is a FULL leader-pool heap scan (RTTI vtable
# enumerate of CArmyLeader/CNavyLeader) == EXPENSIVE -- as costly as commanders_live's own
# scan -- so the join is OPT-IN, default OFF (mirrors the resolve_defnames cost gate). The
# enrichment is STRICTLY import-guarded + fail-soft: a missing/broken reader or any read
# failure returns ({}, False) so the commanders_live plug emits leaders WITHOUT skill/traits
# (the overall LEVEL stays RESISTED) -- never a crash, never a fabricated 0 skill / [] traits.

def _leader_detail_enabled(ctx: CaptureContext) -> bool:
    """Resolve leader skill/traits only when the caller opted in AND ctx.proc is available.
    Default OFF so the base capture path never pays the second full leader-pool heap scan."""
    return bool(ctx.extras.get("resolve_leader_detail")) and ctx.proc is not None


def _leader_detail_map(ctx: CaptureContext):
    """Return ({unit_leader_id: {skill, traits}}, ran_bool), built at most ONCE per capture and
    cached on ctx.extras. Import-guarded + fail-soft: any import/read/build-gate failure returns
    ({}, False) so the caller silently falls back to leaders WITHOUT skill/traits (no crash, no
    fabrication). ran_bool is True ONLY when the reader enumerated to completion -- the assembler
    lists the "leader_detail" provenance id iff ran_bool is True (a captured measurement claim)."""
    if not _leader_detail_enabled(ctx):
        return {}, False
    cache = ctx.extras.setdefault("_leader_detail_cache", None)
    if cache is not None:
        return cache
    out: Dict[int, Dict[str, Any]] = {}
    ran = False
    reader = None
    try:
        import leader_skill_traits_live as _lst  # noqa: import-guarded
        reader = _lst.LeaderSkillTraitsReader()
        # same-process safety: the reader opens its OWN handle (via CharactersLiveReader);
        # confirm it points at OUR process instance before trusting any read (best-effort --
        # the underlying _cl carries the pid). A mismatch raises -> fail-soft below.
        inner = getattr(reader, "_cl", None)
        if inner is not None and getattr(inner, "pid", None) is not None:
            _verify_same_process(ctx, inner)
        ok, _sha = reader.verify_build()
        if ok:
            res = reader.enumerate(read_skill=True, read_traits=True)
            for rec in (res.get("leaders") or []):
                ulid = rec.get("unit_leader_id")
                if not isinstance(ulid, int):
                    continue
                d: Dict[str, Any] = {}
                sk = rec.get("skill")
                if sk is not None:
                    d["skill"] = sk
                tr = rec.get("traits")
                if isinstance(tr, list):
                    d["traits"] = list(tr)   # measured-empty [] kept; None (fail) omitted
                if d:
                    out[ulid] = d
            ran = True
    except Exception:
        out, ran = {}, False   # fail-soft: a broken/missing reader never crashes the plug
    finally:
        # the reader owns a winmem Process (`_p`) + a CharactersLiveReader handle; close both
        # best-effort so the second scan does not leak a handle (never masks the real error).
        if reader is not None:
            for attr in ("_p", "_cl"):
                obj = getattr(reader, attr, None)
                closer = getattr(obj, "close", None)
                if callable(closer):
                    try:
                        closer()
                    except Exception:
                        pass
    result = (out, ran)
    ctx.extras["_leader_detail_cache"] = result
    return result


# ---------------------------------------------------------------------------
# small helpers shared by readers
# ---------------------------------------------------------------------------
def _verify_same_process(ctx: CaptureContext, reader) -> None:
    """A reader that opened its OWN handle must point at the SAME process instance
    as the capture context, else a domain could silently read a different hoi4.exe.

    Compare PID *and* process creation time (Codex code-review #9): PID alone can
    be reused. We also confirm the reader's exe sha matches the context's so a
    reader can't read an unverified build. Raise on any mismatch (fail-closed ->
    the domain is honestly omitted rather than cross-wired)."""
    ident = ctx.identity
    rpid = getattr(reader, "pid", None)
    if ident is None or rpid is None:
        return
    from live_capture import process_identity as _pi
    rid = _pi.identify(reader)
    if int(rid.pid) != int(ident.pid):
        raise RuntimeError("reader attached to pid %s != capture pid %s"
                           % (rid.pid, ident.pid))
    # creation-time check (defeats PID reuse) when both are known.
    ct_r = getattr(rid, "create_time_100ns", None)
    ct_c = getattr(ident, "create_time_100ns", None)
    if ct_r is not None and ct_c is not None and ct_r != ct_c:
        raise RuntimeError("reader pid %s creation-time mismatch (PID reuse)"
                           % rid.pid)
    sha_r = getattr(rid, "exe_sha256", None)
    sha_c = getattr(ident, "exe_sha256", None)
    if sha_r and sha_c and sha_r != sha_c:
        raise RuntimeError("reader exe sha mismatch (different build)")


# ---------------------------------------------------------------------------
# LIVE EXTRACT-RESIDUAL plugs (deep-crack push). Each reuses ctx.proc READ-ONLY and
# folds onto a SAVE-shared home (under a DISTINCT *_live id) OR a NEW live-only home/
# leaf. Every plug STRIPS diagnostic heap addrs + raw tag-db-ids and keeps only schema-
# valid TAG keys before the data reaches the snapshot (no fabrication, no id masquerade).
# ---------------------------------------------------------------------------
def _f_world_meta():
    from world_extract_live import read_scenario_meta  # noqa
    return read_scenario_meta


def _r_world_meta(ctx: CaptureContext, read_scenario_meta) -> Dict[str, Any]:
    """LIVE scenario DATES (world_extract_live.read_scenario_meta): current_game_date +
    start_date, both i32 Jomini DateHour decoded "Y.M.D.H", validated live==save EXACT
    (1946.5.19.9 / 1936.1.1.12). Reads ctx.proc READ-ONLY (a cheap pointer chain off the
    gamestate singleton -- NOT the expensive read_world() key-presence heap scan).

    We carry ONLY the two decoded date strings; the diagnostic _gamestate_addr (a heap addr)
    and the raw date ints are deliberately NOT carried (mirrors the *_addr stripping
    convention). world_tension is COMPUTED-not-stored -> never read, never fabricated.

    Raises if ctx.proc is unavailable OR neither date decoded (a broken read, honestly
    omitted rather than claimed captured-empty -- Codex no-fabrication)."""
    if ctx.proc is None:
        raise RuntimeError("world_meta needs ctx.proc (unavailable)")
    meta = read_scenario_meta(ctx.proc) or {}
    cur = meta.get("current_game_date")   # decoded "Y.M.D.H" or None
    start = meta.get("start_date")
    if cur is None and start is None:
        raise RuntimeError("world_meta: neither current_game_date nor start_date decoded "
                           "-- domain honestly omitted (not captured-empty)")
    return {"current_date_live": cur, "start_date_live": start}


def _f_construction_orders():
    import construction_orders_live  # noqa
    return construction_orders_live


def _pin_construction_armies(ctx: CaptureContext, co):
    """Resolve-or-REUSE the located {army_id: army_obj} pool so a warmed walk SKIPS the army
    vtable arena scan that read_orders ran every walk (the residual construction_orders cost).
    The battle-plan ORDER vectors are RE-READ LIVE from the pinned armies each walk
    (read_orders_from_armies) -- nothing decoded is cached, so an order added/edited on an
    existing army while paused is observed fresh.

    Invalidation (fail-closed, Codex-vetted): validate_armies re-reads EVERY pinned army's
    vtable (== MODULE+ARMY_VT_RVA) AND its (type, army_id) handle (a true ABA identity proof);
    any mismatch -> drop the pin + re-sweep. Completeness (no NEW army appears) rests on the
    HARD PAUSED invariant (no army creation between reads), per Codex. Returns {army_id: [order,
    ...]} -- the SAME shape co.read_orders produces (only armies with >=1 order are keyed)."""
    p = ctx.proc

    def _resolve():
        armies = co.enumerate_armies(p) or {}
        if not armies:
            return None, {}, None                 # nothing located -> do NOT pin
        data = co.read_orders_from_armies(p, armies)
        return armies, data, len(armies)

    def _validate(armies, pop):
        return bool(co.validate_armies(p, armies))

    def _reread(armies):
        return co.read_orders_from_armies(p, armies)

    return _pin_get(ctx, "construction_armies", _resolve, _validate, _reread)


def _r_construction_orders(ctx: CaptureContext, co) -> Dict[str, Any]:
    """LIVE construction queue + battle-plan ORDERS + opinion-modifier decomposition
    (construction_orders_live, M). Reads ctx.proc READ-ONLY. Three independent sub-reads
    folded onto three DISTINCT live-only homes:

      * construction -> {TAG: [queue lines]} (priority order). Keys are already TAGs; we keep
        ONLY schema-valid TAG keys. Each line {line_id, template_id, active_factories, cost,
        produced, priority}; the build target STATE is RESISTED (absent), never fabricated.
      * orders -> per-army battle-plan orders. Each order carries a diagnostic `obj` heap addr
        -> STRIPPED. We flatten to a world list [{army_id, order_type, target_states}];
        target_states is the EXACT type-5 set or None (non-type-5 geometry RESISTED).
      * opinion_modifiers -> {"SRC|TGT": {cached_sum, modifiers:[{token_id, value}]}}. We
        re-key per SOURCE country into {SRC: {TGT: [{token_id, value}]}}, splitting the
        "SRC|TGT" key and re-validating BOTH ends via valid_tag. The token id->NAME registry
        is RESISTED, so only the raw token_id + value survive (never a fabricated name).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("construction_orders needs ctx.proc (unavailable)")
    # STAGE 1+2: thread the shared canonical {bridge, identity} (resolved once at attach) into the
    # two country-keyed sub-reads (read_construction reuses the bridge; read_opinion_modifiers
    # reuses the identity) and pin on the shared country root so a warmed walk skips the country
    # scan. A None shared root -> the sub-reads self-resolve (no fab).
    raw3 = _pin_country_reader(
        ctx, "construction",
        lambda roots: {
            "construction": co.read_construction(ctx.proc, bridge=roots.get("bridge")) or {},
            "opinion_modifiers": co.read_opinion_modifiers(
                ctx.proc, ident=roots.get("identity")) or {},
        },
        need_identity=False)
    # STAGE B: the battle-plan ORDERS scan the ARMY pool (a DIFFERENT root than the country set),
    # which co.read_orders re-scanned every warm walk INSIDE the pinned country reader. Pin the
    # located army pool on its OWN key so a warmed walk skips that arena sweep and only re-reads
    # the order vectors LIVE (fail-closed -- see _pin_construction_armies). A None/empty pool ->
    # {} (self-resolves; honest absence).
    raw_orders_map = _pin_construction_armies(ctx, co) or {}
    # construction queue
    raw_constr = raw3["construction"]
    construction: Dict[str, list] = {}
    for tag, lines in raw_constr.items():
        if not schema.valid_tag(tag) or not isinstance(lines, list):
            continue
        construction[tag] = [dict(l) for l in lines if isinstance(l, dict)]
    # battle-plan orders (flatten + STRIP the diagnostic `obj` heap addr)
    raw_orders = raw_orders_map
    orders: list = []
    for army_id, olist in raw_orders.items():
        for o in (olist or []):
            if not isinstance(o, dict):
                continue
            ts = o.get("target_states")
            orders.append({
                "army_id": int(army_id) if isinstance(army_id, int) else army_id,
                "order_type": o.get("order_type"),
                # measured type-5 state set (list) or RESISTED (None); NEVER fabricated [].
                "target_states": list(ts) if isinstance(ts, list) else None,
            })
    # opinion modifiers: re-key {"SRC|TGT": {...}} -> {SRC: {TGT: [{token_id, value}]}}
    # (already read against the shared identity inside the pinned reader above)
    raw_mods = raw3["opinion_modifiers"]
    opinion_modifiers: Dict[str, Dict[str, list]] = {}
    for pair, rec in raw_mods.items():
        if not isinstance(pair, str) or "|" not in pair or not isinstance(rec, dict):
            continue
        src, tgt = pair.split("|", 1)
        if not (schema.valid_tag(src) and schema.valid_tag(tgt)):
            continue
        mods = rec.get("modifiers")
        if not isinstance(mods, list):
            continue
        clean = [{"token_id": m.get("token_id"), "value": m.get("value")}
                 for m in mods if isinstance(m, dict)]
        opinion_modifiers.setdefault(src, {})[tgt] = clean
    return {
        "construction": construction,
        "orders": orders,
        "opinion_modifiers": opinion_modifiers,
        "count": len(construction),
    }


def _f_decisions_market():
    import decisions_market_live  # noqa
    return decisions_market_live


def _pin_market_contracts(ctx: CaptureContext, dm):
    """Resolve-or-REUSE the located CPurchaseContract object set so a warmed walk SKIPS the
    WHOLE-HEAP CPurchaseContract vtable scan that read_all_market ran every walk. The contract
    FIELDS (seller/buyer/equip_variant_id/amount/price) are RE-DECODED LIVE from each pinned
    object each walk (read_market_from_objs) -- nothing decoded is cached (Codex: re-decode
    live sidesteps "hash every emitted field"; only the located addrs + a contract_id snapshot
    are pinned for ABA).

    Invalidation (fail-closed, Codex-vetted): validate_contracts re-reads EVERY pinned object's
    vtable (== MODULE+PC_VT_RVA) AND its contract_id (== the locate-time snapshot) -- a true ABA
    identity proof; any mismatch -> drop the pin + re-scan.

    EMPTY CASE (Codex ruling: NOT SOUND to pin): at PEACETIME there may be 0 contracts. An
    empty whole-heap scan cannot prove continued absence without a cheap authoritative contract
    count, so we return roots=None when enumerate finds nothing -> the market SCAN re-runs every
    walk (the honest peacetime floor). The country DECISIONS half stays pinned regardless (it is
    a separate pin). Returns the decoded contract list (the SAME shape read_all_market produces)."""
    p = ctx.proc
    bridge = ctx.bridge

    def _resolve():
        addrs = dm.enumerate_contracts(p) or []
        if not addrs:
            return None, [], None                 # 0 contracts -> do NOT pin (re-scan; Codex)
        id_snapshot = {a: dm.contract_id_at(p, a) for a in addrs}
        roots = {"addrs": list(addrs), "ids": id_snapshot}
        data = dm.read_market_from_objs(p, addrs, bridge=bridge)
        return roots, data, len(addrs)

    def _validate(roots, pop):
        addrs = roots.get("addrs") or []
        if pop is not None and len(addrs) != pop:
            return False
        return bool(dm.validate_contracts(p, addrs, id_snapshot=roots.get("ids")))

    def _reread(roots):
        return dm.read_market_from_objs(p, roots["addrs"], bridge=bridge)

    return _pin_get(ctx, "market_contracts", _resolve, _validate, _reread)


def _r_decisions_market(ctx: CaptureContext, dm) -> Dict[str, Any]:
    """LIVE active timed DECISIONS + global equipment-MARKET contracts (decisions_market_live,
    M). Reads ctx.proc READ-ONLY.

      * decisions -> {TAG: status}. read_all_decisions keys by TAG (or a localized fallback
        name); we keep ONLY schema-valid TAG keys and carry the ACTIVE timed decisions
        (active_timed_running, state==active) as a measured-empty-able list, STRIPPING the
        diagnostic status_ptr. Each decision {decision, days, state, state_name}.
      * market -> [contracts]. Each contract carries diagnostic raw seller_tag_db_id /
        buyer_tag_db_id / buyer_ptr -> STRIPPED; we keep only {contract_id, seller(TAG),
        buyer(TAG), equip_variant_id, amount, price}, re-validating seller/buyer via valid_tag
        at this boundary (a non-TAG end becomes None, never a numeric id masquerade).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("decisions_market needs ctx.proc (unavailable)")
    # STAGE 1+2: thread the shared ctx.bridge (resolved once at attach) into the decisions
    # sub-read (it resolves the country set) and pin on the shared country root so a warmed walk
    # skips the country re-resolve. A None bridge -> the sub-read self-resolves (no fabrication).
    raw2 = _pin_country_reader(
        ctx, "decisions",
        lambda roots: {
            "decisions": dm.read_all_decisions(ctx.proc, bridge=roots.get("bridge")) or {},
        },
        need_identity=False)
    # STAGE B: the equipment-MARKET contracts are a DIFFERENT root (a whole-heap CPurchaseContract
    # vtable scan), which read_all_market re-ran every warm walk. Pin the located contract objects
    # on their OWN key so a warmed walk skips that scan and only RE-DECODES the contracts LIVE
    # (fail-closed -- see _pin_market_contracts). At peacetime (0 contracts) it is NOT pinned and
    # re-scans (Codex: an empty whole-heap scan can't be safely pinned without an authoritative
    # contract count) -- but the decisions half above stays pinned regardless.
    raw_market = _pin_market_contracts(ctx, dm) or []
    raw_dec = raw2["decisions"]
    decisions: Dict[str, list] = {}
    for key, status in raw_dec.items():
        if not schema.valid_tag(key) or not isinstance(status, dict):
            continue
        running = status.get("active_timed_running")
        if not isinstance(running, list):
            continue
        decisions[key] = [
            {"decision": d.get("decision"), "days": d.get("days"),
             "state": d.get("state"), "state_name": d.get("state_name")}
            for d in running if isinstance(d, dict)
        ]
    contracts: list = []
    for c in raw_market:
        if not isinstance(c, dict):
            continue
        seller = c.get("seller")
        buyer = c.get("buyer")
        contracts.append({
            "contract_id": c.get("contract_id"),
            # resolved TAGs only (re-validated); a raw tag-db-id / buyer_ptr never reaches here.
            "seller": seller if schema.valid_tag(seller) else None,
            "buyer": buyer if schema.valid_tag(buyer) else None,
            "equip_variant_id": c.get("equip_variant_id"),
            "amount": c.get("amount"),
            "price": c.get("price"),
        })
    return {"decisions": decisions, "market": contracts,
            "count": len(decisions), "market_count": len(contracts)}


def _f_operative_xp():
    import final_residuals_live as _fr  # noqa
    import tech_live as _tl  # noqa
    return (_fr, _tl)


def _pin_operatives(ctx: CaptureContext, fr):
    """Resolve-or-REUSE the located OPERATIVE pool (the {op_id: dict} enumeration from intel_deep's
    (4713,op_id)-handle heap SWEEP -- the dominant operative_xp cost). On a warmed walk the sweep is
    SKIPPED: read_operative_skill_xp_from_operatives re-reads each operative's skill/xp LIVE off the
    pinned interior addresses. Invalidation (fail-closed, Codex-vetted): validate_operatives re-reads
    the (4713,op_id) handle at each pinned operative's identity_addr (type tag==4713 AND op_id==key,
    a true ABA defense); any mismatch / empty pool -> drop the pin + re-sweep. Returns the SKILL/XP
    map keyed by op_id (the SAME shape read_operative_skill_xp produces)."""
    p = ctx.proc

    def _resolve():
        ops = fr.enumerate_operatives(p, bridge=ctx.bridge) or {}
        if not ops:
            return None, {}, None                 # nothing located -> do NOT pin
        data = fr.read_operative_skill_xp_from_operatives(p, ops, bridge=ctx.bridge)
        return ops, data, len(ops)

    def _validate(ops, pop):
        return bool(fr.validate_operatives(p, ops))

    def _reread(ops):
        return fr.read_operative_skill_xp_from_operatives(p, ops, bridge=ctx.bridge)

    return _pin_get(ctx, "operatives", _resolve, _validate, _reread)


def _r_operative_xp(ctx: CaptureContext, impl) -> Dict[str, Any]:
    """LIVE operative SKILL + XP + tech in-progress UNITS (final_residuals_live, M). Reads
    ctx.proc READ-ONLY. ALLOW-LISTS only the two cracked sub-reads (NOT read_all, which also
    carries the RESISTED operations/upgrade-name reports + GT diagnostics).

      * operative skill/xp -> grouped by owner TAG into {TAG: {op_id: {name, skill, experience}}}.
        The operative body base is L = ident - 0x8 (the CORRECTED base; the reader already
        applies it). The diagnostic leader_addr is STRIPPED; owner_tag is re-validated via
        valid_tag. skill/experience may be None per operative (a guarded read miss) -> kept None.
      * tech progress -> {TAG: {slot_index: progress_units}}: progress_units = progress_raw/1000
        = accumulated research UNITS (NOT a completion fraction; the fraction is RESISTED, no
        ground truth this frame -> never fabricated). The assembler MERGES progress_units onto
        the existing research.in_progress_live slots (by slot_index), never replacing the list.

    STAGE 1.5: BOTH sub-reads' heap sweeps are now pinned. The operative pool is pinned via
    _pin_operatives (warm walk skips the operative sweep). The tech in-progress slots are derived
    from the SHARED "tech" pin (_pin_tech) -- the SAME located CTechnologyStatus/CResearchSlot
    objects tech_live already pins -- so operative_xp NEVER re-sweeps the tech pool (it computes
    progress_units = progress_raw/TECH_PROGRESS_SCALE off the pinned tech result).

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    fr, tl = impl
    if ctx.proc is None:
        raise RuntimeError("operative_xp needs ctx.proc (unavailable)")
    # OPERATIVE skill/xp -- pinned pool (warm: no sweep). Thread the shared bridge for naming.
    raw_ops = _pin_operatives(ctx, fr) or {}
    by_tag: Dict[str, Dict[str, Any]] = {}
    for op_id, o in raw_ops.items():
        if not isinstance(o, dict):
            continue
        owner = o.get("owner_tag")
        if not schema.valid_tag(owner):
            continue
        by_tag.setdefault(owner, {})[str(op_id)] = {
            "name": o.get("name"),
            "skill": o.get("skill"),            # may be None (guarded miss) -> kept
            "experience": o.get("experience"),  # may be None -> kept (never fabricated 0)
            # leader_addr (diagnostic heap addr) deliberately NOT carried.
        }
    # TECH in-progress UNITS -- derived from the SHARED tech pin (no second tech sweep). The tech
    # result's active_research carries progress_raw per slot; progress_units = raw/scale (the SAME
    # transform read_tech_progress applied, now off the pinned tech objects). A None active_research
    # (unmeasured) yields no units for that tag (never a fabricated 0).
    _idx_tag, _rec0, _count = _bridge_idx_tag(ctx)
    _scale = float(getattr(fr, "TECH_PROGRESS_SCALE", 1000.0))
    raw_tech = _pin_tech(ctx, tl, _idx_tag, _rec0, _count) or {}
    tech_units: Dict[str, Dict[str, Any]] = {}
    for tag, d in raw_tech.items():
        if not schema.valid_tag(tag) or not isinstance(d, dict):
            continue
        slot_units = {}
        for a in (d.get("active_research") or []):
            if not isinstance(a, dict):
                continue
            si = a.get("slot_index")
            pr = a.get("progress_raw")
            if si is not None and pr is not None:
                slot_units[int(si)] = pr / _scale
        if slot_units:
            tech_units[tag] = slot_units
    return {"operative_skill_xp": by_tag, "tech_progress_units": tech_units,
            "count": len(by_tag)}


def _f_politics_residuals():
    from politics_residuals_live import read_residuals  # noqa
    return read_residuals


def _pin_advisor_roster(ctx: CaptureContext):
    """Resolve-or-REUSE the located CAdvisor pool + its DECODED per-country roster.

    The advisor sweep -- leadership_deep_live.read_advisor_roster -> _find_instances over the
    CAdvisor vtable -- is a FULL committed-heap scan run on EVERY politics_residuals walk (the
    dominant ~52s warm cost). On a PAUSED, build-gated capture the CAdvisor object set is stable
    across walks, so the cold walk LOCATES the pool once (enumerate_advisor_addrs), DECODES the
    roster once, and pins both; a warmed walk returns the cached DECODED roster with NO heap sweep.

    Roots = {"addrs": [adv_addr,...], "snapshot": {adv_addr: (char_ptr, char_id)},
             "roster": {TAG: [{char_id, slot, idea_token}, ...]}}.

    Invalidation (fail-closed, mirrors the designs_variants decoded-catalog pin): validate re-reads
    EVERY pinned CAdvisor's vtable (== module_base + VT_ADVISOR_RVA) AND its linked CCharacter ptr
    AND that character's char_id == the locate-time snapshot (a true ABA defense -- a freed slot
    reused by a same-class object that re-linked a DIFFERENT character is caught), plus the pool
    COUNT. Any mismatch -> drop the pin + full re-sweep + re-decode. So the cached DECODED roster
    is only ever served while every CAdvisor object AND its character link are the SAME live objects
    (the game is paused; the snapshot is the belt-and-braces tripwire if the invariant ever leaked).

    Returns the decoded {TAG: [...]} roster (the SAME dict the cold path produced), or {} when no
    CAdvisor located / no live tag_table (honest absence -> the caller self-resolves the roster).
    The tagmap used for the TAG grouping is the SHARED live ctx.tag_table (re-read each resolve);
    its as_map() is captured into the decode but NOT pinned -- a tag_table change flips the global
    proc-fingerprint epoch only on a base/build change, so on a stable epoch the {tag_db_id: TAG}
    grouping is itself invariant (same posture as every other tag-grouped pinned reader)."""
    import leadership_deep_live as _ld
    p = ctx.proc
    if p is None or ctx.tag_table is None:
        return {}                                 # no proc / no naming -> caller self-resolves

    def _tagmap():
        try:
            return ctx.tag_table.as_map() or {}
        except Exception:
            return {}

    def _resolve():
        addrs, snapshot = _ld.enumerate_advisor_addrs(p)
        if not addrs:
            return None, {}, None                 # nothing located -> do NOT pin (honest absence)
        roster = _ld.read_advisor_roster_from_addrs(p, addrs, _tagmap())
        roots = {"addrs": addrs, "snapshot": snapshot, "roster": roster}
        return roots, roster, len(addrs)

    def _validate(roots, pop):
        addrs = roots.get("addrs") or []
        if pop is not None and len(addrs) != pop:
            return False
        return bool(_ld.validate_advisor_addrs(p, addrs, roots.get("snapshot")))

    def _reread(roots):
        return roots.get("roster") or {}          # decoded once; no sweep, no re-decode

    return _pin_get(ctx, "advisor_roster", _resolve, _validate, _reread)


def _r_politics_residuals(ctx: CaptureContext, read_residuals) -> Dict[str, Any]:
    """LIVE election residuals + appointed advisors (politics_residuals_live, M). Reads
    ctx.proc READ-ONLY.

    read_residuals composes ALL THREE residual groups (read_election_residuals +
    read_appointed_advisors, which itself joins read_country_politics + the advisor roster), so
    appointed advisors are actually read (Codex review #1: read_election_residuals alone does NOT
    carry appointed_advisors). It returns {TAG: {last_election, elections_allowed,
    appointed_tokens, _status_addr, ...}} (all 46/46 EXACT vs save). We keep ONLY schema-valid
    TAG keys, carry the three measured leaves, and STRIP the diagnostic _status_addr + the raw
    last_election_raw. appointed_advisors fold uses `appointed_tokens` (the sorted idea-token
    list of HIRED advisors -- the clean token list our schema leaf wants); a measured-empty []
    is a real "no advisor hired".

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("politics_residuals needs ctx.proc (unavailable)")
    # reuse the shared bridge for naming AND the live country-array base so the composed
    # PP-bridge sub-reads SKIP _locate_country_array (removes redundant relocate + the
    # stale-PP relocate that used to kill the walk). None -> the reader self-locates.
    bridge_idx_tag, _rec0, _count = _bridge_idx_tag(ctx)
    # STAGE-4 (the dominant ~52s warm cost): supply the PINNED advisor roster so read_residuals
    # SKIPS the CAdvisor committed-heap sweep in leadership_deep_live.read_advisor_roster. The pin
    # is fail-closed (see _pin_advisor_roster): a moved/freed/realloc'd CAdvisor re-sweeps. A {}
    # result (no pool located / no tag_table) -> roster=None -> read_residuals self-resolves (the
    # honest cold behaviour, NOT fabrication). Output is byte-identical to the self-resolving path.
    _roster = _pin_advisor_roster(ctx) or None
    raw = read_residuals(ctx.proc, bridge=bridge_idx_tag, rec0=_rec0, count=_count,
                         roster=_roster) or {}
    countries: Dict[str, Any] = {}
    for tag, rec in raw.items():
        if not schema.valid_tag(tag) or not isinstance(rec, dict):
            continue
        out = {}
        if rec.get("last_election") is not None:
            out["last_election"] = rec.get("last_election")
        if rec.get("elections_allowed") is not None:
            out["elections_allowed"] = rec.get("elections_allowed")
        # appointed_tokens is the MEASURED sorted idea-token list (read_appointed_advisors
        # always emits it, incl. a measured-empty [] = no advisor hired). _status_addr +
        # last_election_raw are diagnostics and are NOT carried.
        at = rec.get("appointed_tokens")
        if isinstance(at, list):
            out["appointed_advisors"] = list(at)
        if out:
            countries[tag] = out
    return {"countries": countries, "count": len(countries)}


def _f_intel_deep():
    import intel_deep_live  # noqa
    return intel_deep_live


def _pin_intel_operatives(ctx: CaptureContext, idl):
    """Resolve-or-REUSE the located active-operative IDENTITY map ({op_id: identity_addr}) so a
    warmed walk SKIPS the (4713,op_id)+dual-name committed-heap SWEEP (the dominant intel_deep
    cost). The MUTABLE emitted fields (name / owner_tag / detected) are RE-READ LIVE from each
    pinned identity each walk (read_operatives_from_idents) -- nothing decoded is cached (Codex:
    detected + owner_tag are mutable+emitted -> re-read live).

    Invalidation (fail-closed, Codex-vetted): validate_operatives_intel re-reads, at each pinned
    identity, the (4713,op_id) handle (i32@+0==4713 AND i32@+4==op_id, ABA defense) AND the
    operative DUAL-NAME signature (name@+0x18 == codename@+0x38, non-empty -- re-checked EVERY
    walk so a freed+reused non-operative type-4713 record can never fabricate a row); plus a
    count tripwire. Any mismatch / empty pool -> drop the pin + re-sweep.

    This is a SEPARATE pin from operative_xp's _pin_operatives (same identity pool, but a
    DIFFERENT emitted shape -- name/owner/detected here vs skill/xp there). Returns the
    {op_id: dict} roster (the SAME shape idl.read_operatives produces)."""
    p = ctx.proc
    bridge = ctx.bridge

    def _resolve():
        ops = idl.enumerate_operatives_intel(p, bridge=bridge) or {}
        if not ops:
            return None, {}, None                 # nothing located -> do NOT pin
        data = idl.read_operatives_from_idents(p, ops, bridge=bridge)
        return ops, data, len(ops)

    def _validate(ops, pop):
        return bool(idl.validate_operatives_intel(p, ops))

    def _reread(ops):
        return idl.read_operatives_from_idents(p, ops, bridge=bridge)

    return _pin_get(ctx, "intel_operatives", _resolve, _validate, _reread)


def _r_intel_deep(ctx: CaptureContext, idl) -> Dict[str, Any]:
    """LIVE operative roster + cryptology (intel_deep_live, M). Reads ctx.proc READ-ONLY.

      * read_operatives -> {op_id: {op_id, name, owner_tag, detected, leader_addr,
        identity_addr, owner_tag_db_id, ...}}. We group by owner TAG (valid_tag) and carry
        ONLY {op_id, name, detected}, STRIPPING leader_addr/identity_addr (heap addrs) +
        owner_tag_db_id (raw id). operatives 56/56 + per-country owner counts EXACT vs save.
      * read_cryptology -> {owner_tag: [{target_tag, amount, decrypting}]}. We re-key per owner
        into {TGT: {amount, decrypting}}, re-validating owner + target via valid_tag. 37/37 EXACT.

    Raises if ctx.proc is unavailable (offline -> domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("intel_deep needs ctx.proc (unavailable)")
    bridge = ctx.bridge
    # STAGE B: the operative roster is a full-heap (4713,op_id)+dual-name SWEEP -> pin the located
    # identity set so a warmed walk re-reads name/owner/detected LIVE without re-sweeping
    # (fail-closed -- see _pin_intel_operatives). Cryptology below stays cheap-live (bridge-driven,
    # no heap sweep). A None/empty pool -> {} (self-resolves; honest absence).
    raw_ops = _pin_intel_operatives(ctx, idl) or {}
    operatives_by_tag: Dict[str, list] = {}
    for op_id, o in raw_ops.items():
        if not isinstance(o, dict):
            continue
        owner = o.get("owner_tag")
        if not schema.valid_tag(owner):
            continue
        operatives_by_tag.setdefault(owner, []).append({
            "op_id": o.get("op_id"),
            "name": o.get("name"),
            "detected": o.get("detected"),
            # leader_addr/identity_addr/owner_tag_db_id (diagnostics + raw id) NOT carried.
        })
    raw_crypto = idl.read_cryptology(ctx.proc, bridge=bridge) or {}
    cryptology_by_tag: Dict[str, Dict[str, Any]] = {}
    for owner, targets in raw_crypto.items():
        if not schema.valid_tag(owner) or not isinstance(targets, list):
            continue
        m = {}
        for t in targets:
            if not isinstance(t, dict):
                continue
            tgt = t.get("target_tag")
            if not schema.valid_tag(tgt):
                continue
            m[tgt] = {"amount": t.get("amount"), "decrypting": t.get("decrypting")}
        if m:
            cryptology_by_tag[owner] = m
    return {"operatives_by_tag": operatives_by_tag,
            "cryptology_by_tag": cryptology_by_tag,
            "count": len(operatives_by_tag)}


def _f_buildings_live():
    from buildings_live import BuildingsLive  # noqa
    return BuildingsLive


def _pin_buildings_live(ctx: CaptureContext, BuildingsLive):
    """Resolve-or-REUSE the BuildingsLive root maps ({state_id: state_obj} +
    {province_id: record_base}). On a warmed walk the full state+province heap scan is
    skipped: from_roots rebuilds a FRESH live layer whose building vectors are re-read LIVE
    (per-state/province caches start empty). Invalidation (fail-closed): re-check spread
    SENTINELS on BOTH root maps -- each state sentinel's vtable == module_base+
    STATE_VTABLE_REL AND state_id == key; each province sentinel's record vtable ==
    module_base+RECORD_VTABLE_RVA. Plus both COUNTS unchanged. Any mismatch -> re-scan."""
    import state_detail_live as _sd
    import state_vp_live as _vp
    p = ctx.proc

    def _resolve():
        layer = BuildingsLive.resolve(p)
        if layer is None:
            return None, None, None
        # only pin if the resolved layer EXPOSES its root maps (a real layer); a
        # mock/fake layer is used directly + never pinned.
        roots = None
        pop = None
        states = getattr(layer, "states", None)
        prov_recs = getattr(layer, "prov_recs", None)
        if isinstance(states, dict) and isinstance(prov_recs, dict):
            roots = {"states": dict(states), "prov_recs": dict(prov_recs)}
            pop = (len(states), len(prov_recs))
        return roots, layer, pop

    def _validate(roots, pop):
        states = roots.get("states") or {}
        prov_recs = roots.get("prov_recs") or {}
        if pop is not None and (len(states), len(prov_recs)) != tuple(pop):
            return False
        st_vt = p.module_base + _sd.STATE_VTABLE_REL
        skeys = sorted(states)
        if not skeys:
            return False
        for sid in _sentinel_keys(skeys):
            obj = states[sid]
            if _vt_at(p, obj) != st_vt:
                return False
            try:
                if p.u32(obj + _sd.OFF_SID) != sid:    # ABA: same vtable, wrong id
                    return False
            except OSError:
                return False
        rec_vt = p.module_base + _vp.RECORD_VTABLE_RVA
        pkeys = sorted(prov_recs)
        if not pkeys:
            return False
        for pid in _sentinel_keys(pkeys):
            if _vt_at(p, prov_recs[pid]) != rec_vt:    # province record moved/freed
                return False
        return True

    def _reread(roots):
        return BuildingsLive.from_roots(p, roots["states"], roots["prov_recs"])

    return _pin_get(ctx, "buildings_live", _resolve, _validate, _reread)


def _r_buildings_live(ctx: CaptureContext, BuildingsLive) -> Dict[str, Any]:
    """LIVE per-state + per-province BUILDING levels (buildings_live, M). Reads ctx.proc
    READ-ONLY via the BuildingsLive layer (reuses state_detail / state_vp relocators).

    as_state_map()    -> {state_id: {building_name: level}}
    as_province_map() -> {province_id: {building_name: level}}
    Both validated exact vs the melt. Keys are raw ints (state/province ids, NOT tags); we
    stringify them for JSON.

    HONESTY NOTE (Codex review #2): the reader's as_state_map() omits a state whose building map
    is empty {} (it does not distinguish an unreadable bank from a measured-empty). So a state
    with genuinely zero buildings is simply ABSENT from the live read here -> its
    states.*.buildings leaf stays the save backstop's value (we never fabricate a {} for it).
    That is the conservative no-fabrication choice: we only fold + protect buildings for states
    the reader positively returned. A measured-empty {} we DO receive (if any) is preserved.

    Raises if the build gate fails or the read raises (domain honestly omitted)."""
    if ctx.proc is None:
        raise RuntimeError("buildings_live needs ctx.proc (unavailable)")
    layer = _pin_buildings_live(ctx, BuildingsLive)
    if layer is None:
        raise RuntimeError("buildings_live: could not resolve the buildings layer")
    states = {str(int(sid)): dict(b) for sid, b in (layer.as_state_map() or {}).items()}
    provinces = {str(int(pid)): dict(b) for pid, b in (layer.as_province_map() or {}).items()}
    if not states and not provinces:
        raise RuntimeError("buildings_live: no state or province building resolved "
                           "-- domain honestly omitted (not captured-empty)")
    return {"states": states, "provinces": provinces,
            "count": len(states) + len(provinces)}


def _f_designs_variants():
    import designs_mio_deep_live  # noqa
    return designs_mio_deep_live


_VAR_FP_MAX_ENTRIES = 4096   # sanity cap: a variant's module/upgrade array is tiny (<~64); a
                             # wild size means a stale/garbage header -> fail closed (return None).


def _variant_payload_fp(p, dmd, addr):
    """A per-variant payload fingerprint covering EVERY decoded element: the module-array
    (data ptr, size) + the upgrade-array (data ptr, size), PLUS the (key,val) of EVERY module
    entry and the value of EVERY upgrade entry. A realloc/resize changes a ptr/size; an in-place
    edit of ANY entry -- boundary OR interior -- changes that entry's qwords, so the fingerprint
    fails closed (Codex Design-A risk #1/#3: the root vtable/type/id alone does NOT prove the
    module/upgrade sub-arrays are unchanged, and a boundary-only fp MISSES a same-ptr/same-size
    INTERIOR element swap -- a module changed in a design slot while the designer UI is open on a
    paused frame -- which would otherwise serve a STALE cached decode).

    The arrays are tiny (a handful of entries, stride 0x10), so hashing every entry on each warm
    validate is far cheaper than the per-variant re-decode it gates (the slot-table voting). A size
    beyond _VAR_FP_MAX_ENTRIES is treated as an unreadable/garbage header -> None. Returns a tuple,
    or None if any read faults / the header is insane (-> validate fails closed -> full re-scan +
    re-decode). This no longer relies on the paused-no-mutation invariant for correctness."""
    try:
        md = p.u64(addr + dmd.MOD_DATA_OFF)
        ms = p.u32(addr + dmd.MOD_SIZE_OFF)
        ud = p.u64(addr + dmd.UPG_DATA_OFF)
        us = p.u32(addr + dmd.UPG_SIZE_OFF)
        if ms > _VAR_FP_MAX_ENTRIES or us > _VAR_FP_MAX_ENTRIES:
            return None                                   # insane size -> fail closed
        fp = [md, ms, ud, us]
        # EVERY module entry (stride 0x10: key u32 @+0x00, val u64 @+0x08) -- not just boundaries,
        # so an interior same-size swap is caught.
        if md and ms:
            for i in range(ms):
                e = md + i * 0x10
                fp.append(p.u32(e + 0x00))
                fp.append(p.u64(e + 0x08))
        # EVERY upgrade entry (stride 0x10, value qword @+0x08).
        if ud and us:
            for i in range(us):
                fp.append(p.u64(ud + i * 0x10 + 0x08))
    except OSError:
        return None
    return tuple(fp)


def _decode_variant_catalog(p, dmd, addrs):
    """Decode the FULL {variant_id(str): {modules, upgrade_levels}} catalog from the located
    variant addrs in ONE coherent pass (Codex Design-A #4: build the slot table from the SAME
    module read used to emit each variant, instead of build_slot_table re-reading modules and
    variant_module_map re-reading them AGAIN -- removes the redundant double decode + a TOCTOU
    window). Identical OUTPUT shape to the old two-call path (slot->module map + upgrade levels).
    Also returns the per-variant payload fingerprint map for the pin's warm validate gate."""
    import collections
    # read every variant's raw module list ONCE.
    modules_by_vid = {}
    upgrades_by_vid = {}
    payload_fp = {}
    for vid, addr in addrs.items():
        try:
            modules_by_vid[vid] = dmd.read_variant_modules(p, addr)
        except Exception:
            modules_by_vid[vid] = []
        try:
            upg = dmd.read_variant_upgrades(p, addr)
        except Exception:
            upg = None
        upgrades_by_vid[vid] = upg if isinstance(upg, list) else None
        payload_fp[vid] = _variant_payload_fp(p, dmd, addr)
    # derive the slot_table from those SAME module reads (mirrors build_slot_table's voting, but
    # off the cached live module lists + the save GT -- no second heap read).
    gt = dmd._load_variant_gt()
    slot_table = {}
    if gt:
        votes = collections.defaultdict(collections.Counter)
        cache = {}
        for vid, live in modules_by_vid.items():
            g = gt.get(vid)
            if not g:
                continue
            gmods = g.get("modules") or {}
            cache[vid] = (live, gmods)
            sc = collections.Counter(gmods.values())
            m2s = {m: s for s, m in gmods.items() if sc[m] == 1}
            lc = collections.Counter(m for _, m in live)
            for h, mod in live:
                if lc[mod] == 1 and mod in m2s:
                    votes[h][m2s[mod]] += 1
        slot_table = {h: c.most_common(1)[0][0] for h, c in votes.items()}
        for _ in range(3):
            for vid, (live, gmods) in cache.items():
                if not gmods:
                    continue
                remaining = dict(gmods)
                unknown = []
                for h, mod in live:
                    sn = slot_table.get(h)
                    if sn in remaining and remaining[sn] == mod:
                        del remaining[sn]
                    else:
                        unknown.append((h, mod))
                for h, mod in unknown:
                    cands = [s for s, m in remaining.items() if m == mod]
                    if len(cands) == 1:
                        votes[h][cands[0]] += 1
                        del remaining[cands[0]]
            slot_table = {h: c.most_common(1)[0][0] for h, c in votes.items()}
    # emit the catalog: map each variant's cached module list through the slot_table (same
    # duplicate-slot disambiguation as variant_module_map, but off the cached `live` list).
    catalog = {}
    for vid, live in modules_by_vid.items():
        rec = {}
        out = {}
        used = set()
        leftover = []
        for h, mod in live:
            sn = slot_table.get(h)
            if sn is not None and sn not in used:
                out[sn] = mod
                used.add(sn)
            else:
                leftover.append((h, mod))
        for h, mod in leftover:
            sn = slot_table.get(h)
            if sn is None:
                sn = "slot_%X" % h
            base = sn
            k = 1
            while sn in out:
                sn = "%s#%d" % (base, k)
                k += 1
            out[sn] = mod
        rec["modules"] = out
        upg = upgrades_by_vid.get(vid)
        if isinstance(upg, list):
            rec["upgrade_levels"] = list(upg)
        catalog[str(int(vid))] = rec
    return catalog, payload_fp


def _pin_designs_variants(ctx: CaptureContext, dmd):
    """Resolve-or-REUSE the FULLY-DECODED equipment-variant catalog. On a warmed walk BOTH the
    variant-vtable heap SWEEP (enumerate_variant_addrs) AND the per-variant module/upgrade decode
    (build_slot_table + variant_module_map + read_variant_upgrades over thousands of variants --
    the ~186s warm cost) are SKIPPED: the cold walk decodes the catalog once and pins it.

    Roots = {"addrs": {vid: object_addr}, "catalog": {vid_str: {modules, upgrade_levels}},
             "payload_fp": {vid: (mod_data, mod_size, upg_data, upg_size)}}.

    Invalidation (fail-closed, Codex Design-A hardened): validate re-checks EVERY pinned variant
    (not just sentinels) -- vtable(==module_base+EQ_VARIANT_VT_RVA), type(==70), id(==pinned key,
    ABA defense) -- AND the per-variant payload fingerprint (module/upgrade array data ptr+size +
    EVERY module/upgrade entry's qwords -- boundary AND interior), AND the pool COUNT. Any mismatch
    -> drop the pin + full re-scan + re-decode. So the cached DECODED catalog is only ever served
    while every variant object AND every byte of its module/upgrade sub-arrays are unchanged: the
    full-entry fingerprint catches an in-place same-ptr/same-size INTERIOR element swap (a module
    changed in a design slot on a paused designer frame), so correctness does NOT rely on the
    paused-no-mutation invariant (it is now only a perf assumption)."""
    p = ctx.proc

    def _resolve():
        addrs = dmd.enumerate_variant_addrs(p) or {}
        if not addrs:
            return None, {}, None
        addrs = dict(addrs)
        catalog, payload_fp = _decode_variant_catalog(p, dmd, addrs)
        roots = {"addrs": addrs, "catalog": catalog, "payload_fp": payload_fp}
        return roots, catalog, len(addrs)

    def _validate(roots, pop):
        addrs = roots["addrs"]
        payload_fp = roots["payload_fp"]
        if pop is not None and len(addrs) != pop:
            return False
        vt = p.module_base + dmd.EQ_VARIANT_VT_RVA
        for vid, a in addrs.items():
            try:
                if p.u64(a) != vt:                       # moved/freed/different class
                    return False
                if p.u32(a + 0x08) != dmd.VARIANT_TYPE:
                    return False
                if p.u32(a + dmd.VAR_ID_OFF) != vid:     # ABA: same vtable, wrong id
                    return False
            except OSError:
                return False
            cached_fp = payload_fp.get(vid)
            cur_fp = _variant_payload_fp(p, dmd, a)
            # FAIL CLOSED on an UNVERIFIABLE payload (Codex Q2): a None fp (read fault /
            # insane size) means we cannot prove the module/upgrade sub-arrays are unchanged.
            # A None==None compare would PASS and serve the cached catalog behind an
            # unverifiable variant -> re-decode instead (the cold pin may also have stored a
            # None; either side being None is a miss).
            if cached_fp is None or cur_fp is None:
                return False
            if cur_fp != cached_fp:
                return False                             # module/upgrade sub-array changed
        return True

    def _reread(roots):
        return roots["catalog"]                          # decoded once; no scan, no re-decode

    return _pin_get(ctx, "designs_variants", _resolve, _validate, _reread)


def _r_designs_variants(ctx: CaptureContext, dmd) -> Dict[str, Any]:
    """LIVE equipment-VARIANT module catalog (designs_mio_deep_live, M). Reads ctx.proc
    READ-ONLY. Folds onto the DISTINCT live-only leaf designs.equipment_variants_live (keyed by
    the bare live variant id), kept SEPARATE from the save "id:type"-keyed
    designs.equipment_variants so the two key spaces never collide (Codex review).

    The variant catalog ({variant_id: {modules: {slot->module}, upgrade_levels: [...]}}) is
    decoded once and PINNED (see _pin_designs_variants): a warmed walk returns the cached catalog
    with NO heap sweep and NO per-variant re-decode (the old ~186s warm cost). The object addr is
    a diagnostic heap addr and is NEVER carried. RESISTED: the upgrade NAME token (upgrade_levels
    are LEVELS only).

    Raises if ctx.proc is unavailable OR no variant resolved (a broken read -> honestly
    omitted rather than captured-empty)."""
    if ctx.proc is None:
        raise RuntimeError("designs_variants needs ctx.proc (unavailable)")
    catalog = _pin_designs_variants(ctx, dmd) or {}   # warmed walk: no scan, no re-decode
    if not catalog:
        raise RuntimeError("designs_variants: no equipment variant resolved "
                           "-- domain honestly omitted (not captured-empty)")
    # the catalog is already in the {vid_str: {modules, upgrade_levels}} shape; copy defensively
    # so a downstream mutation can never reach the pinned roots.
    variants = {vid: dict(rec) for vid, rec in catalog.items()}
    return {"equipment_variants": variants, "count": len(variants)}


def _f_division_equipment():
    import division_equipment_live  # noqa
    return division_equipment_live


def _pin_division_equipment(ctx: CaptureContext, deq):
    """Resolve-or-REUSE the CDivision body-address pool ({(owner_tag_db_id, div_id): body_addr}).
    On a warmed walk the full CDivision-vtable heap scan (_iter_division_bodies) is skipped:
    read_division_equipment_from_bodies re-reads each pinned body's equipment LIVE and re-gates
    it (vtable, type==4713, (owner,id)==pinned key -> ABA defense). Invalidation (fail-closed):
    validate re-checks spread SENTINEL bodies' vtable + type + (owner,id) and the pool COUNT;
    any mismatch -> full re-scan + re-pin. Returns the same list-of-dicts the non-cached path
    produced."""
    import struct as _struct
    p = ctx.proc

    def _read(bodies):
        return deq.read_division_equipment_from_bodies(
            p, bodies, tag_table=ctx.tag_table, drop_zero=True)

    def _resolve():
        bodies = deq.enumerate_division_bodies(p)
        if not bodies:
            return None, [], None
        return bodies, _read(bodies), len(bodies)

    def _validate(bodies, pop):
        if pop is not None and len(bodies) != pop:
            return False
        div_vt = p.module_base + deq.DIV_VT_RVA
        for key in _sentinel_keys(sorted(bodies)):   # (owner, did) tuples
            owner_id, did = key
            body = bodies[key]
            b = p.try_read(body, deq._BODY_HEAD)
            if len(b) < deq._BODY_HEAD:
                return False
            if _struct.unpack_from("<Q", b, 0)[0] != div_vt:
                return False
            if _struct.unpack_from("<I", b, deq.OFF_TYPE)[0] != deq.DIVISION_TYPE:
                return False
            live_id = _struct.unpack_from("<I", b, deq.OFF_ID)[0]
            live_owner = _struct.unpack_from("<I", b, deq.OFF_OWNER)[0]
            if (live_owner, live_id) != (owner_id, did):     # ABA: wrong (owner,id)
                return False
        return True

    return _pin_get(ctx, "division_equipment", _resolve, _validate, _read)


def _r_division_equipment(ctx: CaptureContext, deq) -> Dict[str, Any]:
    """LIVE per-division held-equipment inventory (division_equipment_live, M). Reads ctx.proc
    READ-ONLY. Needs the live tag table to resolve each division's owner tag-db-id -> TAG
    (div_id is NOT globally unique; (owner_TAG, div_id) is the join key). We carry the per-
    division equipment {variant_id: count} keyed by "TAG|div_id"; the raw owner_tag_db_id is
    STRIPPED. Validated vs the melt ground truth.

    Raises if ctx.proc / the tag table is unavailable (domain honestly omitted, no fabrication)."""
    if ctx.proc is None:
        raise RuntimeError("division_equipment needs ctx.proc (unavailable)")
    if ctx.tag_table is None:
        raise RuntimeError("division_equipment needs the tag table to key by (TAG, div_id)")
    recs = _pin_division_equipment(ctx, deq)   # warmed walk: no heap scan
    by_div: Dict[str, Any] = {}
    for r in (recs or []):
        if not isinstance(r, dict):
            continue
        owner = r.get("owner")
        did = r.get("div_id")
        if not schema.valid_tag(owner) or not isinstance(did, int):
            continue
        eq = r.get("equipment")
        # the reader coerces an UNREADABLE/rejected bank to {} (NOT distinguishable from a
        # genuine measured-empty), so a {} here is AMBIGUOUS -> we conservatively DROP it
        # rather than fold a possibly-fabricated empty bank (Codex review #4). A division with
        # real held equipment always has a non-empty map.
        if not isinstance(eq, dict) or not eq:
            continue
        by_div["%s|%d" % (owner, did)] = {
            "owner": owner,
            "div_id": did,
            # {variant_id(str): count_held}; the raw owner_tag_db_id is NOT carried.
            "equipment": {str(int(k)): v for k, v in eq.items()},
        }
    if not by_div:
        raise RuntimeError("division_equipment: no division resolved an owner+equipment "
                           "-- domain honestly omitted (not captured-empty)")
    return {"by_div": by_div, "count": len(by_div)}


def _read_country_field(p, obj_addr, spec) -> Optional[Any]:
    """Read one scalar from a country object given an offset spec from the draft.

    spec is expected to look like {"offset": "0x...", "type": "f32"|"u32"|...,
    "scale": <float>} (or a subset). Returns None if the spec is missing/unpinned
    or the read fails -- NEVER a guessed value. A read that lands out of a declared
    sane range returns None (garbage, not a measured value)."""
    if not isinstance(spec, dict):
        return None
    off = spec.get("offset")
    if off is None:
        return None
    try:
        off = int(off, 16) if isinstance(off, str) else int(off)
    except (TypeError, ValueError):
        return None
    typ = (spec.get("type") or "u32").lower()
    try:
        addr = obj_addr + off
        if typ in ("f32", "float"):
            val = p.f32(addr)
        elif typ in ("f64", "double"):
            val = p.f64(addr)
        elif typ in ("i32", "int"):
            val = p.i32(addr)
        elif typ in ("u16",):
            val = p.u16(addr)
        else:
            val = p.u32(addr)
    except Exception:
        return None
    scale = spec.get("scale")
    if scale is None:
        # Defense-in-depth: a fixed-point field may declare its divisor only in the
        # free-text "encoding" (e.g. "fixed_point /100000 ...") without an explicit
        # "scale" key. Derive it so the value isn't returned raw (this masked a real
        # bug: stability reached the snapshot as 75000 instead of 0.75). A raw-count
        # encoding ("raw integer count") has no "/N" -> no division.
        import re as _re
        _m = _re.search(r"/\s*(\d{3,})", str(spec.get("encoding") or ""))
        if _m:
            scale = int(_m.group(1))
    if scale:
        try:
            val = val / float(scale)
        except (TypeError, ZeroDivisionError):
            return None
    lo = spec.get("sane_min")
    hi = spec.get("sane_max")
    if lo is not None and val < lo:
        return None
    if hi is not None and val > hi:
        return None
    return val


# ---------------------------------------------------------------------------
# The default registry.
# ---------------------------------------------------------------------------
def default_registry() -> List[ReaderPlug]:
    """The built-in plugs, in a sensible read order. Combat + (future) weather
    are import-guarded and report unavailable until cracked."""
    return [
        ReaderPlug("state_control", _f_state_control, _r_state_control),
        ReaderPlug("province_control", _f_province_control, _r_province_control),
        ReaderPlug("divisions", _f_divisions, _r_divisions),
        ReaderPlug("country_scalars", _f_country_scalars, _r_country_scalars),
        # NEW live domains wired in this pass (all DONE + validated live==save).
        # Each reuses the resolved ctx layers where it can; designs/ships open
        # their own ctypes handle and are same-process-verified before use.
        ReaderPlug("state_detail", _f_state_detail, _r_state_detail),
        # LIVE per-state VICTORY POINTS -> the SAME canonical save home
        # (states.*.victory_points_total + .victory_points_by_province) under the
        # DISTINCT "state_vp" id (live wins; protected via _LIVE_STATE_DOMAIN_LEAVES in
        # fold_save_bundle). Enumerates the full state asset so a zero-VP state carries a
        # MEASURED 0. Reuses ctx.proc.
        ReaderPlug("state_vp", _f_state_vp, _r_state_vp),
        ReaderPlug("country_misc", _f_country_misc, _r_country_misc),
        # LIVE per-country STATIC scalars -> country.identity.{original_capital,
        # original_tag, reserved_dynamic_country, templates_locked} (M; distinct
        # live-only leaves). Reuses the resolved bridge's country_object pointer.
        ReaderPlug("country_static", _f_country_static, _r_country_static),
        # LIVE per-country SCRIPT-VARIABLES bag -> country.script_variables (M; the
        # mass closure for the nation scripted-GUI overlays). Reuses the bridge;
        # fail-closed (key-count == size header or {}). Reuses ctx.proc.
        ReaderPlug("country_variables", _f_country_variables, _r_country_variables),
        # LIVE per-country CONTAINER fields -> country.identity.{cores, claims,
        # naval_headquarter_status} + country.decisions.status_present +
        # country.country_reports. cores/claims are inverted from per-state vectors.
        ReaderPlug("country_containers", _f_country_containers, _r_country_containers),
        # LIVE per-state extra fields -> states.*.{resources, dynamic_modifier,
        # extra_shared_slots} (M; resources shares the save home, the others are
        # distinct live-only leaves). Reuses the state-object relocator.
        ReaderPlug("state_fields", _f_state_fields, _r_state_fields),
        ReaderPlug("designs", _f_designs, _r_designs),
        ReaderPlug("mio", _f_mio, _r_mio),
        ReaderPlug("ships", _f_ships, _r_ships),
        # LIVE OOB command hierarchy -> the SAME canonical save home (top-level
        # order_of_battle.{armies,army_groups,fleets,theatres}) under the DISTINCT
        # "oob_command" id (live wins; protected via _protect_top in fold_save_bundle).
        # The per-SHIP order_of_battle.ships leaf stays "ships"-owned. Reuses ctx.proc.
        ReaderPlug("oob_command", _f_oob_command, _r_oob_command),
        # LIVE special-projects / facilities / scientists pools (M; uses ctx.proc).
        # Folded into top-level snap["special_projects"]; provenance domain id is
        # "projects_live" (distinct from the SAVE-sourced canonical "projects").
        ReaderPlug("projects_live", _f_projects, _r_projects),
        # LIVE per-country domains that fold into the SAME canonical save HOMES
        # (research / commanders / intel) but under DISTINCT *_live provenance ids
        # so the live channel never collides with the save channel's "tech" /
        # "commanders" / "intel" ids. Live WINS at the leaves they measure (the save
        # backstop protects them via _LIVE_COUNTRY_DOMAIN_LEAVES). tech/intel reuse
        # ctx.proc + the resolved bridge; commanders opens its own same-process handle.
        ReaderPlug("tech_live", _f_tech_live, _r_tech_live),
        ReaderPlug("commanders_live", _f_commanders_live, _r_commanders_live),
        ReaderPlug("intel_live", _f_intel_live, _r_intel_live),
        # LIVE national-focus state -> the SAME canonical save home (country.focus) under
        # the DISTINCT "focus_live" id (live wins; protected via _LIVE_COUNTRY_DOMAIN_
        # LEAVES). Reuses ctx.proc + the resolved bridge.
        ReaderPlug("focus_live", _f_focus_live, _r_focus_live),
        # LIVE production -> a NEW live-only home (country.production) under the DISTINCT
        # "production_live" id (the save "production" domain folds into economy/military,
        # so no collision). Reuses ctx.proc (the reader resolves its own country objects).
        ReaderPlug("production_live", _f_production_live, _r_production_live),
        # LIVE diplomacy -> country.diplomacy.{at_war,casualties,faction_group} (M) + the
        # NEW top-level sidecar snap["diplomacy_live"] (war_relations + faction_members),
        # under the DISTINCT "diplomacy_live" id (the save "diplomacy" folds diplomacy.wars
        # + diplomacy_world, a different identity model -> no collision). The reader resolves
        # its OWN country identity (vtable scan), so it uses ctx.proc only.
        ReaderPlug("diplomacy_live", _f_diplomacy_live, _r_diplomacy_live),
        # LIVE bilateral RELATIONS -> country.diplomacy.{opinion (live-first onto the save-shared
        # leaf), relation_flags, overlord, autonomy_state, autonomy_ratio} under the DISTINCT
        # "diplo_relations" id (distinct from diplomacy_live's wars/factions). The reader resolves
        # its OWN country identity (reuses diplomacy_live's vtable scan), so it uses ctx.proc only.
        ReaderPlug("diplo_relations", _f_diplo_relations, _r_diplo_relations),
        # LIVE politics -> country.politics (party_popularities/ruling_party/ideas/...) under
        # the DISTINCT "politics_live" id (shares the save politics home; live wins on the
        # measured leaves). political_power stays country_scalars-owned (single live owner).
        # Reuses ctx.proc + the resolved bridge.
        ReaderPlug("politics_live", _f_politics_live, _r_politics_live),
        ReaderPlug("supply", _f_supply, _r_supply),
        # LIVE per-country national FUEL -> the SAME canonical save home
        # (country.economy.fuel.{current,max}) under the DISTINCT "fuel_live" id (live wins;
        # protected via _LIVE_COUNTRY_DOMAIN_LEAVES). Portable pointer chain; reuses ctx.proc
        # (the reader resolves its own country identity via the shared intel_live bridge).
        ReaderPlug("fuel_live", _f_fuel_live, _r_fuel_live),
        # LIVE supply solver INPUTS (efficiency/draw per province) -> badged signals added to
        # the supply block (snap["supply"].solver_efficiency_by_province / ..._draw...). These
        # are NOT the exact per-province supply (provably not stored); the existing division-
        # province proxy under the "supply" domain is left untouched. Reuses ctx.proc.
        ReaderPlug("supply_inputs", _f_supply_inputs, _r_supply_inputs),
        # LIVE EXTRACT-RESIDUAL domains wired from the deep-crack push. Each folds onto a SAVE-
        # shared home under a DISTINCT *_live id OR a NEW live-only home/leaf; all reuse ctx.proc
        # (+ the resolved bridge / tag table where needed) and validate live==save.
        # world scenario DATES -> world.scenario.{current_date_live,start_date_live} (M; cheap
        # pointer chain, NOT the expensive key-presence heap scan).
        ReaderPlug("world_meta", _f_world_meta, _r_world_meta),
        # construction queue + battle-plan orders + opinion-modifier decomposition -> country.
        # production.construction_queue + order_of_battle.battle_plan_orders + country.diplomacy.
        # opinion_modifiers (all M, distinct live-only leaves; target geometry/token names RESISTED).
        ReaderPlug("construction_orders", _f_construction_orders, _r_construction_orders),
        # active timed DECISIONS + global equipment-MARKET contracts -> country.decisions.timed_live
        # + the NEW top-level snap["market_live"] sidecar (M; distinct from the SAVE "market" id).
        ReaderPlug("decisions_market", _f_decisions_market, _r_decisions_market),
        # operative SKILL + XP (corrected body base L=ident-0x8) + tech in-progress UNITS ->
        # country.intel.operative_skill_xp + merged onto research.in_progress_live (progress_units;
        # the completion FRACTION stays RESISTED). Allow-lists only the two cracked sub-reads (M).
        ReaderPlug("operative_xp", _f_operative_xp, _r_operative_xp),
        # election residuals + appointed advisors -> country.politics.{last_election,
        # elections_allowed,appointed_advisors} (M, distinct live-only leaves; 46/46 EXACT).
        ReaderPlug("politics_residuals", _f_politics_residuals, _r_politics_residuals),
        # LIVE operative roster + cryptology -> country.intel.{operatives_live,cryptology_live}
        # (M, distinct live-only leaves from the save-shaped intel.operatives; 56/56 + 37/37 EXACT).
        ReaderPlug("intel_deep", _f_intel_deep, _r_intel_deep),
        # LIVE per-state + per-province BUILDING levels -> states.*.buildings + provinces.*.buildings
        # (M; shares the save home, distinct live id). Reuses the state_detail/state_vp relocators.
        ReaderPlug("buildings_live", _f_buildings_live, _r_buildings_live),
        # LIVE equipment-VARIANT module catalog -> designs.equipment_variants_live (M; a DISTINCT
        # live-only leaf, separate from the save "id:type"-keyed equipment_variants). slot->module
        # map + upgrade levels (upgrade NAMES resisted).
        ReaderPlug("designs_variants", _f_designs_variants, _r_designs_variants),
        # LIVE per-division held-equipment inventory -> divisions[].equipment_live (M; a DISTINCT
        # live-only dict leaf, separate from the save-owned equipment list -> no clobber). Needs
        # the tag table to key by (owner TAG, div_id).
        ReaderPlug("division_equipment", _f_division_equipment, _r_division_equipment),
        ReaderPlug("naval_air", _f_sea_control, _r_sea_control),
        ReaderPlug("land_combats", _f_land_combats, _r_land_combats),
        ReaderPlug("combats", _f_combats, _r_combats),
        # Save-backstop plugs: OPT-IN only (they raise unless a save path is in
        # ctx.extras), so they are absent by default in a pure-live capture. They
        # are registered so a caller that DOES supply a melt path gets them, but
        # they never fabricate freshness (Codex Q2).
        ReaderPlug("buildings", _f_buildings_save, _r_buildings_save),
        ReaderPlug("intel", _f_intel_save, _r_intel_save),
        # weather: intentionally NOT registered -- no cheap, true weather read
        # exists yet (see the weather note above). Absent != fabricated.
    ]


def read_all(ctx: CaptureContext,
             plugs: Optional[List[ReaderPlug]] = None) -> Dict[str, DomainResult]:
    """Run every plug once; return {domain_name: DomainResult}. Never raises for a
    plug failure (each plug's read() is already transactional)."""
    if plugs is None:
        plugs = default_registry()
    out: Dict[str, DomainResult] = {}
    for plug in plugs:
        out[plug.name] = plug.read(ctx)
    return out


__all__ = [
    "CaptureContext", "DomainResult", "ReaderPlug",
    "default_registry", "read_all",
]
