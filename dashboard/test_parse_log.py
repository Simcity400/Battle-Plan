#!/usr/bin/env python3
"""Regression harness for the Battle Plan log parser (Suggested action 7).

Feeds parse_log / build_series / make_payload a corpus of HOSTILE logs - nan/inf,
overflowing literals, a missing tag, a "</script>" tag, a duplicate/renamed
HEADER, a tag of "__proto__", and a bad schema-version field - and asserts the
pipeline neither raises nor emits invalid JSON (no bare NaN/Infinity, nothing that
would break JSON.parse or inject markup).

Run:  python test_parse_log.py
Stdlib only (unittest + tempfile + json); no game, no network, no pip installs.
"""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import battle-plan as mod

HEADER = ("HOI4MPSTAT;v1;HEADER;" + ";".join(mod.DEFAULT_COLUMNS))


def _data_row(tick, tag, fill="0"):
    """A v1 DATA line with the given tick/tag and `fill` for every other column."""
    vals = []
    for col in mod.DEFAULT_COLUMNS:
        if col == "tick":
            vals.append(str(tick))
        elif col == "tag":
            vals.append(str(tag))
        else:
            vals.append(fill)
    return "HOI4MPSTAT;v1;DATA;" + ";".join(vals)


def run_pipeline(log_text):
    """Write `log_text` to a temp file, run the full parse->payload->serialize
    pipeline, and return (rows, payload, json_string). Raises on any failure."""
    fd, path = tempfile.mkstemp(suffix=".log")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(log_text)
        rows = mod.parse_log(path)
        payload = mod.make_payload(rows)
        js = mod.dump_json(payload)            # allow_nan=False -> raises on NaN/Inf
        return rows, payload, js
    finally:
        os.remove(path)


class HostileLogTests(unittest.TestCase):
    def _assert_valid(self, log_text):
        """The pipeline must not raise, and must emit JSON that re-parses and
        contains no NaN/Infinity tokens."""
        rows, payload, js = run_pipeline(log_text)
        reparsed = json.loads(js)              # would raise on invalid JSON
        self.assertIsInstance(reparsed.get("series"), dict)
        self.assertNotIn("NaN", js)
        self.assertNotIn("Infinity", js)
        return rows, payload, js

    def test_clean_baseline(self):
        log = "\n".join([HEADER, _data_row(0, "GER", "5"), _data_row(1, "GER", "9")])
        rows, payload, _ = self._assert_valid(log)
        self.assertEqual(len(rows), 2)
        self.assertIn("GER", payload["series"])

    def test_nan_inf_and_overflow_literals(self):
        # nan/inf/1e9999 in numeric fields must be coerced, not crash int(tick)
        # or leak bare NaN/Infinity into the JSON.
        log = "\n".join([
            HEADER,
            _data_row(0, "GER", "nan"),
            _data_row(1, "GER", "inf"),
            _data_row(2, "GER", "-inf"),
            _data_row(3, "GER", "1e9999"),
        ])
        self._assert_valid(log)

    def test_nan_tick_row_skipped(self):
        # A non-integer tick (nan -> 0.0 -> not parseable as the literal) is skipped.
        bad = _data_row("nan", "GER")
        good = _data_row(5, "GER")
        rows, _, _ = self._assert_valid("\n".join([HEADER, bad, good]))
        ticks = [r["tick"] for r in rows]
        self.assertEqual(ticks, [5])

    def test_missing_tag_field(self):
        # A DATA line with no tag column value: must be skipped, not KeyError.
        line = "HOI4MPSTAT;v1;DATA;0"     # tick only, nothing after
        rows, _, _ = self._assert_valid("\n".join([HEADER, line]))
        self.assertEqual(rows, [])

    def test_script_tag_injection_dropped(self):
        evil = _data_row(0, '"><img src=x onerror=alert(1)>')
        closer = _data_row(1, "</script>")
        rows, _, js = self._assert_valid("\n".join([HEADER, evil, closer]))
        self.assertEqual(rows, [])           # both rejected by the tag regex
        self.assertNotIn("<img", js)
        self.assertNotIn("</script>", js)

    def test_duplicate_header_ignored(self):
        # A second HEADER with duplicate columns must not be adopted (would
        # silently remap/overwrite columns); the good prior columns stay in use.
        dup = "HOI4MPSTAT;v1;HEADER;tick;tag;tag;icout"
        log = "\n".join([HEADER, _data_row(0, "GER", "7"), dup, _data_row(1, "GER", "8")])
        rows, payload, _ = self._assert_valid(log)
        self.assertEqual(len(rows), 2)
        self.assertIn("GER", payload["series"])

    def test_renamed_tick_header_ignored(self):
        # A HEADER missing 'tick' must be rejected so the time axis isn't corrupted.
        renamed = "HOI4MPSTAT;v1;HEADER;turn;tag;icout"
        log = "\n".join([renamed, _data_row(3, "GER", "4")])
        rows, _, _ = self._assert_valid(log)
        # Falls back to DEFAULT_COLUMNS -> 'tick' present and parsed.
        self.assertEqual([r["tick"] for r in rows], [3])

    def test_proto_tag_dropped(self):
        log = "\n".join([HEADER, _data_row(0, "__proto__"), _data_row(1, "GER")])
        rows, payload, _ = self._assert_valid(log)
        self.assertNotIn("__proto__", payload["series"])
        self.assertIn("GER", payload["series"])

    def test_bad_schema_version_skipped(self):
        # A future/unknown schema version must be ignored, not mis-parsed as v1.
        v2 = "HOI4MPSTAT;v2;DATA;0;GER;" + ";".join(["1"] * 30)
        log = "\n".join([HEADER, v2, _data_row(2, "USA")])
        rows, payload, _ = self._assert_valid(log)
        self.assertEqual([r["tag"] for r in rows], ["USA"])   # GER (v2) dropped
        self.assertNotIn("GER", payload["series"])

    def test_no_marker_lines_only(self):
        # Pure engine noise -> empty but valid payload, no crash.
        rows, payload, _ = self._assert_valid("[some.cpp:1]: noise\nmore noise\n")
        self.assertEqual(rows, [])
        self.assertEqual(payload["series"], {})

    def test_missing_file(self):
        # A nonexistent path must yield [] (M8), not raise.
        rows = mod.parse_log(os.path.join(tempfile.gettempdir(), "nope_no_such.log"))
        self.assertEqual(rows, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
