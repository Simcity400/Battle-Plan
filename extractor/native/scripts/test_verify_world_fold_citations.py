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
                (1, 0, 0),
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
                (2, 0, 0),
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
                (1, 0, 0),
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
                (1, 1, 0),
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

    def _write_allowance(self, fixture: CitationFixture, modules: dict) -> None:
        path = fixture.root / VERIFY.PROSE_ALLOWANCE
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps({"modules": modules}), encoding="utf-8", newline="\n"
        )

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
                (1, 0, 1),
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
                (1, 0, 0),
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
                (1, 0, 0),
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
