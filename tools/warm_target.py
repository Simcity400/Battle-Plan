"""Start a worktree's cargo target directory from a warm seed build (Python 3.11+, Windows).

    python tools/warm_target.py seed --root seed --like <a worktree path>
    python tools/warm_target.py warm --root seed
    python tools/warm_target.py check --root seed --target-dir <dir>

`seed` checks out a commit S in the seed's own checkout, builds it with the usual commands and
records S, the toolchain, the lockfile hash and every place the build wrote the seed's paths.
`warm` copies that build into a worktree's empty target directory, rewrites the seed's paths
to the worktree's, invalidates whatever it cannot rewrite exactly, and backdates the worktree's
sources whose bytes equal the seed's, so cargo rebuilds only what differs. `check` lists any
reference to the seed left in a target directory.
"""

import argparse
import bisect
import concurrent.futures
import ctypes
import filecmp
import hashlib
import json
import mmap
import os
import re
import shutil
import subprocess
import sys
import time
import uuid
import _winapi
from ctypes import wintypes
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
# A seed root holds two slots, each a checkout and its build with a record and an index in
# `slots/<slot>`; `seed.json` beside them is the record of the slot warms use. A refresh
# rebuilds the other slot and then replaces `seed.json`.
SEED_FILE = "seed.json"
INDEX_FILE = "index.json"
CHECKOUT_FILE = "checkout.txt"  # slot a's checkout path
SLOTS = ("a", "b")
SLOTS_DIR = "slots"
READERS_DIR = "reading"  # one file per running warm, naming the slot it copies
UPDATE_LOCK = "updating"
REFRESH_LOG = "refresh.log"  # what `warm --refresh-behind`'s background refreshes print
REFRESH_COMMIT = "origin/main"
GAME_PROCESS = "hoi4.exe"  # refreshes wait while it runs: benchmarks and oracle runs
LAUNCHER = "warm_target.py"  # the tool at the seed's commit, for scheduled refreshes
STAGING_RECORDS = "warming"  # one file per running warm, naming its staging copy
STAGING_PREFIX = ".warming-"
# `warm --target-roots` keeps a worktree's profile directory in
# `<root>\<worktree name>-<path hash>-<warm pid>-cargo-target\debug`, naming its junction in
# OWNER_FILE.
EXTERNAL_SUFFIX = "-cargo-target"
OWNER_FILE = "worktree.txt"
SEED_FORMAT = 2

# Cargo's encoded dep-info (`.fingerprint/<unit>/dep-*`) as written by the pinned toolchain:
# a marker that older readers reject, a format version, the files rustc read (each tagged as
# relative to the package root or to the target root; files outside the package are stored
# absolute), then the environment variables rustc read.
DEP_INFO_MARKER = b"\x01\x00\x00\x00\xff"
DEP_INFO_VERSION = 1
PACKAGE_RELATIVE = 0
TARGET_RELATIVE = 1

# The commands a seed is built with, run from the seed checkout, each going on past crates
# that fail to compile. `cargo test --no-run` builds exactly the units `cargo nextest run
# --no-run` does (nextest drives it), and reports them.
SEED_COMMANDS = (
    ("check", ["cargo", "check", "--workspace", "--all-targets", "--keep-going"], []),
    ("test", ["cargo", "test", "--workspace", "--no-run", "--no-fail-fast"], []),
)

# Not copied: per-session compiler caches and scratch. Incremental caches are large, hold
# paths of the session that wrote them, and only speed up crates that rebuild anyway.
SKIPPED_DIRS = ("debug/incremental", "tmp", "cargo-timings", "release", "doc", "nextest")
SKIPPED_ROOT_FILES = (".cargo-lock", ".cargo-artifact-lock", ".cargo-build-lock")
ROOT_CACHES = SKIPPED_ROOT_FILES + (".rustc_info.json", "CACHEDIR.TAG")
# Files directly in `debug/` are cargo's uplifted links and locks; cargo recreates the links
# from `deps/` whenever it builds those units.
UPLIFTED_DIR = "debug"

# A PDB (MSF 7.00) begins with this signature, then its page size.
MSF_MAGIC = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\x00\x00\x00"
UNIT_HASH = re.compile(r"-([0-9a-f]{16})(?:\.|$)")
PATH_SEPARATORS = ("\\", "/", "\\\\")
PATH_CASES = ("exact", "lower", "drive")
PATH_ENCODINGS = ("utf-8", "utf-16-le")
STYLES = [(sep, case, enc) for enc in PATH_ENCODINGS for case in PATH_CASES for sep in PATH_SEPARATORS]


class WarmError(Exception):
    """A condition that makes warming unsafe; nothing is left in the target directory."""


class SeedBusy(WarmError):
    """Another seed update is running."""


# ---------------------------------------------------------------------------------------
# Encoded dep-info


def parse_dep_info(data):
    """Returns ([(kind, path)], [(name, value or None)]) or raises WarmError."""
    if not data.startswith(DEP_INFO_MARKER):
        raise WarmError("unsupported dep-info layout (no format marker)")
    pos = len(DEP_INFO_MARKER)
    if data[pos] != DEP_INFO_VERSION:
        raise WarmError(f"unsupported dep-info format version {data[pos]}")
    pos += 1

    def u32():
        nonlocal pos
        value = int.from_bytes(data[pos:pos + 4], "little")
        pos += 4
        return value

    def chunk():
        nonlocal pos
        size = u32()
        value = data[pos:pos + size]
        if len(value) != size:
            raise WarmError("truncated dep-info")
        pos += size
        return value

    def byte():
        nonlocal pos
        value = data[pos]
        pos += 1
        return value

    files = []
    for _ in range(u32()):
        kind = byte()
        if kind not in (PACKAGE_RELATIVE, TARGET_RELATIVE):
            raise WarmError(f"unsupported dep-info path kind {kind}")
        path = chunk().decode("utf-8")
        if byte() != 0:
            raise WarmError("dep-info records content checksums, which warming does not support")
        files.append((kind, path))
    env = []
    for _ in range(u32()):
        name = chunk().decode("utf-8")
        flag = byte()
        if flag not in (0, 1):
            raise WarmError(f"unsupported dep-info env flag {flag}")
        env.append((name, chunk().decode("utf-8") if flag else None))
    if pos != len(data):
        raise WarmError("trailing bytes after dep-info")
    return files, env


def encode_dep_info(files, env):
    out = bytearray(DEP_INFO_MARKER)
    out.append(DEP_INFO_VERSION)

    def chunk(text):
        raw = text.encode("utf-8")
        out.extend(len(raw).to_bytes(4, "little"))
        out.extend(raw)

    out.extend(len(files).to_bytes(4, "little"))
    for kind, path in files:
        out.append(kind)
        chunk(path)
        out.append(0)
    out.extend(len(env).to_bytes(4, "little"))
    for name, value in env:
        chunk(name)
        if value is None:
            out.append(0)
        else:
            out.append(1)
            chunk(value)
    return bytes(out)


# ---------------------------------------------------------------------------------------
# Paths and the forms they are written in


def render(path, style):
    sep, case, encoding = style
    text = path
    if case == "lower":
        text = text.lower()
    elif case == "drive":
        text = text[:1].lower() + text[1:]
    return text.replace("\\", sep).encode(encoding)


