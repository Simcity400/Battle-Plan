#!/usr/bin/env python3
"""The standing cross-workspace AUTHORITY PIN gate.

A producer/consumer authority pin is anything the Replay side REQUIRES a
recorded frame to carry before it will admit it. Drift in a pin is silent by
construction: the producer keeps emitting, the consumer keeps refusing, and
nothing in either workspace's own test suite can see the disagreement because
the two halves never compile together. This script is the thing that sees it.

WHY THIS FILE EXISTS -- three measured instances of one blindness
----------------------------------------------------------------
`runs/gp_ledger_map/authority_vocab_diff.py` was the gate before this one. It
tokenises on ONE name prefix, `exact_a729_`, so a pin spelled any other way is
not "unmet" -- it is invisible, and a lane reporting the gate green is
reporting a fact about somebody else's carrier:

  * IO-F6 (`runs/io_gp_u9/IO_F6_gate_blindness_proof.txt`). Row G4-X9S1's two
    authorities are `a729_ctechnology_status_researched_vector_v1` and
    `a729_country_technology_ui_state_classifier_v1` -- the right KIND of name,
    spelled without `exact_`. Lane IO landed both as producer constants,
    committed them, and re-ran the gate: all five counts identical. Not unmet.
    Unmeasurable.
  * Lane IZ (`runs/iz_gp_u14/DOSSIER_GP_U14_wing_visibility.md` section 8).
    d95/SPU1 has NO authority name at all. Its cross-workspace pin is an
    authority OBJECT whose fields `world.rs::parse_exact_spu1` checks one at a
    time (`schema == "SPU1"`, `producer_rva == "0x016068c0"`,
    `air_wings_available == false`, two manifest hashes, ...). IZ's report
    states, correctly, that widening the prefix fixes G4-X9S1 and leaves d95
    unmeasured.
  * Lane IW (`runs/iw_gp_n5/DOSSIER_GP_N5_producer_seam.md` section 279-290).
    The corrected xmap20/xmap09 carriers pin schema STRINGS --
    `hoi4.xmap20.effect-frame.a729.v1`,
    `hoi4.xmap09.semantic-lighting-input.a729.v1`. IW ran the old gate and
    confirmed zero xmap20/xmap09 tokens in its vocabulary, then had to write a
    lane-local probe because no standing gate covered the shape.

So the gate measures THREE pin classes, and a lane may cite it for any of them:

  name           a snake_case authority name carrying `a729` as a component:
                 `exact_a729_passive_all_country_politics` (the old gate's
                 whole world), `a729_country_technology_ui_state_classifier_v1`
                 (IO-F6), `a729_xui16_occupation_ui_supplement_v1` (the
                 exception the gap map's GP-U6 row had to write out in prose).
  schema_string  a dotted schema constant: `hoi4.xmap20.effect-frame.a729.v1`,
                 `replay.xui12.production-lines-authority.a729.v2`,
                 `xmap17.faction_labels_exact.a729.v1` (IW).
  field_pin      a consumer requiring one FIELD of a JSON map to equal one
                 exact value, checked field by field (IZ). SPU1, FOW1 and MAPC1
                 are the three authority envelopes written this way today, but
                 the class is found by parsing the COMPARISON, not by naming
                 them.

                 STATED BOUND, because it is a superset and reporting it as
                 anything else would be false: nothing lexical distinguishes an
                 authority object from any other `serde_json::Map`. A
                 round-trip assertion in `recorder_host/src/build_declaration.rs`
                 and a current-process PID check in `collector/src/attach_log.rs`
                 are both in this class and neither is an authority pin. The
                 class is what a producer/consumer field agreement WOULD look
                 like; membership is not a claim that a row is a seam pin. Read
                 it as the predecessor's own header asks its numbers to be read:
                 a lower bound on doubt, never a proof. The same applies to the
                 `dynamic` census, which is why neither `dynamic` nor the loose
                 census is counted in `shipping_pins`.

NOTHING HERE IS A HAND-MAINTAINED LIST. A hand list is the drift class this
gate exists to catch, so every vocabulary is derived from the two workspaces'
own committed source. The only constants below are the two SCOPES and the
lexical shape of each class.

FAIL CLOSED, and here is exactly how, because this contract drifted twice.
A CONSUMER pin site whose SHAPE this reader cannot read is reported in
`unparsed` and EXITS 1. A pin whose shape reads but whose CONSTANT cannot be
resolved without implementing Rust name resolution is reported in
`unresolved_constant` -- named, by site, with the expression -- and does NOT set
the exit code, because it is a stated limitation of this reader rather than a
blindness in it. Neither is ever skipped: silence is the failure mode this whole
file is about. FIELD PINS ARE READ ON THE CONSUMER SIDE ONLY, so a producer-side
pin site sets no exit code at all -- a producer checking a field of its own
manifest is an internal consistency check, never a cross-workspace seam pin.

WHAT THIS GATE IS NOT
---------------------
It composes with `runs/hu_wave2_scaffold/wave2_scope_enumerate.py` rather than
duplicating it. That script is the WAVE-2 SCAFFOLD enumerator: it reads the
gap map's part-(a) key list, proves each key is declared in a carrier file AND
called by `wave2_sidecars/mod.rs::carriers()`, and compares each carrier's own
`RequiredSchema` against its consumer's demanded version. It answers "is the
scaffold registered and version-aligned for these 15 named keys". This script
answers a different question over the WHOLE tree: "for every authority pin
either side spells, does the other side spell it too". Neither subsumes the
other -- a carrier can be registered with a matching `RequiredSchema` and still
pin a schema string no producer mints, and a pin can match on both sides for a
carrier that was never registered.

It is also a LOWER BOUND ON DOUBT, not a proof of production, exactly as its
predecessor was. `minted` means the literal occurs in non-comment producer code
outside `runs/`. A test fixture or an unrelated constant of the same spelling
counts. A lane claiming its pin is satisfied still has to point at the emitting
line. What the gate proves is the negative: a pin the producer side never
spells at all cannot be satisfied.

SCOPE HAZARDS, all inherited deliberately from finding GP-F11:
  * `extractor/native/scripts/` is TOOLING, not producer code -- and THIS FILE
    lives there. A gate that scans its own directory mints the vocabulary it
    measures. The first review round found
    `a729_xui16_occupation_ui_supplement_v1`, `hoi4.xmap20.effect-frame.a729.v1`
    and `hoi4.xmap09.semantic-lighting-input.a729.v1` reading `shared` with their
    ONLY producer site inside this file's own `self_test`, which made the three
    headline production changes unable to move the gate. That is finding GP-F11
    exactly, one directory over, and in the flattering direction again. The rule
    is the general one: a script that quotes an authority in order to CHECK it
    does not produce it. `check_scope_excludes_this_gate` asserts the exclusion
    holds rather than trusting the string above.
  * `extractor/native/runs/` is APPEND-ONLY LANE EVIDENCE, not producer code.
    It holds lane dossiers, Codex reviewer transcripts that quote authorities
    back verbatim, and previous gate outputs that enumerate the very tokens a
    gate searches for. It is excluded, or the gate measures itself and the
    error is always in the flattering direction.
  * Comments are stripped before any pin is extracted, so a `//!` header
    naming the authority a file has NOT implemented cannot mint it. Tokens
    that occur only in comments are still reported, in `comment_only`.
  * Test paths are flagged, not excluded. Finding GP-F17 was a name that
    existed solely as a Replay-side test fixture and was not an admitted
    authority anywhere; a gate that counts fixtures as consumers repeats it.

Everything is read from `HEAD`, not the working tree -- a pin added in an
uncommitted edit does not move until it is committed. That is the same rule
the predecessor documented and lanes rely on it when they run a before/after.

USAGE
    python extractor/native/scripts/authority_pin_gate.py \
        --out extractor/native/runs/<your-run>/authority_pin_gate.json
Exit 0 == every pin site parsed. Exit 1 == at least one `unparsed` site.
Drift counts are REPORTED, never exit codes: the tree has known-open pins by
design, and a gate that is permanently red is a gate nobody runs.

kills: reverting the `name` class to the `exact_a729_` prefix drops the two
G4-X9S1 authorities and `self_test` fails on the IO-F6 case; deleting the
`field_pin` extractor makes the SPU1 case fail with zero pins found; deleting
the `schema_string` class makes the IW case fail; removing comment stripping
makes the comment case classify a `//!`-only token as code; removing the
`runs/` exclusion makes the self-contamination case fail; making an
unresolvable pin argument skip instead of report makes the fail-closed case
fail. `self_test()` runs first on every invocation and prints the number of
cases it actually ran rather than a number written here, which is how a count
in a docstring goes stale.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

# The two sides of the seam. `runs/` is excluded for finding GP-F11's reason;
# the Replay side's evidence lives in `replay_interface/re_evidence/`, outside
# `native/`, so it is already out of scope.
#
# Written as (include, exclude) prefix lists rather than as git pathspec magic:
# `git ls-tree` rejects `:(exclude)` outright ("pathspec magic not supported by
# this command"), and a scope whose exclusion is silently inert is the exact
# defect finding GP-F11 recorded. Excluding in Python keeps the exclusion where
# it can be tested.
PRODUCER_SCOPE = (
    ["extractor/native/", "extractor/game_defs/"],
    ["extractor/native/runs/", "extractor/native/scripts/"],
)
CONSUMER_SCOPE = (["replay_interface/native/", "portable_replay/native/"], [])

# Rust AND Python. Not decoration: two of the extractor's producers for pins the
# Replay side enforces are Python, and an `.rs`-only reader reported both of them
# as drift.
#
#   `extractor.ui14_engine_vfs.v1` -- pinned by `game_ui/src/assets.rs::from_runtime_userdir`,
#     minted by `extractor/game_defs/live_binding_resolver.py::UI14_ENGINE_VFS_SCHEMA`;
#   the A27 live-heap catalog envelope (`program_sha256`, `coherence`,
#     `full_replacement`, `transport`) -- pinned by
#     `portable_replay/native/snapshot/src/design_catalog.rs`, minted by
#     `extractor/native/lanes/design_catalog_live/live_a729_acceptance.py`.
#
# This is a FOURTH instance of the blindness class the gate was written for --
# IO-F6's was a name prefix, IZ's a pin shape, IW's a constant shape, and this
# one a file extension -- and it was found by running the gate, which is the
# argument for the gate.
SOURCE_SUFFIXES = (".rs", ".py")

# ---------------------------------------------------------------------------
# Lexical shape of each pin class. These are the only hand-written constants,
# and each is a SHAPE, not a membership list.
# ---------------------------------------------------------------------------

# The build tag every a729-pinned authority carries. Pins are build-scoped by
# construction (see any dossier's build-scope residual), which is what makes
# the tag a reliable discriminator rather than a guess.
BUILD_TAG = "a729"

# A path-shaped literal is never a pin. Without this, `"a729_hook.rs"` and
# `"interface/*.gui"` enter the vocabulary.
PATH_SUFFIXES = (
    ".rs", ".py", ".ps1", ".json", ".toml", ".md", ".txt", ".tsv", ".csv",
    ".yml", ".yaml", ".exe", ".dll", ".pdb", ".log", ".jsonl", ".gui", ".gfx",
    ".dds", ".png", ".bmp", ".bin", ".zip", ".sh", ".lock", ".xml", ".html",
)

# `.v<n>` version tail, optionally followed by a C string terminator. Several
# digest domain-separation prefixes are spelled with a trailing NUL and the NUL
# is part of the pinned value (`xui09_research_contract.rs` asserts exactly
# that), so it is preserved in the token and only ignored when deciding shape.
VERSION_TAIL = re.compile(r"\.v\d+$")

IDENT_CHARS = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_")


def classify_literal(value: str) -> str | None:
    """Which pin class, if any, this string literal belongs to.

    Returns "name", "schema_string" or None. The two positive classes are
    disjoint by construction: a `name` has no `.`, a `schema_string` must have
    one.
    """
    if not value or "\n" in value or "\r" in value:
        return None
    if "/" in value or "\\" in value.replace("\\0", ""):
        return None
    core = value.replace("\\0", "").replace("\0", "")
    if not core:
        return None
    lowered = core.lower()
    if lowered.endswith(PATH_SUFFIXES):
        return None

    # name: snake_case, at least two components, `a729` as a WHOLE component.
    # Whole-component is what keeps `1.19.2.0.a729` and `hoi4-a729` out while
    # admitting `exact_a729_passive_all_country_politics` (the old gate's
    # world), `a729_country_technology_ui_state_classifier_v1` (IO-F6) and
    # `a729_xui16_occupation_ui_supplement_v1` (the gap map's GP-U6 prose
    # exception). A bare `"a729"` is a build tag, not an authority, and has one
    # component, so it is out.
    if re.fullmatch(r"[A-Za-z0-9_]+", core):
        parts = core.split("_")
        if len(parts) >= 2 and BUILD_TAG in parts:
            return "name"
        return None

    # schema_string: a NAMESPACED dotted constant carrying the build tag as its
    # own dotted component.
    #
    # The build tag is what makes this class the CROSS-WORKSPACE one, and it is
    # the same discriminator the `name` class uses one line up. Both workspaces
    # are full of dotted version constants that never cross the seam --
    # `hoi4.production-render-submit.v13`, `hoi4.replay.package.v1`,
    # `extractor.replay_decoder.binary_seek.v1` -- and a first revision that
    # accepted any `.v<n>` tail returned 414 tokens of which 229 were "consumer
    # only" purely because `portable_replay`'s own renderer wire has no reason
    # to appear in the extractor. That is not drift, it is a scope error, and a
    # gate whose drift column is mostly scope error is one nobody reads. `a729`
    # names the reverse-engineered executable this pin was derived against, so a
    # constant carrying it is by construction about the recorded game and not
    # about Replay's internal plumbing.
    #
    # The second requirement exists because the tag alone admits a version
    # number: `1.19.2.0.a729` is the engine build string, dotted, with `a729` as
    # a whole component. What separates it from `hoi4.xmap20.effect-frame.a729.v1`
    # is shape -- a version string is a tuple of integers with a tag on the end,
    # a schema string is a namespace path -- so at least two components must be
    # NAMES rather than integers. `1.19.2.0.a729` has exactly one (`a729`);
    # `hoi4.a729.present-consumed-rolling.v2` has four.
    #
    # STATED BOUND. Two Wave-2 schema tokens are spelled WITHOUT the tag --
    # `xui19.decision_evaluation_exact.v1` and `MAPC1` -- so they are not in
    # this class. They are not unmeasured: both are part-(a) keys of
    # `runs/hu_wave2_scaffold/wave2_scope_enumerate.py`, which compares each
    # carrier's recorded `RequiredSchema` / `REQUIRED_SCHEMA_TOKEN` against the
    # consumer's own literal and exits 1 on drift. That is the composition
    # boundary between the two gates, and it is a boundary rather than a hole
    # only for keys that script enumerates.
    if re.fullmatch(r"[A-Za-z0-9_.\-]+", core) and "." in core:
        dotted = core.split(".")
        if BUILD_TAG not in dotted:
            return None
        named = [part for part in dotted if part and not re.fullmatch(r"\d+", part)]
        if len(named) >= 2:
            return "schema_string"
    return None


# ---------------------------------------------------------------------------
# Rust lexing: comment stripping that PRESERVES OFFSETS, string extraction.
# ---------------------------------------------------------------------------

# Rust raw strings: `r"..."`, `r#"..."#`, and the byte/C variants. Inside one a
# `"` does not end the literal, so a scanner that missed the prefix would read
# the rest of the file in the wrong state. All three prefixes, per the
# reference, not just the ones a counter-example happened to name.
RAW_STRING_OPEN_RE = re.compile(r"(?:[bc]?r)(#*)\"")
# A char literal, told apart from a lifetime tick by the closing quote.
# `const QUOTE: char = '"';` and `b'"'` both occur in this tree; treating
# either as a string opener leaves the scanner inside a string for the rest of
# the file and every following comment is then copied through as code.
CHAR_LITERAL_RE = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^\\'])'")


def lex(text: str) -> tuple[str, str]:
    """Split Rust source into (code, comments), both OFFSET-PRESERVING.

    Every character of the input appears at the same index in exactly one of
    the two outputs; the other holds a space (or a newline, so line numbers
    survive in both). That is what lets a pin found in `code` be located by
    offset and a comment-only token be reported separately without a second
    pass.

    Rust block comments NEST, so `/* outer /* inner */ still a comment */`
    cannot be matched by a non-greedy regex -- it ends at the inner `*/` and
    hands live-looking text back as code. This is a scanner for that reason and
    because it must be string-aware: `"interface/*.gui"` occurs in
    `game_ui/src/production_lines.rs`, and treating that `/*` as a comment
    opener would delete real code up to the next `*/` -- a false PASS built by
    the function meant to prevent one.
    """
    code: list[str] = []
    comment: list[str] = []
    i = 0
    n = len(text)
    depth = 0

    def blank(chunk: str) -> str:
        return "".join("\n" if ch == "\n" else " " for ch in chunk)

    while i < n:
        ch = text[i]
        pair = text[i : i + 2]
        if depth:
            if pair == "/*":
                depth += 1
                code.append("  ")
                comment.append(pair)
                i += 2
                continue
            if pair == "*/":
                depth -= 1
                code.append("  ")
                comment.append(pair)
                i += 2
                continue
            code.append("\n" if ch == "\n" else " ")
            comment.append(ch)
            i += 1
            continue
        if pair == "/*":
            depth = 1
            code.append("  ")
            comment.append(pair)
            i += 2
            continue
        if pair == "//":
            j = i
            while j < n and text[j] != "\n":
                j += 1
            chunk = text[i:j]
            code.append(blank(chunk))
            comment.append(chunk)
            i = j
            continue
        if ch in "rbc" and (i == 0 or text[i - 1] not in IDENT_CHARS):
            m = RAW_STRING_OPEN_RE.match(text, i)
            if m:
                terminator = '"' + m.group(1)
                end = text.find(terminator, m.end())
                end = n if end < 0 else end + len(terminator)
                chunk = text[i:end]
                code.append(chunk)
                comment.append(blank(chunk))
                i = end
                continue
        if ch == "'":
            m = CHAR_LITERAL_RE.match(text, i)
            if m:
                code.append(m.group(0))
                comment.append(blank(m.group(0)))
                i = m.end()
                continue
            code.append(ch)
            comment.append(" ")
            i += 1
            continue
        if ch == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            chunk = text[i:j]
            code.append(chunk)
            comment.append(blank(chunk))
            i = j
            continue
        code.append(ch)
        comment.append(" ")
        i += 1
    return "".join(code), "".join(comment)


# A cooked (non-raw) Rust string literal, with its escapes. Raw strings are
# matched separately below because their body has no escape processing.
COOKED_STRING_RE = re.compile(r'(?<![A-Za-z0-9_])(?:[bc]?)"((?:[^"\\]|\\.)*)"')
RAW_STRING_RE = re.compile(r'(?<![A-Za-z0-9_])(?:[bc]?r)(#*)"')

SIMPLE_ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}


def unescape(raw: str) -> str:
    """Resolve the escapes that matter to a pin's VALUE.

    `\\0` is the one that is load-bearing rather than cosmetic: several digest
    domain-separation prefixes pin a terminating NUL and the byte is part of
    the pinned value. Unknown escapes are passed through with their backslash
    so an unrecognised sequence cannot silently change a token.
    """
    out: list[str] = []
    i = 0
    while i < len(raw):
        if raw[i] == "\\" and i + 1 < len(raw):
            nxt = raw[i + 1]
            if nxt in SIMPLE_ESCAPES:
                out.append(SIMPLE_ESCAPES[nxt])
                i += 2
                continue
            if nxt == "x" and i + 3 < len(raw):
                try:
                    out.append(chr(int(raw[i + 2 : i + 4], 16)))
                    i += 4
                    continue
                except ValueError:
                    pass
            out.append(raw[i : i + 2])
            i += 2
            continue
        out.append(raw[i])
        i += 1
    return "".join(out)


# --- Python -----------------------------------------------------------------
# A Python string opener with any prefix combination. Triple quotes are matched
# FIRST: `"""x"""` read as `""` + `x` + `""` desyncs the scanner for the rest of
# the file, and the extractor's Python producers are docstring-heavy.
PY_STRING_OPEN_RE = re.compile(r"(?:[rRbBuUfF]{0,3})('''|\"\"\"|'|\")")


def lex_python(text: str) -> tuple[str, str]:
    """Split Python source into (code, comments), offset-preserving.

    `#` opens a comment only outside a string; the extractor's Python carries
    `"#"` and `"a729#..."` in ordinary literals.

    A DOCSTRING is treated as a comment, not as code. Python has no `//!`, so
    the module and function docstrings are where a Python producer describes the
    authority it has not implemented -- the same hazard finding GP-F11 recorded
    for Rust doc comments, in the only place Python can express it. The test is
    positional and deliberately narrow: a string literal is a docstring when it
    is the whole of its logical line, i.e. nothing but whitespace precedes it on
    the line and nothing but whitespace (or a `#` comment) follows it. A literal
    used as a value on its own line is vanishingly rare; a docstring never is
    anything else.
    """
    code: list[str] = []
    comment: list[str] = []
    i = 0
    n = len(text)

    def blank(chunk: str) -> str:
        return "".join("\n" if ch == "\n" else " " for ch in chunk)

    while i < n:
        ch = text[i]
        if ch == "#":
            j = i
            while j < n and text[j] != "\n":
                j += 1
            chunk = text[i:j]
            code.append(blank(chunk))
            comment.append(chunk)
            i = j
            continue
        if ch in "\"'" or (
            ch in "rRbBuUfF" and (i == 0 or text[i - 1] not in IDENT_CHARS)
        ):
            m = PY_STRING_OPEN_RE.match(text, i)
            if m:
                quote = m.group(1)
                j = m.end()
                while j < n:
                    if text[j] == "\\":
                        j += 2
                        continue
                    if text.startswith(quote, j):
                        j += len(quote)
                        break
                    if len(quote) == 1 and text[j] == "\n":
                        break
                    j += 1
                chunk = text[i:j]
                line_start = text.rfind("\n", 0, i) + 1
                before = text[line_start:i]
                after_end = text.find("\n", j)
                after = text[j : n if after_end < 0 else after_end]
                after = after.split("#", 1)[0]
                is_docstring = before.strip() == "" and after.strip() == ""
                if is_docstring:
                    code.append(blank(chunk))
                    comment.append(chunk)
                else:
                    code.append(chunk)
                    comment.append(blank(chunk))
                i = j
                continue
        code.append(ch)
        comment.append(" ")
        i += 1
    return "".join(code), "".join(comment)


PY_STRING_RE = re.compile(
    r"(?<![A-Za-z0-9_])(?P<prefix>[rRbBuUfF]{0,3})"
    r"(?P<body>'''[\s\S]*?'''|\"\"\"[\s\S]*?\"\"\"|'(?:[^'\\\n]|\\.)*'|\"(?:[^\"\\\n]|\\.)*\")"
)


def string_literals_python(code: str) -> list[tuple[str, int]]:
    found: list[tuple[str, int]] = []
    for m in PY_STRING_RE.finditer(code):
        body = m.group("body")
        quote = body[:3] if body[:3] in ("'''", '"""') else body[0]
        raw = body[len(quote) : -len(quote)]
        prefix = m.group("prefix").lower()
        # An f-string's value is not fixed at all, so it is not a pin. Raw
        # strings keep their backslashes; everything else is unescaped.
        if "f" in prefix:
            continue
        found.append((raw if "r" in prefix else unescape(raw), m.start()))
    return found


