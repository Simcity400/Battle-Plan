"""Expand a sealed manifest's recording-gap ledger back into flat rows.

``compact_stream.gaps`` and ``compact_stream.domain_gap_outcomes`` used to be
flat arrays with one JSON object per row -- 33.5 KB of manifest per game hour on
the 683-hour t44c capture, ~3.4 GB over a full campaign. They are now one
run-encoded ``compact_stream.gap_ledger`` object; see
``extractor/native/recorder_host/src/gap_ledger.rs`` for the shape, which this
module is the offline reader for.

Every consumer that used to iterate the flat arrays calls :func:`expand_gaps` or
:func:`expand_domain_gap_outcomes` instead and gets the same row dicts, so the
grading logic on top of them is unchanged.

ONE FORMAT. There is deliberately no fallback to the flat arrays (owner ruling
2026-08-09: every capture sealed before this is disposable test data), and a
``gap_ledger`` whose schema/version is not the one this module implements is
REFUSED rather than guessed at. A manifest with no ledger RAISES, because "no
ledger" and "no gaps" are different claims and a consumer that returned zero for
both would report a capture as clean on the strength of a document nobody wrote a
ledger into.
"""

from __future__ import annotations

from typing import Any

GAP_LEDGER_SCHEMA = "hoi4-gap-ledger-v2"
GAP_LEDGER_VERSION = 2

GAP_REASONS = frozenset({
    "coalesced", "skipped_hour", "source_unavailable", "partial_data",
    "bridge_unavailable", "worker_panic", "torn_sweep", "coverage_shortfall",
    "backpressure",
})
GAP_COUNTERS = frozenset({
    "coalesced_hours", "queue_overrun_hours", "sink_backpressure_hours",
})
DOMAIN_GAP_REASONS = frozenset({
    "torn", "overrun", "backpressure", "clock_unreadable", "worker_panic",
    "source_unavailable", "reader_silent", "hour_label_mismatch",
})
DOMAIN_GAP_PARTS = frozenset({"state", "event", "both"})


class GapLedgerError(Exception):
    """The ledger could not be read. Never raised for an EMPTY ledger."""


def _is_integer(value: Any) -> bool:
    """A JSON integer, not a bool and not a float that happens to be whole.

    ``True`` is an ``int`` in Python and ``2.0 == 2``, so the obvious test admits
    ``version: 2.0`` and ``row_count: True``. The recorder writes neither, and a
    reader that accepts what the writer cannot produce is a reader with a second,
    undocumented format.
    """
    return isinstance(value, int) and not isinstance(value, bool)


def _ledger(compact_stream: Any) -> dict[str, Any]:
    """The compacted ledger. Raises when the manifest carries none.

    RAISES rather than returning ``None``: a caller that mistook "no ledger" for
    "no gaps" would report a capture as clean on the strength of a document
    nobody wrote a ledger into, which is the strongest false claim this module
    can make. Callers that legitimately need the predicate ask
    :func:`ledger_present`.

    Both the schema tag and the version have to match EXACTLY: a document that
    happens to share a version number is not this document, and reading it as one
    would publish hours nobody recorded.
    """
    if not isinstance(compact_stream, dict):
        raise GapLedgerError("compact_stream is not an object")
    ledger = compact_stream.get("gap_ledger")
    if ledger is None:
        raise GapLedgerError("compact_stream carries no gap_ledger")
    if not isinstance(ledger, dict):
        raise GapLedgerError("gap_ledger is not an object")
    schema = ledger.get("schema")
    _check_fields(ledger, frozenset({
        "schema", "version", "note", "gaps", "domain_gap_outcomes",
    }), "gap_ledger")
    if "note" in ledger and not isinstance(ledger["note"], str):
        raise GapLedgerError("gap_ledger.note is not a string")
    version = ledger.get("version")
    if schema != GAP_LEDGER_SCHEMA or not _is_integer(version) or version != GAP_LEDGER_VERSION:
        raise GapLedgerError(
            f"gap_ledger schema/version {schema!r}/{version!r} is not "
            f"{GAP_LEDGER_SCHEMA!r}/{GAP_LEDGER_VERSION}; refused rather than guessed"
        )
    return ledger


def ledger_present(compact_stream: Any) -> bool:
    """Whether the manifest states a gap ledger AT ALL.

    "No ledger" and "an empty ledger" are different claims and a gate that
    conflates them passes vacuously on a manifest nobody wrote one into. The
    expanders raise on the first; this is the predicate for a caller that must
    report the difference rather than propagate an exception.
    """
    if not isinstance(compact_stream, dict):
        return False
    return isinstance(compact_stream.get("gap_ledger"), dict)


I32_MIN = -(2 ** 31)
I32_MAX = 2 ** 31 - 1