def shaped_checkout(root, like):
    """A checkout path under `root` with the length and separator count of `like`.

    Built artifacts embed the checkout path (source maps, `env!("CARGO_MANIFEST_DIR")`,
    panic locations). A seed whose path has the shape of the worktrees it warms lets every
    such reference be rewritten in place, byte for byte, instead of rebuilding the unit.
    """
    root, like = str(root).rstrip("\\"), str(like).rstrip("\\")
    extra = like.count("\\") - root.count("\\")
    filler = ["s"] * (extra - 1)
    last = len(like) - len(root) - extra - sum(map(len, filler))
    if extra < 1 or last < len("checkout"):
        raise WarmError(f"seed root {root} is too long to match the shape of {like}")
    return "\\".join([root, *filler, "checkout" + "_" * (last - len("checkout"))])


def under(path, root):
    """`path` relative to `root` (case-insensitive, as on Windows), or None."""
    p, r = os.path.normcase(path), os.path.normcase(root.rstrip("\\"))
    if p == r:
        return ""
    if p.startswith(r + "\\"):
        return path[len(r) + 1:]
    return None


def map_path(path, mapping):
    for old, new in mapping:
        rel = under(path, old)
        if rel is not None:
            return os.path.join(new, rel) if rel else new
    return path


def map_text(text, mapping):
    for old, new in mapping:
        for sep in PATH_SEPARATORS:
            pattern = re.compile(re.escape(old.replace("\\", sep)), re.IGNORECASE)
            text = pattern.sub(lambda _: new.replace("\\", sep), text)
    return text


# ---------------------------------------------------------------------------------------
# Target directory layout


def skipped(rel, is_dir):
    rel = rel.replace("\\", "/")
    if is_dir:
        return rel in SKIPPED_DIRS
    head, _, name = rel.rpartition("/")
    return (head == "" and name in SKIPPED_ROOT_FILES) or head == UPLIFTED_DIR


def target_files(target):
    """Relative paths of the files a warm copies, and a stamp of them (count, bytes and
    newest mtime) that changes whenever cargo writes to the directory."""
    files, total, newest = [], 0, 0
    stack = [""]
    while stack:
        rel_dir = stack.pop()
        with os.scandir(os.path.join(target, rel_dir)) as entries:
            for entry in entries:
                rel = os.path.join(rel_dir, entry.name) if rel_dir else entry.name
                is_dir = entry.is_dir(follow_symlinks=False)
                if skipped(rel, is_dir):
                    continue
                if is_dir:
                    stack.append(rel)
                elif entry.is_file(follow_symlinks=False):
                    files.append(rel)
                    info = entry.stat(follow_symlinks=False)
                    total += info.st_size
                    newest = max(newest, info.st_mtime_ns)
    return files, {"files": len(files), "bytes": total, "newest": newest}


def file_kind(rel):
    """How a file that names the seed is brought over to the worktree.

    `dep-info` and `text` are cargo's and rustc's own bookkeeping, rewritten whatever the
    path lengths. Everything else (artifacts, build-script outputs) is patched in place only
    when the replacement has the same length, else its unit is rebuilt.
    """
    parts = rel.replace("\\", "/").split("/")
    if len(parts) == 4 and parts[1] == ".fingerprint":
        return "dep-info" if parts[3].startswith("dep-") else "text"
    if len(parts) == 4 and parts[1] == "build" and parts[3] in ("output", "root-output", "stderr"):
        return "text"
    if parts[-1].endswith(".d") and len(parts) <= 4 and parts[1] in ("deps", "examples", "build"):
        return "text"
    return "binary"


def unhashed_targets(fingerprint_names):
    """Normalised names of the targets a unit may link without its hash in the file name.

    On MSVC, cargo names the binaries it links for running, bins and examples outside their
    test harnesses and cdylibs, `deps/<name>.exe`, `examples/<name>.exe` or
    `deps/<name>.dll`; their fingerprint files are `bin-<name>`, `example-<name>` and
    `lib-<name>`. Several units of one target share that file.
    """
    return {name.split("-", 1)[1].replace("-", "_") for name in fingerprint_names
            if name.startswith(("bin-", "example-", "lib-")) and not name.endswith(".json")}


def unhashed_owners(profile, units):
    """The unit that last linked each unhashed output: the one whose dep-info, written when
    its compile started, is newest. Cargo would call any other unit of that target fresh
    while the file holds a different unit's code."""
    owners = set()
    for kind in ("deps", "examples"):
        directory = os.path.join(profile, kind)
        names = os.listdir(directory) if os.path.isdir(directory) else []
        for name in names:
            if UNIT_HASH.search(name):
                continue
            candidates = owning_units(os.path.join("debug", kind, name), units)

            def compiled(unit):
                stamps = [os.stat(os.path.join(profile, ".fingerprint", unit, f)).st_mtime_ns
                          for f in units[unit] if f.startswith("dep-")]
                return max(stamps, default=0)

            if candidates:
                owners.add(max(candidates, key=compiled))
    return owners


def owning_units(rel, units):
    """Fingerprint directories that produce `rel`; `units` maps each directory name to the
    names of the files in it."""
    parts = rel.replace("\\", "/").split("/")
    if len(parts) >= 3 and parts[1] in (".fingerprint", "build"):
        return [parts[2]]
    if len(parts) == 3 and parts[1] in ("deps", "examples"):
        found = UNIT_HASH.search(parts[2])
        if found:
            return [name for name in units if name.endswith("-" + found.group(1))]
        stem = parts[2].split(".", 1)[0].replace("-", "_")
        return [name for name, files in units.items() if stem in unhashed_targets(files)]
    return []


def list_units(profile):
    fingerprints = os.path.join(profile, ".fingerprint")
    return {name: os.listdir(os.path.join(fingerprints, name)) for name in os.listdir(fingerprints)}


# ---------------------------------------------------------------------------------------
# Finding the seed's paths in its build


def scan_file(args):
    """Occurrences of the roots in one file: (rel, [(offset, root, style)], unrecognized).

    `markers` name the seed however it is written: any of them outside a whole root path
    counts as unrecognized.
    """
    target, rel, roots, markers = args
    path = os.path.join(target, rel)
    found = []
    with open(path, "rb") as handle:
        if os.fstat(handle.fileno()).st_size == 0:
            return rel, [], 0
        with mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_READ) as data:
            for root_index, root in enumerate(roots):
                for style_index, style in enumerate(STYLES):
                    needle = render(root, style)
                    at = data.find(needle)
                    while at >= 0:
                        found.append((at, len(needle), root_index, style_index))
                        at = data.find(needle, at + 1)
            # Keep the longest match at each place (a target inside the checkout).
            found.sort(key=lambda o: (o[0], -o[1]))
            kept, end = [], -1
            for offset, size, root_index, style_index in found:
                if offset >= end:
                    kept.append((offset, root_index, style_index))
                    end = offset + size
                elif offset + size > end:
                    raise WarmError(f"overlapping seed paths in {rel}")
            starts = [o for o, _, _ in kept]
            stops = [o + len(render(roots[r], STYLES[s])) for o, r, s in kept]

            def covered(at):
                i = bisect.bisect_right(starts, at) - 1
                return i >= 0 and at < stops[i]

            unrecognized = 0
            for marker in markers:
                for needle in {marker, marker.lower(), marker.upper(), marker.replace("\\", "/")}:
                    for encoding in PATH_ENCODINGS:
                        raw = needle.encode(encoding)
                        at = data.find(raw)
                        while at >= 0:
                            unrecognized += not covered(at)
                            at = data.find(raw, at + 1)
            if data[:len(MSF_MAGIC)] == MSF_MAGIC:
                unrecognized += split_paths(data, covered, roots[-1])
    return rel, kept, unrecognized