def string_literals_for(path: str, code: str) -> list[tuple[str, int]]:
    return string_literals_python(code) if path.endswith(".py") else string_literals(code)


def string_literals(code: str) -> list[tuple[str, int]]:
    """Every string literal in comment-stripped source, with its offset."""
    found: list[tuple[str, int]] = []
    for m in COOKED_STRING_RE.finditer(code):
        found.append((unescape(m.group(1)), m.start()))
    for m in RAW_STRING_RE.finditer(code):
        terminator = '"' + m.group(1)
        end = code.find(terminator, m.end())
        if end < 0:
            continue
        found.append((code[m.end() : end], m.start()))
    return found


# ---------------------------------------------------------------------------
# `#[cfg(test)]` regions, so a fixture is FLAGGED rather than counted silently.
# Finding GP-F17 was a name that existed only as a Replay-side test fixture.
# ---------------------------------------------------------------------------

CFG_TEST_RE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")


def cfg_test_spans_for(path: str, code: str) -> list[tuple[int, int]]:
    """Test regions, per language.

    Rust marks them with `#[cfg(test)]`; Python has no equivalent inside a
    module, so a Python file is either a test file by path or it is not. The
    path test already covers `/tests/`; nothing further is inferred, and
    inferring less is the safe direction for a gate.
    """
    return [] if path.endswith(".py") else cfg_test_spans(code)


