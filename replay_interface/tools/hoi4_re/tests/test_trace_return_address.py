"""The return-address knob on `trace_calls`, tested without a live process.

`trace_calls` attaches Frida to a running hoi4.exe, so the parts that can be
tested offline are the two that decide what the live run WILL do: the input
validation, and the injected script plus target list built from it. Both are
pure functions of the inputs, and both are where an additive knob can break a
job spec that predates it.

WHAT THESE KILL: turning the knob into a default-on or an always-present target
field, which would change the injected script and the recorded target list for
every existing spec in the shared broker pool; and dropping the module-range
resolution, which is what makes a return address nameable.
"""
from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import runtime_ops as R


SYM = "sym_test"


def base_target(**extra):
    target = {
        "symbol_id": SYM,
        "rva": "0x23a2520",
        "receiver_vtable_rvas": [],
        "receiver_qword_offsets": [],
        "receiver_pointee_qword_offsets": [],
        "receiver_pointer_chains": [],
        "argument_pointer_arrays": [],
        "skip_hits": 0,
    }
    target.update(extra)
    return target


def _script_before_the_knob():
    """The pre-change `_trace_script`, loaded from a pinned copy of the module.

    Comparing against the real previous implementation is the only way to make
    "byte-identical" a measurement rather than a claim. The first draft of this
    file asserted that `json.dumps(targets)` was unchanged and PASSED while the
    injected script had silently grown by 981 bytes, because the helper function
    was emitted unconditionally.
    """
    # runtime_ops.py is CRLF in this working tree and the reference is LF.
    # That does not matter: Python normalizes newlines both when it imports a
    # module and in read_text below, so each f-string literal yields LF either
    # way and the comparison is of the real emitted bytes.
    reference = HERE / "tests" / "reference_trace_script_before_the_knob.py.txt"
    spec = importlib.util.spec_from_loader("ro_before", loader=None)
    module = importlib.util.module_from_spec(spec)
    module.__dict__["__file__"] = str(reference)
    sys.modules["ro_before"] = module
    exec(compile(reference.read_text(encoding="utf-8"), str(reference), "exec"), module.__dict__)
    return module._trace_script


REAL_SPEC = base_target(
    receiver_qword_offsets=["0x810", "0x818"],
    argument_pointer_arrays=[{
        "register": "rdx", "data_offset": "0x40", "count_offset": "0x50",
        "start_index": 0, "max_items": 4,
        "item_qword_offsets": [], "item_vtable_slot_offsets": [],
    }],
)


def test_a_spec_that_does_not_ask_for_the_caller_injects_the_identical_SCRIPT():
    """Not "equivalent". Identical, byte for byte, against the previous code.

    Other lanes share this broker pool. A spec written before the knob existed
    has to put exactly the same bytes into the shared process as it did before.
    """
    before = _script_before_the_knob()
    for target in (base_target(), REAL_SPEC):
        old = before([target], 64)
        new = R._trace_script([target], 64)
        assert new == old, f"the injected script grew by {len(new) - len(old)} bytes"


def test_a_spec_that_does_not_ask_for_the_caller_records_the_identical_TARGET():
    """The target dict is echoed into the job artifact, so it counts too."""
    target = base_target()
    assert "capture_return_address" not in target
    assert "capture_return_address" not in json.dumps([target])
    assert json.dumps([target]) in R._trace_script([target], 8)


def test_the_helper_is_absent_entirely_when_nothing_asked():
    """A dead helper is still bytes injected into someone else's process."""
    off = R._trace_script([base_target()], 8)
    assert "moduleAddress" not in off
    assert "return_address" not in off


def test_asking_for_the_caller_adds_exactly_one_field_and_selects_the_branch():
    target = base_target(capture_return_address=True)
    script = R._trace_script([target], 8)
    assert json.dumps([target]) in script
    assert "capture_return_address" in json.dumps([target])
    assert "moduleAddress(this.returnAddress)" in script
    assert "return_address: target.capture_return_address" in script


