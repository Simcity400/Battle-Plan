"""Keep the game's and its mods' files out of the repository (Python 3.11+).

usage: python tools/check-game-content.py [--game DIR] [--user-dir DIR] [--cache DIR] [--tree-only]

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
- `copy`: a file whose bytes equal a file of the game or a mod;
- `copied-text`: a line, quoted string or comment of 40 or more characters, spaces folded, that a
  text file of the game or a mod also has;
- `game-value`: `KEY = number` where KEY is one of the game's or a mod's defines and the number is
  a value they give it, other than 0, 1 and -1. A test needs a value, not the game's: use an
  obviously made-up one, or read the number from a fixture we wrote when the test is about it.
It reads the installation and writes nothing there. Its index lives outside the repository in
--cache (default: the user's cache folder) and is rebuilt per game or mod only when Steam's
manifest for it, or the size or time of a file of a mod outside Steam, changes. Where no
installation is found it prints `not run here` and why, and reports only the tree checks.

Findings print as `<file>:<line>: <kind> <detail>` and never quote game text. Exits 1 on any.
"""
import argparse
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
INDEX_VERSION = 1

ASSET_TYPES = {
    "image": {".png", ".jpg", ".jpeg", ".gif", ".bmp", ".dds", ".tga", ".webp", ".ico", ".svg", ".psd", ".pdn", ".tif", ".tiff", ".cur", ".ani"},
    "sound": {".wav", ".ogg", ".mp3", ".flac", ".bank", ".fsb", ".asset"},
    "font": {".ttf", ".otf", ".woff", ".woff2", ".fnt"},
    "video": {".mp4", ".webm", ".bk2", ".bik", ".avi", ".mov"},
    "model": {".mesh", ".anim", ".fbx"},
    "archive": {".zip", ".7z", ".rar", ".gz", ".xz", ".tar", ".pak"},
    "binary data": {".bin", ".dat"},
    "game data": {".gui", ".gfx", ".lua", ".yml", ".yaml", ".shader", ".fxh", ".fx", ".csv", ".mod", ".dlc", ".sfx", ".map"},
    "save or recording": {".hoi4", ".h4s", ".sav", ".battleplan", ".spool"},
}
TYPE_OF = {extension: kind for kind, extensions in ASSET_TYPES.items() for extension in extensions}
BASE64_RUN = 1024
BASE64_ALPHABET = bytes(range(ord("A"), ord("Z") + 1)) + bytes(range(ord("a"), ord("z") + 1)) + b"0123456789+/"
FOLD_BASE64 = bytes(65 if byte in BASE64_ALPHABET else 32 for byte in range(256))

MIN_FILE = 64
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
    if data.count(b"\0") > 8:
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


def allowed(text):
    """The (glob, provenance) of each entry, ignoring blank and `#` lines; an entry needs both."""
    rules, errors = [], []
    for number, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        glob, _, provenance = line.partition(" ")
        if not provenance.strip():
            errors.append(f"{ALLOW_NAME}:{number}: allow entry gives no provenance")
        rules.append((glob, provenance.strip()))
    return rules, errors


def glob_match(name, glob):
    """`*` matches within one folder, `**` across folders."""
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
    """`asset`, `embedded` and `stale-allow` findings for (name, bytes) pairs."""
    used = set()
    found = offenders(files, rules, used)
    found += [f"{ALLOW_NAME}: stale-allow {glob} matches no file" for glob, _ in rules if glob not in used]
    return found


def allow_match(name, rules):
    return next((glob for glob, _ in rules if glob_match(name, glob)), None)


def offenders(files, rules, used=None):
    """`asset` and `embedded` findings for (name, bytes) pairs not on the allow list; adds each
    allow glob used to `used`."""
    found = []
    used = set() if used is None else used
    for name, data in files:
        match = allow_match(name, rules)
        if match:
            used.add(match)
            continue
        kind = asset_kind(name, data)
        if kind:
            found.append(f"{name}:1: asset {kind} file not in {ALLOW.name}")
            continue
        run = base64_run(data)
        if run:
            found.append(f"{name}:{run[0]}: embedded base64 run of {run[1]} characters")
    return found


def allow_rules(root=ROOT):
    """The allow file's rules under `root`, or none when it is absent."""
    path = root / "tools" / ALLOW.name
    return allowed(path.read_text(encoding="utf-8"))[0] if path.exists() else []


