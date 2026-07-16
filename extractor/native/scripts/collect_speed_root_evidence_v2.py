#!/usr/bin/env python3
"""Read-only D57 transcript collector. It never changes game state or emits a verdict."""

from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
import re
import secrets
import struct
import sys
import tempfile
import time
from pathlib import Path

SCHEMA="battle-plan.speed_root_evidence_transcript.v2"
EXE_SHA="870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5C41C65AD".lower()
ROOTS={"cgame_state_singleton_rva":"0x3316e90","clock_singleton_a_rva":"0x33172e0","clock_singleton_b_rva":"0x33172e8","clock_vtable_rva":"0x2952510","tick_progress_offset":"0x4b8","speed_index_offset":"0x4bc","pause_offset":"0x6c1"}
HEX64=re.compile(r"^[0-9a-f]{64}$")
PROCESS_QUERY_INFORMATION=0x0400; PROCESS_VM_READ=0x0010; LIST_MODULES_ALL=0x03


class ProbeError(RuntimeError): pass


def sha256_file(path:Path)->str:
    digest=hashlib.sha256()
    with path.open("rb") as handle:
        while block:=handle.read(1024*1024):digest.update(block)
    return digest.hexdigest()


def canonical_bytes(value)->bytes:return json.dumps(value,sort_keys=True,separators=(",",":"),ensure_ascii=False,allow_nan=False).encode("utf-8")
def transcript_digest(value:dict)->str:
    clone=dict(value);clone.pop("transcript_sha256",None);return hashlib.sha256(canonical_bytes(clone)).hexdigest()
def now_filetime()->int:return 116444736000000000+time.time_ns()//100
def file_identity(path:Path)->dict:
    stat=path.stat();return {"volume_serial_number":int(stat.st_dev),"file_index":int(stat.st_ino),"file_size":int(stat.st_size)}


class FILETIME(ctypes.Structure):_fields_=[("low",wintypes.DWORD),("high",wintypes.DWORD)]
def filetime_int(value:FILETIME)->int:return (int(value.high)<<32)|int(value.low)


class ReadOnlyProcess:
    def __init__(self,pid:int):
        if os.name!="nt":raise ProbeError("live collection is Windows-only")
        self.k32=ctypes.WinDLL("kernel32",use_last_error=True);self.psapi=ctypes.WinDLL("psapi",use_last_error=True)
        self.k32.OpenProcess.restype=wintypes.HANDLE
        self.k32.ReadProcessMemory.argtypes=[wintypes.HANDLE,ctypes.c_void_p,ctypes.c_void_p,ctypes.c_size_t,ctypes.POINTER(ctypes.c_size_t)]
        self.k32.ReadProcessMemory.restype=wintypes.BOOL
        self.handle=self.k32.OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ,False,pid)
        if not self.handle:raise ProbeError(f"OpenProcess read-only failed: {ctypes.get_last_error()}")
        self.pid=pid
    def close(self):
        if self.handle:self.k32.CloseHandle(self.handle);self.handle=None
    def __enter__(self):return self
    def __exit__(self,*_):self.close()
    def exe_path(self)->Path:
        size=wintypes.DWORD(32768);buffer=ctypes.create_unicode_buffer(size.value)
        if not self.k32.QueryFullProcessImageNameW(self.handle,0,buffer,ctypes.byref(size)):raise ProbeError("QueryFullProcessImageNameW failed")
        return Path(buffer.value).resolve()
    def creation_filetime(self)->int:
        created=FILETIME();exited=FILETIME();kernel=FILETIME();user=FILETIME()
        if not self.k32.GetProcessTimes(self.handle,ctypes.byref(created),ctypes.byref(exited),ctypes.byref(kernel),ctypes.byref(user)):raise ProbeError("GetProcessTimes failed")
        return filetime_int(created)
    def module_base(self)->int:
        modules=(wintypes.HMODULE*1)();needed=wintypes.DWORD()
        if not self.psapi.EnumProcessModulesEx(self.handle,modules,ctypes.sizeof(modules),ctypes.byref(needed),LIST_MODULES_ALL):raise ProbeError("EnumProcessModulesEx failed")
        return int(ctypes.cast(modules[0],ctypes.c_void_p).value)
    def read(self,address:int,size:int)->bytes:
        buffer=(ctypes.c_ubyte*size)();read=ctypes.c_size_t()
        if not self.k32.ReadProcessMemory(self.handle,ctypes.c_void_p(address),buffer,size,ctypes.byref(read)) or read.value!=size:raise ProbeError(f"ReadProcessMemory failed at 0x{address:x}")
        return bytes(buffer)
    def u64(self,address:int)->int:return struct.unpack("<Q",self.read(address,8))[0]
    def i32(self,address:int)->int:return struct.unpack("<i",self.read(address,4))[0]
    def u8(self,address:int)->int:return self.read(address,1)[0]
    def u32(self,address:int)->int:return struct.unpack("<I",self.read(address,4))[0]