def test_the_caller_is_read_through_fridas_accessor_and_not_off_the_stack():
    """`this.returnAddress`, not `[rsp]`.

    Interceptor.attach installs its own frame, so the qword at `[rsp]` on entry
    is the trampoline's return address rather than the caller's. A reader that
    took `this.context.rsp` and dereferenced it would produce a plausible
    pointer that names the wrong thing, which is worse than no answer.
    """
    script = R._trace_script([base_target(capture_return_address=True)], 8)
    assert "this.returnAddress" in script
    assert "this.context.rsp.readPointer()" not in script


def test_the_caller_is_resolved_to_a_module_rva_the_way_vtable_slots_are():
    script = R._trace_script([base_target(capture_return_address=True)], 8)
    assert "function moduleAddress(value)" in script
    # The same range compare the vtable-slot reader uses, so an address outside
    # hoi4.exe is reported as such rather than given a meaningless RVA.
    assert script.count("value.compare(module.base) >= 0") >= 1
    assert "in_module: false" in script


def test_the_knob_is_read_only():
    """No write path. The whole point is bounded observation."""
    script = R._trace_script([base_target(capture_return_address=True)], 8)
    for forbidden in ("writePointer", "writeU32", "writeU64", "writeByteArray", "Memory.alloc"):
        assert forbidden not in script


class _Registry:
    """The one call `trace_calls` makes before it reaches Frida."""

    def __init__(self):
        self.calls = 0

    def get_build(self, _identifier):
        return {"build_id": "b", "sha256": "0" * 64}

    def resolve_symbol(self, _symbol_id, _build_id):
        self.calls += 1
        return {"rva": 0x23A2520}


def _validate(**kwargs):
    """Drive validation only: it raises before any Frida import on bad input."""
    return R.trace_calls(
        _Registry(), Path("."), "b", [SYM], 1.0, 8,
        Path("lease"), Path("partial.json"), **kwargs,
    )


@pytest.mark.parametrize("bad", [
    {"not_a_symbol": True},
    {"sym_other": False},
])
def test_a_return_address_key_that_is_not_a_requested_symbol_is_refused(bad):
    with pytest.raises(ValueError, match="capture_return_address keys"):
        _validate(capture_return_address=bad)


@pytest.mark.parametrize("bad", [{SYM: 1}, {SYM: "yes"}, {SYM: None}, {SYM: [True]}])
def test_a_non_boolean_return_address_value_is_refused(bad):
    """`1` is not `True` here. A truthy int would silently enable the knob."""
    with pytest.raises(ValueError, match="capture_return_address values"):
        _validate(capture_return_address=bad)


@pytest.mark.parametrize("bad", [[SYM], [], "", 0, False, 0.0, set()])
def test_a_non_mapping_is_refused_including_the_falsey_ones(bad):
    """`or {}` read `[]`, `""`, `0` and `False` as "absent" and ran anyway.

    Only the truthy list fails under the old expression, so a suite that tested
    only that would pass against the defect this parametrisation exists for.
    """
    with pytest.raises(ValueError, match="capture_return_address keys"):
        _validate(capture_return_address=bad)




# --- The production paths, driven rather than re-implemented -----------------
#
# Review round 12 was right that everything above this line exercises helpers
# with hand-built targets. These four drive the real construction, the real
# schema selection and the real broker call, so a change that broke the
# shared-pool contract could no longer pass by leaving a substring in place.

LEGACY_KNOBS = dict(
    raw_filters={}, raw_offsets={SYM: ["0x810", "0x818"]}, raw_arrays={},
    raw_pointee_offsets={}, raw_pointer_chains={}, raw_skip_hits={},
    raw_return_addresses={},
)


def test_production_records_no_new_key_for_a_legacy_request():
    """`_build_targets` is what writes the artifact. Drive IT.

    Key ORDER matters as much as key presence: the target list is serialized
    into the job result and into the injected script, so a reordering is a
    changed artifact even when the mapping is equal.
    """
    targets = R._build_targets(_Registry(), "b", [SYM], 64, **LEGACY_KNOBS)
    assert len(targets) == 1
    assert list(targets[0]) == [
        "symbol_id", "rva", "receiver_vtable_rvas", "receiver_qword_offsets",
        "receiver_pointee_qword_offsets", "receiver_pointer_chains",
        "argument_pointer_arrays", "skip_hits",
    ]
    assert "capture_return_address" not in targets[0]


