"""Isolated, exact-identity external Windows DLL evidence. No the game registry access."""
from __future__ import annotations
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import uuid
import ctypes as C
from ctypes import wintypes as W
import struct
import pefile
import importlib.util

SOURCE = Path(__file__).resolve().parent
DEFAULT_ROOT = Path("C:/Users/Example/AppData/Local/Temp/project-closeout-20260907/ui-native-control-policy/external-dll-capability/isolated")
GHIDRA = Path("C:/Users/Example/Downloads/ghidra_12.1.2_PUBLIC_20260605/ghidra_12.1.2_PUBLIC")
JAVA = Path("C:/Program Files/Microsoft/jdk-21.0.11.10-hotspot/bin/java.exe")
MODULES = {
    "kernel32": {"name":"kernel32.dll","version":"10.0.26100.9168","bytes":836208,
                 "sha256":"0ab61f2e0d412a585233f1b308c120cba74f800c1030bffe0aabd20df8c6d907",
                 "image_base":0x180000000,"size_of_image":823296,"machine":0x8664},
    "kernelbase": {"name":"KernelBase.dll","version":"10.0.26100.9168","bytes":4199672,
                   "sha256":"9d26852459745322b704e2a49e448922839c584d06e208cf9abcd0d857c4b7cd",
                   "image_base":0x180000000,"size_of_image":4190208,"machine":0x8664},
}
ENTRY_JOINS = {"GetCPInfo":(0x414f0,0x8a110,0x4faa0),
               "MultiByteToWideChar":(0x303b0,0x8abe0,0x1dd20),
               "GetStringTypeW":(0x36f30,0x8abc8,0x89b90)}

class Refusal(RuntimeError):
    """A named evidence refusal, never an accepted partial result."""

def require(ok, why):
    if not ok:
        raise Refusal(why)

def digest(path):
    with Path(path).open("rb") as f:
        h=sha256()
        for block in iter(lambda:f.read(1024*1024),b""):
            h.update(block)
    return h.hexdigest()

def record(path):
    return {"path":str(path),"bytes":path.stat().st_size,"sha256":digest(path)}

def write_json(path, value):
    with path.open("x",encoding="utf-8",newline="\n") as f:
        json.dump(value,f,indent=2);f.write("\n")

def process_runner(job):
    """Load the retained copy of the shared owned-child implementation."""
    source=SOURCE.parent/"ghidra"/"headless_process.py"
    source_record=record(source)
    copied=job/source.name
    shutil.copyfile(source,copied)
    require(digest(copied)==source_record["sha256"],"process runner changed while copying")
    spec=importlib.util.spec_from_file_location("external_owned_headless",copied)
    runner=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    require(digest(copied)==source_record["sha256"],"process runner changed while loading")
    return runner,[source_record,record(copied)]

def verify_process(job, command):
    process=json.loads((job/"process.json").read_text())
    require(process["command"]==command["argv"] and process["state"]=="complete"
            and process["assigned_before_resume"] and process["launcher_returncode"]==0
            and process["job_after_drain"]["active_processes"]==0,
            "owned headless process did not complete and drain")

def identity(path, expected):
    require(path.is_file(),"missing module: "+str(path))
    require(path.name==expected["name"],"module name mismatch")
    require(path.stat().st_size==expected["bytes"],"module size mismatch")
    require(digest(path)==expected["sha256"],"module SHA-256 mismatch")
    pe=pefile.PE(str(path))
    require(pe.FILE_HEADER.Machine==expected["machine"] and pe.OPTIONAL_HEADER.Magic==0x20b,"module machine mismatch")
    require(pe.OPTIONAL_HEADER.ImageBase==expected["image_base"],"module image base mismatch")
    require(pe.OPTIONAL_HEADER.SizeOfImage==expected["size_of_image"],"module image extent mismatch")
    require(hasattr(pe,"VS_FIXEDFILEINFO"),"module version missing")
    f=pe.VS_FIXEDFILEINFO[0]
    version=".".join(map(str,[f.FileVersionMS>>16,f.FileVersionMS&65535,f.FileVersionLS>>16,f.FileVersionLS&65535]))
    require(version==expected["version"],"module version mismatch")
    require(digest(path)==expected["sha256"],"module changed during inspection")
    return pe

