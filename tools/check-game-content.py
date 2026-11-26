"""Keep the game's and its mods' files out of the repository (Python 3.11+).

usage: python tools/check-game-content.py [--against REF] [--game DIR] [--user-dir DIR] [--cache DIR] [--tree-only]

The files checked are those Git tracks or would add (untracked and not ignored), as Boat sends them.

In the tree, on every machine:
- `asset`: a file of an asset or game-data type (images, textures, sound, music, fonts, video,
  models, animations, archives, game script and interface files, shaders, saves, recordings), or
  any binary file, that tools/game-content.allow does not list with where it came from. An entry
  that matches no file fails as `stale-allow`.
- `embedded`: a base64 run of 1,024 characters or more in a text file: an asset carried as text.

Against the installation, where one is found (--game or BATTLEPLAN_GAME_DIR, else the Steam
registry entry) with its mods (each installed workshop item, and the mods of --user-dir or
BATTLEPLAN_USER_DIR, else the Paradox user folder):
- `copy`: a file whose bytes equal a file of the game or a mod, including files on the allow list;
- `copied-text`: prose shared with the installation, or three consecutive substantial script
  lines shared in the same order. A single identifier or interface element name is allowed.
- `game-value`: `KEY = number` where KEY is one of the game's or a mod's defines and the number is
  a value they give it, other than 0, 1 and -1. A test needs a value, not the game's: use an
  obviously made-up one, or read the number from a fixture we wrote when the test is about it.
It reads the installation and writes nothing there. Its index lives outside the repository in
--cache (default: the user's cache folder) and is rebuilt per game or mod only when Steam's
manifest for it, or the size or time of a file of a mod outside Steam, changes; under `--against`
a missing or out-of-date index fails and names `python tools/check-game-content.py` to build or
refresh it, never rebuilding in the push check. Where no installation is found, or when
`--tree-only` is passed, it prints `not run here` and why, and reports only the tree checks.
`--against REF` (as `ci.py check` runs it) compares only the files changed since the merge base
with REF; without it the whole tree is compared.

Findings print as `<file>:<line>: <kind> <detail>` and never quote game text. Exits 1 on any.
"""
import argparse
from contextlib import closing
import hashlib
import os
import re
import sqlite3
import struct
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOW_NAME = "tools/game-content.allow"
ALLOW = ROOT / ALLOW_NAME
APP = "394360"
GAME_FOLDER = "Hearts of Iron IV"
INDEX_VERSION = 3

ASSET_TYPES = {
    "image": {".png", ".jpg", ".jpeg", ".gif", ".bmp", ".dds", ".tga", ".webp", ".ico", ".svg", ".psd", ".pdn", ".tif", ".tiff", ".cur", ".ani"},
    "sound": {".wav", ".ogg", ".mp3", ".flac", ".bank", ".fsb"},
    "font": {".ttf", ".otf", ".woff", ".woff2", ".fnt"},
    "video": {".mp4", ".webm", ".bk2", ".bik", ".avi", ".mov"},
    "model": {".mesh", ".anim", ".fbx"},
    "archive": {".zip", ".7z", ".rar", ".gz", ".xz", ".tar", ".pak"},
    "binary data": {".bin", ".dat"},
    "game data": {".gui", ".gfx", ".lua", ".yml", ".yaml", ".shader", ".fxh", ".fx", ".csv", ".mod", ".dlc", ".sfx", ".map", ".asset"},
    "save or recording": {".hoi4", ".h4s", ".sav", ".battleplan", ".spool"},
}
TYPE_OF = {extension: kind for kind, extensions in ASSET_TYPES.items() for extension in extensions}
BASE64_RUN = 1024
BASE64_ALPHABET = bytes(range(ord("A"), ord("Z") + 1)) + bytes(range(ord("a"), ord("z") + 1)) + b"0123456789+/"
FOLD_BASE64 = bytes(65 if byte in BASE64_ALPHABET else 32 for byte in range(256))

HEAD = 256
MIN_SEGMENT = 40
MIN_PROSE = 20
SCRIPT_MARKS = re.compile(r"[=<>{}\[\]_\\/;]")
SPACES = re.compile(r"\s+")
QUOTED = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
COMMENT = re.compile(r"(?://|#|--)(.*)$")
TEXT_TYPES = {".txt", ".yml", ".yaml", ".gui", ".gfx", ".lua", ".shader", ".fxh", ".fx", ".csv", ".asset", ".md", ".html",
              ".htm", ".json", ".mod", ".info", ".sfx", ".settings", ".xml", ".py", ".js", ".css", ".ini", ".fnt", ".hlsl"}