def test_production_records_the_key_only_for_the_symbol_that_asked():
    targets = R._build_targets(
        _Registry(), "b", [SYM], 64, **{**LEGACY_KNOBS, "raw_return_addresses": {SYM: True}})
    assert targets[0]["capture_return_address"] is True
    # False is not the same as absent, and a false request must record nothing.
    off = R._build_targets(
        _Registry(), "b", [SYM], 64, **{**LEGACY_KNOBS, "raw_return_addresses": {SYM: False}})
    assert "capture_return_address" not in off[0]


def test_production_targets_reproduce_the_pre_change_script_exactly():
    """The whole contract in one line: real targets, real script, pinned old code."""
    targets = R._build_targets(_Registry(), "b", [SYM], 64, **LEGACY_KNOBS)
    assert R._trace_script(targets, 64) == _script_before_the_knob()(targets, 64)


def test_the_real_schema_function_moves_only_with_the_hit_shape():
    assert R._trace_schema(R._build_targets(
        _Registry(), "b", [SYM], 64, **LEGACY_KNOBS)) == "hoi4-re-frida-call-trace-v1"
    assert R._trace_schema(R._build_targets(
        _Registry(), "b", [SYM], 64,
        **{**LEGACY_KNOBS, "raw_return_addresses": {SYM: True}},
    )) == "hoi4-re-frida-call-trace-v2"


def test_the_broker_hands_the_knob_to_the_right_parameter():
    """Positional argument 12. A substring search would not catch a misplacement."""
    import operations

    seen = {}

    def recorder(*args, **kwargs):
        seen["args"] = args
        seen["kwargs"] = kwargs
        return {"ok": True}

    original = operations.trace_calls
    operations.trace_calls = recorder
    try:
        broker = operations.Broker.__new__(operations.Broker)
        broker.registry = _Registry()
        broker.config = type("C", (), {
            "workspace": Path("."), "runtime_lease": Path("lease")})()
        broker._trace_calls(
            {"build_id": "b", "symbol_ids": [SYM],
             "capture_return_address": {SYM: True}},
            "job", Path("."),
        )
    finally:
        operations.trace_calls = original

    # BIND through the real signature rather than eyeballing the last slot.
    # `True` is an `int` in Python, so if operations.py dropped `skip_hits` the
    # knob would bind to it as a valid one-hit skip, `capture_return_address`
    # would be None, no caller would be captured, and a positional check on
    # args[-1] would still pass.
    import inspect
    bound = inspect.signature(R.trace_calls).bind(*seen["args"], **seen["kwargs"])
    bound.apply_defaults()
    assert bound.arguments["capture_return_address"] == {SYM: True}
    assert bound.arguments["skip_hits"] is None


