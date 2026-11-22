"""Stack frames of a release collector.dll, and the hourly hook's stack check.

usage:
  python tools/stack-frames.py <collector.dll> [--pdb <collector.pdb>] [--top N]
  python tools/stack-frames.py <collector.dll> [--pdb <collector.pdb>] --check [--root PATH]

Each frame is the stack a function reserves in its prologue: the return address,
pushed registers and allocation recorded in its x64 unwind information (.pdata),
including chained fragments. A function is attributed by its own start address,
never by the nearest preceding symbol. Names come from the PDB's procedure records
when it has debug information, otherwise from its publics; functions with neither
show as `<0xADDRESS>`. A `--profile release-lto` build keeps line tables for the
collector crates, which name their functions. With `--release`, use
`--config profile.release.package.collector-capture.debug=2` to name the hourly root.

--check follows direct calls and tail jumps from `collector_capture::sampling::tick`,
the hourly hook that runs acquisition on the game's main thread. It fails when a
reachable function's frame exceeds MAX_FRAME or the deepest acyclic chain from a
root exceeds MAX_DEPTH, and prints the offending call chains. Indirect calls, such
as job runners and panic hooks, are not followed. A failure means a large value
sits in a hook-path frame: build it, hold it and consume it out of line, or keep
it in preallocated storage.
Needs llvm-objdump and llvm-pdbutil (the LLVM toolchain that supplies lld-link).
"""
import argparse
import bisect
import os
import re
import shutil
import struct
import subprocess
import sys

MAX_FRAME = 64 * 1024
MAX_DEPTH = 384 * 1024
GAME_THREAD_ROOTS = ("collector_capture::sampling::tick",)

LLVM_DIRS = (r"C:\Program Files\LLVM\bin",)


def tool(name):
    found = shutil.which(name)
    if found:
        return found
    for directory in LLVM_DIRS:
        candidate = os.path.join(directory, name + ".exe")
        if os.path.exists(candidate):
            return candidate
    sys.exit(f"stack-frames: {name} not found (install LLVM or add it to PATH)")


