import copy
import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "verify_a18_dynamic_modifier_fixture.py"
SPEC = importlib.util.spec_from_file_location("verify_a18", SCRIPT)
VERIFY = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(VERIFY)


PROCESS = {
    "pid": 9123,
    "create_time_100ns": 134000000000000000,
    "executable_sha256": VERIFY.EXPECTED_SHA,
}
MANIFEST = {
    "schema": "native_collector_session_v1",
    "pid": PROCESS["pid"],
    "scheduled_domains": ["country_dynamic_modifiers_live"],
    "modset": {
        "resolved": True,
        "errors": [],
        "game": {"exe_sha256": VERIFY.EXPECTED_SHA},
    },
    "compact_stream": {"decodable": True},
    "collector_control_errors": [],
    "parse_errors": [],
}


def join(modifier_id, days):
    return {
        "modifier_id": modifier_id,
        "enabled": True,
        "values_raw": [10_000, -5_000],
        "value_scale": 100_000,
        "remaining_days": days,
        "checks": {
            "enabled_equal": True,
            "values_fixed_five_equal": True,
            "remaining_days_equal": True,
        },
        "accepted": True,
    }


def checkpoint(phase, hour, days, hash_digit):
    timed = (
        {
            "modifier_id": "timed",
            "absent_live": True,
            "absent_save": True,
            "accepted": True,
        }
        if phase == "post_removal"
        else join("timed", days)
    )
    return {
        "schema": "hoi4.a18.dynamic_modifier_save_live_checkpoint.v1",
        "phase": phase,
        "process_identity": dict(PROCESS),
        "save": {"sha256": hash_digit * 64, "game_hour_raw": hour},
        "live": {
            "game_hour_raw_before": hour,
            "game_hour_raw_after": hour,
            "owner_tag": "AST",
            "owner_tag_db_id": 33,
        },
        "target": {"permanent_modifier": "permanent", "timed_modifier": "timed"},
        "joins": [join("permanent", None), timed],
        "same_hour": True,
        "accepted": True,
    }


def row(modifier_id, days):
    return {
        "modifier_id": modifier_id,
        "enabled": True,
        "values_raw": [10_000, -5_000],
        "remaining_days": days,
        "pointer_masked_entry_bytes": [0] * 0x40,
    }


def fixture():
    checkpoints = [
        checkpoint("positive_before", 1000, 2, "1"),
        checkpoint("positive_after", 1024, 1, "2"),
        checkpoint("post_removal", 1048, None, "3"),
    ]
    hook = {
        "schema": "hoi4.a18.dynamic_modifier_lifecycle_witness.v1",
        "executable_sha256": VERIFY.EXPECTED_SHA,
        "exact20_job_id": VERIFY.EXPECTED_JOB,
        "exact20_manifest_sha256": VERIFY.EXPECTED_MANIFEST,
        "update_function_rva": VERIFY.EXPECTED_UPDATE_RVA,
        "erase_function_rva": VERIFY.EXPECTED_ERASE_RVA,
        "process_identity": dict(PROCESS),
        "evidence_install": True,
        "hook_epoch": 1,
        "diagnostics": {
            "retained_updates": 2,
            "ignored_without_timed_instance": 400,
            "dropped_reentrant": 0,
            "dropped_unreadable": 0,
            "dropped_lock_busy": 0,
            "dropped_overflow": 0,
            "completeness_poisoned": False,
        },
        "witnesses": [
            {
                "sequence": 1,
                "thread_id": 7,
                "owner_tag_db_id": 33,
                "game_hour_raw_before": 1024,
                "game_hour_raw_after": 1024,
                "before": [row("timed", 2), row("permanent", None)],
                "erase_boundaries": [],
                "after": [row("timed", 1), row("permanent", None)],
            },
            {
                "sequence": 2,
                "thread_id": 7,
                "owner_tag_db_id": 33,
                "game_hour_raw_before": 1048,
                "game_hour_raw_after": 1048,
                "before": [row("timed", 1), row("permanent", None)],
                "erase_boundaries": [
                    {"start_index": 0, "end_index": 1, "rows_before_erase": [row("timed", 0)]}
                ],
                "after": [row("permanent", None)],
            },
        ],
        "production_admission_ready": False,
    }
    return checkpoints, hook


def test_accepts_exact_positive_decrement_and_synchronous_zero_removal():
    checkpoints, hook = fixture()
    accepted, errors, details = VERIFY.verify_documents(checkpoints, hook, MANIFEST)
    assert accepted, errors
    assert details["checkpoint_days"] == [2, 1]
    assert details["zero_removal_witness_sequence"] == 2
    assert not VERIFY.find_forbidden_expiry(details)


def test_rejects_absence_when_zero_was_not_observed_at_erase_boundary():
    checkpoints, hook = fixture()
    broken = copy.deepcopy(hook)
    broken["witnesses"][1]["erase_boundaries"][0]["rows_before_erase"][0]["remaining_days"] = 1
    accepted, errors, _details = VERIFY.verify_documents(checkpoints, broken, MANIFEST)
    assert not accepted
    assert any("zero" in error for error in errors)


def test_rejects_a_synthesized_expiry_even_when_other_fields_match():
    checkpoints, hook = fixture()
    broken = copy.deepcopy(hook)
    broken["witnesses"][0]["absolute_expiry_hour"] = 9999
    accepted, errors, _details = VERIFY.verify_documents(checkpoints, broken, MANIFEST)
    assert not accepted
    assert any("expiry" in error for error in errors)
