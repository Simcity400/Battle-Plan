"""phase0.py - read-only verification CLI for the Battle Plan memory reader.

This is the Python half of the Phase-0 loop in RESEARCH_memory_reader.md.
Cheat Engine (run by you, on the live game) does the *discovery*; this tool
*verifies* each finding read-only from Python and is the harness we grow into
the real reader.

Nothing here ever writes to the game. Requires 64-bit Python.

phase0 is a manual DISCOVERY/verification tool and deliberately does NOT enforce
the exe build-hash gate (you point read/watch/chain at arbitrary addresses on
whatever build is loaded - that is the whole point). The PRODUCTION reader path
(locate.locate + the readers) IS build-gated; see locate.main. Do not wire
phase0's raw read/watch/chain into an automated reader without adding that gate
(CODE_REVIEW M7).

Quick tour (run with the game open unless noted):
  python phase0.py selftest                 # no game needed - proves the plumbing
  python phase0.py procs --filter hoi        # find the game's process name
  python phase0.py info                       # Step 1: attach + build fingerprint
  python phase0.py watch 0x<addr> --type u32  # Step 2: watch the date tick
  python phase0.py read 0x<addr> --type u32   # one-shot typed read
  python phase0.py chain 0x<pPlayer> 0x408 0x90 --read u32 --at 0x9C --scale 1000
  python phase0.py aob "48 8B 05 ?? ?? ?? ?? 48 89"   # find a code anchor
  python phase0.py rip 0x<match> --disp 3 --len 7     # decode its global

All addresses accept hex (0x..) or decimal. Use --rel to treat an address as
module-base-relative (handy when cribbing offsets from a Cheat Engine table).
"""

import argparse
import sys
import time

import winmem


TYPES = {
    "u8": "u8", "i8": "i8", "u16": "u16", "i16": "i16",
    "u32": "u32", "i32": "i32", "u64": "u64", "i64": "i64",
    "f32": "f32", "f64": "f64",
}


def _int(s):
    return int(str(s), 0)


def _read_typed(proc, addr, t):
    return getattr(proc, t)(addr)


def _fmt(val, scale):
    if scale and scale != 1 and isinstance(val, (int, float)):
        return "%s  (raw %s / %s)" % (val / scale, val, scale)
    return str(val)


def _attach(args):
    proc = winmem.open_process(args.process, args.module)
    if proc is None:
        sys.exit("Process %r not found. Start the game (or pass --process) and retry." % args.process)
    return proc


def _resolve(proc, addr, rel):
    return proc.module_base + addr if rel else addr


# --------------------------------------------------------------------------
# commands
# --------------------------------------------------------------------------
def cmd_procs(args):
    needle = (args.filter or "").lower()
    rows = sorted(winmem.list_processes(), key=lambda r: r[1].lower())
    for pid, name in rows:
        if needle in name.lower():
            print("%-8d %s" % (pid, name))


def cmd_info(args):
    proc = _attach(args)
    with proc:
        fp = proc.fingerprint()
        print("pid          : %d" % fp["pid"])
        print("module       : %s" % fp["module"])
        print("base address : 0x%X" % fp["base"])
        print("module size  : 0x%X (%d bytes)" % (fp["size"], fp["size"]))
        print("exe path     : %s" % fp["path"])
        print("exe sha256   : %s" % fp.get("sha256"))
        print("python bits  : %d" % fp["python_bits"])
        if fp["python_bits"] != 64:
            print("** WARNING: 64-bit Python required to read the 64-bit game **")


def cmd_read(args):
    proc = _attach(args)
    with proc:
        addr = _resolve(proc, _int(args.addr), args.rel)
        val = _read_typed(proc, addr, TYPES[args.type])
        print("0x%X [%s] = %s" % (addr, args.type, _fmt(val, args.scale)))