SKIPPED_DIRS = {".git", ".svn"}
DEFINE = re.compile(r"\b([A-Z][A-Z0-9_]{2,})\s*=\s*(-?(?:[0-9]+\.?[0-9]*|\.[0-9]+))(?![\w.])")
TRIVIAL_VALUES = {0.0, 1.0, -1.0}


def is_binary(data):
    """Not UTF-8 text: any texture, sound, model or archive, whatever its name."""
    if b"\0" in data:
        return True
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return True
    return False


def base64_run(data):
    """(line, length) of the first base64 run of BASE64_RUN characters or more, else None."""
    folded = data.translate(FOLD_BASE64)
    start = folded.find(b"A" * BASE64_RUN)
    if start < 0:
        return None
    end = folded.find(b" ", start)
    return data.count(b"\n", 0, start) + 1, (len(folded) if end < 0 else end) - start


TEXT_KINDS = ("copied-text", "game-value")


def allowed(text):
    """Each entry as (glob, kind, provenance): `<glob> <provenance>` allows an asset file, and
    `<glob> copied-text|game-value <provenance>` exempts a text file from that finding alone.
    Every nonblank line names a file and its provenance."""
    rules, errors = [], []
    for number, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line:
            continue
        glob, _, rest = line.partition(" ")
        kind, _, after = rest.strip().partition(" ")
        kind, provenance = (kind, after.strip()) if kind in TEXT_KINDS else (None, rest.strip())
        if not provenance:
            errors.append(f"{ALLOW_NAME}:{number}: allow entry gives no provenance")
        rules.append((glob, kind, provenance))
    return rules, errors


def glob_match(name, glob):
    """`*` matches within one folder, `**` across folders."""
    if "*" not in glob:
        return name == glob
    pattern = "".join(".*" if part == "**" else "[^/]*" if part == "*" else re.escape(part)
                      for part in re.split(r"(\*\*|\*)", glob))
    return re.fullmatch(pattern, name) is not None


def asset_kind(name, head):
    """What makes a file an asset or game data: its type, or binary content; None for ours-by-kind text."""
    kind = TYPE_OF.get(os.path.splitext(name)[1].lower())
    if kind:
        return kind
    return "binary" if is_binary(head) else None


def tree_findings(files, rules):
    """`asset`, `embedded` and `stale-allow` findings for (name, bytes) pairs: an entry that
    matches no file is stale."""
    found = offenders(files, rules)
    names = {name for name, _ in files}
    found += [f"{ALLOW_NAME}: stale-allow {glob}{f' {kind}' if kind else ''} matches no file"
              for glob, kind, _ in rules if (glob not in names and not ("*" in glob and any(glob_match(name, glob) for name in names)))]
    return found


def allow_match(name, rules, kind=None):
    """The first entry of `kind` (None: an asset entry) whose glob matches `name`."""
    return next((glob for glob, entry_kind, _ in rules if entry_kind == kind and glob_match(name, glob)), None)


def offenders(files, rules):
    """`asset` and `embedded` findings for (name, bytes) pairs that no asset entry allows."""
    found = []
    for name, data in files:
        kind = asset_kind(name, data)
        if kind and not allow_match(name, rules):
            found.append(f"{name}:1: asset {kind} file not in {ALLOW.name}")
            continue
        if kind and is_binary(data):
            continue
        run = base64_run(data)
        if run:
            found.append(f"{name}:{run[0]}: embedded base64 run of {run[1]} characters")
    return found


def allow_rules(root=ROOT):
    """The allow file's rules under `root`, or none when it is absent."""
    path = root / ALLOW_NAME
    return allowed(path.read_text(encoding="utf-8"))[0] if path.exists() else []


def fold(text):
    return SPACES.sub(" ", text.replace('\\"', '"').replace("\\\\", "\\")).strip()


def prose(text):
    """Words, not script: three or more, mostly letters, without script operators or key names."""
    return (len(text) >= MIN_PROSE and text.count(" ") >= 2 and not SCRIPT_MARKS.search(text)
            and sum(char.isalpha() for char in text) >= 0.6 * len(text))