def split_paths(data, covered, root):
    """Paths a PDB stores across two of its pages that are not adjacent in the file.

    A PDB is a multi-stream file whose streams are lists of fixed-size pages, so a string
    can start at the end of one page and continue at the start of another anywhere in the
    file. Such a path cannot be patched, and even its directory name may be cut in two;
    a prefix of the checkout at the end of a page with the rest of it at the start of a
    page counts as one.
    """
    block = int.from_bytes(data[32:36], "little")
    if block not in (512, 1024, 2048, 4096, 8192, 16384, 32768):
        raise WarmError("unsupported PDB page size")
    found = 0
    for style in (("\\", "exact", "utf-8"), ("\\", "lower", "utf-8"), ("/", "exact", "utf-8")):
        form = render(root, style)
        ends, begins = set(), set()
        for boundary in range(block, len(data), block):
            if covered(boundary - 1) and covered(boundary):
                continue
            for k in range(1, len(form)):
                if data[boundary - k:boundary] == form[:k]:
                    ends.add(k)
                if data[boundary:boundary + len(form) - k] == form[k:]:
                    begins.add(k)
        found += len(ends & begins)
    return found


def seed_markers(checkout, root):
    """What gives the seed away besides its whole paths: the checkout's directory name (a
    relative path, another case, a cut string) and the seed's own directory, which tools
    reach by resolving `..` against the checkout (a linker resolving a library's relative
    source paths, for one)."""
    return [checkout.rsplit("\\", 1)[-1], root.rstrip("\\")]


def scan_target(target, roots, files, jobs, markers):
    index = {}
    with concurrent.futures.ProcessPoolExecutor(max_workers=jobs) as pool:
        work = ((target, rel, roots, markers) for rel in files)
        for rel, occurrences, unrecognized in pool.map(scan_file, work, chunksize=64):
            if occurrences or unrecognized:
                index[rel] = {"kind": file_kind(rel), "at": occurrences, "unrecognized": unrecognized}
    return index


# ---------------------------------------------------------------------------------------
# Processes, git and cargo


def run(args, cwd, env=None, check=True):
    result = subprocess.run(args, cwd=cwd, env=env, capture_output=True, text=True,
                            encoding="utf-8", errors="replace")
    if check and result.returncode != 0:
        raise WarmError(f"{' '.join(args)} failed in {cwd}: {result.stderr.strip()}")
    return result.stdout


def git(cwd, *args):
    return run(["git", *args], cwd).strip()


def git_dirty(checkout):
    return git(checkout, "status", "--porcelain", "--untracked-files=all")


def toolchain(cwd):
    return run(["rustc", "-vV"], cwd).strip() + "\n" + run(["cargo", "-V"], cwd).strip()


def cargo_metadata(cwd, env):
    return json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"],
                          cwd, env))


def seed_env():
    env = dict(os.environ)
    env.pop("CARGO_TARGET_DIR", None)
    env.pop("CARGO_BUILD_TARGET_DIR", None)
    return env


def sha256(path):
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def tauri_packages(metadata):
    """Workspace packages whose build runs Tauri's context macro.

    That macro embeds the app's configuration, icons and frontend files at compile time,
    and rustc's dep-info does not list them all; those packages are always rebuilt.
    """
    return sorted(p["name"] for p in metadata["packages"]
                  if any(d["name"] == "tauri-build" for d in p["dependencies"]))


def live_units(logs, profile):
    """Hashes of the units the seed commands used, from cargo's JSON messages, which list
    fresh units too; None if a command reported no units at all."""
    live = set()

    def add(path):
        rel = under(os.path.normpath(path), profile)
        parts = rel.split("\\") if rel else []
        name = parts[1] if len(parts) > 2 and parts[0] == "build" else parts[-1] if parts else ""
        found = UNIT_HASH.search(name)
        if found:
            live.add(found.group(1))

    for log in logs:
        artifacts = 0
        with open(log, encoding="utf-8") as handle:
            for line in handle:
                if not line.startswith("{"):
                    continue
                message = json.loads(line)
                if message.get("reason") == "compiler-artifact":
                    artifacts += 1
                    for path in message["filenames"] + [message.get("executable") or ""]:
                        if path:
                            add(path)
                elif message.get("reason") == "build-script-executed":
                    add(message["out_dir"])
        if artifacts == 0:
            return None
    return live


def collect_garbage(profile, live):
    """Deletes the units no seed command used, so warms copy only what the seed uses. A unit
    whose outputs carry no hash is kept only while it owns them (`unhashed_owners`)."""
    freed = 0
    units = list_units(profile)
    owners = unhashed_owners(profile, units)
    for name, files in units.items():
        if files and (name in owners or UNIT_HASH.search(name).group(1) in live):
            continue
        shutil.rmtree(os.path.join(profile, ".fingerprint", name))
    for kind in ("build", "deps", "examples"):
        directory = os.path.join(profile, kind)
        if not os.path.isdir(directory):
            continue
        for entry in os.scandir(directory):
            found = UNIT_HASH.search(entry.name)
            if not found or found.group(1) in live:
                continue
            if entry.is_dir():
                freed += sum(f.stat().st_size for f in Path(entry.path).rglob("*") if f.is_file())
                shutil.rmtree(entry.path)
            else:
                freed += entry.stat().st_size
                os.remove(entry.path)
    return freed


# ---------------------------------------------------------------------------------------
# Copying


def copy_seed(source, destination, files, threads):
    """Copies `files` from `source` to `destination` with Windows' copy engine, which clones
    blocks instead of copying bytes on ReFS (a Dev Drive).

    Nothing is hard-linked: lld rewrites a small executable in place when it relinks it, so
    a rebuild in a worktree would change the seed's file, and the rlibs and rmeta rustc
    does replace by rename are under a quarter of a warm.
    """
    copy = ctypes.windll.kernel32.CopyFile2
    copy.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR, ctypes.c_void_p]
    copy.restype = ctypes.c_long
    for rel_dir in sorted({os.path.dirname(rel) for rel in files}):
        os.makedirs(os.path.join(destination, rel_dir), exist_ok=True)

    def one(rel):
        result = copy(os.path.join(source, rel), os.path.join(destination, rel), None)
        if result < 0:
            raise WarmError(f"CopyFile2 failed for {rel}: HRESULT {result & 0xFFFFFFFF:#010x}")

    with concurrent.futures.ThreadPoolExecutor(max_workers=threads) as pool:
        list(pool.map(one, files))