def cmd_watch(args):
    proc = _attach(args)
    with proc:
        addr = _resolve(proc, _int(args.addr), args.rel)
        t = TYPES[args.type]
        print("watching 0x%X [%s] every %.2fs (Ctrl-C to stop)" % (addr, args.type, args.interval))
        last = object()
        n = 0
        try:
            while args.count == 0 or n < args.count:
                try:
                    val = _read_typed(proc, addr, t)
                except OSError as e:
                    val = "<read error: %s>" % e
                mark = "" if val == last else "  <-- changed"
                print("[%s] %s%s" % (time.strftime("%H:%M:%S"), _fmt(val, args.scale), mark))
                last = val
                n += 1
                time.sleep(args.interval)
        except KeyboardInterrupt:
            print("stopped.")


def cmd_chain(args):
    proc = _attach(args)
    with proc:
        base = _resolve(proc, _int(args.base), args.rel)
        offsets = [_int(o) for o in args.offsets]
        obj = proc.chain(base, offsets, deref_base=not args.no_deref_base)
        print("chain base=0x%X offsets=%s -> 0x%X" % (base, [hex(o) for o in offsets], obj))
        if args.read:
            at = obj + (_int(args.at) if args.at else 0)
            val = _read_typed(proc, at, TYPES[args.read])
            label = " +0x%X" % _int(args.at) if args.at else ""
            print("  read [%s] @ 0x%X%s = %s" % (args.read, at, label, _fmt(val, args.scale)))


def cmd_aob(args):
    proc = _attach(args)
    with proc:
        want_exec = None if args.any else (False if args.data else True)
        hits = proc.scan_module(args.pattern, want_exec=want_exec,
                                limit=args.limit, allow_short=args.allow_short)
        scope = "any" if args.any else ("data" if args.data else "code")
        print("pattern matched %d site(s) in %s pages:" % (len(hits), scope))
        for a in hits:
            print("  0x%X   (module+0x%X)" % (a, a - proc.module_base))
        if not hits:
            print("  (none - widen wildcards or try --data / --any)")


def cmd_rip(args):
    proc = _attach(args)
    with proc:
        instr = _resolve(proc, _int(args.addr), args.rel)
        tgt = proc.rip_target(instr, args.disp, args.len)
        print("instr 0x%X  disp@+%d len %d -> global 0x%X (module+0x%X)"
              % (instr, args.disp, args.len, tgt, tgt - proc.module_base))


def cmd_selftest(args):
    """Validate the ctypes plumbing without the game: attach to ANY running
    process (default: this Python), read its PE header, and AOB-scan for 'MZ'."""
    name = args.target
    pid = winmem.find_pid(name) if name else None
    if pid is None:
        # fall back to our own process
        import os
        pid = os.getpid()
        name = "(this python)"
    print("selftest target: %s pid=%d" % (name, pid))
    with winmem.Process(pid) as proc:
        fp = proc.fingerprint()
        print("  module       : %s" % fp["module"])
        print("  base         : 0x%X  size 0x%X" % (fp["base"], fp["size"]))
        print("  python bits  : %d" % fp["python_bits"])
        magic = proc.read(proc.module_base, 2)
        print("  base[0:2]    : %r  (expect b'MZ')" % magic)
        assert magic == b"MZ", "PE header read failed - RPM plumbing is broken"
        # e_lfanew at 0x3C -> PE\0\0 signature
        e_lfanew = proc.u32(proc.module_base + 0x3C)
        sig = proc.read(proc.module_base + e_lfanew, 4)
        print("  PE signature : %r  (expect b'PE\\x00\\x00')" % sig)
        assert sig == b"PE\x00\x00", "PE signature mismatch - bad e_lfanew or read"
        hits = proc.scan_module("4D 5A 90 00", want_exec=None, limit=4)  # 'MZ 90 00' in header
        print("  AOB scan     : %d hit(s) for 'MZ 90 00'" % len(hits))
        assert proc.module_base in hits, "AOB scan did not locate the header at module base"
        # direct scan_blob checks: wildcard honoring + anchor-offset math
        t1 = winmem.scan_blob(b"\xAA\x48\x8B\x05\x99\x88\x77\x66\xC3", 0x1000,
                              "48 8B 05 ?? ?? ?? ?? C3")
        assert t1 == [0x1001], "scan_blob wildcard test failed: %r" % t1
        t2 = winmem.scan_blob(b"\x11\x8B\x05\x90\x22", 0x2000, "?? 8B 05 90")
        assert t2 == [0x2000], "scan_blob anchor-offset test failed: %r" % t2
        print("  scan_blob    : wildcard + anchor-offset tests passed")
        print("SELFTEST OK - OpenProcess / module enum / RPM / AOB all working.")