def cfg_test_spans(code: str) -> list[tuple[int, int]]:
    """Half-open [start, end) offsets of every `#[cfg(test)]` item body."""
    spans: list[tuple[int, int]] = []
    for m in CFG_TEST_RE.finditer(code):
        i = code.find("{", m.end())
        if i < 0:
            continue
        depth = 0
        j = i
        while j < len(code):
            if code[j] == "{":
                depth += 1
            elif code[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        spans.append((m.start(), min(j + 1, len(code))))
    return spans


def in_spans(offset: int, spans: list[tuple[int, int]]) -> bool:
    return any(start <= offset < end for start, end in spans)


def is_test_path(path: str) -> bool:
    return "/tests/" in path or path.endswith("/tests.rs") or "/benches/" in path


# ---------------------------------------------------------------------------
# field_pin extraction -- lane IZ's class.
# ---------------------------------------------------------------------------

# A two-`&str` helper whose body reads a key out of a map and compares it to the
# expected value. Found by SHAPE -- closure binding or free function -- so a
# fourth envelope written the same way is picked up without editing this file,
# which is the whole point: a hand list of envelope names is the drift class the
# gate exists for.
EXACT_HELPER_RE = re.compile(
    r"(?:let\s+(\w+)\s*=\s*\|\s*(\w+)\s*:\s*&\s*str\s*,\s*(\w+)\s*:\s*&\s*str\s*\|"
    r"|fn\s+(\w+)\s*\(\s*(\w+)\s*:\s*&\s*str\s*,\s*(\w+)\s*:\s*&\s*str\s*\))"
    r"([\s\S]{0,400})"
)

# A call in PIN SHAPE: a bare-identifier callee with exactly two arguments whose
# first is a string literal. The shape is the filter, not the name.
#
# A first revision keyed on the callee NAME instead (`exact\w*`) and reported
# 1,073 "unparsed pin sites" -- every `exact_frame`, `exact_text`, `exact_order`
# and `exact_binding` in two workspaces, none of which is a pin: they are local
# bindings and test fixtures that happen to start with the same five letters.
# Fail-closed reporting is only useful if what it reports is a pin candidate; a
# thousand false ones hide the real one exactly as silence would.
PIN_SHAPE_CALL_RE = re.compile(
    r"(?<![A-Za-z0-9_.])([a-z_][A-Za-z0-9_]*)\s*\(\s*"
    r'("(?:[^"\\]|\\.)*")\s*,\s*'
    r"((?:[^()\"]|\"(?:[^\"\\]|\\.)*\")*?)\s*,?\s*\)"
)

# The inline spelling: `.get("field").and_then(Value::as_str) != Some("value")`.
# rustfmt splits these across lines, so every gap is `\s*`.
#
# TWO accessor spellings, and the second was found by the loose net below rather
# than by reading the three known envelopes: `game_ui/src/assets.rs::from_runtime_userdir` pins
# `.get("schema").and_then(|v| v.as_str()) != Some("extractor.ui14_engine_vfs.v1")`
# through a CLOSURE, not a path. A reader that only knew `Value::as_str` had
# that pin in its "loose only" census and out of its measured set -- the same
# shape of miss, one spelling over, which is why the census exists.
# The payload of a `Some(...)`: any run of characters with at most one level of
# balanced parentheses, string literals kept intact. Defined ONCE and shared by
# the inline reader and the let-bound reader below -- two copies of a regex this
# fiddly drift apart, and the second copy had already lost its backslash escapes
# and failed to compile.
SOME_PAYLOAD = (
    r'(?:[^()"]|"(?:[^"\\]|\\.)*"|\((?:[^()"]|"(?:[^"\\]|\\.)*")*\))*?'
)

# An `and_then` CHAIN LINK: one `.and_then(...)` whose argument may itself carry a
# balanced pair of parentheses (`|v| v.as_str()`, `|id| id.get("k")`).
AND_THEN_LINK = r"\s*\.\s*and_then\(\s*(?:[^();]|\([^()]*\))*\)"

# `.get("field")` followed by ZERO OR MORE `and_then` links and then the
# comparison.
#
# ZERO, not one. Requiring a link was a conservatism that hid a real shape:
# `compact_replay_verification_gates/src/lib.rs` seals the recorder authority
# with a bare `embedded_manifest.get("portable_content_manifest") != Some(&v)`,
# comparing the whole `Value` with no accessor at all. Two such sites sat in the
# non-failing census while `unparsed 0` was quoted as covering every recognised
# shape. Review round 6, finding 1. The chain is what makes a NESTED read visible:
# `rollover_package_set.rs` pins the campaign identity as
#   .get("campaign_identity").and_then(as_object)
#       .and_then(|identity| identity.get("campaign_id")).and_then(as_str)
#       != Some(segment.campaign_id_hex.as_str())
# and a reader that allowed exactly one link could not see it at all. The FIELD
# and the ACCESSOR are then taken as the LAST of each inside the matched span,
# because in a nested read the pinned field is the inner one -- reporting
# `campaign_identity` there would name the envelope instead of the pin.
DIRECT_PIN_RE = re.compile(
    r'\.\s*get\(\s*"(?P<field>[A-Za-z0-9_]+)"\s*\)'
    r"(?P<chain>(?:" + AND_THEN_LINK + r")*)\s*"
    r"(?P<op>!=|==)\s*Some\(\s*(?P<value>" + SOME_PAYLOAD + r")\s*\)"
)

# Consumers spell a pinned value as a named constant about as often as inline,
# and in every arithmetic type, so all of them are followed. A reader that only
# understood `&str` reported the numeric half as unresolvable and left those
# pins unmeasured -- the same blindness one level down.
CONST_STR_RE = re.compile(
    r'(?:pub\s+)?(?:pub\s*\([^)]*\)\s*)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*&\s*(?:\'\w+\s+)?str\s*=\s*"((?:[^"\\]|\\.)*)"'
)
CONST_NUM_RE = re.compile(
    r"(?:pub\s+)?(?:pub\s*\([^)]*\)\s*)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*"
    r"(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize)\s*=\s*(-?[0-9_]+)\s*;"
)
CONST_BOOL_RE = re.compile(
    r"(?:pub\s+)?(?:pub\s*\([^)]*\)\s*)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*bool\s*=\s*(true|false)\s*;"
)
# `const A: &str = crate::other::B;` -- one alias hop, resolved after the whole
# tree is read so definition order does not matter.
CONST_ALIAS_RE = re.compile(
    r"(?:pub\s+)?(?:pub\s*\([^)]*\)\s*)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*"
    r"(?:&\s*(?:'\w+\s+)?str|u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|bool)\s*=\s*"
    r"((?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*[A-Z][A-Z0-9_]*)\s*;"
)

# A use of an ALL-CAPS constant. Length >= 3 keeps single-letter generics and
# two-letter enum shorthands out; the leading boundary keeps it from matching
# the tail of a longer mixed-case identifier.
CONST_REF_RE = re.compile(r"(?<![A-Za-z0-9_])([A-Z][A-Z0-9_]{2,})(?![A-Za-z0-9_])")

# The WATCH CENSUS for the inline spelling, and it is a census, not a gate.
#
# `DIRECT_PIN_RE` reads the accessor chains the shipping envelopes use; this
# reads any `.get("field")` compared against a `Some(...)` within 200 characters,
# whatever sits between. Sites it finds that the strict reader did not are
# recorded in `loose_only_get_some` and do NOT set the exit code.
#
# An earlier revision's comment claimed these were reported as `unparsed`. That
# was false and review round 1 caught it, along with the reason it mattered:
# `world.rs`'s `fixed_scale`/`displayed_decimals` pins and MAPC1's three digest
# pins were real pins sitting in this census, never given a producer comparison,
# while `unparsed 0` was being quoted as "every recognised pin parsed". The
# strict reader was widened to take them and the claim is now the accurate one:
# this census cannot be an exit code because the same pattern also matches
# ordinary `Option` comparisons that are not authority pins at all. Its NUMBER
# MOVING is the signal a reader should act on, not its value; the count is
# printed on every run for exactly that.
# The LET-BOUND spelling: the field is read into a local and compared further
# down. `replay_core/src/source.rs` admits the compact stream this way --
#     let format = timeline.get("format").and_then(Value::as_str);
#     let role   = timeline.get("raw_stream_role").and_then(Value::as_str);
#     let declares_compact = format == Some(COMPACT_STREAM_TIMELINE_FORMAT)
#                         || role   == Some(COMPACT_STREAM_ROLE);
# -- and review round 2 found that real admission pin sitting in the non-failing
# census while `unparsed 0` was being quoted as "every recognised pin parsed".
# One binding hop, bounded to the same file and to a window after the binding,
# which is enough for every occurrence of the shape in this tree.
# TWO binding forms, because both occur and each hides a real pin:
#
#   let <bind> = <recv>.get("<field>").and_then(<accessor>);   ... <bind> == Some(v)
#       -- `replay_core/src/source.rs`'s compact-stream admission.
#   if let Some(<bind>) = <recv>.get("<field>") { ... <bind>.as_str() != Some(v) }
#       -- found in the (since-deleted) `replay_core/src/g2_engine_capture.rs`
#          packaged-witness join on `effective_vfs_manifest_sha256`, which
#          review round 3 found still sitting in the census because the
#          accessor is applied at the COMPARISON rather than at the binding.
#          The file is gone (lane MA, 2026-08-18); the binding FORM it taught
#          this gate to recognise is not, so the branch and its self-test stay.
BINDING_GET_RE = re.compile(
    r"(?:"
    r"let\s+(?P<bind>[a-z_][A-Za-z0-9_]*)\s*=\s*[^;=]*?"
    r'\.\s*get\(\s*"(?P<field>[A-Za-z0-9_]+)"\s*\)'
    r"(?:" + AND_THEN_LINK + r")*\s*;"
    r"|"
    r"if\s+let\s+Some\(\s*(?P<bind2>[a-z_][A-Za-z0-9_]*)\s*\)\s*=\s*[^;{]*?"
    r'\.\s*get\(\s*"(?P<field2>[A-Za-z0-9_]+)"\s*\)'
    r"(?:" + AND_THEN_LINK + r")*"
    r")"
)


def binding_comparisons(code: str, bind: str, start: int) -> list[tuple[str, str, int]]:
    """EVERY `<bind> [.as_X()] ==/!= Some(<value>)` after the binding.

    ALL of them, not the first. Returning only the first dropped real pins that
    review round 3 named: `design_catalog.rs` accepts a schema that is EITHER
    `A27_LIVE_CATALOG_SCHEMA` or `A27_LIVE_CATALOG_DESIGNER_SCHEMA`, and
    `world.rs` accepts a `reason_component_status` that is one of three values.
    Each accepted value is its own pin -- a producer emitting any of them
    satisfies the consumer -- so dropping the alternatives meant a producer
    change to a dropped value left every count in this report unmoved.

    Bounded to 2,000 characters after the binding so a local cannot pair itself
    with an unrelated local of the same name further down the file.
    """
    window = code[start : start + 2000]
    # STOP AT A REBINDING of the same name. The character bound alone is not
    # enough: the since-deleted `g2_engine_capture.rs` wrote two consecutive
    # `if let Some(value) = content.get(...)` blocks, so the first binding's
    # window reached the SECOND block's comparison and the gate recorded
    # `effective_vfs_manifest_sha256` as being compared against
    # `witness.localisation_language`. A comparison after a rebinding refers to
    # the new binding, always. Review round 4, finding 1 -- with a literal on
    # the far side it would have fabricated a shipping field pin rather than
    # only a dynamic one.
    rebind = re.search(
        r"(?<![A-Za-z0-9_])(?:let\s+" + re.escape(bind) + r"\s*="
        r"|Some\(\s*" + re.escape(bind) + r"\s*\))",
        window,
    )
    if rebind:
        window = window[: rebind.start()]
    pattern = (
        r"(?<![A-Za-z0-9_])" + re.escape(bind)
        + r"(?:\s*\.\s*as_\w+\(\s*\))?\s*(?P<op>==|!=)\s*Some\(\s*(?P<value>"
        + SOME_PAYLOAD + r")\s*\)"
    )
    return [
        (m.group("op"), m.group("value"), start + m.start())
        for m in re.finditer(pattern, window)
    ]


LOOSE_PIN_RE = re.compile(
    r'\.\s*get\(\s*"([A-Za-z0-9_]+)"\s*\)[\s\S]{0,200}?(?:!=|==)\s*Some\('
)


def literal_of(
    expr: str,
    consts: dict[str, tuple[str, str]],
    aliases: dict[str, str] | None = None,
    local: dict[str, tuple[str, str]] | None = None,
    ambiguous: set[str] | None = None,
    defs_by_name: dict[str, list[tuple[str, str, str]]] | None = None,
) -> tuple[str | None, str, str]:
    # `local` is the already-scoped view of this file's constants for the USE
    # SITE -- see `scoped_consts`, which picks the nearest preceding definition.
    """Resolve a pinned-value expression to (value, kind, note).

    `kind` is one of:
      "str" / "int" / "bool"  a CONSTANT pin -- the value is fixed in source.
      "dynamic"               the consumer pins the field to a value it computed
                              at run time (`.get("frame_hour") != Some(frame_hour)`,
                              `.get("game_id") != Some(game_id)`). This is a real
                              pin and it is parsed, but there is no literal for a
                              producer to mint, so it is a stated category rather
                              than an unresolved one. Calling it unresolved was
                              the first revision's mistake: it put 40-odd
                              perfectly-read consistency checks into the
                              fail-closed bucket and made exit 1 permanent, which
                              is how a gate stops being run.
      "unresolved"            a NAMED constant with no definition anywhere in
                              either workspace. That one still fails closed --
                              it is indistinguishable from a pin whose constant
                              this reader cannot parse.

    Named constants are followed because envelopes spell their hashes and
    versions that way (`MAPC1_EXECUTABLE_SHA256`, `THEATRE_SELECTOR_SCHEMA_VERSION`),
    through one alias hop, and across BOTH workspaces: the extractor imports
    schema constants from `portable_replay` crates, so a per-side table reports
    its own dependencies as undefined.
    """
    aliases = aliases or {}
    local = local or {}
    ambiguous = ambiguous or set()
    defs_by_name = defs_by_name or {}
    expr = expr.strip()
    m = re.fullmatch(r'"((?:[^"\\]|\\.)*)"', expr)
    if m:
        return unescape(m.group(1)), "str", ""
    if expr in ("true", "false"):
        return expr, "bool", ""
    if re.fullmatch(r"-?[0-9_]+", expr):
        return expr.replace("_", ""), "int", ""
    # A cast or conversion wrapper around a constant is still a constant pin.
    # `world.rs` writes `== Some(u64::from(SCALE))` and `== Some(DECIMALS as u64)`;
    # a reader that did not unwrap those left real pins in the fail-closed
    # census instead of in the measured set (review round 1, finding 3).
    cast = re.fullmatch(r"(?:[A-Za-z0-9_]+\s*::\s*)?from\(\s*([^()]*?)\s*\)", expr)
    if cast:
        return literal_of(cast.group(1), consts, aliases, local, ambiguous,
                          defs_by_name)
    cast = re.fullmatch(r"(.+?)\s+as\s+[A-Za-z0-9_]+", expr)
    if cast:
        return literal_of(cast.group(1), consts, aliases, local, ambiguous,
                          defs_by_name)

    # Rust's primitive associated constants are LANGUAGE facts with exact values,
    # not project constants, and treating them as undefined names put a real
    # `world.rs` pin into the fail-closed bucket. Computed rather than listed:
    # the widths come from the type name.
    prim = re.fullmatch(r"(?P<ty>[iu](?:8|16|32|64|128|size))\s*::\s*(?P<name>MIN|MAX|BITS)", expr)
    if prim:
        width = 64 if prim.group("ty").endswith("size") else int(prim.group("ty")[1:])
        signed = prim.group("ty").startswith("i")
        if prim.group("name") == "BITS":
            return str(width), "int", "Rust primitive constant"
        if signed:
            value = -(2 ** (width - 1)) if prim.group("name") == "MIN" else 2 ** (width - 1) - 1
        else:
            value = 0 if prim.group("name") == "MIN" else 2 ** width - 1
        return str(value), "int", "Rust primitive constant"

    tail = expr.split("::")[-1].strip()
    if re.fullmatch(r"[A-Z][A-Z0-9_]*", tail):
        # MODULE SCOPE FIRST, then the tree, and only when the tree agrees.
        #
        # Rust constants are module-scoped and this tree reuses the obvious
        # names heavily: `AUTHORITY`, `SOURCE`, `ORDERING` and `MANIFEST_SHA256`
        # are each defined many times with different values. A single flat table
        # keyed on the bare name reported one of those definitions for all of
        # them -- review round 1 measured `world.rs`'s local
        # `AUTHORITY = "exact_a729_cdivision_cached_attrition_badge_law"` being
        # recorded as `exact_a729_passive_all_country_decisions` and called
        # consistent against an unrelated sidecar. A wrong value is worse than
        # no value: it manufactures both false consistency and false drift.
        #
        # So: the using FILE's own definition wins; failing that the tree-wide
        # table is used only when every definition of that name agrees; and a
        # name that is defined in more than one place with more than one value,
        # and not defined locally, is UNRESOLVED and fails closed.
        # A QUALIFIED path is NOT resolved by this reader. It records the
        # module the constant comes from, and following that soundly means
        # implementing Rust's name resolution -- crate prefixes, `mod.rs`
        # directories, crate roots whose name differs from their directory,
        # `pub use` facades, ALIASED re-exports (`pub use x::A as B`), and `use`
        # aliases in the using file. Five consecutive review rounds each found a
        # shape the previous approximation got wrong, twice by resolving a pin to
        # ANOTHER MODULE'S VALUE, which is the worst thing this gate can do.
        #
        # So it is not approximated. A qualified path whose tail is ambiguous
        # tree-wide is reported in `unresolved_constant`, by site, with the
        # expression -- a NAMED limitation of this reader rather than a guess and
        # rather than silence. When the tail is unambiguous the qualification is
        # redundant and the bare rule below resolves it exactly.
        if "::" in expr and tail in ambiguous:
            return (
                None,
                "unresolved_constant",
                f"{expr} is a qualified path and {tail} is defined more than once with "
                f"different values; this reader does not implement Rust name resolution "
                f"and will not guess which module is meant",
            )

        # BARE name: the using file's own definition, then the tree-wide table
        # when every definition of that name agrees, then fail closed.
        seen = {tail}
        name = tail
        while name not in local and name not in consts and name in aliases and aliases[name] not in seen:
            name = aliases[name]
            seen.add(name)
        if name in local:
            value, kind = local[name]
            note = f"resolved through const {tail} defined in this file"
            if name != tail:
                note += f" -> {name}"
            return value, kind, note
        if name in ambiguous:
            return (
                None,
                "unresolved",
                f"bare constant {tail} is defined more than once with different values "
                f"and not in this file; module scope cannot be resolved lexically",
            )
        if name in consts:
            value, kind = consts[name]
            note = f"resolved through const {tail}"
            if name != tail:
                note += f" -> {name}"
            return value, kind, note

        return None, "unresolved", f"named constant {tail} has no constant definition in either workspace"
    return None, "dynamic", f"expected value is computed at run time: {expr[:80]}"


# ---------------------------------------------------------------------------
# git plumbing
# ---------------------------------------------------------------------------


def git(args: list[str], allow_one: bool = False) -> str:
    done = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True)
    if done.returncode > (1 if allow_one else 0):
        raise SystemExit(
            "git %s failed (%d): %s"
            % (args[0], done.returncode, done.stderr.decode("utf-8", "replace"))
        )
    return (done.stdout or b"").decode("utf-8", "replace")