def segments(line):
    parts = QUOTED.findall(line)
    if '\\"' in line:
        parts.extend(re.findall(r'\\"(.*?)\\"', line))
    comment = COMMENT.search(line)
    if comment:
        parts.append(comment.group(1))
    whole = fold(line)
    if len(whole) >= MIN_SEGMENT:
        parts.append(whole)
    return {part for part in map(fold, parts) if prose(part)}


def digest64(data):
    return struct.unpack("<q", hashlib.blake2b(data, digest_size=8).digest())[0]


def text_hashes(text):
    pairs = []
    previous = []
    for number, line in enumerate(text.splitlines(), 1):
        pairs.extend((digest64(part.encode()), number) for part in segments(line))
        whole = fold(line)
        if len(whole) < MIN_SEGMENT or " " not in whole:
            previous.clear()
            continue
        previous.append(whole)
        if len(previous) == 3:
            pairs.append((digest64("\n".join(previous).encode()), number - 2))
            previous.pop(0)
    return pairs


def define_values(text):
    """Each define key and the values `KEY = number` lines give it."""
    values = {}
    for line in text.splitlines():
        for match in DEFINE.finditer(line.partition("--")[0].partition("#")[0]):
            values.setdefault(match.group(1), set()).add(float(match.group(2)))
    return values


def game_values(text, installed):
    """(line, key) of each `KEY = number` giving a define key a value the installation gives it."""
    found = []
    for number, line in enumerate(text.split("\n"), 1):
        if "=" not in line:
            continue
        for match in DEFINE.finditer(line):
            key, value = match.group(1), float(match.group(2))
            if value not in TRIVIAL_VALUES and value in installed.get(key, ()):
                found.append((number, key))
    return found


def vdf_blocks(text):
    """A Valve key-value text file as nested dicts of strings."""
    stack, key = [{}], None
    for token in re.findall(r'"((?:[^"\\]|\\.)*)"|([{}])', text):
        value, brace = token
        if brace == "{":
            child = {}
            stack[-1][key] = child
            stack.append(child)
            key = None
        elif brace == "}":
            stack.pop()
        elif key is None:
            key = value
        else:
            stack[-1][key] = value
            key = None
    return stack[0]


def registry_value(hive, key, name):
    try:
        import winreg
    except ImportError:
        return None
    try:
        with winreg.OpenKey(getattr(winreg, hive), key) as handle:
            value, _ = winreg.QueryValueEx(handle, name)
            return os.path.expandvars(value) if isinstance(value, str) else None
    except OSError:
        return None


def locate(args):
    """(roots, reason): each (key, folder, stamp or None) to index, or no roots and why not."""
    game = args.game or os.environ.get("BATTLEPLAN_GAME_DIR")
    if not game:
        for key in (rf"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {APP}",
                    rf"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {APP}"):
            game = registry_value("HKEY_LOCAL_MACHINE", key, "InstallLocation")
            if game:
                break
    if not game:
        return [], "no game installation: neither --game, BATTLEPLAN_GAME_DIR nor the Steam registry entry names one"
    game = Path(game)
    if not game.is_dir():
        return [], f"the game folder named by --game, BATTLEPLAN_GAME_DIR or the registry does not exist"
    roots = []
    libraries = {game.parent.parent.parent}
    steam = registry_value("HKEY_CURRENT_USER", r"Software\Valve\Steam", "SteamPath")
    if steam:
        try:
            folders = vdf_blocks((Path(steam) / "steamapps" / "libraryfolders.vdf").read_text(encoding="utf-8", errors="replace"))
            libraries |= {Path(entry["path"]) for entry in folders.get("libraryfolders", {}).values()
                          if isinstance(entry, dict) and "path" in entry}
        except OSError:
            pass
    manifest = game.parent.parent / f"appmanifest_{APP}.acf"
    stamp = None
    if manifest.is_file():
        state = vdf_blocks(manifest.read_text(encoding="utf-8", errors="replace")).get("AppState", {})
        depots = state.get("InstalledDepots", {})
        stamp = state.get("buildid", "") + ";" + ";".join(f"{depot}={info.get('manifest', '')}" for depot, info in sorted(depots.items()))
    roots.append(("game", game, stamp))
    covered = set()
    for library in sorted(libraries):
        content = library / "steamapps" / "workshop" / "content" / APP
        if not content.is_dir():
            continue
        installed = {}
        try:
            acf = vdf_blocks((library / "steamapps" / "workshop" / f"appworkshop_{APP}.acf").read_text(encoding="utf-8", errors="replace"))
            installed = acf.get("AppWorkshop", {}).get("WorkshopItemsInstalled", {})
        except OSError:
            pass
        for item in sorted(content.iterdir()):
            if item.is_dir():
                manifest_id = installed.get(item.name, {}).get("manifest")
                roots.append((f"workshop item {item.name}", item, f"manifest {manifest_id}" if manifest_id else None))
                covered.add(os.path.normcase(str(item.resolve())))
    user = args.user_dir or os.environ.get("BATTLEPLAN_USER_DIR") or user_directory()
    if user and (Path(user) / "mod").is_dir():
        mods = Path(user) / "mod"
        for entry in sorted(mods.iterdir()):
            targets = [entry] if entry.is_dir() else []
            if entry.suffix == ".mod":
                text = entry.read_text(encoding="utf-8", errors="replace")
                path = re.search(r'^\s*path\s*=\s*"([^"]+)"', text, re.MULTILINE)
                if path:
                    target = Path(path.group(1))
                    targets.append(target if target.is_absolute() else Path(user) / target)
            for target in targets:
                folded = os.path.normcase(str(target.resolve()))
                if target.is_dir() and folded not in covered:
                    covered.add(folded)
                    roots.append((f"mod {target.name}", target, None))
    return roots, None


