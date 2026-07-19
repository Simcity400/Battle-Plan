#!/usr/bin/env python3
# -*- coding: ascii -*-
"""locjoin_coverage.py -- verify the localisation_join static artifact ships in a
production .battleplan recording and covers the replay first-consumer key families.

Requirement item 9 (replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md:349):
the replay UI renders tokens the recording emits (country names, topbar tokens,
focus / idea / tech / decision names). Those names come from the deterministic
`localisation_join` static artifact packed into every container at
  payload/<game_id>/static/localisation_join/localisation_catalog.json
(role="static_artifact" in manifest.entries). This checker OPENS a run dir (or a
.battleplan file directly), locates that manifest entry, integrity-verifies its
bytes against the manifest sha256/size (fail-closed on mismatch -- zero silent
data loss), and reports per-family coverage.

Two family mechanisms, each matched to how the family appears in the catalog:
  * country-name <TAG>_<ideology> and TOPBAR_* are clean localisation KEYS, so
    they are checked directly against the `keys` map.
  * focus / idea / tech / decision names are bare-id keys with no stable prefix,
    so they are checked against the artifact's own self-reported
    `coverage.categories` (total>0 and resolved>0).

Exit code: 0 = all required families present; 1 = artifact absent, integrity
failure, or a missing/empty family. --selftest runs planted-failure cases against
the real code paths and exits 1 if any planted failure is not caught (it can go
red).

Stdlib only (zipfile, json, hashlib) -> portable to any end-user machine; no rig
paths, run names, or mod facts are hardcoded. The four ideology groups
(fascism/democratic/communism/neutrality) and TOPBAR_ tokens are engine-level and
present in any the game modset.
"""

import argparse
import glob
import hashlib
import io
import json
import os
import re
import sys
import zipfile

LOC_ARTIFACT_SUFFIX = "/static/localisation_join/localisation_catalog.json"
STATIC_ARTIFACT_ROLE = "static_artifact"
IDEOLOGY_GROUPS = ("fascism", "democratic", "communism", "neutrality")
# Bare-id name families measured via the catalog's self-reported coverage.
REQUIRED_CATEGORIES = ("focus", "idea", "tech", "decision")
# Soft floors: below these we WARN (suspiciously thin) but do NOT fail -- a hard
# floor would be brittle across cut-down mods. The hard gate is "> 0".
SOFT_FLOOR_IDEOLOGY = 20
SOFT_FLOOR_TOPBAR = 10


class LocJoinError(Exception):
    """Any fail-closed rejection (absent artifact, integrity mismatch, bad zip)."""


def find_container(run_path):
    """Resolve a run dir (or a direct .battleplan path) to a container file path.

    Prefers the finalized native_session.battleplan; falls back to the
    recovery native_session.failed.battleplan; then any *.battleplan. The
    localisation_join artifact is packed identically in both (it is embedded at
    container-write time, independent of the min-game-hours acceptance gate).
    """
    if os.path.isfile(run_path):
        if run_path.endswith(".battleplan"):
            return run_path
        raise LocJoinError("not a .battleplan file: %s" % run_path)
    if not os.path.isdir(run_path):
        raise LocJoinError("run path not found: %s" % run_path)
    finalized = os.path.join(run_path, "native_session.battleplan")
    if os.path.isfile(finalized):
        return finalized
    failed = os.path.join(run_path, "native_session.failed.battleplan")
    if os.path.isfile(failed):
        return failed
    any_hits = sorted(glob.glob(os.path.join(run_path, "*.battleplan")))
    if any_hits:
        return any_hits[0]
    raise LocJoinError("no .battleplan container in run dir: %s" % run_path)


def open_container(container_path):
    """Open a container, verify the loc-join artifact's manifest entry + bytes.

    Returns (manifest, game_id, container_kind, artifact_entry, catalog_dict).
    Raises LocJoinError on: unreadable zip, no manifest, artifact not listed in
    manifest.entries, size mismatch, or sha256 mismatch (all fail-closed).
    """
    try:
        with open(container_path, "rb") as handle:
            raw = handle.read()
    except OSError as exc:
        raise LocJoinError("cannot read container %s: %s" % (container_path, exc))
    return open_container_bytes(raw, os.path.basename(container_path))