def within(root, path):
    root=root.resolve(); path=path.resolve()
    require(path!=root and path.is_relative_to(root),"path escapes owned root")
    return path

def ensure_root(root):
    root=root.resolve()
    require(root.drive.lower()=="c:","scratch must be on C:")
    require(not any(p.name==".git" or (p/".git").exists() for p in [root,*root.parents]),"scratch root is inside a repository")
    marker=root/".external-windows-evidence.json"
    if not marker.exists():
        require(not root.exists() or not any(root.iterdir()),"nonempty unowned root")
        root.mkdir(parents=True,exist_ok=True)
        write_json(marker,{"schema":"external-windows-root-v1","root":str(root)})
    saved=json.loads(marker.read_text())
    require(saved=={"schema":"external-windows-root-v1","root":str(root)},"root ownership mismatch")
    return root

@contextmanager
def lease(root):
    lock=root/".job.lock"
    token=uuid.uuid4().hex
    try:
        with lock.open("x",encoding="ascii") as f:
            f.write(token)
    except FileExistsError as e:
        raise Refusal("isolated root has an active/stale job lease") from e
    try:
        yield
    finally:
        require(lock.read_text()==token,"job lease ownership changed")
        lock.unlink()

def targets_for(mode, rvas, size, expected):
    require(mode in {"probe","functions","data"},"unknown export mode")
    require(len(rvas)==len(set(rvas)),"duplicate target")
    require((not rvas) if mode=="probe" else 0<len(rvas)<=(4 if mode=="data" else 3),"target count exceeded")
    require(mode!="data" or 0<size<=512,"data size exceeds bound")
    targets=[]
    for rva in rvas:
        require(0<=rva<expected["size_of_image"],"target outside image")
        row={"rva":rva}
        if mode=="data":
            require(rva+size<=expected["size_of_image"],"data outside image")
            row["size"]=size
        targets.append(row)
    return targets

def raw_bytes(pe, rva, size):
    require(size>0,"empty byte range")
    # Only actual file-backed bytes are admissible. No invented zero-fill.
    for section in pe.sections:
        start=section.VirtualAddress
        if start<=rva and rva+size<=start+section.SizeOfRawData:
            offset=section.PointerToRawData+rva-start
            data=pe.__data__[offset:offset+size]
            require(len(data)==size,"short file range")
            return data
    raise Refusal("requested bytes are not in one file-backed PE section")

