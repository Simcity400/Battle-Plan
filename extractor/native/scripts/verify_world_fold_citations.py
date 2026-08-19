#!/usr/bin/env python3
"""Fail closed when wire-format payload-authority citations drift.

The dNN wire modules transcribe the shipping fold rather than calling it.  Their
top-level authority marker therefore pins four things to the source it cites:

* the domain-to-fold dispatch in ``WorldFold::fold_domain_data``;
* the exact lexical function envelope;
* a SHA-256 of the cited source bytes after newline normalization; and
* a SHA-256 of the cited functions' CALLEE CLOSURE -- themselves plus every
  item in ``world_fold.rs`` they can reach, functions AND the constants,
  statics, enums, structs and type aliases they read.  ``DIVISION_FLAG_SUPPLY_
  GAIN`` is a ``const``; re-spelling it ``1 << 9`` changes which bit the
  division fold reads while every ``fn`` stays byte-identical, so a
  function-only digest covered nothing that mattered about it.

THE CLOSURE IS WHAT SURVIVES A REFACTOR.  An envelope digest covers a function
where it stands and nothing else, so lifting a helper out of a cited fold narrows
the authority to the residual: the moved logic leaves every digest, the marker is
re-stamped over the smaller span, and the gate keeps printing PASS while the
rendering it claims to pin can be changed freely.  That is not hypothetical --
d20's document build was moved into a row method and the gate noticed nothing.
The closure covers the moved code wherever it lands in this file, and
``world_fold_authority_closure.json`` records WHICH functions each domain covers
so that a set that shrinks is a diff somebody has to justify rather than a digest
that silently means less.

The closure is computed from call sites, exactly where the call names its owner
(``self.f()``, ``Type::f()``, a bare ``f()``).  A method call on any other
receiver names no type: if exactly one local function carries that name it is
taken, and if several do the gate REFUSES rather than guess, and the calling
function declares the receiver with ``// @fold-callee Type::method`` (or
``// @fold-callee-external name`` when the receiver is not defined here at all).
Refusing is the load-bearing part -- it is why there is no call from a cited fold
to a local function that the closure can quietly omit.

THE SINGLE-CANDIDATE FALLBACK OVER-COVERS ON PURPOSE.  ``country.insert(..)`` on a
`serde_json` map resolves to ``MapJournal::insert`` because that is the only
``insert`` this file defines, so that helper sits in all 45 closures and changing
it re-stamps all 45.  The alternative -- resolve only what the call site spells --
would miss the single most likely shape of the refactor this gate exists to
survive: a helper lifted into a uniquely named method on a row struct, reachable
only as ``row.helper()``.  A redundant member costs a re-stamp; a missing one
costs the coverage.

WHAT IS STILL OUTSIDE.  A helper moved to ANOTHER FILE (the membership record
names it, the re-stamper refuses it, but the closure cannot follow it); trait
dispatch through ``Display``/``Debug``/operators, which names no method at the
call site; and a callee reached through a function pointer or ``dyn`` value
assigned in a function no domain covers.  Where a hole could be turned into a
refusal it has been: a ``macro_rules!`` definition, an unrecognized item scope,
and an unreadable path qualifier all stop the gate rather than cover less.

That marker owns exactly one line range.  Every OTHER line number in these
modules is free text: it drifts on the next insertion into ``world_fold.rs`` and
nothing notices.  Those have been rewritten to name the item they point at, and
the ones that could not be are ratcheted -- see ``verify_prose_references``.

Documents can cite ``world_fold.rs`` by line number too, and until this was
added nothing checked those references at all. A drifted line number in prose
is worse than one in a module: nothing compiles it, nobody reviews it against
the source, and a reader lands on whatever item moved into that position.
Documents are therefore ratcheted exactly like the modules -- see
``verify_document_references``.

The two ports which also transcribe a capture-lane payload decoder (d51 and d52)
carry a second marker.  That marker hashes the original decoder, not the
transcription, so a changed production authority cannot be approved by tests of
the stale copy.

LEAF SOURCE AUTHORITY -- WHAT COVERS A GRAMMAR THAT LEFT THIS FILE.  A closure
follows calls inside ONE file, so the day a shared walk moves into its own crate
the closure shrinks and the digest stops covering it.  That is not hypothetical
either: lane KC moved domain 13's structural walk into ``d13_grammar`` and d13's
closure went from 80 names to 73, after which a changed section length, flag
mask or field offset inside the leaf was invisible to every marker in the tree.
Compilation catches API drift across a crate boundary; it does not catch
behaviour drift inside the leaf.

``// @leaf-source-authority crate=<name> root=<dir> files=<n> sha256=<digest>``
restores it.  The digest is over the crate's attested SOURCE -- its manifest and
every Rust file under it, each contributing its path and its own digest -- and
the marker is repeated at EVERY site that walks or writes that grammar, so
editing the leaf without re-attesting at each consumer fails here exactly as
editing a cited fold does.

A DIGEST OVER FILES IS ONLY AN ATTESTATION IF THOSE FILES ARE WHAT RUNS, so
three checks stand beside it and each answers a question the hashes cannot.
``verify_leaf_is_sealed`` refuses a leaf whose behaviour can come from anywhere
else -- a build script, a redirected ``[lib] path`` or ``#[path]`` module, an
``include!``, ``OUT_DIR``, ``env!``, a dependency (whose version a lockfile or a
patch decides) or a ``feature`` (which any crate in the graph can turn on).
``verify_leaf_consumers_bind_to_it`` requires each consumer's own package to
depend on the registered root BY PATH, because a marker says "I execute this
crate" and only Cargo decides which crate that is.  And
``verify_no_unregistered_users`` looks from the other end -- every file that
names the crate in code must be a registered consumer or a recorded const-only
user (recorded by IDENTIFIER, so it cannot grow a walk) -- because a marker scan
finds markers and can never find a consumer nobody marked.

``leaf_source_authority.json`` records which files each leaf attests.  What that
catches is precise and worth stating: a file deleted, renamed, or moved out of
the root, and a re-stamp that would otherwise accept a smaller set without
comment.  What it does NOT catch on its own is delegation -- ``lib.rs`` staying
put and calling a new crate -- and that hole is closed by the seal above, which
refuses a leaf with dependencies at all.  A repository-wide scan makes a marker
at an unregistered site a failure too, because a marker nothing is required to
keep current still reads as an attestation.

ADOPTING IT FOR A NEW LEAF (this is the whole procedure).  The crate must be
sealable: no dependencies, no features, no build script, no ``[lib] path``, no
``#[path]``/``include!``/``OUT_DIR``/``env!``.  Add one row to
``REQUIRED_LEAF_AUTHORITIES``: the crate's name, its repo-relative root, every
site that walks or writes its grammar and holds its own semantic layer over it
(the readers AND the producer), and -- only where a file reads NAMED constants
and walks nothing -- a ``const_only`` entry listing the identifiers it may name.
Paste the marker line beside the code it attests at each site with a placeholder
digest (``files=0 sha256=0`` is enough; PLACEMENT is the one thing the tooling
must not guess, because a marker that drifts away from the code it attests is a
comment nobody reads -- and mind that a marker inserted into ``world_fold.rs``
moves the ``lines=`` of every dNN marker below it, so replace a blank line
rather than adding one).  Then run ``restamp_world_fold_citations.py``, which
fills in ``files=``/``sha256=`` at every site and records the attested file set.
Nothing else is per-crate.
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import posixpath
import re
import stat
import sys
import tomllib
from collections import Counter
from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path, PurePosixPath
from typing import Iterable


WORLD_FOLD = Path("extractor/native/recorder_host/src/world_fold.rs")
WIRE_SRC = Path("extractor/native/wire_format/src")
# Both design-document trees.  `replay_interface/documents` was found citing
# `world_fold.rs` by line 28 times while only `extractor/documents` was being
# scanned, which is exactly the hole a one-directory scan leaves.
DOCUMENT_ROOTS = (Path("extractor/documents"), Path("replay_interface/documents"))
PROSE_ALLOWANCE = Path(
    "extractor/native/scripts/world_fold_prose_reference_allowance.json"
)
# The membership half of the authority.  The marker's `closure=` digest proves
# the covered SOURCE did not change; this file proves the covered SET did not
# quietly shrink -- a digest alone cannot, because the re-stamper recomputes it
# from whatever the closure happens to be after the edit.
CLOSURE_INVENTORY = Path(
    "extractor/native/scripts/world_fold_authority_closure.json"
)
PROSE_REFERENCE_RE = re.compile(r"world_fold\.rs:(\d+)")

WORLD_MARKER_RE = re.compile(
    r"^// @world-fold-authority "
    r"domain=(?P<domain>\d+) "
    r"functions=(?P<functions>fold_[A-Za-z0-9_]+(?:,fold_[A-Za-z0-9_]+)*) "
    r"lines=(?P<start>\d+)-(?P<end>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64}) "
    r"closure=(?P<closure>[0-9a-f]{64})$",
    re.MULTILINE,
)
LANE_MARKER_RE = re.compile(
    r"^// @lane-payload-authority "
    r"domain=(?P<domain>\d+) "
    r"path=(?P<path>[A-Za-z0-9_./-]+) "
    r"function=(?P<function>[A-Za-z_][A-Za-z0-9_]*) "
    r"lines=(?P<start>\d+)-(?P<end>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64})$",
    re.MULTILINE,
)
# INDENTATION IS PART OF THE MARKER.  A leaf marker sits beside the code it
# attests, and that code is often inside an `impl` block -- `fold_tech` is.
# Forcing column 0 would either put the marker somewhere else in the file or
# put a column-0 comment in the middle of an indented body; the indent is
# captured instead, and the re-stamper writes it back.
LEAF_MARKER_RE = re.compile(
    r"^(?P<indent>[ \t]*)// @leaf-source-authority "
    r"crate=(?P<crate>[A-Za-z_][A-Za-z0-9_]*) "
    r"root=(?P<root>[A-Za-z0-9_./-]+) "
    r"files=(?P<files>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64})$",
    re.MULTILINE,
)
# A line that MEANS to be a leaf marker, whatever it says.  A hand-edited or
# truncated marker must fail as a broken attestation, not silently read as an
# absent one.
#
# It is deliberately narrower than the token: prose about the mechanism (this
# repository's design documents, the Rust test that runs this gate, and the
# leading `//!` docs of the modules that carry a marker) writes
# `@leaf-source-authority` inside a sentence, and a sentence is not a failed
# marker.  What is caught is a line whose comment BEGINS with the token.
LEAF_MARKER_ATTEMPT_RE = re.compile(
    r"(?m)^[ \t]*// @leaf-source-authority.*$"
)
# WHAT AN ATTESTABLE LEAF MAY BE.  A digest over a crate's files only means
# something if those files are the whole of what the crate executes, so the
# shapes that reach outside them are REFUSED rather than hashed around.  Each
# of these lets a leaf's behaviour change with every attested byte identical:
# a build script (runs arbitrary code, reads the environment), a redirected
# `[lib] path` or `#[path]` module (the source is somewhere else), `include!`
# and friends (likewise), `OUT_DIR` (generated source), `env!`/`option_env!`
# (a build-time value), a dependency or a proc macro (behaviour from another
# crate, which a later lockfile or patch can move), and a non-test `feature`
# (a consumer, or any crate in the graph, selecting a different branch).
#
# It is a narrow definition on purpose.  A leaf is a shared GRAMMAR: the whole
# reason d13_grammar could be shared at all is that it has no dependencies
# (its manifest says so), and a leaf that needs any of the above is not one
# this gate can honestly attest.
LEAF_FORBIDDEN_SOURCE_RE = (
    # ANY attribute carrying `path =`, not only a bare `#[path]`:
    # `#[cfg_attr(windows, path = "../../outside.rs")]` redirects a module
    # exactly as well and reads nothing like the simple spelling.
    (
        re.compile(r"#\s*!?\s*\[[^\]]*\bpath\s*="),
        "an attribute that redirects a module's path",
    ),
    (re.compile(r"\binclude(?:_str|_bytes)?\s*!"), "an `include!`-family macro"),
    (re.compile(r"\b(?:option_env|env)\s*!"), "a build-time `env!` value"),
    # A leaf may not DEFINE macros. Not because a macro is bad, but because the
    # four rules above read source text: a `macro_rules!` that expands an
    # identifier into `include!` puts the forbidden construct somewhere no
    # regex over this file can see it. `OUT_DIR` needs no rule of its own --
    # the only ways to reach it are `env!` and `include!`, both refused above.
    (re.compile(r"\bmacro_rules\s*!"), "a macro definition"),
)
# `#[cfg(test)]` is the one conditional a leaf may carry: it excludes the
# crate's own proof, and `test` is set by the test harness rather than chosen by
# whoever builds the consumers.  Every other `cfg` predicate -- a feature, a
# target, a bare `--cfg` from RUSTFLAGS -- selects between branches of the
# attested source, so which branch RAN stops being a function of the digest.
# `#[path = "..."]` on a module, in a CONSUMER (a leaf may not carry one at all).
MODULE_PATH_ATTRIBUTE_RE = re.compile(
    r"#\s*\[\s*path\s*=\s*\"(?P<path>[^\"]+)\"\s*\]"
)
LEAF_PERMITTED_CFG = frozenset({"test", "not(test)"})
LEAF_CFG_RE = re.compile(r"#\s*!?\s*\[\s*cfg\s*\((?P<predicate>[^\]]*)\)\s*\]")
# `#[cfg]` is not the only conditional. `#[cfg_attr(windows, repr(C, packed))]`
# changes a type's LAYOUT -- the thing a byte grammar is -- from outside the
# attested source, and `cfg!(..)` branches at run time on the same inputs.
# Neither has a form this gate could permit narrowly without parsing Rust, and a
# leaf that needs either is not one whose behaviour is a function of its digest.
LEAF_FORBIDDEN_CONDITIONAL_RE = (
    (re.compile(r"\bcfg_attr\s*[(!]"), "a `cfg_attr` conditional attribute"),
    (re.compile(r"\bcfg\s*!\s*\("), "a `cfg!` conditional expression"),
)
FUNCTION_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|async|const|extern"
    r"(?:[ \t]+\"[^\"]+\")?)[ \t]+)*fn[ \t]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
DISPATCH_RE = re.compile(
    r"(?m)^[ \t]*(?P<domain>\d+)[ \t]*=>[ \t]*"
    r"self\.(?P<function>fold_[A-Za-z0-9_]+)[ \t]*\("
)
RAW_STRING_PREFIX_RE = re.compile(r"(?:b)?r(?P<hashes>#{0,255})\"")
CHAR_LITERAL_RE = re.compile(
    r"(?:b)?'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'\r\n])'"
)

REQUIRED_LANE_AUTHORITIES = frozenset(
    {
        (
            51,
            "extractor/native/lanes/scripted_state_flags_variables_live/"
            "scripted_state_compact.rs",
            "decode_payload",
        ),
        (
            52,
            "extractor/native/lanes/occupation_resistance_compliance_live/"
            "domain44_compact.rs",
            "decode_payload",
        ),
        # d99's wire module carries no `compact_contract` dependency (the
        # `compact_contract_dependency_policy` boundary admits only d81), so it
        # transcribes the DCS1 decoder instead of calling it. This marker pins the
        # ORIGINAL, exactly as d51's and d52's do.
        (
            99,
            "extractor/native/compact_contract/src/division_combat_stats.rs",
            "decode_payload",
        ),
    }
)

# THE LEAF REGISTRY.  ``crate name -> (crate root, the sites that must attest)``.
#
# A LEAF is a crate that exists to be the ONE implementation of a payload
# grammar several otherwise-independent trees share.  d13_grammar is the first:
# domain 13's structural walk lived in four places, two of them went stale in a
# day and one refused 59 of 59 D13 records of a live capture, and lane KC moved
# the walk into a zero-dependency crate all four could call.
#
# That move COST COVERAGE, which is why this registry exists.  d13's
# @world-fold-authority closure shrank from 80 names to 73 the moment the walk
# left `world_fold.rs`: the closure follows calls inside ONE file, so logic that
# leaves the file leaves every digest.  Compilation catches API drift across the
# crate boundary and nothing catches BEHAVIOUR drift -- a changed section
# length, flag mask or field offset inside the leaf is invisible to all 45
# markers.  A leaf marker restores the property the closure had: editing the
# shared grammar without re-attesting at each consumer fails this gate.
#
# WHICH SITES BELONG HERE.  Every site that WALKS or WRITES the leaf's byte
# grammar and holds its own semantic layer over it -- the readers and the
# producer.  A file that only re-exports a `const` from the leaf does not:
# compilation already binds it to the value, and there is no second spelling to
# drift (`collector/src/attach_log/offsets.rs` is that case).
#
# ADOPTING IT FOR ANOTHER LEAF: add the row, run
# `restamp_world_fold_citations.py` (it stamps the marker line into every listed
# site and records the attested file set), and commit.  Nothing else is
# per-crate -- the digest, the membership record, the stray-marker scan and the
# report clause are all driven from this table.
@dataclass(frozen=True)
class LeafAuthority:
    """One shared grammar crate and everything that is allowed to touch it.

    ``consumers`` carry the marker.  ``const_only`` is the narrow exception: a
    file that reads NAMED constants out of the leaf and walks nothing.  It
    records the identifiers, not just the file, so a const-only user cannot
    quietly grow into an unattested walker.
    """

    root: str
    consumers: tuple[str, ...]
    const_only: dict[str, tuple[str, ...]] = field(default_factory=dict)


REQUIRED_LEAF_AUTHORITIES = {
    "d13_grammar": LeafAuthority(
        root="extractor/native/d13_grammar",
        consumers=(
            "extractor/native/collector/src/attach_log.rs",
            "extractor/native/collector/src/attach_log/domain13_tech_delta_tests.rs",
            "extractor/native/recorder_host/src/domain_data_authority.rs",
            "extractor/native/recorder_host/src/world_fold.rs",
            "extractor/native/wire_format/src/d13.rs",
        ),
        const_only={
            # Two `const` aliases and nothing else. Compilation already binds
            # them to the leaf's values, so there is no second spelling to
            # drift -- but the identifiers are recorded, so this file cannot
            # start walking the grammar without failing the gate.
            "extractor/native/collector/src/attach_log/offsets.rs": (
                "TECH_LIVE_SCALE_RAW",
                "TECH_PROGRESS_SCALE_RAW",
            ),
        },
    ),
    "d11_grammar": LeafAuthority(
        root="extractor/native/d11_grammar",
        consumers=(
            "extractor/native/collector/src/attach_log.rs",
            "extractor/native/lanes/country_economy_production_stockpile_live/src/deployment_queue.rs",
            "extractor/native/lanes/country_economy_production_stockpile_live/src/lib.rs",
            "extractor/native/recorder_host/src/domain_data_authority.rs",
            "extractor/native/recorder_host/src/world_fold.rs",
            "extractor/native/wire_format/src/d11.rs",
        ),
        const_only={
            # Three aliases and no walk. `PRODUCTION_SCALE_RAW` is a bare
            # `const`; the other two spell `PLIC` and `PDQ1` as
            # `Trailer::…​.magic()`, which is a `const fn` read of the same
            # single-source table rather than a second copy of the four bytes.
            # `Trailer` is recorded here because the scan matches the item after
            # `::`, so this file cannot start naming the rest of the grammar --
            # or start walking it -- without failing the gate.
            "extractor/native/collector/src/attach_log/offsets.rs": (
                "PRODUCTION_SCALE_RAW",
                "Trailer",
            ),
        },
    ),
    "d74_grammar": LeafAuthority(
        root="extractor/native/d74_grammar",
        consumers=(
            # The producer half: the passive manager census builds every PBS1
            # row and hands the crate a whole `Payload` to encode.
            "extractor/native/collector/src/attach_log/d74_power_balance.rs",
            # The emission arm, which decodes the payload it just encoded to
            # price the perf record's row/unit counts.
            "extractor/native/collector/src/attach_log.rs",
            # The two readers: the fold builds `_r2_power_balance_state_live`
            # from a decoded payload, and the authority validator re-walks it.
            "extractor/native/recorder_host/src/domain_data_authority.rs",
            "extractor/native/recorder_host/src/world_fold.rs",
            # The fold's own differential test builds payloads through the
            # crate's constructors, so it holds a semantic layer over the
            # grammar exactly as `domain13_tech_delta_tests.rs` does for d13.
            "extractor/native/recorder_host/src/world_fold/tests.rs",
        ),
        # No `const_only` entry. Every site above names bounds
        # (`MAX_RECORDS`, `MAX_MEMBERS_PER_RECORD`, `MAX_OVERRIDES_PER_RECORD`,
        # `MAX_TOKEN_BYTES`) AND builds or walks the crate's row types, which is
        # not the `crate::CONST`-only shape a const_only entry may carry.
    ),
    "d43_grammar": LeafAuthority(
        root="extractor/native/d43_grammar",
        consumers=(
            "extractor/native/collector/src/attach_log.rs",
            "extractor/native/recorder_host/src/domain_data_authority.rs",
            "extractor/native/recorder_host/src/world_fold.rs",
            "extractor/native/lanes/market_trade_live/src/lib.rs",
            "extractor/native/lanes/market_trade_live/src/leaf_decode.rs",
        ),
        # No `const_only` entry. The sites above each read DOZENS of the
        # leaf's facts and read them through `fixed_row_len(Trailer::…)` and
        # `Trailer::….header_len()` as well as through bare constants, which is
        # not the `crate::CONST` shape a const_only entry may carry -- and they
        # are the sites that hold d43's semantic layer, so they are consumers.
        #
        # The lane gate (`extractor/native/lanes/market_trade_live`) is d43's
        # FOURTH site. It was deliberately absent while it did not depend on
        # this crate at all; lane LX converted it, so it is registered now, in
        # both halves: `lib.rs` holds the gate's own policy over the decoded
        # report and `leaf_decode.rs` executes the shared walk. This registry
        # states what is true rather than what is intended.
    ),
}
# The membership half of a leaf authority, as CLOSURE_INVENTORY is for the
# fold: the marker's digest proves the attested SOURCE did not change, and this
# file proves the attested SET did not quietly shrink -- a file deleted,
# renamed, or moved out of the root, and a re-stamp that would otherwise record
# fewer files without comment.
#
# It does not, by itself, catch the other shape of the same event: `lib.rs`
# staying where it is and delegating to a new crate, where no file leaves at
# all.  `verify_leaf_is_sealed` is what closes that -- a leaf may not have
# dependencies, so there is nothing for it to delegate TO.
LEAF_INVENTORY = Path("extractor/native/scripts/leaf_source_authority.json")
# Where a leaf marker, a leaf user or a leaf dependency is looked for: the WHOLE
# repository.  It was `extractor/native` alone, which is where every consumer
# happens to live today -- and a package under `replay_interface` that
# path-depends on the leaf and walks it was invisible to the scan while the gate
# reported PASS.  A completeness claim scoped to the directory the current
# consumers happen to sit in is not a completeness claim.
LEAF_MARKER_SCAN_ROOTS = (Path("."),)
# Evidence trees and build output are not source.  `runs/` is append-only by
# AGENTS.md -- a marker quoted inside a completed run's notes is a record of
# what was true, not a claim about this tree.
LEAF_SCAN_SKIP_DIRECTORIES = frozenset(
    {"runs", "target", ".git", "re_evidence", ".cargo-target", "node_modules",
     ".claude", ".pnpm-store", ".reference", ".t3-worktrees", "captures"}
)
# MANIFESTS are skipped in fewer places than sources.  An evidence tree is
# append-only prose and a marker quoted inside one is a record, not a claim --
# but a CARGO PACKAGE under `runs/` is not prose: it compiles, it can take the
# leaf as a dependency and walk it, and there are three such probe crates in
# this repository today.  A package that depends on a leaf is therefore found
# wherever it lives, and refused if it lives where this gate cannot read its
# sources.
# `.claude/` holds OTHER checkouts (lane worktrees) of this same repository.
# Their manifests and sources are claims about THEIR trees, not this one; a
# gate that reads them certifies (or refuses) work that is not here.
LEAF_MANIFEST_SKIP_DIRECTORIES = frozenset(
    {"target", ".git", ".cargo-target", "node_modules", ".claude",
     ".pnpm-store", ".reference", ".t3-worktrees", "captures"}
)
# A leaf's attested files: its manifest and every Rust source under it, wherever
# it sits (`src/`, `tests/`, `benches/`, a `build.rs`).  The set is what the
# crate compiles plus what decides how it compiles.
LEAF_ATTESTED_SUFFIXES = (".rs",)
LEAF_ATTESTED_NAMES = ("Cargo.toml",)


class VerificationError(RuntimeError):
    """The checked-in citation authority does not match production source."""


@dataclass(frozen=True)
class Span:
    start: int
    end: int


def _blank_except_newlines(value: str) -> str:
    return "".join("\n" if char == "\n" else " " for char in value)


@lru_cache(maxsize=4)
def sanitize_rust(source: str) -> str:
    """Blank comments and literals while preserving byte count and newlines.

    This is deliberately a small lexical scanner, not a Rust parser.  It knows
    nested block comments, escaped ordinary/byte strings, raw/byte-raw strings,
    and character literals.  That is sufficient to make braces in diagnostics,
    JSON snippets, and comments invisible to the function-envelope walk.

    Cached because the span walk, the item index and the closure walk each want
    the same sanitized copy of the same 1.2 MB file, and building it is a
    character-at-a-time scan.
    """

    return _blank(source, comments=True)


def _blank(source: str, *, comments: bool) -> str:
    """The scanner behind both views.  ``comments=False`` keeps comment text."""

    out: list[str] = []
    index = 0
    length = len(source)
    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            if end < 0:
                end = length
            segment = source[index:end]
            out.append(segment if not comments else _blank_except_newlines(segment))
            index = end
            continue

        if source.startswith("/*", index):
            start = index
            depth = 1
            index += 2
            while index < length and depth:
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            segment = source[start:index]
            out.append(segment if not comments else _blank_except_newlines(segment))
            continue

        raw = RAW_STRING_PREFIX_RE.match(source, index)
        if raw:
            hashes = raw.group("hashes")
            close = '"' + hashes
            start = index
            index = raw.end()
            end = source.find(close, index)
            index = length if end < 0 else end + len(close)
            out.append(_blank_except_newlines(source[start:index]))
            continue

        string_prefix = 2 if source.startswith('b"', index) else (
            1 if source[index] == '"' else 0
        )
        if string_prefix:
            start = index
            index += string_prefix
            escaped = False
            while index < length:
                char = source[index]
                index += 1
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    break
            out.append(_blank_except_newlines(source[start:index]))
            continue

        char_literal = CHAR_LITERAL_RE.match(source, index)
        if char_literal:
            out.append(_blank_except_newlines(source[index : char_literal.end()]))
            index = char_literal.end()
            continue

        out.append(source[index])
        index += 1

    return "".join(out)


def function_spans(source: str) -> dict[str, Span]:
    sanitized = sanitize_rust(source)
    spans: dict[str, Span] = {}
    for match in FUNCTION_RE.finditer(sanitized):
        name = match.group("name")
        # Only authority boundaries are indexed. world_fold.rs also contains
        # ordinary helpers/tests whose names may repeat in different scopes.
        if not (name.startswith("fold_") or name == "decode_payload"):
            continue
        open_brace = sanitized.find("{", match.end())
        if open_brace < 0:
            raise VerificationError(f"{name}: function has no opening brace")
        depth = 0
        close_brace = -1
        for index in range(open_brace, len(sanitized)):
            char = sanitized[index]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    close_brace = index
                    break
        if close_brace < 0:
            raise VerificationError(f"{name}: function has no closing brace")
        span = Span(
            start=source.count("\n", 0, match.start()) + 1,
            end=source.count("\n", 0, close_brace) + 1,
        )
        if name in spans:
            raise VerificationError(f"duplicate function name cannot be pinned: {name}")
        spans[name] = span
    return spans


@dataclass(frozen=True)
class Item:
    """One item defined in the cited file, with the scope that owns it."""

    ident: str
    owner: str
    name: str
    span: Span
    start: int
    end: int
    # "fn", or the value-item keyword (`const`/`static`/`struct`/`enum`/
    # `union`/`type`).  A value item can be NAMED by a fold but never CALLED, so
    # it is a closure member without being a candidate for a method call.
    kind: str = "fn"


class ItemIndex:
    """Every item in one file, addressable by owning scope and by bare name."""

    def __init__(
        self, items: list[Item], source: str, aliases: dict[str, str] | None = None
    ) -> None:
        # The source this index was built from, so the walk cannot be run
        # against a different one and hit the callee cache for answers that
        # describe text no longer present.
        self.source = source
        self.sanitized = sanitize_rust(source)
        self.items = {item.ident: item for item in items}
        # `type Alias = Row;` -- `Alias::helper()` executes `Row::helper`.
        self.aliases = aliases or {}
        self.by_owner: dict[tuple[str, str], list[str]] = {}
        self.by_name: dict[str, list[str]] = {}
        # Only callable items are candidates for an unresolved method call.
        self.callable_by_name: dict[str, list[str]] = {}
        # One index serves all 45 domains and their closures overlap heavily, so
        # resolving each function's callees once is the difference between a
        # gate that runs in well under a second and one that runs in five.
        self.callees: dict[str, set[str]] = {}
        for item in items:
            self.by_owner.setdefault((item.owner, item.name), []).append(item.ident)
            self.by_name.setdefault(item.name, []).append(item.ident)
            if item.kind == "fn":
                self.callable_by_name.setdefault(item.name, []).append(item.ident)

    def owned(self, owner: str, name: str) -> list[str]:
        """`(owner, name)`, following a chain of local type aliases.

        `type Alias = Mid; type Mid = Row;` needs both hops, and a cycle in
        a malformed file must not spin forever.
        """
        found = self.by_owner.get((owner, name))
        seen = {owner}
        while found is None and owner in self.aliases:
            owner = self.aliases[owner]
            if owner in seen:
                break
            seen.add(owner)
            found = self.by_owner.get((owner, name))
        return found or []


# The opening brace is deliberately NOT required on the header line: a header
# that wraps (`impl<'a> From<&'a Row>` / newline / `    for Projection<'a>` /
# newline / `{`) is the shape that made this scanner silently mis-own the `fn`
# inside it, and a mis-owned function is one whose `self.f()` callers no longer
# resolve -- silent UNDER-coverage, the exact failure this gate exists to stop.
SCOPE_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe)[ \t]+)*"
    r"(?:(?P<impl>impl)\b|(?:(?P<kind>mod|trait)[ \t]+(?:r#)?"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b))"
)
# Every line whose first token opens an item scope must be one SCOPE_RE matched.
# Return-position `impl Trait` and `f: impl Fn(..)` are mid-line and are not
# candidates; an item is not.
SCOPE_LINE_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe)[ \t]+)*(?:impl|mod|trait)\b"
)
# ...unless the line is the CONTINUATION of something else. A wrapped signature
# can put `impl Trait` at the start of a line (`) -> ` / newline / `impl Iterator
# <..> {`), and that is a return type, not an item.
CONTINUATION_CHARS = frozenset(",(+->:=&|<")
ANY_FUNCTION_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|async|const|default|extern"
    r"(?:[ \t]+\"[^\"]+\")?)[ \t]+)*fn[ \t]+"
    r"(?:r#)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
# A function is not the only thing a fold executes.  `DIVISION_FLAG_SUPPLY_GAIN`
# is a `const`, and re-spelling it `1 << 9` changes which bit the division fold
# reads while every `fn` in the closure stays byte-identical.  Constants,
# statics, enums, structs and type aliases are therefore indexed as items too,
# and a covered function that names one pulls it in through the same walk.
# A `use` binding decides WHICH implementation a bare name reaches.
# `hour_index_from_raw` is imported, not defined here, and re-pointing that
# import at another module changes what the covered helper executes while every
# indexed byte of this file stays the same.  The binding is therefore an item.
USE_ITEM_RE = re.compile(r"(?m)^[ \t]*(?:pub(?:\([^)\r\n]*\))?[ \t]+)?use[ \t]+")
USE_BINDING_RE = re.compile(
    r"(?:as[ \t\r\n]+(?:r#)?(?P<alias>[A-Za-z_][A-Za-z0-9_]*))"
    r"|(?:(?:r#)?(?P<segment>[A-Za-z_][A-Za-z0-9_]*)[ \t\r\n]*(?=[,}；;]|$))"
)
VALUE_ITEM_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|default)[ \t]+)*"
    r"(?P<kind>const|static|enum|struct|union|type)[ \t]+(?:mut[ \t]+)?"
    r"(?!fn\b)(?:r#)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
# EVERY identifier, not only the ones followed by `(`.  A function reached as a
# VALUE transfers control just as a call does -- `fold_divisions` reaches
# `DivisionFoldRow::equipment_join_key` only as
# `.and_then(DivisionFoldRow::equipment_join_key)`, and a call-shaped scan leaves
# that logic outside the closure entirely.  Whether a token is a call is decided
# from what FOLLOWS it, because that is what separates a method call from a field
# access of the same name.
IDENTIFIER_RE = re.compile(r"(?<![A-Za-z0-9_])(?:r#)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
# Only `(` -- or a turbofish before it -- makes `x.name` a CALL.  `row.items[0]`
# and `if row.ready {` are field reads, and treating them as calls would fire the
# ambiguity refusal on every struct field that shares a method's name.
METHOD_CALL_SUFFIX_RE = re.compile(r"[ \t\r\n]*(?:::[ \t\r\n]*<|\()")
# An item keyword that is NOT the first thing on its line.  `impl` is excluded:
# it appears mid-line legitimately as `-> impl Iterator` and `f: impl Fn(..)`,
# and a line-leading one is already covered by the scope guard.
SECOND_ITEM_ON_A_LINE_RE = re.compile(
    r"(?m)^[^\r\n]*[}\];][ \t]*(?:pub(?:\([^)\r\n]*\))?[ \t]+)?"
    r"(?:fn|struct|enum|union|trait|mod|impl)\b"
)
MACRO_DEFINITION_RE = re.compile(
    r"macro_rules[ \t\r\n]*![ \t\r\n]*([A-Za-z_][A-Za-z0-9_]*)"
)
CALLEE_ANNOTATION_RE = re.compile(
    r"@fold-callee[ \t]+(?P<owner>[A-Za-z_][A-Za-z0-9_]*)"
    r"::(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
# The other half of the declaration: the receivers of `.name()` in this function
# are types this file does not define (`String::as_str`), so the same-named local
# functions are NOT reachable.  Without it the only way to satisfy the gate would
# be to name a local type that is not the callee, which is a worse record than an
# explicit "not from here".
EXTERNAL_ANNOTATION_RE = re.compile(
    r"@fold-callee-external[ \t]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)


def _matching_brace(sanitized: str, from_index: int, label: str) -> tuple[int, int]:
    open_brace = sanitized.find("{", from_index)
    if open_brace < 0:
        raise VerificationError(f"{label}: no opening brace")
    depth = 0
    for index in range(open_brace, len(sanitized)):
        char = sanitized[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return open_brace, index
    raise VerificationError(f"{label}: no closing brace")


def _impl_subject(tail: str) -> str:
    """The type an `impl` header applies to, generics and trait stripped.

    ``tail`` is everything between ``impl`` and the block's ``{``, which may
    span lines, so it is whitespace-normalized before the ``for``/``where``
    splits -- those are word boundaries in a wrapped header just as much as in a
    single-line one.
    """
    tail = " " + " ".join(tail.split()) + " "
    subject = tail.split(" for ", 1)[1] if " for " in tail else tail
    subject = subject.split(" where ", 1)[0].strip()
    # `<K: Ord, V>` immediately after `impl` belongs to the header, not the type.
    if subject.startswith("<"):
        depth = 0
        for index, char in enumerate(subject):
            if char == "<":
                depth += 1
            elif char == ">":
                depth -= 1
                if depth == 0:
                    subject = subject[index + 1 :].strip()
                    break
    return subject.split("<")[0].strip().strip("&").strip()


def index_items(source: str) -> ItemIndex:
    """Index every `fn` in ``source`` under the scope that owns it.

    The owner is the innermost enclosing ``impl`` type, else the innermost
    enclosing ``mod``/``trait`` name, else the empty string for a free function.
    That is deliberately NOT a Rust name resolver: it is the coarsest key under
    which ``self.f()``, ``Type::f()``, and a bare ``f()`` can all be looked up,
    and every call it cannot key exactly is handled by the fallback in
    ``resolve_callees`` rather than by guessing.
    """

    sanitized = sanitize_rust(source)

    # A macro can expand to a call this scanner cannot see, so a file that
    # defines one is a file whose closure would silently under-cover.  There are
    # none today; the day someone adds one, the gate says so instead of quietly
    # covering less.
    macros = sorted(
        {match.group(1) for match in MACRO_DEFINITION_RE.finditer(sanitized)}
    )
    if macros:
        raise VerificationError(
            f"this file now defines macro(s) {macros}; a macro body can call a "
            f"function without naming it at the call site, so the authority "
            f"closure can no longer be trusted to cover what a cited fold runs"
        )

    def continues_a_previous_line(position: int) -> bool:
        cursor = position
        while cursor > 0 and sanitized[cursor - 1] in " \t\r\n":
            cursor -= 1
        return cursor > 0 and sanitized[cursor - 1] in CONTINUATION_CHARS

    scopes: list[tuple[int, int, str]] = []
    recognized: set[int] = set()
    for match in SCOPE_RE.finditer(sanitized):
        if match.group("impl") and continues_a_previous_line(match.start()):
            # `) -> ` / newline / `impl Iterator<..> {` is a return type wrapped
            # onto its own line, not an item.
            recognized.add(match.start())
            continue
        brace = sanitized.find("{", match.end())
        semicolon = sanitized.find(";", match.end())
        if brace < 0 or (0 <= semicolon < brace):
            # `mod tests;` -- a declaration, not a scope, and its contents are in
            # another file which this gate does not claim to cover.
            recognized.add(match.start())
            continue
        open_brace, close_brace = _matching_brace(
            sanitized, match.end(), "scope header"
        )
        if match.group("impl"):
            owner = _impl_subject(sanitized[match.end() : open_brace])
        else:
            owner = match.group("name")
        recognized.add(match.start())
        if owner:
            scopes.append((match.start(), close_brace, owner))

    # A scope form this scanner does not recognize is not a harmless gap: the
    # functions inside it get attributed to whatever encloses it, their callers'
    # `self.f()` stops resolving, and the closure quietly covers less while the
    # gate still passes.  Refuse instead.
    unrecognized = [
        source.count("\n", 0, match.start()) + 1
        for match in SCOPE_LINE_RE.finditer(sanitized)
        if match.start() not in recognized
    ]
    if unrecognized:
        raise VerificationError(
            f"unrecognized item-scope header(s) at line(s) {unrecognized}; the "
            f"functions they own would be attributed to the wrong scope"
        )

    # Every item pattern is anchored at a line start, so a SECOND item on the
    # same line (`fn first() {} fn helper() {}`, or `#[cfg(x)] impl Row {`) is
    # neither indexed nor refused, and a cited fold could call the one that was
    # missed.  Supporting the layout would mean unanchoring every pattern; the
    # file does not use it, so this refuses it instead of covering less.
    crowded_items = sorted(
        source.count("\n", 0, match.start()) + 1
        for match in SECOND_ITEM_ON_A_LINE_RE.finditer(sanitized)
    )
    if crowded_items:
        raise VerificationError(
            f"line(s) {crowded_items} declare more than one item; this scanner "
            f"indexes one item per line, so the later ones would be invisible to "
            f"the authority closure. Put each item on its own line."
        )

    def owning_scope(position: int) -> str:
        innermost = ""
        best = -1
        for start, end, owner in scopes:
            if start <= position <= end and start > best:
                best, innermost = start, owner
        return innermost

    # `source.count("\n", 0, position)` per item is a scan of the whole file per
    # lookup, and there are two lookups for each of 460 functions.  One sorted
    # list of newline offsets answers all of them.
    newlines = [index for index, char in enumerate(source) if char == "\n"]

    def line_of(position: int) -> int:
        return bisect.bisect_left(newlines, position) + 1

    # SANITIZED lines: a comment that happens to end in `]` reads as the tail of
    # a multi-line attribute and walks the span upward past it.  Blanked, a
    # comment terminates that walk exactly as an empty line does.
    lines = _source_lines(sanitized)
    items: list[Item] = []
    seen: Counter[str] = Counter()

    def add(name: str, start: int, end_index: int, kind: str = "fn") -> None:
        owner = owning_scope(start)
        ident = f"{owner}::{name}" if owner else name
        seen[ident] += 1
        if seen[ident] > 1:
            # Two items under one key: different traits implemented for one type
            # (`Display::fmt` and `Debug::fmt`), or one name in two `impl`
            # blocks.  Suffixing keeps both in the closure; dropping either
            # would silently uncover it.
            ident = f"{ident}#{seen[ident]}"
        items.append(
            Item(
                ident=ident,
                owner=owner,
                name=name,
                span=Span(
                    start=_attribute_start(lines, line_of(start)),
                    end=line_of(end_index),
                ),
                start=start,
                end=end_index + 1,
                kind=kind,
            )
        )

    for match in ANY_FUNCTION_RE.finditer(sanitized):
        kind, position = _item_terminator(sanitized, match.end())
        if kind != "brace":
            # `fn f(&self);` -- a trait method DECLARATION with no body.  Walking
            # to the next brace would hand it some later function's body.
            #
            # The scan is DEPTH-AWARE because a signature can carry a semicolon
            # of its own: `fn helper(values: [i32; 5]) { .. }` is a definition,
            # and reading that `;` as a terminator dropped the whole function
            # from the index -- so a cited fold could call it and the closure
            # would contain only the fold.
            continue
        _, close_brace = _matching_brace(sanitized, position, match.group("name"))
        add(match.group("name"), match.start(), close_brace)

    # A `const` declared INSIDE a function body is already inside that
    # function's span, so indexing it again would hash it twice -- and, because
    # such names repeat freely across bodies (`HEADER_LEN` thirteen times), it
    # would key them `#2`..`#13` by source ORDER, so inserting one earlier in the
    # file renames the rest and churns membership that did not change.
    enclosing_extents = [(item.start, item.end) for item in items]

    def inside_an_item(position: int) -> bool:
        return any(start <= position < end for start, end in enclosing_extents)

    aliases: dict[str, str] = {}
    for match in VALUE_ITEM_RE.finditer(sanitized):
        # Nested inside a function OR inside another value item: either way
        # its parent's span already hashes it, and indexing it again gives a
        # second same-named item that makes an unambiguous `crate::FLAG`
        # refuse for an ambiguity Rust does not have.
        if inside_an_item(match.start()):
            continue
        item_kind = match.group("kind")
        if item_kind in ("struct", "enum", "union"):
            # A braced body, or `struct Foo;` / `struct Foo(u32);`.
            kind, position = _item_terminator(sanitized, match.end())
            if kind == "brace":
                # From the terminator's OWN brace, not the first `{` after the
                # name: a braced const-generic default (`struct Row<const N:
                # usize = { 1 }>`) puts one in the header, and matching from
                # there ended the item before its fields.
                _, end_index = _matching_brace(
                    sanitized, position, match.group("name")
                )
            elif kind == "semicolon":
                end_index = position
            else:
                continue
        else:
            # `const`/`static`/`type` end at their OWN semicolon, which is the
            # first one at depth zero.  Taking the first `{` instead truncated
            # `const PICK: i32 = if c { A } else { B };` after the first branch,
            # and taking the first `;` truncated `static X: [u8; 4] = ..` inside
            # the array type.
            kind, position = _item_terminator(
                sanitized, match.end(), stop_at_brace=False
            )
            if kind != "semicolon":
                continue
            end_index = position
            if item_kind == "type":
                target = _alias_target(sanitized[match.end() : end_index])
                if target:
                    aliases[match.group("name")] = target
        add(match.group("name"), match.start(), end_index, item_kind)
        enclosing_extents.append((match.start(), end_index + 1))

    # LAST, so a block-local import inside a constant is already covered
    # by that constant's extent and is not globalized into `by_name`,
    # where it would make an unambiguous `crate::FLAG` refuse.
    for match in USE_ITEM_RE.finditer(sanitized):
        if inside_an_item(match.start()):
            continue
        kind, position = _item_terminator(
            sanitized, match.end(), stop_at_brace=False
        )
        if kind != "semicolon":
            continue
        declaration = sanitized[match.end() : position]
        # `use a::b::{c, d as e};` binds `c` and `e`.  Each binding is indexed
        # over the WHOLE statement, so re-pointing any of them changes the
        # digest of every closure that names one.
        for bound in USE_BINDING_RE.finditer(declaration):
            name = bound.group("alias") or bound.group("segment")
            if name and name not in ("self", "crate", "super"):
                add(name, match.start(), position, "use")

    return ItemIndex(items, source, aliases)


def _item_terminator(
    sanitized: str, start: int, *, stop_at_brace: bool = True
) -> tuple[str, int]:
    """Find where an item's header ends: its body brace, or its semicolon.

    Depth-aware over `()`, `[]` and `{}`, because a semicolon inside an array
    type (`[i32; 5]`) or a brace inside an initializer (`if c { A } else { B }`)
    belongs to the item, not after it.  Returns ``("brace"|"semicolon"|"", index)``.
    """
    depth = 0
    angle = 0
    for index in range(start, len(sanitized)):
        char = sanitized[index]
        if char in "([":
            depth += 1
        elif char in ")]":
            depth -= 1
        # An adjacent `<<` is a SHIFT, not two generic opens: `{ 1 << 2 }`
        # as a const-generic default otherwise left the angle depth stuck
        # open and the real body brace looked nested.  `>>` still
        # decrements twice, because that is how `Vec<Vec<u32>>` closes --
        # guarding both sides broke every nested generic in the file.
        elif char == "<" and not (
            sanitized[index : index + 2] == "<<"
            or (index > start and sanitized[index - 1] == "<")
        ):
            # Generic parameter lists can carry a braced default
            # (`struct Row<const N: usize = { 1 }>`), whose `{` is NOT the item
            # body.  `->` and `=>` are excluded below so a return type does not
            # unbalance the count.
            angle += 1
        elif char == ">" and index > start and sanitized[index - 1] not in "-=":
            angle = max(0, angle - 1)
        elif char == "{":
            if depth == 0 and angle == 0 and stop_at_brace:
                return "brace", index
            depth += 1
        elif char == "}":
            depth -= 1
        elif char == ";" and depth == 0:
            # A `;` cannot appear inside a real generic list, so it terminates
            # regardless of `angle` -- which also unwedges a comparison
            # (`const X: bool = 1 < 2;`) that inflated the count.
            return "semicolon", index
    return "", -1


def _alias_target(declaration: str) -> str:
    """The base type a `type A = B<..>;` alias points at, or "".

    `Alias::helper()` executes `Row::helper`, so an index that only knows
    `(Alias, helper)` silently resolves nothing and the method leaves the
    closure while the alias and the underlying type both stay in it.
    """
    if "=" not in declaration:
        return ""
    target = declaration.split("=", 1)[1].strip()
    target = target.split("<")[0].strip().lstrip("&").strip()
    # `type Alias = m::Row;` aliases `Row`, not the module `m`.
    target = target.split("::")[-1].strip()
    match = re.match(r"(?:r#)?([A-Za-z_][A-Za-z0-9_]*)", target)
    return match.group(1) if match else ""


def _attribute_above(lines: tuple[str, ...], line: int) -> bool:
    """Is the next non-blank line above ``line`` a single-line attribute?"""
    cursor = line - 1
    while cursor >= 1:
        text = lines[cursor - 1].strip()
        if text:
            return text.startswith("#") and text.endswith("]")
        cursor -= 1
    return False


def _attribute_start(lines: tuple[str, ...], fn_line: int) -> int:
    """The first line of the attribute block above ``fn_line``, or ``fn_line``.

    An attribute is part of the function it decorates and can decide whether it
    is the one compiled at all: ``world_fold.rs`` carries two
    ``injected_record_fault`` bodies selected by ``#[cfg(not(test))]`` /
    ``#[cfg(test)]``.  A span that starts at ``fn`` leaves those selectors
    outside every digest, so swapping which body ships changes no membership and
    no digest.  Doc comments are deliberately NOT included: they change no
    behaviour, and hashing them would fail the gate on a typo fix.
    """
    start = fn_line
    cursor = fn_line - 1
    pending = 0
    while cursor >= 1:
        text = lines[cursor - 1].strip()
        if not text:
            # Rust allows `#[cfg(..)]`, a blank line or comment, then the
            # item.  Keep climbing: a blank line does not end the attribute
            # block, only a line that is neither blank nor an attribute does.
            # (Comments are blank in the sanitized view this walks.)
            if pending == 0 and not _attribute_above(lines, cursor):
                break
            cursor -= 1
            continue
        if pending == 0:
            if not text.endswith("]"):
                break
            if text.startswith("#"):
                start = cursor
                cursor -= 1
                continue
            # The tail of an attribute spread over several lines.
            pending = text.count("]") - text.count("[")
            cursor -= 1
            continue
        pending += text.count("]") - text.count("[")
        if pending <= 0:
            if not text.startswith("#"):
                # Balanced, but never an attribute -- do not swallow code.
                break
            start = cursor
            pending = 0
        cursor -= 1
    return start


def _token_before(body: str, index: int) -> tuple[str, int]:
    """The identifier ending at or before ``index``, and where it starts."""
    end = index
    while end > 0 and body[end - 1] in " \t\r\n":
        end -= 1
    start = end
    while start > 0 and (body[start - 1].isalnum() or body[start - 1] == "_"):
        start -= 1
    return body[start:end], start


def _turbofish_owner(body: str, index: int) -> str:
    """The type in front of a `Type::<..>::` turbofish ending at ``index``.

    Returns "" when the angle brackets do not balance or nothing precedes them
    (``<[u8; 4]>::try_from``), which the caller treats as an owner it could not
    read rather than as no owner at all.
    """
    cursor = index
    while cursor > 0 and body[cursor - 1] in " \t\r\n":
        cursor -= 1
    if cursor == 0 or body[cursor - 1] != ">":
        return ""
    depth = 0
    while cursor > 0:
        char = body[cursor - 1]
        if char == ">":
            depth += 1
        elif char == "<":
            depth -= 1
            if depth == 0:
                cursor -= 1
                break
        cursor -= 1
    else:
        return ""
    if cursor >= 2 and body[cursor - 2 : cursor] == "::":
        cursor -= 2
    owner, _ = _token_before(body, cursor)
    return owner


def _call_sites(body: str) -> Iterable[tuple[str, str | None, str, int]]:
    """Classify each identifier in a sanitized function body.

    Yields ``(kind, qualifier, name)`` where kind is ``path`` (``A::b``),
    ``self`` (``self.b(``), ``method`` (any other ``expr.b(``), or ``free``.

    A PATH is yielded whether or not it is followed by ``(``, because
    ``.and_then(Type::method)`` hands control to that method exactly as a call
    does, and a call-shaped scan leaves it outside the closure.  A ``.name``
    that is NOT followed by ``(`` is yielded for nothing: Rust has no way to
    take a method by value through a receiver, so those are field accesses, and
    treating them as calls would drag the ambiguity refusal onto every struct
    field that shares a method's name.

    The classification reads backwards a character at a time rather than
    matching a regex against the whole preceding body: it is exact regardless of
    how the call is wrapped, and it does not make the walk quadratic in the
    length of a 600-line fold function.
    """

    for match in IDENTIFIER_RE.finditer(body):
        name = match.group("name")
        cursor = match.start()
        while cursor > 0 and body[cursor - 1] in " \t\r\n":
            cursor -= 1
        if cursor >= 2 and body[cursor - 2 : cursor] == "::":
            segment, _ = _token_before(body, cursor - 2)
            if not segment:
                segment = _turbofish_owner(body, cursor - 2)
            if segment in ("crate", "super", "self"):
                # A module path this scanner does not model.  Treated as an
                # unread owner so it reaches the refusal rather than resolving
                # to a scope named "crate" that does not exist.
                segment = ""
            if segment:
                yield "path", segment, name, match.start()
                continue
            # A qualifier this cannot read is the same unknown as a receiver it
            # cannot read, and must reach the same refusal rather than resolve to
            # nothing quietly.  Yielded whether or not it is called, because
            # `<Row as Key>::join_key` hands over control as a value too.
            #
            # Its own kind, not "method": `crate::FLAG` can name a CONSTANT,
            # and the method rules consult callable items only, so a value
            # behind an unreadable qualifier fell through and vanished.
            yield "qualified", None, name, match.start()
            continue
        if cursor >= 1 and body[cursor - 1] == ".":
            if METHOD_CALL_SUFFIX_RE.match(body, match.end()):
                receiver, _ = _token_before(body, cursor - 1)
                # `foo().bar(` and `x[0].bar(` leave no receiver token, so the
                # receiver type is not something the call site spells.
                yield (
                    ("self" if receiver == "self" else "method"),
                    None,
                    name,
                    match.start(),
                )
            continue
        yield "free", None, name, match.start()


def _line_offsets(text: str):
    """A 0-based line number for any offset inside ``text``."""
    starts = [0]
    for index, char in enumerate(text):
        if char == "\n":
            starts.append(index + 1)

    def line_of(position: int) -> int:
        return bisect.bisect_right(starts, position) - 1

    return line_of


def resolve_callees(index: ItemIndex, item: Item) -> set[str]:
    """The locally defined items ``item`` can transfer control to.

    Resolution is exact where the call spells its owner (``self.f()``,
    ``Type::f()``, ``module::f()``, a bare ``f()``).  A method call on any other
    receiver names no type, so:

    * exactly one local ``fn`` carries that name -- take it.  Over-inclusion
      (the receiver was really a `std` type with a same-named method) costs one
      redundant closure member; exclusion would cost coverage.
    * several do -- REFUSE.  Picking one would under-cover, taking all would
      chain every domain's digest to every other renderer named ``to_json``.
      The calling function declares the receiver with a
      ``// @fold-callee Type::method`` comment instead.

    Refusing rather than guessing is what makes the closure sound: there is no
    path by which a call reaches a local function that the closure omits without
    the gate saying so.
    """

    cached = index.callees.get(item.ident)
    if cached is not None:
        return cached
    body = index.sanitized[item.start : item.end]
    # Declarations are read from the view where comments survive but string
    # literals do not, so `"// @fold-callee Type::method"` inside a diagnostic
    # cannot act as authority.
    raw_body = blank_literals_only(index.source)[item.start : item.end]
    # (name, declaration line) -> the idents it declares.  Keyed by LINE as
    # well as name so a declaration governs the call it was written for and
    # not every later call that happens to share the method name.
    declared: dict[tuple[str, int], set[str]] = {}
    body_line = _line_offsets(raw_body)
    for annotation in CALLEE_ANNOTATION_RE.finditer(raw_body):
        owner, name = annotation.group("owner"), annotation.group("name")
        idents = [
            ident
            for ident in index.owned(owner, name)
            if index.items[ident].kind == "fn"
        ]
        if not idents:
            # A declaration naming a CONSTANT would be accepted and would
            # then suppress the real method's ambiguity, omitting the body it
            # claims to describe.
            raise VerificationError(
                f"{item.ident}: `@fold-callee {owner}::{name}` names no function "
                f"defined in this file"
            )
        declared.setdefault((name, body_line(annotation.start())), set()).update(
            idents
        )
    for annotation in EXTERNAL_ANNOTATION_RE.finditer(raw_body):
        name = annotation.group("name")
        if name not in index.by_name:
            raise VerificationError(
                f"{item.ident}: `@fold-callee-external {name}` declares away a "
                f"name this file never defines, so it declares nothing; remove it"
            )
        declared.setdefault((name, body_line(annotation.start())), set())

    resolved: set[str] = set()
    governed_calls: dict[tuple[str, int], int] = {}
    unresolved: dict[str, tuple[list[str], list[int]]] = {}
    used: set[tuple[str, int]] = set()
    for kind, qualifier, name, position in _call_sites(body):
        if kind == "path":
            owner = item.owner if qualifier == "Self" else qualifier
            resolved.update(index.owned(owner or "", name))
            continue
        if kind == "self":
            resolved.update(index.owned(item.owner, name))
            continue
        if kind == "free":
            local = index.owned(item.owner, name) or index.owned("", name)
            resolved.update(local)
            continue
        # kind == "method" or "qualified"
        # A declaration governs its own line and the line after it, which is
        # how it is written: the comment sits on the call, or immediately
        # above it.
        call_line = body_line(position)
        governing = [
            key
            for key in ((name, call_line), (name, call_line - 1))
            if key in declared
        ]
        if governing:
            for key in governing:
                resolved.update(declared[key])
                used.add(key)
            governed_calls[(name, call_line)] = (
                governed_calls.get((name, call_line), 0) + 1
            )
            continue
        # A method call can only reach a callable item; an unreadable PATH
        # (`crate::FLAG`, `<Row as Key>::JOIN`) can equally name a constant, a
        # static or a type, so it consults every item of that name.
        candidates = (
            index.by_name.get(name)
            if kind == "qualified"
            else index.callable_by_name.get(name)
        )
        if not candidates:
            continue
        if len(candidates) == 1:
            resolved.update(candidates)
            continue
        unresolved.setdefault(name, (candidates, []))[1].append(
            item.span.start + call_line
        )

    # A declaration that resolves nothing is a claim about a call that is no
    # longer here: the call was renamed, moved, or deleted, and what is left is a
    # standing permission to omit a callee nobody has looked at since.
    # A declaration is bound to a LINE, so two same-named calls on one line
    # share it and the second receiver is silently assumed to be the first.
    crowded = sorted(
        f"{name} (line {item.span.start + line})"
        for (name, line), count in governed_calls.items()
        if count > 1
    )
    if crowded:
        raise VerificationError(
            f"{item.ident}: a declaration governs a LINE, and these carry more "
            f"than one call of the same name so the receivers cannot be told "
            f"apart: {crowded}. Put each call on its own line with its own "
            f"declaration."
        )

    stale = sorted(name for name, _ in set(declared) - used)
    if stale:
        raise VerificationError(
            f"{item.ident}: `@fold-callee` declaration(s) for {stale} govern no "
            f"call on their own line or the next one; move each declaration "
            f"onto the call it describes, or delete it"
        )

    if unresolved:
        detail = "; ".join(
            f"`{name}` at world_fold.rs line(s) {lines} could be "
            f"{sorted(candidates)}"
            for name, (candidates, lines) in sorted(unresolved.items())
        )
        raise VerificationError(
            f"{item.ident}: a reference reaches a locally defined item whose "
            f"owner is not written at the site, and more than one local item "
            f"carries that name, so the authority closure cannot be computed: "
            f"{detail}. Declare it on the line above with a "
            f"`// @fold-callee Type::name` comment."
        )
    resolved.discard(item.ident)
    index.callees[item.ident] = resolved
    return resolved


def callee_closure(index: ItemIndex, roots: Iterable[str]) -> set[str]:
    """Every local item reachable from ``roots``, roots included."""

    members: set[str] = set()
    pending = list(roots)
    while pending:
        ident = pending.pop()
        if ident in members:
            continue
        members.add(ident)
        item = index.items.get(ident)
        if item is None:
            raise VerificationError(f"unknown authority function {ident}")
        if item.kind in ("const", "static"):
            # A constant built from other constants carries them:
            # `KNOWN_FLAGS = A | B | C` means nothing without A, B and C, and
            # changing one of them changes what `KNOWN_FLAGS` admits while its
            # own bytes stay identical.  Only VALUES are followed, never the
            # field types a struct names -- see below.
            pending.extend(
                ident
                for ident in resolve_callees(index, item)
                if index.items[ident].kind in ("const", "static", "fn")
            )
            continue
        if item.kind != "fn":
            # A TYPE IS A LEAF.  Its own text is hashed, which is what catches a
            # widened field or a changed enum discriminant, but the walk is not
            # followed through the types its fields name: doing that made
            # `WorldFold` drag all 138 state structs into all 45 closures, so
            # every domain's digest moved whenever any other domain's state
            # changed and the membership list stopped describing the domain.
            continue
        pending.extend(resolve_callees(index, item))
        if item.owner:
            # The TYPE a method belongs to is part of what that method operates
            # on: every cited fold reads `WorldFold`'s fields directly, so
            # widening one changes what the fold does while the method's own
            # bytes stay identical.  The owner is a lookup key everywhere else;
            # here it is a dependency.
            pending.extend(index.owned("", item.owner))
    return members


def closure_digest(index: ItemIndex, members: Iterable[str]) -> str:
    """A position-independent digest of the closure's source.

    Keyed by qualified name and ordered by it, so inserting an unrelated
    function ABOVE a covered one does not change the digest -- only the covered
    text, or the set of covered names, does.  That is what separates "the file
    moved" (re-stamp mechanically) from "the covered logic changed" (review it).
    """

    digest = hashlib.sha256()
    for ident in sorted(members):
        item = index.items[ident]
        digest.update(ident.encode("utf-8"))
        digest.update(b"\n")
        digest.update(
            _normalized_span_text(index.source, item.span).encode("utf-8")
        )
    return digest.hexdigest()


@lru_cache(maxsize=4)
def blank_literals_only(source: str) -> str:
    """Like ``sanitize_rust`` but KEEPING comments, byte count preserved.

    Receiver declarations live in comments, so they cannot be read from the
    sanitized copy; read from the RAW source, a string literal containing
    ``// @fold-callee Type::method`` would act as authority.  This is the one
    view where a declaration is exactly a declaration.
    """
    return _blank(source, comments=False)


@lru_cache(maxsize=4)
def _source_lines(source: str) -> tuple[str, ...]:
    """`splitlines()` of a 1.2 MB file, once instead of once per span.

    A closure digest reads one span per member, and 45 domains share members, so
    an uncached split here dominated the whole gate's runtime.
    """
    return tuple(source.splitlines())


def _normalized_span_text(source: str, span: Span) -> str:
    lines = _source_lines(source)
    if span.start < 1 or span.end < span.start or span.end > len(lines):
        raise VerificationError(
            f"invalid source span {span.start}-{span.end} for {len(lines)} lines"
        )
    return "\n".join(lines[span.start - 1 : span.end]) + "\n"


def normalized_span_hash(source: str, span: Span) -> str:
    return hashlib.sha256(
        _normalized_span_text(source, span).encode("utf-8")
    ).hexdigest()


def domain_dispatch(source: str, spans: dict[str, Span]) -> dict[int, str]:
    dispatch_span = spans.get("fold_domain_data")
    if dispatch_span is None:
        raise VerificationError("world_fold.rs has no fold_domain_data")
    lines = source.splitlines()
    dispatch_source = "\n".join(
        lines[dispatch_span.start - 1 : dispatch_span.end]
    )
    dispatch: dict[int, str] = {}
    for match in DISPATCH_RE.finditer(dispatch_source):
        domain = int(match.group("domain"))
        function = match.group("function")
        prior = dispatch.setdefault(domain, function)
        if prior != function:
            raise VerificationError(
                f"domain {domain} dispatches to both {prior} and {function}"
            )
    return dispatch


def _read_utf8(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise VerificationError(f"cannot read {path}: {error}") from error


def _only_match(pattern: re.Pattern[str], source: str, label: str) -> re.Match[str]:
    matches = list(pattern.finditer(source))
    if len(matches) != 1:
        raise VerificationError(f"{label}: expected one marker, found {len(matches)}")
    return matches[0]


def gated_citation(marker: re.Match[str]) -> str:
    """The one human citation the ``lines=``/sha256 marker backs."""
    return f"`world_fold.rs:{marker.group('start')}`-`:{marker.group('end')}`"


def ungated_prose_references(source: str, marker: re.Match[str]) -> list[int]:
    """Every `world_fold.rs:N` in a module that the digest gate does not own.

    The gated citation -- the ``lines=`` marker's paired human form -- is the one
    place a line number belongs, because the sha256 beside it fails the moment
    those bytes move.  Every OTHER line number is free text that nothing checks,
    drifts silently on the next insertion, and should name the item instead.

    ONE occurrence is owned, not every occurrence.  Treating them all as owned
    would let a second, unpaired copy of the gated text smuggle two unchecked
    line numbers past the ratchet, and the digest backs a citation, not a string
    that happens to look like one.  ``verify_repo`` separately requires the
    citation to appear exactly once, so the first match is that one occurrence.
    """
    gated = gated_citation(marker)
    found = re.search(re.escape(gated), source)
    owned = (found.start(), found.end()) if found else (-1, -1)
    return sorted(
        int(reference.group(1))
        for reference in PROSE_REFERENCE_RE.finditer(source)
        if not owned[0] <= reference.start() < owned[1]
    )


def load_prose_allowance(repo_root: Path) -> dict:
    """The grandfathered free-text line numbers, or nothing.

    A missing file grandfathers NOTHING, which fails closed: a tree that lost
    the file fails loudly instead of quietly permitting everything.  A corrupt
    one is an error rather than an empty allowance, for the same reason.
    """
    allowance_path = repo_root / PROSE_ALLOWANCE
    if not allowance_path.is_file():
        return {}
    try:
        loaded = json.loads(allowance_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VerificationError(
            f"cannot read the prose-reference allowance {allowance_path}: {error}"
        ) from error
    if not isinstance(loaded, dict):
        # Valid JSON of the wrong shape reached `.get` and raised an
        # AttributeError outside the VerificationError handler.
        raise VerificationError(
            f"{allowance_path}: expected a JSON object at the root"
        )
    return loaded


def verify_document_references(repo_root: Path, allowance: dict) -> int:
    """Fail when a design document gains a ``world_fold.rs`` line number.

    Same ratchet as ``verify_prose_references`` and for a sharper reason.  A
    module's stale line number sits next to a digest-gated marker that a reader
    can fall back on; a document's does not.  Nothing compiles prose, no gate
    read these before this one, and the citation's only reader is a human who
    opens ``world_fold.rs`` at that line and believes whatever is there.

    The budget is per (document, line number), exactly as for modules, with the
    same accepted hole: deleting one occurrence and citing the SAME number again
    elsewhere in the SAME document passes.  Binding to a position would mean
    recording a line number in a file whose own line numbers move.

    Absence of a document root is not failure -- the module fixtures under test
    have no document tree at all -- but absence of the ALLOWANCE still
    grandfathers nothing.

    KNOWN LIMIT: ``*.md`` only, and only under ``DOCUMENT_ROOTS``.  No other file
    under those trees cites ``world_fold.rs`` by line today (the dossier JSON
    does not), and the code that does -- ``collector/src/attach_log.rs``,
    ``recorder_host/src/world_fold/tests.rs``, the ``wire_format`` bench
    binaries -- sits outside the dNN module set the marker gate scans and is not
    claimed here.
    """
    permitted: dict[str, list[int]] = allowance.get("documents", {})
    documents: list[Path] = []
    for root in DOCUMENT_ROOTS:
        document_root = repo_root / root
        if document_root.is_dir():
            documents.extend(document_root.rglob("*.md"))
    scanned = {document.relative_to(repo_root).as_posix() for document in documents}

    # A DOCUMENT THE ALLOWANCE NAMES, THAT EXISTS, AND THAT THIS SCAN DID NOT
    # REACH, DISABLES THE CHECK FOR IT SILENTLY.  Renaming a configured root,
    # moving a document out of one, or changing its extension all leave the
    # citations on disk and the gate reporting a smaller count -- and a count
    # nobody compares against anything is not a check.  Absence is still fine:
    # deleting a document must not force an allowance edit in the same commit.
    # (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 8.)
    unreachable = sorted(
        relative
        for relative in permitted
        if relative not in scanned and (repo_root / relative).is_file()
    )
    if unreachable:
        raise VerificationError(
            "these documents are grandfathered but were not scanned, so their "
            f"`world_fold.rs:<line>` references are unchecked: {unreachable}. "
            f"Either bring them under one of {[str(r) for r in DOCUMENT_ROOTS]} "
            "as `*.md`, or add their location to DOCUMENT_ROOTS."
        )

    total = 0
    for document in sorted(documents):
        relative = document.relative_to(repo_root).as_posix()
        found = sorted(
            int(reference.group(1))
            for reference in PROSE_REFERENCE_RE.finditer(_read_utf8(document))
        )
        total += len(found)
        budget = Counter(permitted.get(relative, []))
        for line in found:
            if budget[line] <= 0:
                raise VerificationError(
                    f"{relative}: `world_fold.rs:{line}` is a free-text line "
                    f"number nothing checks. Line numbers drift on the next "
                    f"insertion into world_fold.rs, and a document's reader has "
                    f"no digest beside it to notice; cite the item instead, as "
                    f"`world_fold.rs::<item_name>`. (If this reference genuinely "
                    f"cannot be named, add it to {PROSE_ALLOWANCE.name} under "
                    f'"documents" with the reason.)'
                )
            budget[line] -= 1
    return total


def verify_prose_references(
    repo_root: Path, modules: Iterable[Path], markers: dict[str, re.Match[str]]
) -> int:
    """Fail when a module gains a world_fold.rs line number outside the gate.

    This is a ratchet, not a clean-tree assertion.  The references that survive
    today are the ones no tool could resolve to an item with total confidence --
    ranges spanning several items, struct fields, and citations that were already
    off by one when they were written.  They are listed per module in
    ``world_fold_prose_reference_allowance.json``, and the list may only shrink:
    citing a NEW line number, or swapping one for another, fails here.

    Known and accepted limit: the budget is per (module, line number), not per
    site.  Deleting one grandfathered occurrence and citing the SAME number again
    elsewhere in the SAME module passes.  Binding to a site would mean recording
    a position in a file whose own line numbers move -- the very failure this
    check exists to stop -- and the hole admits no line number that module was
    not already allowed to cite.
    """
    allowance = load_prose_allowance(repo_root)
    permitted: dict[str, list[int]] = allowance.get("modules", {})

    total = 0
    for module in modules:
        name = module.name
        found = ungated_prose_references(_read_utf8(module), markers[name])
        total += len(found)
        budget = Counter(permitted.get(name, []))
        for line in found:
            if budget[line] <= 0:
                raise VerificationError(
                    f"{name}: `world_fold.rs:{line}` is a free-text line number "
                    f"the digest gate does not own. Line numbers drift on the "
                    f"next insertion into world_fold.rs and nothing catches it; "
                    f"cite the item instead, as `world_fold.rs::<item_name>`. "
                    f"(If this reference genuinely cannot be named, add it to "
                    f"{PROSE_ALLOWANCE.name} with the reason.)"
                )
            budget[line] -= 1
    return total


def authority_roots(index: ItemIndex, functions: Iterable[str]) -> list[str]:
    """The indexed idents for a marker's ``functions=`` list."""
    roots: list[str] = []
    for name in functions:
        idents = index.by_name.get(name)
        if not idents:
            raise VerificationError(f"missing world-fold function {name}")
        if len(idents) > 1:
            raise VerificationError(
                f"authority function {name} is defined {len(idents)} times "
                f"({sorted(idents)}); an authority must name one function"
            )
        roots.append(idents[0])
    return roots


def load_closure_inventory(repo_root: Path) -> dict[str, list[str]]:
    """The recorded closure membership, or a hard failure.

    A missing inventory is NOT an empty one.  An empty inventory would make the
    membership check vacuous while the run still printed PASS, which is the
    shape of the defect this whole gate exists to stop.
    """
    path = repo_root / CLOSURE_INVENTORY
    if not path.is_file():
        raise VerificationError(
            f"the authority closure inventory {CLOSURE_INVENTORY} is missing; "
            f"regenerate it with extractor/native/scripts/restamp_world_fold_citations.py"
        )
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VerificationError(
            f"cannot read the authority closure inventory {path}: {error}"
        ) from error
    if not isinstance(loaded, dict):
        raise VerificationError(
            f"{CLOSURE_INVENTORY}: expected a JSON object at the root"
        )
    domains = loaded.get("domains")
    if not isinstance(domains, dict):
        raise VerificationError(
            f"{CLOSURE_INVENTORY}: expected an object under \"domains\""
        )
    # A domain whose value is not a list of names reaches `sorted()` and
    # `set()` later and raises a TypeError there instead of saying what is
    # wrong with the file.
    malformed = sorted(
        key
        for key, members in domains.items()
        if not isinstance(members, list)
        or not all(isinstance(member, str) for member in members)
    )
    if malformed:
        raise VerificationError(
            f"{CLOSURE_INVENTORY.name}: domain(s) {malformed} do not "
            f"record a list of member names"
        )
    return domains


def leaf_attested_files(repo_root: Path, root: str) -> list[str]:
    """Every file a leaf crate's authority covers, repo-relative and sorted.

    THE WHOLE FILE, NOT THE REACHABLE ITEMS.  The fold's authority hashes a
    callee closure because ``world_fold.rs`` is 38,551 lines of which one domain
    owns a few hundred; a leaf crate is the opposite shape -- it exists only to
    be the shared grammar, so there is no residual to exclude and nothing in it
    a consumer does not rely on.  Hashing the files also covers what an item
    walk cannot: crate attributes (``#![no_std]``, ``#![forbid(unsafe_code)]``),
    ``use`` items, ``#[cfg]`` gates, the doc comments the byte goldens were
    written from, and the manifest's dependency table -- a leaf growing a
    dependency is a change to every consumer that took it BECAUSE it had none.
    The cost is that a comment or a test in the leaf re-stamps every consumer;
    that direction is deliberate, and it is the one the closure walk already
    chose for its single-candidate fallback: a redundant member costs a
    re-stamp, a missing one costs the coverage.

    ``tests.rs`` is attested for the same reason.  A leaf's drift-killer test is
    part of what its consumers rely on -- ``d13_grammar``'s is the only thing
    standing between a new tail and four readers that silently disagree about
    where a row ends -- so weakening it is a change a consumer should have to
    re-attest, not a change nothing records.
    """
    base = repo_root / root
    if not base.is_dir():
        raise VerificationError(
            f"leaf authority root {root} is not a directory; a leaf whose "
            f"source cannot be found would attest to nothing"
        )
    if _is_reparse_point(base):
        # The root itself, not only what is under it: a linked root means the
        # whole attested tree is somewhere else.
        raise VerificationError(
            f"leaf authority root {root} is a link; the source it attests is "
            f"not where the registry says it is"
        )
    attested: list[str] = []
    manifest = False
    for path in sorted(base.rglob("*")):
        if _is_reparse_point(path):
            # A link resolves somewhere the digest never reads, and on this
            # platform a junction does the same thing without looking like a
            # link in review.
            raise VerificationError(
                f"leaf authority root {root} contains a link "
                f"({path.relative_to(base).as_posix()}); what it points at is "
                f"outside every digest"
            )
        if not path.is_file():
            continue
        relative = path.relative_to(base)
        if any(part in LEAF_SCAN_SKIP_DIRECTORIES for part in relative.parts[:-1]):
            continue
        if path.name in LEAF_ATTESTED_NAMES:
            manifest = True
        elif path.suffix not in LEAF_ATTESTED_SUFFIXES:
            continue
        attested.append(path.relative_to(repo_root).as_posix())
    if not manifest:
        # A crate with no manifest is not a crate.  Reporting a digest over
        # whatever `.rs` files happened to be under a mistyped root is the
        # vacuous pass this whole gate exists to refuse.
        raise VerificationError(
            f"leaf authority root {root} carries no "
            f"{'/'.join(LEAF_ATTESTED_NAMES)}; it is not a crate root"
        )
    return sorted(attested)


def leaf_manifest_digest(repo_root: Path, attested: Iterable[str]) -> str:
    """A digest of a leaf's attested source, keyed by path.

    Each file contributes its PATH and the digest of its newline-normalized
    bytes, so the digest moves when a file's contents change, when a file is
    added or removed, and when one is renamed -- and does not move when the
    checkout's line endings differ (``_read_utf8`` reads with universal
    newlines, which is the normalization).
    """
    digest = hashlib.sha256()
    for relative in sorted(attested):
        digest.update(relative.encode("utf-8"))
        digest.update(b"\n")
        digest.update(
            hashlib.sha256(
                _read_utf8(repo_root / relative).encode("utf-8")
            ).hexdigest().encode("ascii")
        )
        digest.update(b"\n")
    return digest.hexdigest()


def _read_manifest(path: Path, label: str) -> dict:
    try:
        with path.open("rb") as handle:
            return tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise VerificationError(f"cannot read {label} manifest {path}: {error}") from error


def _is_reparse_point(path: Path) -> bool:
    """A link by any name, including a Windows junction.

    ``Path.is_symlink()`` returns False for a junction, and a junction points
    outside the attested tree exactly as well as a symlink does -- on the
    platform this repository is developed on, which is the one where the check
    would otherwise be missing.
    """
    try:
        status = path.lstat()
    except OSError as error:
        # FAIL CLOSED.  A child this walk enumerated and then could not stat is
        # not a file to skip quietly: `is_file()` would say False for the same
        # reason and the entry would leave the attested set without a word.
        raise VerificationError(
            f"cannot read the metadata of {path}: {error}; a leaf's attested "
            f"set may not be decided by a failed stat"
        ) from error
    attributes = getattr(status, "st_file_attributes", None)
    if attributes is not None:
        return bool(attributes & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0))
    return path.is_symlink()


def _declared_target_paths(manifest: dict) -> Iterable[tuple[str, str]]:
    """Every source path a manifest names, build script included.

    Cargo runs `package.build` exactly as it compiles `[lib]`, so a package
    that names either from somewhere this gate does not read has a walker this
    gate cannot see -- which is the whole reason the paths are inspected.
    """
    package = manifest.get("package")
    if isinstance(package, dict) and isinstance(package.get("build"), str):
        yield "the build script", package["build"]
    for table in ("lib", "bin", "test", "bench", "example"):
        targets = manifest.get(table)
        targets = [targets] if isinstance(targets, dict) else targets
        if not isinstance(targets, list):
            continue
        for target in targets:
            declared = target.get("path") if isinstance(target, dict) else None
            if isinstance(declared, str):
                yield f"[{table}]", declared


def _dependency_tables(manifest: dict) -> Iterable[tuple[str, dict]]:
    """Every dependency table in a manifest, target-conditional ones included.

    `[target.'cfg(windows)'.dependencies]` is a dependency table that a
    top-level scan does not see, and a leaf that grows one has grown behaviour
    from another crate on one platform only -- the hardest kind to notice.
    """
    for table in ("dependencies", "build-dependencies", "dev-dependencies"):
        entry = manifest.get(table)
        if isinstance(entry, dict):
            yield table, entry
    targets = manifest.get("target")
    if isinstance(targets, dict):
        for selector, tables in targets.items():
            if not isinstance(tables, dict):
                continue
            for table in ("dependencies", "build-dependencies", "dev-dependencies"):
                entry = tables.get(table)
                if isinstance(entry, dict):
                    yield f"target.{selector}.{table}", entry


def verify_leaf_is_sealed(repo_root: Path, crate: str, root: str, attested: Iterable[str]) -> None:
    """Refuse a leaf whose behaviour can change with its files unchanged.

    A digest over a crate's source is only an attestation if the source is the
    whole of what the crate executes.  Every shape refused here breaks that:
    the code that runs comes from a build script, another path, a generated
    file, the environment, or another crate whose version a lockfile decides.
    None of them can be hashed around -- a gate that tried would be claiming to
    cover a build system it does not read -- so they are refused, and a leaf
    stays what the name says: source, and nothing else.
    """
    manifest_path = repo_root / root / "Cargo.toml"
    manifest = _read_manifest(manifest_path, f"leaf {crate}")
    package = manifest.get("package")
    if not isinstance(package, dict) or package.get("name") != crate:
        raise VerificationError(
            f"{crate}: {root}/Cargo.toml declares package "
            f"{package.get('name') if isinstance(package, dict) else None!r}; a "
            f"marker naming another crate attests a package nobody builds"
        )
    if package.get("build") is not None:
        raise VerificationError(
            f"{crate}: declares a build script; a build script runs code and "
            f"reads inputs no source digest covers"
        )
    if (repo_root / root / "build.rs").is_file():
        raise VerificationError(
            f"{crate}: carries build.rs, which Cargo runs implicitly; a leaf's "
            f"behaviour must come from its attested source alone"
        )
    library = manifest.get("lib")
    if isinstance(library, dict) and library.get("path") is not None:
        raise VerificationError(
            f"{crate}: sets [lib] path={library['path']!r}; the crate's source "
            f"would be somewhere the attested set does not reach"
        )
    if not (repo_root / root / "src" / "lib.rs").is_file():
        # With no `[lib] path` and no `src/lib.rs`, there is no library here at
        # all: a manifest-only directory would otherwise be stamped, digested
        # and reported as an attested leaf.
        raise VerificationError(
            f"{crate}: has no src/lib.rs and declares no [lib] path; there is "
            f"no library here for a consumer to execute"
        )
    for table, declared in _dependency_tables(manifest):
        if declared:
            raise VerificationError(
                f"{crate}: declares [{table}] ({sorted(declared)}); behaviour "
                f"that arrives through another crate is behaviour a later "
                f"lockfile or patch can change with every attested byte "
                f"identical. A shared grammar leaf has no dependencies -- that "
                f"is what lets every consumer take it."
            )
    features = manifest.get("features")
    if isinstance(features, dict) and features:
        raise VerificationError(
            f"{crate}: declares [features] ({sorted(features)}); any crate in "
            f"the build graph can turn one on, so the code that runs stops "
            f"being a function of the attested source"
        )
    for relative in attested:
        if not relative.endswith(".rs"):
            continue
        source = sanitize_rust(_read_utf8(repo_root / relative))
        for pattern, what in LEAF_FORBIDDEN_SOURCE_RE:
            if pattern.search(source):
                raise VerificationError(
                    f"{crate}: {relative} uses {what}; the code that runs would "
                    f"not be the code this digest covers"
                )
        for pattern, what in LEAF_FORBIDDEN_CONDITIONAL_RE:
            if pattern.search(source):
                raise VerificationError(
                    f"{crate}: {relative} uses {what}; what the crate does "
                    f"would depend on how the consumers are built rather than "
                    f"on the source this digest covers"
                )
        for match in LEAF_CFG_RE.finditer(source):
            predicate = "".join(match.group("predicate").split())
            if predicate not in LEAF_PERMITTED_CFG:
                raise VerificationError(
                    f"{crate}: {relative} carries #[cfg({predicate})]; which "
                    f"branch of the attested source runs would depend on how "
                    f"the consumers are built, not on the digest. Only "
                    f"{sorted(LEAF_PERMITTED_CFG)} are permitted in a leaf."
                )
    verify_leaf_all_tables_are_complete(repo_root, crate, attested)


# A leaf's drift boundary is an `enum` whose whole grammar -- offsets, widths,
# masks, dispatch -- is generated from an `ALL` array by exhaustive `match`es
# with no wildcard arm.  Adding a variant is then a compile error at every site
# that matches on one, WHICH IS THE POINT, and it is why these crates are built
# this way.
#
# The property has one hole and rustc cannot close it: an added variant is a
# compile error in every `match`, and it is NOT an error to leave it out of
# `ALL`.  A variant missing from `ALL` is a family, a flag or a block that every
# table-driven loop, every drift-killer test and every coverage assertion walks
# straight past -- silently, because they all iterate `ALL`.  Closing it inside
# the crate needs `std::mem::variant_count` (unstable) or a derive macro, and a
# sealed leaf may have neither: no dependencies, no `macro_rules!`.
#
# So it is closed HERE, where the leaf's source is already being read for the
# seal.  Codex named this the right home (review thread
# `01a007cf-1a5e-7910-b822-f1c595f5d00d`, round 3).
#
# THIS CHECK FAILS CLOSED, which is the whole of its second round (round 4).
# The first version paired `pub enum` declarations with `impl` blocks at column
# zero and read the source raw, so it SILENTLY SKIPPED an indented `impl`, a
# `pub(crate)` or private enum, a generic or qualified impl, an attribute on the
# same line, an `ALL` outside an impl or in another file, and it miscounted
# braces and variants around comments.  A check that skips is worse than no
# check, because the report says PASS.  So: the source is read through
# `sanitize_rust` (comments and literals blanked, byte count preserved), the
# declarations are found by TOKEN rather than by shape, and every `enum` and
# every `const ALL` the scan finds must be PAIRED AND PARSED or the leaf is
# refused with the thing it could not read.
# `r#ALL` is the SAME identifier as `ALL` -- callers still write `E::ALL` -- so a
# scan anchored on the literal token found no table at all and let an incomplete
# one through as though the enum had none (Codex round 6). The optional `r#` is
# accepted everywhere an identifier is matched.
LEAF_ENUM_TOKEN_RE = re.compile(r"\b(?:r#)?enum\s+(?:r#)?([A-Za-z_][A-Za-z0-9_]*)")
LEAF_ALL_TOKEN_RE = re.compile(r"\bconst\s+(?:r#)?ALL\b")
LEAF_ALL_DECL_RE = re.compile(
    r"\bconst\s+(?:r#)?ALL\s*:\s*\[\s*(?:r#)?(?P<of>[A-Za-z_][A-Za-z0-9_]*)\s*;\s*"
    r"(?P<count>[0-9_]+)\s*\]\s*=\s*"
)
LEAF_IMPL_TOKEN_RE = re.compile(r"(?<![A-Za-z0-9_])(?:r#)?impl\s+(?P<head>[^{;]*)\{")


def _balanced_block(text: str, open_at: int) -> str | None:
    """The text inside the `{...}` whose `{` is at ``open_at``.

    The caller passes SANITIZED source, so braces inside comments and string
    literals are already blanked and cannot end a block early.
    """
    depth = 0
    index = open_at
    while index < len(text):
        character = text[index]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return text[open_at + 1 : index]
        index += 1
    return None


def _declaration_body_brace(text: str, start: int) -> int:
    """The `{` that opens a declaration's BODY, from just after its name.

    Not "the first `{`": a generic bound or a const-generic argument can carry
    one first, and valid Rust such as `enum E where [(); { 1 }]: Sized { A, B }`
    made the first version read `{ 1 }` as the enum's body and then find no
    variants at all -- a silent skip, which is the failure mode this whole check
    exists to avoid (Codex round 5). Braces inside `<>`, `[]` and `()` are
    skipped; the first one at depth zero is the body.

    Returns -1 when there is none, which the caller reports rather than passes
    over.
    """
    angle = square = round_ = 0
    index = start
    while index < len(text):
        character = text[index]
        if character == "<":
            angle += 1
        elif character == ">":
            if angle:
                angle -= 1
        elif character == "[":
            square += 1
        elif character == "]":
            if square:
                square -= 1
        elif character == "(":
            round_ += 1
        elif character == ")":
            if round_:
                round_ -= 1
        elif character == ";" and not (angle or square or round_):
            # A declaration with no body at all (`enum E;` is not legal, but a
            # `pub use`-style line reaching here would be). Nothing to read.
            return -1
        elif character == "{":
            if not (angle or square or round_):
                return index
            depth = 0
            while index < len(text):
                if text[index] == "{":
                    depth += 1
                elif text[index] == "}":
                    depth -= 1
                    if depth == 0:
                        break
                index += 1
        index += 1
    return -1


def _top_level_items(block: str) -> str:
    """A block with every NESTED brace body blanked, byte count preserved.

    An inherent `impl` contains function bodies, and a function body can declare
    its own `const ALL` and its own types. Searching the whole block attributed
    a function-local table to the enclosing enum and checked it against the
    wrong type (Codex round 5). Only depth-one associated items are the impl's
    own.
    """
    out = []
    depth = 0
    for character in block:
        if character == "{":
            depth += 1
            out.append("{" if depth == 1 else " ")
            continue
        if character == "}":
            out.append("}" if depth == 1 else " ")
            depth -= 1
            continue
        out.append(character if depth == 0 else " ")
    return "".join(out)


def _balanced_bracket(text: str, open_at: int) -> str | None:
    """The text inside the `[...]` whose `[` is at ``open_at``.

    Bracket-aware rather than "up to the first `]`", so an attribute or a nested
    array inside an `ALL` table cannot truncate it -- which the first version
    did, and truncation there made the whole table vanish from the check.
    """
    depth = 0
    index = open_at
    while index < len(text):
        character = text[index]
        if character == "[":
            depth += 1
        elif character == "]":
            depth -= 1
            if depth == 0:
                return text[open_at + 1 : index]
        index += 1
    return None


def _enum_variants(body: str) -> list[str]:
    """The variant names of a SANITIZED enum body.

    Payload variants (`A(u32)`), struct variants (`A { x: u8 }`) and explicit
    discriminants (`A = 3`) all reduce to their leading identifier.
    """
    without_attributes = re.sub(r"#\[[^\]]*\]", " ", body)
    variants: list[str] = []
    depth = 0
    current = ""
    for character in without_attributes:
        if character in "({[":
            depth += 1
        elif character in ")}]":
            depth -= 1
        if character == "," and depth == 0:
            variants.append(current)
            current = ""
        else:
            current += character
    variants.append(current)
    names = []
    for variant in variants:
        stripped = variant.strip()
        if not stripped:
            continue
        match = re.match(r"^(?:r#)?([A-Za-z_][A-Za-z0-9_]*)", stripped)
        if match:
            names.append(match.group(1))
    return names


def _impl_target(head: str) -> str | None:
    """The type an `impl ... {` head is for, or None if this scan cannot tell.

    Handles `impl X`, `impl<T> X<T>`, `impl X for Y` (returns None -- a trait
    impl carries no inherent `ALL`), and a qualified `impl a::b::X`.
    """
    text = head.strip()
    if text.startswith("<"):
        depth = 0
        for index, character in enumerate(text):
            if character == "<":
                depth += 1
            elif character == ">":
                depth -= 1
                if depth == 0:
                    text = text[index + 1 :].strip()
                    break
        else:
            return None
    if re.search(r"(?<![A-Za-z0-9_])for(?![A-Za-z0-9_])", text):
        return None
    text = re.sub(r"(?<![A-Za-z0-9_])where(?![A-Za-z0-9_]).*$", "", text).strip()
    text = text.split("<")[0].strip()
    text = text.split("::")[-1].strip()
    if text.startswith("r#"):
        text = text[2:]
    return text or None


def verify_leaf_all_tables_are_complete(
    repo_root: Path, crate: str, attested: Iterable[str]
) -> None:
    """Every `enum` with an `ALL` table lists every one of its variants.

    Refused: an `ALL` shorter than its enum, an `ALL` whose declared length
    disagrees with what it contains, a duplicate, an entry that is not a variant
    -- and, because a silent skip is the failure mode that matters here, any
    `enum` or `const ALL` this scan finds and cannot pair and parse.

    Not refused, and not refusable: an enum with no `ALL` at all. That is an
    enum this idiom was never applied to, and there is nothing to be complete
    against.
    """
    enums: dict[str, list[str]] = {}
    enum_sources: dict[str, str] = {}
    tables: list[tuple[str, str, str, int, list[str]]] = []
    unreadable: list[str] = []
    all_tokens = 0
    paired_tables = 0

    for relative in sorted(attested):
        path = repo_root / relative
        if path.suffix != ".rs":
            continue
        source = sanitize_rust(_read_utf8(path))

        for match in LEAF_ENUM_TOKEN_RE.finditer(source):
            name = match.group(1)
            brace = _declaration_body_brace(source, match.end())
            if brace < 0:
                unreadable.append(f"{relative}: `enum {name}` has no body this scan can find")
                continue
            body = _balanced_block(source, brace)
            if body is None:
                unreadable.append(f"{relative}: `enum {name}`'s body is unbalanced")
                continue
            if name in enums:
                unreadable.append(
                    f"{relative}: a second `enum {name}` (first in "
                    f"{enum_sources[name]}); this scan cannot tell which one an "
                    f"`ALL` table belongs to"
                )
                continue
            enums[name] = _enum_variants(body)
            enum_sources[name] = relative

        all_tokens += len(LEAF_ALL_TOKEN_RE.findall(source))

        for match in LEAF_IMPL_TOKEN_RE.finditer(source):
            target = _impl_target(match.group("head"))
            whole = _balanced_block(source, match.end() - 1)
            if whole is None:
                unreadable.append(f"{relative}: an `impl` block is unbalanced")
                continue
            # Only the impl's OWN associated items. A `const ALL` inside one of
            # its function bodies belongs to that function, not to this type.
            block = _top_level_items(whole)
            nested_tables = len(LEAF_ALL_TOKEN_RE.findall(whole)) - len(
                LEAF_ALL_TOKEN_RE.findall(block)
            )
            if nested_tables:
                unreadable.append(
                    f"{relative}: an `impl` block carries {nested_tables} "
                    f"`const ALL` inside a function body; this scan reads "
                    f"associated items only and will not guess which type such "
                    f"a table is for"
                )
                continue
            if not LEAF_ALL_TOKEN_RE.search(block):
                continue
            if target is None:
                unreadable.append(
                    f"{relative}: an `impl {match.group('head').strip()}{{` carries "
                    f"a `const ALL` and this scan cannot tell what type it is for"
                )
                continue
            declaration = LEAF_ALL_DECL_RE.search(block)
            if declaration is None:
                unreadable.append(
                    f"{relative}: `{target}` carries a `const ALL` this scan "
                    f"cannot read as `[Type; N] = [...]`"
                )
                continue
            bracket = block.find("[", declaration.end())
            entries_text = (
                _balanced_bracket(block, bracket) if bracket >= 0 else None
            )
            if entries_text is None:
                unreadable.append(
                    f"{relative}: `{target}::ALL`'s entry list is unbalanced"
                )
                continue
            entries = [
                entry.split("::")[-1].strip().removeprefix("r#")
                for entry in entries_text.split(",")
                if entry.strip()
            ]
            declared = int(declaration.group("count").replace("_", ""))
            tables.append((relative, target, declaration.group("of"), declared, entries))
            paired_tables += 1

    if unreadable:
        raise VerificationError(
            f"{crate}: this gate could not read {len(unreadable)} declaration(s) "
            f"in the attested source, and a shape it cannot read is a shape it "
            f"would pass over: " + "; ".join(sorted(unreadable))
        )
    if paired_tables != all_tokens:
        raise VerificationError(
            f"{crate}: found {all_tokens} `const ALL` declaration(s) in the "
            f"attested source and could pair {paired_tables} with an enum. An "
            f"`ALL` this gate does not read is an `ALL` it does not check."
        )

    for relative, target, of, declared, entries in tables:
        if target not in enums:
            raise VerificationError(
                f"{crate}: {relative}: `{target}::ALL` is on a type this scan "
                f"found no `enum` for; it cannot be checked for completeness"
            )
        if of != target:
            raise VerificationError(
                f"{crate}: {relative}: `{target}::ALL` is declared as `[{of}; "
                f"{declared}]`, an array of another type"
            )
        variants = enums[target]
        if len(entries) != declared:
            raise VerificationError(
                f"{crate}: {relative}: {target}::ALL declares {declared} "
                f"entries and lists {len(entries)}"
            )
        missing = [variant for variant in variants if variant not in entries]
        if missing:
            raise VerificationError(
                f"{crate}: {relative}: {target}::ALL omits {missing}. Every "
                f"table-driven loop, drift-killer and coverage assertion in "
                f"this crate iterates ALL, so a variant missing from it is a "
                f"variant nothing walks -- and rustc cannot catch that, "
                f"because leaving a variant out of an array is not an error "
                f"the way leaving one out of a `match` is."
            )
        extra = [entry for entry in entries if entry not in variants]
        if extra:
            raise VerificationError(
                f"{crate}: {relative}: {target}::ALL lists {extra}, which are "
                f"not variants of {target}"
            )
        duplicated = sorted({entry for entry in entries if entries.count(entry) > 1})
        if duplicated:
            raise VerificationError(
                f"{crate}: {relative}: {target}::ALL lists {duplicated} more "
                f"than once"
            )


def _nearest_manifest(repo_root: Path, relative: str) -> Path | None:
    """The Cargo.toml Cargo reads for a source file: the nearest one above it."""
    for parent in (repo_root / relative).parents:
        candidate = parent / "Cargo.toml"
        if candidate.is_file():
            return candidate
        if parent == repo_root:
            break
    return None


def verify_leaf_dependency_edges(
    repo_root: Path,
    crate: str,
    root: str,
    consumers: Iterable[str],
    const_only: Iterable[str],
    scanned: Iterable[Path],
    failures: list[str],
) -> set[str]:
    """Every package that can reach the leaf is bound to it, and named it.

    Returns the repo-relative directories of the packages that DEPEND on the
    leaf, which is the set `verify_no_hidden_modules` then has to look inside.

    Two things are checked here, and they are the same check from both ends.

    FROM THE CONSUMER: a marker says "I execute this crate", and only Cargo
    decides which crate that is.  A site whose package re-points the dependency
    at a registry version, a vendored copy or a fork keeps compiling, keeps its
    marker green, and stops executing the attested source.

    FROM THE MANIFESTS: any package that depends on the leaf can walk it, and a
    package with no registered site is one whose files the marker scan and the
    source scan both pass over.  A Cargo ALIAS is what makes that invisible --
    ``grammar = { package = "d13_grammar" }`` and the source never spells the
    crate's name at all -- so the dependency must be spelled with the crate's
    own name, everywhere.  That one restriction is what makes the source scan
    in ``verify_no_unregistered_users`` mean what it says, and the three
    packages that take d13_grammar already spell it that way.
    """
    leaf_directory = (repo_root / root).resolve()
    dependent: set[str] = set()
    owning: dict[str, Path] = {}
    for relative in sorted(set(consumers) | set(const_only)):
        manifest_path = _nearest_manifest(repo_root, relative)
        if manifest_path is None:
            failures.append(
                f"{relative}: no Cargo.toml above it, so nothing binds this "
                f"site to {crate}"
            )
            continue
        owning[relative] = manifest_path

    # EVERY manifest under the scan roots, found by walking for manifests --
    # not by asking which manifest owns each scanned `.rs`.  A crate can name a
    # library `[lib] path = "source.rust"`, which Cargo compiles happily and a
    # `*.rs` walk never sees; deriving the manifest set from the sources it
    # found made that package invisible, dependency and all.
    manifests = set(leaf_scan_manifests(repo_root))
    manifests.update(owning.values())
    bound_manifests = set(owning.values())

    for manifest_path in sorted(manifests):
        try:
            manifest = _read_manifest(manifest_path, f"{crate} consumer")
        except VerificationError as error:
            failures.append(str(error))
            continue
        relative_manifest = manifest_path.relative_to(repo_root).as_posix()
        if relative_manifest == f"{root}/Cargo.toml":
            continue
        # EVERY entry that could be this crate, not the first one found: a
        # manifest carrying both a redirected `d13_grammar = { package = "fork" }`
        # and an unused correct alias would otherwise pass on the alias while
        # the source executes the fork.
        matching: list[tuple[str, object]] = []
        for table, entry in _dependency_tables(manifest):
            for key, value in entry.items():
                named = value.get("package") if isinstance(value, dict) else None
                if named == crate or key == crate:
                    matching.append((f"{table}.{key}", value))
        if not matching:
            continue
        dependent.add(manifest_path.parent.relative_to(repo_root).as_posix())
        # A package that takes the leaf must LIVE where this gate reads source.
        # A crate under `runs/` compiles, tests and walks the grammar exactly
        # like any other, and the source scan skips that directory as prose --
        # so the package, its marker and its walker were all invisible while
        # this gate said PASS.
        package_parts = manifest_path.parent.relative_to(repo_root).parts
        hidden = sorted(
            {part for part in package_parts if part in LEAF_SCAN_SKIP_DIRECTORIES}
        )
        if hidden:
            failures.append(
                f"{relative_manifest}: depends on {crate} and lives under "
                f"{hidden}, which this gate's source scan skips; nothing in "
                f"this package can be attested, so a walker here is invisible. "
                f"Move the package, or drop the dependency."
            )
        # It may not hide its source behind a target path this scan cannot read
        # either.  `[lib] path = "source.rust"` compiles,
        # `build = "build.rust"` runs, and `[lib] path = "runs/source.rs"` sits
        # inside a directory this scan skips -- and every source-level check
        # below (the marker, the re-export rule, the unregistered-user scan)
        # steps straight over all three.
        for table, declared_source in _declared_target_paths(manifest):
            unreadable = None
            if not declared_source.endswith(".rs"):
                unreadable = "this gate's source scan reads only `.rs` files"
            else:
                resolved = (manifest_path.parent / declared_source).resolve()
                try:
                    inside = resolved.relative_to(repo_root)
                except ValueError:
                    unreadable = "it is outside the repository"
                else:
                    if any(
                        part in LEAF_SCAN_SKIP_DIRECTORIES for part in inside.parts
                    ):
                        unreadable = (
                            f"it sits under a directory this scan skips "
                            f"({sorted(LEAF_SCAN_SKIP_DIRECTORIES)})"
                        )
            if unreadable is not None:
                failures.append(
                    f"{relative_manifest}: depends on {crate} and builds "
                    f"{table} from {declared_source!r}, which this gate's "
                    f"source scan does not read ({unreadable}); a walker there "
                    f"would be invisible to every check in this file"
                )
        if manifest_path not in bound_manifests:
            failures.append(
                f"{relative_manifest}: depends on {crate} and owns no "
                f"registered consumer or const-only site; a package that can "
                f"walk the grammar with no attestation is the defect this "
                f"gate exists to stop"
            )
        for where, value in matching:
            key = where.rsplit(".", 1)[1]
            named = value.get("package") if isinstance(value, dict) else None
            if key != crate or (named is not None and named != crate):
                failures.append(
                    f"{relative_manifest}: declares {crate} as {where}"
                    + (f" (package={named})" if named else "")
                    + f"; the dependency must be spelled {crate}, because the "
                    f"source scan that finds an unattested walker looks for "
                    f"that name and an alias hides it"
                )
                continue
            declared = value
            if isinstance(declared, dict) and declared.get("workspace") is True:
                failures.append(
                    f"{relative_manifest}: inherits {crate} from the "
                    f"workspace; the path this gate attests must be declared "
                    f"where the dependency is, not somewhere a later workspace "
                    f"edit can move under it"
                )
                continue
            declared_path = declared.get("path") if isinstance(declared, dict) else None
            if declared_path is None:
                failures.append(
                    f"{relative_manifest}: depends on {crate} without a path; "
                    f"the attested source is this repository's copy and nothing "
                    f"says that is what Cargo resolves"
                )
                continue
            resolved = (manifest_path.parent / declared_path).resolve()
            if resolved != leaf_directory:
                failures.append(
                    f"{relative_manifest}: depends on {crate} at "
                    f"{declared_path}, which resolves outside the registered "
                    f"root {root}"
                )

    for relative, manifest_path in sorted(owning.items()):
        try:
            manifest = _read_manifest(manifest_path, f"{crate} consumer")
        except VerificationError:
            continue  # already reported above
        if not any(
            key == crate or (isinstance(value, dict) and value.get("package") == crate)
            for _, entry in _dependency_tables(manifest)
            for key, value in entry.items()
        ):
            failures.append(
                f"{relative}: {manifest_path.relative_to(repo_root).as_posix()} "
                f"does not depend on {crate}; the marker attests a crate this "
                f"package does not build against"
            )
    return dependent


def verify_no_hidden_modules(
    repo_root: Path,
    crate: str,
    dependent: Iterable[str],
    scanned: Iterable[Path],
    scannable: set[str],
    failures: list[str],
) -> None:
    """A package that takes the leaf may not include source from outside the scan.

    Cargo's target paths are not the only way to compile a file this gate does
    not read: `#[path = "../../runs/hidden.rs"] mod hidden;` inside a perfectly
    ordinary registered consumer pulls in a module from an evidence directory,
    and that module can walk the grammar while every visible file is clean.
    The redirect is refused unless it lands on a file this scan reads.
    """
    prefixes = tuple(f"{directory}/" for directory in sorted(dependent))
    if not prefixes:
        return
    for path in sorted(scanned):
        relative = path.relative_to(repo_root).as_posix()
        if not relative.startswith(prefixes):
            continue
        # EVERY source in the package, not only the ones the leaf scan decoded:
        # the file carrying the redirect need not mention the crate at all.
        # `path` as a byte substring is the pre-filter; the attribute pattern
        # below is what decides.
        try:
            raw = path.read_bytes()
        except OSError as error:
            raise VerificationError(f"cannot read {path}: {error}") from error
        if b"path" not in raw:
            continue
        source = _read_utf8(path)
        # The attribute's VALUE is a string literal, which `sanitize_rust`
        # blanks -- so the pattern runs over the raw text, and the sanitized
        # copy (same length, comments blanked to spaces) is what says whether
        # the match is code or a sentence about code.
        code = sanitize_rust(source)
        for match in MODULE_PATH_ATTRIBUTE_RE.finditer(source):
            if code[match.start()] != "#":
                continue
            declared = match.group("path")
            # Rust resolves a `#[path]` on a module relative to the directory
            # of the file carrying it (for a `mod.rs`/crate root; the nested
            # form differs, and both candidates are tried before refusing).
            here = PurePosixPath(relative).parent
            candidates = {
                (here / declared).as_posix(),
                (here / PurePosixPath(relative).stem / declared).as_posix(),
            }
            # `posixpath.normpath`, never `os.path`: on Windows the latter
            # returns backslashes, which then match nothing in a set of
            # repo-relative POSIX paths and turn every redirect into a
            # failure -- including the two legitimate ones this repository
            # already has.
            resolved = {posixpath.normpath(candidate) for candidate in candidates}
            if resolved & scannable:
                continue
            # An INLINE module changes the base again -- `mod outer { #[path =
            # "inner.rs"] mod inner; }` loads `outer/inner.rs` -- and this
            # check does not parse module nesting.  So a redirect that lands on
            # some file inside the same package is accepted: the point of the
            # rule is a source OUTSIDE the scan, and refusing legal Rust
            # because the base was guessed wrong would be the false failure
            # that teaches people to route around this gate.
            tail = "/" + posixpath.normpath(declared)
            package = next(
                (prefix for prefix in prefixes if relative.startswith(prefix)), None
            )
            if package is not None and any(
                candidate.startswith(package) and candidate.endswith(tail)
                for candidate in scannable
            ):
                continue
            failures.append(
                f"{relative}: takes {crate} and includes {declared!r} with a "
                f"`#[path]` module, which this gate's source scan does not "
                f"read; a walker there is invisible to every check in this "
                f"file"
            )


def verify_no_unregistered_users(
    crate: str,
    consumers: Iterable[str],
    root: str,
    allowance: dict[str, tuple[str, ...]],
    sources: dict[str, str],
    failures: list[str],
) -> None:
    """A marker scan finds markers; it cannot find a consumer nobody marked.

    So the leaf is looked for from the other end: every file that names the
    crate must be a registered consumer, the leaf itself, or an entry in the
    per-leaf allowance -- which records the IDENTIFIERS it may name, not merely
    the file, so a `const`-only re-export cannot quietly grow a walk.

    A RE-EXPORT would defeat it -- a registered consumer writing
    ``pub use d13_grammar::...`` lets a downstream file walk the grammar
    without ever naming the crate -- so a public re-export of the leaf is
    refused wherever it appears.  Together with the "no Cargo alias" rule in
    ``verify_leaf_dependency_edges``, that is what makes "names the crate" the
    same set as "can reach the crate".

    KNOWN LIMIT: a local module whose name equals the crate's shadows it, and
    this reads the two the same way -- it refuses, rather than resolving names
    the way rustc would.  A leaf crate is named after its domain; a module of
    the same name in another crate is a collision worth a human's attention.
    """
    registered = set(consumers)
    leaf_prefix = f"{root}/"
    reference = re.compile(
        rf"(?<![A-Za-z0-9_]){re.escape(crate)}\s*::\s*(?P<item>[A-Za-z_][A-Za-z0-9_]*)"
    )
    # EVERY OCCURRENCE OF THE NAME IN CODE COUNTS.  An earlier version asked for
    # a `crate::` path, a `use` or an `extern crate`, which reads as the careful
    # choice and is not: `use {d13_grammar as g};` (a grouped import) and
    # `macro_rules! call { ($c:ident) => { $c::walk() } } call!(d13_grammar)`
    # both reach the leaf while matching none of those, and each was measured
    # passing this gate.  Distinguishing "names the crate" from "uses the crate"
    # needs the name resolution rustc does; a regex that tries loses
    # completeness, which is the whole point of this scan.  So a bare token is a
    # use, and the cost -- a local item that shares the crate's name is refused
    # -- is a collision worth a human's attention anyway.
    token = re.compile(rf"(?<![A-Za-z0-9_]){re.escape(crate)}(?![A-Za-z0-9_])")
    # A public `use`/`extern crate` ITEM that mentions the crate, in any of its
    # spellings: `pub use d13_grammar::walk;`, `pub use {d13_grammar::walk};`,
    # `pub(crate) use d13_grammar as g;`, `pub extern crate d13_grammar;`.  Each
    # lets a downstream file reach the grammar without naming the crate, which
    # is the one route the token scan above cannot see.
    public_item = re.compile(
        r"(?m)^[ \t]*pub(?:\s*\([^)]*\))?\s+(?:use|extern\s+crate)\b[^;]*;"
    )
    for relative, source in sorted(sources.items()):
        if relative.startswith(leaf_prefix):
            continue
        if crate not in source:
            continue
        # Comments and string literals name the crate all over this tree
        # (including in the markers themselves); only code counts.
        code = sanitize_rust(source)
        for item in public_item.finditer(code):
            if token.search(item.group(0)):
                failures.append(
                    f"{relative}: publicly re-exports {crate} "
                    f"(`{' '.join(item.group(0).split())}`); a downstream file "
                    f"could then walk the grammar without naming the crate, "
                    f"which is the one thing this scan cannot see"
                )
        if relative in registered:
            continue
        if not token.search(code):
            continue
        permitted = allowance.get(relative)
        if permitted is None:
            failures.append(
                f"{relative}: executes {crate} and is not a registered "
                f"consumer. Add it to REQUIRED_LEAF_AUTHORITIES with a marker, "
                f"or record it as a const-only user."
            )
            continue
        # EVERY occurrence must be an allowed `crate::IDENT`, not merely every
        # direct reference: `use grammar_leaf as g;` followed by `g::walk()`
        # names no item at all, so an identifier-only check reads it as a file
        # that touches nothing.
        occurrences = len(token.findall(code))
        allowed = [
            match
            for match in reference.finditer(code)
            if match.group("item") in set(permitted)
        ]
        named = sorted(
            {
                match.group("item")
                for match in reference.finditer(code)
                if match.group("item") not in set(permitted)
            }
        )
        if named or len(allowed) != occurrences:
            failures.append(
                f"{relative}: recorded as a const-only user of {crate}, and "
                + (
                    f"names {named}"
                    if named
                    else f"reaches it in {occurrences - len(allowed)} way(s) "
                    f"that are not a recorded `{crate}::<const>` reference"
                )
                + "; a site that reads more than the recorded constants is a "
                "consumer and needs its own attestation"
            )


def _walk_pruned(base: Path, pattern_name: str, skip: frozenset[str]) -> list[Path]:
    """Walk `base` for files named `pattern_name` (or `*.rs` when it starts
    with `*.`), PRUNING skipped directories instead of visiting-then-filtering.

    `Path.rglob` descends into every directory first and filters afterwards, so
    a skipped tree that is being written by someone else at that moment -- a
    lane worktree's `.cargo-target` under `.claude/`, whose rustc scratch dirs
    appear and vanish mid-walk -- raised FileNotFoundError from inside the
    walk and failed a landing gate over a directory this gate never wanted to
    read (2026-08-16). Pruning at os.walk time means skipped trees are never
    entered at all; a directory that vanishes elsewhere is a real error still.
    """
    import os
    suffix = pattern_name[1:] if pattern_name.startswith("*.") else None
    found: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:] = [d for d in dirnames if d not in skip]
        for name in filenames:
            if (suffix is not None and name.endswith(suffix)) or name == pattern_name:
                found.append(Path(dirpath) / name)
    return found


def leaf_scan_manifests(repo_root: Path) -> list[Path]:
    """Every Cargo manifest under the scan roots.

    Found by walking for manifests, because the packages this gate most needs
    to see are exactly the ones whose SOURCE it cannot: a crate whose library is
    `[lib] path = "source.rust"` compiles fine and contains no file a `*.rs`
    walk visits.
    """
    found: list[Path] = []
    for root in LEAF_MARKER_SCAN_ROOTS:
        base = repo_root / root
        if not base.is_dir():
            raise VerificationError(
                f"leaf marker scan root {root.as_posix()} is missing; the scan "
                f"that catches an unregistered marker would cover nothing"
            )
        # Pruned at walk time (see `_walk_pruned`); parts are still checked
        # relative to `base`, so a checkout living under a skipped name is not
        # skipped wholesale.
        for path in _walk_pruned(base, "Cargo.toml", LEAF_MANIFEST_SKIP_DIRECTORIES):
            if any(
                part in LEAF_MANIFEST_SKIP_DIRECTORIES
                for part in path.relative_to(base).parts
            ):
                continue
            found.append(path)
    return sorted(found)


def leaf_scan_files(repo_root: Path) -> list[Path]:
    """Every Rust source a stray leaf marker could be hiding in."""
    found: list[Path] = []
    for root in LEAF_MARKER_SCAN_ROOTS:
        base = repo_root / root
        if not base.is_dir():
            raise VerificationError(
                f"leaf marker scan root {root.as_posix()} is missing; the scan "
                f"that catches an unregistered marker would cover nothing"
            )
        # Pruned at walk time (see `_walk_pruned`).  Relative parts, not the
        # absolute path's: a checkout that happens to live under a directory
        # called `runs` or `target` -- a lane worktree can -- would otherwise
        # skip the entire tree and report a confident PASS over nothing.
        for path in _walk_pruned(base, "*.rs", LEAF_SCAN_SKIP_DIRECTORIES):
            if any(
                part in LEAF_SCAN_SKIP_DIRECTORIES
                for part in path.relative_to(base).parts
            ):
                continue
            found.append(path)
    return sorted(found)


def load_leaf_inventory(repo_root: Path) -> dict[str, dict]:
    """The recorded attested-file membership, or a hard failure.

    Missing is NOT empty, for the reason ``load_closure_inventory`` gives: an
    empty membership record makes the set check vacuous while the run still
    prints PASS.
    """
    path = repo_root / LEAF_INVENTORY
    if not path.is_file():
        raise VerificationError(
            f"the leaf source authority inventory {LEAF_INVENTORY} is missing; "
            f"regenerate it with "
            f"extractor/native/scripts/restamp_world_fold_citations.py"
        )
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VerificationError(
            f"cannot read the leaf source authority inventory {path}: {error}"
        ) from error
    if not isinstance(loaded, dict):
        raise VerificationError(f"{LEAF_INVENTORY}: expected a JSON object at the root")
    leaves = loaded.get("leaves")
    if not isinstance(leaves, dict):
        raise VerificationError(f'{LEAF_INVENTORY}: expected an object under "leaves"')
    malformed = sorted(
        name
        for name, record in leaves.items()
        if not isinstance(record, dict)
        or not isinstance(record.get("root"), str)
        or not isinstance(record.get("files"), list)
        or not all(isinstance(member, str) for member in record["files"])
    )
    if malformed:
        raise VerificationError(
            f"{LEAF_INVENTORY.name}: leaf/leaves {malformed} do not record a "
            f"root and a list of attested file paths"
        )
    return leaves


def validate_leaf_registry(registry: dict[str, LeafAuthority]) -> None:
    """The registry itself is checked, not trusted.

    It is a Python literal rather than a data file, which keeps a REQUIRED set
    changing through code review exactly as ``REQUIRED_LANE_AUTHORITIES`` does
    -- but "hand-written" is not "correct", and a duplicated consumer, a path
    that escapes the repository or a const-only entry that is also a registered
    consumer would each make some check below quietly weaker.  The one shape
    this cannot see is a duplicated dict KEY, which Python resolves before this
    function is reached; that is what review and a linter are for.
    """
    for crate, leaf in sorted(registry.items()):
        if not isinstance(leaf, LeafAuthority):
            raise VerificationError(
                f"REQUIRED_LEAF_AUTHORITIES[{crate!r}] is not a LeafAuthority"
            )
        paths = [("root", leaf.root)]
        paths += [("consumer", consumer) for consumer in leaf.consumers]
        paths += [("const-only", relative) for relative in leaf.const_only]
        for what, value in paths:
            if (
                not value
                or value != PurePosixPath(value).as_posix()
                or value.startswith("/")
                or ".." in PurePosixPath(value).parts
                or PurePosixPath(value).is_absolute()
            ):
                raise VerificationError(
                    f"{crate}: {what} path {value!r} is not a normalized "
                    f"repository-relative POSIX path"
                )
        if not leaf.consumers:
            raise VerificationError(
                f"{crate}: registers no consumer site, so nothing attests it"
            )
        duplicates = sorted(
            {
                consumer
                for consumer in leaf.consumers
                if leaf.consumers.count(consumer) > 1
            }
        )
        if duplicates:
            raise VerificationError(
                f"{crate}: registers {duplicates} more than once; the marker "
                f"count per site would be checked against the wrong total"
            )
        overlap = sorted(set(leaf.consumers) & set(leaf.const_only))
        if overlap:
            raise VerificationError(
                f"{crate}: {overlap} are registered as consumers AND recorded "
                f"as const-only users; a site cannot be both attested and exempt"
            )
        for relative, identifiers in sorted(leaf.const_only.items()):
            if not identifiers or not all(
                re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name) for name in identifiers
            ):
                raise VerificationError(
                    f"{crate}: const-only entry {relative} must name the "
                    f"identifiers it may read"
                )


def verify_leaf_authorities(
    repo_root: Path, registry: dict[str, LeafAuthority] | None = None
) -> tuple[int, int]:
    """Every consumer of a shared leaf grammar attests to the leaf's source.

    Returns ``(leaves, consumer sites)``.  Failures accumulate, for the reason
    the module checks accumulate: a run that stops at the first stale marker
    reports one site and hides the other four.
    """
    if registry is None:
        # Read at CALL time, not bound as a default: the tests drive this with a
        # synthetic registry over a synthetic tree, and a default bound at
        # definition time would silently ignore them.
        registry = REQUIRED_LEAF_AUTHORITIES
    validate_leaf_registry(registry)
    if not registry:
        return 0, 0

    recorded = load_leaf_inventory(repo_root)
    failures: list[str] = []

    orphaned = sorted(set(recorded) - set(registry))
    if orphaned:
        raise VerificationError(
            f"{LEAF_INVENTORY.name} records attested files for leaf/leaves "
            f"{orphaned}, which no consumer is required to attest; nothing "
            f"checks those entries. Re-stamp to drop them."
        )

    # Every marker in the tree, so an unregistered or duplicated one is a
    # failure rather than decoration.  A marker copied into a file nobody
    # verifies is worse than no marker: it reads as an attestation.
    #
    # ONE PASS, AND A BYTE PRE-FILTER.  The scan set is 53 MB of Rust; decoding
    # all of it to look for two short tokens cost more than the rest of this
    # gate put together, and both this check and the unregistered-user scan
    # want the same files.  Bytes are read once, the interesting ones are
    # decoded once, and the rest are never decoded at all.
    scanned = leaf_scan_files(repo_root)
    scanned_relative = {path.relative_to(repo_root).as_posix() for path in scanned}
    tokens = [b"@leaf-source-authority"] + [
        crate.encode("utf-8") for crate in sorted(registry)
    ]
    sources: dict[str, str] = {}
    for path in scanned:
        try:
            raw = path.read_bytes()
        except OSError as error:
            raise VerificationError(f"cannot read {path}: {error}") from error
        if not any(token in raw for token in tokens):
            continue
        sources[path.relative_to(repo_root).as_posix()] = _read_utf8(path)

    found: dict[str, list[re.Match[str]]] = {}
    for relative, source in sorted(sources.items()):
        if "@leaf-source-authority" not in source:
            continue
        markers = list(LEAF_MARKER_RE.finditer(source))
        attempts = [
            match.group(0).strip()
            for match in LEAF_MARKER_ATTEMPT_RE.finditer(source)
        ]
        if len(attempts) != len(markers):
            # A line that begins as a marker and does not parse is a broken
            # attestation.  Reading it as an absent marker would report the
            # site as un-attested at best and, where another site still carries
            # a good marker, say nothing at all.
            unparsed = [
                line
                for line in attempts
                if not LEAF_MARKER_RE.fullmatch(line)
            ]
            failures.append(
                f"{relative}: {len(unparsed)} line(s) begin as a leaf "
                f"authority marker and do not parse as one: {unparsed}"
            )
            continue
        if markers:
            found[relative] = markers

    expected_sites: dict[str, set[str]] = {}
    for crate, leaf in sorted(registry.items()):
        root, consumers = leaf.root, leaf.consumers
        expected_sites[crate] = set(consumers)
        outside = sorted(set(consumers) - scanned_relative)
        if outside:
            # A consumer the stray-marker scan cannot reach is a site whose
            # marker could be duplicated somewhere this gate never looks.
            scan_roots = [scan_root.as_posix() for scan_root in LEAF_MARKER_SCAN_ROOTS]
            failures.append(
                f"{crate}: registered consumer(s) {outside} are not Rust "
                f"sources under {scan_roots}"
            )
        try:
            attested = leaf_attested_files(repo_root, root)
            digest = leaf_manifest_digest(repo_root, attested)
        except VerificationError as error:
            failures.append(str(error))
            continue
        # WHAT THE DIGEST IS A DIGEST OF. These three answer the question the
        # file hashes cannot: are the attested files the whole of what runs
        # (`verify_leaf_is_sealed`), is the crate a consumer builds against the
        # one that was attested (`..._consumers_bind_to_it`), and is the
        # registered consumer set the whole of what executes the leaf
        # (`..._no_unregistered_users`). Without them a green marker means the
        # attested bytes did not change, not that they are the bytes that ran.
        try:
            verify_leaf_is_sealed(repo_root, crate, root, attested)
        except VerificationError as error:
            failures.append(str(error))
        dependent = verify_leaf_dependency_edges(
            repo_root, crate, root, consumers, leaf.const_only, scanned, failures
        )
        verify_no_hidden_modules(
            repo_root, crate, dependent, scanned, scanned_relative, failures
        )
        verify_no_unregistered_users(
            crate, consumers, root, leaf.const_only, sources, failures
        )

        record = recorded.get(crate)
        if record is None:
            failures.append(
                f"{crate}: no attested file membership recorded in "
                f"{LEAF_INVENTORY.name}"
            )
        else:
            if record["root"] != root:
                failures.append(
                    f"{crate}: {LEAF_INVENTORY.name} records root "
                    f"{record['root']}, the registry says {root}"
                )
            if sorted(record["files"]) != attested:
                dropped = sorted(set(record["files"]) - set(attested))
                gained = sorted(set(attested) - set(record["files"]))
                failures.append(
                    f"{crate}: attested file membership changed; no longer "
                    f"attested={dropped}, newly attested={gained}. A file "
                    f"leaving a leaf is the leaf's logic leaving every "
                    f"consumer's digest -- confirm where it went before "
                    f"re-stamping."
                )

        for consumer in sorted(consumers):
            markers = [
                marker
                for marker in found.get(consumer, [])
                if marker.group("crate") == crate
            ]
            if len(markers) != 1:
                failures.append(
                    f"{consumer}: expected exactly one {crate} leaf authority "
                    f"marker, found {len(markers)}"
                )
                continue
            marker = markers[0]
            if marker.group("root") != root:
                failures.append(
                    f"{consumer}: {crate} marker attests root "
                    f"{marker.group('root')}, the registry says {root}"
                )
            if int(marker.group("files")) != len(attested):
                failures.append(
                    f"{consumer}: {crate} marker attests "
                    f"{marker.group('files')} file(s), the crate has "
                    f"{len(attested)}"
                )
            if marker.group("sha256") != digest:
                failures.append(
                    f"{consumer}: {crate} leaf authority digest changed over "
                    f"{len(attested)} attested file(s); the shared grammar was "
                    f"edited and this consumer has not re-attested to it. "
                    f"Review what moved, then re-stamp."
                )

    for consumer, markers in sorted(found.items()):
        for marker in markers:
            crate = marker.group("crate")
            if crate not in registry:
                failures.append(
                    f"{consumer}: leaf authority marker for unregistered crate "
                    f"{crate}; nothing verifies it. Add it to "
                    f"REQUIRED_LEAF_AUTHORITIES or remove the marker."
                )
            elif consumer not in expected_sites[crate]:
                failures.append(
                    f"{consumer}: leaf authority marker for {crate} at a site "
                    f"the registry does not list; a marker nothing is required "
                    f"to keep current reads as an attestation and is not one."
                )

    if failures:
        raise VerificationError(
            f"{len(failures)} leaf source authority failure(s): "
            + "; ".join(failures)
        )
    return len(registry), sum(len(leaf.consumers) for leaf in registry.values())


def verify_repo(
    repo_root: Path,
    *,
    required_lane_authorities: Iterable[tuple[int, str, str]] = (
        REQUIRED_LANE_AUTHORITIES
    ),
    # (dNN modules, lane authorities, grandfathered module line references,
    # grandfathered document line references)
) -> tuple[int, int, int, int]:
    repo_root = repo_root.resolve()
    world_path = repo_root / WORLD_FOLD
    world_source = _read_utf8(world_path)
    spans = function_spans(world_source)
    dispatch = domain_dispatch(world_source, spans)
    world_index = index_items(world_source)
    recorded_closures = load_closure_inventory(repo_root)

    wire_dir = repo_root / WIRE_SRC
    modules = sorted(
        (
            path
            for path in wire_dir.glob("d*.rs")
            if re.fullmatch(r"d[0-9]+\.rs", path.name)
        ),
        key=lambda path: int(path.stem[1:]),
    )
    if not modules:
        raise VerificationError(f"no dNN modules found under {wire_dir}")

    # An inventory key with no module is a membership record nothing
    # compares against anything.  Left alone it accumulates coverage
    # claims for domains that no longer exist -- the same shape as a
    # grandfathered document no scan reaches, which this gate refuses.
    module_domains = {str(int(path.stem[1:])) for path in modules}
    orphaned = sorted(set(recorded_closures) - module_domains)
    if orphaned:
        raise VerificationError(
            f"{CLOSURE_INVENTORY.name} records closure membership for "
            f"domain(s) {orphaned}, which have no dNN module; nothing "
            f"checks those entries, so they are a coverage claim no run "
            f"verifies. Re-stamp to drop them."
        )

    lane_markers: dict[tuple[int, str, str], tuple[Span, str]] = {}
    # Every module is checked on every run and each failure is retained. A
    # checker that stops at the FIRST drifted module reports one defect and
    # hides the rest: one stale d11 range masked 31 further stale ranges for
    # days, and because this check runs before Cargo, the whole central gate
    # was unreachable the entire time. Report the complete set so one fix pass
    # can close it.
    #
    # The marker checks below are deliberately NOT also run as a fail-fast
    # pre-pass: the same checks reporting twice would restore exactly the
    # stop-at-the-first-defect behaviour this accumulator exists to remove.
    module_failures: list[str] = []
    world_markers: dict[str, re.Match[str]] = {}
    for module in modules:
        domain = int(module.stem[1:])
        module_source = _read_utf8(module)
        try:
            marker = _only_match(
                WORLD_MARKER_RE, module_source, module.relative_to(repo_root).as_posix()
            )
            world_markers[module.name] = marker
            marker_domain = int(marker.group("domain"))
            if marker_domain != domain:
                raise VerificationError(
                    f"{module.name}: marker domain {marker_domain} != filename domain {domain}"
                )
            functions = marker.group("functions").split(",")
            dispatched = dispatch.get(domain)
            if dispatched is None:
                raise VerificationError(f"{module.name}: domain absent from fold_domain_data")
            if functions[0] != dispatched:
                raise VerificationError(
                    f"{module.name}: first authority {functions[0]} != dispatch {dispatched}"
                )
            try:
                function_ranges = [spans[name] for name in functions]
            except KeyError as error:
                raise VerificationError(
                    f"{module.name}: missing world-fold function {error.args[0]}"
                ) from error
            expected = Span(
                min(span.start for span in function_ranges),
                max(span.end for span in function_ranges),
            )
            declared = Span(int(marker.group("start")), int(marker.group("end")))
            if declared != expected:
                raise VerificationError(
                    f"{module.name}: declared lines {declared.start}-{declared.end}, "
                    f"actual {expected.start}-{expected.end}"
                )
            # Membership is checked BEFORE the envelope digest, because the two
            # can fail together and this is the one that says something the
            # other does not: the envelope reports that bytes moved, which a
            # re-stamp answers, while this reports that a function stopped being
            # covered, which a re-stamp must not.
            #
            # The envelope digest covers the cited functions where they stand
            # and nothing else.  It does NOT survive a helper being lifted out
            # of one: the envelope narrows to the residual, the re-stamp records
            # the narrower bytes, and the moved logic is outside every digest
            # while the gate still prints PASS.  The closure covers the cited
            # functions AND everything in this file they call, wherever it sits.
            members = callee_closure(
                world_index, authority_roots(world_index, functions)
            )
            recorded = recorded_closures.get(str(domain))
            if recorded is None:
                raise VerificationError(
                    f"{module.name}: no closure membership recorded in "
                    f"{CLOSURE_INVENTORY.name}"
                )
            if sorted(recorded) != sorted(members):
                dropped = sorted(set(recorded) - members)
                gained = sorted(members - set(recorded))
                raise VerificationError(
                    f"{module.name}: authority closure membership changed; "
                    f"no longer covered={dropped}, newly covered={gained}. "
                    f"A name leaving this list is coverage leaving the gate -- "
                    f"confirm the logic moved somewhere still cited before "
                    f"re-stamping."
                )
            expected_hash = normalized_span_hash(world_source, expected)
            if marker.group("sha256") != expected_hash:
                raise VerificationError(
                    f"{module.name}: authority digest changed for "
                    f"{expected.start}-{expected.end}"
                )
            expected_closure = closure_digest(world_index, members)
            if marker.group("closure") != expected_closure:
                raise VerificationError(
                    f"{module.name}: authority closure digest changed over "
                    f"{len(members)} function(s) reachable from "
                    f"{','.join(functions)}"
                )
            human_citation = gated_citation(marker)
            # Exactly once, not at least once.  The digest backs one citation; a
            # second copy of the same text is two more unchecked line numbers,
            # and the free-text ratchet would read them as gated.
            occurrences = module_source.count(human_citation)
            if occurrences != 1:
                raise VerificationError(
                    f"{module.name}: expected exactly one human authority citation "
                    f"{human_citation}, found {occurrences}"
                )
        except VerificationError as error:
            module_failures.append(str(error))

        # Lane markers are an independent authority (they pin a capture-lane
        # decoder, not the fold envelope), so they are still collected for a
        # module whose world-fold citation drifted. Skipping them would turn one
        # stale line range into a spurious "lane authority marker set drifted".
        for lane in LANE_MARKER_RE.finditer(module_source):
            lane_domain = int(lane.group("domain"))
            if lane_domain != domain:
                module_failures.append(
                    f"{module.name}: lane marker domain {lane_domain} != {domain}"
                )
                continue
            key = (lane_domain, lane.group("path"), lane.group("function"))
            if key in lane_markers:
                module_failures.append(f"duplicate lane authority marker: {key}")
                continue
            lane_markers[key] = (
                Span(int(lane.group("start")), int(lane.group("end"))),
                lane.group("sha256"),
            )

    required = set(required_lane_authorities)
    if set(lane_markers) != required:
        missing = sorted(required - set(lane_markers))
        unexpected = sorted(set(lane_markers) - required)
        module_failures.append(
            f"lane authority marker set drifted; missing={missing}, "
            f"unexpected={unexpected}"
        )

    for (domain, relative, function), (declared, digest) in lane_markers.items():
        try:
            authority_path = (repo_root / relative).resolve()
            try:
                authority_path.relative_to(repo_root)
            except ValueError as error:
                raise VerificationError(
                    f"d{domain}: lane authority escapes repository: {relative}"
                ) from error
            authority_source = _read_utf8(authority_path)
            authority_spans = function_spans(authority_source)
            actual = authority_spans.get(function)
            if actual is None:
                raise VerificationError(
                    f"d{domain}: {relative} has no function {function}"
                )
            if declared != actual:
                raise VerificationError(
                    f"d{domain}: lane authority {function} declared "
                    f"{declared.start}-{declared.end}, actual "
                    f"{actual.start}-{actual.end}"
                )
            actual_hash = normalized_span_hash(authority_source, actual)
            if digest != actual_hash:
                raise VerificationError(
                    f"d{domain}: lane authority digest changed for {relative}::{function}"
                )
        except VerificationError as error:
            module_failures.append(str(error))

    if module_failures:
        raise VerificationError(
            f"{len(module_failures)} citation authority failure(s): "
            + "; ".join(module_failures)
        )

    grandfathered = verify_prose_references(repo_root, modules, world_markers)
    documented = verify_document_references(
        repo_root, load_prose_allowance(repo_root)
    )

    return len(modules), len(lane_markers), grandfathered, documented


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[3],
    )
    args = parser.parse_args(argv)
    # BOTH halves run on every invocation, and both report.  Letting a
    # world-fold failure skip the leaf check would hide every stale leaf marker
    # behind one drifted line range -- the same stop-at-the-first-defect
    # behaviour `module_failures` exists to remove, one level up.
    failures: list[str] = []
    counts: tuple[int, int, int, int] | None = None
    covered: set[str] = set()
    try:
        counts = verify_repo(args.repo_root)
        # The inventory is only trustworthy to report from BECAUSE verify_repo
        # just proved it equals the computed closure, domain by domain.
        covered = {
            ident
            for members in load_closure_inventory(args.repo_root.resolve()).values()
            for ident in members
        }
    except VerificationError as error:
        failures.append(str(error))
    try:
        leaves, leaf_sites = verify_leaf_authorities(args.repo_root.resolve())
    except VerificationError as error:
        failures.append(str(error))
        leaves = leaf_sites = 0
    if failures or counts is None:
        print(
            "world-fold citation authority check: FAIL: " + "; ".join(failures),
            file=sys.stderr,
        )
        return 1
    module_count, lane_count, grandfathered, documented = counts
    print(
        "world-fold citation authority check: PASS "
        f"({module_count} dNN modules, {lane_count} lane authorities, "
        f"{len(covered)} world_fold.rs items inside an authority closure, "
        f"{leaves} leaf source authorities over {leaf_sites} consumer sites, "
        f"{grandfathered} grandfathered prose line references, "
        f"{documented} grandfathered document line references)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