def source_paths(scope: tuple[list[str], list[str]]) -> list[str]:
    """Every source file at HEAD under a scope, minus its exclusions.

    `git ls-tree` exits 0 with an EMPTY listing for a pathspec that matches
    nothing, exactly as it does for a directory that is genuinely empty, so a
    scope typo would otherwise become a silently empty vocabulary and a gate
    that reports everything as one-sided. Each include AND each exclude is
    therefore required to match something: an exclusion that has stopped
    matching -- because a directory moved -- is how `runs/` got back into the
    producer vocabulary the first time (finding GP-F11).
    """
    includes, excludes = scope
    listing = git(["ls-tree", "-r", "HEAD", "--name-only", "--", *includes]).splitlines()
    for include in includes:
        if not any(path.startswith(include) for path in listing):
            raise SystemExit(
                "scope include %r matched nothing at HEAD -- a typo here becomes an "
                "empty vocabulary and a gate that cannot fail" % include
            )
    for exclude in excludes:
        if not any(path.startswith(exclude) for path in listing):
            raise SystemExit(
                "scope exclusion %r matched nothing at HEAD -- an exclusion that has "
                "stopped matching is silently inert, which is finding GP-F11 exactly"
                % exclude
            )
    return [
        path
        for path in listing
        if path.endswith(SOURCE_SUFFIXES)
        and not any(path.startswith(exclude) for exclude in excludes)
    ]


def read_blobs(paths: list[str]) -> dict[str, str]:
    """Stream every blob at HEAD in one `git cat-file --batch`."""
    if not paths:
        return {}
    request = "".join("HEAD:%s\n" % path for path in paths).encode("utf-8")
    done = subprocess.run(
        ["git", "-C", str(ROOT), "cat-file", "--batch"], input=request, capture_output=True
    )
    if done.returncode != 0:
        raise SystemExit("git cat-file failed: %s" % done.stderr.decode("utf-8", "replace"))
    blob = done.stdout
    texts: dict[str, str] = {}
    pos = 0
    for path in paths:
        nl = blob.index(b"\n", pos)
        header = blob[pos:nl].decode("utf-8", "replace")
        pos = nl + 1
        if header.endswith("missing"):
            raise SystemExit("blob missing at HEAD: %s" % path)
        size = int(header.split()[2])
        texts[path] = blob[pos : pos + size].decode("utf-8", "replace")
        pos += size + 1
    return texts


# ---------------------------------------------------------------------------
# The scan
# ---------------------------------------------------------------------------


def line_of(code: str, offset: int) -> int:
    return code.count("\n", 0, offset) + 1