def verify_bundle(job, expected):
    request_path=job/"request.json"
    request=json.loads(request_path.read_text())
    require(request["schema"]=="external-windows-request-v1","request schema mismatch")
    require(request["identity"]==expected and request["depth"]==0,"request identity/depth mismatch")
    mode=request["mode"]
    targets=request["targets"]
    require(targets==targets_for(mode,[t["rva"] for t in targets],targets[0].get("size",16) if targets else 16,expected),"invalid target request")
    snapshot=Path(request["snapshot"])
    pe=identity(snapshot,expected)
    output=job/"evidence"
    require(Path(request["output"]).resolve()==output.resolve(),"wrong output location")
    manifest_path=output/"manifest.json"
    require(manifest_path.is_file(),"missing complete manifest")
    manifest=json.loads(manifest_path.read_text())
    require(manifest["schema"]=="external-windows-evidence-v1","manifest schema mismatch")
    require(manifest["request_sha256"]==digest(request_path),"request hash mismatch")
    require(manifest["mode"]==mode and manifest["depth"]==0 and manifest["targets"]==targets,"manifest target expansion")
    got=manifest["identity"]
    require(all(got.get(k)==v for k,v in expected.items()),"manifest identity mismatch")
    require(got["original_import_sha256"]==expected["sha256"] and got["modified_import_sha256"]==expected["sha256"],"imported bytes mismatch")
    wanted=set()
    if mode=="functions":
        for t in targets:
            wanted.update(f"func_{t['rva']:x}."+ext for ext in ("decompile.c","instructions.txt","instructions.json"))
    elif mode=="data":
        wanted={f"data_{t['rva']:x}.bin" for t in targets}
    actual=[a["path"] for a in manifest["artifacts"]]
    require(len(actual)==len(set(actual)) and set(actual)==wanted,"missing or extra artifacts")
    require({p.name for p in output.iterdir()}==wanted|{"manifest.json"},"unmanifested output")
    for a in manifest["artifacts"]:
        require(Path(a["path"]).name==a["path"],"artifact path traversal")
        path=within(output,output/a["path"])
        require(path.stat().st_size==a["bytes"] and digest(path)==a["sha256"],"artifact byte/hash mismatch")
    functions=manifest["functions"]
    require([f["entry_rva"] for f in functions]==([t["rva"] for t in targets] if mode=="functions" else []),"function expansion")
    checked=0
    for f in functions:
        ranges=f["body_ranges"]
        require(0<sum(length for _,length in ranges)<=65536,"body extent bound")
        seen=set()
        instructions=json.loads((output/f"func_{f['entry_rva']:x}.instructions.json").read_text())
        require(0<len(instructions)<=8192,"instruction count bound")
        require(instructions[0]["rva"]==f["entry_rva"],"instruction entry mismatch")
        for ins in instructions:
            rva=ins["rva"]; data=bytes.fromhex(ins["bytes"])
            require(rva not in seen and 0<len(data)<=15,"invalid instruction")
            require(any(start<=rva and rva+len(data)<=start+length for start,length in ranges),"instruction outside body")
            require(raw_bytes(pe,rva,len(data))==data,"instruction bytes differ from pinned PE")
            seen.add(rva);checked+=len(data)
    if mode=="data":
        for t in targets:
            data=(output/f"data_{t['rva']:x}.bin").read_bytes()
            require(data==raw_bytes(pe,t["rva"],t["size"]),"data bytes differ from pinned PE")
            checked+=len(data)
    return {"schema":"external-windows-verification-v1","manifest":record(manifest_path),
            "identity":expected,"mode":mode,"targets":targets,"functions":len(functions),
            "artifacts":len(wanted),"file_bytes_compared":checked}

def snapshot_module(root, key):
    require(key in MODULES,"unknown module")
    expected=MODULES[key]
    source=Path("C:/Windows/System32")/expected["name"]
    identity(source,expected)
    dst=within(root,root/"snapshots"/expected["sha256"]/expected["name"])
    dst.parent.mkdir(parents=True,exist_ok=True)
    if not dst.exists():
        with dst.open("xb") as f:
            f.write(source.read_bytes())
    identity(dst,expected);identity(source,expected)
    return dst

def verify_job(job, expected):
    """Reverify a completed job, including its original command and acceptance receipt."""
    prior=json.loads((job/"verification.json").read_text())
    fresh=verify_bundle(job,expected)
    for key,value in fresh.items():
        require(prior.get(key)==value,"accepted receipt changed: "+key)
    for key,name in [("command","command.json"),("exit","exit.json"),("console","console.log"),
                     ("process","process.json")]:
        require(prior[key]==record(job/name),"job custody changed: "+key)
    command=json.loads((job/"command.json").read_text())
    verify_process(job,command)
    require(json.loads((job/"exit.json").read_text())["exit_code"]==0,"job exit not successful")
    for item in [command["script"],command["wrapper"],*command["toolchain"]]:
        require(record(Path(item["path"]))==item,"generating source/tool changed")
    return fresh

