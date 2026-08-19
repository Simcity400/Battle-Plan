#!/usr/bin/env python3
'''C0 pass 2: per-hour compact status spans and origin reconciliation.

Usage: python c0_pass2_spans.py [run_dir ...]
       python c0_pass2_spans.py --selftest

The walker uses canonical H4CR framing and the uniform DOMAIN_PERF status word.
It exits 1 on malformed evidence or any unreconciled failure-class incidence.
'''

from __future__ import annotations

import hashlib
import contextlib
import io
import json
from pathlib import Path
import struct
import sys
import tempfile
from dataclasses import dataclass, field
from typing import BinaryIO, Dict, Iterable, Mapping, Optional, Sequence, Set, Tuple

H4CR_MAGIC = b'H4CR'
ZSTD_MAGIC = b'\x28\xb5\x2f\xfd'
H4CR_VERSION = 1
H4CR_HEADER_LEN = 32
H4CR_MAX_PAYLOAD_LEN = 1 << 28
PERF_MAX_PAYLOAD_LEN = 1 << 20
OTHER_MAX_PAYLOAD_LEN = 16 << 20
DOMAIN_PERF = 1
FRAME_END = 2
DOMAIN_DATA = 3
PERF_PAYLOAD_LEN = 40
PERF_STATUS_OFFSET = 32
FRAME_DOMAIN_ID = 1000
RC_PROBE_DOMAIN_ID = 250


@dataclass(frozen=True)
class StatusSpec:
    name: str
    failure_class: bool


GLOBAL_STATUS_BITS: Mapping[int, StatusSpec] = {
    0: StatusSpec('STATE_UNAVAILABLE', True),
    1: StatusSpec('PROFILE_MISMATCH', True),
    2: StatusSpec('NO_CLOCK', True),
    3: StatusSpec('NO_SCALAR_SOURCE', True),
    4: StatusSpec('WAR_BRIDGE_APPLIED', False),
    5: StatusSpec('EXTRACTION_PERF_GATE', False),
    6: StatusSpec('WORKER_PANIC', True),
    7: StatusSpec('BRIDGE_UNAVAILABLE', True),
    8: StatusSpec('SOURCE_UNAVAILABLE', True),
    9: StatusSpec('PARTIAL_DATA', True),
    10: StatusSpec('DELTA_DATA', False),
    11: StatusSpec('TRANSITION_BASELINE_RESET', False),
    12: StatusSpec('CONTROL_INVALID', True),
    13: StatusSpec('WAR_SEGMENT_ABSENT', False),
    14: StatusSpec('SOURCE_EMPTY', False),
}

DOMAIN_STATUS_BITS: Mapping[int, Mapping[int, StatusSpec]] = {
    49: {11: StatusSpec('LANE:LOCATION_CANDIDATE_UNVALIDATED', True)},
    51: {
        11: StatusSpec('LANE:FLAG_NAMES_PARTIAL', True),
        12: StatusSpec('LANE:DELAYED_EVENTS_ROOT_UNRESOLVED', True),
        13: StatusSpec('LANE:SCRIPTED_GUI_ROOT_UNRESOLVED', True),
    },
    53: {
        16: StatusSpec('LANE:OPERATIONS_DETAIL_RESISTED', True),
        17: StatusSpec('LANE:INTEL_MAP_RESISTED', True),
        18: StatusSpec('LANE:WAR_ROOT_UNVALIDATED', True),
        19: StatusSpec('LANE:INCOMING_ACTION_QUEUE_UNVALIDATED', True),
        20: StatusSpec('LANE:WAR_LIFECYCLE_DETAIL_UNVALIDATED', True),
        21: StatusSpec('LANE:FACTION_GROUP_UNVALIDATED', True),
        22: StatusSpec('LANE:INTEL_NETWORK_GRAPH_UNVALIDATED', True),
    },
    60: {
        16: StatusSpec('LANE:WAR_CASUALTIES_RUNNING_ACCUMULATOR', False),
        17: StatusSpec('LANE:WAR_RESIDUALS_EXACT', False),
    },
    99: {
        16: StatusSpec('LANE:TORN_ROWS_PRESENT', True),
        17: StatusSpec('LANE:STAT_BLOCK_NULL_PRESENT', True),
        18: StatusSpec('LANE:INCOMPLETE_OBSERVATION', True),
    },
}
LANE_LOCAL_MASKS = {
    domain: sum(1 << bit for bit in bits)
    for domain, bits in DOMAIN_STATUS_BITS.items()
}
DEFAULT_RUN_NAMES = (
    'production_speed5_ShortSpeed5_20260711_113625',
    'production_speed5_ShortSpeed5_20260711_142948',
)

ATTRIBUTION_UNITS: Mapping[Tuple[int, int], Tuple[str, ...]] = {
    (43, 8): ('so_d43_source_empty_warm_known', 'so_d43_source_empty_cold'),
    (45, 9): ('so_d45_partial_targeted_timed_structural',),
    (48, 6): ('so_d48_worker_panic',),
    (48, 9): ('so_d48_partial_inherited', 'so_d48_partial_roots_err',
              'so_d48_partial_hard_or_gate', 'so_d48_worker_panic'),
    (49, 6): ('so_naval_worker_panic',),
    (49, 8): ('so_naval_source_empty',),
    (49, 9): ('so_naval_partial_inherited', 'so_naval_partial_torn_walk',
              'so_naval_partial_join', 'so_naval_worker_panic'),
    (50, 6): ('so_naval_worker_panic',),
    (50, 8): ('so_naval_source_empty',),
    (50, 9): ('so_naval_partial_inherited', 'so_naval_partial_torn_walk',
              'so_naval_partial_join', 'so_naval_worker_panic'),
    (53, 6): ('so_d53_worker_panic',),
    (53, 8): ('so_d53_source_unavailable', 'so_d53_postbuild_gate'),
    (53, 9): ('so_d53_partial_inherited', 'so_d53_partial_row_skipped',
              'so_d53_partial_shadow_mismatch', 'so_d53_postbuild_gate',
              'so_d53_worker_panic'),
    (54, 6): ('so_d54_worker_panic',),
    (54, 8): ('so_d54_hard_dependency', 'so_d54_source_dependency'),
    (54, 9): ('so_d54_partial_inherited', 'so_d54_partial_army_orders',
              'so_d54_partial_oob_command', 'so_d54_hard_dependency',
              'so_d54_partial_skipped_rows', 'so_d54_worker_panic'),
    (55, 11): ('so_d55_transition_baseline_reset',),
    (60, 6): ('so_d60_worker_panic',),
    (60, 8): ('so_d60_source_unavailable',),
    (60, 9): ('so_d60_partial_inherited', 'so_d60_partial_torn_row',
              'so_d60_worker_panic'),
    (65, 8): ('so_d65_source_empty_no_witness',),
    (65, 9): ('so_d65_partial_dropped_ambiguous',),
}