def copy_robocopy(source, destination, threads):
    excluded_dirs = [os.path.join(source, d.replace("/", "\\")) for d in SKIPPED_DIRS]
    excluded_files = []
    for rel_dir in ("", "debug"):
        if os.path.isdir(os.path.join(source, rel_dir)):
            with os.scandir(os.path.join(source, rel_dir)) as entries:
                excluded_files += [e.path for e in entries if e.is_file()
                                   and skipped(os.path.join(rel_dir, e.name), False)]
    args = ["robocopy", source, destination, "/E", "/COPY:DAT", "/DCOPY:T", f"/MT:{threads}",
            "/R:2", "/W:1", "/XJ", "/NFL", "/NDL", "/NP", "/NJH", "/NJS"]
    args += ["/XD", *excluded_dirs] + ["/XF", *excluded_files]
    result = subprocess.run(args, capture_output=True, text=True, errors="replace")
    # Robocopy's exit code is a bit set; 8 and above mean some files were not copied.
    if result.returncode >= 8:
        raise WarmError(f"robocopy failed ({result.returncode}): {result.stdout.strip()[-2000:]}")


# ---------------------------------------------------------------------------------------
# Bringing the copy over to the worktree


def keep_mtime(path, write):
    """Runs `write` and restores the file's times: cargo compares artifact mtimes."""
    info = os.stat(path)
    result = write()
    os.utime(path, ns=(info.st_atime_ns, info.st_mtime_ns))
    return result


def patch_in_place(path, occurrences, old_roots, new_roots):
    with open(path, "r+b") as handle:
        for offset, root_index, style_index in occurrences:
            style = STYLES[style_index]
            old, new = render(old_roots[root_index], style), render(new_roots[root_index], style)
            handle.seek(offset)
            if handle.read(len(old)) != old:
                raise WarmError(f"{path} changed after the seed was indexed")
            handle.seek(offset)
            handle.write(new)


def rewrite_text(path, mapping):
    with open(path, "rb") as handle:
        data = handle.read()
    text = data.decode("utf-8", errors="surrogateescape")
    with open(path, "wb") as handle:
        handle.write(map_text(text, mapping).encode("utf-8", errors="surrogateescape"))


def rewrite_dep_info(path, mapping):
    """Rewrites the absolute paths in one dep-info. Returns False, writing nothing, when an
    environment value rustc read names the seed: the unit was built for the seed's paths."""
    with open(path, "rb") as handle:
        files, env = parse_dep_info(handle.read())
    if any(value is not None and map_text(value, mapping) != value for _, value in env):
        return False
    files = [(kind, map_path(p, mapping) if os.path.isabs(p) else p) for kind, p in files]
    # A seed path written some other way (`\\?\` verbatim, say) is not rewritten.
    if any(map_text(p, mapping) != p for _, p in files):
        return False
    with open(path, "wb") as handle:
        handle.write(encode_dep_info(files, env))
    return True


def patchable(occurrences, old_roots, new_roots):
    return all(len(render(old_roots[r], STYLES[s])) == len(render(new_roots[r], STYLES[s]))
               for _, r, s in occurrences)


def invalidate(staging, units):
    """Removes units' fingerprints, build-script directories and hashed outputs: cargo
    rebuilds them, and everything that depends on them, on first use."""
    profile = os.path.join(staging, "debug")
    for name in units:
        for kind in (".fingerprint", "build"):
            shutil.rmtree(os.path.join(profile, kind, name), ignore_errors=True)
    hashes = {UNIT_HASH.search(name).group(1) for name in units}
    for kind in ("deps", "examples"):
        directory = os.path.join(profile, kind)
        if os.path.isdir(directory):
            for entry in os.scandir(directory):
                found = UNIT_HASH.search(entry.name)
                if found and found.group(1) in hashes:
                    os.remove(entry.path)


def translate(staging, index, old_roots, new_roots, forced_packages):
    """Rewrites or invalidates every indexed reference to the seed. Returns counts."""
    mapping = list(zip(old_roots, new_roots))
    profile = os.path.join(staging, "debug")
    invalid, removed, counts = set(), [], {"rewritten": 0, "patched": 0}
    units = list_units(profile)
    for rel, entry in index.items():
        path = os.path.join(staging, rel)
        if not os.path.exists(path):
            continue
        clear = entry["unrecognized"] == 0
        if clear and entry["kind"] == "dep-info" and keep_mtime(
                path, lambda: rewrite_dep_info(path, mapping)):
            counts["rewritten"] += 1
        elif clear and entry["kind"] == "text":
            keep_mtime(path, lambda: rewrite_text(path, mapping))
            counts["rewritten"] += 1
        elif clear and entry["kind"] == "binary" and patchable(entry["at"], old_roots, new_roots):
            keep_mtime(path, lambda: patch_in_place(path, entry["at"], old_roots, new_roots))
            counts["patched"] += 1
        else:
            owners = owning_units(rel, units)
            if not owners:
                raise WarmError(f"{rel} names the seed and belongs to no unit")
            invalid.update(owners)
            removed.append(path)
    invalid.update(name for name in units if name.rsplit("-", 1)[0] in forced_packages)
    invalidate(staging, invalid)
    for path in removed:
        if os.path.exists(path):
            os.remove(path)
    counts["invalidated units"] = len(invalid)
    counts["removed files"] = len(removed)
    return counts


def source_inputs(staging, packages):
    """Worktree-relative files and directories cargo checks the mtimes of."""
    profile = os.path.join(staging, "debug")
    fingerprints = os.path.join(profile, ".fingerprint")
    inputs = set()
    with os.scandir(fingerprints) as units:
        units = list(units)
    for unit in units:
        package = packages.get(unit.name.rsplit("-", 1)[0])
        with os.scandir(unit.path) as entries:
            entries = list(entries)
        for entry in entries:
            if entry.name.startswith("dep-"):
                with open(entry.path, "rb") as handle:
                    files, _ = parse_dep_info(handle.read())
                for kind, path in files:
                    # Files outside the package are absolute, whichever root they are
                    # tagged with; relative target-root files were generated in the target
                    # directory and keep their copied mtimes.
                    if os.path.isabs(path):
                        inputs.add(path)
                    elif kind == PACKAGE_RELATIVE:
                        if package is None:
                            raise WarmError(f"{unit.name} tracks sources but is not a workspace package")
                        inputs.add(os.path.join(package, path))
            elif entry.name.endswith(".json") and package is not None:
                with open(entry.path, encoding="utf-8") as handle:
                    fingerprint = json.load(handle)
                for local in fingerprint.get("local", []):
                    changed = local.get("RerunIfChanged")
                    if changed:
                        inputs.update(p if os.path.isabs(p) else os.path.join(package, p)
                                      for p in changed["paths"])
    return inputs


def same_tree(a, b):
    """Whether two files, or two directories recursively, hold the same bytes."""
    if os.path.isfile(a) and os.path.isfile(b):
        return filecmp.cmp(a, b, shallow=False)
    if not (os.path.isdir(a) and os.path.isdir(b)):
        return False
    names = sorted(os.listdir(a))
    return names == sorted(os.listdir(b)) and all(
        same_tree(os.path.join(a, n), os.path.join(b, n)) for n in names)


def set_times(path, times):
    """Sets the mtime of a file, or of a directory and everything in it. `times` maps a
    path relative to `path` to (atime, mtime) in nanoseconds."""
    for dirpath, dirnames, filenames in os.walk(path) if os.path.isdir(path) else []:
        for name in filenames + dirnames:
            full = os.path.join(dirpath, name)
            if not os.path.islink(full):
                os.utime(full, ns=times(os.path.relpath(full, path)))
    os.utime(path, ns=times("."))


