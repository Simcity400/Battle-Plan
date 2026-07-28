#!/usr/bin/env python3
"""Focused deterministic tests for verify_world_fold_citations.py."""

from __future__ import annotations

import importlib.util
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
                (1, 0),
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
                (2, 0),
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
                (1, 0),
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
                (1, 1),
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


if __name__ == "__main__":
    unittest.main()