def atomic_json(path:Path,value)->None:
    path.parent.mkdir(parents=True,exist_ok=True);temp=path.with_name(path.name+f".{secrets.token_hex(8)}.tmp")
    data=(json.dumps(value,indent=2,sort_keys=True)+"\n").encode();
    with temp.open("xb") as handle:handle.write(data);handle.flush();os.fsync(handle.fileno())
    os.replace(temp,path)


def exclusive_json(path:Path,value)->None:
    path.parent.mkdir(parents=True,exist_ok=True);temp=path.with_name(path.name+f".{secrets.token_hex(8)}.tmp")
    data=(json.dumps(value,indent=2,sort_keys=True)+"\n").encode()
    try:
        with temp.open("xb") as handle:handle.write(data);handle.flush();os.fsync(handle.fileno())
        os.link(temp,path)
    finally:
        try:temp.unlink()
        except FileNotFoundError:pass


def issued_path(root:Path,nonce:str)->Path:return root/f"{nonce}.issued.json"
def consume_nonce(root:Path,nonce:str,state:str,session_id:str,transcript_path:Path,transcript_sha256:str,created:int,consumed:int|None=None)->None:
    source=issued_path(root,nonce);marker=source.with_suffix(source.suffix+".consumed");destination=root/f"{nonce}.{state}.json"
    value={"schema":"battle-plan.speed_root_nonce_seal.v2","state":state,"session_id":session_id,"evidence_nonce_hex":nonce,"transcript_path":str(transcript_path.resolve()),"transcript_sha256":transcript_sha256,"created_filetime_utc":created,"consumed_filetime_utc":now_filetime() if consumed is None else consumed}
    conflicting=[root/f"{nonce}.{candidate}.json" for candidate in ("consumed_success","consumed_failure") if candidate!=state]
    if any(path.exists() for path in conflicting):raise ProbeError("nonce already has a conflicting terminal outcome")
    if destination.exists():
        if json.loads(destination.read_text(encoding="utf-8-sig"))!=value:raise ProbeError("nonce outcome already exists with different binding")
    else:exclusive_json(destination,value)
    if source.exists():
        if marker.exists():raise ProbeError("nonce has both issued and consumed markers")
        os.rename(source,marker)
    elif not marker.is_file():raise ProbeError("nonce is absent or has no durable consumed marker")


def load_transcript(path:Path)->dict:
    value=json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value,dict) or value.get("schema")!=SCHEMA:raise ProbeError("unsupported transcript")
    if value.get("transcript_sha256"):raise ProbeError("sealed transcript is immutable")
    return value


def process_binding(process:ReadOnlyProcess)->dict:
    exe=process.exe_path()
    return {"pid":process.pid,"creation_filetime_utc":process.creation_filetime(),"exe_path":str(exe),"exe_sha256":sha256_file(exe),"exe_file_id":file_identity(exe),"module_base":f"0x{process.module_base():x}"}