class SideScan:
    """One workspace side's pin vocabulary, derived from its own source."""

    def __init__(self, name: str, scope: tuple[list[str], list[str]]) -> None:
        self.name = name
        self.paths = source_paths(scope)
        self.texts = read_blobs(self.paths)
        # token -> list of site dicts
        self.tokens: dict[str, list[dict]] = {}
        self.comment_tokens: dict[str, int] = {}
        self.consts: dict[str, tuple[str, str]] = {}
        self.aliases: dict[str, str] = {}
        self.local_consts: dict[str, dict[str, tuple[str, str]]] = {}
        self.ambiguous: set[str] = set()
        self.defs_by_name: dict[str, list[tuple[str, str, str]]] = {}
        self.field_pins: list[dict] = []
        self.unparsed: list[dict] = []
        self.unresolved_constant: list[dict] = []
        self.loose_only: list[dict] = []
        self.dynamic_pins: list[dict] = []
        self._lexed: dict[str, tuple[str, str]] = {}

    def lex_all(self) -> None:
        for path, text in self.texts.items():
            self._lexed[path] = lex_python(text) if path.endswith(".py") else lex(text)

    def scoped_consts(self, path: str, offset: int) -> dict[str, tuple[str, str]]:
        """This file's constants as they stand AT `offset`.

        For each name, the NEAREST PRECEDING definition wins. Rust declares a
        function-local `const` above the code that uses it, so "nearest
        preceding" is the right reading of item scope for every shape in this
        tree, and it is what makes `world.rs`'s five separate `AUTHORITY`
        constants resolve to five different values instead of all to the first.

        STATED BOUND, and it is deliberate rather than overlooked: this is
        lexical proximity, not a scope tree. A `const` declared in function A
        and a use in function B further down, with no definition of that name in
        between, resolves to A's -- Rust would reject that program, so the case
        does not arise in code that compiles, but the reader is not proving it.
        A name used BEFORE any definition in its file falls back to the file's
        single definition when there is exactly one, and fails closed when there
        are several, because then nothing lexical distinguishes them.
        """
        table: dict[str, tuple[str, str]] = {}
        for name, definitions in self.local_consts.get(path, {}).items():
            preceding = [row for row in definitions if row[0] <= offset]
            if preceding:
                _, value, kind = preceding[-1]
            elif len(definitions) == 1:
                _, value, kind = definitions[0]
            else:
                continue
            table[name] = (value, kind)
        return table

    def collect_consts(self) -> None:
        """Build the per-FILE constant tables and the tree-wide one together.

        Each file's table maps a name to EVERY definition of it in that file
        WITH ITS OFFSET, not to one value. Rust `const` is item-scoped and this
        tree declares them inside the function that uses them, so one file
        routinely holds several `AUTHORITY`, `MANIFEST_SHA256` and `ORDERING`
        constants with different values. Keeping the first definition per file --
        which the previous revision did -- reported
        `exact_a729_cdivision_cached_attrition_badge_law` for a pin whose own
        function declares `exact_a729_foreign_homebase_nodes` six thousand lines
        later, and called it consistent against an unrelated producer. Review
        round 2, finding 1: a pin in a later function could then change without
        the reported row changing at all.

        `ambiguous` records every name whose definitions disagree ACROSS files,
        so a use in a file that does not define it fails closed rather than
        picking whichever definition was read first.
        """
        for path, (code, _) in self._lexed.items():
            local: dict[str, list[tuple[int, str, str]]] = {}
            for pattern, kind in (
                (CONST_STR_RE, "str"), (CONST_NUM_RE, "int"), (CONST_BOOL_RE, "bool")
            ):
                for m in pattern.finditer(code):
                    raw = m.group(2)
                    value = unescape(raw) if kind == "str" else raw.replace("_", "")
                    local.setdefault(m.group(1), []).append((m.start(), value, kind))
                    self.defs_by_name.setdefault(m.group(1), []).append((path, value, kind))
                    known = self.consts.get(m.group(1))
                    if known is None:
                        self.consts[m.group(1)] = (value, kind)
                    elif known != (value, kind):
                        self.ambiguous.add(m.group(1))
            for definitions in local.values():
                definitions.sort()
            self.local_consts[path] = local
            for m in CONST_ALIAS_RE.finditer(code):
                self.aliases.setdefault(m.group(1), m.group(2).split("::")[-1].strip())

    def collect_tokens(self) -> None:
        for path, (code, comment) in self._lexed.items():
            test_spans = cfg_test_spans_for(path, code)
            for value, offset, via in self.spelled_values(path, code):
                cls = classify_literal(value)
                if cls is None:
                    continue
                row = {
                    "class": cls,
                    "site": "%s:%d" % (path, line_of(code, offset)),
                    "test": is_test_path(path) or in_spans(offset, test_spans),
                }
                if via:
                    row["via_const"] = via
                self.tokens.setdefault(value, []).append(row)
            # Comment-only tokens: a `//!` header naming an authority a file has
            # not implemented must never mint it, but it must not vanish either.
            for m in re.finditer(r"[A-Za-z0-9_.\-]*a729[A-Za-z0-9_.\-]*", comment):
                cls = classify_literal(m.group(0))
                if cls is not None:
                    self.comment_tokens[m.group(0)] = self.comment_tokens.get(m.group(0), 0) + 1

    def spelled_values(self, path: str, code: str):
        """Every constant STRING VALUE this file spells, and where.

        Yields `(value, offset, via_const_or_None)`.

        Two ways a file spells a value, and only counting the first was a
        measured over-report. `recorder_host/src/main.rs` mints the portable
        design-catalog authority by REFERENCING `PORTABLE_DESIGN_CATALOG_SCHEMA`,
        a `pub const` that lives in `portable_replay/native/snapshot`, because
        the extractor depends on that crate. A scan that only saw string
        literals found no producer spelling for `hoi4.portable-design-catalog.v3`
        and reported the consumer's pin on it as unminted -- a false drift row,
        which costs a gate its credibility faster than a missed one.

        Only ALL-CAPS identifiers are followed, and only through the tree-wide
        constant table built from both workspaces, so a reference resolves to
        exactly the literal a compiler would substitute.
        """
        for value, offset in string_literals_for(path, code):
            yield value, offset, None
        for m in CONST_REF_RE.finditer(code):
            name = m.group(1)
            if (name not in self.consts and name not in self.aliases
                    and name not in self.local_consts.get(path, {})):
                continue
            value, kind, _ = literal_of(
                name, self.consts, self.aliases,
                self.scoped_consts(path, m.start()), self.ambiguous,
                    self.defs_by_name)
            if kind == "str" and value:
                yield value, m.start(), name

    def collect_field_pins(self) -> None:
        for path, (code, _) in self._lexed.items():
            test_spans = cfg_test_spans_for(path, code)
            helpers: dict[str, int] = {}
            for m in EXACT_HELPER_RE.finditer(code):
                name = m.group(1) or m.group(4)
                key_param = m.group(2) or m.group(5)
                expected_param = m.group(3) or m.group(6)
                body = m.group(7)
                # A pin helper READS the key out of a map AND COMPARES the
                # result to the expected parameter. Both halves are load-bearing
                # and each was put here by a measured failure:
                #
                #   * `.get(<key_param>)` literally, not just `.get(` -- the
                #     first revision admitted `path_is_under(path, prefix)`,
                #     whose body is `path.get(..prefix.len())`, a slice range,
                #     and `validate_hash(field, value)`.
                #   * the COMPARISON, not merely an occurrence of the expected
                #     parameter -- review round 1 found `dump_map_labels.rs`'s
                #     `let file = |key: &str, default: &str|`, an ordinary
                #     manifest loader ending `.unwrap_or(default)`. It was
                #     classified as an authority helper and five plain filename
                #     defaults entered the report as authority pins.
                squashed = re.sub(r"\s+", "", body)
                compares = (
                    ("Some(%s)" % expected_param) in squashed
                    or ("==%s" % expected_param) in squashed
                    or ("!=%s" % expected_param) in squashed
                )
                if ("get(%s)" % key_param) in squashed and compares:
                    helpers.setdefault(name, m.start())
            parsed_from: set[str] = set()
            strict_offsets: set[int] = set()
            let_bound_offsets: set[int] = set()
            measured_lines: set[int] = set()
            for m in PIN_SHAPE_CALL_RE.finditer(code):
                helper, field_lit, expr = m.group(1), m.group(2), m.group(3).strip()
                if helper not in helpers:
                    continue
                line = line_of(code, m.start())
                site = "%s:%d" % (path, line)
                testish = is_test_path(path) or in_spans(m.start(), test_spans)
                field = unescape(field_lit[1:-1])
                value, kind, vnote = literal_of(
                    expr, self.consts, self.aliases,
                    self.scoped_consts(path, m.start()), self.ambiguous,
                    self.defs_by_name)
                if kind == "unresolved_constant":
                    self.unresolved_constant.append(
                        {"site": site, "field": field, "reason": vnote, "test": testish}
                    )
                    continue
                if kind == "unresolved":
                    self.unparsed.append(
                        {"site": site, "helper": helper, "field": field,
                         "reason": vnote, "test": testish}
                    )
                    continue
                parsed_from.add(helper)
                if kind == "dynamic":
                    self.dynamic_pins.append(
                        {"field": field, "site": site, "spelling": "helper:%s" % helper,
                         "note": vnote, "test": testish}
                    )
                    continue
                self.field_pins.append(
                    {"field": field, "value": value, "kind": kind, "site": site,
                     "spelling": "helper:%s" % helper, "note": vnote, "test": testish}
                )
            # A pin helper that exists and yields nothing means the CALL reader
            # has gone blind while the helper reader still works -- the exact
            # half-broken state a gate must not report as clean.
            for helper, offset in helpers.items():
                if helper in parsed_from:
                    continue
                self.unparsed.append(
                    {
                        "site": "%s:%d" % (path, line_of(code, offset)),
                        "helper": helper,
                        "reason": "a two-&str pin helper %r is defined here and no call to it "
                                  "parsed into a (field, value) pin" % helper,
                        "test": is_test_path(path) or in_spans(offset, test_spans),
                    }
                )
            for m in DIRECT_PIN_RE.finditer(code):
                # The LAST `get("...")` and the LAST `as_X` inside the matched
                # span, not the first: in a nested read the pinned field is the
                # inner one, and naming the outer one would report the envelope
                # instead of the pin.
                span = m.group(0)
                gets = re.findall(r'get\(\s*"([A-Za-z0-9_]+)"\s*\)', span)
                field = gets[-1] if gets else m.group("field")
                accs = re.findall(r"as_(\w+)", span)
                accessor = accs[-1] if accs else "unknown"
                op, expr = m.group("op"), m.group("value")
                line = line_of(code, m.start())
                site = "%s:%d" % (path, line)
                testish = is_test_path(path) or in_spans(m.start(), test_spans)
                strict_offsets.add(m.start())
                for line_no in range(line, line_of(code, m.end()) + 1):
                    measured_lines.add(line_no)
                value, kind, vnote = literal_of(
                    expr, self.consts, self.aliases,
                    self.scoped_consts(path, m.start()), self.ambiguous,
                    self.defs_by_name)
                if kind == "unresolved_constant":
                    self.unresolved_constant.append(
                        {"site": site, "field": field, "reason": vnote, "test": testish}
                    )
                    continue
                if kind == "unresolved":
                    self.unparsed.append(
                        {"site": site, "helper": "inline", "field": field,
                         "reason": vnote, "test": testish}
                    )
                    continue
                if kind == "dynamic":
                    self.dynamic_pins.append(
                        {"field": field, "site": site,
                         "spelling": "inline:as_%s %s" % (accessor, op),
                         "note": vnote, "test": testish}
                    )
                    continue
                self.field_pins.append(
                    {"field": field, "value": value, "kind": kind, "site": site,
                     "spelling": "inline:as_%s %s" % (accessor, op), "note": vnote,
                     "test": testish}
                )
            for m in BINDING_GET_RE.finditer(code):
                bind = m.group("bind") or m.group("bind2")
                # The LAST `get("...")` in the binding's own chain, exactly as
                # the inline reader does. A binding can carry a nested read too:
                # `snapshot/src/container.rs` binds
                # `manifest.get("build").and_then(|b| b.get("exe_sha256"))` and
                # `rollover_package_set.rs` binds
                # `.get("campaign_identity")…get("observed_game_hour_raw")`.
                # Taking the first named the ENVELOPE, so a producer change to
                # the inner field moved no correctly-named row, and the inner
                # field stayed in the census. Review round 4, finding 2.
                gets = re.findall(r'get\(\s*"([A-Za-z0-9_]+)"\s*\)', m.group(0))
                field = gets[-1] if gets else (m.group("field") or m.group("field2"))
                site = "%s:%d" % (path, line_of(code, m.start()))
                testish = is_test_path(path) or in_spans(m.start(), test_spans)
                scoped = self.scoped_consts(path, m.start())
                for op, expr, at in binding_comparisons(code, bind, m.end()):
                    if at in strict_offsets:
                        continue
                    value, kind, vnote = literal_of(
                        expr, self.consts, self.aliases, scoped, self.ambiguous)
                    if kind == "unresolved_constant":
                        self.unresolved_constant.append(
                            {"site": site, "field": field, "reason": vnote, "test": testish}
                        )
                        continue
                    if kind == "unresolved":
                        self.unparsed.append(
                            {"site": site, "helper": "binding", "field": field,
                             "reason": vnote, "test": testish}
                        )
                        continue
                    let_bound_offsets.add(m.start())
                    # EVERY line the binding's chain and its comparison cover,
                    # not just the two endpoints. A multi-line `.get()` chain
                    # left its middle lines in the census, so the same pin was
                    # reported as measured AND as unread -- which made the
                    # dossier's "none of the census is read by a strict reader"
                    # false. Review round 4, finding 3.
                    for line_no in range(line_of(code, m.start()),
                                         line_of(code, at) + 1):
                        measured_lines.add(line_no)
                    if kind == "dynamic":
                        self.dynamic_pins.append(
                            {"field": field, "site": site,
                             "spelling": "binding %s" % op,
                             "note": vnote, "test": testish}
                        )
                        continue
                    self.field_pins.append(
                        {"field": field, "value": value, "kind": kind, "site": site,
                         "spelling": "binding %s" % op, "note": vnote,
                         "test": testish}
                    )
            for m in LOOSE_PIN_RE.finditer(code):
                # De-duplicate on the LINE, not on the byte offset. A let-bound
                # read and the loose pattern match at different offsets on the
                # same statement, so an offset test left `source.rs`'s `format`
                # pin listed as unread on the very line the let-bound reader had
                # just measured. A census that lists measured sites is a census
                # nobody can act on.
                if (m.start() in strict_offsets or m.start() in let_bound_offsets
                        or line_of(code, m.start()) in measured_lines):
                    continue
                self.loose_only.append(
                    {
                        "site": "%s:%d" % (path, line_of(code, m.start())),
                        "field": m.group(1),
                        "test": is_test_path(path) or in_spans(m.start(), test_spans),
                    }
                )

    def run(self) -> None:
        """Single-side run. `scan_both` is what production uses -- see there for
        why the constant table cannot be per-side."""
        self.lex_all()
        self.collect_consts()
        self.collect_tokens()
        self.collect_field_pins()

    # -- queries used by the diff --------------------------------------------

    def code_sites(self, token: str, include_tests: bool = True) -> list[dict]:
        rows = self.tokens.get(token, [])
        return rows if include_tests else [row for row in rows if not row["test"]]


# ---------------------------------------------------------------------------
# self-test -- one named case per shape a review round or a lane finding
# constructed. Cases are NAMED, never numbered, and the count is printed rather
# than written down.
# ---------------------------------------------------------------------------


