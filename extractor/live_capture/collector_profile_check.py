"""Collector profile preflight checks for a running the game process."""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
from pathlib import Path
from typing import Any, Dict, List, Optional

from .collector_attach import attach_noop
from .collector_frames import latest_verified_root_probe, validate_collector_frame


PROCESS_QUERY_LIMITED_INFORMATION = 0x1000


class ProfileCheckError(RuntimeError):
    pass


def load_profile(path: Path) -> Dict[str, Any]:
    profile = json.loads(Path(path).read_text(encoding="utf-8"))
    problems = validate_profile(profile)
    if problems:
        raise ProfileCheckError("invalid collector profile: " + "; ".join(problems))
    return profile


def validate_profile(profile: Any) -> List[str]:
    problems: List[str] = []
    if not isinstance(profile, dict):
        return ["profile not a dict"]
    for key in ("label", "exe_sha256", "hooks", "roots", "domains_enabled"):
        if key not in profile:
            problems.append(f"profile missing {key!r}")
    if "label" in profile and not isinstance(profile.get("label"), str):
        problems.append("profile.label not str")
    exe_sha = profile.get("exe_sha256")
    if not isinstance(exe_sha, str) or len(exe_sha) != 64:
        problems.append("profile.exe_sha256 not 64-char hex string")
    elif not all(c in "0123456789abcdefABCDEF" for c in exe_sha):
        problems.append("profile.exe_sha256 not hex")
    if "hooks" in profile and not isinstance(profile.get("hooks"), dict):
        problems.append("profile.hooks not dict")
    if "roots" in profile and not isinstance(profile.get("roots"), dict):
        problems.append("profile.roots not dict")
    domains = profile.get("domains_enabled")
    if not isinstance(domains, list) or not all(isinstance(d, str) for d in domains):
        problems.append("profile.domains_enabled not list[str]")
    return problems


def check_running_process(
    pid: int,
    profile: Dict[str, Any],
    *,
    dll: Optional[Path] = None,
    dwell_s: float = 0.5,
) -> Dict[str, Any]:
    exe_path = process_image_path(pid)
    actual_hash = sha256_file(exe_path)
    expected_hash = str(profile["exe_sha256"]).upper()
    result: Dict[str, Any] = {
        "type": "collector_profile_check",
        "pid": pid,
        "profile": profile.get("label"),
        "exe_sha256_expected": expected_hash,
        "exe_sha256_actual": actual_hash,
        "exe_sha256_match": actual_hash == expected_hash,
        "attach_checked": False,
        "ok": False,
        "problems": [],
    }
    if actual_hash != expected_hash:
        result["problems"].append("exe_sha256_mismatch")
        return result

    if dll is None:
        result["ok"] = True
        return result

    attach = attach_noop(pid, Path(dll), dwell_s=dwell_s, use_pipe=True)
    result["attach_checked"] = True
    if attach.pipe_frame is None:
        result["problems"].append("collector_pipe_frame_missing")
        return result
    frame_problems = validate_collector_frame(attach.pipe_frame)
    if frame_problems:
        result["problems"].extend(f"frame:{p}" for p in frame_problems)
        return result
    root_probe = latest_verified_root_probe([attach.pipe_frame])
    if root_probe is None:
        result["problems"].append("root_probe_not_verified")
        return result
    result["root_probe_quality_status"] = attach.pipe_frame.get("quality_status")
    result["root_probe_profile"] = root_probe.get("profile")
    result["root_probe_domains"] = root_probe.get("domains") or []
    result.update(country_payload_summary(attach.pipe_frame, root_probe))
    result["transport_crc32"] = attach.pipe_frame.get("transport_crc32")
    result["transport_bytes"] = attach.pipe_frame.get("transport_bytes")
    result["sequence"] = attach.pipe_frame.get("sequence")
    result["problems"].extend(validate_root_probe_against_profile(root_probe, profile))
    result["ok"] = not result["problems"]
    return result


