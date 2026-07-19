import argparse
import hashlib
import json
import struct
from pathlib import Path


if not __debug__:
    raise SystemExit(
        "verify_oracle.py: refusing to run with assertions disabled"
        " (python -O / PYTHONOPTIMIZE); rerun without optimization"
    )


HERE = Path(__file__).resolve().parent
DESCRIPTOR_FIELDS = (
    "stage", "slot", "declared_name", "is_null", "view_desc", "dxgi_format",
    "width", "height", "mip_count", "array_size", "sample_count",
    "sample_quality", "bind_flags", "usage", "cpu_access_flags", "misc_flags",
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fnv1a64(data: bytes) -> str:
    value = 0xCBF29CE484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def descriptor(resource: dict) -> dict:
    return {field: resource.get(field) for field in DESCRIPTOR_FIELDS}


def bmp_rgbx(data: bytes, width: int, height: int) -> bytes:
    if data[:2] != b"BM":
        raise AssertionError("HeightNormal source is not BMP")
    offset = struct.unpack_from("<I", data, 10)[0]
    source_width, source_height = struct.unpack_from("<ii", data, 18)
    bpp = struct.unpack_from("<H", data, 28)[0]
    compression = struct.unpack_from("<I", data, 30)[0]
    assert source_width == width and abs(source_height) == height
    assert bpp in (24, 32) and compression == 0
    pixel_bytes = bpp // 8
    stride = (width * pixel_bytes + 3) & ~3
    output = bytearray()
    for y in range(height):
        source_y = height - 1 - y if source_height > 0 else y
        row = offset + source_y * stride
        for x in range(width):
            start = row + x * pixel_bytes
            b, g, r = data[start:start + 3]
            output.extend((b, g, r, 0))
    return bytes(output)


def pack_bc3_dds(data: bytes, width: int, height: int, mip_count: int):
    assert data[:4] == b"DDS " and data[84:88] == b"DXT5"
    assert struct.unpack_from("<I", data, 12)[0] == height
    assert struct.unpack_from("<I", data, 16)[0] == width
    assert max(1, struct.unpack_from("<I", data, 28)[0]) == mip_count
    source = memoryview(data)[128:]
    source_offset = 0
    output = bytearray()
    subresources = []
    for mip in range(mip_count):
        mip_width = max(1, width >> mip)
        mip_height = max(1, height >> mip)
        compact_pitch = max(1, (mip_width + 3) // 4) * 16
        row_pitch = max(128, compact_pitch)
        block_rows = max(1, (mip_height + 3) // 4)
        file_offset = len(output)
        for _ in range(block_rows):
            end = source_offset + compact_pitch
            assert end <= len(source), "truncated BC3 source"
            output.extend(source[source_offset:end])
            output.extend(bytes(row_pitch - compact_pitch))
            source_offset = end
        byte_len = len(output) - file_offset
        subresources.append({
            "mip_slice": mip, "array_slice": 0, "file_offset": file_offset,
            "byte_len": byte_len, "row_pitch": row_pitch,
            "depth_pitch": byte_len, "width": mip_width, "height": mip_height,
        })
    assert source_offset == len(source), "DDS contains unconsumed bytes"
    return bytes(output), subresources


def assert_compact_rows_equal(captured: bytes, source_dds: bytes, subs: list[dict]):
    source = memoryview(source_dds)[128:]
    source_offset = 0
    for sub in subs:
        compact_pitch = max(1, (sub["width"] + 3) // 4) * 16
        block_rows = max(1, (sub["height"] + 3) // 4)
        capture_offset = sub["file_offset"]
        for row in range(block_rows):
            expected = source[source_offset:source_offset + compact_pitch]
            actual_start = capture_offset + row * sub["row_pitch"]
            assert captured[actual_start:actual_start + compact_pitch] == expected
            source_offset += compact_pitch
    assert source_offset == len(source)


def verify(args) -> dict:
    oracle = json.loads(args.oracle.read_text(encoding="utf-8"))
    legacy = json.loads((args.trees / "map_frame_bundle.json").read_text(encoding="utf-8"))
    pan2 = json.loads((args.pan2 / "terrain_chunk_library.json").read_text(encoding="utf-8"))
    rows = {row["symbol"]: row for row in oracle["resources"]}
    checks = []

    assert len(rows) == 8 and len(legacy["draws"]) == 7
    checks.extend(("eight-resource-oracle", "seven-legacy-draws"))
    pan2_resources = {
        item.get("declared_name"): item
        for item in pan2["chunks"][0]["draw"]["shader_resources"]
        if item.get("declared_name") in rows
    }
    assert set(pan2_resources) == set(rows)
    checks.append("pan2-eight-resource-oracle")

    joined = 0
    for draw in legacy["draws"]:
        legacy_resources = {
            item.get("declared_name"): item
            for item in draw["shader_resources"]
            if item.get("declared_name") in rows
        }
        assert set(legacy_resources) == set(rows)
        for symbol, expected in rows.items():
            resource = legacy_resources[symbol]
            assert resource.get("file") is None
            assert resource.get("fnv1a64") is None and resource.get("byte_len") == 0
            assert resource["stage"] == "PS"
            assert (resource["slot"], resource["dxgi_format"], resource["width"], resource["height"], resource["mip_count"]) == (
                expected["slot"], expected["dxgi_format"], expected["width"], expected["height"], expected["mip_count"]
            )
            assert descriptor(resource) == descriptor(pan2_resources[symbol])
            joined += 1
    assert joined == oracle["join_law"]["required_join_count"] == 56
    checks.append("legacy-56-descriptor-bijection")

    for symbol, expected in rows.items():
        source_path = args.game / Path(expected["source"])
        source = source_path.read_bytes()
        assert len(source) == expected["source_bytes"]
        assert sha256(source) == expected["source_sha256"]
        capture_resource = pan2_resources[symbol]
        captured = (args.pan2 / expected["captured_payload_file"]).read_bytes()
        assert len(captured) == capture_resource["byte_len"] == expected["payload_bytes"]
        assert sha256(captured) == expected["captured_payload_sha256"]
        assert fnv1a64(captured) == capture_resource["fnv1a64"] == expected["captured_payload_fnv1a64"]
        if expected["dxgi_format"] in (77, 78):
            normalized, subresources = pack_bc3_dds(source, expected["width"], expected["height"], expected["mip_count"])
            assert subresources == capture_resource["subresources"]
            assert_compact_rows_equal(captured, source, subresources)
        elif symbol == "HeightNormal":
            normalized = bmp_rgbx(source, expected["width"], expected["height"])
        elif symbol == "TerrainColorTint":
            assert source[:4] == b"DDS " and source[84:88] == bytes(4)
            normalized = source[128:]
        else:
            raise AssertionError(f"unhandled source producer for {symbol}")
        assert len(normalized) == expected["payload_bytes"]
        assert sha256(normalized) == expected["normalized_payload_sha256"]
        assert fnv1a64(normalized) == expected["normalized_payload_fnv1a64"]
        checks.append(f"source-payload:{symbol}")

    # Focused negative controls: descriptor drift and source corruption fail exact identity.
    first = dict(next(iter(legacy["draws"]))["shader_resources"][0])
    first["slot"] = 999
    assert descriptor(first) != descriptor(next(iter(pan2_resources.values())))
    terrain_source = (args.game / rows["TerrainDiffuse"]["source"]).read_bytes()
    corrupted = bytearray(terrain_source)
    corrupted[-1] ^= 1
    assert sha256(bytes(corrupted)) != rows["TerrainDiffuse"]["source_sha256"]
    checks.extend(("negative-descriptor-drift", "negative-source-corruption"))
    return {
        "schema": "hoi4-trees-measure-legacy-static-srv-join-verification-v1",
        "status": "PASS_EXACT_SOURCE_AND_CAPTURE_ORACLE_FROZEN",
        "checks_passed": len(checks),
        "legacy_draws": len(legacy["draws"]),
        "resources_per_draw": len(rows),
        "descriptor_joins": joined,
        "negative_controls": 2,
        "production_code_modified": False,
        "program_status_modified": False,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle", type=Path, default=HERE / "oracle.json")
    parser.add_argument("--trees", type=Path, default=Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\live_trees_measure"))
    parser.add_argument("--pan2", type=Path, default=Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\live_PAN2"))
    parser.add_argument("--game", type=Path, default=Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = verify(args)
    if args.output:
        args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    raise SystemExit(main())