class CensusError(Exception):
    '''Evidence is missing, malformed, truncated, or inconsistent.'''


@dataclass
class Incidence:
    frame_count: int = 0
    hours: Set[int] = field(default_factory=set)


@dataclass(frozen=True)
class Origin:
    value: int
    first_hour: Optional[int]
    last_hour: Optional[int]
    no_clock_observations: int


@dataclass
class StreamScan:
    incidences: Dict[Tuple[int, int], Incidence] = field(default_factory=dict)
    records: int = 0
    physical_records: int = 0
    perf_records: int = 0
    data_records: int = 0
    frame_end_records: int = 0
    diagnostic_records: int = 0
    frame_end_counts: Dict[int, int] = field(default_factory=dict)
    perf_counts: Dict[Tuple[int, int], int] = field(default_factory=dict)
    data_counts: Dict[Tuple[int, int], int] = field(default_factory=dict)
    diagnostic_counts: Dict[int, int] = field(default_factory=dict)
    decoded_bytes: int = 0
    decoded_sha256: str = ''


class TrackingReader:
    def __init__(self, raw: BinaryIO):
        self.raw, self.count, self.digest = raw, 0, hashlib.sha256()

    def read(self, size: int = -1) -> bytes:
        chunk = self.raw.read(size)
        if chunk:
            self.count += len(chunk)
            self.digest.update(chunk)
        return chunk


class StrictZstdReader:
    '''One-frame incremental decoder which requires the physical frame footer.'''

    def __init__(self, stored: BinaryIO, zstandard_module: object):
        self.stored = stored
        self.decoder = zstandard_module.ZstdDecompressor().decompressobj()
        self.buffer = bytearray()
        self.position = 0
        self.done = False

    def read(self, size: int = -1) -> bytes:
        if size < 0:
            size = 1 << 30
        while len(self.buffer) - self.position < size and not self.done:
            chunk = self.stored.read(1 << 20)
            if chunk:
                self.buffer.extend(self.decoder.decompress(chunk))
                if self.decoder.eof:
                    trailing = self.decoder.unused_data + self.stored.read()
                    if trailing:
                        raise CensusError('zstd stream has trailing frame or bytes')
                    self.done = True
            else:
                if not self.decoder.eof:
                    raise CensusError('incomplete zstd frame')
                self.done = True
        end = min(len(self.buffer), self.position + size)
        out = bytes(self.buffer[self.position:end])
        self.position = end
        if self.position >= (1 << 20) and self.position * 2 >= len(self.buffer):
            del self.buffer[:self.position]
            self.position = 0
        return out

    def close(self) -> None:
        self.buffer.clear()
        self.position = 0


def repo_runs_dir() -> Path:
    return Path(__file__).resolve().parents[1] / 'runs'


def default_run_dirs() -> Sequence[Path]:
    return [repo_runs_dir() / name for name in DEFAULT_RUN_NAMES]


