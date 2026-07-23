"""Static unit coverage for passive CID1 matrix admission.

These tests use no game process, sockets, or subprocesses. They pin the one
intentionally incomplete production state: all local arms are independently
validated while M3 remains an evidence-free cross-machine blocker.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent.parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import verify_campaign_identity_matrix as matrix  # noqa: E402


def _inputs(tmp_path: Path):
    contract = tmp_path / "contract.json"
    receipt = tmp_path / "receipt.json"
    contract.write_text("{}", encoding="utf-8")
    receipt.write_text("{}", encoding="utf-8")
    arms = {
        arm: {
            "status": "complete",
            "action_receipt_path": "unused",
            "action_receipt_sha256": "a" * 64,
            "endpoints": [],
        }
        for arm in matrix.ARMS
    }
    arms["M3"] = {
        "status": "blocked_external_second_machine",
        "action_receipt_path": "",
        "action_receipt_sha256": "",
        "endpoints": [],
    }
    values = {
        contract.resolve(): {
            "schema": matrix.CONTRACT_SCHEMA,
            "pinned_executable_sha256": matrix.CURRENT_EXE_SHA,
        },
        receipt.resolve(): {
            "schema": matrix.SCHEMA,
            "contract_path": str(contract.resolve()),
            "contract_sha256": "a" * 64,
            "matrix_nonce_hex": "b" * 64,
            "created_filetime_utc": 1,
            "arms": arms,
        },
    }
    args = SimpleNamespace(
        contract=contract,
        expected_contract_sha256="a" * 64,
        receipt=receipt,
        allow_external_m3_pending=True,
    )
    return args, values


def _common_patches(values):
    return (
        patch.object(matrix, "checked_path", lambda path, _digest, _label: Path(path).resolve()),
        patch.object(matrix, "load_json", lambda path, _label: values[Path(path).resolve()]),
        patch.object(matrix, "sha256_file", lambda _path: "b" * 64),
    )


class PassiveCampaignIdentityTests(unittest.TestCase):
    def test_local_matrix_passes_without_fabricating_m3(self):
        with tempfile.TemporaryDirectory(prefix="cid1-passive-test-") as raw:
            args, values = _inputs(Path(raw))
            seen = []

            def accept_local(arm, _value, **context):
                seen.append(arm)
                self.assertEqual(context["contract_path"], args.contract.resolve())
                self.assertEqual(context["contract_sha256"], "a" * 64)
                self.assertEqual(context["matrix_nonce_hex"], "b" * 64)
                return {"arm": arm, "pair_relation": "checked", "endpoint_transcripts": ["x", "y"]}

            checked, loaded, hashed = _common_patches(values)
            with checked, loaded, hashed, patch.object(matrix, "verify_arm", accept_local):
                report = matrix.verify(args)
            self.assertEqual(seen, list(matrix.ARMS[:-1]))
            self.assertEqual(report["verdict"], "blocked_external_m3")
            self.assertIs(report["local_production_ready"], True)
            self.assertIs(report["cross_machine_rehost_ready"], False)
            self.assertEqual(report["arms"][-1], {
                "arm": "M3",
                "status": "blocked_external_second_machine",
                "pair_relation": "unobserved",
                "endpoint_transcripts": [],
            })

    def test_pending_m3_rejects_any_claimed_evidence(self):
        cases = (
            ("action_receipt_path", "fabricated.json"),
            ("action_receipt_sha256", "c" * 64),
            ("endpoints", [{"role": "rehost"}]),
        )
        for field, value in cases:
            with self.subTest(field=field), tempfile.TemporaryDirectory(prefix="cid1-passive-test-") as raw:
                args, values = _inputs(Path(raw))
                values[args.receipt.resolve()]["arms"]["M3"][field] = value
                checked, loaded, hashed = _common_patches(values)
                with (
                    checked,
                    loaded,
                    hashed,
                    patch.object(matrix, "verify_arm", lambda arm, _value, **_context: {"arm": arm}),
                    self.assertRaisesRegex(matrix.GateError, "pending M3"),
                ):
                    matrix.verify(args)

    def test_m3_is_not_skipped_without_explicit_local_finalize_flag(self):
        with tempfile.TemporaryDirectory(prefix="cid1-passive-test-") as raw:
            args, values = _inputs(Path(raw))
            args.allow_external_m3_pending = False

            def reject_pending(arm, _value, **_context):
                if arm == "M3":
                    raise matrix.GateError("arm M3 is not complete")
                return {"arm": arm}

            checked, loaded, hashed = _common_patches(values)
            with (
                checked,
                loaded,
                hashed,
                patch.object(matrix, "verify_arm", reject_pending),
                self.assertRaisesRegex(matrix.GateError, "M3 is not complete"),
            ):
                matrix.verify(args)

    def test_matrix_receipt_requires_nonzero_nonce(self):
        with tempfile.TemporaryDirectory(prefix="cid1-passive-test-") as raw:
            args, values = _inputs(Path(raw))
            values[args.receipt.resolve()]["matrix_nonce_hex"] = "0" * 64
            checked, loaded, _hashed = _common_patches(values)
            with checked, loaded, self.assertRaisesRegex(matrix.GateError, "nonce is zero"):
                matrix.verify(args)

    def test_passive_request_contract_is_part_of_exact_schema(self):
        request = {key: None for key in matrix.PASSIVE_REQUEST_KEYS}
        del request["contract_sha256"]
        with self.assertRaisesRegex(matrix.GateError, "missing=.*contract_sha256"):
            matrix.exact(request, matrix.PASSIVE_REQUEST_KEYS, "passive request")


if __name__ == "__main__":
    unittest.main()