def test_the_extraction_changed_nothing_for_any_OTHER_knob():
    """`_build_targets` builds what the pre-change `trace_calls` built.

    Moving a hundred-line loop out of a function other lanes depend on is the
    risky part of this change, and the five knobs this lane never touched -
    vtable filters, qword offsets, pointer arrays, pointee offsets, pointer
    chains, skip_hits - would break silently.

    The old code cannot be driven past `_load_winmem` offline, so it is stopped
    there with a sentinel and its `targets` read out of the raised traceback's
    frame. That is exotic, and it is the only way to compare against the real
    previous construction rather than against a hand-copied expectation.

    kills: any edit to `_build_targets` that changes what an existing spec
    records or injects, for any knob.
    """
    reference = HERE / "tests" / "reference_trace_script_before_the_knob.py.txt"
    spec = importlib.util.spec_from_loader("ro_before_full", loader=None)
    before = importlib.util.module_from_spec(spec)
    before.__dict__["__file__"] = str(reference)
    sys.modules["ro_before_full"] = before
    exec(compile(reference.read_text(encoding="utf-8"), str(reference), "exec"), before.__dict__)

    class _Stop(Exception):
        pass

    before._load_winmem = lambda *_a, **_k: (_ for _ in ()).throw(_Stop())

    knobs = dict(
        raw_filters={SYM: ["0x2b47280", 0x1234]},
        raw_offsets={SYM: ["0x810", 2072]},
        raw_arrays={SYM: [{
            "register": "rdx", "data_offset": "0x40", "count_offset": 80,
            "start_index": 2, "max_items": 4,
            "item_qword_offsets": [0, 8], "item_vtable_slot_offsets": ["0x10"],
        }]},
        raw_pointee_offsets={SYM: [{"field_offset": "0x18", "pointee_qword_offsets": [0, "0x8"]}]},
        raw_pointer_chains={SYM: [{"pointer_offsets": ["0x10", 24], "qword_offsets": [0, 8]}]},
        raw_skip_hits={SYM: 7},
    )
    try:
        before.trace_calls(
            _Registry(), Path("."), "b", [SYM], 1.0, 8, Path("lease"), Path("partial"),
            knobs["raw_filters"], knobs["raw_offsets"], knobs["raw_arrays"],
            knobs["raw_pointee_offsets"], knobs["raw_pointer_chains"], knobs["raw_skip_hits"],
        )
    except _Stop:
        old_targets = sys.exc_info()[2].tb_next.tb_frame.f_locals["targets"]
    else:  # pragma: no cover - the sentinel must fire
        pytest.fail("the pre-change trace_calls did not reach _load_winmem")

    new_targets = R._build_targets(
        _Registry(), "b", [SYM], 8, raw_return_addresses={}, **knobs)

    assert new_targets == old_targets
    assert [list(t) for t in new_targets] == [list(t) for t in old_targets]
    assert R._trace_script(new_targets, 8) == before._trace_script(old_targets, 8)


def test_the_validation_runs_before_the_frida_import():
    """Ordering, pinned two-sidedly.

    `import frida` sits inside `trace_calls`, so on a machine without Frida a
    malformed request would fail with ModuleNotFoundError instead of the reason.
    Setting `sys.modules['frida'] = None` makes the import raise, which lets
    both directions be asserted: malformed input still raises ValueError because
    validation precedes the import, and well-formed input reaches the import and
    raises from it.

    kills: moving `import frida` back above the capture_return_address checks.
    """
    saved = sys.modules.get("frida", "absent")
    sys.modules["frida"] = None
    try:
        with pytest.raises(ValueError, match="capture_return_address values"):
            _validate(capture_return_address={SYM: 1})
        with pytest.raises(ValueError, match="capture_return_address keys"):
            _validate(capture_return_address={"other": True})
        # And a well-formed request does get as far as the import, which proves
        # the sentinel is real rather than the checks being unreachable.
        with pytest.raises(ImportError):
            _validate(capture_return_address={SYM: True})
    finally:
        if saved == "absent":
            del sys.modules["frida"]
        else:
            sys.modules["frida"] = saved


@pytest.mark.parametrize("knob, spec, message", [
    ("raw_arrays", {"register": "rdx", "data_offset": 0, "count_offset": 0,
                    "start_index": 0, "max_items": 4096,
                    "item_qword_offsets": [], "item_vtable_slot_offsets": []},
     "8192-item"),
    # 128 items is exactly the item budget at 64 hits, so this one has to trip
    # the QWORD budget rather than the item budget that precedes it.
    ("raw_arrays", {"register": "rdx", "data_offset": 0, "count_offset": 0,
                    "start_index": 0, "max_items": 128,
                    "item_qword_offsets": [0, 8, 16, 24, 32, 40, 48, 56],
                    "item_vtable_slot_offsets": []},
     "8192-qword"),
])
def test_the_extracted_budgets_still_bite(knob, spec, message):
    """The four capture budgets moved into `_build_targets` with the loop.

    They are what stops one job from asking the traced process for an unbounded
    amount of work, and the equivalence test above stays under every cap, so
    deleting a budget would not have failed anything.

    kills: dropping or loosening a budget check during the extraction.
    """
    knobs = dict(
        raw_filters={}, raw_offsets={}, raw_arrays={}, raw_pointee_offsets={},
        raw_pointer_chains={}, raw_skip_hits={}, raw_return_addresses={},
    )
    knobs[knob] = {SYM: [spec]}
    with pytest.raises(ValueError, match=message):
        R._build_targets(_Registry(), "b", [SYM], 64, **knobs)


