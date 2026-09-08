"""Focused identity, range, and custody refusals. No Ghidra launch in unit tests."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest
import os
import subprocess
import uuid
import external_module_evidence as e

class IdentityTests(unittest.TestCase):
    def setUp(self):
        self.expected=copy.deepcopy(e.MODULES["kernel32"])
        self.source=Path("C:/Windows/System32/kernel32.dll")
    def test_exact_current_module(self):
        self.assertEqual(e.identity(self.source,self.expected).FILE_HEADER.Machine,0x8664)
    def test_missing_module_refused(self):
        with self.assertRaisesRegex(e.Refusal,"missing module"):
            e.identity(self.source.parent/"__absent_external_evidence__.dll",self.expected)
    def test_wrong_hash_refused(self):
        self.expected["sha256"]="0"*64
        with self.assertRaisesRegex(e.Refusal,"SHA-256 mismatch"):
            e.identity(self.source,self.expected)
    def test_wrong_base_refused(self):
        self.expected["image_base"]+=0x10000
        with self.assertRaisesRegex(e.Refusal,"image base mismatch"):
            e.identity(self.source,self.expected)
    def test_wrong_name_refused(self):
        self.expected["name"]="KernelBase.dll"
        with self.assertRaisesRegex(e.Refusal,"name mismatch"):
            e.identity(self.source,self.expected)
    def test_loaded_header_mismatch_refused(self):
        pe=e.identity(self.source,self.expected)
        header=bytearray(self.source.read_bytes()[:4096])
        e.loaded_header_matches(header,pe)
        offset=int.from_bytes(header[0x3c:0x40],"little")+8
        header[offset]^=1
        with self.assertRaisesRegex(e.Refusal,"loaded header/disk"):
            e.loaded_header_matches(header,pe)
    def test_changed_snapshot_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            snapshot=Path(tmp)/self.expected["name"]
            shutil.copyfile(self.source,snapshot)
            with snapshot.open("r+b") as f:
                f.seek(0x100);old=f.read(1);f.seek(0x100);f.write(bytes([old[0]^1]))
            with self.assertRaisesRegex(e.Refusal,"SHA-256 mismatch"):
                e.identity(snapshot,self.expected)

class CustodyTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.job=Path(self.temp.name)/"job";self.job.mkdir()
        self.expected=e.MODULES["kernel32"]
        self.snapshot=Path("C:/Windows/System32/kernel32.dll")
        self.pe=e.identity(self.snapshot,self.expected)
        self.rva=0x414f0
        self.request={"schema":"external-windows-request-v1","identity":self.expected,
                      "snapshot":str(self.snapshot),"output":str(self.job/"evidence"),
                      "depth":0,"mode":"data","targets":[{"rva":self.rva,"size":16}]}
        e.write_json(self.job/"request.json",self.request)
        self.output=self.job/"evidence";self.output.mkdir()
        self.data=self.output/f"data_{self.rva:x}.bin"
        self.data.write_bytes(e.raw_bytes(self.pe,self.rva,16))
        self.manifest={"schema":"external-windows-evidence-v1","request_sha256":e.digest(self.job/"request.json"),
                       "identity":{**self.expected,"original_import_sha256":self.expected["sha256"],
                                   "modified_import_sha256":self.expected["sha256"]},
                       "mode":"data","depth":0,"targets":self.request["targets"],"functions":[],
                       "artifacts":[{"path":self.data.name,"bytes":16,"sha256":e.digest(self.data)}]}
        self.save_manifest()
    def save_manifest(self):
        (self.output/"manifest.json").write_text(json.dumps(self.manifest))
    def test_exact_bounded_data_bundle(self):
        self.assertEqual(e.verify_bundle(self.job,self.expected)["file_bytes_compared"],16)
    def test_no_complete_manifest_refused(self):
        (self.output/"manifest.json").unlink()
        with self.assertRaisesRegex(e.Refusal,"missing complete"):
            e.verify_bundle(self.job,self.expected)
    def test_missing_artifact_refused(self):
        self.data.unlink()
        with self.assertRaisesRegex(e.Refusal,"unmanifested output"):
            e.verify_bundle(self.job,self.expected)
    def test_modified_artifact_even_rehashed_refused(self):
        self.data.write_bytes(b"\0"*16)
        self.manifest["artifacts"][0]["sha256"]=e.digest(self.data)
        self.save_manifest()
        with self.assertRaisesRegex(e.Refusal,"differ from pinned PE"):
            e.verify_bundle(self.job,self.expected)
    def test_extra_caller_artifact_refused(self):
        (self.output/"func_other.decompile.c").write_text("unexpected caller")
        with self.assertRaisesRegex(e.Refusal,"unmanifested output"):
            e.verify_bundle(self.job,self.expected)
    def test_wrong_manifest_identity_refused(self):
        self.manifest["identity"]["sha256"]="f"*64;self.save_manifest()
        with self.assertRaisesRegex(e.Refusal,"identity mismatch"):
            e.verify_bundle(self.job,self.expected)
    def test_depth_expansion_refused(self):
        self.manifest["depth"]=1;self.save_manifest()
        with self.assertRaisesRegex(e.Refusal,"target expansion"):
            e.verify_bundle(self.job,self.expected)
    def test_escaped_artifact_refused(self):
        self.manifest["artifacts"][0]["path"]="../outside.bin";self.save_manifest()
        with self.assertRaisesRegex(e.Refusal,"missing or extra"):
            e.verify_bundle(self.job,self.expected)
    def test_non_file_backed_data_refused(self):
        with self.assertRaisesRegex(e.Refusal,"file-backed"):
            e.raw_bytes(self.pe,self.expected["size_of_image"]-1,16)
    def test_duplicate_and_oversize_requests_refused(self):
        for rvas,size in [([1,1],16),([1],513),([self.expected["size_of_image"]-1],16)]:
            with self.subTest(rvas=rvas,size=size),self.assertRaises(e.Refusal):
                e.targets_for("data",rvas,size,self.expected)
    def test_existing_root_and_busy_lease_refused(self):
        root=Path(self.temp.name)/"owned"
        e.ensure_root(root)
        with e.lease(root),self.assertRaisesRegex(e.Refusal,"job lease"):
            with e.lease(root):
                self.fail("second lease entered")
        unowned=Path(self.temp.name)/"unowned";unowned.mkdir()
        (unowned/"keep").write_text("keep")
        with self.assertRaisesRegex(e.Refusal,"nonempty unowned"):
            e.ensure_root(unowned)
        self.assertEqual((unowned/"keep").read_text(),"keep")
    def test_repository_root_refused_before_write(self):
        repo=Path(self.temp.name)/"repo";repo.mkdir();(repo/".git").write_text("gitdir: unused")
        candidate=repo/"scratch"
        with self.assertRaisesRegex(e.Refusal,"inside a repository"):
            e.ensure_root(candidate)
        self.assertFalse(candidate.exists())

@unittest.skipUnless(os.environ.get("EXTERNAL_DLL_HEADLESS_ROOT"),"set EXTERNAL_DLL_HEADLESS_ROOT after prepare")
class HeadlessIdentityTests(unittest.TestCase):
    def reject(self, kind, reason):
        root=Path(os.environ["EXTERNAL_DLL_HEADLESS_ROOT"]).resolve()
        expected=e.MODULES["kernel32"]
        prepared=[p for p in (root/"jobs").iterdir() if "kernel32-prepare" in p.name and (p/"verification.json").is_file()]
        self.assertEqual(len(prepared),1)
        base=prepared[0]
        e.verify_job(base,expected)
        job=root/"jobs"/("negative-"+kind+"-"+uuid.uuid4().hex[:8]);job.mkdir()
        shutil.copytree(base/"scripts",job/"scripts")
        request=json.loads((base/"request.json").read_text())
        request["output"]=str(job/"evidence")
        if kind=="hash": request["identity"]["sha256"]="0"*64
        elif kind=="name": request["identity"]["name"]="wrong.dll"
        elif kind=="base": request["identity"]["image_base"]+=0x10000
        elif kind=="missing": request["snapshot"]=str(root/"snapshots"/"not-present.dll")
        e.write_json(job/"request.json",request)
        command=json.loads((base/"command.json").read_text())
        argv=command["argv"]
        stop=argv.index("-import")
        argv=argv[:stop]+["-process",expected["name"],"-noanalysis","-readOnly","-max-cpu","2",
            "-scriptPath",str(job/"scripts"),"-postScript","ExternalModuleEvidence.java",str(job/"request.json"),
            "-log",str(job/"ghidra.log"),"-scriptlog",str(job/"script.log")]
        env=os.environ.copy();env.update(command["environment_overrides"])
        e.write_json(job/"command.json",{"argv":argv,"script":e.record(job/"scripts/ExternalModuleEvidence.java")})
        with e.lease(root), (job/"console.log").open("wb") as console:
            result=subprocess.run(argv,stdin=subprocess.DEVNULL,stdout=console,stderr=subprocess.STDOUT,
                cwd=job,env=env,timeout=180,creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0))
        log=(job/"console.log").read_text(errors="replace")
        self.assertIn(reason,log)
        self.assertFalse((job/"evidence").exists(),"negative guard emitted output")
        e.write_json(job/"negative_result.json",{"case":kind,"expected_reason":reason,"reason_seen":True,
            "evidence_directory_absent":True,"headless_exit":result.returncode,"console":e.record(job/"console.log")})
    def test_headless_wrong_hash(self): self.reject("hash","snapshot hash mismatch")
    def test_headless_wrong_name(self): self.reject("name","program name mismatch")
    def test_headless_wrong_base(self): self.reject("base","program image base mismatch")
    def test_headless_missing_snapshot(self): self.reject("missing","not-present.dll")

if __name__=="__main__":
    unittest.main()