def assert_same_process(process:ReadOnlyProcess,transcript:dict)->tuple[int,dict]:
    actual=process_binding(process)
    if actual!=transcript["process"]:raise ProbeError("PID/process creation/executable/module binding changed")
    if sha256_file(Path(__file__).resolve())!=transcript["tool"]["probe_sha256"]:raise ProbeError("probe tool bytes changed after initialization")
    return int(actual["module_base"],16),actual


def collect_sample(process:ReadOnlyProcess,base:int,sequence:int,ui:int,phase:str,read_count:int,delay_ms:int)->dict:
    gs_slot=base+0x3316E90;clock_a_slot=base+0x33172E0;clock_b_slot=base+0x33172E8;expected_vtable=base+0x2952510
    gs=process.u64(gs_slot);clock_a=process.u64(clock_a_slot);clock_b=process.u64(clock_b_slot)
    if gs<0x10000 or clock_a<0x10000 or clock_a!=clock_b or process.u64(clock_a)!=expected_vtable:raise ProbeError("root/sentinel validation failed")
    addresses={"cgame_state_slot":f"0x{gs_slot:x}","clock_slot_a":f"0x{clock_a_slot:x}","clock_slot_b":f"0x{clock_b_slot:x}","clock_vtable":f"0x{expected_vtable:x}","cgame_state":f"0x{gs:x}","clock":f"0x{clock_a:x}","speed":f"0x{gs+0x4BC:x}","pause":f"0x{clock_a+0x6C1:x}","tick":f"0x{gs+0x4B8:x}"}
    reads=[]
    for ordinal in range(1,read_count+1):
        current_gs=process.u64(gs_slot);current_a=process.u64(clock_a_slot);current_b=process.u64(clock_b_slot);vtable=process.u64(current_a)
        tick_bits=process.u32(current_gs+0x4B8)
        reads.append({"ordinal":ordinal,"observed_filetime_utc":now_filetime(),"cgame_state_value":f"0x{current_gs:x}","clock_slot_a_value":f"0x{current_a:x}","clock_slot_b_value":f"0x{current_b:x}","clock_vtable_value":f"0x{vtable:x}","speed_raw":process.i32(current_gs+0x4BC),"pause_raw":process.u8(current_a+0x6C1),"tick_progress_bits":f"0x{tick_bits:08x}"})
        if ordinal<read_count:time.sleep(delay_ms/1000.0)
    return {"sequence":sequence,"action_nonce_hex":secrets.token_hex(32),"requested_ui_speed":ui,"expected_raw_speed":ui-1,"phase":phase,"addresses":addresses,"reads":reads}


def command_init(args)->None:
    args._nonce_issued_by_this_invocation=False
    if args.action_token!="AUTHORIZE_READ_ONLY_SPEED_ROOT_EVIDENCE_INIT":raise ProbeError("explicit initialization action token missing")
    if not HEX64.fullmatch(args.evidence_nonce):raise ProbeError("evidence nonce must be a caller-issued lowercase 64-hex value")
    if not re.fullmatch(r"[A-Za-z0-9_.-]{8,128}",args.session_id):raise ProbeError("session id is invalid")
    if args.out.exists():raise ProbeError("refusing to reuse transcript path")
    ledger=args.nonce_ledger_root.resolve();ledger.mkdir(parents=True,exist_ok=True);created=now_filetime()
    issue=issued_path(ledger,args.evidence_nonce)
    history=[issue,issue.with_suffix(issue.suffix+".consumed"),ledger/f"{args.evidence_nonce}.consumed_success.json",ledger/f"{args.evidence_nonce}.consumed_failure.json"]
    if any(path.exists() for path in history):raise ProbeError("evidence nonce has prior ledger history and cannot be reused")
    issued={"schema":"battle-plan.speed_root_nonce_issue.v2","state":"issued","session_id":args.session_id,"evidence_nonce_hex":args.evidence_nonce,"transcript_path":str(args.out.resolve()),"created_filetime_utc":created}
    exclusive_json(issue,issued);args._nonce_issued_by_this_invocation=True
    static=args.static_disasm.resolve();probe=Path(__file__).resolve()
    with ReadOnlyProcess(args.pid) as process:binding=process_binding(process)
    if binding["exe_sha256"]!=EXE_SHA or Path(binding["exe_path"]).name.lower()!="hoi4.exe":raise ProbeError("target is not the supported hoi4.exe build")
    transcript={"schema":SCHEMA,"capture":{"session_id":args.session_id,"evidence_nonce_hex":args.evidence_nonce,"nonce_ledger_root":str(ledger),"created_filetime_utc":created,"completed_filetime_utc":0},"process":binding,"tool":{"probe_path":str(probe),"probe_sha256":sha256_file(probe)},"static_disasm":{"artifact_path":str(static),"artifact_sha256":sha256_file(static)},"roots":ROOTS,"speed_samples":[],"pause_samples":[],"transcript_sha256":""}
    atomic_json(args.out,transcript);print(f"initialized observation transcript: {args.out}")