def _required_int(container: Any, key: str, what: str) -> int:
    """A field that must be present and must be a JSON integer.

    Indexing the dict directly raised ``KeyError`` for a class missing a field,
    which is not a :class:`GapLedgerError` -- so the two callers that catch this
    module's own error terminated instead of reporting a problem. A validator
    that dies mid-manifest publishes no verdict at all.
    """
    if not isinstance(container, dict) or key not in container:
        raise GapLedgerError(f"{what} is missing")
    value = container[key]
    if not _is_integer(value):
        raise GapLedgerError(f"{what} is not an integer")
    return value


def _i32(value: int, what: str) -> int:
    """A game hour. Out of range is refused, never truncated.

    Hours are `i32` on the wire and in the recorder's own reader. A reader that
    admitted `2147483648` would attribute a fact to an hour that cannot exist and
    that the producer's own decoder refuses -- two readers of one document
    disagreeing about which hour a gap belongs to.
    """
    if value < I32_MIN or value > I32_MAX:
        raise GapLedgerError(f"{what} = {value} is outside the i32 hour range")
    return value


def _name(container: Any, key: str, what: str) -> str:
    """A field that must be present and must be a non-empty JSON string.

    Copying it unchecked was not caught by the exception guard, because nothing
    THREW: the expander returned rows whose `reason` was a dict, and the callers
    then died grouping by it (`TypeError: unhashable type`). A wrapper cannot
    bound a failure that happens after it returns; the field has to be checked
    where it is read.
    """
    if not isinstance(container, dict) or key not in container:
        raise GapLedgerError(f"{what} is missing")
    value = container[key]
    if not isinstance(value, str) or not value:
        raise GapLedgerError(f"{what} is not a name")
    return value


def _check_class_count(table: Any, actual: int, what: str) -> None:
    """`class_count` is published beside the class list and must agree with it.

    A count that disagrees is a second claim about the same list, and a reader
    that ignores it lets two documents differ in a field neither reads.
    """
    declared = table.get("class_count")
    if not _is_integer(declared):
        raise GapLedgerError(f"{what}.class_count is not an integer")
    if declared != actual:
        raise GapLedgerError(
            f"{what}.class_count is {declared} but the list holds {actual}")


GAP_CLASS_FIELDS = frozenset({
    "reason", "grouping", "domains", "span_hours", "witness", "hour_runs", "repeat",
})
OUTCOME_CLASS_FIELDS = frozenset({
    "reason", "part", "domains", "queue_capacity", "queue_depth", "queue_dropped",
    "range_start_rel", "range_end_rel", "tick_before", "tick_after", "hour_runs",
    "ordinals",
})
LANE_ENTRY_FIELDS = frozenset({"domain_id", "part", "first"})
WITNESS_FIELDS = {
    "counter": frozenset({"kind", "counter", "observed"}),
    "hour_discontinuity": frozenset({"kind", "previous_hour_rel", "next_hour_rel"}),
    "status_bits": frozenset({"kind", "status_bits"}),
    "coverage": frozenset({"kind", "expected_perf_records", "observed_perf_records",
                           "expected_data_records", "observed_data_records"}),
    "coverage_and_status": frozenset({"kind", "expected_perf_records",
                                      "observed_perf_records", "expected_data_records",
                                      "observed_data_records", "status_bits"}),
    "torn_sweep": frozenset({"kind", "tick_start", "tick_end"}),
}


def _check_fields(value: Any, allowed: frozenset[str], what: str) -> None:
    """Refuse a value carrying any field outside its exact allowed set.

    THE STRUCTURAL CLOSE, not another per-field rule. Every "ignored field"
    defect this ledger has had is one instance of the same thing: a document
    carrying something no reader uses expands to exactly what the document
    without it does. Worse, an unread field still counts toward a class's
    IDENTITY, so adding one splits an aggregation key and walks through the
    same-hour check. A closed field set ends the family.
    """
    if not isinstance(value, dict):
        raise GapLedgerError(f"{what} is not an object")
    unexpected = set(value) - allowed
    if unexpected:
        raise GapLedgerError(
            f"{what} carries {sorted(unexpected)!r}, which this format does not define")


def _class_identity(class_: Any, *, without_domains: bool = False) -> str:
    """A class's identity: every field except the ones naming its hours.

    Two classes with one identity would have been ONE class, and splitting a run
    across them expands to exactly what the merged class does -- a second
    document with one meaning, which the within-class maximal-run rule alone does
    not close.
    """
    import json as _json
    dropped = {"hour_runs", "ordinals"}
    if without_domains:
        dropped.add("domains")
    return _json.dumps(
        {key: value for key, value in class_.items() if key not in dropped},
        sort_keys=True, separators=(",", ":"), default=str)


