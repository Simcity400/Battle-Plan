#!/usr/bin/env python3
"""Focused deterministic tests for verify_world_fold_citations.py."""

from __future__ import annotations

import importlib.util
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("verify_world_fold_citations.py")
SPEC = importlib.util.spec_from_file_location("verify_world_fold_citations", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = VERIFY
SPEC.loader.exec_module(VERIFY)


class CitationFixture:
    def __init__(self, root: Path, world: str) -> None:
        self.root = root
        self.world = world
        self.world_path = root / VERIFY.WORLD_FOLD
        self.wire_dir = root / VERIFY.WIRE_SRC
        self.world_path.parent.mkdir(parents=True)
        self.wire_dir.mkdir(parents=True)
        self.world_path.write_text(world, encoding="utf-8", newline="\n")

    def add_module(
        self,
        domain: int,
        functions: tuple[str, ...],
        *,
        digest_override: str | None = None,
        include_marker: bool = True,
    ) -> None:
        spans = VERIFY.function_spans(self.world)
        envelope = VERIFY.Span(
            min(spans[name].start for name in functions),
            max(spans[name].end for name in functions),
        )
        digest = digest_override or VERIFY.normalized_span_hash(
            self.world, envelope
        )
        marker = ""
        if include_marker:
            marker = (
                "// @world-fold-authority "
                f"domain={domain} functions={','.join(functions)} "
                f"lines={envelope.start}-{envelope.end} sha256={digest}\n"
            )
        citation = f"`world_fold.rs:{envelope.start}`-`:{envelope.end}`"
        (self.wire_dir / f"d{domain}.rs").write_text(
            f"//! fixture derived from {citation}\n{marker}",
            encoding="utf-8",
            newline="\n",
        )

    def add_lane_authority(
        self,
        domain: int,
        relative: str,
        function: str,
        source: str,
    ) -> tuple[int, str, str]:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8", newline="\n")
        spans = VERIFY.function_spans(source)
        span = spans[function]
        digest = VERIFY.normalized_span_hash(source, span)
        module = self.wire_dir / f"d{domain}.rs"
        with module.open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(
                "// @lane-payload-authority "
                f"domain={domain} path={relative} function={function} "
                f"lines={span.start}-{span.end} sha256={digest}\n"
            )
        return (domain, relative, function)


WORLD = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_one(),
            39 => self.fold_shared(),
            40 => self.fold_shared(),
            51 => self.fold_lane(),
            67 => self.fold_multi(),
            _ => (),
        }
    }

    fn fold_one(&mut self) {
        let diagnostic = "a brace in a string: }";
        /* and a nested comment { /* } */ } */
        self.value += 1;
    }

    fn fold_shared(&mut self) {
        self.value += 2;
    }

    fn fold_lane(&mut self) {
        self.value += 3;
    }

    fn fold_multi(&mut self) {
        self.fold_multi_helper();
    }

    fn fold_multi_helper(&mut self) {
        let raw = r#"raw braces { }"#;
    }
}
"""


class VerifyWorldFoldCitationsTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), WORLD)

    def test_accepts_exact_dispatch_span_and_digest(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                ),
                (1, 0, 0, 0),
            )

    def test_rejects_line_drift_even_when_function_is_unchanged(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            fixture.world_path.write_text(
                "// inserted before every citation\n" + WORLD,
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "declared lines"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                )

    def test_reports_every_drifted_module_not_only_the_first(self) -> None:
        # A first-failure-only checker reported one stale citation and hid 31
        # more, and because this check gates Cargo the whole central validation
        # was unreachable while they accumulated. One run must name them all.
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            fixture.add_module(39, ("fold_shared",))
            fixture.add_module(40, ("fold_shared",))
            fixture.world_path.write_text(
                "// inserted before every citation\n" + WORLD,
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaises(VERIFY.VerificationError) as raised:
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())
            message = str(raised.exception)
            self.assertIn("3 citation authority failure(s)", message)
            for name in ("d1.rs", "d39.rs", "d40.rs"):
                self.assertIn(name, message)

    def test_lane_authority_is_still_collected_from_a_drifted_module(self) -> None:
        # The lane marker pins a capture-lane decoder, not the fold envelope.
        # Dropping it because the module's world-fold line range drifted would
        # turn one stale range into a spurious "lane authority marker set
        # drifted" and bury the real cause.
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(51, ("fold_lane",))
            key = fixture.add_lane_authority(
                51,
                "extractor/native/lanes/example/original.rs",
                "decode_payload",
                "pub fn decode_payload(payload: &[u8]) -> bool {\n"
                "    !payload.is_empty()\n"
                "}\n",
            )
            fixture.world_path.write_text(
                "// inserted before every citation\n" + WORLD,
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaises(VERIFY.VerificationError) as raised:
                VERIFY.verify_repo(fixture.root, required_lane_authorities=(key,))
            message = str(raised.exception)
            self.assertIn("d51.rs: declared lines", message)
            self.assertNotIn("lane authority marker set drifted", message)

    def test_rejects_same_line_count_semantic_drift_by_digest(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            fixture.world_path.write_text(
                WORLD.replace("self.value += 1;", "self.value += 9;"),
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "authority digest changed"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                )

    def test_shared_dispatch_function_is_valid_for_both_domains(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(39, ("fold_shared",))
            fixture.add_module(40, ("fold_shared",))
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                ),
                (2, 0, 0, 0),
            )

    def test_multi_function_authority_pins_the_whole_envelope(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(
                67, ("fold_multi", "fold_multi_helper")
            )
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                ),
                (1, 0, 0, 0),
            )

    def test_missing_marker_fails_closed(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",), include_marker=False)
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "expected one marker, found 0"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                )

    def test_lane_authority_digest_reads_the_original_function(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(51, ("fold_lane",))
            key = fixture.add_lane_authority(
                51,
                "extractor/native/lanes/example/original.rs",
                "decode_payload",
                "pub fn decode_payload(payload: &[u8]) -> bool {\n"
                "    !payload.is_empty()\n"
                "}\n",
            )
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=(key,)
                ),
                (1, 1, 0, 0),
            )
            authority = fixture.root / key[1]
            authority.write_text(
                "pub fn decode_payload(payload: &[u8]) -> bool {\n"
                "    payload.is_empty()\n"
                "}\n",
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "lane authority digest changed"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=(key,)
                )

    # --- the free-text line-number ratchet ---------------------------------

    def _append_comment(self, fixture: CitationFixture, domain: int, text: str) -> None:
        module = fixture.wire_dir / f"d{domain}.rs"
        with module.open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(text)

    def _write_allowance(
        self,
        fixture: CitationFixture,
        modules: dict,
        documents: dict | None = None,
    ) -> None:
        path = fixture.root / VERIFY.PROSE_ALLOWANCE
        path.parent.mkdir(parents=True, exist_ok=True)
        payload: dict = {"modules": modules}
        if documents is not None:
            payload["documents"] = documents
        path.write_text(json.dumps(payload), encoding="utf-8", newline="\n")

    def _write_document(
        self,
        fixture: CitationFixture,
        name: str,
        text: str,
        root_index: int = 0,
    ) -> str:
        path = fixture.root / VERIFY.DOCUMENT_ROOTS[root_index] / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")
        return path.relative_to(fixture.root).as_posix()

    def test_rejects_a_new_free_text_line_number(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._append_comment(fixture, 1, "// see `world_fold.rs:123`\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:123` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_grandfathered_line_number_is_allowed_exactly_once(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._append_comment(fixture, 1, "// see `world_fold.rs:123`\n")
            self._write_allowance(fixture, {"d1.rs": [123]})
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 1, 0),
            )
            # A second use of the same number is still a NEW reference.
            self._append_comment(fixture, 1, "// and again `world_fold.rs:123`\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:123` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_the_allowance_may_shrink_but_swapping_fails(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_allowance(fixture, {"d1.rs": [123, 456]})
            # Citing neither is fine: the ratchet only ever turns one way.
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )
            # Trading a listed number for an unlisted one is not shrinking.
            self._append_comment(fixture, 1, "// see `world_fold.rs:789`\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:789` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_the_gated_citation_is_never_counted_as_free_text(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            # The fixture module carries only its gated `world_fold.rs:S`-`:E`
            # citation, and no allowance file exists at all.
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_missing_allowance_file_grandfathers_nothing(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._append_comment(fixture, 1, "// see `world_fold.rs:123`\n")
            self.assertFalse((fixture.root / VERIFY.PROSE_ALLOWANCE).exists())
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:123` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_second_copy_of_the_gated_citation_is_not_a_second_exemption(
        self,
    ) -> None:
        # Found by review: if every occurrence of the gated text counted as
        # owned, pasting a second unpaired copy would smuggle two unchecked line
        # numbers past the ratchet.  The digest backs one citation.
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            module = fixture.wire_dir / "d1.rs"
            source = module.read_text(encoding="utf-8")
            citation = re.search(
                r"`world_fold\.rs:\d+`-`:\d+`", source
            ).group(0)
            self._append_comment(fixture, 1, f"// a second copy: {citation}\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                "expected exactly one human authority citation",
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

            # And the extraction itself must see the copy's numbers as free text.
            marker = VERIFY.WORLD_MARKER_RE.search(
                module.read_text(encoding="utf-8")
            )
            found = VERIFY.ungated_prose_references(
                module.read_text(encoding="utf-8"), marker
            )
            self.assertEqual(len(found), 1, found)

    def test_rejects_a_new_document_line_number(self) -> None:
        # Documents had NO coverage before this check: every `world_fold.rs:N`
        # in `extractor/documents` was free text nothing read.
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_document(
                fixture, "DESIGN.md", "the fold refuses it at `world_fold.rs:4242`.\n"
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                r"DESIGN\.md: `world_fold\.rs:4242` is a free-text",
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_grandfathered_document_line_number_is_allowed_exactly_once(
        self,
    ) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            relative = self._write_document(
                fixture, "DESIGN.md", "see `world_fold.rs:4242`.\n"
            )
            self._write_allowance(fixture, {}, {relative: [4242]})
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 1),
            )
            # A second citation of the same number is still a NEW reference.
            path = fixture.root / relative
            path.write_text(
                "see `world_fold.rs:4242` and again `world_fold.rs:4242`.\n",
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:4242` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_document_budget_does_not_carry_across_documents(self) -> None:
        # The budget is per (document, line), so one document's amnesty must not
        # license the same number in another. Nested documents are reached too.
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            allowed = self._write_document(
                fixture, "DESIGN.md", "see `world_fold.rs:4242`.\n"
            )
            self._write_allowance(fixture, {}, {allowed: [4242]})
            self._write_document(
                fixture, "nested/OTHER.md", "also `world_fold.rs:4242`.\n"
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                r"nested/OTHER\.md: `world_fold\.rs:4242` is a free-text",
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_missing_allowance_grandfathers_no_document_reference(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_document(
                fixture, "DESIGN.md", "see `world_fold.rs:4242`.\n"
            )
            self.assertFalse((fixture.root / VERIFY.PROSE_ALLOWANCE).exists())
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:4242` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_every_document_root_is_scanned(self) -> None:
        # Scanning one document tree and calling it "the documents" is how this
        # gap existed in the first place: `replay_interface/documents` was
        # citing world_fold.rs by line 28 times while only
        # `extractor/documents` was read. Each configured root must be reached.
        for index in range(len(VERIFY.DOCUMENT_ROOTS)):
            temporary, fixture = self.fixture()
            with temporary:
                fixture.add_module(1, ("fold_one",))
                self._write_document(
                    fixture,
                    "DESIGN.md",
                    "see `world_fold.rs:4242`.\n",
                    root_index=index,
                )
                with self.assertRaisesRegex(
                    VERIFY.VerificationError,
                    r"`world_fold\.rs:4242` is a free-text",
                    msg=f"document root {VERIFY.DOCUMENT_ROOTS[index]} is not scanned",
                ):
                    VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_grandfathered_document_outside_every_root_fails(self) -> None:
        # Renaming a root, moving a document out of one, or changing its
        # extension leaves the citations on disk and quietly shrinks the
        # reported count. Absence is still fine -- deleting a document must not
        # force an allowance edit in the same commit -- so the failure is
        # specifically "exists, is grandfathered, was not scanned".
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            stray = fixture.root / "somewhere_else" / "DESIGN.md"
            stray.parent.mkdir(parents=True, exist_ok=True)
            stray.write_text(
                "see `world_fold.rs:4242`.\n", encoding="utf-8", newline="\n"
            )
            relative = stray.relative_to(fixture.root).as_posix()
            self._write_allowance(fixture, {}, {relative: [4242]})
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "grandfathered but were not scanned"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_deleted_grandfathered_document_is_not_an_error(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_allowance(
                fixture, {}, {"extractor/documents/GONE.md": [4242]}
            )
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_named_item_citation_in_a_document_needs_no_amnesty(self) -> None:
        # The whole point of the ratchet: the replacement form is free.
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_document(
                fixture,
                "DESIGN.md",
                "the refusal lives in `world_fold.rs::fold_one`.\n",
            )
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_corrupt_allowance_file_fails_rather_than_opens_the_gate(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            path = fixture.root / VERIFY.PROSE_ALLOWANCE
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("{not json", encoding="utf-8", newline="\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "cannot read the prose-reference allowance"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())


if __name__ == "__main__":
    unittest.main()