def seed_times(seed_path):
    def times(rel):
        info = os.stat(os.path.join(seed_path, rel))
        return info.st_atime_ns, info.st_mtime_ns
    return times


def checkout_files(checkout, rel_dir=None):
    """Relative paths of a checkout's files that are not ignored, tracked or not: the files
    cargo lists for a package."""
    args = ["ls-files", "-z", "--cached", "--others", "--exclude-standard"]
    out = run(["git", *args] + (["--", rel_dir] if rel_dir else []), checkout)
    # The index still lists files deleted from the working tree.
    return {os.path.normpath(p) for p in out.split("\0")
            if p and os.path.lexists(os.path.join(checkout, p))}


def precalculated_units(staging, packages, worktree, checkout):
    """Build-script runs whose package's file list differs from the seed's.

    A workspace build script that names no `rerun-if-changed` path reruns when the newest
    mtime among its package's files changes. Backdating keeps that mtime for unchanged
    files, but a file deleted from the package would not move it, so such a run is rebuilt
    whenever the package gained or lost a file.
    """
    stale = []
    fingerprints = os.path.join(staging, "debug", ".fingerprint")
    for unit in os.listdir(fingerprints):
        package = packages.get(unit.rsplit("-", 1)[0])
        if package is None:
            continue
        for name in os.listdir(os.path.join(fingerprints, unit)):
            if not (name.startswith("run-build-script-") and name.endswith(".json")):
                continue
            with open(os.path.join(fingerprints, unit, name), encoding="utf-8") as handle:
                local = json.load(handle).get("local", [])
            if any("Precalculated" in entry for entry in local) and (
                    checkout_files(worktree, package) != checkout_files(checkout, package)):
                stale.append(unit)
    return stale


def newest_mtime(path):
    stamps = [os.stat(path).st_mtime_ns]
    for dirpath, dirnames, filenames in os.walk(path) if os.path.isdir(path) else []:
        stamps += [os.stat(os.path.join(dirpath, n)).st_mtime_ns for n in filenames + dirnames]
    return max(stamps)


def backdate(inputs, worktree, checkout, started):
    """Gives each input whose bytes equal the seed's the seed's mtime, and every other input
    the current time, so cargo's mtime comparison sees exactly the inputs that differ.

    Inputs written since the warm `started` keep their mtimes, which are newer than any of
    the seed's builds: cargo may already have built them into the installed copy.
    """
    same = differ = 0
    now = time.time_ns()
    for path in sorted(inputs):
        rel = under(os.path.normpath(os.path.join(worktree, path)), worktree)
        if not rel:
            continue  # outside the worktree, through `..` or another root
        mine, seeds = os.path.join(worktree, rel), os.path.join(checkout, rel)
        if not os.path.exists(mine) or os.path.islink(mine) or os.path.islink(seeds):
            continue
        if newest_mtime(mine) >= started:
            continue
        if same_tree(mine, seeds):
            set_times(mine, seed_times(seeds))
            # An edit between the comparison and the backdating would otherwise be hidden.
            if same_tree(mine, seeds):
                same += 1
                continue
        # The worktree may have been checked out before the seed was built, so a differing
        # input's own mtime can predate the seed's dep-info.
        set_times(mine, lambda _: (now, now))
        differ += 1
    return same, differ


# ---------------------------------------------------------------------------------------
# Commands


def read_seed(root):
    """The record of the seed slot warms use now, or None."""
    return read_record(os.path.join(root, SEED_FILE))


def read_record(path):
    if not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as handle:
        record = json.load(handle)
    return record if record.get("format") == SEED_FORMAT else None


def write_record(path, record):
    with open(path + ".tmp", "w", encoding="utf-8") as handle:
        json.dump(record, handle, indent=1)
    os.replace(path + ".tmp", path)


def slot_dir(root, slot):
    return os.path.join(root, SLOTS_DIR, slot)


def slot_checkout(root, slot):
    """Each slot's checkout: the recorded path, with its last character naming slot `b`, so
    both have the shape the seed was made `--like`."""
    with open(os.path.join(root, CHECKOUT_FILE), encoding="utf-8") as handle:
        base = handle.read().strip()
    return base if slot == SLOTS[0] else base[:-1] + slot


def register_reader(root, slot):
    """Records that this process copies from `slot`; a refresh waits for it before
    rebuilding that slot."""
    directory = os.path.join(root, READERS_DIR)
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, str(os.getpid()))
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(slot)
    return path


def slot_readers(root, slot):
    """Live processes copying from `slot`; records of dead ones are removed."""
    directory = os.path.join(root, READERS_DIR)
    readers = []
    for name in os.listdir(directory) if os.path.isdir(directory) else []:
        path = os.path.join(directory, name)
        if not name.isdigit() or not process_alive(int(name)):
            os.remove(path)
            continue
        with open(path, encoding="utf-8") as handle:
            if handle.read().strip() == slot:
                readers.append(int(name))
    return readers


def update_running(lock):
    """Whether a live process holds the update lock."""
    try:
        with open(os.path.join(lock, "pid"), encoding="utf-8") as handle:
            return process_alive(int(handle.read()))
    except FileNotFoundError:
        # A holder writes its pid right after taking the lock.
        return os.path.isdir(lock) and time.time() - os.stat(lock).st_mtime < 60
    except ValueError:
        return False


def take_update_lock(root):
    """Claims the seed for one update. A lock whose process has exited (an update that was
    killed) is taken over: the slot it was building has no record, so nothing used it."""
    lock = os.path.join(root, UPDATE_LOCK)
    while True:
        try:
            os.mkdir(lock)
            break
        except FileExistsError:
            if update_running(lock):
                raise SeedBusy(f"another seed update holds {lock}") from None
            print(f"taking over {lock}: the update that held it is no longer running")
            shutil.rmtree(lock, ignore_errors=True)
    with open(os.path.join(lock, "pid"), "w", encoding="utf-8") as handle:
        handle.write(str(os.getpid()))
    return lock


def command_seed(args):
    root = os.path.abspath(args.root)
    os.makedirs(root, exist_ok=True)
    try:
        lock = take_update_lock(root)
    except SeedBusy as busy:
        if args.if_moved:
            print(f"seed: {busy}; skipped")
            return 0
        raise
    try:
        return update_seed(root, args)
    finally:
        shutil.rmtree(lock)