def self_test() -> int:
    cases = 0

    def check(name: str, got, want) -> None:
        nonlocal cases
        cases += 1
        if got != want:
            raise SystemExit("self-test case %r failed: got %r, want %r" % (name, got, want))

    # -- classify_literal ----------------------------------------------------
    check("old gate's own class still classifies",
          classify_literal("exact_a729_passive_all_country_politics"), "name")
    check("IO-F6 G4 authority is now visible",
          classify_literal("a729_ctechnology_status_researched_vector_v1"), "name")
    check("IO-F6 X9S1 authority is now visible",
          classify_literal("a729_country_technology_ui_state_classifier_v1"), "name")
    check("gap map GP-U6's prose exception is now visible",
          classify_literal("a729_xui16_occupation_ui_supplement_v1"), "name")
    check("lane IW xmap20 schema string is visible",
          classify_literal("hoi4.xmap20.effect-frame.a729.v1"), "schema_string")
    check("lane IW xmap09 schema string is visible",
          classify_literal("hoi4.xmap09.semantic-lighting-input.a729.v1"), "schema_string")
    check("dotted schema string without the game prefix is visible",
          classify_literal("replay.xui12.production-lines-authority.a729.v2"), "schema_string")
    check("a NUL-terminated domain prefix keeps its class",
          classify_literal("hoi4.a729.present-consumed-rolling.v2\0"), "schema_string")
    check("the bare build tag is not an authority", classify_literal("a729"), None)
    check("an engine version string is not an authority",
          classify_literal("1.19.2.0.a729"), None)
    check("a hyphenated build tag is not an authority", classify_literal("hoi4-a729"), None)
    check("a source file name is not an authority", classify_literal("a729_hook.rs"), None)
    check("a glob path is not an authority", classify_literal("interface/*.gui"), None)
    check("a token with no build tag is not in this vocabulary",
          classify_literal("exact_passive_all_country_politics"), None)

    # -- lexing --------------------------------------------------------------
    src = 'let a = "exact_a729_alpha"; // exact_a729_beta\n'
    code, comment = lex(src)
    check("comment text is not code", "exact_a729_beta" in code, False)
    check("comment text is kept for reporting", "exact_a729_beta" in comment, True)
    check("code literal survives lexing", "exact_a729_alpha" in code, True)
    check("lexing preserves offsets", len(code), len(src))

    nested = 'A/* outer /* inner */ still comment */B"exact_a729_gamma"\n'
    code, _ = lex(nested)
    check("nested block comments do not leak code",
          "still comment" in code, False)
    check("code after a nested block comment is still read",
          "exact_a729_gamma" in code, True)

    globby = 'let p = "interface/*.gui"; let q = "exact_a729_delta";\n'
    code, _ = lex(globby)
    check("a glob in a string does not open a comment",
          "exact_a729_delta" in code, True)

    quoted_char = "const Q: char = '\"'; let r = \"exact_a729_epsilon\";\n"
    code, _ = lex(quoted_char)
    check("a quote char literal does not open a string",
          "exact_a729_epsilon" in code, True)

    rawstr = 'let r = r#"has " quote"#; let s = "exact_a729_zeta";\n'
    code, _ = lex(rawstr)
    check("a raw string with an embedded quote does not desync the lexer",
          "exact_a729_zeta" in code, True)

    check("NUL escape is part of the pinned value",
          unescape("hoi4.a729.x.v1\\0"), "hoi4.a729.x.v1\0")

    # -- Python lexing (the fourth blindness instance) ------------------------
    py = "SCHEMA = 'extractor.ui14_engine_vfs.v1'  # exact_a729_commented\n"
    pcode, pcomment = lex_python(py)
    check("a python literal is code", "extractor.ui14_engine_vfs.v1" in pcode, True)
    check("a python # comment is not code", "exact_a729_commented" in pcode, False)
    check("a python # comment is kept for reporting",
          "exact_a729_commented" in pcomment, True)
    check("python lexing preserves offsets", len(pcode), len(py))

    doc = '"""Module doc naming exact_a729_unbuilt."""\nX = "exact_a729_built"\n'
    dcode, dcomment = lex_python(doc)
    check("a python docstring does not mint an authority",
          "exact_a729_unbuilt" in dcode, False)
    check("a python docstring is kept for reporting",
          "exact_a729_unbuilt" in dcomment, True)
    check("a python assignment after a docstring is still code",
          "exact_a729_built" in dcode, True)

    hashy = "X = '#not-a-comment'\nY = 'exact_a729_after_hash'\n"
    hcode, _ = lex_python(hashy)
    check("a hash inside a python string does not open a comment",
          "exact_a729_after_hash" in hcode, True)

    pl = dict(string_literals_python("a = 'one'\nb = \"two\"\nc = f'{x}three'\n"))
    check("python literals are extracted and f-strings are not pins",
          sorted(pl), ["one", "two"])

    # -- string extraction ---------------------------------------------------
    lits = dict(string_literals('let a = "one"; let b = r"two"; let c = b"three";'))
    check("cooked, raw and byte literals are all extracted",
          sorted(lits), ["one", "three", "two"])

    # -- minting through a shared constant ------------------------------------
    class _Side:
        pass
    side = _Side()
    side.consts = {"PORTABLE_DESIGN_CATALOG_SCHEMA": ("hoi4.portable-design-catalog.v3", "str")}
    side.aliases = {}
    side.local_consts = {}
    side.ambiguous = set()
    side.defs_by_name = {}
    side.scoped_consts = SideScan.scoped_consts.__get__(side, _Side)
    side.spelled_values = SideScan.spelled_values.__get__(side, _Side)
    spelled = {value for value, _, _ in
               side.spelled_values("x.rs", "let s = PORTABLE_DESIGN_CATALOG_SCHEMA;")}
    check("a value minted through a shared constant counts as spelled",
          "hoi4.portable-design-catalog.v3" in spelled, True)

    # -- cfg(test) flagging (finding GP-F17) --------------------------------
    tsrc = 'fn a() {}\n#[cfg(test)]\nmod t { fn f() { let x = "exact_a729_fixture"; } }\n'
    tcode, _ = lex(tsrc)
    spans = cfg_test_spans(tcode)
    off = tcode.index('"exact_a729_fixture"')
    check("a literal inside #[cfg(test)] is flagged", in_spans(off, spans), True)
    check("a literal outside #[cfg(test)] is not flagged", in_spans(0, spans), False)

    # -- field_pin parsing (lane IZ's class) ---------------------------------
    consts = {"MAPC1_EXECUTABLE_SHA256": ("870ebf", "str"),
              "THEATRE_SELECTOR_SCHEMA_VERSION": ("6", "int")}
    check("an inline pinned string resolves",
          literal_of('"SPU1"', consts), ("SPU1", "str", ""))
    check("a pinned bool resolves", literal_of("false", consts)[:2], ("false", "bool"))
    check("a pinned integer resolves", literal_of("1", consts)[:2], ("1", "int"))
    check("a pinned named constant is followed",
          literal_of("MAPC1_EXECUTABLE_SHA256", consts)[:2], ("870ebf", "str"))
    check("a pinned numeric constant is followed too",
          literal_of("THEATRE_SELECTOR_SCHEMA_VERSION", consts)[:2], ("6", "int"))
    check("a constant alias is followed one hop",
          literal_of("ALIAS", consts, {"ALIAS": "MAPC1_EXECUTABLE_SHA256"})[:2],
          ("870ebf", "str"))
    check("a Rust primitive MIN resolves rather than failing closed",
          literal_of("i32::MIN", {})[:2], ("-2147483648", "int"))
    check("a Rust primitive MAX resolves too",
          literal_of("u8::MAX", {})[:2], ("255", "int"))
    check("a usize MIN is treated as 64-bit on this target",
          literal_of("usize::MIN", {})[:2], ("0", "int"))
    spu1 = (
        'let exact = |key: &str, expected: &str| authority.get(key)'
        '.and_then(Value::as_str) == Some(expected);\n'
        'if !exact("schema", "SPU1")\n'
        '  || authority.get("air_wings_available").and_then(Value::as_bool) != Some(false)\n'
        '{ }\n'
    )
    m = EXACT_HELPER_RE.search(spu1)
    check("the SPU1 pin helper is found by shape, not by name", bool(m), True)
    check("the helper's binding name is captured", m.group(1) if m else None, "exact")
    dm = DIRECT_PIN_RE.search(spu1)
    check("the SPU1 bool field pin is found",
          dm.group("field") if dm else None, "air_wings_available")
    check("the SPU1 bool field pin keeps its value",
          dm.group("value") if dm else None, "false")

    closure_form = (
        'if receipt.get("schema").and_then(|v| v.as_str()) != Some("extractor.ui14_engine_vfs.v1") {'
    )
    cm = DIRECT_PIN_RE.search(closure_form)
    check("a closure accessor pin is read, not only a path accessor",
          (cm.group("field"), re.findall(r"as_(\w+)", cm.group(0))[-1],
           cm.group("value")) if cm else None,
          ("schema", "str", '"extractor.ui14_engine_vfs.v1"'))

    split = (
        'authority\n    .get("schema_version")\n'
        '    .and_then(serde_json::Value::as_u64)\n    != Some(1)\n'
    )
    sm = DIRECT_PIN_RE.search(split)
    check("a rustfmt-split pin is still one pin",
          sm.group("field") if sm else None, "schema_version")

    # -- review round 1's five findings, one named case each -----------------
    scoped = {"AUTHORITY": ("exact_a729_passive_all_country_decisions", "str")}
    localdef = {"AUTHORITY": ("exact_a729_cdivision_cached_attrition_badge_law", "str")}
    check("a constant defined in the using file wins over the tree-wide table",
          literal_of("AUTHORITY", scoped, {}, localdef)[0],
          "exact_a729_cdivision_cached_attrition_badge_law")
    check("a constant whose definitions disagree and is not local fails closed",
          literal_of("AUTHORITY", scoped, {}, {}, {"AUTHORITY"})[1], "unresolved")
    check("an unambiguous tree-wide constant still resolves",
          literal_of("AUTHORITY", scoped, {}, {}, set())[0],
          "exact_a729_passive_all_country_decisions")

    consts_cast = {"SCALE": ("100000", "int")}
    check("a from() cast around a constant is still a constant pin",
          literal_of("u64::from(SCALE)", consts_cast)[:2], ("100000", "int"))
    check("an as-cast around a constant is still a constant pin",
          literal_of("SCALE as u64", consts_cast)[:2], ("100000", "int"))
    fixed = 'authority.get("fixed_scale").and_then(Value::as_u64) == Some(u64::from(SCALE))'
    fm = DIRECT_PIN_RE.search(fixed)
    check("world.rs's fixed_scale pin is read by the STRICT reader, not the loose census",
          (fm.group("field"), fm.group("value")) if fm else None,
          ("fixed_scale", "u64::from(SCALE)"))
    digest = ('authority.get("state_owner_sha256").and_then(serde_json::Value::as_str)'
              ' != Some(state_owner_sha256.as_str())')
    dgm = DIRECT_PIN_RE.search(digest)
    check("MAPC1's digest pin is read by the STRICT reader",
          (dgm.group("field"), dgm.group("value")) if dgm else None,
          ("state_owner_sha256", "state_owner_sha256.as_str()"))

    loader = (
        "let file = |key: &str, default: &str| -> String {\n"
        "    manifest.get(\"files\").and_then(|f| f.get(key))\n"
        "        .and_then(|v| v.as_str()).unwrap_or(default).to_string()\n};\n"
    )
    lm = EXACT_HELPER_RE.search(loader)
    body = lm.group(7) if lm else ""
    squashed = re.sub(r"\s+", "", body)
    admitted = ("get(key)" in squashed and (
        "Some(default)" in squashed or "==default" in squashed or "!=default" in squashed))
    check("a manifest loader that never compares its default is not a pin helper",
          admitted, False)
    pin_helper = ('let exact = |key: &str, expected: &str| '
                  'authority.get(key).and_then(Value::as_str) == Some(expected);')
    pm2 = EXACT_HELPER_RE.search(pin_helper)
    body2 = re.sub(r"\s+", "", pm2.group(7)) if pm2 else ""
    check("a real pin helper is still admitted",
          "get(key)" in body2 and "Some(expected)" in body2, True)

    # -- review round 2's findings, one named case each ----------------------
    class _Scoped:
        pass
    sc = _Scoped()
    sc.local_consts = {"world.rs": {"AUTHORITY": [
        (100, "exact_a729_cdivision_cached_attrition_badge_law", "str"),
        (900, "exact_a729_foreign_homebase_nodes", "str"),
    ]}}
    sc.scoped_consts = SideScan.scoped_consts.__get__(sc, _Scoped)
    check("a use before the second definition sees the FIRST constant",
          sc.scoped_consts("world.rs", 500)["AUTHORITY"][0],
          "exact_a729_cdivision_cached_attrition_badge_law")
    check("a use after the second definition sees the SECOND constant",
          sc.scoped_consts("world.rs", 1000)["AUTHORITY"][0],
          "exact_a729_foreign_homebase_nodes")
    check("a use before EVERY definition of a multiply-defined name fails closed",
          "AUTHORITY" in sc.scoped_consts("world.rs", 10), False)
    sc.local_consts = {"one.rs": {"SOLO": [(900, "only", "str")]}}
    check("a use before the single definition in its file still resolves",
          sc.scoped_consts("one.rs", 10)["SOLO"][0], "only")

    letbound = (
        'let format = timeline.get("format").and_then(Value::as_str);\n'
        'let role = timeline.get("raw_stream_role").and_then(Value::as_str);\n'
        'let declares_compact = format == Some(COMPACT_STREAM_TIMELINE_FORMAT)\n'
        '    || role == Some(COMPACT_STREAM_ROLE);\n'
    )
    binds = [(m.group("field") or m.group("field2"),
              m.group("bind") or m.group("bind2"), m.end())
             for m in BINDING_GET_RE.finditer(letbound)]
    check("both let-bound field reads are found",
          [field for field, _, _ in binds], ["format", "raw_stream_role"])
    check("source.rs's compact-stream format pin is paired with its comparison",
          binding_comparisons(letbound, binds[0][1], binds[0][2])[0][1],
          "COMPACT_STREAM_TIMELINE_FORMAT")
    check("source.rs's raw_stream_role pin is paired with its comparison",
          binding_comparisons(letbound, binds[1][1], binds[1][2])[0][1],
          "COMPACT_STREAM_ROLE")
    no_comparison = 'let x = m.get("f").and_then(Value::as_str);\nuse(x);\n'
    check("a let-bound read with no later comparison is not a pin",
          binding_comparisons(no_comparison, "x", 0), [])

    # -- review round 3's findings, one named case each ----------------------
    alts = (
        'let schema = value.get("schema").and_then(Value::as_str);\n'
        'if (schema != Some(A27_LIVE_CATALOG_SCHEMA)\n'
        '    && schema != Some(A27_LIVE_CATALOG_DESIGNER_SCHEMA)) { }\n'
    )
    am = BINDING_GET_RE.search(alts)
    check("EVERY accepted value of an alternation is a pin, not just the first",
          [value for _, value, _ in binding_comparisons(alts, am.group("bind"), am.end())],
          ["A27_LIVE_CATALOG_SCHEMA", "A27_LIVE_CATALOG_DESIGNER_SCHEMA"])

    three = (
        'let status = row.get("reason_component_status").and_then(Value::as_str);\n'
        'let ok = status == Some("a") || status == Some(REASON_STATUS)\n'
        '    || status == Some("c");\n'
    )
    tm = BINDING_GET_RE.search(three)
    check("all three accepted values of world.rs's status pin are read",
          len(binding_comparisons(three, tm.group("bind"), tm.end())), 3)

    iflet = (
        'if let Some(value) = content.get("effective_vfs_manifest_sha256") {\n'
        '    if value.as_str() != Some(witness.effective_vfs_manifest_sha256.as_str()) { }\n'
        '}\n'
    )
    im = BINDING_GET_RE.search(iflet)
    check("the if-let binding form is a recognised pin",
          im.group("field2") if im else None, "effective_vfs_manifest_sha256")
    check("its comparison, with the accessor at the COMPARISON, is paired",
          len(binding_comparisons(iflet, im.group("bind2"), im.end())) if im else 0, 1)

    nested = (
        'if object.get("campaign_identity").and_then(Value::as_object)\n'
        '    .and_then(|identity| identity.get("campaign_id"))\n'
        '    .and_then(Value::as_str) != Some(segment.campaign_id_hex.as_str()) { }\n'
    )
    nm = DIRECT_PIN_RE.search(nested)
    check("a NESTED accessor chain is read at all", bool(nm), True)
    check("a nested read names the INNER field, not the envelope",
          re.findall(r'get\(\s*"([A-Za-z0-9_]+)"\s*\)', nm.group(0))[-1] if nm else None,
          "campaign_id")


    # -- review round 4's findings, one named case each ----------------------
    rebound = (
        'if let Some(value) = content.get("effective_vfs_manifest_sha256") {\n'
        '    if value.as_str() != Some(witness.effective_vfs_manifest_sha256.as_str()) { }\n'
        '}\n'
        'if let Some(value) = content.get("localisation_language") {\n'
        '    if value.as_str() != Some(witness.localisation_language.as_str()) { }\n'
        '}\n'
    )
    rb = list(BINDING_GET_RE.finditer(rebound))
    check("two consecutive if-let blocks are two bindings",
          [m.group("field2") for m in rb],
          ["effective_vfs_manifest_sha256", "localisation_language"])
    check("a binding does NOT reach past a rebinding of its own name",
          len(binding_comparisons(rebound, rb[0].group("bind2"), rb[0].end())), 1)
    check("the second binding still finds its own comparison",
          len(binding_comparisons(rebound, rb[1].group("bind2"), rb[1].end())), 1)

    nested_bind = (
        'let build_sha = manifest\n'
        '    .get("build")\n'
        '    .and_then(|b| b.get("exe_sha256"))\n'
        '    .and_then(Value::as_str);\n'
    )
    nb = BINDING_GET_RE.search(nested_bind)
    check("a nested BINDING chain names the inner field, not the envelope",
          re.findall(r'get\(\s*"([A-Za-z0-9_]+)"\s*\)', nb.group(0))[-1] if nb else None,
          "exe_sha256")

    boundary_bind = (
        'let observed_boundary = object\n'
        '    .get("campaign_identity")\n'
        '    .and_then(serde_json::Value::as_object)\n'
        '    .and_then(|identity| identity.get("observed_game_hour_raw"))\n'
        '    .and_then(serde_json::Value::as_i64);\n'
    )
    bb = BINDING_GET_RE.search(boundary_bind)
    check("rollover_package_set's boundary-hour pin names its inner field",
          re.findall(r'get\(\s*"([A-Za-z0-9_]+)"\s*\)', bb.group(0))[-1] if bb else None,
          "observed_game_hour_raw")
    check("a multi-line binding chain spans more than one line",
          bb.group(0).count("\n") > 1 if bb else False, True)


    # -- review round 6's findings, one named case each ----------------------
    bare = 'if embedded_manifest.get("portable_content_manifest") != Some(&container_content_value)\n'
    bm = DIRECT_PIN_RE.search(bare)
    check("a BARE get/Some comparison with no accessor is a recognised pin",
          bm.group("field") if bm else None, "portable_content_manifest")
    check("its payload is carried through for classification",
          bm.group("value").strip() if bm else None, "&container_content_value")


    # -- review round 10's finding, THROUGH THE PRODUCTION MERGE -------------
    # Round 11 found the previous revision of this case building the expected
    # ambiguity set by hand, so it passed while `main`'s merge still missed the
    # disagreement. These stubs now go through `merge_workspace_tables` itself:
    # kills = the cross-workspace comparison being dropped from the production
    # merge, or the merge reverting to a plain union of per-side ambiguity.
    class _Ws:
        def __init__(self, consts, defs):
            self.consts = consts
            self.aliases = {}
            self.ambiguous = set()
            self.defs_by_name = defs
    consumer_ws = _Ws(
        {"PARTICLE_SHADER_SOURCE": ("gfx/FX/particle.shader", "str"),
         "AGREED": ("same", "str")},
        {"PARTICLE_SHADER_SOURCE": [
            ("replay_interface/native/map_viewer/src/weather_law.rs",
             "gfx/FX/particle.shader", "str")]},
    )
    producer_ws = _Ws(
        {"PARTICLE_SHADER_SOURCE": ("gfx/fx/particle.shader", "str"),
         "AGREED": ("same", "str")},
        {"PARTICLE_SHADER_SOURCE": [
            ("extractor/native/lanes/production_render_submit_live/src/weather_catalog.rs",
             "gfx/fx/particle.shader", "str")]},
    )
    cross_consts, _, cross_defs, cross_ambiguous = merge_workspace_tables(
        consumer_ws, producer_ws)
    check("the PRODUCTION merge marks a name defined once per SIDE with "
          "different values as ambiguous",
          "PARTICLE_SHADER_SOURCE" in cross_ambiguous, True)
    check("while a name the sides agree on stays unambiguous",
          "AGREED" in cross_ambiguous, False)
    check("and a pin on the disagreeing name outside its defining file fails "
          "closed rather than taking the other workspace's value",
          literal_of("PARTICLE_SHADER_SOURCE", cross_consts, {}, {},
                     cross_ambiguous, cross_defs)[1],
          "unresolved")
    check("while the file that DEFINES it still resolves locally",
          literal_of("PARTICLE_SHADER_SOURCE", cross_consts, {},
                     {"PARTICLE_SHADER_SOURCE": ("gfx/FX/particle.shader", "str")},
                     cross_ambiguous, cross_defs)[0],
          "gfx/FX/particle.shader")

    # -- surviving behaviour whose self-tests went with the resolver deletion,
    #    restored (round 11 finding 3: the doc claimed other cases covered
    #    them; only bare ambiguity was) --------------------------------------
    check("an unknown constant is reported, not skipped",
          literal_of("NO_SUCH_CONST", {"OTHER": ("1", "int")})[1], "unresolved")
    check("a run-time expected value is a dynamic pin, not an unparsed one",
          literal_of("frame_hour", {"OTHER": ("1", "int")})[1], "dynamic")
    pm = PIN_SHAPE_CALL_RE.search('if !exact("schema", "SPU1") {')
    check("a pin-shaped call splits into field and value",
          (pm.group(1), pm.group(2), pm.group(3)) if pm else None,
          ("exact", '"schema"', '"SPU1"'))
    check("a call whose first argument is not a literal is not pin-shaped",
          PIN_SHAPE_CALL_RE.search("exact(key, expected)"), None)
    check("a one-argument call is not pin-shaped",
          PIN_SHAPE_CALL_RE.search('exact_tree_instance_source("x")'), None)


    check_scope_excludes_this_gate()
    cases += 1

    print("self-test: %d cases, all pass" % cases)
    return cases