def open_container_bytes(raw, name_hint):
    try:
        archive = zipfile.ZipFile(io.BytesIO(raw))
    except zipfile.BadZipFile as exc:
        raise LocJoinError("not a valid zip container (%s): %s" % (name_hint, exc))
    names = set(archive.namelist())
    if "manifest.json" not in names:
        # Recovery-only / unfinalized container (spec: no finalized manifest).
        raise LocJoinError(
            "container %s has no manifest.json (unfinalized/recovery-only; "
            "cannot verify static artifacts)" % name_hint
        )
    try:
        manifest = json.loads(archive.read("manifest.json"))
    except (ValueError, KeyError) as exc:
        raise LocJoinError("manifest.json is not valid JSON in %s: %s" % (name_hint, exc))
    if manifest.get("format") != "hoi4replay":
        raise LocJoinError("manifest.format != 'hoi4replay' in %s" % name_hint)
    game_id = manifest.get("game_id")
    if not isinstance(game_id, str) or not game_id:
        raise LocJoinError("manifest.game_id missing in %s" % name_hint)
    entries = manifest.get("entries")
    if not isinstance(entries, list):
        raise LocJoinError("manifest.entries missing/not-a-list in %s" % name_hint)

    artifact_entry = None
    for ent in entries:
        if not isinstance(ent, dict):
            continue
        path = ent.get("path")
        if not isinstance(path, str):
            continue
        if path.endswith(LOC_ARTIFACT_SUFFIX) and ent.get("role") == STATIC_ARTIFACT_ROLE:
            artifact_entry = ent
            break
    if artifact_entry is None:
        raise LocJoinError(
            "localisation_join static artifact is NOT listed in manifest.entries "
            "of %s (expected role=%s path ending %s). The recording did not ship "
            "the loc join -- check the launcher passes --static-localisation-join."
            % (name_hint, STATIC_ARTIFACT_ROLE, LOC_ARTIFACT_SUFFIX)
        )

    path = artifact_entry["path"]
    declared_sha = artifact_entry.get("sha256")
    declared_bytes = artifact_entry.get("bytes")
    if path not in names:
        raise LocJoinError(
            "manifest lists loc artifact %s but it is absent from the zip payload "
            "of %s (tamper/packaging bug)" % (path, name_hint)
        )
    payload = archive.read(path)
    if isinstance(declared_bytes, int) and len(payload) != declared_bytes:
        raise LocJoinError(
            "loc artifact size mismatch in %s: zip=%d manifest=%s"
            % (name_hint, len(payload), declared_bytes)
        )
    got_sha = hashlib.sha256(payload).hexdigest()
    if isinstance(declared_sha, str) and got_sha != declared_sha:
        raise LocJoinError(
            "loc artifact sha256 mismatch in %s: got=%s manifest=%s (integrity "
            "failure; fail-closed)" % (name_hint, got_sha, declared_sha)
        )
    try:
        catalog = json.loads(payload)
    except ValueError as exc:
        raise LocJoinError("loc artifact is not valid JSON in %s: %s" % (name_hint, exc))
    if name_hint.endswith(".failed.battleplan"):
        kind = "recovery(failed-gate)"
    else:
        kind = "finalized"
    return manifest, game_id, kind, artifact_entry, catalog


class FamilyResult(object):
    def __init__(self, name, ok, count, detail, warn=False):
        self.name = name
        self.ok = ok
        self.count = count
        self.detail = detail
        self.warn = warn


def check_families(catalog):
    """Return a list of FamilyResult. `ok=False` on any missing/empty family."""
    results = []
    keys = catalog.get("keys")
    if not isinstance(keys, dict):
        results.append(FamilyResult("keys_map", False, 0, "catalog has no 'keys' map"))
        return results

    # 1. Country-name <TAG>_<ideology> families (one check per ideology group).
    for ideology in IDEOLOGY_GROUPS:
        rx = re.compile(r"^[A-Z]{3}_%s$" % ideology)
        count = sum(1 for k in keys if rx.match(k))
        ok = count > 0
        warn = ok and count < SOFT_FLOOR_IDEOLOGY
        detail = "%d <TAG>_%s keys" % (count, ideology)
        if warn:
            detail += " (below soft floor %d)" % SOFT_FLOOR_IDEOLOGY
        if not ok:
            detail = "MISSING: no <TAG>_%s country-name keys" % ideology
        results.append(
            FamilyResult("country_name.%s" % ideology, ok, count, detail, warn)
        )

    # 2. Topbar tokens (TOPBAR_*).
    topbar_count = sum(1 for k in keys if k.startswith("TOPBAR_"))
    ok = topbar_count > 0
    warn = ok and topbar_count < SOFT_FLOOR_TOPBAR
    detail = "%d TOPBAR_* keys" % topbar_count
    if warn:
        detail += " (below soft floor %d)" % SOFT_FLOOR_TOPBAR
    if not ok:
        detail = "MISSING: no TOPBAR_* keys"
    results.append(FamilyResult("topbar_tokens", ok, topbar_count, detail, warn))

    # 3. Bare-id name families via the catalog's self-reported coverage.
    categories = {}
    cov = catalog.get("coverage")
    if isinstance(cov, dict) and isinstance(cov.get("categories"), dict):
        categories = cov["categories"]
    for cat_name in REQUIRED_CATEGORIES:
        cat = categories.get(cat_name)
        if not isinstance(cat, dict):
            results.append(
                FamilyResult(
                    "category.%s" % cat_name,
                    False,
                    0,
                    "MISSING: coverage.categories has no '%s'" % cat_name,
                )
            )
            continue
        total = cat.get("total") or 0
        resolved = cat.get("resolved") or 0
        pct = cat.get("loc_resolvable_percent")
        ok = isinstance(total, int) and total > 0 and isinstance(resolved, int) and resolved > 0
        detail = "total=%s resolved=%s resolvable%%=%s" % (total, resolved, pct)
        if not ok:
            detail = "MISSING/EMPTY: %s (total=%s resolved=%s)" % (cat_name, total, resolved)
        results.append(FamilyResult("category.%s" % cat_name, ok, resolved, detail))

    return results