def as_uint(value: object, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise CensusError(f'{label} must be a nonnegative integer')
    return value


def as_optional_int(value: object, label: str) -> Optional[int]:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise CensusError(f'{label} must be an integer or null')
    return value


def load_manifest(run_dir: Path) -> Tuple[dict, Path]:
    path = run_dir / 'native_session_manifest.json'
    try:
        with path.open('r', encoding='utf-8') as handle:
            manifest = json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        raise CensusError(f'cannot read {path}: {exc}') from exc
    if not isinstance(manifest, dict):
        raise CensusError(f'{path}: manifest root is not an object')
    return manifest, path


def compact_object(manifest: Mapping[str, object], path: Path) -> dict:
    compact = manifest.get('compact_stream')
    if not isinstance(compact, dict):
        raise CensusError(f'{path}: compact_stream is missing or not an object')
    return compact


def domain_names(manifest: Mapping[str, object], path: Path) -> Dict[int, str]:
    per_domain = compact_object(manifest, path).get('per_domain')
    if not isinstance(per_domain, dict):
        raise CensusError(f'{path}: compact_stream.per_domain is missing or invalid')
    names: Dict[int, str] = {}
    for name, raw in per_domain.items():
        if not isinstance(name, str) or not isinstance(raw, dict):
            raise CensusError(f'{path}: malformed per_domain entry')
        domain_id = as_uint(raw.get('domain_id'), f'{path}: {name}.domain_id')
        if domain_id > 0xFFFF or (domain_id in names and names[domain_id] != name):
            raise CensusError(f'{path}: invalid or duplicate domain id {domain_id}')
        names[domain_id] = name
    return names


def expected_domain_ids(
    manifest: Mapping[str, object], path: Path, names: Mapping[int, str]
) -> Tuple[int, ...]:
    compact = compact_object(manifest, path)
    raw_expected = compact.get('expected_domains')
    if not isinstance(raw_expected, list) or not raw_expected:
        raise CensusError(f'{path}: compact_stream.expected_domains is missing or empty')
    by_name = {name: domain_id for domain_id, name in names.items()}
    expected = []
    for index, name in enumerate(raw_expected):
        if not isinstance(name, str) or not name:
            raise CensusError(f'{path}: expected_domains[{index}] is invalid')
        if name not in by_name:
            raise CensusError(f'{path}: expected domain {name!r} lacks per_domain metadata')
        expected.append(by_name[name])
    if len(set(expected)) != len(expected):
        raise CensusError(f'{path}: compact_stream.expected_domains has duplicates')

    coverage = compact.get('domain_coverage')
    if not isinstance(coverage, dict):
        raise CensusError(f'{path}: compact_stream.domain_coverage is missing or invalid')
    for member in ('ok', 'expected_domain_count', 'errors'):
        if member not in coverage:
            raise CensusError(f'{path}: domain_coverage.{member} is missing')
    ok = coverage['ok']
    errors = coverage['errors']
    count = as_uint(coverage['expected_domain_count'], f'{path}: expected_domain_count')
    if not isinstance(ok, bool) or not isinstance(errors, list) or not all(
        isinstance(error, str) for error in errors
    ):
        raise CensusError(f'{path}: compact_stream.domain_coverage has invalid members')
    if count != len(expected):
        raise CensusError(
            f'{path}: expected_domain_count {count} != expected_domains {len(expected)}'
        )
    if not ok or errors:
        raise CensusError(
            f'{path}: manifest domain_coverage is not clean: ok={ok!r} errors={errors!r}'
        )
    return tuple(expected)


def status_origins(manifest: Mapping[str, object], path: Path) -> Dict[str, Origin]:
    raw_origins = compact_object(manifest, path).get('status_origins')
    if raw_origins is None:
        return {}
    if not isinstance(raw_origins, dict):
        raise CensusError(f'{path}: compact_stream.status_origins is not an object')
    origins: Dict[str, Origin] = {}
    for name, raw in raw_origins.items():
        if not isinstance(name, str) or not name.startswith('so_') or not isinstance(raw, dict):
            raise CensusError(f'{path}: malformed status-origin entry {name!r}')
        for member in ('value', 'first_hour', 'last_hour', 'no_clock_observations'):
            if member not in raw:
                raise CensusError(f'{path}: {name}.{member} is missing')
        value = as_uint(raw.get('value'), f'{path}: {name}.value')
        first = as_optional_int(raw.get('first_hour'), f'{path}: {name}.first_hour')
        last = as_optional_int(raw.get('last_hour'), f'{path}: {name}.last_hour')
        no_clock = as_uint(raw.get('no_clock_observations'), f'{path}: {name}.no_clock')
        if (first is None) != (last is None) or (first is not None and first > last):
            raise CensusError(f'{path}: invalid hour span for {name}')
        if not name.endswith('_sample') and value == 0 and (first is not None or no_clock):
            raise CensusError(f'{path}: zero-valued {name} carries incidence metadata')
        origins[name] = Origin(value, first, last, no_clock)
    return origins


def stream_path(manifest: Mapping[str, object], manifest_path: Path, run_dir: Path) -> Path:
    raw_stream = manifest.get('raw_stream')
    if not isinstance(raw_stream, str) or not raw_stream:
        raise CensusError(f'{manifest_path}: raw_stream is missing or invalid')
    relative = Path(raw_stream)
    if relative.is_absolute() or '..' in relative.parts:
        raise CensusError(f'{manifest_path}: raw_stream escapes the run directory')
    path = run_dir / relative
    if not path.is_file():
        raise CensusError(f'{manifest_path}: raw stream does not exist: {path}')
    return path


def storage_metadata(manifest: Mapping[str, object], path: Path) -> Tuple[str, int, str]:
    storage = manifest.get('stream_storage')
    if not isinstance(storage, dict):
        raise CensusError(f'{path}: stream_storage is missing or invalid')
    compression = storage.get('compression')
    if compression not in ('raw', 'none', 'zstd'):
        raise CensusError(f'{path}: unsupported stream compression {compression!r}')
    decoded_bytes = as_uint(storage.get('decoded_bytes'), f'{path}: decoded_bytes')
    decoded_sha = storage.get('decoded_sha256')
    if not isinstance(decoded_sha, str) or len(decoded_sha) != 64:
        raise CensusError(f'{path}: decoded_sha256 is not 64 characters')
    try:
        bytes.fromhex(decoded_sha)
    except ValueError as exc:
        raise CensusError(f'{path}: decoded_sha256 is not hexadecimal') from exc
    return compression, decoded_bytes, decoded_sha.lower()


def open_decoded(path: Path, compression: str) -> Tuple[BinaryIO, BinaryIO]:
    stored = path.open('rb')
    leading = stored.read(4)
    stored.seek(0)
    detected = 'zstd' if leading == ZSTD_MAGIC else 'raw' if leading == H4CR_MAGIC else 'unknown'
    expected = 'raw' if compression in ('raw', 'none') else compression
    if detected != expected:
        stored.close()
        raise CensusError(f'{path}: metadata says {expected}, bytes detect {detected}')
    if detected == 'raw':
        return stored, stored
    try:
        import zstandard
    except ImportError as exc:
        stored.close()
        raise CensusError(f'{path}: zstd requires the Python zstandard module') from exc
    try:
        decoded = StrictZstdReader(stored, zstandard)
    except Exception as exc:
        stored.close()
        raise CensusError(f'{path}: cannot open zstd decoder: {exc}') from exc
    return decoded, stored


def read_exact(reader: TrackingReader, size: int, label: str) -> bytes:
    out = bytearray()
    while len(out) < size:
        try:
            chunk = reader.read(size - len(out))
        except Exception as exc:
            raise CensusError(f'{label}: decoded read failed: {exc}') from exc
        if not chunk:
            raise CensusError(f'{label}: truncated ({len(out)} of {size} bytes)')
        out.extend(chunk)
    return bytes(out)


def discard_exact(reader: TrackingReader, size: int, label: str) -> None:
    remaining = size
    while remaining:
        remaining -= len(read_exact(reader, min(remaining, 1 << 20), label))


def validate_stream_coverage(
    path: Path, scan: StreamScan, expected_domains: Sequence[int]
) -> None:
    frame_hours = set(scan.frame_end_counts)
    all_hours = frame_hours | {hour for hour, _ in scan.perf_counts}
    all_hours |= {hour for hour, _ in scan.data_counts}
    all_hours |= set(scan.diagnostic_counts)
    if not frame_hours:
        raise CensusError(f'{path}: stream contains no FRAME_END records')
    for hour in sorted(all_hours):
        frame_count = scan.frame_end_counts.get(hour, 0)
        if frame_count != 1:
            raise CensusError(f'{path}: hour {hour} FRAME_END count {frame_count} != 1')
    for hour in sorted(frame_hours):
        for domain_id in expected_domains:
            perf_count = scan.perf_counts.get((hour, domain_id), 0)
            if perf_count != 1:
                raise CensusError(
                    f'{path}: hour {hour} d{domain_id} DOMAIN_PERF count '
                    f'{perf_count} != 1'
                )
        diagnostic_count = scan.diagnostic_counts.get(hour, 0)
        if diagnostic_count > 1:
            raise CensusError(
                f'{path}: hour {hour} d{RC_PROBE_DOMAIN_ID} diagnostic PERF '
                f'count {diagnostic_count} > 1'
            )


def validate_manifest_counts(
    manifest: Mapping[str, object], manifest_path: Path,
    names: Mapping[int, str], scan: StreamScan,
) -> None:
    compact = compact_object(manifest, manifest_path)
    comparisons = (
        ('record_count', scan.records),
        ('domain_perf_record_count', scan.perf_records),
        ('domain_data_record_count', scan.data_records),
        ('frame_end_record_count', scan.frame_end_records),
    )
    for member, actual in comparisons:
        if member not in compact:
            raise CensusError(f'{manifest_path}: compact_stream.{member} is missing')
        expected = as_uint(compact[member], f'{manifest_path}: {member}')
        if actual != expected:
            raise CensusError(
                f'{manifest_path}: stream {member} {actual} != manifest {expected}'
            )
    per_domain = compact.get('per_domain')
    assert isinstance(per_domain, dict)
    for domain_id, name in names.items():
        raw = per_domain[name]
        assert isinstance(raw, dict)
        perf_expected = as_uint(
            raw.get('record_count'), f'{manifest_path}: {name}.record_count'
        )
        data_expected = as_uint(
            raw.get('data_record_count'), f'{manifest_path}: {name}.data_record_count'
        )
        perf_actual = sum(
            count for (hour, candidate), count in scan.perf_counts.items()
            if candidate == domain_id
        )
        data_actual = sum(
            count for (hour, candidate), count in scan.data_counts.items()
            if candidate == domain_id
        )
        if perf_actual != perf_expected or data_actual != data_expected:
            raise CensusError(
                f'{manifest_path}: d{domain_id} counts perf/data '
                f'{perf_actual}/{data_actual} != manifest '
                f'{perf_expected}/{data_expected}'
            )


def scan_stream(
    path: Path, manifest: Mapping[str, object], manifest_path: Path,
    expected_domains: Sequence[int], names: Mapping[int, str],
) -> StreamScan:
    compression, expected_bytes, expected_sha = storage_metadata(manifest, manifest_path)
    expected_set = set(expected_domains)
    decoded, stored = open_decoded(path, compression)
    reader, scan, previous, offset = TrackingReader(decoded), StreamScan(), None, 0
    open_hour: Optional[int] = None
    finalized_hours: Set[int] = set()
    try:
        while True:
            try:
                header = reader.read(H4CR_HEADER_LEN)
            except Exception as exc:
                raise CensusError(f'{path}: decoded read failed at {offset}: {exc}') from exc
            if not header:
                break
            if len(header) != H4CR_HEADER_LEN:
                header += read_exact(
                    reader, H4CR_HEADER_LEN - len(header),
                    f'{path}: header at offset {offset}',
                )
            fields = struct.unpack('<4sHHQiHHII', header)
            magic, version, header_len, sequence, hour = fields[:5]
            domain_id, record_type, payload_len = fields[5:8]
            if magic != H4CR_MAGIC:
                raise CensusError(f'{path}: invalid H4CR magic at offset {offset}')
            if version != H4CR_VERSION or header_len != H4CR_HEADER_LEN:
                raise CensusError(f'{path}: unsupported H4CR header at offset {offset}')
            if payload_len > H4CR_MAX_PAYLOAD_LEN:
                raise CensusError(f'{path}: payload {payload_len} exceeds bound at {offset}')
            if previous is not None and sequence != ((previous + 1) & 0xFFFFFFFFFFFFFFFF):
                raise CensusError(
                    f'{path}: sequence discontinuity at offset {offset}: '
                    f'{previous} -> {sequence}'
                )
            previous = sequence
            scan.physical_records += 1

            if record_type == FRAME_END and payload_len != 32:
                raise CensusError(
                    f'{path}: FRAME_END payload length {payload_len} != 32 at {offset}'
                )
            if record_type == DOMAIN_PERF and payload_len > PERF_MAX_PAYLOAD_LEN:
                raise CensusError(
                    f'{path}: DOMAIN_PERF payload {payload_len} exceeds '
                    f'{PERF_MAX_PAYLOAD_LEN} at {offset}'
                )
            if record_type not in (DOMAIN_PERF, DOMAIN_DATA, FRAME_END) \
                    and payload_len > OTHER_MAX_PAYLOAD_LEN:
                raise CensusError(
                    f'{path}: record type {record_type} payload {payload_len} exceeds '
                    f'{OTHER_MAX_PAYLOAD_LEN} at {offset}'
                )

            if record_type == DOMAIN_DATA:
                if open_hour is not None and open_hour != hour:
                    raise CensusError(
                        f'{path}: hour interleaving at offset {offset}: DOMAIN_DATA '
                        f'for hour {hour} while hour {open_hour} is still open'
                    )
                open_hour = hour
            elif record_type == FRAME_END and open_hour is not None:
                if open_hour != hour:
                    raise CensusError(
                        f'{path}: mismatched FRAME_END at offset {offset}: closes '
                        f'hour {hour} while hour {open_hour} is open'
                    )
                open_hour = None

            if record_type in (DOMAIN_PERF, DOMAIN_DATA) and hour in finalized_hours:
                raise CensusError(f'{path}: late record after FRAME_END for hour {hour}')
            if record_type == DOMAIN_PERF and domain_id == RC_PROBE_DOMAIN_ID:
                if payload_len < PERF_PAYLOAD_LEN:
                    raise CensusError(f'{path}: short diagnostic DOMAIN_PERF at {offset}')
                discard_exact(reader, payload_len, f'{path}: diagnostic perf at {offset}')
                scan.diagnostic_records += 1
                scan.diagnostic_counts[hour] = scan.diagnostic_counts.get(hour, 0) + 1
            elif record_type == DOMAIN_PERF:
                if domain_id not in expected_set:
                    raise CensusError(f'{path}: unexpected DOMAIN_PERF domain d{domain_id}')
                if payload_len < PERF_PAYLOAD_LEN:
                    raise CensusError(f'{path}: short DOMAIN_PERF payload at {offset}')
                prefix = read_exact(reader, PERF_PAYLOAD_LEN, f'{path}: perf at {offset}')
                status = struct.unpack_from('<I', prefix, PERF_STATUS_OFFSET)[0]
                for bit, _ in status_bits(domain_id, status):
                    row = scan.incidences.setdefault((domain_id, bit), Incidence())
                    row.frame_count += 1
                    row.hours.add(hour)
                discard_exact(
                    reader, payload_len - PERF_PAYLOAD_LEN,
                    f'{path}: perf trailer at {offset}',
                )
                key = (hour, domain_id)
                scan.perf_counts[key] = scan.perf_counts.get(key, 0) + 1
                scan.perf_records += 1
                scan.records += 1
            elif record_type == DOMAIN_DATA:
                if domain_id not in expected_set:
                    raise CensusError(f'{path}: unexpected DOMAIN_DATA domain d{domain_id}')
                discard_exact(reader, payload_len, f'{path}: data at {offset}')
                key = (hour, domain_id)
                scan.data_counts[key] = scan.data_counts.get(key, 0) + 1
                scan.data_records += 1
                scan.records += 1
            elif record_type == FRAME_END:
                if domain_id != FRAME_DOMAIN_ID:
                    raise CensusError(f'{path}: FRAME_END has unexpected domain d{domain_id}')
                discard_exact(reader, payload_len, f'{path}: frame end at {offset}')
                scan.frame_end_counts[hour] = scan.frame_end_counts.get(hour, 0) + 1
                scan.frame_end_records += 1
                scan.records += 1
                finalized_hours.add(hour)
            else:
                raise CensusError(f'{path}: unexpected record type {record_type} at {offset}')
            offset += H4CR_HEADER_LEN + payload_len
    finally:
        try:
            decoded.close()
        finally:
            if stored is not decoded:
                stored.close()
    if open_hour is not None:
        raise CensusError(f'{path}: game hour {open_hour} left open at EOF')
    if scan.frame_end_records == 0:
        raise CensusError(f'{path}: stream contains no FRAME_END records')
    scan.decoded_bytes = reader.count
    scan.decoded_sha256 = reader.digest.hexdigest()
    if scan.decoded_bytes != expected_bytes:
        raise CensusError(
            f'{path}: decoded bytes {scan.decoded_bytes} != manifest {expected_bytes}'
        )
    if scan.decoded_sha256 != expected_sha:
        raise CensusError(
            f'{path}: decoded sha256 {scan.decoded_sha256} != manifest {expected_sha}'
        )
    validate_stream_coverage(path, scan, expected_domains)
    validate_manifest_counts(manifest, manifest_path, names, scan)
    return scan


def spans(hours: Iterable[int]) -> Sequence[Tuple[int, int]]:
    ordered = sorted(set(hours))
    if not ordered:
        return []
    out, first, previous = [], ordered[0], ordered[0]
    for hour in ordered[1:]:
        if hour == previous + 1:
            previous = hour
        else:
            out.append((first, previous))
            first = previous = hour
    out.append((first, previous))
    return out


def format_spans(hours: Iterable[int]) -> str:
    return ','.join(
        str(first) if first == last else f'{first}-{last}'
        for first, last in spans(hours)
    ) or '-'


def status_spec(domain_id: int, bit: int) -> StatusSpec:
    domain_bits = DOMAIN_STATUS_BITS.get(domain_id, {})
    if bit in domain_bits:
        return domain_bits[bit]
    return GLOBAL_STATUS_BITS.get(bit, StatusSpec(f'UNKNOWN_BIT_{bit}', True))


def status_bits(domain_id: int, status: int) -> Iterable[Tuple[int, StatusSpec]]:
    local_mask = LANE_LOCAL_MASKS.get(domain_id, 0)
    domain_bits = DOMAIN_STATUS_BITS.get(domain_id, {})
    for bit in range(32):
        if status & local_mask & (1 << bit):
            yield bit, domain_bits[bit]
    global_status = status & ~local_mask
    for bit in range(32):
        if global_status & (1 << bit):
            yield bit, status_spec(domain_id, bit)


def candidate_units(domain_id: int, bit: int) -> Tuple[str, ...]:
    if bit in DOMAIN_STATUS_BITS.get(domain_id, {}):
        return ATTRIBUTION_UNITS.get((domain_id, bit), ())
    if bit == 12:
        return ('so_frame_control_invalid',)
    return ATTRIBUTION_UNITS.get((domain_id, bit), ())


def reconcile(
    incidence: Incidence,
    candidates: Sequence[str],
    origins: Mapping[str, Origin],
) -> Tuple[str, str]:
    positive = [
        (name, origins[name]) for name in candidates
        if name in origins and origins[name].value
    ]
    if not positive:
        return 'NONE', 'no positive candidate status-origin counter'
    status_hours, proven_hours = incidence.hours, set()
    total = sum(origin.value for _, origin in positive)
    no_clock = sum(origin.no_clock_observations for _, origin in positive)
    overlap = False
    for _, origin in positive:
        if origin.first_hour is None or origin.last_hour is None:
            continue
        proven_hours.update((origin.first_hour, origin.last_hour))
        overlap |= any(
            origin.first_hour <= hour <= origin.last_hour for hour in status_hours
        )
    exact = (
        status_hours <= proven_hours
        and total == incidence.frame_count
        and no_clock == 0
        and proven_hours <= status_hours
    )
    if exact:
        return 'ALL', 'candidate counts and proven hours exactly match status frames'
    if overlap or status_hours.intersection(proven_hours):
        return 'PARTIAL', 'span overlaps but aggregate ledger cannot prove every frame'
    return 'NONE', 'candidate counters are disjoint from status hours'


def render_run(run_dir: Path) -> Tuple[str, bool]:
    manifest, manifest_path = load_manifest(run_dir)
    names = domain_names(manifest, manifest_path)
    expected = expected_domain_ids(manifest, manifest_path, names)
    origins = status_origins(manifest, manifest_path)
    path = stream_path(manifest, manifest_path, run_dir)
    scan = scan_stream(path, manifest, manifest_path, expected, names)
    session = manifest.get('session_id')
    if not isinstance(session, str) or not session:
        session = run_dir.name
    lines = [
        f'RUN {run_dir.name}',
        f'  session={session}',
        f'  stream={path.name} records={scan.records} perf_records={scan.perf_records} '
        f'data_records={scan.data_records} frame_end_records={scan.frame_end_records}',
        f'  physical_records={scan.physical_records} '
        f'reserved_diagnostic_records={scan.diagnostic_records}',
        f'  decoded_bytes={scan.decoded_bytes} decoded_sha256={scan.decoded_sha256}',
        '  STATUS INCIDENCE',
    ]
    for (domain_id, bit), incidence in sorted(scan.incidences.items()):
        name = names.get(domain_id, f'UNKNOWN_DOMAIN_{domain_id}')
        bit_name = status_spec(domain_id, bit).name
        ordered = sorted(incidence.hours)
        lines.append(
            f'    d{domain_id} {name} bit{bit} {bit_name}: '
            f'first={ordered[0]} last={ordered[-1]} '
            f'frames={incidence.frame_count} hours={len(ordered)} '
            f'spans={format_spans(ordered)}'
        )
    lines.append('  STATUS ORIGINS')
    if not origins:
        lines.append(
            '    NONE RECORDED (compact_stream.status_origins missing or empty)'
        )
    else:
        for name, origin in sorted(origins.items()):
            kind = 'sample' if name.endswith('_sample') else 'counter'
            first = 'null' if origin.first_hour is None else str(origin.first_hour)
            last = 'null' if origin.last_hour is None else str(origin.last_hour)
            lines.append(
                f'    {name}: kind={kind} value={origin.value} '
                f'first={first} last={last} '
                f'no_clock_observations={origin.no_clock_observations}'
            )
    lines.append('  RECONCILIATION')
    reconciled, saw_failure = True, False
    for (domain_id, bit), incidence in sorted(scan.incidences.items()):
        spec = status_spec(domain_id, bit)
        if not spec.failure_class:
            continue
        saw_failure = True
        candidates = candidate_units(domain_id, bit)
        verdict, reason = reconcile(incidence, candidates, origins)
        reconciled &= verdict == 'ALL'
        name = names.get(domain_id, f'UNKNOWN_DOMAIN_{domain_id}')
        bit_name = spec.name
        candidate_text = ','.join(candidates) if candidates else '<no registered candidate>'
        lines.append(
            f'    d{domain_id} {name} bit{bit} {bit_name}: {verdict}; '
            f'candidates={candidate_text}; {reason}'
        )
    if not saw_failure:
        lines.append('    NO FAILURE-CLASS INCIDENCE')
    lines.append(f'''  RUN VERDICT: {'RECONCILED' if reconciled else 'UNRECONCILED'}''')
    return '\n'.join(lines), reconciled


def run_paths(paths: Sequence[Path]) -> int:
    if not paths:
        raise CensusError('no run directories selected')
    outputs, all_reconciled = [], True
    for path in paths:
        if not path.is_dir():
            raise CensusError(f'run directory does not exist: {path}')
        output, reconciled = render_run(path)
        outputs.append(output)
        all_reconciled &= reconciled
    print('C0 PASS 2 SPANS')
    print('\n\n'.join(outputs))
    print()
    print(f'''OVERALL VERDICT: {'RECONCILED' if all_reconciled else 'UNRECONCILED'}''')
    return 0 if all_reconciled else 1


def fixture_record(
    sequence: int, hour: int, domain_id: int, record_type: int, payload: bytes
) -> bytes:
    return struct.pack(
        '<4sHHQiHHII', H4CR_MAGIC, H4CR_VERSION, H4CR_HEADER_LEN,
        sequence, hour, domain_id, record_type, len(payload), 0,
    ) + payload


def fixture_perf(sequence: int, hour: int, domain_id: int, status: int = 0) -> bytes:
    payload = bytearray(PERF_PAYLOAD_LEN)
    struct.pack_into('<I', payload, PERF_STATUS_OFFSET, status)
    return fixture_record(sequence, hour, domain_id, DOMAIN_PERF, bytes(payload))


def fixture_data(sequence: int, hour: int, domain_id: int) -> bytes:
    return fixture_record(sequence, hour, domain_id, DOMAIN_DATA, b'')


def fixture_frame(sequence: int, hour: int) -> bytes:
    return fixture_record(sequence, hour, FRAME_DOMAIN_ID, FRAME_END, bytes(32))


FIXTURE_DOMAINS = {
    45: 'focus_research_decisions_depth_live',
    49: 'naval_combat_lifecycle_live',
    51: 'scripted_state_flags_variables_live',
    53: 'diplomacy_intel_operatives_live',
    60: 'capitulation_war_state_live',
}


def fixture_manifest(
    stream: bytes, compression: str, origins: Mapping[str, dict],
    expected_domains: Sequence[int] = (45,), coverage_ok: bool = True,
    coverage_errors: Sequence[str] = (),
) -> dict:
    perf_counts: Dict[int, int] = {domain_id: 0 for domain_id in expected_domains}
    data_counts: Dict[int, int] = {domain_id: 0 for domain_id in expected_domains}
    records = perf_records = data_records = frame_records = 0
    offset = 0
    while offset < len(stream):
        if len(stream) - offset < H4CR_HEADER_LEN:
            break
        fields = struct.unpack_from('<4sHHQiHHII', stream, offset)
        domain_id, record_type, payload_len = fields[5:8]
        if not (record_type == DOMAIN_PERF and domain_id == RC_PROBE_DOMAIN_ID):
            records += 1
            if record_type == DOMAIN_PERF:
                perf_records += 1
                if domain_id in perf_counts:
                    perf_counts[domain_id] += 1
            elif record_type == DOMAIN_DATA:
                data_records += 1
                if domain_id in data_counts:
                    data_counts[domain_id] += 1
            elif record_type == FRAME_END:
                frame_records += 1
        offset += H4CR_HEADER_LEN + payload_len
    per_domain = {}
    expected_names = []
    for domain_id in expected_domains:
        name = FIXTURE_DOMAINS[domain_id]
        expected_names.append(name)
        per_domain[name] = {
            'domain_id': domain_id,
            'record_count': perf_counts[domain_id],
            'data_record_count': data_counts[domain_id],
        }
    return {
        'session_id': 'synthetic',
        'raw_stream': 'collector_stream.bin',
        'stream_storage': {
            'compression': compression,
            'decoded_bytes': len(stream),
            'decoded_sha256': hashlib.sha256(stream).hexdigest(),
        },
        'compact_stream': {
            'record_count': records,
            'domain_perf_record_count': perf_records,
            'domain_data_record_count': data_records,
            'frame_end_record_count': frame_records,
            'expected_domains': expected_names,
            'domain_coverage': {
                'ok': coverage_ok,
                'expected_domain_count': len(expected_domains),
                'errors': list(coverage_errors),
            },
            'per_domain': per_domain,
            'status_origins': dict(origins),
        },
    }


def write_fixture(run: Path, stream: bytes, manifest: Mapping[str, object]) -> Path:
    raw = run / 'collector_stream.bin'
    raw.write_bytes(stream)
    path = run / 'native_session_manifest.json'
    path.write_text(json.dumps(manifest), encoding='utf-8')
    return path


def scan_fixture(run: Path) -> StreamScan:
    manifest, manifest_path = load_manifest(run)
    names = domain_names(manifest, manifest_path)
    expected = expected_domain_ids(manifest, manifest_path, names)
    status_origins(manifest, manifest_path)
    return scan_stream(
        stream_path(manifest, manifest_path, run), manifest, manifest_path,
        expected, names,
    )


def expect_census_error(action: object, needle: str) -> None:
    try:
        action()
    except CensusError as exc:
        assert needle in str(exc), (needle, str(exc))
    else:
        raise AssertionError(f'expected CensusError containing {needle!r}')


def selftest() -> int:
    tests = 0
    valid = b''.join((
        fixture_perf(1, 10, 45, 1 << 9),
        fixture_data(2, 10, 45),
        fixture_frame(3, 10),
    ))
    counter_origin = {
        'so_d45_partial_targeted_timed_structural': {
            'value': 1, 'first_hour': 10, 'last_hour': 10,
            'no_clock_observations': 0,
        },
    }
    with tempfile.TemporaryDirectory(prefix='c0_pass2_') as tmp:
        run = Path(tmp)
        manifest = fixture_manifest(valid, 'raw', counter_origin)
        manifest_path = write_fixture(run, valid, manifest)
        scan = scan_fixture(run)
        ledger = status_origins(manifest, manifest_path)
        assert scan.incidences[(45, 9)].frame_count == 1
        assert scan.records == 3 and scan.physical_records == 3
        assert reconcile(
            scan.incidences[(45, 9)], candidate_units(45, 9), ledger
        )[0] == 'ALL'
        tests += 1

        d51 = b''.join((
            fixture_perf(1, 20, 51, (1 << 11) | (1 << 12)),
            fixture_data(2, 20, 51), fixture_frame(3, 20),
        ))
        write_fixture(run, d51, fixture_manifest(d51, 'raw', {}, (51,)))
        scan = scan_fixture(run)
        assert status_spec(51, 11).name == 'LANE:FLAG_NAMES_PARTIAL'
        assert status_spec(51, 12).name == 'LANE:DELAYED_EVENTS_ROOT_UNRESOLVED'
        assert status_spec(51, 12).failure_class
        assert candidate_units(51, 12) == ()
        assert candidate_units(45, 12) == ('so_frame_control_invalid',)
        assert set(scan.incidences) == {(51, 11), (51, 12)}
        tests += 1

        d60 = b''.join((
            fixture_perf(1, 30, 60, 1 << 16),
            fixture_data(2, 30, 60), fixture_frame(3, 30),
        ))
        write_fixture(run, d60, fixture_manifest(d60, 'raw', {}, (60,)))
        output, reconciled = render_run(run)
        badge = 'LANE:WAR_CASUALTIES_RUNNING_ACCUMULATOR'
        assert reconciled and output.count(badge) == 1
        assert not status_spec(60, 16).failure_class
        tests += 1

        d53 = b''.join((
            fixture_perf(1, 40, 53, (1 << 16) | (1 << 22)),
            fixture_data(2, 40, 53), fixture_frame(3, 40),
        ))
        write_fixture(run, d53, fixture_manifest(d53, 'raw', {}, (53,)))
        scan_fixture(run)
        assert status_spec(53, 16).name == 'LANE:OPERATIONS_DETAIL_RESISTED'
        assert status_spec(53, 22).failure_class
        tests += 1

        missing_member = {
            'so_d45_partial_targeted_timed_structural': {
                'value': 1, 'first_hour': 10, 'last_hour': 10,
            },
        }
        malformed = fixture_manifest(valid, 'raw', missing_member)
        write_fixture(run, valid, malformed)
        expect_census_error(lambda: scan_fixture(run), 'no_clock_observations is missing')
        tests += 1

        zero_sample = {
            'so_d45_partial_targeted_timed_structural_sample': {
                'value': 0, 'first_hour': 10, 'last_hour': 10,
                'no_clock_observations': 0,
            },
        }
        sample_manifest = fixture_manifest(valid, 'raw', zero_sample)
        write_fixture(run, valid, sample_manifest)
        sample = status_origins(sample_manifest, manifest_path)
        assert sample['so_d45_partial_targeted_timed_structural_sample'].value == 0
        assert sample['so_d45_partial_targeted_timed_structural_sample'].first_hour == 10
        tests += 1

        zero_counter = {
            'so_d45_partial_targeted_timed_structural': {
                'value': 0, 'first_hour': 10, 'last_hour': 10,
                'no_clock_observations': 0,
            },
        }
        bad_counter = fixture_manifest(valid, 'raw', zero_counter)
        expect_census_error(
            lambda: status_origins(bad_counter, manifest_path),
            'zero-valued',
        )
        tests += 1

        zero_perf = b''.join((fixture_data(1, 50, 45), fixture_frame(2, 50)))
        write_fixture(run, zero_perf, fixture_manifest(zero_perf, 'raw', {}, (45,)))
        expect_census_error(lambda: scan_fixture(run), 'DOMAIN_PERF count 0 != 1')
        tests += 1

        missing_domain = b''.join((
            fixture_perf(1, 60, 45), fixture_data(2, 60, 45),
            fixture_frame(3, 60),
        ))
        write_fixture(
            run, missing_domain,
            fixture_manifest(missing_domain, 'raw', {}, (45, 53)),
        )
        expect_census_error(lambda: scan_fixture(run), 'd53 DOMAIN_PERF count 0 != 1')
        tests += 1

        duplicate_perf = b''.join((
            fixture_perf(1, 70, 45), fixture_perf(2, 70, 45),
            fixture_data(3, 70, 45), fixture_frame(4, 70),
        ))
        write_fixture(
            run, duplicate_perf, fixture_manifest(duplicate_perf, 'raw', {}),
        )
        expect_census_error(lambda: scan_fixture(run), 'DOMAIN_PERF count 2 != 1')
        tests += 1

        missing_frame = fixture_perf(1, 80, 45)
        write_fixture(run, missing_frame, fixture_manifest(missing_frame, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'no FRAME_END records')
        tests += 1

        unexpected_domain = b''.join((
            fixture_perf(1, 90, 999), fixture_frame(2, 90),
        ))
        write_fixture(
            run, unexpected_domain, fixture_manifest(unexpected_domain, 'raw', {}),
        )
        expect_census_error(lambda: scan_fixture(run), 'unexpected DOMAIN_PERF domain d999')
        tests += 1

        unexpected_type = b''.join((
            fixture_record(1, 100, 45, 99, b''), fixture_frame(2, 100),
        ))
        write_fixture(
            run, unexpected_type, fixture_manifest(unexpected_type, 'raw', {}),
        )
        expect_census_error(lambda: scan_fixture(run), 'unexpected record type 99')
        tests += 1

        late_record = b''.join((
            fixture_perf(1, 110, 45), fixture_data(2, 110, 45),
            fixture_frame(3, 110), fixture_data(4, 110, 45),
        ))
        write_fixture(run, late_record, fixture_manifest(late_record, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'late record after FRAME_END')
        tests += 1

        sequence_gap = b''.join((
            fixture_perf(1, 120, 45), fixture_data(3, 120, 45),
            fixture_frame(4, 120),
        ))
        write_fixture(run, sequence_gap, fixture_manifest(sequence_gap, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'sequence discontinuity')
        tests += 1

        wrapping_sequence = b''.join((
            fixture_perf(0xFFFFFFFFFFFFFFFE, 121, 45),
            fixture_data(0xFFFFFFFFFFFFFFFF, 121, 45),
            fixture_frame(0, 121),
        ))
        write_fixture(
            run, wrapping_sequence, fixture_manifest(wrapping_sequence, 'raw', {}),
        )
        scan = scan_fixture(run)
        assert scan.records == 3 and scan.physical_records == 3
        tests += 1

        oversized_frame = b''.join((
            fixture_perf(1, 121, 45), fixture_data(2, 121, 45),
            fixture_record(3, 121, FRAME_DOMAIN_ID, FRAME_END, bytes(33)),
        ))
        write_fixture(run, oversized_frame, fixture_manifest(oversized_frame, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'FRAME_END payload length 33 != 32')
        tests += 1

        undersized_frame = b''.join((
            fixture_perf(1, 121, 45), fixture_data(2, 121, 45),
            fixture_record(3, 121, FRAME_DOMAIN_ID, FRAME_END, bytes(31)),
        ))
        write_fixture(run, undersized_frame, fixture_manifest(undersized_frame, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'FRAME_END payload length 31 != 32')
        tests += 1

        interleaved = b''.join((
            fixture_perf(1, 122, 45), fixture_data(2, 122, 45),
            fixture_perf(3, 123, 45), fixture_data(4, 123, 45),
            fixture_frame(5, 123), fixture_frame(6, 122),
        ))
        write_fixture(run, interleaved, fixture_manifest(interleaved, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'hour interleaving')
        tests += 1

        mismatched_frame = b''.join((
            fixture_perf(1, 124, 45), fixture_data(2, 124, 45),
            fixture_perf(3, 125, 45), fixture_frame(4, 125),
            fixture_frame(5, 124),
        ))
        write_fixture(
            run, mismatched_frame, fixture_manifest(mismatched_frame, 'raw', {}),
        )
        expect_census_error(lambda: scan_fixture(run), 'mismatched FRAME_END')
        tests += 1

        open_at_eof = b''.join((
            fixture_perf(1, 126, 45), fixture_data(2, 126, 45),
        ))
        write_fixture(run, open_at_eof, fixture_manifest(open_at_eof, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'left open at EOF')
        tests += 1

        oversized_perf = b''.join((
            fixture_record(1, 127, 45, DOMAIN_PERF, bytes((1 << 20) + 1)),
            fixture_data(2, 127, 45), fixture_frame(3, 127),
        ))
        write_fixture(run, oversized_perf, fixture_manifest(oversized_perf, 'raw', {}))
        expect_census_error(lambda: scan_fixture(run), 'DOMAIN_PERF payload')
        tests += 1

        broken = valid[:-1]
        write_fixture(run, broken, fixture_manifest(broken, 'raw', counter_origin))
        expect_census_error(lambda: scan_fixture(run), 'truncated')
        tests += 1

        try:
            import zstandard
        except ImportError:
            pass
        else:
            compressed = run / 'compressed.bin'
            compressed.write_bytes(
                zstandard.ZstdCompressor(write_checksum=True).compress(valid)
            )
            zmanifest = fixture_manifest(valid, 'zstd', counter_origin)
            names = domain_names(zmanifest, manifest_path)
            expected = expected_domain_ids(zmanifest, manifest_path, names)
            zscan = scan_stream(compressed, zmanifest, manifest_path, expected, names)
            assert zscan.incidences[(45, 9)].frame_count == 1
            tests += 1
            torn = run / 'torn_zstd.bin'
            torn.write_bytes(compressed.read_bytes()[:-1])
            expect_census_error(
                lambda: scan_stream(torn, zmanifest, manifest_path, expected, names),
                'decoded read failed',
            )
            tests += 1

        write_fixture(run, d60, fixture_manifest(d60, 'raw', {}, (60,)))
        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            assert run_paths((run,)) == 0
        captured.getvalue().encode('ascii')
        tests += 1

        write_fixture(run, valid, fixture_manifest(valid, 'raw', {}))
        with contextlib.redirect_stdout(io.StringIO()):
            assert run_paths((run,)) == 1
        tests += 1
    print(f'SELFTEST PASS: {tests} tests')
    return 0


def main(argv: Sequence[str]) -> int:
    if list(argv) == ['--selftest']:
        return selftest()
    if '--selftest' in argv:
        raise CensusError('--selftest cannot be combined with run directories')
    for arg in argv:
        if arg.startswith('-'):
            raise CensusError(f'unknown option: {arg}')
    paths = [Path(arg).resolve() for arg in argv] if argv else list(default_run_dirs())
    return run_paths(paths)


if __name__ == '__main__':
    try:
        raise SystemExit(main(sys.argv[1:]))
    except CensusError as exc:
        print(f'ERROR: {exc}', file=sys.stderr)
        raise SystemExit(1)
