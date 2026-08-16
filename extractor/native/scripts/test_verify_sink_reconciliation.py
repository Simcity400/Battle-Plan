"""Golden vectors for the THIRD re-derivation of the CaptureReadyV2 payload.

The 512-byte authenticated `H4RD` payload is laid out by three independent
implementations: the producer (`CaptureReadyV2::encode_authenticated` in
`compact_contract/src/control_transport/payloads.rs`), the harness mirror
(`Get-PassiveCaptureReadyPayloadSha256` in `PassiveCapture.Lifecycle.ps1`), and
this file's subject, `verify_sink_reconciliation.capture_ready_payload_sha256`.
The first two are pinned against each other by
`passive_capture_ready_pins_the_harness_rebuild_vectors_at_offset_50` and by the
`t63_install_freeze_*` cases in
`PassiveCaptureReady.ReceiptDialects.fixture.v1.json`. The third had no test at
all, and it had drifted TWICE without anything noticing: it wrote a constant
zero at offset 20 (the collector's compile-time feature word) and another at
offset 50 (the install-freeze count), so it disagreed with every receipt the
recorder had written for weeks.

WHAT THESE TESTS ARE. The same vectors the other two implementations are pinned
to, computed from the same receipt fields and the same pipe token. They are
golden numbers produced by the Rust encoder, not by this module, so this module
restating its own output cannot satisfy them.

WHAT PRODUCTION CHANGE MAKES THEM FAIL. Putting a constant back at offset 20 or
offset 50; moving either field; changing a field's width or the little-endian
order; reordering the thirteen digests; changing the HMAC key derivation or the
final SHA-256; or making a missing `install_freeze_count` fall back to zero
instead of refusing the rebuild.
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "verify_sink_reconciliation",
    Path(__file__).with_name("verify_sink_reconciliation.py"),
)
assert _SPEC is not None and _SPEC.loader is not None
_VSR = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_VSR)

# Run t63's own receipt fields, with `pipe_token_sha256` rebound to the token
# below because that is the only per-attach token still known and the payload's
# token binding requires the two to agree. Identical to the `t63_install_freeze_*`
# fixture cases and to `t63_ready_fields` in the Rust test.
PIPE_TOKEN = "f743c8f124d2572ad18b99db033554ba"

T63_RECEIPT: dict[str, object] = {
    "readiness_flags": 255,
    "pid": 34900,
    "collector_feature_bits": 1,
    "process_creation_filetime_utc": 134313300665890026,
    "requested_duration_ms": 120000,
    "validated_domain_count": 67,
    "scheduler_queue_capacity": 1,
    "scheduler_worker_count": 12,
    "allowed_hook_count": 45,
    "active_hook_count": 45,
    "install_freeze_count": 13,
    "clock_hour_raw": 60831264,
    "clock_observation_ordinal": 12,
    "target_executable_sha256":
        "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    "collector_dll_sha256":
        "18883f3ad61d796d13e406b28869e764efca61863928150fbe08cdcb23bb07b8",
    "hook_allowlist_sha256":
        "54348f3d32277a114474c0a2b667ddf32cab6388adfd2a42500cac79ea8eaa79",
    "hook_outcomes_sha256":
        "416db3019bcd56f982a9e11978bcc47a0b9dc06d23147935d56dbbf30907951b",
    "validated_domain_roster_sha256":
        "3e74b2f4cdad7b9a4f6f5326cba28dd14bf0eb2f8475be8f87b39ef65bd4b1ca",
    "clock_witness_sha256":
        "b9783a3e914bc1cb498e69ab9d6d2c5c7979854e6b5d4d981cad6b0539922b39",
    "scheduler_config_sha256":
        "bccf0cf2af1a2d002316ce163e5c41504b1507dca3ebd901e5082866c56a9bc4",
    "output_pipe_identity_sha256":
        "8d85b8126cb3811b7f49b002c865d543cf66cd8f0c3c3c854f520bb975499218",
    "pipe_token_sha256":
        "468803408b348616f75c0e5fad146849d75c4b03af945baf14a1412e205afd3b",
    "zero_mutation_policy_sha256":
        "bfd0f328539c2dc07982553870d218d8c6dd5b2ab2683ec082b3392cf465dcae",
    "session_id_sha256":
        "33c6ece20b57f5ebaaa0fbef6f384c0bce421f511082c56a3581657e19d6b602",
    "capture_id_sha256":
        "b751d03e68c6f9edc58e5a6924031a584a04d827e1cf44b198e185acaf22648d",
    "control_sha256":
        "758746092eb8b83c15916bce1842ab8d1d50d99dd0c14cafe0d3403206bded36",
}

# Produced by CaptureReadyV2::encode_authenticated, not by this module.
PAYLOAD_SHA_FREEZE_13 = \
    "e8b08ebca73f6b9d4821931b3c53f74dc55e7c48bc203b0f6a69f4b596ad16b8"
PAYLOAD_SHA_FREEZE_1000 = \
    "5089b85d8f0fa835790baa90d45984c8c1b64036e88da2f7070ac616aff27bfe"


def receipt(**overrides: object) -> dict[str, object]:
    merged = dict(T63_RECEIPT)
    merged.update(overrides)
    return merged


class PayloadDigestMatchesTheProducer(unittest.TestCase):
    def test_the_real_receipts_count_reaches_offset_50(self) -> None:
        self.assertEqual(
            _VSR.capture_ready_payload_sha256(receipt(), PIPE_TOKEN),
            PAYLOAD_SHA_FREEZE_13,
        )

    def test_a_second_count_moves_the_digest(self) -> None:
        """One vector is satisfied by a constant 13, exactly as the broken
        implementation satisfied everything with a constant 0. 1000 also does
        not fit in a byte, so a one-byte-plus-pad encoding fails here too."""
        self.assertEqual(
            _VSR.capture_ready_payload_sha256(
                receipt(install_freeze_count=1000), PIPE_TOKEN
            ),
            PAYLOAD_SHA_FREEZE_1000,
        )

    def test_the_feature_word_reaches_offset_20(self) -> None:
        """Offset 20 is the SAME defect one revision earlier and was equally
        untested here. A constant there gives the declared and undeclared
        dialects one digest."""
        self.assertNotEqual(
            _VSR.capture_ready_payload_sha256(
                receipt(collector_feature_bits=0), PIPE_TOKEN
            ),
            PAYLOAD_SHA_FREEZE_13,
        )

    def test_a_receipt_without_the_count_is_refused_not_assumed(self) -> None:
        """A missing property must not be rebuilt as a zero. That assumption is
        what turned a producer skew into a digest mismatch with nothing in it
        for an operator to read."""
        without = receipt()
        del without["install_freeze_count"]
        self.assertIsNone(
            _VSR.capture_ready_payload_sha256(without, PIPE_TOKEN)
        )

    def test_the_token_is_the_hmac_key(self) -> None:
        self.assertNotEqual(
            _VSR.capture_ready_payload_sha256(receipt(), PIPE_TOKEN + "0"),
            PAYLOAD_SHA_FREEZE_13,
        )


class ReceiptRosterAdmitsWhatTheRecorderWrites(unittest.TestCase):
    def test_install_freeze_count_is_a_required_receipt_property(self) -> None:
        """The roster and the rebuild have to agree that the field is required;
        a roster that still called it optional would admit the receipt and then
        fail on the digest, which is the failure this lane exists to remove."""
        source = Path(__file__).with_name(
            "verify_sink_reconciliation.py"
        ).read_text(encoding="utf-8")
        head, _, tail = source.partition("ready_keys = {")
        self.assertTrue(tail, "the ready_keys roster is no longer spelled here")
        roster, _, _ = tail.partition("}")
        self.assertIn('"install_freeze_count"', roster)

    def test_a_missing_count_is_reported_by_name(self) -> None:
        """`capture_ready_property_set_invalid` and the digest problem are both
        true when the field is absent, and neither tells an operator which field
        moved. The whole cost of this defect was a refusal nobody could read, so
        the absence gets a problem code of its own."""
        problems: list[str] = []
        _VSR.require(
            "install_freeze_count" in receipt(), "unexpected", problems
        )
        self.assertEqual(problems, [])
        without = receipt()
        del without["install_freeze_count"]
        _VSR.require(
            "install_freeze_count" in without,
            "capture_ready_install_freeze_count_missing",
            problems,
        )
        self.assertEqual(
            problems, ["capture_ready_install_freeze_count_missing"]
        )

    def test_the_named_check_is_wired_into_the_receipt_verifier(self) -> None:
        """The assertion above proves the problem code behaves; this one proves
        production actually raises it. Deleting the `require(...)` call leaves
        the code spelled nowhere and this fails."""
        source = Path(__file__).with_name(
            "verify_sink_reconciliation.py"
        ).read_text(encoding="utf-8")
        self.assertIn(
            '"capture_ready_install_freeze_count_missing"', source
        )
        self.assertIn('"install_freeze_count" in ready,', source)


if __name__ == "__main__":
    unittest.main(verbosity=2)
