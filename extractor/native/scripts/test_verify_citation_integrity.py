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

    def verify(self):
        return VERIFY.verify_repo(self.root)


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


if __name__ == "__main__":
    unittest.main()
