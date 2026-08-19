#!/usr/bin/env python3
"""the game save-file "monthly oracle".

Parses a the game save (Clausewitz / Paradox key=value + nested {})
into clean JSON: authoritative per-state owner, per-province controller, and a
cheap per-country snapshot. This is the cross-check / ground-truth layer
("Architecture A") for the Battle Plan tool.

- stdlib only. subprocess is used solely to invoke `rakaly` for melting binary
  saves; plaintext saves are parsed directly.
- the game binary saves begin with "HOI4bin" and must be melted to plaintext
  (begins with "HOI4txt") first.
- The personal save-games path is NOT hardcoded: it is discovered from
  Path.home() / known Documents locations, or taken as a CLI argument. The
  resolved path is printed at runtime.

Usage:
    python parse_save.py [save_or_melted_path]
    python parse_save.py --rakaly /path/to/rakaly.exe [save_path]
    python parse_save.py --out-dir <dir> [save_path]

If no path is given, the most recently modified candidate save (*.hoi4 in the
known save-games dirs, plus *_melted.txt in Downloads) is parsed.

Design reviewed with Codex: quote-aware streaming tokenizer, depth-zero section
recognition, section-specific child parsers, balanced skip for everything else.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ENCODING = "latin1"

FACTORY_BUILDINGS = {
    "industrial_complex": "civ",
    "arms_factory": "mil",
    "dockyard": "dockyard",
}



LBRACE = "{"
RBRACE = "}"
EQUALS = "="


class Tokenizer:
    """Streaming, quote-aware tokenizer over the whole save text.

    Produces a flat stream of tokens: '{', '}', '=', or ('atom', value).
    Quotes and braces inside quoted strings are treated as literal text, so
    brace depth is never thrown off by names like "A{B" or escaped quotes.
    """

    __slots__ = ("s", "n", "i", "_peeked")

    def __init__(self, text: str):
        self.s = text
        self.n = len(text)
        self.i = 0
        self._peeked = None

    def _skip_ws_and_comments(self) -> None:
        s, n = self.s, self.n
        i = self.i
        while i < n:
            c = s[i]
            if c in " \t\r\n":
                i += 1
            elif c == "#":
                while i < n and s[i] != "\n":
                    i += 1
            else:
                break
        self.i = i

    def _read_token(self):
        self._skip_ws_and_comments()
        s, n = self.s, self.n
        i = self.i
        if i >= n:
            return None
        c = s[i]
        if c == "{":
            self.i = i + 1
            return LBRACE
        if c == "}":
            self.i = i + 1
            return RBRACE
        if c == "=":
            self.i = i + 1
            return EQUALS
        if c == '"':
            i += 1
            buf = []
            while i < n:
                ch = s[i]
                if ch == "\\" and i + 1 < n:
                    nxt = s[i + 1]
                    if nxt == '"':
                        buf.append('"')
                    elif nxt == "\\":
                        buf.append("\\")
                    elif nxt == "n":
                        buf.append("\n")
                    elif nxt == "t":
                        buf.append("\t")
                    else:
                        buf.append(nxt)
                    i += 2
                    continue
                if ch == '"':
                    i += 1
                    break
                buf.append(ch)
                i += 1
            self.i = i
            return ("atom", "".join(buf))
        start = i
        while i < n:
            ch = s[i]
            if ch in " \t\r\n{}=\"#":
                break
            i += 1
        self.i = i
        return ("atom", s[start:i])

    def next(self):
        if self._peeked is not None:
            tok = self._peeked
            self._peeked = None
            return tok
        return self._read_token()

    def peek(self):
        if self._peeked is None:
            self._peeked = self._read_token()
        return self._peeked


def atom_value(tok):
    """Return the string value of an atom token, else None."""
    if isinstance(tok, tuple) and tok[0] == "atom":
        return tok[1]
    return None



def skip_value(tz: Tokenizer) -> None:
    """Consume exactly one value: either a single atom or a balanced block.

    Call this positioned just AFTER an '=' (or to discard a stray atom). It
    leaves the cursor right after the value it consumed.
    """
    tok = tz.next()
    if tok is None:
        return
    if tok is LBRACE:
        _skip_block_body(tz)
        return


def _skip_block_body(tz: Tokenizer) -> None:
    """Consume tokens until the matching RBRACE (the opening LBRACE already eaten)."""
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            return
        if tok is LBRACE:
            depth += 1
        elif tok is RBRACE:
            depth -= 1



def iter_block_entries(tz: Tokenizer):
    """Yield (key, opener) for each entry in the block whose '{' was just consumed.

    `opener` is one of:
      - LBRACE  -> the value is a {...} block; caller must consume it
                   (parse it, or call _skip_block_body to discard).
      - ('atom', v) -> the value is a scalar v already read.
      - None    -> there was no '=' after key (key was itself a bare list item);
                   nothing was consumed past the key.

    Stops (and returns) when the matching RBRACE for this block is reached.
    """
    while True:
        tok = tz.next()
        if tok is None or tok is RBRACE:
            return
        if tok is LBRACE:
            _skip_block_body(tz)
            continue
        key = atom_value(tok)
        if key is None:
            continue
        nxt = tz.peek()
        if nxt is EQUALS:
            tz.next()
            vpeek = tz.peek()
            if vpeek is RBRACE:
                tz.next()
                return
            val = tz.next()
            if val is LBRACE:
                yield key, LBRACE
            else:
                yield key, val
        else:
            yield key, None



def parse_states_section(tz: Tokenizer, result: dict) -> None:
    """Parse states={ <id>={ owner=.. controller=.. buildings={..} .. } ... }."""
    states = result["states"]
    for state_id, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        info = {"owner": None, "controller": None,
                "victory_points": 0,
                "factories": {"civ": 0, "mil": 0, "dockyard": 0}}
        _parse_state_block(tz, info)
        states[state_id] = info


def _parse_state_block(tz: Tokenizer, info: dict) -> None:
    for key, opener in iter_block_entries(tz):
        if key == "owner" and opener not in (LBRACE, None):
            info["owner"] = atom_value(opener)
        elif key == "controller" and opener not in (LBRACE, None):
            info["controller"] = atom_value(opener)
        elif key == "buildings" and opener is LBRACE:
            _parse_state_buildings(tz, info["factories"])
        elif key == "victory_points" and opener not in (LBRACE, None):
            v = atom_value(opener)
            try:
                info["victory_points"] = int(float(v)) if v is not None else 0
            except (TypeError, ValueError):
                pass
        elif opener is LBRACE:
            _skip_block_body(tz)


def _parse_state_buildings(tz: Tokenizer, factories: dict) -> None:
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            bucket = FACTORY_BUILDINGS.get(key)
            if bucket is not None:
                lvl = _read_building_level(tz)
                factories[bucket] += lvl
            else:
                _skip_block_body(tz)


def _read_building_level(tz: Tokenizer) -> int:
    """Read a building block (LBRACE consumed) and return its `level` (int)."""
    level = 0
    for key, opener in iter_block_entries(tz):
        if key == "level" and opener not in (LBRACE, None):
            v = atom_value(opener)
            try:
                level = int(float(v))
            except (TypeError, ValueError):
                level = 0
        elif opener is LBRACE:
            _skip_block_body(tz)
    return level


def parse_provinces_section(tz: Tokenizer, result: dict) -> None:
    """Parse provinces={ <pid>={ controller="TAG" / buildings={..} } ... }."""
    prov = result["provinces"]
    for pid, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        controller = _parse_province_block(tz)
        if controller is not None:
            prov[pid] = controller


def _parse_province_block(tz: Tokenizer):
    controller = None
    for key, opener in iter_block_entries(tz):
        if key == "controller" and opener not in (LBRACE, None):
            controller = atom_value(opener)
        elif opener is LBRACE:
            _skip_block_body(tz)
    return controller


def parse_countries_section(tz: Tokenizer, result: dict) -> None:
    """Parse countries={ <TAG>={ ...huge... } ... } extracting cheap scalars."""
    countries = result["countries"]
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        snap = {
            "stability": None,
            "war_support": None,
            "political_power": None,
            "command_power": None,
            "ruling_party": None,
            "capital": None,
            "original_capital": None,
            "major": False,
            "manpower_ratio": None,
            "fuel": None,
            "max_fuel": None,
            "num_ships": None,
        }
        _parse_country_block(tz, snap)
        countries[tag] = snap


def _to_float(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def _to_int(v):
    f = _to_float(v)
    return int(f) if f is not None else None


def _parse_country_block(tz: Tokenizer, snap: dict) -> None:
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "politics":
                _parse_politics(tz, snap)
            elif key == "manpower":
                snap["manpower_ratio"] = _read_scalar_field(tz, "ratio", _to_int)
            elif key == "fuel_status":
                _parse_fuel_status(tz, snap)
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(opener)
        if val is None:
            continue
        if key == "stability":
            snap["stability"] = _to_float(val)
        elif key == "war_support":
            snap["war_support"] = _to_float(val)
        elif key == "command_power":
            snap["command_power"] = _to_float(val)
        elif key == "capital":
            snap["capital"] = _to_int(val)
        elif key == "original_capital":
            snap["original_capital"] = _to_int(val)
        elif key == "major":
            snap["major"] = (val == "yes")
        elif key == "num_ships":
            snap["num_ships"] = _to_int(val)


def _parse_politics(tz: Tokenizer, snap: dict) -> None:
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        val = atom_value(opener)
        if val is None:
            continue
        if key == "political_power":
            snap["political_power"] = _to_float(val)
        elif key == "ruling_party":
            snap["ruling_party"] = val


def _parse_fuel_status(tz: Tokenizer, snap: dict) -> None:
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        val = atom_value(opener)
        if val is None:
            continue
        if key == "fuel":
            snap["fuel"] = _to_float(val)
        elif key == "max_fuel":
            snap["max_fuel"] = _to_float(val)


def _read_scalar_field(tz: Tokenizer, field: str, conv):
    """Inside a block whose '{' was just consumed, return the value of `field`."""
    out = None
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
        elif key == field:
            out = conv(atom_value(opener))
    return out



TOPLEVEL_SCALARS = {"date", "player", "ideology", "difficulty", "version"}


def parse_save_text(text: str) -> dict:
    """Parse a melted/plaintext the game save string into the oracle result dict."""
    tz = Tokenizer(text)
    result = {
        "meta": {"date": None, "player": None, "version": None,
                 "ideology": None, "difficulty": None,
                 "player_countries": []},
        "states": {},
        "provinces": {},
        "countries": {},
    }

    while True:
        tok = tz.next()
        if tok is None:
            break
        if tok is RBRACE:
            continue
        if tok is LBRACE:
            _skip_block_body(tz)
            continue
        key = atom_value(tok)
        if key is None:
            continue
        nxt = tz.peek()
        if nxt is not EQUALS:
            continue
        tz.next()
        val = tz.next()
        if val is LBRACE:
            if key == "states":
                parse_states_section(tz, result)
            elif key == "provinces":
                parse_provinces_section(tz, result)
            elif key == "countries":
                parse_countries_section(tz, result)
            elif key == "player_countries":
                result["meta"]["player_countries"] = _read_atom_list(tz)
            else:
                _skip_block_body(tz)
        else:
            sval = atom_value(val)
            if key in TOPLEVEL_SCALARS and sval is not None:
                result["meta"][key] = sval

    return result


def _read_atom_list(tz: Tokenizer):
    """Read a {a b c ...} list (LBRACE consumed); return list of atom strings."""
    out = []
    while True:
        tok = tz.next()
        if tok is None or tok is RBRACE:
            break
        if tok is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(tok)
        if v is not None and v != EQUALS:
            out.append(v)
    return out



def candidate_save_dirs() -> list[Path]:
    """Known the game save locations, discovered relative to the user's home.

    No personal absolute path is hardcoded; everything derives from Path.home()
    (or BATTLE_PLAN_SAVE_DIR env override).
    """
    dirs: list[Path] = []
    env = os.environ.get("BATTLE_PLAN_SAVE_DIR", os.environ.get("HOI4_SAVE_DIR"))
    if env:
        dirs.append(Path(env))
    home = Path.home()
    rel = Path("Paradox Interactive") / "Hearts of Iron IV" / "save games"
    for docs in ("OneDrive/Documents", "Documents"):
        dirs.append(home / docs / rel)
    dirs.append(home / "Downloads")
    seen, out = set(), []
    for d in dirs:
        try:
            rp = d.resolve()
        except OSError:
            rp = d
        if rp in seen:
            continue
        seen.add(rp)
        if d.is_dir():
            out.append(d)
    return out


def find_latest_save() -> Path | None:
    """Most-recently-modified candidate across all known dirs.

    Considers *.hoi4 saves and *_melted.txt plaintext dumps.
    """
    candidates: list[Path] = []
    for d in candidate_save_dirs():
        try:
            for p in d.iterdir():
                name = p.name.lower()
                if name.endswith(".hoi4") or name.endswith("_melted.txt"):
                    if p.is_file():
                        candidates.append(p)
        except OSError:
            continue
    if not candidates:
        return None
    return max(candidates, key=lambda p: p.stat().st_mtime)


def find_rakaly(explicit: str | None) -> Path | None:
    """Locate the rakaly executable (PATH, ~/.cargo/bin, Downloads, etc.)."""
    if explicit:
        p = Path(explicit)
        return p if p.exists() else None
    which = shutil.which("rakaly") or shutil.which("rakaly.exe")
    if which:
        return Path(which)
    home = Path.home()
    guesses = [
        home / ".cargo" / "bin" / "rakaly",
        home / ".cargo" / "bin" / "rakaly.exe",
        home / "AppData" / "Local" / "rakaly" / "rakaly.exe",
    ]
    guesses += list((home / "Downloads").glob("**/rakaly.exe"))
    for g in guesses:
        if g.exists():
            return g
    return None


def read_save_header(path: Path) -> bytes:
    with open(path, "rb") as fh:
        return fh.read(16)


def melt_if_needed(path: Path, rakaly: Path | None, out_dir: Path):
    """Return (text, was_melted). Melts binary saves via rakaly to a temp file.

    Raises RuntimeError if a binary save is found but rakaly is unavailable.
    """
    header = read_save_header(path)
    if header.startswith(b"HOI4txt"):
        return path.read_text(encoding=ENCODING, errors="replace"), False
    if header.startswith(b"HOI4bin"):
        if rakaly is None:
            raise RuntimeError(
                f"{path.name} is a BINARY save (HOI4bin) but rakaly was not "
                f"found. Install rakaly or pass --rakaly <path>, or supply an "
                f"already-melted plaintext save."
            )
        tmp = Path(tempfile.gettempdir()) / (path.stem + "_melted.txt")
        with open(tmp, "wb") as out_fh:
            proc = subprocess.run(
                [str(rakaly), "melt", "--format", "the game",
                 "--to-stdout", "-u", "stringify", str(path)],
                stdout=out_fh, stderr=subprocess.PIPE,
            )
        header = b""
        try:
            with open(tmp, "rb") as fh:
                header = fh.read(8)
        except OSError:
            pass
        size = tmp.stat().st_size if tmp.exists() else 0
        if header.startswith(b"HOI4txt") and size > 1024:
            if proc.returncode != 0:
                warn = _scrub(proc.stderr.decode("utf-8", "replace").strip())
                print(f"[oracle] note: rakaly exited {proc.returncode} with a "
                      f"warning but produced valid plaintext ({warn[:120]})")
            return tmp.read_text(encoding=ENCODING, errors="replace"), True
        raise RuntimeError(
            f"rakaly melt failed (exit {proc.returncode}, "
            f"{size} bytes out): "
            f"{_scrub(proc.stderr.decode('utf-8', 'replace'))[:500]}"
        )
    return path.read_text(encoding=ENCODING, errors="replace"), False



def build_outputs(result: dict, source: Path, was_melted: bool):
    states = result["states"]
    provinces = result["provinces"]
    countries = result["countries"]

    state_ownership = {}
    for sid, info in states.items():
        state_ownership[sid] = {
            "owner": info.get("owner"),
            "controller": info.get("controller") or info.get("owner"),
        }

    factory_totals: dict[str, dict] = {}
    for sid, info in states.items():
        owner = info.get("owner")
        if not owner:
            continue
        f = info.get("factories", {})
        agg = factory_totals.setdefault(
            owner, {"civ": 0, "mil": 0, "dockyard": 0})
        agg["civ"] += f.get("civ", 0)
        agg["mil"] += f.get("mil", 0)
        agg["dockyard"] += f.get("dockyard", 0)

    country_snapshot = {}
    for tag, snap in countries.items():
        merged = dict(snap)
        merged["factories"] = factory_totals.get(
            tag, {"civ": 0, "mil": 0, "dockyard": 0})
        merged["is_human"] = tag in result["meta"]["player_countries"]
        country_snapshot[tag] = merged
    for tag, totals in factory_totals.items():
        if tag not in country_snapshot:
            country_snapshot[tag] = {
                "factories": totals,
                "is_human": tag in result["meta"]["player_countries"],
                "_no_country_block": True,
            }

    owned = [s for s in state_ownership.values() if s["owner"]]
    owner_tags = sorted({s["owner"] for s in owned})

    meta = {
        "source_filename": source.name,
        "source_path_redacted": _redact(source),
        "was_melted": was_melted,
        "save_date": result["meta"].get("date"),
        "save_version": result["meta"].get("version"),
        "player": result["meta"].get("player"),
        "player_countries": result["meta"].get("player_countries", []),
        "stats": {
            "states_parsed": len(states),
            "states_with_owner": len(owned),
            "distinct_owner_tags": len(owner_tags),
            "provinces_with_controller": len(provinces),
            "countries_parsed": len(countries),
            "countries_in_snapshot": len(country_snapshot),
        },
    }
    return state_ownership, country_snapshot, meta


def _redact(path: Path) -> str:
    """A home-relative, user-name-free representation of the source path."""
    try:
        rel = path.relative_to(Path.home())
        return str(Path("~") / rel).replace("\\", "/")
    except ValueError:
        return path.name


def _scrub(text: str) -> str:
    """Replace the user's home dir in arbitrary text with '~'.

    Used to sanitize anything user-visible (console prints, rakaly stderr) so
    the personal `C:\\Users\\Example path never appears in output that might be
    pasted or logged. Handles both slash styles.
    """
    home = str(Path.home())
    out = text.replace(home, "~").replace(home.replace("\\", "/"), "~")
    return out


def validation_report(state_ownership, country_snapshot, meta) -> str:
    st = meta["stats"]
    lines = []
    lines.append("=== SAVE ORACLE VALIDATION ===")
    lines.append(f"source        : {meta['source_filename']} "
                 f"(melted={meta['was_melted']})")
    lines.append(f"save date     : {meta['save_date']}  "
                 f"version={meta['save_version']}")
    lines.append(f"player        : {meta['player']}  "
                 f"humans={meta['player_countries']}")
    lines.append(f"states parsed : {st['states_parsed']}  "
                 f"(expect ~1081 land states)")
    lines.append(f"with owner    : {st['states_with_owner']}")
    lines.append(f"owner tags    : {st['distinct_owner_tags']} distinct")
    lines.append(f"prov ctrl     : {st['provinces_with_controller']} provinces "
                 f"have an explicit controller override")
    lines.append(f"countries     : {st['countries_parsed']} parsed, "
                 f"{st['countries_in_snapshot']} in snapshot")

    sample_ids = []
    for sid in ("64", "126", "1", "153", "270"):
        if sid in state_ownership:
            sample_ids.append(sid)
    if not sample_ids:
        sample_ids = list(state_ownership.keys())[:5]
    lines.append("spot-check state_id -> owner / controller:")
    for sid in sample_ids[:8]:
        so = state_ownership[sid]
        lines.append(f"   state {sid:>5} -> owner={so['owner']} "
                     f"controller={so['controller']}")

    lines.append("country snapshot sample:")
    for tag in list(country_snapshot.keys())[:4]:
        c = country_snapshot[tag]
        fac = c.get("factories", {})
        lines.append(
            f"   {tag}: stab={c.get('stability')} pp={c.get('political_power')} "
            f"fac(civ/mil/doc)={fac.get('civ')}/{fac.get('mil')}/"
            f"{fac.get('dockyard')} human={c.get('is_human')}")
    return "\n".join(lines)



def main(argv=None) -> int:
    ap = argparse.ArgumentParser(
        description="the game save -> JSON oracle (state owner, province controller, "
                    "country snapshot).")
    ap.add_argument("save", nargs="?", default=None,
                    help="Path to a .hoi4 save or melted plaintext. "
                         "If omitted, the most recent candidate is used.")
    ap.add_argument("--rakaly", default=None,
                    help="Path to rakaly executable (for binary saves).")
    ap.add_argument("--out-dir", default=None,
                    help="Output directory (default: <script>/output).")
    args = ap.parse_args(argv)

    script_dir = Path(__file__).resolve().parent
    out_dir = Path(args.out_dir) if args.out_dir else script_dir / "output"
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.save:
        save_path = Path(args.save)
        if not save_path.exists():
            print(f"ERROR: save not found: {save_path}", file=sys.stderr)
            return 2
    else:
        save_path = find_latest_save()
        if save_path is None:
            print("ERROR: no candidate save found in known locations.",
                  file=sys.stderr)
            print("Searched:", file=sys.stderr)
            for d in candidate_save_dirs():
                print(f"   {_scrub(str(d))}", file=sys.stderr)
            print("Pass a save path explicitly, or set BATTLE_PLAN_SAVE_DIR.",
                  file=sys.stderr)
            return 2

    print(f"[oracle] resolved save : {_scrub(str(save_path))}")
    rakaly = find_rakaly(args.rakaly)
    print(f"[oracle] rakaly        : "
          f"{_scrub(str(rakaly)) if rakaly else 'NOT FOUND'}")

    try:
        text, was_melted = melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 3

    print(f"[oracle] melted        : {was_melted}  "
          f"({len(text):,} chars to parse)")
    result = parse_save_text(text)
    state_ownership, country_snapshot, meta = build_outputs(
        result, save_path, was_melted)

    (out_dir / "state_ownership.json").write_text(
        json.dumps(state_ownership, indent=1), encoding="utf-8")
    (out_dir / "country_snapshot.json").write_text(
        json.dumps(country_snapshot, indent=1), encoding="utf-8")
    (out_dir / "meta.json").write_text(
        json.dumps(meta, indent=2), encoding="utf-8")

    report = validation_report(state_ownership, country_snapshot, meta)
    print()
    print(report)
    print()
    print(f"[oracle] wrote: {_scrub(str(out_dir / 'state_ownership.json'))}")
    print(f"[oracle] wrote: {_scrub(str(out_dir / 'country_snapshot.json'))}")
    print(f"[oracle] wrote: {_scrub(str(out_dir / 'meta.json'))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