def user_directory():
    documents = registry_value("HKEY_CURRENT_USER", r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders", "Personal")
    candidates = [documents] + [os.path.join(os.environ[v], "Documents") for v in ("OneDrive", "USERPROFILE") if os.environ.get(v)]
    for documents in filter(None, candidates):
        folder = Path(documents) / "Paradox Interactive" / GAME_FOLDER
        if folder.is_dir():
            return folder
    return None


def walk(folder):
    """Each regular file under `folder` as (relative path, stat), skipping version-control folders."""
    for base, dirs, names in os.walk(folder):
        dirs[:] = [d for d in dirs if d not in SKIPPED_DIRS]
        for name in names:
            path = os.path.join(base, name)
            try:
                status = os.stat(path)
            except OSError:
                continue
            yield os.path.relpath(path, folder).replace(os.sep, "/"), status


class Index:
    """What the check needs of the game and its mods, kept in a SQLite file outside the repository."""

    def __init__(self, path, *, readonly=False):
        if readonly:
            self.db = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True)
            return
        path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(path)
        self.db.executescript("""
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS roots(id INTEGER PRIMARY KEY, key TEXT UNIQUE, folder TEXT, stamp TEXT);
            CREATE TABLE IF NOT EXISTS files(root INTEGER, path TEXT, size INTEGER, head INTEGER);
            CREATE INDEX IF NOT EXISTS files_by_head ON files(size, head);
            CREATE TABLE IF NOT EXISTS segments(hash INTEGER, root INTEGER, PRIMARY KEY(hash, root)) WITHOUT ROWID;
            CREATE TABLE IF NOT EXISTS defines(root INTEGER, key TEXT, value REAL);
        """)

    def current(self, key, folder, stamp):
        """(root id or None when the index is not of this stamp, full stamp, walked listing or None)."""
        listing = None
        if stamp is None:
            listing = sorted(walk(folder))
            stamp = "walk " + hashlib.sha256("\n".join(f"{p}\t{s.st_size}\t{s.st_mtime_ns}" for p, s in listing).encode()).hexdigest()
        stamp = f"{folder}|{stamp}"
        row = self.db.execute("SELECT id, stamp FROM roots WHERE key = ?", (key,)).fetchone()
        return (row[0] if row and row[1] == stamp else None), stamp, listing

    def refresh(self, key, folder, stamp, say):
        """Indexes one game or mod folder unless its stamp is the one indexed; returns its id."""
        root, stamp, listing = self.current(key, folder, stamp)
        if root is not None:
            return root
        row = self.db.execute("SELECT id FROM roots WHERE key = ?", (key,)).fetchone()
        started = time.monotonic()
        if row:
            root = row[0]
            for table in ("files", "segments", "defines"):
                self.db.execute(f"DELETE FROM {table} WHERE root = ?", (root,))
            self.db.execute("UPDATE roots SET folder = ?, stamp = NULL WHERE id = ?", (str(folder), root))
        else:
            root = self.db.execute("INSERT INTO roots(key, folder) VALUES (?, ?)", (key, str(folder))).lastrowid
        files, hashes, defines = [], set(), []
        for relative, status in listing if listing is not None else walk(folder):
            extension = os.path.splitext(relative)[1].lower()
            text = extension in TEXT_TYPES or extension == ""
            if not status.st_size:
                continue
            try:
                with open(os.path.join(folder, relative), "rb") as handle:
                    data = handle.read() if text else handle.read(HEAD)
            except OSError:
                continue
            files.append((root, relative, status.st_size, digest64(data[:HEAD])))
            if text and not is_binary(data):
                decoded = data.decode("utf-8", "replace")
                hashes.update(hash_ for hash_, _ in text_hashes(decoded))
                if extension == ".lua" and "/defines/" in "/" + relative:
                    defines += [(root, k, v) for k, values in define_values(decoded).items() for v in values]
        self.db.executemany("INSERT INTO files VALUES (?, ?, ?, ?)", files)
        self.db.executemany("INSERT OR IGNORE INTO segments VALUES (?, ?)", ((h, root) for h in hashes))
        self.db.executemany("INSERT INTO defines VALUES (?, ?, ?)", defines)
        self.db.execute("UPDATE roots SET stamp = ? WHERE id = ?", (stamp, root))
        self.db.commit()
        say(f"indexed {key}: {len(files)} files, {len(hashes)} text runs in {time.monotonic() - started:.0f} s")
        return root

    def drop_others(self, keep):
        gone = [row[0] for row in self.db.execute("SELECT id FROM roots") if row[0] not in keep]
        for root in gone:
            for table in ("files", "segments", "defines"):
                self.db.execute(f"DELETE FROM {table} WHERE root = ?", (root,))
            self.db.execute("DELETE FROM roots WHERE id = ?", (root,))
        self.db.commit()

