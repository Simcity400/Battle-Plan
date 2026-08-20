import hashlib
import json
import multiprocessing
import sqlite3
import sys
import tempfile
import unittest
from contextlib import closing
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import server
from correlation import correlate
from diffing import diff_builds
from emulation import emulate_x64_blob
from evidence import ingest_ghidra_jobs, parse_instruction_artifact
from offline_analysis import (
    _read_function_index_snapshot, _run_command, candidate_census, cross_analyze, inspect_pe,
    offline_slot,
)
from operations import Broker, Config
from registry import Registry


SHA_A = "a" * 64
SHA_B = "b" * 64
IMAGE_BASE = 0x140000000


def hold_offline_slot(root, slots, queue):
    import time

    with offline_slot(Path(root), slots, 10) as slot:
        queue.put(("start", time.monotonic(), slot))
        time.sleep(0.2)
        queue.put(("end", time.monotonic(), slot))


def function_row(build_id, rva, name, byte_hash, shape):
    return {
        "build_id": build_id,
        "rva": rva,
        "name": name,
        "body_min": rva,
        "body_max": rva + 8,
        "instruction_count": len(shape.split()),
        "call_count": shape.split().count("CALL:IMM"),
        "byte_sha256": byte_hash,
        "shape_sha256": hashlib.sha256(shape.encode()).hexdigest(),
        "shape_text": shape,
        "source_job_id": "fixture",
        "source_artifact": "fixture.txt",
        "source_artifact_sha256": "f" * 64,
    }


class RegistryDiffTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.registry = Registry(Path(self.temp.name) / "registry.sqlite3")
        self.a = self.registry.upsert_build("hoi4.exe a", SHA_A, "0x140000000")
        self.b = self.registry.upsert_build("hoi4.exe b", SHA_B, "0x140000000")

    def tearDown(self):
        self.temp.cleanup()

    def test_diff_links_exact_and_shape_matches_but_not_unmatched(self):
        common = hashlib.sha256(b"common").hexdigest()
        self.registry.upsert_functions([
            function_row(self.a, 0x1000, "old_exact", common, "MOV:REG,REG RET"),
            function_row(self.a, 0x2000, "old_shape", hashlib.sha256(b"old").hexdigest(), "MOV:REG,IMM ADD:REG,REG RET"),
            function_row(self.a, 0x3000, "old_missing", hashlib.sha256(b"missing").hexdigest(), "XOR:REG,REG RET"),
            function_row(self.b, 0x5000, "new_exact", common, "MOV:REG,REG RET"),
            function_row(self.b, 0x6000, "new_shape", hashlib.sha256(b"new").hexdigest(), "MOV:REG,IMM ADD:REG,REG RET"),
        ])
        result = diff_builds(self.registry, self.a, self.b, "job")
        self.assertEqual(3, result["source_function_count"])
        self.assertEqual(2, result["matched_function_count"])
        self.assertEqual(1, result["dispositions"]["exact"])
        self.assertEqual(1, result["dispositions"]["high_confidence"])
        self.assertEqual(1, result["dispositions"]["unmatched"])
        source_symbol = self.registry.source_symbol(self.a, 0x1000)
        self.assertEqual(source_symbol, self.registry.source_symbol(self.b, 0x5000))
        self.assertEqual("draft-not-promoted", result["compatibility_pack"]["status"])
        self.assertEqual("0x5000", result["compatibility_pack"]["symbols"][source_symbol]["rva"])
        self.assertFalse(result["promotion_allowed"])

    def test_duplicate_fingerprint_is_ambiguous(self):
        common = hashlib.sha256(b"duplicate").hexdigest()
        shape = "MOV:REG,REG RET"
        self.registry.upsert_functions([
            function_row(self.a, 0x1000, "old", common, shape),
            function_row(self.b, 0x5000, "new_a", common, shape),
            function_row(self.b, 0x6000, "new_b", common, shape),
        ])
        result = diff_builds(self.registry, self.a, self.b, "job")
        self.assertEqual(1, result["dispositions"]["ambiguous"])
        self.assertEqual(0, result["matched_function_count"])

    def test_duplicate_source_fingerprint_is_not_auto_ported(self):
        common = hashlib.sha256(b"duplicate-source").hexdigest()
        shape = "XOR:REG,REG RET"
        self.registry.upsert_functions([
            function_row(self.a, 0x1000, "old_a", common, shape),
            function_row(self.a, 0x2000, "old_b", common, shape),
            function_row(self.b, 0x5000, "new", common, shape),
        ])
        result = diff_builds(self.registry, self.a, self.b, "job")
        self.assertEqual(2, result["dispositions"]["ambiguous"])
        self.assertEqual(0, result["matched_function_count"])


class RegistryFunctionClaimTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.registry = Registry(Path(self.temp.name) / "registry.sqlite3")

    def tearDown(self):
        self.temp.cleanup()

    def _claim(self, claim_id, value, build_id=None):
        self.registry.record_claim(
            {
                "claim_id": claim_id,
                "subject": "fixture",
                "predicate": "annotation",
                "value": value,
                "status": "proposed",
                "build_id": build_id,
                "evidence_job_ids": [],
            }
        )

    def test_function_claim_lookup_uses_boundaries_func_paths_and_build_isolation(self):
        address = 0x140C2C920
        self._claim("claim_long", {"note": "mentions 0x140c2c9200 only"})
        self._claim("claim_func", {"path": "evidence/func_140c2c920.graph.txt"})
        other_build = self.registry.upsert_build("other", SHA_B, "0x140000000")
        self._claim("claim_wrong_build", {"note": "0x140c2c920"}, other_build)
        rows = self.registry.claims_for_function(address, IMAGE_BASE, SHA_A)
        self.assertEqual(["claim_func"], [row["claim_id"] for row in rows])

    def test_address_token_is_not_matched_inside_a_longer_identifier(self):
        address = 0x140C2C920
        self._claim("claim_embedded", {"note": "prefix0x140c2c920suffix"})
        self._claim("claim_real", {"note": "0x140c2c920 is the producer"})
        rows = self.registry.claims_for_function(address, IMAGE_BASE, SHA_A)
        self.assertEqual(["claim_real"], [row["claim_id"] for row in rows])

    def test_rva_forms_are_not_matched_inside_a_longer_identifier(self):
        address = 0x140C2C920
        self._claim("rva_embedded", {"note": "rva:0xc2c920suffix"})
        self._claim("rva_prefixed", {"note": "prefixrva:0xc2c920"})
        self._claim("plus_embedded", {"note": "at +0xc2c920suffix"})
        self._claim("rva_real", {"note": "rva:0xc2c920 confirmed"})
        rows = self.registry.claims_for_function(address, IMAGE_BASE, SHA_A)
        self.assertEqual(["rva_real"], [row["claim_id"] for row in rows])

    def test_orphan_build_claims_remain_visible_and_are_labelled(self):
        # A claim whose build row no longer exists is legacy, not evidence about a
        # different binary; hiding it loses real findings for this exact address.
        address = 0x140C2C920
        orphan_build = self.registry.upsert_build("legacy", SHA_B, "0x140000000")
        self._claim("claim_orphan", {"note": "0x140c2c920 verified"}, orphan_build)
        with self.registry.connect() as connection:
            connection.execute("DELETE FROM builds WHERE build_id=?", (orphan_build,))
            connection.commit()
        rows = self.registry.claims_for_function(address, IMAGE_BASE, SHA_A)
        self.assertEqual(["claim_orphan"], [row["claim_id"] for row in rows])
        self.assertEqual("orphan", rows[0]["build_attribution"])

    def test_claims_for_a_different_registered_build_stay_isolated(self):
        address = 0x140C2C920
        other_build = self.registry.upsert_build("other", SHA_B, "0x140000000")
        self._claim("claim_other", {"note": "0x140c2c920"}, other_build)
        self.registry.upsert_build("current", SHA_A, "0x140000000")
        rows = self.registry.claims_for_function(address, IMAGE_BASE, SHA_A)
        self.assertEqual([], [row["claim_id"] for row in rows])


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_instruction_artifact_is_address_independent(self):
        path = self.root / "func_140001000.instructions.txt"
        path.write_text(
            "binary_id=hoi4.exe test\n"
            "executable_sha256=" + SHA_A + "\n"
            "image_base=0x140000000\n"
            "function=Named@140001000\n"
            "body_min=140001000\nbody_max=140001008\n\n"
            "140001000  488bc1  MOV RAX, RCX\n"
            "140001003  4883c001  ADD RAX, 0x1\n"
            "140001007  c3  RET\n",
            encoding="utf-8",
        )
        row = parse_instruction_artifact(path, "build", IMAGE_BASE, "job")
        self.assertEqual(0x1000, row["rva"])
        self.assertEqual("Named", row["name"])
        self.assertEqual("MOV:REG,REG ADD:REG,IMM RET", row["shape_text"])
        self.assertEqual(3, row["instruction_count"])

    def test_instruction_artifact_accepts_decorated_name_with_at_signs(self):
        path = self.root / "func_140001000.instructions.txt"
        path.write_text(
            "binary_id=hoi4.exe test\nexecutable_sha256=" + SHA_A + "\n"
            "image_base=0x140000000\n"
            "function=method@namespace@@QEAAHXZ@140001000\n"
            "body_min=140001000\nbody_max=140001000\n\n140001000  c3  RET\n",
            encoding="utf-8",
        )
        row = parse_instruction_artifact(path, "build", IMAGE_BASE, "job")
        self.assertEqual("method@namespace@@QEAAHXZ", row["name"])
        self.assertEqual(0x1000, row["rva"])

    def test_ingest_verifies_manifest_artifact_and_indexes_function(self):
        registry = Registry(self.root / "registry.sqlite3")
        job = self.root / "exports" / "20260721T000000Z-test-1234abcd"
        evidence = job / "evidence"
        evidence.mkdir(parents=True)
        instruction = evidence / "func_140001000.instructions.txt"
        instruction.write_text(
            "binary_id=hoi4.exe test\nexecutable_sha256=" + SHA_A + "\n"
            "image_base=0x140000000\nfunction=FUN_140001000@140001000\n"
            "body_min=140001000\nbody_max=140001000\n\n140001000  c3  RET\n",
            encoding="utf-8",
        )
        digest = hashlib.sha256(instruction.read_bytes()).hexdigest()
        manifest = {
            "schema": "hoi4-function-evidence-v1", "binary_id": "hoi4.exe test",
            "executable_sha256": SHA_A, "image_base": "0x140000000",
            "artifacts": [{"path": instruction.name, "bytes": instruction.stat().st_size, "sha256": digest}],
        }
        (evidence / "_FUNCTION_EVIDENCE_MANIFEST.json").write_text(json.dumps(manifest), encoding="utf-8")
        (job / "job.json").write_text(json.dumps({"status": "complete", "operation": "export_functions"}), encoding="utf-8")
        result = ingest_ghidra_jobs(registry, job.parent)
        self.assertEqual(1, result["functions_ingested"])
        build_id = result["build_ids"][0]
        self.assertEqual(1, len(registry.functions_for_build(build_id)))
        repeated = ingest_ghidra_jobs(registry, job.parent)
        self.assertEqual(1, repeated["manifests_skipped"])
        self.assertEqual(0, repeated["functions_ingested"])

    def test_ingest_reports_invalid_job_identity_and_error(self):
        registry = Registry(self.root / "registry.sqlite3")
        job = self.root / "exports" / "20260721T000000Z-invalid-1234abcd"
        evidence = job / "evidence"
        evidence.mkdir(parents=True)
        instruction = evidence / "func_140001000.instructions.txt"
        instruction.write_text(
            "binary_id=hoi4.exe test\nexecutable_sha256=" + SHA_A + "\n"
            "image_base=0x140000000\nfunction=FUN_140001000@140001000\n"
            "body_min=140001000\nbody_max=140001000\n\n140001000  c3  RET\n",
            encoding="utf-8",
        )
        manifest = {
            "binary_id": "hoi4.exe test", "executable_sha256": SHA_A,
            "image_base": "0x140000000",
            "artifacts": [{
                "path": instruction.name, "bytes": instruction.stat().st_size,
                "sha256": "0" * 64,
            }],
        }
        (evidence / "_FUNCTION_EVIDENCE_MANIFEST.json").write_text(
            json.dumps(manifest), encoding="utf-8",
        )
        (job / "job.json").write_text(
            json.dumps({"status": "complete", "operation": "export_functions"}),
            encoding="utf-8",
        )
        result = ingest_ghidra_jobs(registry, job.parent)
        self.assertEqual(1, result["invalid_jobs"])
        self.assertEqual(job.name, result["invalid_details"][0]["job_id"])
        self.assertIn("artifact hash mismatch", result["invalid_details"][0]["error"])