def check_scope_excludes_this_gate() -> None:
    """This gate must not be inside the producer scope it measures.

    Asserted rather than trusted. Review round 1 measured three headline
    authorities -- `a729_xui16_occupation_ui_supplement_v1` and lane IW's two
    schema strings -- reading `shared` with their ONLY producer site inside this
    file's own `self_test`, because `extractor/native/scripts/` was in scope and
    Python has no `#[cfg(test)]` for the test-site flag to catch. That is
    finding GP-F11's defect with a different directory in it, so the check is on
    the SCOPE and not on this file's name.
    """
    own = Path(__file__).resolve()
    try:
        rel = own.relative_to(ROOT).as_posix()
    except ValueError:
        return
    includes, excludes = PRODUCER_SCOPE
    inside = any(rel.startswith(item) for item in includes)
    fenced = any(rel.startswith(item) for item in excludes)
    if inside and not fenced:
        raise SystemExit(
            "self-test case 'the gate is not inside its own producer scope' failed: %s is "
            "scanned as producer code, so every authority this file quotes in order to "
            "CHECK it would be counted as minted" % rel
        )


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------


def resolve_out(args) -> Path:
    """Where to write, refusing to overwrite recorded evidence.

    `runs/` is append-only and lanes cite each other's committed numbers, so a
    later lane that simply runs the gate must not silently rewrite them. The
    rule is on the DESTINATION, not on how the path was named: `--out` is not
    an exemption, because naming a path does not imply owning it.
    """
    out = Path(args.out).resolve() if args.out else (ROOT / "authority_pin_gate.json")
    if out.exists() and not args.force:
        where = "append-only evidence" if "/runs/" in out.as_posix() else "an existing file"
        raise SystemExit(
            "refusing to overwrite %s at %s.\n"
            "Write into your own run directory instead:\n"
            "    python %s --out extractor/native/runs/<your-run>/authority_pin_gate.json\n"
            "or pass --force if that file is yours to replace." % (where, out, Path(__file__).name)
        )
    return out