def country_payload_summary(
    frame: Dict[str, Any],
    root_probe: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    countries = frame.get("countries") if isinstance(frame, dict) else None
    countries = countries if isinstance(countries, dict) else {}
    root_probe = root_probe if isinstance(root_probe, dict) else {}
    domains = root_probe.get("domains")
    domains = domains if isinstance(domains, list) else []
    identity = root_probe.get("country_object_identity")
    identity = identity if isinstance(identity, dict) else {}
    scalar_rows = 0
    for row in countries.values():
        if not isinstance(row, dict):
            continue
        scalars = row.get("scalars")
        if isinstance(scalars, dict) and any(value is not None for value in scalars.values()):
            scalar_rows += 1
    return {
        "country_rows": len(countries),
        "country_object_identity_status": identity.get("status"),
        "country_scalar_domain_present": "country_scalars" in domains,
        "country_scalar_rows": scalar_rows,
    }


def validate_root_probe_against_profile(
    root_probe: Dict[str, Any],
    profile: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    expected_label = profile.get("label")
    if root_probe.get("profile") != expected_label:
        problems.append("root_probe_profile_mismatch")
    if root_probe.get("daily_update_sentinel_match") is not True:
        problems.append("daily_update_sentinel_not_verified")
    if root_probe.get("long_string_sentinel_match") is not True:
        problems.append("long_string_sentinel_not_verified")

    image_base = _parse_hex(root_probe.get("image_base"))
    if image_base is not None:
        daily_offset = _profile_hook_offset(profile, "daily_update")
        if daily_offset is not None:
            expected = f"0x{image_base + daily_offset:X}"
            if root_probe.get("daily_update") != expected:
                problems.append("daily_update_address_mismatch")
        long_offset = _profile_root_offset(profile, "long_string")
        if long_offset is not None:
            expected = f"0x{image_base + long_offset:X}"
            if root_probe.get("get_long_string") != expected:
                problems.append("long_string_address_mismatch")

    expected_domains = set(profile.get("domains_enabled") or [])
    observed_domains = set(root_probe.get("domains") or [])
    missing_domains = sorted(expected_domains - observed_domains)
    if missing_domains:
        problems.append("root_probe_missing_domains:" + ",".join(missing_domains))
    return problems


def process_image_path(pid: int) -> Path:
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.QueryFullProcessImageNameW.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        wintypes.LPWSTR,
        ctypes.POINTER(wintypes.DWORD),
    ]
    kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        raise ProfileCheckError(f"OpenProcess failed for pid {pid}: {ctypes.get_last_error()}")
    try:
        buf = ctypes.create_unicode_buffer(32768)
        size = wintypes.DWORD(len(buf))
        if not kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size)):
            raise ProfileCheckError(
                f"QueryFullProcessImageNameW failed for pid {pid}: {ctypes.get_last_error()}"
            )
        return Path(buf.value)
    finally:
        kernel32.CloseHandle(handle)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest().upper()


def _profile_hook_offset(profile: Dict[str, Any], name: str) -> Optional[int]:
    hooks = profile.get("hooks") if isinstance(profile.get("hooks"), dict) else {}
    hook = hooks.get(name) if isinstance(hooks, dict) else None
    if not isinstance(hook, dict):
        return None
    return _parse_hex(hook.get("offset"))


def _profile_root_offset(profile: Dict[str, Any], name: str) -> Optional[int]:
    roots = profile.get("roots") if isinstance(profile.get("roots"), dict) else {}
    root = roots.get(name) if isinstance(roots, dict) else None
    if not isinstance(root, dict):
        return None
    return _parse_hex(root.get("offset"))


def _parse_hex(value: Any) -> Optional[int]:
    if not isinstance(value, str):
        return None
    try:
        return int(value, 16)
    except ValueError:
        return None


def _main() -> int:
    parser = argparse.ArgumentParser(description="Check a running the game process against a collector profile")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--dll", type=Path, default=None)
    parser.add_argument("--dwell-s", type=float, default=0.5)
    args = parser.parse_args()

    profile = load_profile(args.profile)
    result = check_running_process(args.pid, profile, dll=args.dll, dwell_s=args.dwell_s)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result.get("ok") else 2


if __name__ == "__main__":
    raise SystemExit(_main())