# --------------------------------------------------------------------------
# arg parsing
# --------------------------------------------------------------------------
def build_parser():
    p = argparse.ArgumentParser(description="Battle Plan - Phase-0 read-only memory verifier")
    p.add_argument("--process", default="hoi4.exe", help="target process exe name (default hoi4.exe)")
    p.add_argument("--module", default=None, help="module to base offsets on (default = main exe)")
    sub = p.add_subparsers(dest="cmd", required=True)

    sp = sub.add_parser("procs", help="list processes (optionally --filter)")
    sp.add_argument("--filter", default="")
    sp.set_defaults(func=cmd_procs)

    sp = sub.add_parser("info", help="Step 1: attach + print build fingerprint")
    sp.set_defaults(func=cmd_info)

    sp = sub.add_parser("read", help="one typed read")
    sp.add_argument("addr")
    sp.add_argument("--type", default="u32", choices=list(TYPES))
    sp.add_argument("--rel", action="store_true", help="addr is module-relative")
    sp.add_argument("--scale", type=float, default=1.0, help="divide raw by this (fixed-point)")
    sp.set_defaults(func=cmd_read)

    sp = sub.add_parser("watch", help="Step 2: poll an address and flag changes")
    sp.add_argument("addr")
    sp.add_argument("--type", default="u32", choices=list(TYPES))
    sp.add_argument("--rel", action="store_true")
    sp.add_argument("--interval", type=float, default=1.0)
    sp.add_argument("--count", type=int, default=0, help="stop after N reads (0 = forever)")
    sp.add_argument("--scale", type=float, default=1.0)
    sp.set_defaults(func=cmd_watch)

    sp = sub.add_parser("chain", help="Steps 3-4: resolve a pointer chain, optionally read a field")
    sp.add_argument("base")
    sp.add_argument("offsets", nargs="*", help="e.g. 0x408 0x90")
    sp.add_argument("--rel", action="store_true", help="base is module-relative")
    sp.add_argument("--no-deref-base", action="store_true", help="base is already an object addr")
    sp.add_argument("--read", default=None, choices=list(TYPES), help="read a field at the resolved object")
    sp.add_argument("--at", default=None, help="field offset to add before reading (e.g. 0x9C)")
    sp.add_argument("--scale", type=float, default=1.0)
    sp.set_defaults(func=cmd_chain)

    sp = sub.add_parser("aob", help="AOB-scan the module for a wildcard pattern")
    sp.add_argument("pattern", help='e.g. "48 8B 05 ?? ?? ?? ?? 48 89"')
    sp.add_argument("--data", action="store_true", help="scan data pages instead of code")
    sp.add_argument("--any", action="store_true", help="scan all readable pages")
    sp.add_argument("--limit", type=int, default=64)
    sp.add_argument("--allow-short", action="store_true",
                    help="permit a pattern whose longest fixed run is <3 bytes (broad/slow scan)")
    sp.set_defaults(func=cmd_aob)

    sp = sub.add_parser("rip", help="decode a RIP-relative load to recover a global address")
    sp.add_argument("addr", help="address of the instruction")
    sp.add_argument("--disp", type=int, default=3, help="byte offset of disp32 within the instr")
    sp.add_argument("--len", type=int, default=7, help="instruction length")
    sp.add_argument("--rel", action="store_true")
    sp.set_defaults(func=cmd_rip)

    sp = sub.add_parser("selftest", help="prove the ctypes plumbing without the game")
    sp.add_argument("--target", default=None, help="attach to this process instead of our own python")
    sp.set_defaults(func=cmd_selftest)

    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        args.func(args)
    except (OSError, ValueError, AssertionError, RuntimeError) as e:
        sys.exit("error: %s" % e)
    except KeyboardInterrupt:
        sys.exit(130)


if __name__ == "__main__":
    main()