class CorrelationAndEmulationTests(unittest.TestCase):
    def test_correlation_is_discovery_only(self):
        result = correlate(
            {"country": {"pp": 10, "stability": 0.5}},
            {"country": {"pp": 12, "stability": 0.5}},
            {"path_pp": 10, "noise": 7},
            {"path_pp": 12, "noise": 9},
        )
        exact = [row for row in result["candidates"] if row["score"] == 1.0]
        self.assertEqual(1, len(exact))
        self.assertEqual("$.country.pp", exact[0]["save_path"])
        self.assertFalse(result["promotion_allowed"])

    def test_unicorn_blob_obeys_ms_x64_argument_registers(self):
        # mov rax,rcx ; add rax,rdx ; ret
        result = emulate_x64_blob(bytes.fromhex("4889c84801d0c3"), {"rcx": 7, "rdx": 5})
        self.assertEqual("0xc", result["registers"]["rax"])
        self.assertTrue(result["returned"])


class OfflineAnalysisTests(unittest.TestCase):
    def test_candidate_census_is_compact_and_accounts_for_pdata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            identity = inspect_pe(Path(sys.executable), include_pdata=True)
            self.assertGreater(identity["pdata_function_count"], 0)
            first_rva = identity["pdata_rvas"][0]
            index = root / "function_index.sqlite3"
            with closing(sqlite3.connect(index)) as connection:
                connection.execute("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
                connection.execute("CREATE TABLE functions(address INTEGER PRIMARY KEY)")
                connection.executemany(
                    "INSERT INTO meta(key,value) VALUES(?,?)",
                    [
                        ("schema_version", "3"),
                        ("generation_complete", "1"),
                        ("full_store_scan_complete", "1"),
                        ("generation_id", "fixture-generation"),
                        ("executable_sha256", identity["sha256"]),
                        ("image_base", identity["image_base"]),
                        ("store_root", str(root / "store")),
                    ],
                )
                connection.execute(
                    "INSERT INTO functions(address) VALUES(?)",
                    (int(identity["image_base"], 0) + first_rva,),
                )
                connection.commit()
            output = root / "candidate_census.json"
            result = candidate_census(
                Path(sys.executable), index, output, root / "store"
            )
            self.assertEqual(1, result["pdata_index_overlap"])
            self.assertEqual(
                result["pdata_function_count"] - 1, result["pdata_not_indexed_count"]
            )
            self.assertTrue((root / result["missing_candidate_rvas_artifact"]).is_file())
            self.assertNotIn("pdata_not_indexed", json.loads(output.read_text(encoding="utf-8")))
            self.assertEqual("fixture-generation", result["function_index_generation"])

    def test_candidate_census_refuses_unpublished_or_wrong_identity_index(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            index = root / "function_index.sqlite3"
            with closing(sqlite3.connect(index)) as connection:
                connection.execute("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
                connection.execute("CREATE TABLE functions(address INTEGER PRIMARY KEY)")
                connection.executemany(
                    "INSERT INTO meta(key,value) VALUES(?,?)",
                    [
                        ("schema_version", "3"),
                        ("generation_complete", "0"),
                        ("full_store_scan_complete", "0"),
                        ("generation_id", "not-published"),
                        ("executable_sha256", "0" * 64),
                        ("image_base", "0x140000000"),
                    ],
                )
                connection.commit()
            with self.assertRaisesRegex(ValueError, "unpublished or has the wrong"):
                candidate_census(Path(sys.executable), index, root / "census.json")

    def test_function_index_identity_and_addresses_share_one_read_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            identity = inspect_pe(Path(sys.executable), include_pdata=True)
            image_base = int(identity["image_base"], 0)
            old_address = image_base + identity["pdata_rvas"][0]
            new_address = image_base + identity["pdata_rvas"][1]
            index = root / "function_index.sqlite3"
            with closing(sqlite3.connect(index)) as connection:
                connection.execute("PRAGMA journal_mode=WAL")
                connection.execute("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
                connection.execute("CREATE TABLE functions(address INTEGER PRIMARY KEY)")
                connection.executemany(
                    "INSERT INTO meta(key,value) VALUES(?,?)",
                    [
                        ("schema_version", "3"), ("generation_complete", "1"),
                        ("full_store_scan_complete", "1"), ("generation_id", "old"),
                        ("executable_sha256", identity["sha256"]),
                        ("image_base", identity["image_base"]),
                        ("store_root", str(root / "store")),
                    ],
                )
                connection.execute("INSERT INTO functions(address) VALUES(?)", (old_address,))
                connection.commit()

            def publish_new_generation():
                with closing(sqlite3.connect(index, timeout=10.0)) as connection:
                    connection.execute("UPDATE meta SET value='new' WHERE key='generation_id'")
                    connection.execute("DELETE FROM functions")
                    connection.execute("INSERT INTO functions(address) VALUES(?)", (new_address,))
                    connection.commit()

            meta, addresses = _read_function_index_snapshot(
                index, identity["sha256"], identity["image_base"], root / "store",
                publish_new_generation,
            )
            self.assertEqual("old", meta["generation_id"])
            self.assertEqual({old_address}, addresses)

    def test_server_exposes_new_offline_analysis_operations(self):
        operation_schema = next(tool for tool in server.TOOLS if tool["name"] == "re_run")
        values = operation_schema["inputSchema"]["properties"]["operation"]["enum"]
        self.assertIn("candidate_census", values)
        self.assertIn("cross_analyze_function", values)

    def test_offline_slot_pool_caps_parallel_agent_processes(self):
        with tempfile.TemporaryDirectory() as temporary:
            context = multiprocessing.get_context("spawn")
            queue = context.Queue()
            processes = [
                context.Process(target=hold_offline_slot, args=(temporary, 2, queue))
                for _ in range(4)
            ]
            for process in processes:
                process.start()
            events = [queue.get(timeout=15) for _ in range(8)]
            for process in processes:
                process.join(timeout=15)
                self.assertEqual(0, process.exitcode)
            active = maximum = 0
            for kind, _stamp, _slot in sorted(events, key=lambda row: row[1]):
                active += 1 if kind == "start" else -1
                maximum = max(maximum, active)
            self.assertEqual(2, maximum)
            self.assertEqual(0, active)

    def test_timed_out_engine_preserves_diagnostics(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "engine.json"
            result = _run_command(
                [sys.executable, "-c", "import time; time.sleep(2)"], output, 0.1
            )
            self.assertEqual("timed_out", result["status"])
            self.assertTrue(output.is_file())
            self.assertIn(
                "TimeoutExpired", output.with_suffix(".json.stderr.log").read_text(encoding="utf-8")
            )

    def test_failed_engine_preserves_stderr(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "engine.json"
            result = _run_command([
                sys.executable, "-c",
                "import sys; sys.stderr.write('engine diagnostic\\n'); raise SystemExit(7)",
            ], output, 10)
            self.assertEqual("failed", result["status"])
            self.assertEqual(7, result["exit_code"])
            self.assertIn(
                "engine diagnostic", output.with_suffix(".json.stderr.log").read_text(encoding="utf-8")
            )

    def test_rizin_only_analysis_refuses_empty_recovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            exe = root / "hoi4.exe"
            exe.write_bytes(b"immutable fixture")
            digest = hashlib.sha256(exe.read_bytes()).hexdigest()
            job = root / "job"
            job.mkdir()

            def empty_json(_command, output, _timeout):
                output.write_text("[]\n", encoding="utf-8")
                return {
                    "status": "complete", "exit_code": 0, "output": output.name,
                    "bytes": output.stat().st_size, "stderr": None,
                }

            with patch("offline_analysis.MACHINE_LOCK_ROOT", root / "locks"):
                with patch("offline_analysis._run_command", side_effect=empty_json):
                    with self.assertRaisesRegex(RuntimeError, "escaped the requested byte range"):
                        cross_analyze(
                            exe, 0x140001000, job, SimpleNamespace(rizin_path=Path("rizin")),
                            ["rizin"], 10, 64, digest,
                        )


class BrokerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        workspace = root / "workspace"
        workspace.mkdir()
        output = root / "ghidra-output"
        output.mkdir()
        install = root / "ghidra"
        (install / "support").mkdir(parents=True)
        (install / "support" / "analyzeHeadless.bat").write_text("", encoding="ascii")
        scripts = root / "scripts"
        scripts.mkdir()
        (scripts / "BuildInventory.java").write_text("", encoding="ascii")
        self.broker = Broker(Config(workspace, root / "state", output, install, scripts, 2))

    def tearDown(self):
        self.temp.cleanup()

    def test_durable_claim_job_and_safe_artifact_read(self):
        result = self.broker.run("record_claim", {
            "subject": "sym_test", "predicate": "role", "value": "test",
            "status": "proposed", "evidence_job_ids": [],
        })
        artifact = self.broker.read_artifact(result["job_id"], "result.json")
        self.assertIn("claim_id", artifact["text"])
        with self.assertRaises(ValueError):
            self.broker.read_artifact(result["job_id"], "../registry.sqlite3")

    def test_mcp_dispatch_returns_validation_as_tool_error(self):
        result = server.dispatch(self.broker, "tools/call", {
            "name": "re_run", "arguments": {"operation": "not-real", "inputs": {}}
        })
        self.assertTrue(result["isError"])

    def test_content_addressed_snapshot_survives_source_replacement(self):
        source = Path(self.temp.name) / "hoi4.exe"
        source.write_bytes(b"exact-build")
        digest = hashlib.sha256(b"exact-build").hexdigest()
        snapshot = self.broker._materialize_snapshot(source, digest)
        source.write_bytes(b"steam-update")
        self.assertEqual(snapshot, self.broker._materialize_snapshot(source, digest))
        self.assertEqual(b"exact-build", snapshot.read_bytes())

    def test_failed_engine_job_indexes_partial_diagnostics(self):
        source = Path(self.temp.name) / "hoi4.exe"
        source.write_bytes(Path(sys.executable).read_bytes())
        registration = self.broker.run("register_build", {"exe_path": str(source)})
        build_id = registration["summary"]["build_id"]
        self.broker.config = replace(self.broker.config, angr_python=Path(sys.executable))
        with patch("offline_analysis.MACHINE_LOCK_ROOT", Path(self.temp.name) / "machine-locks"):
            with self.assertRaisesRegex(RuntimeError, "angr failed"):
                self.broker.run("cross_analyze_function", {
                    "build_id": build_id, "rva": "0x1000", "engines": ["angr"],
                    "timeout_s": 10, "max_bytes": 64,
                })
        failed = self.broker.list_jobs(1)["rows"][0]
        self.assertEqual("failed", failed["status"])
        with self.broker.registry.connect() as connection:
            paths = {
                row[0] for row in connection.execute(
                    "SELECT path FROM artifacts WHERE job_id=?", (failed["job_id"],)
                )
            }
        self.assertIn("cross_analysis.partial.json", paths)
        self.assertIn("failure.traceback.log", paths)

    def test_registration_does_not_mutate_registry_before_snapshot_succeeds(self):
        source = Path(self.temp.name) / "hoi4.exe"
        source.write_bytes(Path(sys.executable).read_bytes())
        with patch.object(
            self.broker, "_materialize_snapshot", side_effect=RuntimeError("snapshot failed")
        ):
            with self.assertRaisesRegex(RuntimeError, "snapshot failed"):
                self.broker.run("register_build", {"exe_path": str(source)})
        self.assertEqual([], self.broker.query("builds", {"limit": 10})["rows"])

    def test_registration_derives_identity_from_verified_snapshot(self):
        source = Path(self.temp.name) / "hoi4.exe"
        source.write_bytes(Path(sys.executable).read_bytes())
        snapshot = Path(self.temp.name) / "verified" / "hoi4.exe"
        snapshot.parent.mkdir()
        snapshot.write_bytes(source.read_bytes())
        with patch.object(self.broker, "_materialize_snapshot", return_value=snapshot):
            with patch("operations._pe_identity", return_value=("snapshot identity", "0x400000")) as identity:
                result = self.broker.run("register_build", {"exe_path": str(source)})
        identity.assert_called_once_with(snapshot)
        self.assertEqual("snapshot identity", result["summary"]["binary_id"])

    def test_status_refuses_malformed_function_index(self):
        index = Path(self.temp.name) / "malformed-index.sqlite3"
        with closing(sqlite3.connect(index)) as connection:
            connection.execute("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        self.broker.config = replace(self.broker.config, function_index_path=index)
        with patch("operations.runtime_status", return_value={"running": False}):
            status = self.broker.status()
        self.assertFalse(status["checks"]["function_index"])
        self.assertFalse(status["function_index"]["available"])
        self.assertFalse(status["ready"])

    def test_cross_analysis_window_must_fit_one_executable_section(self):
        source = Path(self.temp.name) / "hoi4.exe"
        source.write_bytes(Path(sys.executable).read_bytes())
        registration = self.broker.run("register_build", {"exe_path": str(source)})
        build_id = registration["summary"]["build_id"]
        identity = inspect_pe(Path(sys.executable))
        last = max(identity["executable_ranges"], key=lambda row: row["rva_end"])
        with self.assertRaisesRegex(ValueError, "not inside one executable section"):
            self.broker.run("cross_analyze_function", {
                "build_id": build_id, "rva": last["rva_end"] - 8,
                "engines": ["rizin"], "timeout_s": 10, "max_bytes": 16,
            })


if __name__ == "__main__":
    unittest.main()