def update_seed(root, args):
    """Builds the slot warms are not using and then points warms at it, so a refresh never
    leaves them without a seed."""
    record_path = os.path.join(root, CHECKOUT_FILE)
    if not os.path.exists(record_path):
        if args.if_moved:
            print(f"seed: no seed in {root} to refresh; create it with --like; skipped")
            return 0
        if not args.like:
            raise WarmError("a new seed needs --like <worktree path> to choose its checkout path")
        with open(record_path, "w", encoding="utf-8") as handle:
            handle.write(shaped_checkout(root, os.path.abspath(args.like)) + "\n")
    # Any slot's checkout is a worktree of the repository, so the launcher copy of this tool
    # beside the seed can refresh it.
    checkouts = [slot_checkout(root, slot) for slot in SLOTS]
    repo = args.repo or next((c for c in checkouts if os.path.exists(os.path.join(c, ".git"))), str(ROOT))
    if args.if_moved and game_running():
        print("seed: the game is running; skipped")
        return 0
    if not args.no_fetch:
        git(repo, "fetch", "--quiet", "origin")
    commit = git(repo, "rev-parse", "--verify", args.commit + "^{commit}")
    sweep_staging(root)
    active = read_seed(root)
    if args.if_moved and active and active["commit"] == commit:
        print(f"seed: already at {commit[:10]}; skipped")
        return 0
    slot = SLOTS[1] if active and active["slot"] == SLOTS[0] else SLOTS[0]
    # Warms that began before the last refresh may still copy from this slot.
    deadline = time.time() + args.reader_wait * 60
    while slot_readers(root, slot):
        if time.time() > deadline:
            print(f"seed: warms still copy from slot {slot}; skipped")
            return 0
        time.sleep(5)
    directory = slot_dir(root, slot)
    os.makedirs(directory, exist_ok=True)
    slot_record = os.path.join(directory, SEED_FILE)
    if os.path.exists(slot_record):
        os.remove(slot_record)  # a slot being rebuilt is not used
    checkout = slot_checkout(root, slot)
    try:
        # Never check out in a repository the slot directory merely sits inside.
        top = os.path.normcase(os.path.abspath(git(checkout, "rev-parse", "--show-toplevel")))
        if top != os.path.normcase(checkout):
            raise WarmError(f"{checkout} is not a checkout of its own")
        git(checkout, "checkout", "--quiet", "--detach", "--force", commit)
    except (WarmError, OSError):
        # No checkout yet, or one a killed refresh left half made: start it again.
        run(["git", "worktree", "remove", "--force", checkout], repo, check=False)
        shutil.rmtree(checkout, ignore_errors=True)
        git(repo, "worktree", "prune")
        git(repo, "worktree", "add", "--detach", checkout, commit)
    env = seed_env()
    metadata = cargo_metadata(checkout, env)
    target = os.path.normpath(metadata["target_directory"])
    rel_target = under(target, checkout)
    clean = ["clean", "-ffdxq"] + (["-e", "/" + rel_target.replace("\\", "/") + "/"] if rel_target else [])
    git(checkout, *clean)
    if git_dirty(checkout):
        raise WarmError(f"seed checkout {checkout} is not clean")
    profile = os.path.join(target, "debug")
    if active and not os.path.isdir(profile):
        # A new slot starts as a warm copy of the other, so only what changed compiles.
        result = warm_into(root, active, checkout, target, None, "copyfile2", 16)
        print(f"slot {slot} started from slot {active['slot']}: " + report(result))
    logs = os.path.join(directory, "logs")
    os.makedirs(logs, exist_ok=True)
    results = []
    for name, cargo_args, tail in SEED_COMMANDS:
        full = cargo_args + ["-j", str(args.jobs), "--message-format=json-render-diagnostics"] + tail
        start = time.time()
        with open(os.path.join(logs, f"{name}.json"), "w", encoding="utf-8") as out, \
                open(os.path.join(logs, f"{name}.err"), "w", encoding="utf-8") as err:
            code = subprocess.run(full, cwd=checkout, env=env, stdout=out, stderr=err).returncode
        results.append({"command": " ".join(cargo_args + tail),
                        "exit": code, "seconds": round(time.time() - start, 1)})
        print(f"{name}: exit {code} in {results[-1]['seconds']} s", flush=True)
    if git(checkout, "rev-parse", "HEAD") != commit or git_dirty(checkout):
        raise WarmError(f"seed checkout {checkout} changed during the build")
    # A unit no seed command used is either outdated or blocked by a crate that failed to
    # compile at this commit; neither is worth copying.
    live = live_units([os.path.join(logs, f"{c[0]}.json") for c in SEED_COMMANDS], profile)
    if live is None:
        print("a seed command reported no units: older units are kept")
    else:
        freed = collect_garbage(profile, live)
        print(f"{len(live)} units in use; removed {freed / 2**30:.1f} GiB of older units")
    if not args.keep_incremental:
        # Warms never copy them, and they grew to six times the rest of the seed; the next
        # update compiles the crates that changed without them.
        shutil.rmtree(os.path.join(profile, "incremental"), ignore_errors=True)
    start = time.time()
    files, stamp = target_files(target)
    index = scan_target(target, [target, checkout], files, args.jobs, seed_markers(checkout, root))
    index_path = os.path.join(directory, INDEX_FILE)
    with open(index_path, "w", encoding="utf-8") as handle:
        json.dump(index, handle)
    print(f"indexed {len(files)} files ({stamp['bytes'] / 2**30:.1f} GiB), {len(index)} name the seed, "
          f"in {time.time() - start:.0f} s", flush=True)
    packages = {}
    for package in metadata["packages"]:
        rel = under(os.path.dirname(os.path.normpath(package["manifest_path"])), checkout)
        if rel is not None:
            packages[package["name"]] = rel
    seed = {
        "format": SEED_FORMAT,
        "generation": uuid.uuid4().hex,
        "slot": slot,
        "commit": commit,
        "checkout": checkout,
        "target": target,
        "index": index_path,
        "toolchain": toolchain(checkout),
        "cargo_lock_sha256": sha256(os.path.join(checkout, "Cargo.lock")),
        "packages": packages,
        "rebuilt_packages": tauri_packages(metadata),
        "commands": results,
        "stamp": stamp,
        "built": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    write_record(slot_record, seed)
    write_record(os.path.join(root, SEED_FILE), seed)  # warms switch to this slot
    # The launcher a scheduled refresh runs: this commit's tool (or, in a repository without
    # it, the running one), beside the seed.
    tool = os.path.join(checkout, "tools", "warm_target.py")
    tool = tool if os.path.exists(tool) else os.path.abspath(__file__)
    shutil.copy2(tool, os.path.join(root, LAUNCHER) + ".tmp")
    os.replace(os.path.join(root, LAUNCHER) + ".tmp", os.path.join(root, LAUNCHER))
    print(f"seed {commit[:10]} ready in slot {slot} ({checkout})")
    return 0


def staging_owner(name):
    """The pid in a warm's staging directory name, or None for any other entry."""
    suffix = name[len(STAGING_PREFIX):] if name.startswith(STAGING_PREFIX) else ""
    return int(suffix) if suffix.isdigit() else None


def register_staging(root, staging):
    """Records a warm's staging copy in the seed, so that if the warm is killed the next
    warm of any worktree removes it."""
    directory = os.path.join(root, STAGING_RECORDS)
    os.makedirs(directory, exist_ok=True)
    record = os.path.join(directory, str(os.getpid()))
    with open(record, "w", encoding="utf-8") as handle:
        handle.write(staging)
    return record


def sweep_staging(root):
    directory = os.path.join(root, STAGING_RECORDS)
    for name in os.listdir(directory) if os.path.isdir(directory) else []:
        if not name.isdigit() or process_alive(int(name)):
            continue
        record = os.path.join(directory, name)
        with open(record, encoding="utf-8") as handle:
            staging = handle.read().strip()
        if staging_owner(os.path.basename(staging)) == int(name):
            shutil.rmtree(staging, ignore_errors=True)
        os.remove(record)


def process_alive(pid):
    handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)  # query limited information
    if not handle:
        return False
    code = wintypes.DWORD()
    ctypes.windll.kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
    ctypes.windll.kernel32.CloseHandle(handle)
    return code.value == 259  # still active


