#!/usr/bin/env python3
"""Focused deterministic tests for verify_citation_integrity.py.

Each test builds a throwaway tree with the same shape the real one has -- a
crate source root, a document root, and an allowance -- and asks the gate one
question about it.  The point of every case is stated as the production change
that would make it fail if the gate stopped doing its job.
"""

from __future__ import annotations

import importlib.util
import io
import json
import marshal
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


def _load(name: str):
    script = Path(__file__).with_name(f"{name}.py")
    spec = importlib.util.spec_from_file_location(name, script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


VERIFY = _load("verify_citation_integrity")
RESTAMP = _load("restamp_citation_allowance")

# The fixture citations below are ASSEMBLED, never spelt out literally.  This
# file lives inside the gate's own scan roots, so a worked example written out in
# full would be a citation the gate then demanded be grandfathered -- and a test
# whose fixtures sit in the production allowance is a test that has quietly
# changed what it tests.  Written this way each one is inert text on disk and
# becomes a citation only inside the temporary tree.
DOT_RS = "." + "rs"
A = f"crate_a/src/lib{DOT_RS}"
B = f"crate_b/src/lib{DOT_RS}"
BARE = f"lib{DOT_RS}"
EXTERNAL = f"libc-0.2.189/src/psp{DOT_RS}"


class Fixture:
    """A miniature repository: one crate, one document tree, one allowance."""

    def __init__(self, root: Path) -> None:
        self.root = root
        (root / "extractor/native/crate_a/src").mkdir(parents=True)
        (root / "extractor/native/crate_b/src").mkdir(parents=True)
        (root / "extractor/documents").mkdir(parents=True)
        (root / "extractor/native/scripts").mkdir(parents=True)
        self.write_source(A, "pub fn only_in_a() {}\n" + "// filler\n" * 40)
        self.write_source(B, "pub fn only_in_b() {}\n" + "// filler\n" * 40)
        self.set_allowance({})

    def write_source(self, crate_relative: str, text: str) -> None:
        """Write under `extractor/native/`, which is one of the scan roots."""
        path = self.root / "extractor/native" / crate_relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")

    def write_document(self, name: str, text: str) -> None:
        (self.root / "extractor/documents" / name).write_text(
            text, encoding="utf-8", newline="\n"
        )

    def set_allowance(self, allowance: dict) -> None:
        path = self.root / VERIFY.ALLOWANCE
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps(allowance, indent=2) + "\n", encoding="utf-8", newline="\n"
        )

    def allowance(self) -> dict:
        return json.loads((self.root / VERIFY.ALLOWANCE).read_text(encoding="utf-8"))

    def restamp(self) -> None:
        RESTAMP.main(["--repo-root", str(self.root)])

    def verify(self, **options):
        """Run the gate over this tree.

        Caching is ON by default here, exactly as it is in production, so every
        case in this file that verifies twice is also a case about the cache
        being unable to hide what the second run should see.
        """
        return VERIFY.verify_repo(self.root, **options)

    def cache_path(self) -> Path:
        return self.root / VERIFY.CACHE

    def cache(self) -> dict:
        return json.loads(self.cache_path().read_text(encoding="utf-8"))

    def set_cache(self, document: dict) -> None:
        self.cache_path().parent.mkdir(parents=True, exist_ok=True)
        self.cache_path().write_text(
            json.dumps(document), encoding="utf-8", newline="\n"
        )


class CitationIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.fixture = Fixture(Path(self._tmp.name))

    # ---------------------------------------------------------------- rule L

    def test_a_new_line_citation_fails(self) -> None:
        """kills: deleting the `line_references` budget check in verify_repo.

        This is the whole ratchet.  Without it a lane may write a fresh line
        number into any comment and nothing objects, which is the state lane HK
        measured: 1,143 of them in the wire modules alone.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("new free-text line number", str(raised.exception))

    def test_a_grandfathered_line_citation_passes(self) -> None:
        """kills: making the ratchet an outright ban.

        The existing population is 3,263 citations.  A gate that refused them
        all could not be turned on, so the budget has to admit what is recorded.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        files, lines, items, unused = self.fixture.verify()
        self.assertEqual((lines, items, unused), (1, 0, 0))
        self.assertGreaterEqual(files, 1)

    def test_the_budget_is_per_citation_not_per_file(self) -> None:
        """kills: counting citations per file instead of per citation text.

        A file allowed one citation must not thereby be allowed a different one.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        self.fixture.write_document("DESIGN.md", f"see `{A}:3` and `{A}:9`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn(f"{A}:9", str(raised.exception))

    def test_a_second_occurrence_of_a_budgeted_citation_fails(self) -> None:
        """kills: treating the budget as a set membership test.

        The recorded number is a COUNT.  If one occurrence licensed any number
        of copies, a lane could paste a stale citation across a document freely.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        self.fixture.write_document("DESIGN.md", f"see `{A}:3` twice `{A}:3`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_deleting_a_citation_is_not_an_error(self) -> None:
        """kills: making an unused allowance entry fail.

        Removing a citation must never force an allowance edit in the same
        commit -- otherwise the cheapest way to satisfy the gate is to leave the
        stale citation where it is.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        self.fixture.write_document("DESIGN.md", "no citation here\n")
        _files, lines, _items, unused = self.fixture.verify()
        self.assertEqual((lines, unused), (0, 1))

    # ---------------------------------------------------------------- rule U

    def test_a_grandfathered_citation_past_the_end_of_its_file_fails(self) -> None:
        """kills: dropping the range check once a citation is grandfathered.

        This is the half that catches rot as it happens.  A recorded citation is
        licensed to EXIST; it is not licensed to stop landing.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:30`\n")
        self.fixture.restamp()
        self.fixture.write_source(A, "fn a() {}\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("past the end", str(raised.exception))

    def test_an_ambiguous_basename_fails(self) -> None:
        """kills: resolving a bare basename to its first match.

        252 of the citations lane HK counted named `lib` + `.rs` when 71 files
        carry that name.  Picking one would invent evidence; the gate refuses.
        """
        self.fixture.write_document("DESIGN.md", f"see `{BARE}:3`\n")
        self.fixture.restamp()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("ambiguous", str(raised.exception))

    def test_qualifying_an_ambiguous_basename_resolves_it(self) -> None:
        """kills: matching a written path anywhere but as a path SUFFIX.

        The prescribed fix for an ambiguous citation is to write enough of the
        path to name one file, so that has to be a fix the gate accepts.
        """
        self.fixture.write_document("DESIGN.md", f"see `{B}:3`\n")
        self.fixture.restamp()
        self.assertEqual(self.fixture.verify()[1], 1)

    def test_a_reasoned_unresolved_entry_admits_a_broken_citation(self) -> None:
        """kills: removing the `unresolved` escape hatch.

        Some citations cannot be repaired -- quoted panic output, and citations
        in files another lane is holding.  Without a reasoned exemption the only
        way to green the gate would be to falsify them.
        """
        self.fixture.write_document("DESIGN.md", f"see `{BARE}:3`\n")
        self.fixture.restamp()
        allowance = self.fixture.allowance()
        allowance["unresolved"] = {
            "extractor/documents/DESIGN.md": {f"{BARE}:3": "quoted output"}
        }
        self.fixture.set_allowance(allowance)
        self.assertEqual(self.fixture.verify()[1], 1)

    def test_an_unresolved_entry_without_a_line_reference_fails(self) -> None:
        """kills: letting `unresolved` stand alone.

        An exemption for a citation rule L would reject anyway permits nothing;
        left alone those accumulate and get quoted as precedent.
        """
        self.fixture.set_allowance(
            {"unresolved": {"extractor/documents/DESIGN.md": {f"{BARE}:3": "why"}}}
        )
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("permit nothing", str(raised.exception))

    # ---------------------------------------------------------------- rule I

    def test_an_item_citation_naming_no_definition_fails(self) -> None:
        """kills: accepting an item citation without checking the item exists.

        The item form is the migration target for every line citation in the
        tree.  If it were unchecked the conversion would move citations from one
        unchecked class into another.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::gone`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("defines", str(raised.exception))

    def test_an_item_citation_is_resolved_by_which_file_defines_it(self) -> None:
        """kills: treating an item citation's basename ambiguity as fatal.

        A bare-basename item citation is unambiguous even though 71 files share
        the name, because one of them defines the item.  That is the property
        that makes the item form worth converting to.
        """
        self.fixture.write_document("DESIGN.md", f"see `{BARE}::only_in_b`\n")
        self.assertEqual(self.fixture.verify()[2], 1)

    def test_an_item_defined_in_two_candidates_fails(self) -> None:
        """kills: taking the first defining candidate.

        Two files defining the name is a real ambiguity and the citation has to
        say which; silently choosing one is how a reader is misdirected.
        """
        self.fixture.write_source(B, "pub fn only_in_a() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{BARE}::only_in_a`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("ambiguous", str(raised.exception))

    def test_a_mention_is_not_a_definition(self) -> None:
        """kills: resolving an item citation by searching for the bare name.

        `bin/stream_fold` MENTIONS `fold_compact_stream_file` in its module doc
        and does not define it.  A mention-based probe would make every such
        pair ambiguous and the gate unusable.
        """
        self.fixture.write_source(
            B, "//! calls only_in_a() from crate_a\nfn other() { only_in_a(); }\n"
        )
        self.fixture.write_document("DESIGN.md", f"see `{BARE}::only_in_a`\n")
        self.assertEqual(self.fixture.verify()[2], 1)

    def test_an_item_path_may_name_a_member_of_a_defined_item(self) -> None:
        """kills: requiring the LAST segment to be a declared item.

        `Row::VERSION` names an associated constant and only `Row` carries a
        keyword; rejecting those would push authors back to line numbers.
        """
        self.fixture.write_source(
            A, "pub struct Row;\nimpl Row {\n    pub const VERSION: u32 = 1;\n}\n"
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::VERSION`\n")
        self.assertEqual(self.fixture.verify()[2], 1)

    # --------------------------------------------------------------- scoping

    def test_evidence_directories_are_not_scanned(self) -> None:
        """kills: scanning `runs/`.

        `extractor/native/runs/` is append-only by AGENTS.md.  A gate that
        demanded its citations resolve would be demanding that completed run
        records be rewritten.
        """
        self.fixture.write_source("runs/old_lane/NOTES.md", f"see `{BARE}:99999`\n")
        self.assertEqual(self.fixture.verify()[1], 0)

    def test_an_external_source_is_skipped(self) -> None:
        """kills: dropping `external_sources`.

        A vendored dependency's source is a real file that is not checked in;
        demanding it resolve would demand the vendor tree be added here.
        """
        self.fixture.write_document("DESIGN.md", f"see `{EXTERNAL}:505`\n")
        self.fixture.set_allowance({"external_sources": [EXTERNAL]})
        self.assertEqual(self.fixture.verify()[1], 0)

    def test_a_missing_allowance_is_an_error_not_an_empty_one(self) -> None:
        """kills: treating an absent allowance as grandfathering nothing quietly.

        An empty allowance fails every citation, which invites deleting the
        file; a deleted file that grandfathers everything is the silent-pass
        shape this repository has been burned by before.
        """
        (self.fixture.root / VERIFY.ALLOWANCE).unlink()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("is missing", str(raised.exception))

    def test_an_item_citation_is_not_also_read_as_a_line_citation(self) -> None:
        """kills: scanning line citations before item citations.

        An item citation must not additionally register as a malformed line
        reference; if it did, converting a citation to the item form would
        create a new rule-L failure and the conversion would be unavailable.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        _files, lines, items, _unused = self.fixture.verify()
        self.assertEqual((lines, items), (0, 1))

    # ------------------------------------------------ Codex review round 1

    def test_a_range_endpoint_is_part_of_the_budget_key(self) -> None:
        """kills: dropping the `-<end>` group from the citation's canonical form.

        A range citation keyed on its start alone meant the endpoint could be
        changed to any number without changing a budget key, so nothing noticed.
        (Codex round 1, finding 3.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:2-9`\n")
        self.fixture.restamp()
        self.assertEqual(self.fixture.verify()[1], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}:2-11`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn(f"{A}:2-11", str(raised.exception))

    def test_a_range_endpoint_past_the_end_of_the_file_fails(self) -> None:
        """kills: range-checking only the START of a range citation.

        A range whose start lands and whose end does not is a citation that runs
        off the file, and it read as resolvable. (Codex round 1, finding 3.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:2-999`\n")
        self.fixture.restamp()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("past the end", str(raised.exception))

    def test_a_backwards_range_fails(self) -> None:
        """kills: accepting a range whose end precedes its start.

        Both endpoints are inside the file, so a per-endpoint check passes it
        while the citation denotes nothing.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:9-2`\n")
        self.fixture.restamp()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("backwards range", str(raised.exception))

    def test_a_member_that_does_not_exist_fails(self) -> None:
        """kills: accepting any member of a type the file declares.

        `Row::DOES_NOT_EXIST` passed because `Row` was declared, which made the
        item form checkable in name only. (Codex round 1, finding 4.)
        """
        self.fixture.write_source(A, "pub struct Row {\n    pub known: u32,\n}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::DOES_NOT_EXIST`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("defines", str(raised.exception))

    def test_an_enum_variant_still_resolves(self) -> None:
        """kills: over-tightening finding 4 into "the last segment must be a
        declared item".

        A variant carries no declaration keyword; rejecting those would push
        authors back to line numbers, which is the thing being migrated away
        from.
        """
        self.fixture.write_source(
            A, "pub enum Kind {\n    Alpha,\n    Beta = 2,\n}\n"
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Kind::Beta`\n")
        self.assertEqual(self.fixture.verify()[2], 1)

    def test_a_repository_root_document_is_scanned(self) -> None:
        """kills: enumerating scan roots instead of walking the repository.

        `ACCEPTANCE.md` sits at the repository root and carried an ambiguous
        `lib.rs:<line>` that an enumerated six-root scan never saw. (Codex round
        1, finding 5.)
        """
        (self.fixture.root / "ROOT_DOC.md").write_text(
            f"see `{BARE}:3`\n", encoding="utf-8", newline="\n"
        )
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("ROOT_DOC.md", str(raised.exception))

    def test_every_text_file_is_scanned_whatever_its_extension(self) -> None:
        """kills: reintroducing a suffix whitelist.

        The whitelist leaked five times -- `.sh`, `.json`, `.txt`, `.toml`,
        `.yml` -- and a case-sensitive one still missed `.C`, `seh_read.c`,
        `StringXref.java` and `f1_stress.hlsl`. (Codex rounds 1, 6, 8, 9.) The
        rule is now "read it unless it is binary", asserted over the extensions
        a whitelist would plausibly have missed.
        """
        for suffix in (".rs", ".md", ".sh", ".json", ".toml", ".yml",
                       ".c", ".java", ".hlsl", ".C", ".MD", "", ".unheard_of"):
            with self.subTest(suffix=suffix):
                probe = self.fixture.root / f"probe{suffix}"
                probe.write_text(f"see `{A}:3`\n", encoding="utf-8", newline="\n")
                try:
                    with self.assertRaises(VERIFY.VerificationError) as raised:
                        self.fixture.verify()
                    self.assertIn(f"probe{suffix}", str(raised.exception))
                finally:
                    probe.unlink()

    def test_a_binary_file_is_not_scanned(self) -> None:
        """kills: reading every file as text.

        A NUL byte means bytes nobody wrote comments into, and the game-asset
        trees are full of them. Decided by a sniff, not by extension, so a
        binary with an unlisted extension is still skipped.
        """
        probe = self.fixture.root / "asset.unheard_of"
        probe.write_bytes(b"\x00\x01binary see `" + A.encode() + b":3`\n")
        self.assertNotIn("asset.unheard_of", VERIFY.scan_files(self.fixture.root))
        self.assertEqual(self.fixture.verify()[1], 0)

    # ------------------------------------------------ Codex review round 2

    def test_a_split_range_endpoint_is_part_of_the_budget_key(self) -> None:
        """kills: reading only the one-token `a-b` range form.

        This repository writes most ranges with the endpoint in its own
        backticks, across 102 files. An endpoint outside the citation is one no
        budget keys on and nothing checks. (Codex round 2, finding 1.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:2`-`:9`\n")
        self.fixture.restamp()
        self.assertEqual(self.fixture.allowance()["line_references"], {
            "extractor/documents/DESIGN.md": {f"{A}:2-9": 1}
        })
        self.fixture.write_document("DESIGN.md", f"see `{A}:2`-`:11`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn(f"{A}:2-11", str(raised.exception))

    def test_a_split_range_endpoint_past_the_end_fails(self) -> None:
        """kills: folding the split endpoint into the key without checking it."""
        self.fixture.write_document("DESIGN.md", f"see `{A}:2`–`:999`\n")
        self.fixture.restamp()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("past the end", str(raised.exception))

    def test_an_item_path_whose_owner_is_absent_fails(self) -> None:
        """kills: resolving an item path by its last segment alone.

        `MissingType::existing` passed because `existing` was declared somewhere
        in the file. (Codex round 2, finding 2.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::MissingType::only_in_a`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("defines", str(raised.exception))

    def test_a_local_assignment_is_not_a_member(self) -> None:
        """kills: admitting a bare `=` line as a member introducer.

        `phantom = 0;` in a function body made `Row::phantom` resolve against a
        local variable. (Codex round 2, finding 2.)
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\nfn f() {\n    let mut phantom = 1;\n    phantom = 0;\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::phantom`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_line_zero_fails(self) -> None:
        """kills: bounding the cited line from above only. Files start at 1."""
        self.fixture.write_document("DESIGN.md", f"see `{A}:0`\n")
        self.fixture.restamp()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("files start at line 1", str(raised.exception))

    def test_one_line_past_the_end_fails(self) -> None:
        """kills: counting lines as `count("\\n") + 1`.

        That invents a line past the end of every newline-terminated file, so a
        citation one line too far resolved. (Codex round 2, finding 3.)
        """
        self.fixture.write_source(A, "pub fn only_in_a() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}:2`\n")
        self.fixture.restamp()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("past the end", str(raised.exception))

    def test_the_allowance_itself_is_not_scanned(self) -> None:
        """kills: scanning `.json` without excluding the allowance.

        Its KEYS are citations; scanning it would read every grandfathered
        citation back as a fresh citation OF the allowance file.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        self.assertNotIn(
            VERIFY.ALLOWANCE.as_posix(), VERIFY.scan_files(self.fixture.root)
        )
        self.assertEqual(self.fixture.verify()[1], 1)

    def test_an_empty_reason_is_not_a_reason(self) -> None:
        """kills: validating an exemption's reason with `isinstance(_, str)`.

        `""` bought the same exemption as a written justification. (Codex round
        2, finding 5.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{BARE}:3`\n")
        self.fixture.restamp()
        allowance = self.fixture.allowance()
        allowance["unresolved"] = {"extractor/documents/DESIGN.md": {f"{BARE}:3": "  "}}
        self.fixture.set_allowance(allowance)
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("empty reason", str(raised.exception))

    # ------------------------------------------------ Codex review round 3

    def test_a_member_of_another_type_fails(self) -> None:
        """kills: resolving an item path by independent file-wide presence.

        `Row::ONLY_OTHER` passed while `ONLY_OTHER` belonged to `Other`. An item
        path names a route, and a route that does not lead anywhere misdirects
        exactly like a drifted line number. (Codex round 3, finding 1.)
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\nimpl Row {\n    pub const MINE: u32 = 1;\n}\n"
            "pub struct Other;\nimpl Other {\n    pub const ONLY_OTHER: u32 = 2;\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::MINE`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::ONLY_OTHER`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("defines", str(raised.exception))

    def test_a_call_in_a_function_body_is_not_a_member(self) -> None:
        """kills: admitting `NAME(` anywhere as a member introducer.

        `LOCAL_ONLY();` in a function body made `Row::LOCAL_ONLY` resolve.
        (Codex round 3, finding 1.)
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\nfn helper() {}\nfn f() {\n    helper();\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::helper`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_impl_blocks_for_one_type_merge(self) -> None:
        """kills: keeping only the last `impl` block's members for a type.

        Rust reads several `impl` blocks as one surface and so must this, or a
        correct citation of a method in the first block would be rejected.
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\nimpl Row {\n    pub fn first(&self) {}\n}\n"
            "impl Row {\n    pub fn second(&self) {}\n}\n",
        )
        self.fixture.write_document(
            "DESIGN.md", f"see `{A}::Row::first` and `{A}::Row::second`\n"
        )
        self.assertEqual(self.fixture.verify()[2], 2)

    def test_a_split_range_may_wrap_onto_the_next_comment_line(self) -> None:
        """kills: allowing only horizontal whitespace inside a split range.

        A range written at the end of a `///` line continues on the next one,
        and reading only spaces lost the endpoint entirely -- it was neither
        keyed nor bounds-checked. (Codex round 9, finding 2.)
        """
        self.fixture.write_source(A, "pub fn a() {}\n" + "// filler\n" * 8)
        self.fixture.write_source(
            "crate_a/src/citing.rs",
            f"/// see `{A}:1`-\n/// `:999`\npub fn b() {{}}\n",
        )
        self.fixture.restamp()
        allowance = self.fixture.allowance()["line_references"]
        self.assertEqual(
            allowance["extractor/native/crate_a/src/citing.rs"], {f"{A}:1-999": 1}
        )
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("past the end", str(raised.exception))

    # ------------------------------------------------ Codex review round 4

    def test_a_function_local_item_is_not_a_member(self) -> None:
        """kills: collecting an owner's whole body regardless of brace depth.

        A `fn` declared inside a method is local to that method. Collecting it
        as a member made `Row::LOCAL_ONLY` resolve against something no reader
        could reach by that path. (Codex round 4, finding 1.)
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\nimpl Row {\n    pub fn host() {\n"
            "        fn nested_only() {}\n        nested_only();\n    }\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::host`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::nested_only`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_raw_identifier_resolves_and_a_missing_one_does_not(self) -> None:
        """kills: parsing an item segment without its `r#` prefix.

        A citation of a raw identifier parsed as just its `r` prefix and resolved
        against a
        function called `r`. (Codex round 4, finding 2.) Both halves are
        asserted: the real raw identifier must resolve, and the missing one must
        not silently truncate to a prefix that does.
        """
        self.fixture.write_source(A, "pub fn r() {}\npub fn r#match() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::r#match`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::r#missing`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("r#missing", str(raised.exception))

    # ------------------------------------------------ Codex review round 5

    def test_same_named_types_in_two_modules_do_not_share_members(self) -> None:
        """kills: keying the scope index by bare owner name.

        Two modules may each define a `Row`. Merged by name, `alpha::Row` lends
        its members to `beta::Row` and a route that does not exist resolves.
        (Codex round 5, finding 1.) Both real routes must still pass, or the fix
        would have bought correctness by rejecting correct citations.
        """
        self.fixture.write_source(
            A,
            "pub mod alpha {\n    pub struct Row;\n"
            "    impl Row {\n        pub fn alpha_only() {}\n    }\n}\n"
            "pub mod beta {\n    pub struct Row;\n"
            "    impl Row {\n        pub fn beta_only() {}\n    }\n}\n",
        )
        self.fixture.write_document(
            "DESIGN.md", f"see `{A}::alpha::Row::alpha_only` and `{A}::beta::Row::beta_only`\n"
        )
        self.assertEqual(self.fixture.verify()[2], 2)
        self.fixture.write_document("DESIGN.md", f"see `{A}::alpha::Row::beta_only`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_trailing_path_separator_is_reported(self) -> None:
        """kills: letting an item pattern stop at a resolvable prefix, and
        kills: saying nothing about the malformed citation that remains.

        `<file>::okay::` was first read as `<file>::okay` and accepted (round 5),
        then matched nothing at all and passed in silence (round 6). Silence
        about a malformed citation is the same failure as a wrong one.
        """
        self.fixture.write_source(A, "pub fn okay() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::okay::`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("not a complete item citation", str(raised.exception))

    def test_a_non_ascii_identifier_is_not_truncated(self) -> None:
        """kills: an ASCII-only identifier class in the item pattern.

        Rust identifiers may be non-ASCII. A citation of one truncated to its
        ASCII head and resolved against a different, shorter name. (Codex round
        5, finding 2.)
        """
        self.fixture.write_source(A, "pub fn f() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::föo`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("föo", str(raised.exception))

    # ------------------------------------------------ Codex review round 6

    def test_a_function_body_is_a_route_barrier(self) -> None:
        """kills: treating any lexically enclosed scope as a route child.

        A `struct` declared inside a method is local to that method. Attached as
        a child of the type, it made `Row::Inner::field` -- a route nothing can
        follow -- resolve. (Codex round 6, finding 2.)
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\nimpl Row {\n    pub fn host() {\n"
            "        struct Inner {\n            field: u32,\n        }\n"
            "        let _ = Inner { field: 0 };\n    }\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::Inner::field`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_function_local_definition_satisfies_nothing(self) -> None:
        """kills: building the definition index from a raw file-wide scan.

        A `fn` inside another function is not addressable from outside it, so a
        citation naming it names nothing. (Codex round 6, finding 3.)
        """
        self.fixture.write_source(
            A, "pub fn host() {\n    fn local_only() {}\n    local_only();\n}\n"
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::host`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::local_only`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_definition_inside_a_comment_satisfies_nothing(self) -> None:
        """kills: scanning raw text for declarations.

        Rust-looking text inside a block comment declares nothing. (Codex round
        6, finding 3.)
        """
        self.fixture.write_source(A, "/*\nfn ghost() {}\n*/\npub fn real() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::ghost`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_definition_inside_a_string_literal_satisfies_nothing(self) -> None:
        """kills: scanning raw text for declarations.

        A raw string containing Rust source declares nothing. (Codex round 6,
        finding 3.)
        """
        self.fixture.write_source(
            A, 'pub const TEXT: &str = r#"fn ghost() {}"#;\npub fn real() {}\n'
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::ghost`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_the_fast_sanitizer_agrees_with_the_reviewed_one(self) -> None:
        """kills: this gate's sanitizer drifting from the world_fold gate's.

        This gate carries its own regex sanitizer because the reviewed
        character-at-a-time one costs three minutes over the Rust it indexes.
        Two implementations of one contract need a link, and this is it: both
        run over the shapes that make a scanner wrong, and must agree
        byte-for-byte.
        """
        reviewed = _load("verify_world_fold_citations")
        cases = [
            "// fn ghost() {}\nfn real() {}\n",
            "/* fn ghost() {} */\nfn real() {}\n",
            'let s = "a \\" b // not a comment";\n',
            'let s = "wrapped \\\n    continuation";\n',
            'let r = r#"fn ghost() {} "quoted" "#;\n',
            "let c = '}';\nlet d = b'\\n';\n",
            "fn f<'a>(x: &'a str) -> &'a str { x }\n",
            "let b = b\"bytes { } \";\n",
        ]
        for index, case in enumerate(cases):
            with self.subTest(case=index):
                self.assertEqual(VERIFY._sanitize_rust(case), reviewed.sanitize_rust(case))

    # ------------------------------------------------ Codex review round 7

    def test_an_unfinished_raw_identifier_is_reported(self) -> None:
        """kills: malformed-item detection that stops at a trailing `::`.

        `<file>::okay::r#` stops on a raw-identifier prefix and is as unfinished
        as one that stops on the separator, but matched neither pattern. (Codex
        round 7, finding 1.)
        """
        self.fixture.write_source(A, "pub fn okay() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::okay::r#`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("not a complete item citation", str(raised.exception))

    def test_a_field_does_not_resolve_without_its_owner(self) -> None:
        """kills: flattening members into the declaration index.

        `<file>::field` resolved because some struct in the file happened to
        have a field of that name. A member is reachable only through its owner.
        (Codex round 7, finding 2.)
        """
        self.fixture.write_source(A, "pub struct Row {\n    pub field: u32,\n}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::field`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::field`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_an_impl_header_path_declares_only_its_subject(self) -> None:
        """kills: treating the first token after `impl` as a declaration.

        `impl std::fmt::Display for Row` made `<file>::std` resolve. The subject
        is `Row`; `std` and `fmt` are path segments of a trait that lives
        elsewhere. (Codex round 7, finding 3.)
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\nimpl std::fmt::Display for Row {\n"
            "    fn fmt(&self) {}\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::fmt`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::std`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_type_implemented_here_but_declared_elsewhere_resolves(self) -> None:
        """kills: over-tightening finding 3 into "only `struct` declares a type".

        Dropping `impl` from the declaration pattern would otherwise make
        `<file>::ForeignType::method` unresolvable in the file that implements
        it, which is where a reader would look.
        """
        self.fixture.write_source(
            A, "impl ForeignType {\n    pub fn method(&self) {}\n}\n"
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::ForeignType::method`\n")
        self.assertEqual(self.fixture.verify()[2], 1)

    # ------------------------------------------------ Codex review round 8

    def test_an_item_citation_with_no_segments_at_all_is_reported(self) -> None:
        """kills: requiring at least one completed segment before reporting.

        `<file>::` and `<file>::r#` name nothing whatever, and both were ignored
        because the malformed pattern demanded a completed segment first. (Codex
        round 8, finding 1.)
        """
        self.fixture.write_source(A, "pub fn okay() {}\n")
        for tail in ("::", "::r#"):
            with self.subTest(tail=tail):
                self.fixture.write_document("DESIGN.md", f"see `{A}{tail}`\n")
                with self.assertRaises(VERIFY.VerificationError) as raised:
                    self.fixture.verify()
                self.assertIn("not a complete item citation", str(raised.exception))

    def test_an_anonymous_const_declares_no_name(self) -> None:
        """kills: indexing `_` as a declaration.

        `const _: u32 = 0;` is an anonymous item: `_` is the absence of a name,
        so `<file>::_` names nothing. (Codex round 8, finding 3.)
        """
        self.fixture.write_source(A, "const _: u32 = 0;\npub fn real() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::_`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    # ------------------------------------------------ Codex review round 10

    def test_a_text_file_with_a_binary_suffix_is_still_scanned(self) -> None:
        """kills: returning False from `is_text_file` on the suffix alone.

        The suffix list is a hint; the bytes are the decision. A UTF-8 file
        named `probe.png` was skipped unread and its citation never checked.
        (Codex round 10, finding 1.)
        """
        probe = self.fixture.root / "probe.png"
        probe.write_text(f"see `{A}:99999`\n", encoding="utf-8", newline="\n")
        self.assertIn("probe.png", VERIFY.scan_files(self.fixture.root))
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("probe.png", str(raised.exception))

    def test_a_struct_literal_in_an_impl_is_not_an_associated_item(self) -> None:
        """kills: collecting MEMBER_RE names from a body that declares no fields.

        `impl Row { const VALUE: Other = Other { .. }; }` made `Row::Other`
        resolve — `Other` is a type being constructed, not an associated item.
        The flag is per BODY, not per route, because `struct Row {..}` and
        `impl Row {..}` share the route. (Codex round 10, finding 2.)
        """
        self.fixture.write_source(
            A,
            "pub struct Row;\npub struct Other {\n    pub x: u32,\n}\n"
            "impl Row {\n    pub const VALUE: Other = Other {\n        x: 1,\n    };\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::VALUE`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::Other`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    # ------------------------------------------------ Codex review round 11

    def test_a_wrapped_field_type_is_not_a_member(self) -> None:
        """kills: reading a struct body line by line.

        `pub actual:` / `    Other,` put the field's TYPE on its own
        comma-terminated line, and a line pattern read it as a second entry, so
        `Row::Other` resolved. Entries are separated by depth-one commas, not by
        newlines. (Codex round 11, finding 1.)
        """
        self.fixture.write_source(
            A,
            "pub struct Other;\npub struct Row {\n    pub actual:\n        Other,\n}\n",
        )
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::actual`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::Other`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_enum_variants_of_every_shape_are_members(self) -> None:
        """kills: an entry parser that only understands bare variants.

        Tuple and struct variants, discriminants and attributes are all normal;
        rejecting them would push authors back to line numbers.
        """
        self.fixture.write_source(
            A,
            "pub enum Kind {\n    Alpha,\n    Beta = 2,\n    Gamma(u32),\n"
            "    #[non_exhaustive]\n    Delta { x: u32 },\n}\n",
        )
        for variant in ("Alpha", "Beta", "Gamma", "Delta"):
            with self.subTest(variant=variant):
                self.fixture.write_document(
                    "DESIGN.md", f"see `{A}::Kind::{variant}`\n"
                )
                self.assertEqual(self.fixture.verify()[2], 1)

    def test_a_non_ascii_filename_is_cited_like_any_other(self) -> None:
        """kills: an ASCII-only path/filename class.

        A citation of a non-ASCII filename was not seen AT ALL -- neither
        checked nor reported. (Codex round 11, finding 2.)
        """
        # Assembled, not spelt out, for the reason given at the top of this file.
        unicode_file = f"crate_a/src/\u00e9{DOT_RS}"
        self.fixture.write_source(unicode_file, "pub fn only_here() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{unicode_file}::missing`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn(f"\u00e9{DOT_RS}", str(raised.exception))
        self.fixture.write_document("DESIGN.md", f"see `{unicode_file}::only_here`\n")
        self.assertEqual(self.fixture.verify()[2], 1)

    # ------------------------------------------------ Codex review round 12

    def test_a_generic_argument_is_not_a_member(self) -> None:
        """kills: splitting entries on every comma at body depth.

        `pub actual: Pair<Other, Ghost>,` is ONE field, and splitting on its
        inner comma invented a member called `Ghost`. (Codex round 12, finding
        2.)
        """
        self.fixture.write_source(
            A,
            "pub struct Other;\npub struct Ghost;\npub struct Row {\n"
            "    pub actual: Pair<Other,\n        Ghost>,\n    pub second: u32,\n}\n",
        )
        for real in ("actual", "second"):
            with self.subTest(member=real):
                self.fixture.write_document("DESIGN.md", f"see `{A}::Row::{real}`\n")
                self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::Row::Ghost`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_decomposed_item_name_is_not_truncated(self) -> None:
        """kills: an identifier class that stops at a combining mark.

        Rust identifiers follow Unicode XID rules and compare after NFC, so a
        decomposed citation names the composed item. Truncating at the mark made
        it resolve against a DIFFERENT, shorter function. (Codex round 12,
        finding 3.)
        """
        composed = "caf\u00e9"
        decomposed = "cafe\u0301"
        self.fixture.write_source(A, f"pub fn cafe() {{}}\npub fn {composed}() {{}}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::{decomposed}`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::cafe\u0301x`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    def test_a_decomposed_filename_is_consumed_whole(self) -> None:
        """kills: letting a match restart inside a filename component.

        `cafe` + U+0301 + `good.rs` was matched as `good.rs`, so the citation
        resolved against a different file entirely. (Codex round 12, finding 4.)
        """
        odd = f"crate_a/src/cafe\u0301good{DOT_RS}"
        self.fixture.write_source(odd, "pub fn only_here() {}\n")
        self.fixture.write_source("crate_a/src/good" + DOT_RS, "pub fn missing() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{odd}::missing`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()
        self.fixture.write_document("DESIGN.md", f"see `{odd}::only_here`\n")
        self.assertEqual(self.fixture.verify()[2], 1)

    # ------------------------------------------------ Codex review round 13

    def test_an_xid_mark_outside_the_listed_blocks_is_not_truncated(self) -> None:
        """kills: hand-enumerated combining-mark ranges in the identifier class.

        Round 12's repair listed six mark BLOCKS, and the XID property is wider
        than any six blocks: U+0483 continues an identifier and is in none of
        them, so `<file>::a` + U+0483 was still truncated to `a` and still
        resolved against the wrong, shorter function. (Codex round 13, finding
        1.) The classes are derived from Python's own XID tables now, so the
        test that matters is the one no hand-written list can pass: EVERY XID
        continuation character is part of the name.
        """
        self.fixture.write_source(A, "pub fn a() {}\n")
        # One mark from each of five blocks the hand-written list did not name,
        # every one of them a valid XID continuation character.
        for mark in ("\u0483", "\u0eb4", "\u1cd0", "\u0f39", "\U000e0100"):
            with self.subTest(mark=hex(ord(mark))):
                self.fixture.write_document("DESIGN.md", f"see `{A}::a{mark}`\n")
                with self.assertRaises(VERIFY.VerificationError):
                    self.fixture.verify()

    def test_a_decomposed_declaration_is_indexed_whole(self) -> None:
        """kills: a separate, narrower identifier class on the DECLARATION side.

        `DEFINITION_RE`, `ENTRY_NAME` and `SCOPE_HEADER` had their own
        `[^\\W\\d]\\w*`, so `pub fn cafe` + U+0301 was INDEXED as `cafe` -- and a
        citation of the shorter name, which no item has, then resolved against
        it. The truncation defect running backwards. (Codex round 13, finding 2.)
        """
        composed = "caf\u00e9"
        decomposed = "cafe\u0301"
        self.fixture.write_source(A, f"pub fn {decomposed}() {{}}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::cafe`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()
        # The real name still resolves, written either way.
        for spelling in (composed, decomposed):
            with self.subTest(spelling=repr(spelling)):
                self.fixture.write_document("DESIGN.md", f"see `{A}::{spelling}`\n")
                self.assertEqual(self.fixture.verify()[2], 1)
        # A scope header and a member entry are read with the same class.
        self.fixture.write_source(
            A, "pub struct Ro\u0301w {\n    pub fie\u0301ld: u32,\n}\n"
        )
        for shorter in ("Ro", "Ro\u0301w::fie"):
            with self.subTest(citation=repr(shorter)):
                self.fixture.write_document("DESIGN.md", f"see `{A}::{shorter}`\n")
                with self.assertRaises(VERIFY.VerificationError):
                    self.fixture.verify()
        self.fixture.write_document(
            "DESIGN.md", f"see `{A}::Ro\u0301w::fie\u0301ld`\n"
        )
        self.assertEqual(self.fixture.verify()[2], 1)

    def test_a_filename_containing_a_space_is_consumed_whole(self) -> None:
        """kills: letting a match restart after a space inside a filename.

        A space is legal in a filename and cannot be in the path class, so
        `crate_a/src/my file.rs` matched only its tail `file.rs`, which then
        resolved against an unrelated `file.rs` in another crate. (Codex round
        13, finding 3.) The match is extended over spaces only as far as a
        spelling that names an indexed file, so an ordinary sentence -- where no
        such file exists -- is unaffected.
        """
        spaced = f"crate_a/src/my file{DOT_RS}"
        self.fixture.write_source(spaced, "pub fn only_here() {}\n")
        self.fixture.write_source(f"crate_b/src/file{DOT_RS}", "pub fn missing() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{spaced}::missing`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("my file" + DOT_RS, str(raised.exception))
        # The whole component resolves when it is what the author meant.
        self.fixture.write_document("DESIGN.md", f"see `{spaced}::only_here`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        # A space in a DIRECTORY component is completed the same way.
        deep = f"crate_a/my dir/deep{DOT_RS}"
        self.fixture.write_source(deep, "pub fn in_the_deep() {}\n")
        self.fixture.write_source(f"crate_b/src/deep{DOT_RS}", "pub fn elsewhere() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{deep}::elsewhere`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

    # ------------------------------------------------ Codex review round 14

    def test_a_spaced_filename_that_names_nothing_is_reported(self) -> None:
        """kills: resolving a spaced path by existence alone.

        Extending only as far as an INDEXED file left the case the gate is for:
        a citation of a spaced file that does not exist -- a typo, or a file
        since deleted -- extended to nothing, fell back to its tail, and the
        tail resolved against an unrelated file of that name. (Codex round 14,
        finding 1.) Inside backticks a path written whole is read whole, so it
        is reported.
        """
        self.fixture.write_source(f"crate_b/src/file{DOT_RS}", "pub fn missing() {}\n")
        gone = f"crate_a/src/missing file{DOT_RS}"
        for citation in (f"{gone}::missing", f"{gone}:3"):
            with self.subTest(citation=citation):
                self.fixture.write_document("DESIGN.md", f"see `{citation}`\n")
                with self.assertRaises(VERIFY.VerificationError) as raised:
                    self.fixture.verify()
                self.assertIn("missing file" + DOT_RS, str(raised.exception))

    def test_a_backticked_span_without_a_separator_is_not_widened(self) -> None:
        """kills: reading every backticked prefix as part of the path.

        `see <file>::<item>` inside backticks is prose with a citation in it,
        not a citation of a file called `see <file>`: a separator-less prefix
        drops no path, so widening it would turn ordinary prose into a failure.
        Only a run carrying a path separator is a path the author wrote whole.
        (Codex round 14, finding 1.)
        """
        self.fixture.write_document("DESIGN.md", f"`see {A}::only_in_a` holds\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        # And the citation is still keyed as written, not as the widened span.
        self.fixture.write_document("DESIGN.md", f"`see {A}:3` holds\n")
        self.fixture.restamp()
        self.assertEqual(
            list(
                self.fixture.allowance()["line_references"][
                    "extractor/documents/DESIGN.md"
                ]
            ),
            [f"{A}:3"],
        )

    def test_extending_over_a_space_never_widens_an_ordinary_citation(self) -> None:
        """kills: admitting spaces into the path class outright.

        A path class with a space in it reads `see <file>::<item>` as a
        citation of a file called `see <file>`, which names nothing -- turning
        every prose citation in the repository into a failure. The extension is
        allowed only where the wider spelling names an INDEXED file, so with no
        such file the narrow match stands. (Codex round 13, finding 3.)
        """
        self.fixture.write_document(
            "DESIGN.md",
            f"the note in {A}::only_in_a and the range in {A}:3 both hold\n",
        )
        self.fixture.restamp()
        _files, lines, items, unused = self.fixture.verify()
        self.assertEqual((lines, items, unused), (1, 1, 0))

    def test_the_restamper_keys_a_spaced_filename_as_the_gate_does(self) -> None:
        """kills: extending the path in the gate but not in the restamper.

        The two must spell one citation the same way or the allowance the
        restamper writes grandfathers a citation nobody wrote, and the gate
        rejects the file it just generated. (Codex round 13, finding 3.)
        """
        spaced = f"crate_a/src/my file{DOT_RS}"
        self.fixture.write_source(spaced, "pub fn only_here() {}\n" + "// f\n" * 20)
        self.fixture.write_document("DESIGN.md", f"see `{spaced}:3`\n")
        self.fixture.restamp()
        self.assertEqual(
            list(self.fixture.allowance()["line_references"]["extractor/documents/DESIGN.md"]),
            [f"{spaced}:3"],
        )
        self.assertEqual(self.fixture.verify()[1], 1)

    def test_a_failure_message_survives_a_narrow_console(self) -> None:
        """kills: printing a report through a stream that cannot encode it.

        A name is captured whole now, so a failure message can carry any
        character an identifier may carry. On the cp1252 console this gate runs
        on, printing one raised `UnicodeEncodeError` -- a traceback with no
        citation in it, in place of the report. `main` escapes instead.
        """
        self.fixture.write_source(A, "pub fn a() {}\n")
        self.fixture.write_document("DESIGN.md", f"see `{A}::a\u0483`\n")
        stream = io.TextIOWrapper(io.BytesIO(), encoding="cp1252", newline="")
        saved = sys.stderr
        sys.stderr = stream
        try:
            code = VERIFY.main(["--repo-root", str(self.fixture.root)])
        finally:
            sys.stderr = saved
        stream.flush()
        self.assertEqual(code, 1)
        self.assertIn("FAIL", stream.buffer.getvalue().decode("cp1252"))

    def test_restamping_does_not_invent_a_reason(self) -> None:
        """kills: letting the restamper write `unresolved`/`items` entries.

        A reason is a claim about why a broken citation may stay broken.
        Generating one automatically creates a standing permission nobody read.
        """
        self.fixture.write_document("DESIGN.md", f"see `{BARE}:3`\n")
        self.fixture.restamp()
        allowance = self.fixture.allowance()
        self.assertEqual(allowance["unresolved"], {})
        self.assertEqual(allowance["items"], {})


class IncrementalCacheTests(unittest.TestCase):
    """The cache may only ever make the gate SLOWER than it could have been.

    Lane JN made this gate skip files it has already verified, because it was
    70.8 s of the 106.6 s the Extractor serial half holds the machine-wide mutex
    and 61.4 s of that was re-running three regular expressions over files that
    had not changed.  A skip is sound only while every input to a recorded result
    is still what it was, and the cases here are one per input: the citing file's
    own bytes, the allowance, this script, the SET of files in the tree, and the
    CONTENT of each file a result was resolved against.

    Every case takes the same shape -- verify, change one thing, verify again --
    and asserts the second run sees what a full run would.  A cache that ignored
    the changed input would pass the first assertion and fail the second, which
    is what makes these tests about the cache rather than about the rules.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.fixture = Fixture(Path(self._tmp.name))

    def assert_reused(self, notices: list[str], expected: int) -> None:
        summary = [line for line in notices if "reused from cache" in line]
        self.assertEqual(len(summary), 1, notices)
        self.assertIn(f"{expected} file(s) reused", summary[0])

    # -------------------------------------------------- the cache is consulted

    def test_a_recorded_result_is_what_a_reused_file_reports(self) -> None:
        """kills: making the cache decorative -- written, never read.

        Every other case here asserts that some change DEFEATS the cache. If the
        cache were never consulted at all they would all still pass, and this
        gate would simply be slow again. So one case proves the opposite
        direction: a result that is recorded, and whose recorded inputs all still
        hold, is the result the run reports -- even when it is a lie about the
        file. Nothing else in this file may write a forged entry.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::ghost_item`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()

        # Take the entry the failing run wrote and re-tag it as a clean one.
        document = self.fixture.cache()
        entry = document["entries"]["extractor/documents/DESIGN.md"]
        self.assertTrue(entry["failures"])
        entry["failures"] = []
        entry["item_total"] = 0
        root = self.fixture.root.resolve()
        entry["tag"] = VERIFY.entry_tag(
            VERIFY.cache_key(root, VERIFY.read_allowance(root)[0]), entry
        )
        self.fixture.set_cache(document)

        notices: list[str] = []
        files, lines, items, unused = self.fixture.verify(notices=notices)
        self.assertEqual((lines, items), (0, 0))
        self.assert_reused(notices, files)

        # And the same tree, verified with no cache consulted, still fails.
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify(use_cache=False)

    # ------------------------------------------------ the five recorded inputs

    def test_a_citation_broken_after_a_cached_pass_is_caught(self) -> None:
        """kills: keying a cached result on anything but the file's bytes.

        The commonest case by far: a lane edits a comment. If the citing file's
        own content did not invalidate its result, this gate would go blind to
        exactly the edits it exists to check.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_document("DESIGN.md", f"see `{A}::renamed_away`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("renamed_away", str(raised.exception))

    def test_a_narrowed_allowance_is_not_hidden_by_a_cached_pass(self) -> None:
        """kills: leaving the allowance out of the cache key.

        The allowance is the ratchet's recorded position. If an entry could be
        deleted -- or a whole file's budget dropped -- while cached results kept
        reporting the pass they were recorded under, the ratchet would stop
        moving and nothing would say so. The allowance is hashed into the key, so
        touching it re-verifies the tree.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        self.assertEqual(self.fixture.verify()[1], 1)

        allowance = self.fixture.allowance()
        allowance["line_references"] = {}
        self.fixture.set_allowance(allowance)
        notices: list[str] = []
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify(notices=notices)
        self.assertIn("new free-text line number", str(raised.exception))
        self.assertTrue(
            any("verifying every file" in notice for notice in notices), notices
        )

    def test_a_widened_allowance_is_not_hidden_either(self) -> None:
        """kills: keying only on the allowance ENTRIES a file uses.

        The narrowing case above could be passed by a cache that keyed each file
        on its own allowance section. This one could not: adding an exemption
        changes what a DIFFERENT file is allowed, and an exemption that takes
        effect without being read is the standing permission the allowance's
        reasons exist to prevent.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        self.assertEqual(self.fixture.verify()[1], 1)

        self.fixture.write_document("SECOND.md", f"see `{A}:9`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()
        allowance = self.fixture.allowance()
        allowance["line_references"]["extractor/documents/SECOND.md"] = {f"{A}:9": 1}
        self.fixture.set_allowance(allowance)
        self.assertEqual(self.fixture.verify()[1], 2)

    def test_a_file_appearing_elsewhere_defeats_a_cached_resolution(self) -> None:
        """kills: recording only the CONTENT a result depended on.

        The subtle one, and the reason `Tree.candidates` records its answers. A
        citation resolves against the SET of files carrying a basename. Adding a
        third `lib.rs` that also defines the item makes an already-written
        citation ambiguous -- with no change to the citing file and no change to
        any file the resolution read. A cache that hashed only file contents
        would report the old pass forever.
        """
        self.fixture.write_document("DESIGN.md", f"see `{BARE}::only_in_a`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        self.fixture.write_source(f"crate_c/src/lib{DOT_RS}", "pub fn only_in_a() {}\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("ambiguous", str(raised.exception))

    def test_a_shrunken_cited_file_defeats_a_cached_line_citation(self) -> None:
        """kills: recording the citation but not the file it landed in.

        Rule U catches a grandfathered line citation that has stopped landing.
        The citing file does not change when the CITED file shrinks, so this is
        rot happening entirely outside the file whose result is cached -- which
        is why every content question the resolution asked is recorded with the
        digest the answer came from.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:30`\n")
        self.fixture.restamp()
        self.assertEqual(self.fixture.verify()[1], 1)
        self.fixture.write_source(A, "pub fn only_in_a() {}\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("past the end", str(raised.exception))

    def test_a_deleted_cited_file_defeats_a_cached_resolution(self) -> None:
        """kills: reusing a resolution whose cited file has been deleted.

        End to end, through the gate. Deleting a file changes BOTH recorded
        answers about it -- the candidate list it was in and the content digest
        -- and the candidate check is the earlier of the two, so this case does
        not on its own isolate the content check; the isolation of that lives in
        `test_a_dependency_that_is_gone_is_rejected_on_its_own` and in
        `test_a_shrunken_cited_file_defeats_a_cached_line_citation`, where the
        candidate list does not move. (Codex review round 1, finding 4: this
        docstring used to claim the isolation it does not have.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.assertEqual(self.fixture.verify()[2], 1)
        (self.fixture.root / "extractor/native" / A).unlink()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("names no file", str(raised.exception))

    def test_a_key_from_elsewhere_discards_the_whole_cache(self) -> None:
        """kills: reusing entries filed under a key that is not this run's."""
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        document = self.fixture.cache()
        self.assertNotEqual(document["key"], "")
        document["key"] = "a key from another copy of this script"
        self.fixture.set_cache(document)
        notices: list[str] = []
        self.fixture.verify(notices=notices)
        self.assert_reused(notices, 0)

    def test_the_gates_own_code_is_in_the_key(self) -> None:
        """kills: dropping the script from the cache key.

        The case above changes the STORED key, so it would pass even if the key
        were built from nothing but a constant -- it proves the comparison, not
        what is compared. This one changes the GATE and requires the key to move:
        a real copy of this module with a rule added computes a different key for
        the same tree, while a byte-identical copy at a different path computes
        the same one, so what is being keyed on is the gate and not where it
        happens to live.

        A `#` comment added to the gate deliberately does NOT move the key: the
        key is over the module's compiled CODE (see `_script_digest`, and the
        race that forced it there), and a comment cannot change how another file
        is verified. It does change this file's own citing-side content, which is
        caught by this file's own entry digest like anyone else's.
        """
        source = Path(VERIFY.__file__).read_bytes()
        root = self.fixture.root.resolve()
        allowance = b'{"line_references": {}}'
        with tempfile.TemporaryDirectory() as directory:
            twin = _load_from(Path(directory) / "twin.py", source)
            commented = _load_from(
                Path(directory) / "commented.py", source + b"\n# one more comment\n"
            )
            altered = _load_from(
                Path(directory) / "altered.py", source + b"\nEXTRA_RULE = 1\n"
            )
            self.assertEqual(
                twin.cache_key(root, allowance), VERIFY.cache_key(root, allowance)
            )
            self.assertEqual(
                commented.cache_key(root, allowance), VERIFY.cache_key(root, allowance)
            )
            self.assertNotEqual(
                altered.cache_key(root, allowance), VERIFY.cache_key(root, allowance)
            )

    def test_the_gates_identity_ignores_object_identity(self) -> None:
        """kills: marshalling the code object with version 3 or 4.

        Marshal's modern versions carry a reference table: an object that appears
        twice is written once and back-referenced, so the bytes depend on which
        equal strings the process happened to have interned. MEASURED, not
        feared -- compiling this gate before and after an unrelated test in one
        interpreter produced two different version-4 blobs, one byte apart, the
        FLAG_REF bit on an interned string.

        Nothing would have failed. Every run would simply have computed a key no
        other run computes, and the cache would never be reused again: this gate
        back to 70 s inside the mutex, with the whole suite still green. Version 2
        predates the reference table, so it encodes value alone.
        """
        self.assertEqual(VERIFY.CODE_MARSHAL_VERSION, 2)
        code = compile("def f():\n    a = 'sample'\n    return a, 'sample'\n", "p", "exec")
        self.assertEqual(
            VERIFY.code_digest(code),
            VERIFY.digest_bytes(marshal.dumps(VERIFY._anonymous(code), 2)),
        )

    def test_a_gate_that_cannot_identify_itself_records_nothing(self) -> None:
        """kills: inventing a per-process identity when the real one is missing.

        A made-up key reads exactly like an ordinary cold run -- for ever -- so
        the run must instead say it has no identity, verify everything, and write
        nothing for a later run to find under a key that means nothing.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::ghost_item`\n")
        # A LIE LEFT WHERE AN UNIDENTIFIED RUN MIGHT FIND IT. Deleting the cache
        # first would have proved only that nothing was written; a run that
        # consulted the cache under the empty key would have passed that.
        # (Codex review round 3, finding 3.)
        relative = "extractor/documents/DESIGN.md"
        clean = {
            "digest": VERIFY.digest_bytes(
                (self.fixture.root / "extractor/documents/DESIGN.md").read_bytes()
            ),
            "candidates": [],
            "files": {},
            "failures": [],
            "line_total": 0,
            "item_total": 0,
            "used": [],
        }
        clean["tag"] = VERIFY.entry_tag("", clean)
        self.fixture.set_cache(
            {"schema": VERIFY.CACHE_SCHEMA, "key": "", "entries": {relative: clean}}
        )
        untouched = self.fixture.cache_path().read_bytes()

        genuine = VERIFY.SCRIPT_DIGEST
        VERIFY.SCRIPT_DIGEST = None
        try:
            notices: list[str] = []
            with self.assertRaises(VERIFY.VerificationError) as raised:
                self.fixture.verify(notices=notices)
            # The forged entry was not consulted: the broken citation is found.
            self.assertIn("ghost_item", str(raised.exception))
            # And no key can be minted while the gate has no identity.
            with self.assertRaises(VERIFY.VerificationError):
                VERIFY.cache_key(self.fixture.root.resolve(), b"{}")
        finally:
            VERIFY.SCRIPT_DIGEST = genuine
        self.assertTrue(
            any("identity of its own code" in notice for notice in notices), notices
        )
        # Nothing recorded: the file is byte for byte what was left there. The
        # key alone was not enough -- a run that skipped the forged entry but
        # then wrote its own results under the empty key would have kept it.
        # (Codex review round 4, finding 4.)
        self.assertEqual(self.fixture.cache_path().read_bytes(), untouched)

    def test_the_gates_identity_is_the_same_in_every_process(self) -> None:
        """kills: a script digest that varies run to run.

        The case above pins the encoding; this one checks the property end to
        end, in the place it has to hold: two interpreters, different hash seeds,
        same answer.
        """
        program = (
            "import importlib.util, sys;"
            f"spec = importlib.util.spec_from_file_location('g', {VERIFY.__file__!r});"
            "m = importlib.util.module_from_spec(spec);"
            "spec.loader.exec_module(m);"
            "print(m.SCRIPT_DIGEST)"
        )
        digests = set()
        for seed in ("0", "1"):
            environment = dict(os.environ, PYTHONHASHSEED=seed)
            finished = subprocess.run(
                [sys.executable, "-c", program],
                capture_output=True,
                text=True,
                env=environment,
            )
            self.assertEqual(finished.returncode, 0, finished.stderr)
            digests.add(finished.stdout.strip())
        self.assertEqual(len(digests), 1, digests)
        self.assertNotIn("unidentified-script-", digests.pop())

    def test_the_allowance_bytes_are_in_the_key(self) -> None:
        """kills: dropping the allowance from the cache key."""
        root = self.fixture.root.resolve()
        self.assertNotEqual(
            VERIFY.cache_key(root, b'{"line_references": {}}'),
            VERIFY.cache_key(root, b'{"line_references": {"a": {"b": 1}}}'),
        )

    def test_the_allowance_is_hashed_and_parsed_from_one_read(self) -> None:
        """kills: reading the allowance once to parse and again to hash.

        Two reads admit a run that verifies under one allowance and RECORDS the
        result under the key of another; unlike an ordinary read-while-changing
        race that one persists, because the next run computes the new key and
        finds the stale results filed under it.

        Comparing `read_allowance`'s two outputs proves nothing here -- on an
        unchanged file two reads agree, so that version passed whether the gate
        read once or twice. (Codex review round 2, finding 4.) So the file is
        made to CHANGE UNDER THE RUN: every read after the first returns a
        different (still valid) allowance. A gate that read twice would file its
        results under a key built from bytes it never verified against, and the
        two assertions below -- one read, and a stored key that matches the FIRST
        bytes -- are exactly what that would break.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}:3`\n")
        self.fixture.restamp()
        root = self.fixture.root.resolve()
        allowance_path = root / VERIFY.ALLOWANCE
        genuine = VERIFY.read_bytes
        first: list[bytes] = []

        def changing(path):
            data = genuine(path)
            try:
                is_allowance = os.path.samefile(path, allowance_path)
            except OSError:
                is_allowance = False
            if is_allowance:
                first.append(data)
                if len(first) == 1:
                    # THE FILE ITSELF IS REPLACED, with bytes that are not JSON
                    # at all. A second read reached by ANY route -- this stub,
                    # `Path.read_bytes`, `open` -- now sees them, so a gate that
                    # parses twice fails outright and a gate that hashes a second
                    # read stores a key these assertions reject. Whitespace was
                    # not enough: it re-parses to the same object, so an
                    # implementation that hashed one read and parsed another
                    # still satisfied both assertions. (Codex review rounds 3
                    # and 4, finding 2 both times.)
                    allowance_path.write_bytes(b"{ this is not an allowance")
                else:
                    return data + b" " * len(first)
            return data

        VERIFY.read_bytes = changing
        try:
            self.fixture.verify()
        finally:
            VERIFY.read_bytes = genuine

        self.assertEqual(len(first), 1, f"the allowance was read {len(first)} times")
        self.assertEqual(
            self.fixture.cache()["key"], VERIFY.cache_key(root, first[0])
        )

    # ------------------------------------------------------- damage and repair

    def test_a_hand_edited_entry_is_verified_again(self) -> None:
        """kills: trusting an entry's recorded RESULT without a tag over it.

        The dependency checks compare an entry's record of the world against the
        world; on their own they cannot notice an entry whose recorded FAILURES
        were deleted in place, and deleting them would buy a green run over a
        broken citation. The tag covers the record and the result together.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::ghost_item`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()
        document = self.fixture.cache()
        entry = document["entries"]["extractor/documents/DESIGN.md"]
        entry["failures"] = []
        self.fixture.set_cache(document)
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify()
        self.assertIn("ghost_item", str(raised.exception))

    def test_every_recorded_field_is_covered_by_the_tag(self) -> None:
        """kills: adding a field to an entry and leaving it unauthenticated.

        Asked of `entry_tag` DIRECTLY, not through `reusable`. Through `reusable`
        the case was partly vacuous: perturbing `digest` or `files` is also
        rejected by the dependency checks, so dropping either from `ENTRY_FIELDS`
        would have left the test green while the field travelled unsigned.
        (Codex review round 1, finding 4.) Here the only mechanism that can
        notice is the tag itself.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        original = self.fixture.cache()["entries"]["extractor/documents/DESIGN.md"]
        root = self.fixture.root.resolve()
        key = VERIFY.cache_key(root, VERIFY.read_allowance(root)[0])
        self.assertEqual(original["tag"], VERIFY.entry_tag(key, original))
        fields = set(original) - {"tag"}
        self.assertTrue(fields, original)
        for field in fields:
            damaged = json.loads(json.dumps(original))
            damaged[field] = _perturb(original[field])
            self.assertNotEqual(
                VERIFY.entry_tag(key, damaged),
                original["tag"],
                f"a changed `{field}` does not move the tag",
            )

    def test_the_tag_is_bound_to_the_cache_key(self) -> None:
        """kills: tagging an entry with nothing but its own contents.

        An entry carried into a tree with a different allowance or a different
        script would otherwise still authenticate itself.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        entry = self.fixture.cache()["entries"]["extractor/documents/DESIGN.md"]
        root = self.fixture.root.resolve()
        self.assertNotEqual(
            VERIFY.entry_tag(VERIFY.cache_key(root, b"{}"), entry),
            VERIFY.entry_tag(VERIFY.cache_key(root, b'{"items": {}}'), entry),
        )

    def test_a_dependency_that_is_gone_is_rejected_on_its_own(self) -> None:
        """kills: skipping a recorded content dependency that no longer exists.

        Isolated from the candidate check, which in a real deletion fires first
        and would carry the case on its own. Here the entry is retagged so that
        the ONLY thing wrong with it is a recorded content dependency naming a
        file the tree does not have -- and a run that read that as "nothing to
        compare" would reuse the entry. (Codex review round 1, finding 4.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        relative = "extractor/documents/DESIGN.md"
        entry = self.fixture.cache()["entries"][relative]
        root = self.fixture.root.resolve()
        key = VERIFY.cache_key(root, VERIFY.read_allowance(root)[0])
        store = VERIFY.FileStore(root)
        tree = VERIFY.Tree(root, store)
        self.assertTrue(VERIFY.reusable(entry, relative, tree, store, key))

        entry["files"] = dict(entry["files"])
        entry["files"]["extractor/native/never_existed.rs"] = "0" * 64
        entry["tag"] = VERIFY.entry_tag(key, entry)
        self.assertFalse(VERIFY.reusable(entry, relative, tree, store, key))

    def test_an_unreadable_cache_verifies_everything(self) -> None:
        """kills: any repair, partial read, or silent skip on a damaged cache.

        A cache is a file: it can be truncated by a killed process or by a full
        disk. The only safe reading of one that will not parse is that nothing is
        known, and the run says so rather than continuing quietly.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::ghost_item`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()
        text = self.fixture.cache_path().read_text(encoding="utf-8")
        self.fixture.cache_path().write_text(text[: len(text) // 2], encoding="utf-8")
        notices: list[str] = []
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify(notices=notices)
        self.assertIn("ghost_item", str(raised.exception))
        self.assertTrue(
            any("could not be read" in notice for notice in notices), notices
        )

    def test_a_deeply_nested_cache_verifies_everything(self) -> None:
        """kills: catching only `ValueError` when the cache will not parse.

        A few thousand nested arrays are valid JSON text that `json.loads`
        refuses with `RecursionError` -- a `RuntimeError`, which slipped past the
        `ValueError` catch and out through `main` as a traceback rather than as
        a full verification. (Codex review round 1, finding 2.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::ghost_item`\n")
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()
        depth = sys.getrecursionlimit() * 3
        self.fixture.cache_path().write_text(
            "[" * depth + "]" * depth, encoding="utf-8"
        )
        notices: list[str] = []
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify(notices=notices)
        self.assertIn("ghost_item", str(raised.exception))
        self.assertTrue(
            any("could not be read" in notice for notice in notices), notices
        )

    def test_a_symlinked_cache_override_into_the_scan_set_is_refused(self) -> None:
        """kills: judging the cache path only by where it resolves to.

        A scanned path that is a symlink out of the repository resolves outside
        it, so a resolve-only check accepted it -- and the gate then wrote the
        cache through the link, leaving the in-tree symlink for the next scan to
        follow and read as a file full of citations nobody wrote. (Codex review
        round 1, finding 3.) The written spelling is what the scan walks, so the
        written spelling has to be outside it too.
        """
        outside = Path(self._tmp.name).parent / f"jn_outside_{os.getpid()}.json"
        link = self.fixture.root / "extractor/documents/cache.json"
        try:
            link.symlink_to(outside)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"this machine does not allow symlinks here: {error}")
        self.addCleanup(lambda: outside.unlink(missing_ok=True))
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify(cache_path=link)
        self.assertIn("inside the tree this gate scans", str(raised.exception))
        self.assertFalse(outside.exists(), "the cache was written through the link")

    def test_a_recursive_allowance_is_a_named_failure(self) -> None:
        """kills: letting a `RecursionError` out of the allowance parser.

        Unlike a damaged cache, an allowance nobody can parse must be FATAL and
        named: continuing with an empty one would fail every citation in the
        tree, and `main` catches only `VerificationError`, so an escaping
        `RecursionError` is a traceback where the gate promises a report.
        (Codex review round 2, finding 3.)
        """
        depth = sys.getrecursionlimit() * 3
        (self.fixture.root / VERIFY.ALLOWANCE).write_text(
            "[" * depth + "]" * depth, encoding="utf-8"
        )
        with self.assertRaises(VERIFY.VerificationError):
            self.fixture.verify()
        # EXIT 1 IS NOT ENOUGH: a traceback also exits non-zero, and a silent
        # failure exits 1 too. What this gate promises is a REPORT that names
        # the file and the reason. (Codex review round 3, finding 4.)
        captured = io.StringIO()
        saved = sys.stderr
        sys.stderr = captured
        try:
            code = VERIFY.main(["--repo-root", str(self.fixture.root)])
        finally:
            sys.stderr = saved
        printed = captured.getvalue()
        self.assertEqual(code, 1)
        self.assertNotIn("Traceback", printed)
        self.assertIn("citation integrity check: FAIL", printed)
        self.assertIn(VERIFY.ALLOWANCE.name, printed)
        self.assertIn("recursion", printed.lower())

    def test_an_aliased_windows_path_cannot_smuggle_the_cache_into_the_scan(self) -> None:
        """kills: judging the cache path by its spelling instead of its identity.

        One directory has several names on Windows -- an extended-length prefix,
        an administrative share, a different case -- and none of them can be made
        relative to the ordinary spelling by text, so a lexical containment check
        accepted every one of them and would have written the cache into the
        scanned tree. (Codex review round 2, finding 2.)
        """
        root = self.fixture.root.resolve()
        target = "extractor/documents/cache.json"
        aliases = [str(root / target).upper(), "\\\\?\\" + str(root / target)]
        for alias in aliases:
            with self.assertRaises(VERIFY.VerificationError, msg=alias) as raised:
                self.fixture.verify(cache_path=Path(alias))
            self.assertIn("inside the tree this gate scans", str(raised.exception))
        # And the default is still accepted, by the same identity walk.
        self.assertTrue(str(VERIFY.cache_location(root, None)).endswith(
            os.path.join("target", "citation_integrity_cache.json")
        ))

    def test_the_cache_cannot_be_pointed_at_the_allowance(self) -> None:
        """kills: letting the cache be written over the gate's own authority.

        `UNSCANNED` names two files -- the cache and the ALLOWANCE -- and a
        containment test that accepted anything listed there accepted the
        allowance as a cache destination: the run would read it, verify against
        it, and then atomically replace it with the cache document, destroying
        the record of what is grandfathered. (Codex review round 4, finding 1.)
        Only a skipped DIRECTORY may hold the cache.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        allowance = self.fixture.root / VERIFY.ALLOWANCE
        before = allowance.read_bytes()
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify(cache_path=allowance)
        self.assertIn("inside the tree this gate scans", str(raised.exception))
        self.assertEqual(allowance.read_bytes(), before)

    def test_a_file_changing_mid_run_is_refused_not_absorbed(self) -> None:
        """kills: recording a dependency against bytes it was not checked against.

        `Tree` remembers a file's declarations the first time a citation resolves
        against it. If that file is then edited WHILE the run continues, a later
        read of it -- as a citing file, say -- would move the remembered digest
        to the new bytes, and a citation resolved after that would be recorded as
        depending on bytes that were never analysed. The entry would look valid
        for ever. (Codex review round 4, finding 2.) One file has one content per
        run; a second read that disagrees stops the run instead.
        """
        root = self.fixture.root.resolve()
        store = VERIFY.FileStore(root)
        relative = "extractor/native/" + A
        self.assertTrue(store.digest(relative))
        (root / relative).write_text("pub fn only_in_a() {}\n// moved\n", encoding="utf-8")
        with self.assertRaises(VERIFY.VerificationError) as raised:
            store.text(relative)
        self.assertIn("changed while this run was reading", str(raised.exception))

    def test_an_unreachable_cache_location_does_not_fail_the_gate(self) -> None:
        """kills: letting an unreachable cache path escape as a traceback.

        `Path.resolve` touches the filesystem, and on Windows an unreachable UNC
        host raises WinError 53 -- out past `main`, which catches only
        `VerificationError`. An unavailable network drive would have turned an
        optional optimization into an unexplained failure of the citation gate.
        (Codex review round 3, finding 1.) There is nothing to fail about: a
        cache that cannot be reached is a cache that is not used.
        The failure is INJECTED rather than provoked with a real unreachable
        host: a genuine `\\\\host.invalid\\share` probe costs about 35 seconds of
        Windows name resolution, and this suite runs in every battery. The error
        raised is the one Windows raises.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        unreachable = Path(r"\\a-host-that-is-not-there\share\cache.json")
        genuine = Path.resolve

        def refusing(self, *args, **kwargs):
            if str(self).startswith(r"\\a-host-that-is-not-there"):
                raise OSError(53, "The network path was not found")
            return genuine(self, *args, **kwargs)

        Path.resolve = refusing
        try:
            notices: list[str] = []
            _files, _lines, items, _unused = self.fixture.verify(
                cache_path=unreachable, notices=notices
            )
            self.assertEqual(items, 1)
            self.assertTrue(
                any("cannot be reached" in notice for notice in notices), notices
            )
            # And a broken citation is still caught while the cache is unusable.
            self.fixture.write_document("DESIGN.md", f"see `{A}::ghost_item`\n")
            with self.assertRaises(VERIFY.VerificationError) as raised:
                self.fixture.verify(cache_path=unreachable)
            self.assertIn("ghost_item", str(raised.exception))
        finally:
            Path.resolve = genuine

    def test_a_cache_that_is_not_an_object_verifies_everything(self) -> None:
        """kills: assuming the file's shape once it has parsed as JSON."""
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        self.fixture.cache_path().write_text("[1, 2, 3]", encoding="utf-8")
        notices: list[str] = []
        self.fixture.verify(notices=notices)
        self.assert_reused(notices, 0)

    def test_an_entry_of_the_wrong_shape_is_verified_again(self) -> None:
        """kills: reading counts out of an entry without checking their types.

        A `True` sums into a total as 1 and a string does not sum at all. The
        malformed entry here is CORRECTLY TAGGED and its digest is the real one,
        so the tag check and the dependency checks all pass it and `well_formed`
        is the only thing standing between a wrong type and a reported count --
        an untagged fixture would have been caught by the tag instead and proved
        nothing about the shape check. (Codex review round 1, finding 4.)
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        relative = "extractor/documents/DESIGN.md"
        entry = self.fixture.cache()["entries"][relative]
        root = self.fixture.root.resolve()
        key = VERIFY.cache_key(root, VERIFY.read_allowance(root)[0])
        store = VERIFY.FileStore(root)
        tree = VERIFY.Tree(root, store)
        for field, wrong in (
            ("item_total", True),
            ("line_total", -1),
            ("failures", [17]),
            ("used", [["a citation", "not a count"]]),
            ("files", {"a/path.rs": 5}),
            ("candidates", [["", "lib.rs"]]),
        ):
            damaged = json.loads(json.dumps(entry))
            damaged[field] = wrong
            damaged["tag"] = VERIFY.entry_tag(key, damaged)
            self.assertFalse(
                VERIFY.well_formed(damaged), f"`{field}` = {wrong!r} passed as shaped"
            )
            self.assertFalse(
                VERIFY.reusable(damaged, relative, tree, store, key),
                f"a correctly tagged entry with `{field}` = {wrong!r} was reused",
            )

    def test_a_cache_that_cannot_be_written_does_not_fail_the_gate(self) -> None:
        """kills: making the gate's verdict depend on a scratch file.

        A cache that cannot be written costs the NEXT run its speed and costs
        this one's correctness nothing, so it is a notice rather than a failure.
        """
        blocked = self.fixture.root / "target" / "citation_integrity_cache.json"
        blocked.parent.mkdir(parents=True, exist_ok=True)
        # A DIRECTORY where the file belongs: every write to it fails, on every
        # platform, without needing permissions this test cannot rely on.
        blocked.mkdir()
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        notices: list[str] = []
        files, lines, items, unused = self.fixture.verify(notices=notices)
        self.assertEqual(items, 1)
        self.assertTrue(any("not written" in notice for notice in notices), notices)

    # ------------------------------------------------------------ the two paths

    def test_full_and_incremental_agree_on_the_same_tree(self) -> None:
        """kills: letting the two paths report different counts.

        The counts are what `recorder_host`'s test compares against the tree, so
        a path that reported its own numbers would be a gate reporting on a
        repository nobody has.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.write_document("SECOND.md", f"and `{B}::only_in_b`\n")
        self.fixture.write_source(f"crate_a/src/extra{DOT_RS}", "pub fn spare() {}\n")
        full = self.fixture.verify(use_cache=False)
        warm = self.fixture.verify()
        self.assertEqual(full, warm)
        self.assertEqual(full[2], 2)

    def test_full_refreshes_the_cache_it_did_not_read(self) -> None:
        """kills: leaving the landing run's work unrecorded.

        The central runner passes `--full`, and the in-lane runs that follow it
        are the ones this lane exists to shorten. A full run that did not write
        would leave every one of them cold.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify(use_cache=False)
        self.assertTrue(self.fixture.cache_path().is_file())
        notices: list[str] = []
        files, _lines, _items, _unused = self.fixture.verify(notices=notices)
        self.assert_reused(notices, files)

    def test_an_entry_for_a_deleted_file_does_not_survive(self) -> None:
        """kills: letting the cache grow a result about a file nobody has.

        Only the files scanned this run are carried forward. An entry that
        outlived its file is a record about nothing that a later file of the same
        path would inherit.
        """
        self.fixture.write_document("GONE.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        self.assertIn("extractor/documents/GONE.md", self.fixture.cache()["entries"])
        (self.fixture.root / "extractor/documents/GONE.md").unlink()
        self.fixture.verify()
        self.assertNotIn("extractor/documents/GONE.md", self.fixture.cache()["entries"])

    def test_the_cache_is_not_scanned_and_may_not_be_put_where_it_would_be(self) -> None:
        """kills: writing the cache anywhere the gate then reads.

        The cache spells out every failure message and every citation a file
        carries. Scanned, it would fail the gate on citations nobody wrote -- so
        its home is under a skipped directory, and an override that would land in
        the scan set is refused rather than accepted and regretted.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        self.fixture.verify()
        relative = VERIFY.CACHE.as_posix()
        self.assertIn(relative, VERIFY.UNSCANNED)
        self.assertNotIn(relative, VERIFY.scan_files(self.fixture.root.resolve()))
        with self.assertRaises(VERIFY.VerificationError) as raised:
            self.fixture.verify(
                cache_path=self.fixture.root / "extractor/documents/cache.json"
            )
        self.assertIn("inside the tree this gate scans", str(raised.exception))

    def test_the_report_line_is_the_whole_of_stdout(self) -> None:
        """kills: letting a cache notice reach stdout.

        `recorder_host`'s citation test refuses a stdout that is not exactly one
        `PASS (` line, because a report that can grow a line is a report whose
        counts stop being compared. Everything about how the run was performed
        goes to stderr.
        """
        self.fixture.write_document("DESIGN.md", f"see `{A}::only_in_a`\n")
        for expected_runs in range(2):
            captured = io.StringIO()
            saved = sys.stdout
            sys.stdout = captured
            try:
                code = VERIFY.main(["--repo-root", str(self.fixture.root)])
            finally:
                sys.stdout = saved
            self.assertEqual(code, 0, expected_runs)
            printed = captured.getvalue().strip()
            self.assertFalse("\n" in printed, printed)
            self.assertTrue(printed.startswith("citation integrity check: PASS ("))


class DecoderTests(unittest.TestCase):
    """The bytes-first reader must say exactly what the text reader said.

    The cache needs a file's BYTES to hash, so the gate now reads bytes and
    decodes them itself. That is only safe while the decode is the same decode --
    a difference over a BOM, a lone CR or an invalid sequence would move line
    numbers and change which citations resolve.
    """

    def test_decode_matches_reading_the_file_as_text(self) -> None:
        """kills: hand-rolling the newline or error handling in `decode`, and
        letting the gate's own reader bypass it.

        `FileStore.text` is what production actually reads citing files with, so
        it is exercised here beside `decode` and `read_text`: a store that
        decoded its own way would have left this green while the gate read
        something else. (Codex review round 1, finding 4.)
        """
        cases = {
            "crlf": b"a\r\nb\r\n",
            "lone cr": b"a\rb\r",
            "mixed": b"a\r\nb\nc\rd",
            "bom": "﻿# a comment\n".encode("utf-8"),
            "invalid utf-8": b"before \xff\xfe after\n",
            "truncated sequence": b"head \xe2\x82\n",
            "empty": b"",
            "no trailing newline": b"tail",
            "nul-free binary-ish": b"\x01\x02\x03",
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "probe.txt"
            for name, data in cases.items():
                path.write_bytes(data)
                with io.open(path, encoding="utf-8", errors="replace") as handle:
                    expected = handle.read()
                self.assertEqual(VERIFY.decode(data), expected, name)
                self.assertEqual(VERIFY.read_text(path), expected, name)
                # The reader the gate actually verifies files through, including
                # the second read a store takes after it has taken the digest.
                store = VERIFY.FileStore(root)
                self.assertEqual(store.text("probe.txt"), expected, name)
                self.assertEqual(store.digest("probe.txt"), VERIFY.digest_bytes(data))
                self.assertEqual(store.text("probe.txt"), expected, name)

    def test_a_missing_file_is_named_rather_than_traced(self) -> None:
        """kills: dropping the guard that turns an OSError into a report."""
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "absent.txt"
            with self.assertRaises(VERIFY.VerificationError) as raised:
                VERIFY.read_bytes(missing)
            self.assertIn("absent.txt", str(raised.exception))


def _load_from(path: Path, source: bytes):
    """Import a real copy of the gate from ``path``, so its own bytes differ."""
    path.write_bytes(source)
    spec = importlib.util.spec_from_file_location(f"twin_{path.stem}", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _perturb(value):
    """A different value of the same JSON kind, for the tag-coverage sweep."""
    if isinstance(value, str):
        return value + "x"
    if isinstance(value, bool):
        return not value
    if isinstance(value, int):
        return value + 1
    if isinstance(value, list):
        return value + [["extractor/", f"other{DOT_RS}", []]] if value else ["added"]
    if isinstance(value, dict):
        changed = dict(value)
        changed["extractor/native/nothing.rs"] = "0" * 64
        return changed
    raise AssertionError(f"no perturbation for {value!r}")


if __name__ == "__main__":
    unittest.main()
