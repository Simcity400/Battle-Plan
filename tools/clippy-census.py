import base64
import gzip
import hashlib
import json
import re
import subprocess
import sys
import tomllib
from collections import defaultdict
from pathlib import Path


def emit(section, records):
    data = gzip.compress(json.dumps(records, ensure_ascii=True, separators=(",", ":")).encode())
    payload = base64.b64encode(data).decode()
    chunks = [payload[start:start + 8192] for start in range(0, len(payload), 8192)]
    print(f"CENSUS {section} MANIFEST {len(records)} {len(chunks)} {hashlib.sha256(data).hexdigest()}")
    for index, chunk in enumerate(chunks, 1):
        print(f"CENSUS {section} {index} {chunk}")


def main():
    manifest = tomllib.loads(Path("Cargo.toml").read_text(encoding="utf-8"))
    lints = list(manifest["workspace"]["lints"]["clippy"])[-9:]
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version=1", "--no-deps"], encoding="utf-8"))
    members = {package["id"]: package for package in metadata["packages"]}
    findings = {lint: [] for lint in lints}
    artifacts = defaultdict(set)
    exempt = 0
    finished = None
    command = ["cargo", "clippy", "--workspace", "--all-targets", "--keep-going",
               "--no-deps", "--message-format=json"]
    print(" ".join(command), flush=True)
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True, encoding="utf-8") as cargo:
        for line in cargo.stdout:
            if not line.startswith("{"):
                print(line, end="")
                continue
            row = json.loads(line)
            package_id = row.get("package_id")
            if row["reason"] == "build-finished":
                finished = row["success"]
            elif row["reason"] == "compiler-artifact" and package_id in members:
                target = row["target"]
                artifacts[package_id].add((target["name"], tuple(target["kind"]), row["profile"]["test"]))
            elif row["reason"] == "compiler-message":
                message = row["message"]
                code = (message["code"] or {}).get("code", "").removeprefix("clippy::")
                if code not in findings or package_id not in members:
                    if message["level"] == "error":
                        print(message["rendered"] or message["message"], end="\n")
                    continue
                if code == "unwrap_used" and (
                    "test" in row["target"]["kind"] or any(
                        span["is_primary"] and re.search(r"(?:^|/)crates/[^/]+/tests/", span["file_name"].replace("\\", "/"))
                        for span in message["spans"]
                    )
                ):
                    exempt += 1
                    continue
                findings[code].append(row)
        status = cargo.wait()
    inventory = [
        {"name": package["name"], "id": key, "manifest": package["manifest_path"],
         "targets": package["targets"], "artifacts": sorted(artifacts[key])}
        for key, package in sorted(members.items())
    ]
    emit("inventory", inventory)
    for lint, records in findings.items():
        emit(lint, records)
    print(f"CENSUS COMPLETE cargo_exit={status} build_finished={finished} unwrap_integration_exempt={exempt}")
    return status or (finished is not True) or bool(set(metadata["workspace_members"]) - set(artifacts))


if __name__ == "__main__":
    sys.exit(main())