def _classes(table: Any, what: str) -> list[Any]:
    """A table's class list, which must BE a list.

    `table.get("classes") or []` read a missing list as an empty one, so a
    malformed table with no classes at all graded as a capture with no gaps --
    and the recorder's own reader refuses it. Two readers of one document
    disagreeing about whether it says anything.
    """
    classes = table.get("classes")
    if not isinstance(classes, list):
        raise GapLedgerError(f"{what}.classes is not an array")
    _check_class_count(table, len(classes), what)
    return classes


def _domains(class_: dict[str, Any]) -> list[int]:
    """A class's affected-domain list, type-checked.

    A domain id is a `u16` on the wire. A list holding a string expanded into
    rows whose `domain_id` was a string, which the callers then group and count
    by -- a domain that exists in the report and nowhere in the game.
    """
    raw = class_.get("domains")
    if raw is None:
        raise GapLedgerError("class.domains is missing")
    if not isinstance(raw, list):
        raise GapLedgerError("class.domains is not an array")
    for domain in raw:
        if not _is_integer(domain) or domain < 0 or domain > 0xFFFF:
            raise GapLedgerError(f"class.domains entry {domain!r} is not a domain id")
    if any(a > b for a, b in zip(raw, raw[1:])):
        raise GapLedgerError("class.domains is not in ascending order")
    return list(raw)


def _runs(class_: dict[str, Any], limit: int | None = None) -> list[int]:
    """Every hour a class names, in order, WITH multiplicity.

    A run is ``[first_hour, count]`` and names ``count`` consecutive hours. A
    zero count is refused: it would let two different class lists mean the same
    expansion, and the encoder never writes one.
    """
    hours: list[int] = []
    raw = class_.get("hour_runs")
    if not isinstance(raw, list):
        raise GapLedgerError("class.hour_runs is not an array")
    if not raw:
        raise GapLedgerError("class.hour_runs is empty, so the class names no row")
    for run in raw:
        if not isinstance(run, list) or len(run) != 2:
            raise GapLedgerError("class.hour_runs entry is not a [first, count] pair")
        first, count = run
        if not isinstance(first, int) or isinstance(first, bool):
            raise GapLedgerError("class.hour_runs first is not an integer")
        if (not isinstance(count, int) or isinstance(count, bool) or count < 1
                or count > 0xFFFFFFFF):
            raise GapLedgerError("class.hour_runs count is not a u32 above zero")
        if limit is not None and len(hours) + count > limit:
            raise GapLedgerError(
                f"a class names more than the {limit} rows the table declares")
        _i32(first, "class.hour_runs first hour")
        _i32(first + count - 1, "class.hour_runs last hour")
        if hours:
            if first < hours[-1] + 1:
                raise GapLedgerError("class.hour_runs is not ascending")
            if first == hours[-1] + 1:
                raise GapLedgerError("class.hour_runs splits a run; runs must be maximal")
        hours.extend(range(first, first + count))
    return hours