def fold(text):
    return SPACES.sub(" ", text.replace('\\"', '"').replace("\\\\", "\\")).strip()


def prose(text):
    """Words, not script: three or more, mostly letters, without script operators or key names."""
    return (len(text) >= MIN_PROSE and text.count(" ") >= 2 and not SCRIPT_MARKS.search(text)
            and sum(char.isalpha() for char in text) >= 0.6 * len(text))


def segments(line):
    """The folded line of at least MIN_SEGMENT characters, and each quoted string and the comment
    of at least MIN_SEGMENT, or MIN_PROSE when it is prose: in-game text sits in strings and comments."""
    whole = fold(line)
    found = {whole} if len(whole) >= MIN_SEGMENT else set()
    parts = QUOTED.findall(line)
    comment = COMMENT.search(line)
    if comment:
        parts.append(comment.group(1))
    for part in map(fold, parts):
        if len(part) >= MIN_SEGMENT or prose(part):
            found.add(part)
    return found


def digest64(data):
    return struct.unpack("<q", hashlib.blake2b(data, digest_size=8).digest())[0]


def text_hashes(text):
    """(hash, line) of each segment of each line."""
    return [(digest64(segment.encode()), number)
            for number, line in enumerate(text.split("\n"), 1) for segment in segments(line)]


def define_values(text):
    """Each define key and the values `KEY = number` lines give it."""
    values = {}
    for line in text.splitlines():
        for match in DEFINE.finditer(line.split("#", 1)[0]):
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

    def __init__(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(path)
        self.db.executescript(f"""
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS meta(version INTEGER);
            CREATE TABLE IF NOT EXISTS roots(id INTEGER PRIMARY KEY, key TEXT UNIQUE, folder TEXT, stamp TEXT);
            CREATE TABLE IF NOT EXISTS files(root INTEGER, path TEXT, size INTEGER, head INTEGER);
            CREATE INDEX IF NOT EXISTS files_by_head ON files(size, head);
            CREATE TABLE IF NOT EXISTS segments(hash INTEGER, root INTEGER, PRIMARY KEY(hash, root)) WITHOUT ROWID;
            CREATE TABLE IF NOT EXISTS defines(root INTEGER, key TEXT, value REAL);
            CREATE TABLE IF NOT EXISTS tree(path TEXT PRIMARY KEY, size INTEGER, mtime INTEGER, hashes BLOB, lines BLOB);
        """)
        version = self.db.execute("SELECT version FROM meta").fetchone()
        if version is None or version[0] != INDEX_VERSION:
            self.db.executescript("DELETE FROM roots; DELETE FROM files; DELETE FROM segments; DELETE FROM defines; DELETE FROM tree; DELETE FROM meta;")
            self.db.execute("INSERT INTO meta VALUES (?)", (INDEX_VERSION,))
            self.db.commit()

    def refresh(self, key, folder, stamp, say):
        """Indexes one game or mod folder unless its stamp is the one indexed; returns its id."""
        listing = None
        if stamp is None:
            listing = sorted(walk(folder))
            stamp = "walk " + hashlib.sha256("\n".join(f"{p}\t{s.st_size}\t{s.st_mtime_ns}" for p, s in listing).encode()).hexdigest()
        stamp = f"{folder}|{stamp}"
        row = self.db.execute("SELECT id, stamp FROM roots WHERE key = ?", (key,)).fetchone()
        if row and row[1] == stamp:
            return row[0]
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
            if status.st_size < MIN_FILE and not text:
                continue
            try:
                with open(os.path.join(folder, relative), "rb") as handle:
                    data = handle.read() if text else handle.read(HEAD)
            except OSError:
                continue
            if status.st_size >= MIN_FILE:
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

    def tree_hashes(self, name, path, data):
        """(hash, line) pairs of a tree text file, reused while its size and time stay put."""
        status = path.stat()
        row = self.db.execute("SELECT size, mtime, hashes, lines FROM tree WHERE path = ?", (name,)).fetchone()
        if row and row[0] == status.st_size and row[1] == status.st_mtime_ns:
            count = len(row[2]) // 8
            return list(zip(struct.unpack(f"<{count}q", row[2]), struct.unpack(f"<{count}I", row[3])))
        pairs = text_hashes(data.decode("utf-8", "replace"))
        self.db.execute("INSERT OR REPLACE INTO tree VALUES (?, ?, ?, ?, ?)",
                        (name, status.st_size, status.st_mtime_ns, struct.pack(f"<{len(pairs)}q", *(h for h, _ in pairs)),
                         struct.pack(f"<{len(pairs)}I", *(n for _, n in pairs))))
        return pairs


def install_findings(index, files, rules):
    """`copy`, `copied-text` and `game-value` findings for the tree's (name, bytes) pairs. A file on
    the allow list may share text (a licence mods also ship) but may not equal a game file."""
    keys = {row[0]: row[1] for row in index.db.execute("SELECT id, key FROM roots")}
    folders = {row[0]: Path(row[1]) for row in index.db.execute("SELECT id, folder FROM roots")}
    found = []
    installed = {}
    for key, value in index.db.execute("SELECT key, value FROM defines"):
        installed.setdefault(key, set()).add(value)
    index.db.execute("CREATE TEMP TABLE IF NOT EXISTS probe(hash INTEGER, file INTEGER, line INTEGER)")
    index.db.execute("DELETE FROM probe")
    names = []
    for name, data in files:
        if len(data) >= MIN_FILE:
            for root, path in index.db.execute("SELECT root, path FROM files WHERE size = ? AND head = ?", (len(data), digest64(data[:HEAD]))):
                try:
                    same = (folders[root] / path).read_bytes() == data
                except OSError:
                    same = False
                if same:
                    found.append(f"{name}:1: copy equals {keys[root]} file {path}")
        if is_binary(data) or allow_match(name, rules):
            continue
        number = len(names)
        names.append(name)
        index.db.executemany("INSERT INTO probe VALUES (?, ?, ?)", ((h, number, line) for h, line in index.tree_hashes(name, ROOT / name, data)))
        found += [f"{name}:{line}: game-value {key} has the installation's value"
                  for line, key in game_values(data.decode("utf-8", "replace"), installed)]
    index.db.commit()
    hits = index.db.execute("SELECT DISTINCT probe.file, probe.line, segments.root FROM probe JOIN segments ON segments.hash = probe.hash ORDER BY 1, 2")
    by_line = {}
    for number, line, root in hits:
        by_line.setdefault((names[number], line), []).append(keys[root])
    found += [f"{name}:{line}: copied-text shares a run with {', '.join(sorted(set(where))[:3])}" for (name, line), where in sorted(by_line.items())]
    return found


def candidates():
    """Repository paths Git tracks or would add, existing as regular files."""
    listed = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT, capture_output=True, check=True)
    names = sorted({n for n in listed.stdout.decode("utf-8").split("\0") if n})
    return [name for name in names if (ROOT / name).is_file()]