def token_rows(producer: SideScan, consumer: SideScan, cls: str) -> list[dict]:
    # Comments ANNOTATE the vocabulary; they never create it.
    #
    # Comment text is scanned as identifiers, not as string literals, so a
    # prose mention of a Rust function whose name happens to carry the build tag
    # -- `from_exact_a729_observation`, `abort_a729_terminal_custody_and_uninstall`,
    # `install_a729_design_catalog_context_hook` -- is shaped exactly like an
    # authority name. Admitting those invented 17 tokens that no code on either
    # side spells and that no producer could ever be owed for. What finding
    # GP-F11 actually requires is the opposite direction and is preserved below:
    # a token the CONSUMER pins in code and the producer names only in a `//!`
    # header stays `consumer_only`, with its comment sites carried beside it, so
    # the doc comment can never read as minting.
    names = {tok for tok, rows in producer.tokens.items() if rows[0]["class"] == cls}
    names |= {tok for tok, rows in consumer.tokens.items() if rows[0]["class"] == cls}
    rows = []
    for token in sorted(names):
        p_code = producer.code_sites(token)
        c_code = consumer.code_sites(token)
        p_ship = [row for row in p_code if not row["test"]]
        c_ship = [row for row in c_code if not row["test"]]
        if p_ship and c_ship:
            state = "shared"
        elif c_ship and not p_ship:
            state = "consumer_only"
        elif p_ship and not c_ship:
            state = "producer_only"
        else:
            state = "test_only"
        rows.append(
            {
                "token": token,
                "class": cls,
                "state": state,
                "producer_sites": len(p_ship),
                "consumer_sites": len(c_ship),
                "producer_test_sites": len(p_code) - len(p_ship),
                "consumer_test_sites": len(c_code) - len(c_ship),
                "producer_comment_sites": producer.comment_tokens.get(token, 0),
                "consumer_comment_sites": consumer.comment_tokens.get(token, 0),
                "producer_site": p_ship[0]["site"] if p_ship else (p_code[0]["site"] if p_code else None),
                "consumer_site": c_ship[0]["site"] if c_ship else (c_code[0]["site"] if c_code else None),
            }
        )
    return rows


def field_pin_rows(producer: SideScan, consumer: SideScan) -> list[dict]:
    """Every consumer field pin, with what the producer side spells.

    Two levels, and the second is honestly bounded:

      field_minted  -- some producer source spells this FIELD name as a string
                       literal. A pin on a field nothing produces cannot be
                       satisfied by any capture, whatever its value.
      value_minted  -- some producer source spells the pinned VALUE. Only
                       meaningful for a string-valued pin: `false` and `1`
                       occur in every file in the tree, so for bool and integer
                       pins this is recorded as `not_applicable` rather than
                       guessed. That bound is stated, not hidden -- a numeric
                       pin's agreement is what `wave2_scope_enumerate.py`
                       checks for the carriers it enumerates.
    """
    producer_literals: dict[str, list[dict]] = {}
    for path, (code, _) in producer._lexed.items():
        spans = cfg_test_spans_for(path, code)
        for value, offset, via in producer.spelled_values(path, code):
            row = {
                "site": "%s:%d" % (path, line_of(code, offset)),
                "test": is_test_path(path) or in_spans(offset, spans),
            }
            if via:
                row["via_const"] = via
            producer_literals.setdefault(value, []).append(row)

    def ship_sites(value: str) -> list[dict]:
        return [row for row in producer_literals.get(value, []) if not row["test"]]

    rows = []
    seen: set[tuple] = set()
    for pin in consumer.field_pins:
        key = (pin["field"], pin["value"], pin["kind"], pin["site"])
        if key in seen:
            continue
        seen.add(key)
        field_hits = ship_sites(pin["field"])
        if pin["kind"] == "str":
            value_hits = ship_sites(pin["value"])
            value_state = "minted" if value_hits else "unminted"
        else:
            value_hits = []
            value_state = "not_applicable"
        if not field_hits:
            state = "field_unminted"
        elif value_state == "unminted":
            state = "value_unminted"
        elif value_state == "minted":
            state = "consistent"
        else:
            state = "field_minted_value_out_of_reach"
        rows.append(
            {
                "field": pin["field"],
                "value": pin["value"],
                "kind": pin["kind"],
                "spelling": pin["spelling"],
                "consumer_site": pin["site"],
                "consumer_is_test": pin["test"],
                "producer_field_site": field_hits[0]["site"] if field_hits else None,
                "producer_value_site": value_hits[0]["site"] if value_hits else None,
                "value_state": value_state,
                "state": state,
                "note": pin["note"],
            }
        )
    rows.sort(key=lambda row: (row["state"], row["field"], row["value"]))
    return rows


def merge_workspace_tables(
    consumer, producer
) -> tuple[
    dict[str, tuple[str, str]],
    dict[str, str],
    dict[str, list[tuple[str, str, str]]],
    set[str],
]:
    """Merge the two workspaces' constant tables into one resolution view.

    Returns (consts, aliases, defs_by_name, ambiguous). This is THE production
    merge -- `main` uses it and the self-test calls this same function, because
    review round 11 found the round-10 self-test constructing the expected
    ambiguity set by hand and passing while the merge below still missed it.

    A name defined in BOTH workspaces with different values is ambiguous even
    when each side is internally consistent: each side's `ambiguous` set only
    records disagreement it saw itself, so a name with one producer value and
    one different consumer value slipped through the union and let a consumer
    pin resolve to the FOREIGN producer value (round 10 finding 1:
    `map_viewer::PARTICLE_SHADER_SOURCE`, "gfx/FX/particle.shader" vs the
    producer's lowercase spelling). The cross-workspace comparison closes that:
    disagreement between the sides' first-definition tables joins `ambiguous`,
    and membership there fails a non-local pin closed before the merged table
    is ever consulted.
    """
    consts = {**consumer.consts, **producer.consts}
    aliases = {**consumer.aliases, **producer.aliases}
    defs: dict[str, list[tuple[str, str, str]]] = {}
    for side in (consumer, producer):
        for name, rows in side.defs_by_name.items():
            defs.setdefault(name, []).extend(rows)
    ambiguous = consumer.ambiguous | producer.ambiguous
    for name in consumer.consts.keys() & producer.consts.keys():
        if consumer.consts[name] != producer.consts[name]:
            ambiguous.add(name)
    return consts, aliases, defs, ambiguous


def main() -> int:
    parser = argparse.ArgumentParser(description="the standing authority-pin gate")
    parser.add_argument("--out", help="where to write the JSON report")
    parser.add_argument("--force", action="store_true", help="allow replacing an existing output")
    parser.add_argument("--self-test-only", action="store_true", help="run the self-test and stop")
    args = parser.parse_args()

    self_test()
    if args.self_test_only:
        return 0

    out = resolve_out(args)

    producer = SideScan("producer", PRODUCER_SCOPE)
    consumer = SideScan("consumer", CONSUMER_SCOPE)
    # The constant table is built across BOTH workspaces before either side
    # reads a pin. The extractor imports schema constants from `portable_replay`
    # crates (`PORTABLE_DESIGN_CATALOG_SCHEMA`, `PORTABLE_ARTIFACT_PATH`), so a
    # per-side table reports a side's own dependencies as having no definition
    # and files them under fail-closed -- noise that buries the real ones.
    for side in (producer, consumer):
        side.lex_all()
        side.collect_consts()
    consts, aliases, defs, ambiguous = merge_workspace_tables(consumer, producer)
    for side in (producer, consumer):
        side.consts = consts
        side.aliases = aliases
        side.defs_by_name = defs
        side.ambiguous = ambiguous
        side.collect_tokens()
    # FIELD PINS ARE SCANNED ON THE CONSUMER SIDE ONLY, which is what §5 item 6
    # of the dossier already states about the ROWS. Scanning the producer for
    # them measured nothing -- a producer checking a field of its own manifest
    # is an internal consistency check, never a cross-workspace seam pin, and
    # `field_pin_rows` builds rows from the consumer alone. Because only the
    # consumer's pins are collected, only CONSUMER parse failures can reach the
    # exit code; a producer-side pin site is never parsed and cannot fail this
    # gate. Three review rounds were spent on producer-side sites that could
    # never have become a measurement. The vocabulary classes (`name`,
    # `schema_string`) are still collected from BOTH sides: that diff is the
    # whole point.
    consumer.collect_field_pins()

    names = token_rows(producer, consumer, "name")
    schemas = token_rows(producer, consumer, "schema_string")
    pins = field_pin_rows(producer, consumer)
    unparsed = consumer.unparsed
    unresolved_constant = consumer.unresolved_constant
    # `.get("field") … == Some(…)` sites the STRICT reader did not match. Each
    # is a pin the accessor reader may not be seeing, so they are reported as a
    # standing census rather than swallowed. They are not exit-code failures:
    # the loose pattern also matches ordinary option comparisons that are not
    # authority pins at all, so the number is a watch line -- it moving is the
    # signal, not its value.
    loose_only = consumer.loose_only
    dynamic_pins = consumer.dynamic_pins

    def tally(rows: list[dict]) -> dict:
        counts: dict[str, int] = {}
        for row in rows:
            counts[row["state"]] = counts.get(row["state"], 0) + 1
        return counts

    # SHIPPING vs TEST, separately, because a total that silently folds fixtures
    # in is the number a reader will quote. Review round 1 caught "500 pins"
    # being the JSON's row count rather than a count of production pins.
    shipping = {
        "name": sum(1 for row in names if row["state"] != "test_only"),
        "schema_string": sum(1 for row in schemas if row["state"] != "test_only"),
        "field_pin": sum(1 for row in pins if not row["consumer_is_test"]),
    }
    shipping["total"] = sum(shipping.values())

    report = {
        "head": git(["rev-parse", "HEAD"]).strip(),
        "shipping_pins": shipping,
        "scopes": {"producer": PRODUCER_SCOPE, "consumer": CONSUMER_SCOPE},
        "files_scanned": {"producer": len(producer.paths), "consumer": len(consumer.paths)},
        "counts": {
            "name": tally(names),
            "schema_string": tally(schemas),
            "field_pin": tally(pins),
            "unparsed_pin_sites": len(unparsed),
            "unresolved_constant_pins": len(unresolved_constant),
            "loose_only_get_some_sites": len(loose_only),
            "dynamic_pins": len(dynamic_pins),
        },
        "name": names,
        "schema_string": schemas,
        "field_pin": pins,
        "unparsed": unparsed,
        "unresolved_constant": unresolved_constant,
        "loose_only_get_some": loose_only,
        "dynamic_pin": dynamic_pins,
    }

    payload = json.dumps(report, indent=1) + "\n"
    if args.force:
        out.write_text(payload, encoding="utf-8")
    else:
        # Exclusive creation: two runs can both see the path absent, spend a
        # second scanning, then both truncate it. Check-then-write cannot be
        # made safe by checking harder.
        try:
            with open(out, "x", encoding="utf-8") as handle:
                handle.write(payload)
        except FileExistsError:
            raise SystemExit(
                "refusing to overwrite %s: it appeared while this run was scanning.\n"
                "Pass --force if that file is yours to replace." % out
            ) from None

    print("wrote", out)
    print("files scanned: producer %d, consumer %d"
          % (len(producer.paths), len(consumer.paths)))
    for cls, rows in (("name", names), ("schema_string", schemas)):
        counts = tally(rows)
        print("%-14s %3d tokens  %s" % (cls, len(rows),
              "  ".join("%s=%d" % item for item in sorted(counts.items()))))
    counts = tally(pins)
    print("%-14s %3d pins    %s" % ("field_pin", len(pins),
          "  ".join("%s=%d" % item for item in sorted(counts.items()))))
    print("dynamic pins (expected value computed at run time): %d" % len(dynamic_pins))
    print("SHIPPING pins (test rows excluded): %d  = name %d + schema_string %d + field_pin %d"
          % (shipping["total"], shipping["name"], shipping["schema_string"],
             shipping["field_pin"]))
    if unresolved_constant:
        print("")
        print("-- pins whose CONSTANT this reader will not guess at (%d) --"
              % len(unresolved_constant))
        for row in unresolved_constant:
            print("   %s  %s" % (row["site"], row["reason"]))
    print("unresolved-constant pins: %d (named, not guessed; see the bound in the header)"
          % len(unresolved_constant))
    print("unparsed pin sites: %d   loose get/Some sites not read strictly: %d"
          % (len(unparsed), len(loose_only)))

    for cls, rows in (("name", names), ("schema_string", schemas)):
        drift = [row for row in rows if row["state"] == "consumer_only"]
        if drift:
            print("\n-- %s pins the consumer demands and no producer mints (%d) --"
                  % (cls, len(drift)))
            for row in drift:
                print("   %s  <- %s" % (row["token"], row["consumer_site"]))
    unminted = [row for row in pins if row["state"] == "field_unminted"]
    if unminted:
        print("\n-- field pins on a field no producer writes (%d) --" % len(unminted))
        for row in unminted:
            print("   %s == %r  <- %s" % (row["field"], row["value"], row["consumer_site"]))
    drifted = [row for row in pins if row["state"] == "value_unminted"]
    if drifted:
        print("\n-- field pins whose VALUE no producer mints (%d) --" % len(drifted))
        for row in drifted:
            print("   %s == %r  <- %s" % (row["field"], row["value"], row["consumer_site"]))

    if unparsed:
        print("\nFAIL-CLOSED: %d pin site(s) the classifier could not resolve" % len(unparsed))
        for row in unparsed:
            print("   %s  %s" % (row["site"], row["reason"]))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
