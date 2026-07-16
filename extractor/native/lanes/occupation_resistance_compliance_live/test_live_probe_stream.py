import struct
import hashlib
import unittest
from pathlib import Path

import live_probe


class CompactStreamSequenceTests(unittest.TestCase):
    def test_source_identity_binds_the_exact_emitter_bytes_and_build(self):
        identity = live_probe.source_identity()
        source_sha256 = hashlib.sha256(Path(live_probe.__file__).read_bytes()).hexdigest()

        self.assertEqual(identity["schema"], live_probe.SOURCE_IDENTITY_SCHEMA)
        self.assertEqual(identity["emitter_path"], live_probe.SOURCE_IDENTITY_PATH)
        self.assertEqual(identity["emitter_sha256"], source_sha256)
        self.assertEqual(identity["expected_exe_sha256"], live_probe.EXPECTED_SHA256)

    def test_proof_stream_uses_one_based_contiguous_canonical_sequences(self):
        payload = live_probe.encode_payload([], 7, 0, 0)
        stream = live_probe.encode_compact_stream(
            payload,
            [],
            elapsed_wall_ms=11,
            read_time_us=13,
            status_flags=0,
            game_hour_raw=17,
        )

        records = []
        cursor = 0
        while cursor < len(stream):
            (
                magic,
                version,
                header_len,
                sequence,
                game_hour_raw,
                domain_id,
                record_type,
                payload_len,
                record_flags,
            ) = struct.unpack_from("<4sHHQiHHII", stream, cursor)
            records.append(
                (
                    magic,
                    version,
                    header_len,
                    sequence,
                    game_hour_raw,
                    domain_id,
                    record_type,
                    record_flags,
                )
            )
            cursor += header_len + payload_len

        self.assertEqual(cursor, len(stream))
        self.assertEqual([record[3] for record in records], [1, 2, 3])
        self.assertEqual(
            [(record[5], record[6]) for record in records],
            [
                (live_probe.DOMAIN_ID, live_probe.COMPACT_RECORD_TYPE_DOMAIN_PERF),
                (live_probe.DOMAIN_ID, live_probe.COMPACT_RECORD_TYPE_DOMAIN_DATA),
                (live_probe.COMPACT_DOMAIN_FRAME, live_probe.COMPACT_RECORD_TYPE_FRAME_END),
            ],
        )
        self.assertTrue(
            all(record[0] == live_probe.COMPACT_RECORD_MAGIC for record in records)
        )
        self.assertTrue(
            all(record[1] == live_probe.COMPACT_RECORD_VERSION for record in records)
        )
        self.assertTrue(
            all(record[2] == live_probe.COMPACT_RECORD_HEADER_LEN for record in records)
        )
        self.assertTrue(all(record[4] == 17 for record in records))
        self.assertTrue(all(record[7] == 0 for record in records))


if __name__ == "__main__":
    unittest.main()