def default_cache():
    base = os.environ.get("LOCALAPPDATA") or os.environ.get("XDG_CACHE_HOME") or os.path.join(os.path.expanduser("~"), ".cache")
    return Path(base) / "battleplan-tools" / "game-content"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--game", help="the game's folder (default: BATTLEPLAN_GAME_DIR, else the Steam registry entry)")
    parser.add_argument("--user-dir", help="the Paradox user folder whose mods to read (default: BATTLEPLAN_USER_DIR, else Documents)")
    parser.add_argument("--cache", type=Path, help="where the index lives (default: the user's cache folder)")
    parser.add_argument("--tree-only", action="store_true", help="skip the comparison with the installation")
    args = parser.parse_args(argv)
    started = time.monotonic()
    say = lambda text: print(text, flush=True)
    rules, found = allowed(ALLOW.read_text(encoding="utf-8")) if ALLOW.exists() else ([], [])
    files = [(name, (ROOT / name).read_bytes()) for name in candidates()]
    found += tree_findings(files, rules)
    say(f"tree: {len(files)} files checked for assets and embedded data")
    if args.tree_only:
        say("not run here: installation comparison (--tree-only)")
    else:
        roots, reason = locate(args)
        if not roots:
            say(f"not run here: installation comparison ({reason})")
        else:
            index = Index((args.cache or default_cache()) / "index.sqlite3")
            kept = {index.refresh(key, folder, stamp, say) for key, folder, stamp in roots}
            index.drop_others(kept)
            found += install_findings(index, files, rules)
            say(f"installation comparison: {len(roots)} game and mod folders, {len(files)} files")
    for line in found:
        print(line)
    say(f"{len(found)} finding(s) in {time.monotonic() - started:.1f} s")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
