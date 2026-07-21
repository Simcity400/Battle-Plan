import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import server
from correlation import correlate
from diffing import diff_builds
from emulation import emulate_x64_blob
from evidence import ingest_ghidra_jobs, parse_instruction_artifact
from operations import Broker, Config
from registry import Registry


SHA_A = "a" * 64
SHA_B = "b" * 64
IMAGE_BASE = 0x140000000


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


if __name__ == "__main__":
    unittest.main()