def command_speed(args)->None:
    transcript=load_transcript(args.transcript)
    expected=len(transcript["speed_samples"])+1
    if args.ui_speed!=expected or args.action_token!=f"SET_VISIBLE_UI_SPEED_{args.ui_speed}_AND_ACK":raise ProbeError(f"set visible UI speed {expected}, verify it, then use its exact action token")
    with ReadOnlyProcess(transcript["process"]["pid"]) as process:
        base,_=assert_same_process(process,transcript);sample=collect_sample(process,base,expected,args.ui_speed,"speed_cycle",args.read_count,args.read_delay_ms)
    transcript["speed_samples"].append(sample);atomic_json(args.transcript,transcript);print(f"recorded observation sequence {expected}; no verdict emitted")


def command_pause(args)->None:
    transcript=load_transcript(args.transcript)
    if len(transcript["speed_samples"])!=5:raise ProbeError("record the ordered UI speed 1..5 cycle first")
    phases=[("running_before",0,"SET_VISIBLE_UI_SPEED_3_UNPAUSED_AND_ACK"),("paused",1,"PAUSE_AT_VISIBLE_UI_SPEED_3_AND_ACK"),("running_after",0,"RESUME_AT_VISIBLE_UI_SPEED_3_AND_ACK")]
    index=len(transcript["pause_samples"])
    if index>=3:raise ProbeError("all pause phases are already recorded")
    phase,_pause,token=phases[index]
    if args.phase!=phase or args.action_token!=token:raise ProbeError(f"perform explicit action for {phase} and use token {token}")
    with ReadOnlyProcess(transcript["process"]["pid"]) as process:
        base,_=assert_same_process(process,transcript);sample=collect_sample(process,base,6+index,3,phase,args.read_count,args.read_delay_ms)
    transcript["pause_samples"].append(sample);atomic_json(args.transcript,transcript);print(f"recorded observation sequence {6+index}; no verdict emitted")


def command_seal(args)->None:
    transcript=load_transcript(args.transcript)
    if args.action_token!="SEAL_TRANSCRIPT_WITHOUT_VERDICT":raise ProbeError("explicit seal token missing")
    if len(transcript["speed_samples"])!=5 or len(transcript["pause_samples"])!=3:raise ProbeError("transcript is structurally incomplete")
    transcript["capture"]["completed_filetime_utc"]=now_filetime();transcript["transcript_sha256"]=transcript_digest(transcript)
    atomic_json(args.transcript,transcript)
    consume_nonce(Path(transcript["capture"]["nonce_ledger_root"]),transcript["capture"]["evidence_nonce_hex"],"consumed_success",transcript["capture"]["session_id"],args.transcript,transcript["transcript_sha256"],transcript["capture"]["created_filetime_utc"],transcript["capture"]["completed_filetime_utc"])
    print(f"sealed transcript bytes only: {args.transcript}; run the independent verifier for a verdict")


