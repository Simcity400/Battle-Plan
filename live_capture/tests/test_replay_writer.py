"""Tests for the streaming `.battleplan` writer (live_capture/replay_writer.py).

NO game, NO Rust required. Proves:

  * the Python canonical-JSON helper BYTE-MATCHES snapshot/src/canon.rs (the
    digest-critical invariant) -- checked against hand-computed canonical bytes
    AND against the actual Rust `pack_replay verify` when cargo is available;
  * a streamed recording finalizes into a valid container the reader accepts
    (Python re-verify always; Rust `pack_replay verify` when cargo is available),
    with byte-identical payload round-trip;
  * a crash MID-RECORDING (finalize never called, or called on a partial store)
    still yields a valid container of the pre-crash hours;
  * NO FABRICATION survives the writer: a measured 0 stays 0, an explicit null
    stays null, an absent field stays absent (payload byte-copied), and absent
    identity (modset/build None) becomes JSON null, never a guessed value;
  * the sidecar is append-only + skips non-canonical paths;
  * safety: an unsafe container path is refused.

Run:
    python -m unittest live_capture.tests.test_replay_writer
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from live_capture import replay_writer as rw  # noqa: E402
from live_capture.durable_store import DurableCaptureStore  # noqa: E402
from live_capture import assembler as asm  # noqa: E402
from live_capture import registry as reg  # noqa: E402
from replay import schema  # noqa: E402


# --------------------------------------------------------------------------
# Test fixtures
# --------------------------------------------------------------------------
class _FakeIdentity:
    def __init__(self, exe_sha="16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"):
        self.instance_id = "inst-A"
        self.pid = 4321
        self.exe_sha256 = exe_sha

    def build_ok(self):
        return True


def _stub_snapshot(date, hour_extra=None):
    """An assembler-built snapshot with a measured-0, an explicit-null, and an
    absent field, so the no-fabrication trichotomy can be checked after a round
    trip."""
    res = {
        "country_scalars": reg.DomainResult("country_scalars", True, True, {
            "countries": {
                # GER: measured stability 0.0 (must stay 0.0, not null/absent)
                "GER": {"stability": 0.0, "war_support": 0.4},
                # SOV: war_support absent -> stays null (unmeasured)
                "SOV": {"stability": 0.5},
            }, "count": 2}),
    }
    snap = asm.assemble_snapshot(res, game_id="rec_test", date=date,
                                 exe_sha256=_FakeIdentity().exe_sha256,
                                 identity=_FakeIdentity(), modset=None)
    return snap


def _read_bytes(path):
    with open(path, "rb") as fh:
        return fh.read()


def _pack_replay_exe():
    """Return the built pack_replay.exe path if present (so the Rust cross-check
    runs), else None (the Python re-verify still fully covers the invariants)."""
    cand = os.path.join(_REPO, "replay_native", "target", "debug",
                        "pack_replay.exe")
    if os.path.isfile(cand):
        return cand
    cand2 = os.path.join(_REPO, "replay_native", "target", "debug",
                         "pack_replay")
    return cand2 if os.path.isfile(cand2) else None


# --------------------------------------------------------------------------
# Canonical-JSON byte-match (the digest-critical invariant)
# --------------------------------------------------------------------------
class TestCanonicalJson(unittest.TestCase):
    def test_keys_sorted_minimal_separators(self):
        # mirrors canon.rs::keys_sorted_and_minimal
        v = {"b": 1, "a": [3, 2], "c": {"y": True, "x": None}}
        self.assertEqual(rw._canonical(v),
                         b'{"a":[3,2],"b":1,"c":{"x":null,"y":true}}')

    def test_string_escaping_unambiguous(self):
        # mirrors canon.rs::string_escaping_is_unambiguous (a slash + newline)
        v = {"path": "payload/g/x\n.json"}
        self.assertEqual(rw._canonical(v),
                         b'{"path":"payload/g/x\\n.json"}')

    def test_digest_strips_integrity_fields(self):
        # mirrors canon.rs::strips_integrity_fields_deterministically
        a = {"x": 1, "manifest_digest": "AAA", "signature": "S"}
        b = {"x": 1, "manifest_digest": "BBB", "signature": None}
        self.assertEqual(rw.manifest_digest(a), rw.manifest_digest(b))

    def test_non_ascii_value_kept_as_utf8(self):
        # ensure_ascii=False -> a non-ASCII string VALUE is raw UTF-8, matching
        # canon.rs's push(c) for c >= 0x20 (e.g. a mod name with an accent).
        v = {"name": "Björn"}
        self.assertEqual(rw._canonical(v), '{"name":"Björn"}'.encode("utf-8"))

    def test_digest_rejects_float_value(self):
        # Codex A: a float in a manifest numeric position would serialize as "17.0"
        # in Python but "17" in the Rust canon -> a SILENT cross-impl digest mismatch.
        # manifest_digest must FAIL CLOSED on it rather than ship a wrong digest.
        with self.assertRaises(ValueError):
            rw.manifest_digest({"bytes": 17.0, "manifest_digest": None,
                                "signature": None})
        # nested in a list (the entries array) is caught too.
        with self.assertRaises(ValueError):
            rw.manifest_digest({"entries": [{"bytes": 17.0}], "signature": None})
        # an honest int (and a bool, the intended build.pinned) digest fine.
        self.assertIsInstance(
            rw.manifest_digest({"bytes": 17, "pinned": True, "signature": None}), str)

    def test_digest_rejects_non_ascii_key(self):
        # Codex A: Python sort_keys orders by code point; canon.rs by UTF-16 code unit.
        # They agree only for ASCII keys -> a non-ASCII KEY must fail closed.
        with self.assertRaises(ValueError):
            rw.manifest_digest({"naïve": 1, "signature": None})


# --------------------------------------------------------------------------
# Streaming finalize -> valid container + byte round-trip
# --------------------------------------------------------------------------
class TestStreamingFinalize(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rec_test_")
        self.store_root = os.path.join(self.tmp, "store")
        self.out = os.path.join(self.tmp, "out", "rec_test.battleplan")
        self.store = DurableCaptureStore(self.store_root)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _write_hours(self, recorder, dates):
        paths = []
        for d in dates:
            snap = _stub_snapshot(d)
            # stamp a segment so the continuity-keyed filename is stable.
            snap["_capture"] = {"segment_id": 0, "campaign_id": "c0",
                                "capture_seq": len(paths)}
            p = self.store.write_snapshot(snap, allow_unknown_hour=False)
            recorder.on_snapshot_written(p)
            paths.append(p)
        return paths

    def _recorder(self, modset=None, build="abc123", pinned=True):
        return rw.ReplayRecorder(
            store_root=self.store_root, game_id="rec_test", out_path=self.out,
            modset=modset, build_exe_sha256=build, build_pinned=pinned,
            schema_version=schema.SCHEMA_VERSION)

    def test_finalize_and_verify_python(self):
        rec = self._recorder()
        self._write_hours(rec, ["1936.1.1.12", "1936.1.1.13", "1936.1.1.14"])
        self.assertTrue(rec.finalize_and_verify())
        self.assertTrue(os.path.isfile(self.out))
        # re-verify directly too.
        self.assertTrue(rw._verify_container(self.out))

    def test_byte_identical_payload_roundtrip(self):
        rec = self._recorder()
        paths = self._write_hours(rec, ["1936.1.1.12", "1936.1.1.13"])
        rec.finalize()
        by_name = rw._zip_read_all(_read_bytes(self.out))
        for p in paths:
            file = os.path.basename(p)
            cpath = "payload/rec_test/%s" % file
            self.assertIn(cpath, by_name)
            with open(p, "rb") as fh:
                orig = fh.read()
            # raw bytes survive untouched (loss-LESS) -> byte-identity.
            self.assertEqual(by_name[cpath], orig)
        # index.json is packed too.
        self.assertIn("payload/rec_test/index.json", by_name)

    def test_no_fabrication_trichotomy_survives(self):
        rec = self._recorder()
        self._write_hours(rec, ["1936.1.1.12"])
        rec.finalize()
        by_name = rw._zip_read_all(_read_bytes(self.out))
        # find the snapshot entry (the one .snapshot.json under payload)
        snap_paths = [k for k in by_name
                      if k.endswith(".snapshot.json") and "index" not in k]
        self.assertEqual(len(snap_paths), 1)
        snap = json.loads(by_name[snap_paths[0]].decode("utf-8"))
        ger = snap["countries"]["GER"]["politics"]
        sov = snap["countries"]["SOV"]["politics"]
        # measured 0 stays 0 (not null, not absent)
        self.assertEqual(ger["stability"], 0.0)
        self.assertIn("stability", ger)
        # explicit null (war_support unmeasured for SOV) stays null
        self.assertIsNone(sov["war_support"])

    def test_absent_identity_is_json_null_not_fabricated(self):
        # modset None + build None -> manifest records JSON null, never a guess.
        rec = rw.ReplayRecorder(
            store_root=self.store_root, game_id="rec_test", out_path=self.out,
            modset=None, build_exe_sha256=None, build_pinned=False,
            schema_version=schema.SCHEMA_VERSION)
        self._write_hours(rec, ["1936.1.1.12"])
        rec.finalize()
        by_name = rw._zip_read_all(_read_bytes(self.out))
        man = json.loads(by_name["manifest.json"].decode("utf-8"))
        self.assertIsNone(man["mod_identity"])
        self.assertIsNone(man["build"]["exe_sha256"])
        self.assertFalse(man["build"]["pinned"])

    def test_modset_passed_through_verbatim(self):
        ms = {"modset_id": "deadbeef", "source": "dlc_load.json",
              "load_order": ["test.mod"], "mods": []}
        rec = self._recorder(modset=ms)
        self._write_hours(rec, ["1936.1.1.12"])
        rec.finalize()
        by_name = rw._zip_read_all(_read_bytes(self.out))
        man = json.loads(by_name["manifest.json"].decode("utf-8"))
        self.assertEqual(man["mod_identity"], ms)

    def test_manifest_field_set_matches_reference(self):
        # the manifest carries exactly the pack.rs field set (so the container is
        # reader-identical).
        rec = self._recorder()
        self._write_hours(rec, ["1936.1.1.12"])
        rec.finalize()
        by_name = rw._zip_read_all(_read_bytes(self.out))
        man = json.loads(by_name["manifest.json"].decode("utf-8"))
        for k in ("format", "container_version", "required_features",
                  "optional_features", "schema_version", "game_id", "build",
                  "mod_identity", "created_utc", "recorder", "counts", "entries",
                  "manifest_digest", "signature"):
            self.assertIn(k, man)
        self.assertEqual(man["format"], "hoi4replay")
        self.assertEqual(man["container_version"], 1)
        self.assertEqual(man["recorder"],
                         {"tool": "hoi4replay-recorder", "tool_version": "1.0.0"})
        self.assertEqual(man["counts"]["snapshots"], 1)

    @unittest.skipUnless(_pack_replay_exe(), "pack_replay.exe not built")
    def test_rust_reader_accepts_container(self):
        # the cross-impl proof: the RUST reader opens + verifies + byte-identity
        # checks the container this Python writer produced.
        rec = self._recorder()
        self._write_hours(rec, ["1936.1.1.12", "1936.1.1.13", "1936.1.1.14"])
        rec.finalize()
        exe = _pack_replay_exe()
        gdir_root = self.store_root
        r = subprocess.run([exe, "verify", self.out, gdir_root, "rec_test"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0,
                         "pack_replay verify failed: %s" % r.stderr)
        self.assertIn("verify: OK", r.stderr)
        self.assertIn("byte-identity OK", r.stderr)


# --------------------------------------------------------------------------
# Crash mid-recording -> still valid
# --------------------------------------------------------------------------
class TestCrashSafety(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rec_crash_")
        self.store_root = os.path.join(self.tmp, "store")
        self.out = os.path.join(self.tmp, "rec.battleplan")
        self.store = DurableCaptureStore(self.store_root)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _rec(self):
        return rw.ReplayRecorder(
            store_root=self.store_root, game_id="rec_test", out_path=self.out,
            modset=None, build_exe_sha256="abc", build_pinned=True,
            schema_version=schema.SCHEMA_VERSION)

    def __init__(self, *a, **kw):
        super().__init__(*a, **kw)
        self._day = 0

    def _write(self, rec, n):
        for _ in range(n):
            self._day += 1
            snap = _stub_snapshot("1936.1.%d.12" % self._day)
            snap["_capture"] = {"segment_id": 0, "campaign_id": "c0",
                                "capture_seq": self._day}
            p = self.store.write_snapshot(snap, allow_unknown_hour=False)
            rec.on_snapshot_written(p)

    def test_interrupted_record_recovers_via_refinalize(self):
        # simulate: 3 hours streamed into the store + sidecar, then the process
        # "died" before finalize (no finalize() called). A FRESH recorder over the
        # same store finalizes a valid container of those 3 hours.
        rec = self._rec()
        self._write(rec, 3)
        # process "dies" here -- nothing finalized; the store + sidecar persist.
        del rec
        # recovery: a new recorder over the same store re-finalizes.
        rec2 = self._rec()
        rec2.finalize()
        self.assertTrue(rw._verify_container(self.out))
        by_name = rw._zip_read_all(_read_bytes(self.out))
        snaps = [k for k in by_name
                 if k.endswith(".snapshot.json") and "index" not in k]
        self.assertEqual(len(snaps), 3)

    def test_finalize_midstream_then_more_hours(self):
        # finalize after 2 hours (a valid 2-hour file), then 2 more + finalize ->
        # a valid 4-hour file. Each finalize is a complete atomic re-publish.
        rec = self._rec()
        self._write(rec, 2)
        rec.finalize()
        self.assertTrue(rw._verify_container(self.out))
        n2 = len([k for k in rw._zip_read_all(_read_bytes(self.out))
                  if k.endswith(".snapshot.json") and "index" not in k])
        self.assertEqual(n2, 2)
        self._write(rec, 2)
        rec.finalize()
        self.assertTrue(rw._verify_container(self.out))
        n4 = len([k for k in rw._zip_read_all(_read_bytes(self.out))
                  if k.endswith(".snapshot.json") and "index" not in k])
        self.assertEqual(n4, 4)

    def test_finalize_fails_closed_on_missing_referenced_file(self):
        # Codex: silently dropping an indexed-but-missing snapshot would publish a
        # container that omits data the index advertises. finalize must REFUSE.
        rec = self._rec()
        self._write(rec, 2)
        # delete one snapshot file the index references (simulate a torn store).
        gdir = os.path.join(self.store_root, "rec_test")
        with open(os.path.join(gdir, "index.json"), encoding="utf-8") as fh:
            idx = json.load(fh)
        victim = os.path.join(gdir, idx["snapshots"][0]["file"])
        os.remove(victim)
        with self.assertRaises(ValueError):
            rec.finalize()

    def test_finalize_fails_closed_on_unsafe_index_file_name(self):
        # a tampered index naming a traversal/separator file -> fail closed.
        rec = self._rec()
        self._write(rec, 1)
        gdir = os.path.join(self.store_root, "rec_test")
        ipath = os.path.join(gdir, "index.json")
        with open(ipath, encoding="utf-8") as fh:
            idx = json.load(fh)
        idx["snapshots"][0]["file"] = "../escape.snapshot.json"
        with open(ipath, "w", encoding="utf-8") as fh:
            json.dump(idx, fh)
        with self.assertRaises(ValueError):
            rec.finalize()

    def test_finalize_fails_closed_on_malformed_file_entry(self):
        # Codex F1 residual: an index snapshot entry with no usable `file` advertises
        # a snapshot with no packable payload -> fail closed.
        rec = self._rec()
        self._write(rec, 1)
        gdir = os.path.join(self.store_root, "rec_test")
        ipath = os.path.join(gdir, "index.json")
        with open(ipath, encoding="utf-8") as fh:
            idx = json.load(fh)
        idx["snapshots"].append({"hour_index": 99, "file": None})
        with open(ipath, "w", encoding="utf-8") as fh:
            json.dump(idx, fh)
        with self.assertRaises(ValueError):
            rec.finalize()

    def test_finalize_fails_closed_on_duplicate_container_path(self):
        # Codex F2 hardening: a snapshot whose `file` collides with index.json must
        # not produce a duplicate-path zip the reader would reject.
        rec = self._rec()
        self._write(rec, 1)
        gdir = os.path.join(self.store_root, "rec_test")
        ipath = os.path.join(gdir, "index.json")
        with open(ipath, encoding="utf-8") as fh:
            idx = json.load(fh)
        idx["snapshots"][0]["file"] = "index.json"  # collide with the real index
        with open(ipath, "w", encoding="utf-8") as fh:
            json.dump(idx, fh)
        # also create a file named index.json-as-snapshot would resolve to (the real
        # index.json already exists), so the open succeeds and the dup-path guard
        # (not a missing-file error) is what fires.
        with self.assertRaises(ValueError):
            rec.finalize()

    def test_sidecar_reconciliation_flags_lost_index_update(self):
        # the sidecar references a canonical path the (rolled-back) index doesn't
        # pack -> finalize records a recovery note (sidecar is NOT dead code).
        rec = self._rec()
        self._write(rec, 2)
        gdir = os.path.join(self.store_root, "rec_test")
        ipath = os.path.join(gdir, "index.json")
        with open(ipath, encoding="utf-8") as fh:
            idx = json.load(fh)
        # simulate a lost index update: drop the last snapshot from the index but
        # keep its file + sidecar line.
        dropped = idx["snapshots"].pop()
        with open(ipath, "w", encoding="utf-8") as fh:
            json.dump(idx, fh)
        rec.finalize()
        # the recovery note names the path the sidecar streamed but the index lost.
        notes = getattr(rec, "_last_recovery", [])
        self.assertTrue(any(dropped["file"] in n for n in notes),
                        "sidecar reconciliation should flag the lost hour")
        # the published container is still valid (store is authoritative).
        self.assertTrue(rw._verify_container(self.out))

    def test_torn_tmp_never_replaces_published(self):
        # the published file is only ever a complete container (.tmp -> os.replace).
        # A leftover .tmp must not be treated as the output.
        rec = self._rec()
        self._write(rec, 1)
        rec.finalize()
        # forge a torn .tmp next to the output; it must not affect the published.
        with open(self.out + ".tmp", "wb") as fh:
            fh.write(b"PK\x03\x04 garbage")
        self.assertTrue(rw._verify_container(self.out))


# --------------------------------------------------------------------------
# Sidecar behavior
# --------------------------------------------------------------------------
class TestSidecar(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rec_side_")
        self.store_root = os.path.join(self.tmp, "store")
        self.out = os.path.join(self.tmp, "rec.battleplan")
        self.store = DurableCaptureStore(self.store_root)
        self.rec = rw.ReplayRecorder(
            store_root=self.store_root, game_id="rec_test", out_path=self.out,
            modset=None, build_exe_sha256="abc", build_pinned=True,
            schema_version=schema.SCHEMA_VERSION)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_sidecar_is_append_only(self):
        sc = self.rec._sidecar_path
        for i in range(3):
            snap = _stub_snapshot("1936.1.%d.12" % (i + 1))
            snap["_capture"] = {"segment_id": 0, "campaign_id": "c0",
                                "capture_seq": i}
            p = self.store.write_snapshot(snap, allow_unknown_hour=False)
            self.rec.on_snapshot_written(p)
        with open(sc, encoding="utf-8") as _f:
            lines = [ln for ln in _f.read().splitlines() if ln]
        self.assertEqual(len(lines), 3)
        for ln in lines:
            rec = json.loads(ln)
            self.assertEqual(rec["role"], "snapshot")
            self.assertTrue(rec["path"].startswith("payload/rec_test/"))

    def test_noncanonical_path_skipped(self):
        sc = self.rec._sidecar_path
        # an unknown-hour snapshot goes to _noncanonical/ -> the recorder skips it.
        snap = _stub_snapshot(None)   # no trusted date -> hour_index None
        snap.pop("hour_index", None)
        snap["hour_index"] = None
        p = self.store.write_snapshot(snap, allow_unknown_hour=False)
        self.assertIn("_noncanonical", p)
        self.rec.on_snapshot_written(p)
        self.assertFalse(os.path.isfile(sc) and
                         os.path.getsize(sc) > 0)

    def test_sidecar_append_repairs_missing_trailing_newline(self):
        # CRASH-SAFETY (Codex B): if a prior crash left a syntactically-valid JSON
        # object in the sidecar WITHOUT its trailing newline, the NEXT append must
        # NOT concatenate the two onto one physical line (which json.loads can't
        # parse -> BOTH records' paths are lost from the reconcile). The newline is
        # the commit marker: a new append repairs the tail first, so every record
        # lands on its own parseable line.
        sc = self.rec._sidecar_path
        os.makedirs(os.path.dirname(sc), exist_ok=True)
        torn = json.dumps({"path": "payload/rec_test/s0000.000000000001.snapshot.json",
                           "sha256": "0" * 64, "bytes": 10, "role": "snapshot",
                           "file": "s0000.000000000001.snapshot.json"},
                          separators=(",", ":"))
        with open(sc, "w", encoding="utf-8") as fh:
            fh.write(torn)               # NO trailing newline (crash before the \n)
        # now stream a real snapshot -> on_snapshot_written appends another record.
        snap = _stub_snapshot("1936.1.2.12")
        snap["_capture"] = {"segment_id": 0, "campaign_id": "c0", "capture_seq": 1}
        p = self.store.write_snapshot(snap, allow_unknown_hour=False)
        self.rec.on_snapshot_written(p)
        with open(sc, encoding="utf-8") as fh:
            lines = [ln for ln in fh.read().splitlines() if ln]
        # BOTH records must be present AND each must parse on its own line.
        self.assertEqual(len(lines), 2, "the torn first record and the new one "
                         "must each occupy their own parseable line")
        recs = [json.loads(ln) for ln in lines]   # MUST NOT raise
        paths = {r["path"] for r in recs}
        self.assertIn("payload/rec_test/s0000.000000000001.snapshot.json", paths)


# --------------------------------------------------------------------------
# Safety: bad container path refused
# --------------------------------------------------------------------------
class TestPathSafety(unittest.TestCase):
    def test_traversal_and_absolute_paths_rejected(self):
        for bad in ("../escape.json", "/abs.json", "C:/x.json",
                    "payload\\g\\x.json", "payload/../x.json",
                    "payload/g/CON.json"):
            self.assertFalse(rw._path_ok(bad), "should reject %r" % bad)
        for good in ("payload/rec_test/index.json",
                     "payload/rec_test/s0000.000000000017.snapshot.json",
                     "manifest.json"):
            self.assertTrue(rw._path_ok(good), "should accept %r" % good)


if __name__ == "__main__":
    unittest.main()