@pytest.mark.parametrize("knob, spec, message", [
    # 64 offsets is exactly the budget at 64 hits, so both need two specs.
    ("raw_pointee_offsets",
     [{"field_offset": 0, "pointee_qword_offsets": list(range(0, 512, 8))},
      {"field_offset": 8, "pointee_qword_offsets": list(range(0, 512, 8))}],
     "4096-qword"),
    ("raw_pointer_chains",
     [{"pointer_offsets": [0], "qword_offsets": list(range(0, 512, 8))},
      {"pointer_offsets": [8], "qword_offsets": list(range(0, 512, 8))}],
     "4096-qword"),
])
def test_the_other_two_extracted_budgets_still_bite(knob, spec, message):
    knobs = dict(
        raw_filters={}, raw_offsets={}, raw_arrays={}, raw_pointee_offsets={},
        raw_pointer_chains={}, raw_skip_hits={}, raw_return_addresses={},
    )
    knobs[knob] = {SYM: spec}
    with pytest.raises(ValueError, match=message):
        R._build_targets(_Registry(), "b", [SYM], 64, **knobs)


def _knobs(**over):
    base = dict(
        raw_filters={}, raw_offsets={}, raw_arrays={}, raw_pointee_offsets={},
        raw_pointer_chains={}, raw_skip_hits={}, raw_return_addresses={},
    )
    base.update(over)
    return base


def _array(max_items, qword_count):
    return {"register": "rdx", "data_offset": 0, "count_offset": 0, "start_index": 0,
            "max_items": max_items,
            "item_qword_offsets": [8 * i for i in range(qword_count)],
            "item_vtable_slot_offsets": []}


# Each row is (knob, at-the-limit spec, one-over spec, the message). At 64 hits
# the item and qword budgets are 8192 and the pointee and chain budgets 4096, so
# the limits are 128 items, 128 item-qwords, 64 pointee-qwords and 64
# chain-qwords. The at-limit case must be ACCEPTED and the one-over REFUSED.
BUDGET_EDGES = [
    ("raw_arrays", [_array(128, 0)], [_array(129, 0)], "8192-item"),
    ("raw_arrays", [_array(16, 8)], [_array(17, 8)], "8192-qword"),
    ("raw_pointee_offsets",
     [{"field_offset": 0, "pointee_qword_offsets": [8 * i for i in range(32)]},
      {"field_offset": 8, "pointee_qword_offsets": [8 * i for i in range(32)]}],
     [{"field_offset": 0, "pointee_qword_offsets": [8 * i for i in range(32)]},
      {"field_offset": 8, "pointee_qword_offsets": [8 * i for i in range(33)]}],
     "4096-qword"),
    ("raw_pointer_chains",
     [{"pointer_offsets": [0], "qword_offsets": [8 * i for i in range(32)]},
      {"pointer_offsets": [8], "qword_offsets": [8 * i for i in range(32)]}],
     [{"pointer_offsets": [0], "qword_offsets": [8 * i for i in range(32)]},
      {"pointer_offsets": [8], "qword_offsets": [8 * i for i in range(33)]}],
     "4096-qword"),
]


@pytest.mark.parametrize("knob, at_limit, over, message", BUDGET_EDGES,
                         ids=[row[3] + "-" + row[0] for row in BUDGET_EDGES])
def test_each_budget_is_pinned_at_its_exact_threshold(knob, at_limit, over, message):
    """The LIMIT, not merely that something far above it is refused.

    Tests that only submit a request many times over a cap pass just as happily
    against a cap loosened by orders of magnitude - 8192 raised to 262143 would
    still refuse a 262144-item request. A budget is only pinned by the pair:
    exactly at the limit is accepted, one unit over is refused.

    kills: raising any of the four capture budgets.
    """
    accepted = R._build_targets(_Registry(), "b", [SYM], 64, **_knobs(**{knob: {SYM: at_limit}}))
    assert len(accepted) == 1

    with pytest.raises(ValueError, match=message):
        R._build_targets(_Registry(), "b", [SYM], 64, **_knobs(**{knob: {SYM: over}}))
