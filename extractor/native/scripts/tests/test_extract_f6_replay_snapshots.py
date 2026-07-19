import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import extract_f6_replay_snapshots as extractor


class ExtractSnapshotsTests(unittest.TestCase):
    def make_replay(self, root: Path, *, duplicate=False) -> Path:
        replay = root / "capture.battleplan"
        index = {"game_id": "g", "snapshots": []}
        rows = []
        for hour in (1, 2):
            relative = f"snapshots/{hour:012}.snapshot.json"
            index["snapshots"].append({"hour_index": hour, "file": relative})
            rows.append((f"payload/g/{relative}", json.dumps({"hour_index": hour})))
        with zipfile.ZipFile(replay, "w") as archive:
            archive.writestr("payload/g/index.json", json.dumps(index))
            for name, body in rows:
                archive.writestr(name, body)
            if duplicate:
                archive.writestr(rows[0][0], rows[0][1])
        return replay

    def test_extracts_only_index_authorized_snapshots(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            report = extractor.extract(self.make_replay(root), root / "out")
            self.assertTrue(report["ok"])
            self.assertEqual(report["snapshot_count"], 2)

    def test_duplicate_zip_name_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            with self.assertRaises(extractor.ExtractError):
                extractor.extract(self.make_replay(root, duplicate=True), root / "out")


if __name__ == "__main__":
    unittest.main()
