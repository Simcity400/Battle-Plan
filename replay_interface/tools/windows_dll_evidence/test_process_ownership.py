"""Harmless Windows child trees and the real external sentinel lease; no Ghidra."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import external_module_evidence as e


@unittest.skipUnless(os.name == "nt", "Windows owned-process contract")
class ProcessOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix="external-process-test-")
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        evidence=os.environ.get("EXTERNAL_DLL_PROCESS_TEST_EVIDENCE_ROOT")
        if evidence:
            destination=Path(evidence).resolve()/self._testMethodName
            repository=Path(__file__).resolve().parents[3]
            if destination.drive.lower()!="c:" or repository in destination.parents:
                raise RuntimeError("test evidence must be on C: outside the repository")
            self.addCleanup(lambda: shutil.copytree(self.root,destination))

    def exercise(self, name, *, tail="p.wait()", child_seconds=0.15,
                 timeout=5, fault=None, exception=None):
        job=self.root/name;job.mkdir()
        runner,records=e.process_runner(job)
        for item in records:
            self.assertEqual(item,e.record(Path(item["path"])))
        marker=job/"started.txt"
        child=f"import time; print('owned child',flush=True); time.sleep({child_seconds})"
        code=("import subprocess,sys,pathlib; "
              f"p=subprocess.Popen([sys.executable,'-B','-u','-c',{child!r}]); "
              f"pathlib.Path({str(marker)!r}).write_text(str(p.pid)); "
              "print('owned launcher',flush=True); "+tail)
        argv=[sys.executable,"-B","-u","-c",code]
        lock=self.root/".job.lock"
        owner=self
        observations=[]
        class ProbeJob(runner._Job):
            def accounting(self):
                state=super().accounting()
                owner.assertTrue(lock.is_file())
                with owner.assertRaisesRegex(e.Refusal,"job lease"):
                    with e.lease(owner.root):
                        owner.fail("overlapping external lease")
                observations.append(state)
                return state
        original_unlink=Path.unlink
        releases=[]
        def unlink(path,*args,**kwargs):
            if path==lock:
                owner.assertTrue(observations)
                owner.assertEqual(0,observations[-1]["active_processes"])
                releases.append(observations[-1])
            return original_unlink(path,*args,**kwargs)
        with patch.object(runner,"_Job",ProbeJob),patch.object(Path,"unlink",unlink):
            with e.lease(self.root):
                def run():
                    return runner.run_headless(argv,cwd=str(job),env=os.environ.copy(),
                        console_path=job/"console.log",receipt_path=job/"process.json",
                        timeout_seconds=timeout)
                def check():
                    if exception:
                        with self.assertRaises(exception): run()
                    else:
                        self.assertEqual(0,run().returncode)
                        e.verify_process(job,{"argv":argv})
                if fault:
                    with patch.object(ProbeJob,fault,side_effect=OSError("injected startup failure")):
                        check()
                else:
                    check()
        self.assertEqual(1,len(releases))
        self.assertFalse(lock.exists())
        receipt=json.loads((job/"process.json").read_text())
        self.assertEqual(0,receipt["job_after_drain"]["active_processes"])
        if fault:
            self.assertFalse(marker.exists(),"suspended launcher executed before startup refusal")
        else:
            self.assertTrue(marker.is_file())
            self.assertGreaterEqual(receipt["job_after_drain"]["total_processes"],2)
        e.write_json(job/"lease_observations.json",{"accounting_while_locked":observations,
                     "at_unlink":releases,"lease_absent_after_drain":True})
        return job

    def test_success_drains_before_external_lease_unlink(self):
        self.exercise("success")

    def test_surviving_child_on_zero_and_nonzero_exit_drains_before_unlink(self):
        for code in [0,7]:
            with self.subTest(returncode=code):
                self.exercise(str(code),tail=f"sys.exit({code})",child_seconds=60,
                              exception=RuntimeError)

    def test_timeout_drains_inherited_output_child_before_unlink(self):
        self.exercise("timeout",tail="p.wait()",child_seconds=60,timeout=0.8,
                      exception=subprocess.TimeoutExpired)

    def test_startup_failures_release_only_after_suspended_child_drain(self):
        for fault in ["creation_time","assign","resume"]:
            with self.subTest(fault=fault):
                self.exercise(fault,fault=fault,exception=OSError)

    def test_process_receipt_rejects_live_or_wrong_command(self):
        job=self.exercise("receipt")
        process=json.loads((job/"process.json").read_text())
        with self.assertRaisesRegex(e.Refusal,"complete and drain"):
            e.verify_process(job,{"argv":["wrong.exe"]})
        process["job_after_drain"]["active_processes"]=1
        tampered=self.root/"tampered";tampered.mkdir()
        e.write_json(tampered/"process.json",process)
        with self.assertRaisesRegex(e.Refusal,"complete and drain"):
            e.verify_process(tampered,{"argv":process["command"]})

    def test_execute_calls_owned_runner_inside_existing_lease(self):
        projects=self.root/"projects"/"kernel32";projects.mkdir(parents=True)
        (projects/"external_kernel32_0ab61f2e0d41.gpr").touch()
        e.write_json(self.root/".external-windows-evidence.json",
                     {"schema":"external-windows-root-v1","root":str(self.root.resolve())})
        original=e.process_runner
        identity=e.identity
        pe_handles=[]
        def inspect(path,expected):
            pe=identity(path,expected)
            pe_handles.append(pe)
            return pe
        calls=[]
        def load(job):
            runner,records=original(job)
            def refuse(argv,**kwargs):
                self.assertTrue((self.root/".job.lock").is_file())
                self.assertIn("-readOnly",argv)
                self.assertIn("-noanalysis",argv)
                self.assertNotIn("-import",argv)
                self.assertEqual(900,kwargs["timeout_seconds"])
                request=json.loads((job/"request.json").read_text())
                self.assertEqual([{"rva":0x409d0}],request["targets"])
                self.assertEqual(0,request["depth"])
                calls.append(argv)
                raise e.Refusal("injected before any native launch")
            runner.run_headless=refuse
            return runner,records
        try:
            with patch.object(e,"process_runner",load),patch.object(e,"identity",inspect):
                with self.assertRaisesRegex(e.Refusal,"before any native launch"):
                    e.execute(self.root,"kernel32","functions",[0x409d0])
        finally:
            for pe in pe_handles:
                pe.close()
        self.assertEqual(1,len(calls))
        self.assertFalse((self.root/".job.lock").exists())


if __name__=="__main__":
    unittest.main()