class Image:
    """The PE sections and .pdata of a PE32+ image."""

    def __init__(self, path):
        with open(path, "rb") as file:
            self.data = data = file.read()
        pe = struct.unpack_from("<I", data, 0x3C)[0]
        if data[pe:pe + 4] != b"PE\0\0":
            sys.exit(f"stack-frames: {path} is not a PE image")
        sections, optional_size = struct.unpack_from("<H12xH", data, pe + 6)
        optional = pe + 24
        if struct.unpack_from("<H", data, optional)[0] != 0x20B:
            sys.exit(f"stack-frames: {path} is not PE32+")
        self.image_base = struct.unpack_from("<Q", data, optional + 24)[0]
        exception_rva, exception_size = struct.unpack_from("<II", data, optional + 112 + 3 * 8)
        self.sections = []
        for index in range(sections):
            header = optional + optional_size + 40 * index
            name = data[header:header + 8].rstrip(b"\0").decode("ascii", "replace")
            virtual_size, rva, raw_size, raw_offset = struct.unpack_from("<IIII", data, header + 8)
            self.sections.append((name, rva, max(virtual_size, raw_size), raw_offset))
        self.pdata = [struct.unpack_from("<III", data, self.offset(exception_rva) + 12 * i)
                      for i in range(exception_size // 12)]

    def offset(self, rva):
        for _, start, size, raw in self.sections:
            if start <= rva < start + size:
                return raw + rva - start
        raise ValueError(f"RVA {rva:#x} is outside every section")

    def section_rva(self, number):
        return self.sections[number - 1][1]

    def unwind(self, rva):
        """(stack bytes the prologue reserves, chained RUNTIME_FUNCTION or None)."""
        data = self.data
        at = self.offset(rva)
        version_flags, _, count = data[at], data[at + 1], data[at + 2]
        codes = struct.unpack_from(f"<{count}H", data, at + 4)
        size, index = 0, 0
        while index < count:
            code = codes[index]
            op, info = (code >> 8) & 0xF, code >> 12
            if op == 0:
                size, index = size + 8, index + 1
            elif op == 1:
                if info == 0:
                    size, index = size + codes[index + 1] * 8, index + 2
                else:
                    size, index = size + (codes[index + 1] | codes[index + 2] << 16), index + 3
            elif op == 2:
                size, index = size + info * 8 + 8, index + 1
            elif op == 10:
                size, index = size + (48 if info else 40), index + 1
            elif op in (4, 6, 8):
                index += 2
            elif op in (5, 9):
                index += 3
            else:
                index += 1
        chained = None
        if (version_flags >> 3) & 4:
            chained = struct.unpack_from("<III", data, at + 4 + 2 * (count + (count & 1)))
        return size, chained


def demangle(symbol):
    """Rust legacy mangling to a path without the hash; other names unchanged."""
    match = re.match(r"_?_ZN(.*)E(?:\.|$)", symbol)
    if not match:
        return symbol
    rest, parts = match.group(1), []
    while rest and rest[0].isdigit():
        digits = re.match(r"\d+", rest).group(0)
        length = int(digits)
        parts.append(rest[len(digits):len(digits) + length])
        rest = rest[len(digits) + length:]
    if parts and re.fullmatch(r"h[0-9a-f]{16}", parts[-1]):
        parts.pop()
    escapes = {"$LT$": "<", "$GT$": ">", "$RF$": "&", "$BP$": "*", "$C$": ",", "$LP$": "(",
               "$RP$": ")", "$SP$": "@", "..": "::"}

    def unescape(part):
        if part.startswith("_$"):
            part = part[1:]
        for escape, text in escapes.items():
            part = part.replace(escape, text)
        return re.sub(r"\$u([0-9a-f]+)\$", lambda m: chr(int(m.group(1), 16)), part)

    return "::".join(unescape(part) for part in parts)


def symbol_names(image, pdb):
    """Function start address -> name, procedure records preferred over publics."""
    pdbutil = tool("llvm-pdbutil")
    names = {}
    dump = subprocess.run([pdbutil, "dump", "--publics", pdb],
                          capture_output=True, text=True, check=True).stdout
    pending = None
    for line in dump.splitlines():
        match = re.search(r"S_PUB32 \[size = \d+\] `(.*)`", line)
        if match:
            pending = match.group(1)
            continue
        match = re.search(r"flags = function, addr = (\d+):(\d+)", line)
        if match and pending:
            address = image.image_base + image.section_rva(int(match.group(1))) + int(match.group(2))
            names.setdefault(address, demangle(pending))
        pending = None
    dump = subprocess.run([pdbutil, "dump", "--symbols", pdb],
                          capture_output=True, text=True, check=True).stdout
    pending = None
    for line in dump.splitlines():
        match = re.search(r"S_[GL]PROC32(?:_ID)? \[size = \d+\] `(.*)`", line)
        if match:
            pending = match.group(1)
            continue
        match = re.search(r"addr = (\d+):(\d+), code size", line)
        if match and pending:
            address = image.image_base + image.section_rva(int(match.group(1))) + int(match.group(2))
            names[address] = pending
        pending = None
    return names


class Functions:
    """Every function's frame from .pdata, keyed by its primary start address."""

    def __init__(self, image):
        base = image.image_base
        self.frame = {}
        self.ranges = []
        for begin, end, unwind in image.pdata:
            size, chained, primary = 0, (begin, end, unwind), begin
            seen = 0
            while chained is not None and seen < 32:
                part, next_chained = image.unwind(chained[2])
                size += part
                primary = chained[0]
                chained, seen = next_chained, seen + 1
            primary += base
            self.frame[primary] = max(self.frame.get(primary, 0), size + 8)
            self.ranges.append((begin + base, end + base, primary))
        self.ranges.sort()
        self.begins = [begin for begin, _, _ in self.ranges]

    def containing(self, address):
        index = bisect.bisect_right(self.begins, address) - 1
        if index >= 0 and address < self.ranges[index][1]:
            return self.ranges[index][2]
        return None


def call_graph(dll, functions):
    """Direct call and tail-jump edges between function starts: caller -> {callee: is_call}."""
    objdump = tool("llvm-objdump")
    edges = []
    pattern = re.compile(r"\s*([0-9a-f]+):\s+(call|callq|j[a-z]+)\s+0x([0-9a-f]+)")
    process = subprocess.Popen([objdump, "-d", "--no-show-raw-insn", "--section=.text", dll],
                               stdout=subprocess.PIPE, text=True, bufsize=1 << 20)
    for line in process.stdout:
        if "0x" not in line:
            continue
        match = pattern.match(line)
        if match:
            edges.append((int(match.group(1), 16), match.group(2).startswith("call"),
                          int(match.group(3), 16)))
    if process.wait() != 0:
        sys.exit("stack-frames: llvm-objdump failed")
    leaves = sorted({target for _, call, target in edges
                     if call and functions.containing(target) is None})
    for leaf in leaves:
        functions.frame.setdefault(leaf, 8)

    def owner(address):
        primary = functions.containing(address)
        if primary is not None:
            return primary
        index = bisect.bisect_right(leaves, address) - 1
        return leaves[index] if index >= 0 else None

    graph = {}
    for source, call, target in edges:
        caller = owner(source)
        if caller is None:
            continue
        if call:
            callee = owner(target)
        else:
            callee = functions.containing(target)
            if callee != target or callee == caller:
                continue
        if callee is not None and callee != caller:
            callees = graph.setdefault(caller, {})
            callees[callee] = callees.get(callee, False) or call
    return graph


def deepest(graph, frame, root):
    """(stack depth, call chain) of the deepest acyclic direct-call chain from root."""
    memo, active = {}, set()

    def through(node, callee):
        return memo[callee][0] + (frame.get(node, 8) if graph[node][callee] else 0)

    def visit(function):
        stack = [(function, iter(sorted(graph.get(function, ()))))]
        active.add(function)
        best = {function: (frame.get(function, 8), None)}
        while stack:
            node, callees = stack[-1]
            advanced = False
            for callee in callees:
                if callee in active:
                    continue
                if callee not in memo:
                    active.add(callee)
                    best[callee] = (frame.get(callee, 8), None)
                    stack.append((callee, iter(sorted(graph.get(callee, ())))))
                    advanced = True
                    break
                depth = through(node, callee)
                if depth > best[node][0]:
                    best[node] = (depth, callee)
            if advanced:
                continue
            stack.pop()
            active.discard(node)
            memo[node] = best[node]
            if stack:
                parent = stack[-1][0]
                depth = through(parent, node)
                if depth > best[parent][0]:
                    best[parent] = (depth, node)
        return memo[function]

    total, _ = visit(root)
    chain, node = [], root
    while node is not None:
        chain.append(node)
        node = memo[node][1]
    return total, chain


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("dll")
    parser.add_argument("--pdb", help="defaults to the DLL path with .pdb")
    parser.add_argument("--top", type=int, default=0, help="list the N largest frames")
    parser.add_argument("--check", action="store_true", help="check the game-thread hook paths")
    parser.add_argument("--root", action="append",
                        help="check from this function instead of GAME_THREAD_ROOTS (repeatable)")
    arguments = parser.parse_args()
    pdb = arguments.pdb or os.path.splitext(arguments.dll)[0] + ".pdb"
    image = Image(arguments.dll)
    functions = Functions(image)
    names = symbol_names(image, pdb)

    def label(address):
        return f"{functions.frame.get(address, 8):>7} B  {names.get(address, f'<{address:#x}>')}"

    if arguments.top:
        for address in sorted(functions.frame, key=lambda a: -functions.frame[a])[:arguments.top]:
            size = functions.frame[address]
            print(f"{size:>10} {size / 1024:9.1f} KiB  {address:#x}  {names.get(address, '?')}")
    if not arguments.check:
        return 0
    graph = call_graph(arguments.dll, functions)
    failures = 0
    for root_name in arguments.root or GAME_THREAD_ROOTS:
        roots = sorted(address for address, name in names.items()
                       if name == root_name or name.startswith(root_name + "<"))
        if not roots:
            print(f"FAIL {root_name}: no function carries this name")
            failures += 1
            continue
        parent, queue = dict.fromkeys(roots), list(roots)
        for node in queue:
            for callee in sorted(graph.get(node, ())):
                if callee not in parent:
                    parent[callee] = node
                    queue.append(callee)
        large = sorted((a for a in parent if functions.frame.get(a, 8) > MAX_FRAME),
                       key=lambda a: -functions.frame[a])
        for address in large:
            chain, node = [], address
            while node is not None:
                chain.append(label(node))
                node = parent[node]
            print(f"FAIL frame over {MAX_FRAME} B, called from:")
            for line in chain:
                print(f"    {line}")
        failures += len(large)
        depth, chain = max(deepest(graph, functions.frame, root) for root in roots)
        verdict = "FAIL" if large or depth > MAX_DEPTH else "ok"
        failures += depth > MAX_DEPTH
        largest = max(functions.frame.get(a, 8) for a in parent)
        print(f"{verdict} {root_name}: {len(parent)} reachable functions, largest frame "
              f"{largest} B (bound {MAX_FRAME}), deepest chain {depth} B (bound {MAX_DEPTH})")
        if depth > MAX_DEPTH or arguments.top:
            for node in chain:
                print(f"    {label(node)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