def command_self_test(_args)->None:
    with tempfile.TemporaryDirectory(prefix="speed_root_nonce_v2_") as temp:
        root=Path(temp);nonce="a"*64;transcript=root/"transcript.json";transcript.write_text("{}",encoding="utf-8")
        issue=issued_path(root,nonce);issued={"schema":"battle-plan.speed_root_nonce_issue.v2","state":"issued","session_id":"self_test","evidence_nonce_hex":nonce,"transcript_path":str(transcript.resolve()),"created_filetime_utc":100}
        exclusive_json(issue,issued);consume_nonce(root,nonce,"consumed_failure","self_test",transcript,"",100,200)
        marker=issue.with_suffix(issue.suffix+".consumed");outcome=root/f"{nonce}.consumed_failure.json"
        if issue.exists() or not marker.is_file() or not outcome.is_file():raise AssertionError("nonce was not durably consumed")
        if json.loads(marker.read_text(encoding="utf-8-sig"))!=issued:raise AssertionError("consumed issue marker changed")
        try:consume_nonce(root,nonce,"consumed_success","self_test",transcript,"f"*64,100,201)
        except ProbeError:pass
        else:raise AssertionError("conflicting nonce outcome was accepted")
        try:exclusive_json(outcome,{})
        except FileExistsError:pass
        else:raise AssertionError("exclusive outcome write overwrote history")
    print("read-only speed-root collector nonce self-test: passed=3 failed=0")


def main(argv=None)->int:
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest="command",required=True)
    sub.add_parser("self-test")
    init=sub.add_parser("init");init.add_argument("--pid",type=int,required=True);init.add_argument("--session-id",required=True);init.add_argument("--evidence-nonce",required=True);init.add_argument("--nonce-ledger-root",type=Path,required=True);init.add_argument("--static-disasm",type=Path,required=True);init.add_argument("--out",type=Path,required=True);init.add_argument("--action-token",required=True)
    speed=sub.add_parser("speed");speed.add_argument("--transcript",type=Path,required=True);speed.add_argument("--ui-speed",type=int,choices=range(1,6),required=True);speed.add_argument("--action-token",required=True)
    pause=sub.add_parser("pause");pause.add_argument("--transcript",type=Path,required=True);pause.add_argument("--phase",choices=("running_before","paused","running_after"),required=True);pause.add_argument("--action-token",required=True)
    seal=sub.add_parser("seal");seal.add_argument("--transcript",type=Path,required=True);seal.add_argument("--action-token",required=True)
    for action in (speed,pause):action.add_argument("--read-count",type=int,choices=range(3,17),default=3);action.add_argument("--read-delay-ms",type=int,choices=range(50,1001),default=500)
    args=parser.parse_args(argv)
    try:{"self-test":command_self_test,"init":command_init,"speed":command_speed,"pause":command_pause,"seal":command_seal}[args.command](args)
    except (Exception,KeyboardInterrupt) as exc:
        try:
            if args.command=="init" and getattr(args,"_nonce_issued_by_this_invocation",False) and HEX64.fullmatch(str(getattr(args,"evidence_nonce",""))):
                issue=issued_path(args.nonce_ledger_root.resolve(),args.evidence_nonce)
                if issue.is_file():
                    issued=json.loads(issue.read_text(encoding="utf-8-sig"))
                    consume_nonce(args.nonce_ledger_root.resolve(),args.evidence_nonce,"consumed_failure",args.session_id,args.out,"",issued["created_filetime_utc"])
            elif args.command in ("speed","pause","seal") and args.transcript.is_file():
                failed=json.loads(args.transcript.read_text(encoding="utf-8-sig"));capture=failed["capture"]
                consume_nonce(Path(capture["nonce_ledger_root"]),capture["evidence_nonce_hex"],"consumed_failure",capture["session_id"],args.transcript,failed.get("transcript_sha256",""),capture["created_filetime_utc"])
        except Exception:pass
        print(f"read-only speed-root collector: FAIL: {exc}",file=sys.stderr);return 1
    return 0


if __name__=="__main__":raise SystemExit(main())