def warm_target_dir(worktree, requested):
    if requested:
        return os.path.abspath(requested)
    return os.path.normpath(cargo_metadata(worktree, dict(os.environ))["target_directory"])


def junction_target(path):
    """Where a directory junction (or symbolic link) points, or None for anything else."""
    try:
        target = os.readlink(path)
    except (OSError, ValueError):
        return None
    return target[4:] if target.startswith("\\\\?\\") else target


def free_bytes(path):
    probe = Path(path)
    while not probe.exists():
        probe = probe.parent
    return shutil.disk_usage(probe).free


def external_profile_dir(target_root, worktree):
    """Where this warm puts a worktree's profile directory on `target_root`: named after the
    worktree, after its full path and after this process, so no other warm, of this worktree
    or another, ever fills, installs or deletes it. The junction decides which warm installs."""
    tag = hashlib.sha256(os.path.normcase(worktree).encode()).hexdigest()[:8]
    name = f"{os.path.basename(worktree)}-{tag}-{os.getpid()}{EXTERNAL_SUFFIX}"
    return os.path.join(target_root, name)


def sweep_external(target_roots):
    """Deletes profile directories on the target roots whose worktree junction no longer
    points at them (the worktree was removed or cleaned) and that no live warm is filling."""
    for target_root in target_roots:
        for name in os.listdir(target_root) if os.path.isdir(target_root) else []:
            directory = os.path.join(target_root, name)
            if not name.endswith(EXTERNAL_SUFFIX) or not os.path.isdir(directory):
                continue
            try:
                with open(os.path.join(directory, OWNER_FILE), encoding="utf-8") as handle:
                    junction = handle.read().strip()
            except FileNotFoundError:
                junction = ""
            profile = os.path.join(directory, "debug")
            owned = junction and os.path.normcase(junction_target(junction) or "") == os.path.normcase(profile)
            filling = any(staging_owner(n) and process_alive(staging_owner(n)) for n in os.listdir(directory))
            young = time.time() - os.stat(directory).st_mtime < 60  # a warm is just creating it
            if not (owned or filling or young):
                shutil.rmtree(directory, ignore_errors=True)


def game_running():
    out = run(["tasklist", "/FI", f"IMAGENAME eq {GAME_PROCESS}", "/NH"], os.getcwd(), check=False)
    return GAME_PROCESS.lower() in out.lower()


def start_refresh(root, worktree):
    """Starts `seed --if-moved` for the seed in the background, detached from this process
    and its console and below normal priority (cargo and rustc inherit it), when the seed is
    behind this worktree's origin/main, no update runs and the game is not running.
    Returns at once, with the refresh's pid if it started one; the refresh writes to
    REFRESH_LOG beside the seed."""
    seed = read_seed(root)
    try:
        main = git(worktree, "rev-parse", "--verify", REFRESH_COMMIT + "^{commit}")
    except WarmError:
        main = None
    if seed and seed["commit"] == main:
        return None
    if update_running(os.path.join(root, UPDATE_LOCK)):
        print("warm: a seed refresh is already running")
        return None
    if game_running():
        print("warm: the game is running; the seed is not refreshed")
        return None
    # The newest tool there is: the active slot's, else the launcher, else this one.
    tools = [os.path.join(seed["checkout"], "tools", "warm_target.py")] if seed else []
    tools += [os.path.join(root, LAUNCHER), os.path.abspath(__file__)]
    tool = next(t for t in tools if os.path.exists(t))
    log_path = os.path.join(root, REFRESH_LOG)
    command = [sys.executable, tool, "seed", "--root", root, "--if-moved", "-j", "8"]
    flags = (subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP
             | subprocess.BELOW_NORMAL_PRIORITY_CLASS)
    with open(log_path, "ab") as log:
        log.write(f"\n--- {time.strftime('%Y-%m-%d %H:%M:%S')} {' '.join(command)}\n".encode())
        log.flush()
        try:  # outlive a job object that would end it with the hook
            child = subprocess.Popen(command, cwd=root, stdin=subprocess.DEVNULL, stdout=log,
                                     stderr=subprocess.STDOUT,
                                     creationflags=flags | subprocess.CREATE_BREAKAWAY_FROM_JOB)
        except OSError:
            child = subprocess.Popen(command, cwd=root, stdin=subprocess.DEVNULL, stdout=log,
                                     stderr=subprocess.STDOUT, creationflags=flags)
    print(f"warm: the seed is behind {REFRESH_COMMIT}; refreshing it in the background "
          f"(process {child.pid}, log {log_path})")
    return child.pid


def command_warm(args):
    root = os.path.abspath(args.root)
    worktree = os.path.abspath(git(os.path.abspath(args.worktree), "rev-parse", "--show-toplevel"))
    try:
        return warm_worktree(args, root, worktree)
    finally:
        if args.refresh_behind:
            start_refresh(root, worktree)


def warm_worktree(args, root, worktree):
    seed = read_seed(root)
    if seed is None:
        print(f"warm: no usable seed in {root}; cargo will build from scratch")
        return 0
    target_roots = [os.path.abspath(r) for r in (args.target_roots or "").split(";") if r.strip()]
    if target_roots and args.target_dir:
        raise WarmError("--target-roots and --target-dir exclude each other")
    target = warm_target_dir(worktree, args.target_dir)
    os.makedirs(target, exist_ok=True)
    sweep_staging(root)
    sweep_external(target_roots)
    profile = os.path.join(target, "debug")
    if os.path.lexists(profile) and junction_target(profile) and not os.path.exists(profile):
        os.rmdir(profile)  # a junction to a profile directory that is gone
    # Cargo's caches beside the profile directory do not make a build.
    if any(not staging_owner(name) and name not in ROOT_CACHES for name in os.listdir(target)):
        print(f"warm: {target} already holds a build; left as it is")
        return 0
    if toolchain(worktree) != seed["toolchain"]:
        print("warm: the worktree's toolchain differs from the seed's; cargo will build from scratch")
        return 0
    external = None
    if target_roots:
        # The profile directory goes to the root with the most free space, behind a junction
        # at the path cargo uses, so every path the build records is still the worktree's.
        chosen = max(target_roots, key=free_bytes)
        left = free_bytes(chosen) - seed["stamp"]["bytes"]
        if left < args.min_free_gb * 2**30:
            print(f"warm: {chosen} would keep {left / 2**30:.0f} GiB free, under the "
                  f"{args.min_free_gb} GiB floor; cargo will build from scratch")
            return 0
        external = external_profile_dir(chosen, worktree)
        shutil.rmtree(external, ignore_errors=True)  # left by an exited warm with this pid
    result = warm_into(root, seed, worktree, target, external, args.copy, args.threads)
    if result is None:
        print(f"warm: cargo started building in {target} first; left its build alone")
        return 0
    lock = sha256(os.path.join(worktree, "Cargo.lock")) == seed["cargo_lock_sha256"]
    print(f"warm: {target} seeded from {seed['commit'][:10]}"
          + (f", its profile directory on {external}" if external else "")
          + ("" if lock else " (Cargo.lock differs: units it changes rebuild)"))
    print("  " + report(result))
    return 0