def install_findings(index, files, rules, active):
    index.db.execute("CREATE TEMP TABLE IF NOT EXISTS active(id INTEGER PRIMARY KEY)")
    index.db.execute("DELETE FROM active")
    index.db.executemany("INSERT INTO active VALUES (?)", ((root,) for root in active))
    roots = {root: (key, Path(folder)) for root, key, folder in index.db.execute(
        "SELECT roots.id, key, folder FROM roots JOIN active USING(id)")}
    found = []
    installed = {}
    for key, value in index.db.execute("SELECT key, value FROM defines JOIN active ON active.id = defines.root"):
        installed.setdefault(key, set()).add(value)
    index.db.execute("CREATE TEMP TABLE IF NOT EXISTS probe(hash INTEGER, file INTEGER, line INTEGER)")
    index.db.execute("DELETE FROM probe")
    names, text_found = [], []
    for name, data in files:
        if data.strip():
            for root, path in index.db.execute("SELECT root, path FROM files JOIN active ON active.id = files.root WHERE size = ? AND head = ?", (len(data), digest64(data[:HEAD]))):
                try:
                    same = (roots[root][1] / path).read_bytes() == data
                except OSError:
                    same = False
                if same:
                    found.append(f"{name}:1: copy equals {roots[root][0]} file {path}")
        if is_binary(data):
            continue
        decoded = data.decode("utf-8")
        number = len(names)
        names.append(name)
        index.db.executemany("INSERT INTO probe VALUES (?, ?, ?)", ((h, number, line) for h, line in text_hashes(decoded)))
        text_found += [(name, line, "game-value", f"game-value {key} has the installation's value")
                       for line, key in game_values(decoded, installed)]
    hits = index.db.execute("SELECT DISTINCT probe.file, probe.line, segments.root FROM probe JOIN segments ON segments.hash = probe.hash JOIN active ON active.id = segments.root ORDER BY 1, 2")
    by_line = {}
    for number, line, root in hits:
        by_line.setdefault((names[number], line), []).append(roots[root][0])
    text_found += [(name, line, "copied-text", f"copied-text shares a run with {', '.join(sorted(set(where))[:3])}")
                   for (name, line), where in sorted(by_line.items())]
    found.extend(f"{name}:{line}: {detail}" for name, line, kind, detail in text_found if not allow_match(name, rules, kind))
    return found


def candidates():
    """Repository paths Git tracks or would add, existing as regular files."""
    listed = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT, capture_output=True, check=True)
    names = {n for n in listed.stdout.decode("utf-8").split("\0") if n}
    deleted = subprocess.run(["git", "ls-files", "-z", "--deleted"], cwd=ROOT, capture_output=True, check=True)
    return names - set(deleted.stdout.decode("utf-8").split("\0"))