def _bounded_runs(class_: dict[str, Any], declared: int,
                  hours_claimed: set[int]) -> list[int]:
    """The class's hours, bounded by the work the table admits to.

    A run count is a `u32`, so an unbounded walk is 4.29 billion hours on a
    ledger nobody wrote -- a hang rather than a verdict. A class cannot name more
    hours than the table has rows, and `row_count` is itself checked against the
    expansion, so this bounds the work by a number the document already has to be
    honest about.

    Also claims each hour for this class's aggregation key, so two classes
    differing only in their domain list cannot both name it.
    """
    per_hour = 1
    if class_.get("grouping") == "shared":
        per_hour = class_.get("repeat") if _is_integer(class_.get("repeat")) else 1
    else:
        per_hour = max(len(class_.get("domains") or []), 1)
    hours = _runs(class_, max(declared, 0) // max(per_hour, 1))
    for hour in hours:
        if hour in hours_claimed:
            raise GapLedgerError(
                f"two classes differing only in their domains both name hour {hour}; "
                "the encoder aggregates a hour's domains into one class")
        hours_claimed.add(hour)
    return hours


def _anchored(field: Any, anchor: int, what: str) -> int:
    """Resolve a tick stated either relative to its hour or absolutely.

    Exactly one spelling is ever written. A field carrying both, or neither, has
    no single meaning, and choosing one would be inventing the witness.
    """
    if not isinstance(field, dict):
        raise GapLedgerError(f"{what} is not an object")
    _check_fields(field, frozenset({"rel", "abs"}), what)
    has_rel = "rel" in field
    has_abs = "abs" in field
    if has_rel == has_abs:
        raise GapLedgerError(f"{what} must carry exactly one of `rel` and `abs`")
    if has_abs:
        return _u64(field, "abs", f"{what}.abs")
    resolved = anchor + _i64(field, "rel", f"{what}.rel")
    if resolved < 0 or resolved > 0xFFFFFFFFFFFFFFFF:
        raise GapLedgerError(f"{what} resolves to {resolved}, outside the u64 tick range")
    return resolved


def _lane_part(entry: Any) -> str:
    """A lane's `part`, checked against the contract vocabulary."""
    part = _name(entry, "part", "lane_ordinal_first.part")
    if part not in DOMAIN_GAP_PARTS:
        raise GapLedgerError(f"unknown gap part {part!r}")
    return part


def _u32(container: Any, key: str, what: str) -> int:
    """A count or status word: a JSON integer inside the `u32` domain.

    UNSIGNED, because the contract's field is. A reader that admits a negative
    count where the recorder's own decoder requires a `u32` hands its caller a
    figure the producer could not have written.
    """
    value = _required_int(container, key, what)
    if value < 0 or value > 0xFFFFFFFF:
        raise GapLedgerError(f"{what} = {value} is outside the u32 range")
    return value


def _u64(container: Any, key: str, what: str) -> int:
    """A tick, an ordinal or a drop count: a JSON integer inside `u64`."""
    value = _required_int(container, key, what)
    if value < 0 or value > 0xFFFFFFFFFFFFFFFF:
        raise GapLedgerError(f"{what} = {value} is outside the u64 range")
    return value


def _i64(container: Any, key: str, what: str) -> int:
    """An OFFSET: signed, and bounded by what the encoder can write.

    `Anchored::Rel` is an `i64` in the recorder's own reader, so an offset
    outside that range is one no encoder produced and one the producer's decoder
    refuses -- even when it happens to resolve to a tick that fits.
    """
    value = _required_int(container, key, what)
    if value < -(2 ** 63) or value > 2 ** 63 - 1:
        raise GapLedgerError(f"{what} = {value} is outside the i64 offset range")
    return value


def _witness(template: Any, hour_start: int) -> dict[str, Any]:
    """Rebuild a flat witness object from a class's template.

    Every payload field is REBUILT AND TYPE-CHECKED rather than copied. Copying
    the template wholesale let a `status_bits` of `{}` through, which the
    recorder's own reader refuses and which reaches a caller as a row field it
    then masks and compares.
    """
    if not isinstance(template, dict):
        raise GapLedgerError("class.witness is not an object")
    kind = _name(template, "kind", "class.witness.kind")
    if kind not in WITNESS_FIELDS:
        raise GapLedgerError(f"unknown witness kind {kind!r}")
    _check_fields(template, WITNESS_FIELDS[kind], f"witness {kind}")
    if kind == "hour_discontinuity":
        return {
            "kind": kind,
            "previous_hour": _i32(
                hour_start + _required_int(template, "previous_hour_rel",
                                           "witness.previous_hour_rel"),
                "witness.previous_hour"),
            "next_hour": _i32(
                hour_start + _required_int(template, "next_hour_rel",
                                           "witness.next_hour_rel"),
                "witness.next_hour"),
        }
    if kind == "torn_sweep":
        return {
            "kind": kind,
            "tick_start": _anchored(template.get("tick_start"), hour_start,
                                    "witness.tick_start"),
            "tick_end": _anchored(template.get("tick_end"), hour_start,
                                  "witness.tick_end"),
        }
    if kind == "counter":
        counter = _name(template, "counter", "witness.counter")
        if counter not in GAP_COUNTERS:
            raise GapLedgerError(f"unknown gap counter {counter!r}")
        return {"kind": kind, "counter": counter,
                "observed": _u64(template, "observed", "witness.observed")}
    if kind == "status_bits":
        return {"kind": kind,
                "status_bits": _u32(template, "status_bits", "witness.status_bits")}
    if kind in ("coverage", "coverage_and_status"):
        rebuilt = {
            "kind": kind,
            "expected_perf_records": _u32(template, "expected_perf_records",
                                          "witness.expected_perf_records"),
            "observed_perf_records": _u32(template, "observed_perf_records",
                                          "witness.observed_perf_records"),
            "expected_data_records": _u32(template, "expected_data_records",
                                          "witness.expected_data_records"),
            "observed_data_records": _u32(template, "observed_data_records",
                                          "witness.observed_data_records"),
        }
        if kind == "coverage_and_status":
            rebuilt["status_bits"] = _u32(template, "status_bits",
                                          "witness.status_bits")
        return rebuilt
    raise GapLedgerError(f"unknown witness kind {kind!r}")


def _expand_gaps(compact_stream: Any) -> list[dict[str, Any]]:
    """The rows the flat ``compact_stream.gaps`` array used to carry.

    ``h4gp_hex`` is NOT reproduced: it was a pure function of the record and the
    only consumer that ever read it is the contract's own codec, which can
    re-derive it. Every other field is exactly what the flat array held.

    Raises :class:`GapLedgerError` when the manifest carries no ledger; ask
    :func:`ledger_present` first if that is a case the caller must handle.
    """
    ledger = _ledger(compact_stream)
    table = ledger.get("gaps")
    if not isinstance(table, dict):
        raise GapLedgerError("gap_ledger.gaps is not an object")
    _check_fields(table, frozenset({"row_count", "class_count", "classes"}), "gaps")
    declared_rows = table.get("row_count")
    if not _is_integer(declared_rows):
        raise GapLedgerError("gap_ledger.gaps.row_count is not an integer")
    declared = declared_rows
    rows: list[dict[str, Any]] = []
    seen: set[str] = set()
    claimed: dict[str, set[int]] = {}
    for class_ in _classes(table, "gap_ledger.gaps"):
        if not isinstance(class_, dict):
            raise GapLedgerError("gap_ledger.gaps.classes entry is not an object")
        identity = _class_identity(class_)
        if identity in seen:
            raise GapLedgerError(
                "two classes share one identity; they would have been one class")
        seen.add(identity)
        aggregation = _class_identity(class_, without_domains=True)
        hours_claimed = claimed.setdefault(aggregation, set())
        _check_fields(class_, GAP_CLASS_FIELDS, "gap class")
        grouping = _name(class_, "grouping", "class.grouping")
        domains = _domains(class_)
        span = _required_int(class_, "span_hours", "class.span_hours")
        if span < 1:
            raise GapLedgerError("class.span_hours is below one")
        reason = _name(class_, "reason", "class.reason")
        if reason not in GAP_REASONS:
            raise GapLedgerError(f"unknown gap reason {reason!r}")
        if "ordinals" in class_:
            raise GapLedgerError(
                "a gap class carries `ordinals`, which the gap table has no use for")
        if grouping == "per_domain" and "repeat" in class_:
            raise GapLedgerError("a per_domain class carries `repeat`, which it ignores")
        for hour_start in _bounded_runs(class_, declared, hours_claimed):
            row = {
                "hour_start": hour_start,
                "hour_end": _i32(hour_start + span - 1, "class hour_end"),
                "reason": reason,
                "witness": _witness(class_.get("witness"), hour_start),
            }
            if grouping == "per_domain":
                if not domains:
                    raise GapLedgerError(
                        "a per_domain class names no domain, so it names no row")
                for domain in domains:
                    rows.append({**row, "domains": [domain]})
            elif grouping == "shared":
                if len(domains) == 1:
                    raise GapLedgerError(
                        "a shared class names exactly one domain; that record is per_domain")
                repeat = class_.get("repeat")
                if not isinstance(repeat, int) or isinstance(repeat, bool) or repeat < 1:
                    raise GapLedgerError("shared class.repeat is not a positive integer")
                for _ in range(repeat):
                    rows.append({**row, "domains": list(domains)})
            else:
                raise GapLedgerError(f"unknown domain grouping {grouping!r}")
    declared = table.get("row_count")
    if not _is_integer(declared):
        raise GapLedgerError("gap_ledger.gaps.row_count is not an integer")
    if declared != len(rows):
        raise GapLedgerError(
            f"gap_ledger.gaps declares {declared} rows but expands to {len(rows)}"
        )
    rows.sort(key=lambda row: (row["hour_start"], row["domains"]))
    return rows


def _expand_domain_gap_outcomes(compact_stream: Any) -> list[dict[str, Any]]:
    """The rows the flat ``compact_stream.domain_gap_outcomes`` array carried."""
    ledger = _ledger(compact_stream)
    table = ledger.get("domain_gap_outcomes")
    if not isinstance(table, dict):
        raise GapLedgerError("gap_ledger.domain_gap_outcomes is not an object")
    _check_fields(table, frozenset({
        "row_count", "class_count", "classes", "ordinal_encoding",
        "lane_ordinal_first",
    }), "domain_gap_outcomes")
    declared_rows = table.get("row_count")
    if not _is_integer(declared_rows):
        raise GapLedgerError("gap_ledger.domain_gap_outcomes.row_count is not an integer")
    declared = declared_rows
    encoding = table.get("ordinal_encoding")
    if encoding not in ("lane_sequential", "explicit"):
        raise GapLedgerError(f"unknown ordinal_encoding {encoding!r}")
    explicit = encoding == "explicit"

    if explicit and "lane_ordinal_first" in table:
        raise GapLedgerError(
            "an explicit ordinal table also carries lane_ordinal_first, which it ignores")
    lane_first: dict[tuple[int, str], int] = {}
    if not explicit:
        lanes = table.get("lane_ordinal_first")
        if not isinstance(lanes, list):
            raise GapLedgerError("lane_ordinal_first is not an array")
        for entry in lanes:
            _check_fields(entry, LANE_ENTRY_FIELDS, "lane_ordinal_first entry")
            domain_id = _required_int(entry, "domain_id",
                                      "lane_ordinal_first.domain_id")
            if domain_id < 0 or domain_id > 0xFFFF:
                raise GapLedgerError(
                    f"lane_ordinal_first.domain_id {domain_id} is not a domain id")
            lane = (domain_id, _lane_part(entry))
            if lane in lane_first:
                raise GapLedgerError(f"lane_ordinal_first names lane {lane} twice")
            lane_first[lane] = _u64(entry, "first", "lane_ordinal_first.first")

    rows: list[dict[str, Any]] = []
    seen: set[str] = set()
    claimed: dict[str, set[int]] = {}
    for class_index, class_ in enumerate(_classes(table, "gap_ledger.domain_gap_outcomes")):
        if not isinstance(class_, dict):
            raise GapLedgerError("outcome class is not an object")
        identity = _class_identity(class_)
        if identity in seen:
            raise GapLedgerError(
                "two classes share one identity; they would have been one class")
        seen.add(identity)
        aggregation = _class_identity(class_, without_domains=True)
        hours_claimed = claimed.setdefault(aggregation, set())
        _check_fields(class_, OUTCOME_CLASS_FIELDS, "outcome class")
        part = _name(class_, "part", "class.part")
        if part not in DOMAIN_GAP_PARTS:
            raise GapLedgerError(f"unknown gap part {part!r}")
        reason = _name(class_, "reason", "class.reason")
        if reason not in DOMAIN_GAP_REASONS:
            raise GapLedgerError(f"unknown domain gap reason {reason!r}")
        domains = _domains(class_)
        if not domains:
            raise GapLedgerError("an outcome class names no domain, so it names no row")
        if not explicit and "ordinals" in class_:
            raise GapLedgerError(
                "a lane_sequential class also carries `ordinals`, which it ignores")
        ordinals = class_.get("ordinals") if explicit else None
        if explicit:
            if not isinstance(ordinals, list):
                raise GapLedgerError("explicit outcome class carries no ordinals")
            for ordinal in ordinals:
                if not _is_integer(ordinal) or ordinal < 0 or ordinal > 0xFFFFFFFFFFFFFFFF:
                    raise GapLedgerError(
                        f"class.ordinals entry {ordinal!r} is not a u64 ordinal")
        position = 0
        for target in _bounded_runs(class_, declared, hours_claimed):
            for domain in domains:
                row = {
                    "domain_id": domain,
                    "part": part,
                    "reason": reason,
                    "target_hour_raw": target,
                    "range_start_hour_raw": _i32(
                        target + _required_int(class_, "range_start_rel",
                                               "class.range_start_rel"),
                        "range_start_hour_raw"),
                    "range_end_hour_raw": _i32(
                        target + _required_int(class_, "range_end_rel",
                                               "class.range_end_rel"),
                        "range_end_hour_raw"),
                    "tick_before": _anchored(class_.get("tick_before"), target,
                                             "class.tick_before"),
                    "tick_after": _anchored(class_.get("tick_after"), target,
                                            "class.tick_after"),
                    "queue_capacity": _u32(class_, "queue_capacity",
                                           "class.queue_capacity"),
                    "queue_depth": _u32(class_, "queue_depth", "class.queue_depth"),
                    "queue_dropped": _u64(class_, "queue_dropped",
                                          "class.queue_dropped"),
                    "outcome_ordinal": (ordinals[position] if explicit else 0),
                }
                rows.append(row)
                row["_class_index"] = class_index
                position += 1
        if explicit and ordinals is not None and len(ordinals) != position:
            raise GapLedgerError(
                f"outcome class carries {len(ordinals)} ordinals for {position} rows"
            )

    if not explicit:
        lanes: dict[tuple[int, str], list[dict[str, Any]]] = {}
        for row in rows:
            lanes.setdefault((row["domain_id"], row["part"]), []).append(row)
        for lane in lane_first:
            if lane not in lanes:
                raise GapLedgerError(
                    f"lane_ordinal_first names lane {lane}, which no class expands to")
        for lane, lane_rows in lanes.items():
            lane_rows.sort(key=lambda row: row["target_hour_raw"])
            hours = [row["target_hour_raw"] for row in lane_rows]
            if len(set(hours)) != len(hours):
                raise GapLedgerError(
                    f"lane {lane} has two outcomes at one hour under lane_sequential ordinals"
                )
            if lane not in lane_first:
                raise GapLedgerError(f"lane {lane} has no lane_ordinal_first")
            for offset, row in enumerate(lane_rows):
                ordinal = lane_first[lane] + offset
                if ordinal > 0xFFFFFFFFFFFFFFFF:
                    raise GapLedgerError("derived outcome ordinal overflows u64")
                row["outcome_ordinal"] = ordinal

    declared = table.get("row_count")
    if not _is_integer(declared):
        raise GapLedgerError("gap_ledger.domain_gap_outcomes.row_count is not an integer")
    if declared != len(rows):
        raise GapLedgerError(
            f"gap_ledger.domain_gap_outcomes declares {declared} rows but expands to {len(rows)}"
        )
    rows.sort(key=lambda row: (row["target_hour_raw"], row["domain_id"], row["_class_index"]))
    for row in rows:
        del row["_class_index"]
    return rows


def _bounded(expand, compact_stream: Any, what: str) -> list[dict[str, Any]]:
    """Run an expander so that NOTHING but :class:`GapLedgerError` escapes it.

    The two callers -- the capture scorecard and the sink verifier -- catch this
    module's own error and report a problem. Every other exception class walks
    straight past them and kills the tool: a `KeyError` for a class missing
    `range_start_rel`, a `TypeError` for a `domains` list holding a string. A
    validator that dies mid-manifest publishes no verdict at all, and "the
    validator crashed" is not a grade.

    Belt AND braces: every field access above is already checked, and this is
    what makes the CLASS of failure a promise rather than a claim about how
    carefully the checks were written.
    """
    try:
        return expand(compact_stream)
    except GapLedgerError:
        raise
    except Exception as error:
        raise GapLedgerError(f"{what}: {type(error).__name__}: {error}") from error


def expand_gaps(compact_stream: Any) -> list[dict[str, Any]]:
    """The rows the flat ``compact_stream.gaps`` array used to carry.

    See :func:`_expand_gaps`. Raises only :class:`GapLedgerError`.
    """
    return _bounded(_expand_gaps, compact_stream, "gap_ledger.gaps is unreadable")


def expand_domain_gap_outcomes(compact_stream: Any) -> list[dict[str, Any]]:
    """The rows the flat ``compact_stream.domain_gap_outcomes`` array carried.

    See :func:`_expand_domain_gap_outcomes`. Raises only :class:`GapLedgerError`.
    """
    return _bounded(_expand_domain_gap_outcomes, compact_stream,
                    "gap_ledger.domain_gap_outcomes is unreadable")


def pack_rows_for_tests(gaps: list[dict[str, Any]] | None = None,
                        outcomes: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    """Wrap flat rows in a valid ledger, for TEST FIXTURES only.

    Fixtures state the rows they want a consumer to see, and every consumer now
    reads through :func:`expand_gaps` / :func:`expand_domain_gap_outcomes`. This
    builds the smallest valid ledger carrying exactly those rows: one class per
    row, ordinals stated explicitly.

    Deliberately the DUMB encoding -- no grouping, no run merging. The recorder's
    own encoder is the compact one and is held to the conformance vector; a
    second compact encoder here would be a second thing to keep right, and a
    fixture does not need one.
    """
    gap_classes = []
    for row in gaps or []:
        witness = dict(row.get("witness") or {})
        hour_start = int(row["hour_start"])
        kind = witness.get("kind")
        if kind == "hour_discontinuity":
            for absolute, relative in (("previous_hour", "previous_hour_rel"),
                                       ("next_hour", "next_hour_rel")):
                if absolute in witness:
                    witness[relative] = int(witness.pop(absolute)) - hour_start
        elif kind == "torn_sweep":
            for field in ("tick_start", "tick_end"):
                if field in witness:
                    witness[field] = {"abs": int(witness[field])}
        domains = list(row.get("domains") or [])
        shared = len(domains) != 1
        gap_class = {
            "reason": row["reason"],
            "grouping": "shared" if shared else "per_domain",
            "domains": domains,
            "span_hours": int(row["hour_end"]) - hour_start + 1,
            "witness": witness,
            "hour_runs": [[hour_start, 1]],
        }
        if shared:
            gap_class["repeat"] = 1
        gap_classes.append(gap_class)

    outcome_classes = []
    for row in outcomes or []:
        target = int(row["target_hour_raw"])
        outcome_classes.append({
            "reason": row["reason"],
            "part": row["part"],
            "domains": [row["domain_id"]],
            "queue_capacity": row.get("queue_capacity", 0),
            "queue_depth": row.get("queue_depth", 0),
            "queue_dropped": row.get("queue_dropped", 0),
            "range_start_rel": int(row["range_start_hour_raw"]) - target,
            "range_end_rel": int(row["range_end_hour_raw"]) - target,
            "tick_before": {"abs": int(row.get("tick_before", 0))},
            "tick_after": {"abs": int(row.get("tick_after", 0))},
            "hour_runs": [[target, 1]],
            "ordinals": [int(row.get("outcome_ordinal", 0))],
        })

    gap_classes = _merge_packed_classes(gap_classes)
    outcome_classes = _merge_packed_classes(outcome_classes)
    return {
        "schema": GAP_LEDGER_SCHEMA,
        "version": GAP_LEDGER_VERSION,
        "gaps": {"row_count": _packed_row_count(gap_classes),
                 "class_count": len(gap_classes), "classes": gap_classes},
        "domain_gap_outcomes": {"row_count": _packed_row_count(outcome_classes),
                                "class_count": len(outcome_classes),
                                "ordinal_encoding": "explicit",
                                "classes": outcome_classes},
    }


def _packed_row_count(classes: list[dict[str, Any]]) -> int:
    total = 0
    for class_ in classes:
        hours = sum(run[1] for run in class_["hour_runs"])
        if class_.get("grouping") == "shared":
            total += hours * class_.get("repeat", 1)
        else:
            total += hours * len(class_.get("domains") or [])
    return total


def _merge_packed_classes(classes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Fold one-row classes into the classes the recorder's encoder would emit.

    THE ENCODER'S OWN TWO-LEVEL GROUPING, on already-flat rows. A packer that
    emitted one class per row produced documents its own expander refuses, in
    three separate ways once the readers became canonical: two classes with one
    identity, a run list that is not maximal, and two classes differing only in
    their domains both naming one hour.

    Level one buckets the rows by (identity-without-domains, hour) and collects
    that HOUR's domains -- which is where same-hour multiplicity lands, as a
    domain repeated in the list, exactly as the wire carries it. Level two
    buckets those hours by (identity-without-domains, that hour's exact domain
    list), so an hour whose domain set differs becomes its own class, which is
    what the recorder does.
    """
    shared_counts: dict[str, dict[str, Any]] = {}
    per_hour: dict[tuple[str, int], dict[str, Any]] = {}
    template: dict[str, dict[str, Any]] = {}
    for class_ in classes:
        if class_.get("grouping") == "shared":
            bucket = shared_counts.setdefault(
                _class_identity(class_), {"class": class_, "hours": {}})
            for first, count in class_["hour_runs"]:
                for hour in range(first, first + count):
                    bucket["hours"][hour] = bucket["hours"].get(hour, 0) + 1
            continue
        identity = _class_identity(class_, without_domains=True)
        template.setdefault(identity, class_)
        row_domains = list(class_.get("domains") or [])
        row_ordinals = list(class_.get("ordinals") or [])
        for first, count in class_["hour_runs"]:
            for hour in range(first, first + count):
                slot = per_hour.setdefault((identity, hour),
                                           {"domains": [], "ordinals": []})
                slot["domains"].extend(row_domains)
                slot["ordinals"].extend(row_ordinals)

    grouped: dict[tuple[str, tuple[int, ...]], dict[str, Any]] = {}
    for (identity, hour), slot in sorted(per_hour.items(), key=lambda item: item[0][1]):
        order = sorted(range(len(slot["domains"])), key=lambda i: slot["domains"][i])
        domains = tuple(slot["domains"][i] for i in order)
        ordinals = [slot["ordinals"][i] for i in order] if slot["ordinals"] else []
        bucket = grouped.setdefault((identity, domains),
                                    {"identity": identity, "domains": list(domains),
                                     "hours": [], "ordinals": []})
        bucket["hours"].append(hour)
        bucket["ordinals"].extend(ordinals)

    out: list[dict[str, Any]] = []
    for bucket in shared_counts.values():
        by_repeat: dict[int, list[int]] = {}
        for hour, times in bucket["hours"].items():
            by_repeat.setdefault(times, []).append(hour)
        for repeat, hours in by_repeat.items():
            runs: list[list[int]] = []
            for hour in sorted(hours):
                if runs and hour == runs[-1][0] + runs[-1][1]:
                    runs[-1][1] += 1
                else:
                    runs.append([hour, 1])
            rebuilt = {key: value for key, value in bucket["class"].items()
                       if key not in ("hour_runs", "ordinals")}
            rebuilt["hour_runs"] = runs
            rebuilt["repeat"] = repeat
            out.append(rebuilt)
    for bucket in grouped.values():
        hours = sorted(bucket["hours"])
        runs: list[list[int]] = []
        for hour in hours:
            if runs and hour == runs[-1][0] + runs[-1][1]:
                runs[-1][1] += 1
            else:
                runs.append([hour, 1])
        rebuilt = {key: value for key, value in template[bucket["identity"]].items()
                   if key not in ("hour_runs", "ordinals")}
        rebuilt["domains"] = bucket["domains"]
        rebuilt["hour_runs"] = runs
        if bucket["ordinals"]:
            rebuilt["ordinals"] = bucket["ordinals"]
        out.append(rebuilt)
    return out