def execute(root, key, operation, rvas=(), size=16):
    require(key in MODULES,"unknown module")
    expected=MODULES[key]
    mode="probe" if operation in {"prepare","probe"} else operation
    targets=targets_for(mode,list(rvas),size,expected)
    root=ensure_root(root)
    with lease(root):
        snapshot=snapshot_module(root,key)
        projects=within(root,root/"projects"/key)
        projects.mkdir(parents=True,exist_ok=True)
        project="external_"+key+"_"+expected["sha256"][:12]
        project_file=projects/(project+".gpr")
        require((not project_file.exists()) if operation=="prepare" else project_file.is_file(),
                "project already exists" if operation=="prepare" else "missing prepared module project")
        job=within(root,root/"jobs"/(datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")+"-"+key+"-"+operation+"-"+uuid.uuid4().hex[:8]))
        job.mkdir(parents=True)
        scripts=job/"scripts";scripts.mkdir()
        script=SOURCE/"ExternalModuleEvidence.java"
        shutil.copyfile(script,scripts/script.name)
        shutil.copyfile(Path(__file__),job/Path(__file__).name)
        runner,runner_records=process_runner(job)
        request={"schema":"external-windows-request-v1","identity":expected,"snapshot":str(snapshot),
                 "mode":mode,"depth":0,"targets":targets,"output":str(job/"evidence")}
        write_json(job/"request.json",request)
        profile=within(root,root/"profile_v2");profile.mkdir(exist_ok=True)
        tmp=within(root,root/"tmp");tmp.mkdir(exist_ok=True)
        for sub in ["Roaming","Local"]:
            (profile/sub).mkdir(exist_ok=True)
        library_search=within(root,root/"empty_library_search");library_search.mkdir(exist_ok=True)
        require(not any(library_search.iterdir()),"library search directory must remain empty")
        utility=GHIDRA/"Ghidra/Framework/Utility/lib/Utility.jar"
        require(JAVA.is_file() and utility.is_file(),"missing headless toolchain")
        cmd=[str(JAVA),"-Xmx2G","-XX:ParallelGCThreads=2","-XX:CICompilerCount=2",
             "-Djava.system.class.loader=ghidra.GhidraClassLoader","-Dfile.encoding=UTF8",
             "-Djava.awt.headless=true",f"-Duser.home={profile}",f"-Djava.io.tmpdir={tmp}",
             "-Dlog4j.skipJansi=true","-Xshare:off","--enable-native-access=ALL-UNNAMED",
             "-cp",str(utility),"ghidra.Ghidra","ghidra.app.util.headless.AnalyzeHeadless",
             str(projects),project,"-librarySearchPaths",str(library_search)]
        if operation=="prepare":
            cmd+=["-import",str(snapshot),"-loader","PeLoader","-loader-loadLibraries","false",
                  "-loader-linkExistingProjectLibraries","false","-loader-ordinalLookup","false",
                  "-analysisTimeoutPerFile","300"]
        else:
            cmd+=["-process",expected["name"],"-noanalysis","-readOnly"]
        cmd+=["-max-cpu","2","-scriptPath",str(scripts),"-postScript","ExternalModuleEvidence.java",str(job/"request.json"),
              "-log",str(job/"ghidra.log"),"-scriptlog",str(job/"script.log")]
        env=os.environ.copy()
        env.update(APPDATA=str(profile/"Roaming"),LOCALAPPDATA=str(profile/"Local"),TEMP=str(tmp),TMP=str(tmp))
        tool_records=[record(p) for p in [JAVA,utility,GHIDRA/"Ghidra/Framework/SoftwareModeling/lib/SoftwareModeling.jar",
                      GHIDRA/"Ghidra/Features/Decompiler/lib/Decompiler.jar",
                      GHIDRA/"Ghidra/Features/Decompiler/os/win_x86_64/decompile.exe",
                      GHIDRA/"Ghidra/Features/Base/lib/Base.jar",GHIDRA/"Ghidra/application.properties"]]+runner_records
        command={"argv":cmd,"environment_overrides":{k:env[k] for k in ["APPDATA","LOCALAPPDATA","TEMP","TMP"]},
                 "toolchain":tool_records,"script":record(scripts/script.name),"wrapper":record(job/Path(__file__).name)}
        write_json(job/"command.json",command)
        print(json.dumps({"job":str(job),"operation":operation}),flush=True)
        completed=runner.run_headless(cmd,env=env,cwd=str(job),timeout_seconds=900,
                                     console_path=job/"console.log",receipt_path=job/"process.json")
        write_json(job/"exit.json",{"exit_code":completed.returncode})
        require(completed.returncode==0,"headless process failed: "+str(job))
        verify_process(job,command)
        for item in runner_records:
            require(record(Path(item["path"]))==item,"process runner changed during job")
        require(digest(script)==command["script"]["sha256"],"script changed during job")
        verified=verify_bundle(job,expected)
        verified.update(command=record(job/"command.json"),exit=record(job/"exit.json"),
                        console=record(job/"console.log"),process=record(job/"process.json"))
        write_json(job/"verification.json",verified)
        print(json.dumps({"status":"verified","job":str(job),**verified}),flush=True)
        return job

def loaded_header_matches(header, pe):
    require(len(header)>=64 and header[:2]==b"MZ","loaded header missing")
    nt=struct.unpack_from("<I",header,0x3c)[0]
    require(nt+84<=len(header) and header[nt:nt+4]==b"PE\0\0","loaded NT header invalid")
    actual=(struct.unpack_from("<H",header,nt+4)[0],struct.unpack_from("<I",header,nt+8)[0],
            struct.unpack_from("<I",header,nt+80)[0])
    require(actual==(pe.FILE_HEADER.Machine,pe.FILE_HEADER.TimeDateStamp,pe.OPTIONAL_HEADER.SizeOfImage),
            "loaded header/disk identity mismatch")

def resolve_imports(root, job):
    """Three named IAT joins, own process only, after verified KERNEL32 entry exports."""
    root=ensure_root(root);job=within(root,job)
    proof=verify_job(job,MODULES["kernel32"])
    require(proof["mode"]=="functions" and {t["rva"] for t in proof["targets"]}=={v[0] for v in ENTRY_JOINS.values()},
            "imports requires exactly the three KERNEL32 entry exports")
    require(os.name=="nt" and C.sizeof(C.c_void_p)==8,"requires 64-bit Windows")
    k=C.WinDLL("kernel32",use_last_error=True)
    k.GetModuleHandleW.argtypes=[W.LPCWSTR];k.GetModuleHandleW.restype=W.HMODULE
    k.GetModuleHandleExW.argtypes=[W.DWORD,C.c_void_p,C.POINTER(W.HMODULE)];k.GetModuleHandleExW.restype=W.BOOL
    k.GetModuleFileNameW.argtypes=[W.HMODULE,W.LPWSTR,W.DWORD];k.GetModuleFileNameW.restype=W.DWORD
    k.GetProcAddress.argtypes=[W.HMODULE,C.c_char_p];k.GetProcAddress.restype=C.c_void_p
    def loaded(key, handle):
        require(bool(handle),"missing loaded module")
        buf=C.create_unicode_buffer(32768)
        count=k.GetModuleFileNameW(handle,buf,len(buf))
        require(0<count<len(buf),"loaded module path unavailable")
        path=Path(buf.value).resolve()
        require(path==Path("C:/Windows/System32").joinpath(MODULES[key]["name"]).resolve(),"unexpected loaded module path")
        pe=identity(path,MODULES[key])
        loaded_header_matches(C.string_at(handle,4096),pe)
        return pe,{"disk":record(path),"identity":MODULES[key],"observed_base":hex(handle),"loaded_header_matches":True}
    with lease(root):
        h32=k.GetModuleHandleW("kernel32.dll");hb=k.GetModuleHandleW("KernelBase.dll")
        pe32,m32=loaded("kernel32",h32);peb,mb=loaded("kernelbase",hb)
        imports={s.name.decode():(d.dll.decode(),s.address-pe32.OPTIONAL_HEADER.ImageBase)
                 for d in pe32.DIRECTORY_ENTRY_IMPORT for s in d.imports if s.name and s.name.decode() in ENTRY_JOINS}
        rows=[]
        for name,(entry,slot,target) in ENTRY_JOINS.items():
            require(name in imports and imports[name][1]==slot,"named PE import slot mismatch")
            exported=[s for s in pe32.DIRECTORY_ENTRY_EXPORT.symbols if s.name==name.encode()]
            require(len(exported)==1 and not exported[0].forwarder and exported[0].address==entry,"expected code export missing")
            require(k.GetProcAddress(h32,name.encode())==h32+entry,"GetProcAddress entry mismatch")
            instructions=json.loads((job/"evidence"/f"func_{entry:x}.instructions.json").read_text())
            for ins in instructions:
                b=bytes.fromhex(ins["bytes"])
                require(C.string_at(h32+ins["rva"],len(b))==b,"loaded entry bytes differ from pinned export")
            pointer=C.c_void_p.from_address(h32+slot).value
            owner=W.HMODULE()
            require(pointer is not None and k.GetModuleHandleExW(6,C.c_void_p(pointer),C.byref(owner)),"import target owner unavailable")
            require(owner.value==hb and pointer-hb==target,"import target module/RVA mismatch")
            require(any(s.name==name.encode() and not s.forwarder and s.address==target for s in peb.DIRECTORY_ENTRY_EXPORT.symbols),
                    "KernelBase named direct export mismatch")
            rows.append({"name":name,"kernel32_entry_rva":entry,"import_contract":imports[name][0],"kernel32_iat_rva":slot,
                         "kernelbase_rva":target,"observed_pointer":hex(pointer),"loaded_entry_bytes_match":True})
        # Repeat module identity/header checks after bounded reads; no target functions invoked.
        loaded("kernel32",h32);loaded("kernelbase",hb)
        out=root/"jobs"/("loader-joins-"+uuid.uuid4().hex[:8]);out.mkdir()
        result={"schema":"external-windows-loader-joins-v1","utc":datetime.now(timezone.utc).isoformat(),
                "os_build":sys.getwindowsversion().build,"modules":[m32,mb],"rows":rows,"entry_export_receipt":record(job/"verification.json"),
                "wrapper":record(Path(__file__)),"scope":"own helper process; entry control flow requires cited D/I; not a the game loaded-module witness"}
        write_json(out/"loader_joins.json",result)
        print(json.dumps({"loader_joins":str(out/"loader_joins.json")}),flush=True)
        return out

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("operation",choices=["prepare","probe","functions","data","verify","imports"])
    p.add_argument("module",choices=sorted(MODULES))
    p.add_argument("--root",type=Path,default=DEFAULT_ROOT)
    p.add_argument("--rva",type=lambda v:int(v,0),action="append",default=[])
    p.add_argument("--size",type=int,default=16)
    p.add_argument("--job",type=Path)
    args=p.parse_args()
    if args.operation=="imports":
        require(args.module=="kernel32" and args.job is not None,"imports requires kernel32 --job")
        resolve_imports(args.root,args.job)
    elif args.operation=="verify":
        require(args.job is not None,"verify requires --job")
        job=within(ensure_root(args.root),args.job)
        print(json.dumps(verify_job(job,MODULES[args.module]),indent=2))
    else:
        require(args.job is None,"--job only applies to verify")
        execute(args.root,args.module,args.operation,args.rva,args.size)

if __name__=="__main__":
    try:
        main()
    except (Refusal,OSError,ValueError,KeyError,subprocess.TimeoutExpired) as exc:
        print(json.dumps({"status":"refused","reason":str(exc)}),file=sys.stderr)
        sys.exit(1)