def changed_since(ref):
    """Paths the working tree changes or adds against its merge base with `ref`."""
    git = lambda *args: subprocess.run(["git", *args], cwd=ROOT, capture_output=True, check=True).stdout
    base = git("merge-base", ref, "HEAD").decode().strip()
    names = git("diff", "--name-only", "-z", "--no-renames", "--diff-filter=d", base) + git("ls-files", "-z", "--others", "--exclude-standard")
    return {name for name in names.decode("utf-8").split("\0") if name}


def default_cache():
    base = os.environ.get("LOCALAPPDATA") or os.environ.get("XDG_CACHE_HOME") or os.path.join(os.path.expanduser("~"), ".cache")
    return Path(base) / "battleplan-tools" / "game-content"


def compare_installation(args, files, rules, say):
    roots, reason = locate(args)
    if not roots:
        say(f"not run here: installation comparison ({reason})")
        return []
    cache = ((args.cache or default_cache()) / f"index-v{INDEX_VERSION}.sqlite3").resolve()
    if cache.is_relative_to(ROOT) or any(cache.is_relative_to(folder.resolve()) for _, folder, _ in roots):
        return ["installation index must be outside the worktree, game and mod folders"]
    if args.against and not cache.is_file():
        detail = f"no index at {cache}; run python tools/check-game-content.py"
        say(f"not run here: installation comparison ({detail})")
        return [f"installation comparison: {detail}"]
    try:
        index = Index(cache, readonly=bool(args.against))
    except sqlite3.Error as error:
        if not args.against:
            raise
        detail = f"unreadable index at {cache}: {error}; run python tools/check-game-content.py"
        say(f"not run here: installation comparison ({detail})")
        return [f"installation comparison: {detail}"]
    with closing(index.db):
        if args.against:
            try:
                index.db.execute("BEGIN")
                current = {key: index.current(key, folder, stamp)[0] for key, folder, stamp in roots}
                stale = [key for key, root in current.items() if root is None]
                active = set(current.values()) - {None}
                findings = install_findings(index, files, rules, active) if active else []
            except sqlite3.Error as error:
                detail = f"unreadable index at {cache}: {error}; run python tools/check-game-content.py"
                say(f"not run here: installation comparison ({detail})")
                return [f"installation comparison: {detail}"]
            if stale:
                missing = ", ".join(stale)
                say(f"not run here: installation comparison for {missing} (index out of date; run python tools/check-game-content.py)")
                findings.append(f"installation comparison: index out of date for {missing}; run python tools/check-game-content.py")
        else:
            active = {index.refresh(key, folder, stamp, say) for key, folder, stamp in roots}
            index.drop_others(active)
            findings = install_findings(index, files, rules, active) if active else []
    say(f"installation comparison: {len(active)} game and mod folders, {len(files)} files"
        + (f" changed since {args.against}" if args.against else ""))
    return findings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--game", help="the game's folder (default: BATTLEPLAN_GAME_DIR, else the Steam registry entry)")
    parser.add_argument("--user-dir", help="the Paradox user folder whose mods to read (default: BATTLEPLAN_USER_DIR, else Documents)")
    parser.add_argument("--cache", type=Path, help="where the index lives (default: the user's cache folder)")
    parser.add_argument("--tree-only", action="store_true", help="skip the comparison with the installation")
    parser.add_argument("--against", metavar="REF", help="compare with the installation only the files changed since the merge base with REF")
    args = parser.parse_args(argv)
    started = time.monotonic()
    say = lambda text: print(text, flush=True)
    rules, found = allowed(ALLOW.read_text(encoding="utf-8")) if ALLOW.exists() else ([], [])
    names = candidates()
    selected = names & changed_since(args.against) if args.against else names
    files = [(name, (ROOT / name).read_bytes()) for name in sorted(names)]
    found += tree_findings(files, rules)
    say(f"tree: {len(files)} files checked for assets and embedded data")
    if args.tree_only:
        say("not run here: installation comparison (--tree-only)")
    else:
        found += compare_installation(args, [(name, data) for name, data in files if name in selected], rules, say)
    for line in found:
        print(line)
    say(f"{len(found)} finding(s) in {time.monotonic() - started:.1f} s")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