def warm_into(root, seed, worktree, target, external, copy, threads):
    """Copies the seed slot's build into the empty `target` of `worktree` and makes it the
    worktree's own. Returns what it did, or None if cargo started building there first."""
    checkout = seed["checkout"]
    slot_record = os.path.join(slot_dir(root, seed["slot"]), SEED_FILE)
    reader = register_reader(root, seed["slot"])
    try:
        if read_record(slot_record) != seed or git_dirty(checkout):
            raise WarmError(f"seed slot {seed['slot']} no longer matches its record")
        with open(seed["index"], encoding="utf-8") as handle:
            index = json.load(handle)
        started = time.time_ns()
        staging = os.path.join(external or target, f"{STAGING_PREFIX}{os.getpid()}")
        record = register_staging(root, staging)
        os.makedirs(staging)
        if external:
            with open(os.path.join(external, OWNER_FILE), "w", encoding="utf-8") as handle:
                handle.write(os.path.join(target, "debug"))
        timings = {}
        installed = False
        try:
            start = time.time()
            if copy == "robocopy":
                copy_robocopy(seed["target"], staging, threads)
            else:
                copy_seed(seed["target"], staging, target_files(seed["target"])[0], threads)
            timings[f"copy ({copy})"] = time.time() - start
            # Cargo run in the seed checkout outside `seed` would leave the index stale.
            if target_files(staging)[1] != seed["stamp"]:
                raise WarmError("the seed's build changed after it was indexed; run `seed` again")

            start = time.time()
            old_roots, new_roots = [seed["target"], checkout], [target, worktree]
            counts = translate(staging, index, old_roots, new_roots, set(seed["rebuilt_packages"]))
            timings["translate"] = time.time() - start

            stale = precalculated_units(staging, seed["packages"], worktree, checkout)
            invalidate(staging, stale)
            counts["invalidated units"] += len(stale)
            # Every file cargo might stat: what the units read, and every file a package
            # lists (build scripts without `rerun-if-changed` paths watch all of them).
            inputs = source_inputs(staging, seed["packages"]) | checkout_files(worktree)

            if read_record(slot_record) != seed or git_dirty(checkout):
                raise WarmError("the seed changed while it was copied")
            installed = install(staging, target, external)
            if not installed:
                return None
        finally:
            shutil.rmtree(staging, ignore_errors=True)
            os.remove(record)
            if external and not installed:
                shutil.rmtree(external, ignore_errors=True)
        # Backdating still reads the seed checkout: a refresh may replace it once this reader goes.
        start = time.time()
        same, differ = backdate(inputs, worktree, checkout, started)
        timings["backdate"] = time.time() - start
    finally:
        os.remove(reader)

    counts["sources unchanged"], counts["changed"] = same, differ
    return seed["stamp"]["bytes"], timings, counts


def report(result):
    copied, timings, counts = result
    return (f"copied {copied / 2**30:.2f} GiB; "
            + ", ".join(f"{k} {v:.1f} s" for k, v in timings.items()) + "; "
            + ", ".join(f"{k} {v}" for k, v in counts.items()))


def install(staging, target, external):
    """Puts the translated copy in place. The profile directory goes first: if cargo started
    building in the target meanwhile, it owns the directory and the copy is dropped (False).
    With `external`, the profile directory moves there and a junction in the target points
    at it; the rest of the copy is cargo's small caches, which it recreates if missing."""
    profile = os.path.join(target, "debug")
    if external:
        os.rename(os.path.join(staging, "debug"), os.path.join(external, "debug"))
        try:
            _winapi.CreateJunction(os.path.join(external, "debug"), profile)
        except OSError:
            if os.path.lexists(profile):
                return False
            raise
        for name in os.listdir(staging):
            if not os.path.exists(os.path.join(target, name)):
                shutil.copy2(os.path.join(staging, name), os.path.join(target, name))
        return True
    names = sorted(os.listdir(staging), key=lambda name: name != "debug")
    for name in names:
        try:
            os.rename(os.path.join(staging, name), os.path.join(target, name))
        except FileExistsError:
            if name == "debug":
                return False
    return True


def command_check(args):
    root = os.path.abspath(args.root)
    if not os.path.exists(os.path.join(root, CHECKOUT_FILE)):
        raise WarmError(f"no seed in {root}")
    checkouts = [slot_checkout(root, slot) for slot in SLOTS]
    roots = [read_record(os.path.join(slot_dir(root, slot), SEED_FILE)) for slot in SLOTS]
    roots = [r["target"] for r in roots if r and under(r["target"], r["checkout"]) is None] + checkouts
    target = os.path.abspath(args.target_dir)
    files, _ = target_files(target)
    markers = [c.rsplit("\\", 1)[-1] for c in checkouts] + [root.rstrip("\\")]
    index = scan_target(target, roots, files, args.jobs, markers)
    for rel, entry in sorted(index.items()):
        print(f"{rel}: {len(entry['at'])} seed paths, {entry['unrecognized']} other references")
    print(f"{len(index)} of {len(files)} files name the seed")
    return 1 if index else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    seed = commands.add_parser("seed", help="build or update the seed at a commit")
    seed.add_argument("--root", required=True, help="directory holding the seed")
    seed.add_argument("--like", help="a worktree path whose shape the seed checkout copies")
    seed.add_argument("--commit", default=REFRESH_COMMIT)
    seed.add_argument("--no-fetch", action="store_true")
    seed.add_argument("--repo", help="the repository the seed checks out (default: the seed's "
                      "own checkouts, or the one this tool is in)")
    seed.add_argument("--keep-incremental", action="store_true",
                      help="keep rustc's incremental caches, for faster updates at the cost of disk")
    seed.add_argument("--if-moved", action="store_true",
                      help="do nothing when the seed is at the commit already or another update runs")
    seed.add_argument("--reader-wait", type=int, default=15,
                      help="minutes to wait for warms still copying from the slot to rebuild")
    seed.add_argument("-j", "--jobs", type=int, default=8)
    warm = commands.add_parser("warm", help="seed this worktree's empty target directory")
    warm.add_argument("--root", required=True)
    warm.add_argument("--worktree", default=".")
    warm.add_argument("--target-dir", help="default: the directory cargo would use")
    warm.add_argument("--copy", choices=("copyfile2", "robocopy"), default="copyfile2")
    warm.add_argument("--target-roots", help="directories, separated by `;`, one of which (the "
                      "one with the most free space) holds the profile directory, behind a junction")
    warm.add_argument("--min-free-gb", type=int, default=60,
                      help="with --target-roots, warm only if this much stays free after the copy")
    warm.add_argument("--threads", type=int, default=16)
    warm.add_argument("--refresh-behind", action="store_true",
                      help="afterwards, refresh the seed in the background if it is behind origin/main")
    check = commands.add_parser("check", help="list references to the seed in a target directory")
    check.add_argument("--root", required=True)
    check.add_argument("--target-dir", required=True)
    check.add_argument("-j", "--jobs", type=int, default=8)
    args = parser.parse_args()
    try:
        return {"seed": command_seed, "warm": command_warm, "check": command_check}[args.command](args)
    except WarmError as error:
        print(f"{args.command}: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