def report(container_path, manifest, game_id, kind, artifact_entry, results):
    lines = []
    lines.append("=== localisation_join coverage ===")
    lines.append("container      : %s" % container_path)
    lines.append("kind           : %s" % kind)
    lines.append("game_id        : %s" % game_id)
    lines.append("artifact path  : %s" % artifact_entry.get("path"))
    lines.append("artifact bytes : %s" % artifact_entry.get("bytes"))
    lines.append("artifact sha256: %s" % artifact_entry.get("sha256"))
    lines.append("integrity      : VERIFIED (sha256 + size match manifest.entries)")
    lines.append("")
    lines.append("family                       status  detail")
    lines.append("-" * 72)
    for r in results:
        status = "OK"
        if not r.ok:
            status = "FAIL"
        elif r.warn:
            status = "WARN"
        lines.append("%-28s %-7s %s" % (r.name, status, r.detail))
    return "\n".join(lines)


def run_one(run_path):
    container_path = find_container(run_path)
    manifest, game_id, kind, artifact_entry, catalog = open_container(container_path)
    results = check_families(catalog)
    text = report(container_path, manifest, game_id, kind, artifact_entry, results)
    missing = [r for r in results if not r.ok]
    return text, missing


# --------------------------------------------------------------------------
# Selftest: build in-memory containers, plant failures, assert they are caught.
# --------------------------------------------------------------------------

def _make_catalog(drop_ideology=None, drop_topbar=False, drop_category=None,
                  zero_resolved_category=None):
    keys = {}
    for tag in ("GER", "ENG", "USA", "SOV", "FRA", "ITA", "JAP"):
        for ideo in IDEOLOGY_GROUPS:
            if drop_ideology == ideo:
                continue
            keys["%s_%s" % (tag, ideo)] = {"name": "%s %s" % (tag, ideo), "raw": ""}
    if not drop_topbar:
        for tok in ("ARMY", "NAVY", "AIR", "CONSTRUCTION"):
            keys["TOPBAR_%s" % tok] = {"name": tok, "raw": ""}
    keys["GER_rhineland"] = {"name": "Rhineland", "raw": ""}
    keys["infantry_weapons"] = {"name": "Infantry Equipment", "raw": ""}
    cats = {}
    for cat_name in REQUIRED_CATEGORIES:
        if drop_category == cat_name:
            continue
        resolved = 0 if zero_resolved_category == cat_name else 100
        cats[cat_name] = {"total": 100, "resolved": resolved, "loc_resolvable_percent": 99.0}
    return {"keys": keys, "coverage": {"categories": cats}, "schema": "test"}


def _make_container_bytes(catalog=None, omit_artifact=False, corrupt_sha=False,
                          omit_manifest=False, game_id="selftest"):
    buf = io.BytesIO()
    payload = json.dumps(catalog if catalog is not None else _make_catalog()).encode("ascii")
    art_path = "payload/%s%s" % (game_id, LOC_ARTIFACT_SUFFIX)
    sha = hashlib.sha256(payload).hexdigest()
    if corrupt_sha:
        sha = "0" * 64
    entries = []
    if not omit_artifact:
        entries.append({
            "path": art_path,
            "role": STATIC_ARTIFACT_ROLE,
            "bytes": len(payload),
            "sha256": sha,
        })
    manifest = {"format": "hoi4replay", "game_id": game_id, "entries": entries}
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        if not omit_manifest:
            z.writestr("manifest.json", json.dumps(manifest))
        if not omit_artifact:
            z.writestr(art_path, payload)
    return buf.getvalue()


def selftest():
    passed = 0
    failed = 0
    log = []

    def expect(label, cond):
        nonlocal passed, failed
        if cond:
            passed += 1
            log.append("[PASS] %s" % label)
        else:
            failed += 1
            log.append("[FAIL] %s" % label)

    # --- Positive: a good container passes with zero missing families. ---
    try:
        _m, _g, _k, _e, cat = open_container_bytes(_make_container_bytes(), "good.battleplan")
        missing = [r for r in check_families(cat) if not r.ok]
        expect("good container: artifact opens + integrity ok", True)
        expect("good container: no missing families", len(missing) == 0)
    except LocJoinError as exc:
        expect("good container opens (got LocJoinError: %s)" % exc, False)

    # --- Planted failure: artifact absent from manifest.entries. ---
    try:
        open_container_bytes(_make_container_bytes(omit_artifact=True), "noart.battleplan")
        expect("absent-artifact is caught", False)
    except LocJoinError:
        expect("absent-artifact is caught", True)

    # --- Planted failure: integrity (sha256) mismatch. ---
    try:
        open_container_bytes(_make_container_bytes(corrupt_sha=True), "badsha.battleplan")
        expect("sha256-mismatch is caught", False)
    except LocJoinError:
        expect("sha256-mismatch is caught", True)

    # --- Planted failure: no manifest.json (recovery-only). ---
    try:
        open_container_bytes(_make_container_bytes(omit_manifest=True), "noman.battleplan")
        expect("missing-manifest is caught", False)
    except LocJoinError:
        expect("missing-manifest is caught", True)

    # --- Planted failure: a missing country-name ideology family. ---
    _m, _g, _k, _e, cat = open_container_bytes(
        _make_container_bytes(_make_catalog(drop_ideology="fascism")), "noideo.battleplan")
    res = check_families(cat)
    expect("dropped fascism ideology -> family FAIL",
           any((not r.ok) and r.name == "country_name.fascism" for r in res))
    expect("other ideologies still OK",
           all(r.ok for r in res if r.name.startswith("country_name.") and "fascism" not in r.name))

    # --- Planted failure: missing TOPBAR tokens. ---
    _m, _g, _k, _e, cat = open_container_bytes(
        _make_container_bytes(_make_catalog(drop_topbar=True)), "notop.battleplan")
    res = check_families(cat)
    expect("dropped TOPBAR -> topbar_tokens FAIL",
           any((not r.ok) and r.name == "topbar_tokens" for r in res))

    # --- Planted failure: missing focus category. ---
    _m, _g, _k, _e, cat = open_container_bytes(
        _make_container_bytes(_make_catalog(drop_category="focus")), "nofocus.battleplan")
    res = check_families(cat)
    expect("dropped focus category -> category.focus FAIL",
           any((not r.ok) and r.name == "category.focus" for r in res))

    # --- Planted failure: focus category present but resolved=0. ---
    _m, _g, _k, _e, cat = open_container_bytes(
        _make_container_bytes(_make_catalog(zero_resolved_category="tech")), "zeroresolved.battleplan")
    res = check_families(cat)
    expect("tech resolved=0 -> category.tech FAIL",
           any((not r.ok) and r.name == "category.tech" for r in res))

    # --- Negative control: the checker must NOT flag a healthy family as missing. ---
    _m, _g, _k, _e, cat = open_container_bytes(_make_container_bytes(), "good2.battleplan")
    res = check_families(cat)
    expect("healthy container flags nothing (no false positive)",
           all(r.ok for r in res))

    print("\n".join(log))
    print("")
    print("selftest: %d passed, %d failed" % (passed, failed))
    return 0 if failed == 0 else 1


def main(argv):
    ap = argparse.ArgumentParser(
        description="Verify the localisation_join static artifact ships in a "
                    ".battleplan recording and covers the replay key families.")
    ap.add_argument("run_path", nargs="?",
                    help="run dir OR a .battleplan file to inspect")
    ap.add_argument("--selftest", action="store_true",
                    help="run planted-failure selftests (exits 1 if any is not caught)")
    args = ap.parse_args(argv)

    if args.selftest:
        return selftest()
    if not args.run_path:
        ap.error("run_path is required unless --selftest is given")

    try:
        text, missing = run_one(args.run_path)
    except LocJoinError as exc:
        print("[FAIL] %s" % exc)
        return 1
    print(text)
    print("")
    if missing:
        print("RESULT: FAIL -- %d missing/empty famil%s: %s"
              % (len(missing), "y" if len(missing) == 1 else "ies",
                 ", ".join(r.name for r in missing)))
        return 1
    print("RESULT: PASS -- all required key families present and integrity-verified")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
