"""Release steps for Battle Plan and Battle Plan Recorder, which share one version (Python 3.11+).

The workspace version in Cargo.toml is the last stable release. A stable release is its tag,
`v<version>`. Nightly and preview builds are the next patch with a dated prerelease suffix,
`<major>.<minor>.<patch+1>-nightly.<yyyymmdd>.<run>`, so semver orders them after that stable
release and before the next one. The pipeline stamps the version into Cargo.toml before it builds.

  version  --channel stable --tag v1.2.3 | --channel nightly|preview --date 20261108 --run 7
  stamp    VERSION
  notes    --since COMMIT|'' --until COMMIT --channel CHANNEL --output FILE
  manifest --version V --signature FILE --url URL --notes FILE --commit SHA --output FILE
  verify   --installer FILE --signature FILE --version V
  prune    --target DIR --cap-gb N
  newer    --than OLD NEW   (fails unless NEW is a later version than OLD)
"""
import argparse
import base64
import datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO = ROOT / "Cargo.toml"
PUBLIC_KEY = ROOT / "crates" / "app-update" / "updater.pub"
STABLE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
VERSION_LINE = re.compile(r'(?ms)^(\[workspace\.package\]\s*?\n(?:(?!\[).*\n)*?version = ")([^"]+)(")')
MERGE = re.compile(r"^Merge pull request #(\d+) from ")
SQUASHED = re.compile(r"\(#(\d+)\)$")
NOTES_SHOWN = 40


def fail(message):
    print(message, file=sys.stderr)
    sys.exit(1)


def workspace_version():
    version = tomllib.loads(CARGO.read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    if not STABLE.match(version):
        fail(f"The workspace version {version} in Cargo.toml must be a stable release, like 1.2.3.")
    return version


def outputs(values, github_output):
    lines = [f"{key}={value}" for key, value in values.items()]
    if github_output:
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write("\n".join(lines) + "\n")
    print("\n".join(lines))


def version(args):
    base = workspace_version()
    if args.channel == "stable":
        if args.tag != f"v{base}":
            fail(f"Tag {args.tag} does not match the workspace version {base} in Cargo.toml. "
                 "Merge a change that sets the version, then tag that commit.")
        resolved = base
    else:
        if not (args.date and re.fullmatch(r"\d{8}", args.date) and args.run and args.run.isdigit()):
            fail("Nightly and preview versions need --date YYYYMMDD and --run N.")
        major, minor, patch = STABLE.match(base).groups()
        resolved = f"{major}.{minor}.{int(patch) + 1}-{args.channel}.{args.date}.{int(args.run)}"
    label = {"stable": "", "nightly": " Nightly", "preview": " Preview (pipeline test, do not install)"}[args.channel]
    outputs({"version": resolved, "tag": f"v{resolved}", "name": f"Battle Plan{label} {resolved}"}, args.github_output)


def stamp(args):
    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", args.version):
        fail(f"{args.version} is not a release version.")
    text = CARGO.read_text(encoding="utf-8")
    stamped, count = VERSION_LINE.subn(lambda match: match[1] + args.version + match[3], text, count=1)
    if count != 1:
        fail("Cargo.toml has no [workspace.package] version to stamp.")
    CARGO.write_text(stamped, encoding="utf-8", newline="\n")
    print(f"Stamped {args.version}; run `cargo update --workspace` to carry it into Cargo.lock.")


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8").stdout


def notes(args):
    """The pull requests merged on main since the last release on this channel, by their titles,
    which GitHub gives; never commit text. Needs GITHUB_REPOSITORY and GH_TOKEN."""
    if not args.since:
        body = f"The first {args.channel} release.\n"
    else:
        repository = os.environ.get("GITHUB_REPOSITORY") or fail("GITHUB_REPOSITORY names no repository.")
        subjects = git("log", "--first-parent", "--format=%s", f"{args.since}..{args.until}").splitlines()
        numbers, unmerged = [], 0
        for subject in subjects:
            merged = MERGE.match(subject) or SQUASHED.search(subject)
            if merged:
                numbers.append(int(merged[1]))
            elif subject.strip():
                unmerged += 1
        items = []
        for number in numbers:
            title = subprocess.run(["gh", "api", f"repos/{repository}/pulls/{number}", "--jq", ".title"],
                                   capture_output=True, text=True, encoding="utf-8")
            if title.returncode != 0:
                fail(f"Pull request #{number}'s title could not be read: {title.stderr.strip()}")
            items.append(f"{title.stdout.strip()} (#{number})")
        shown = [f"- {item}" for item in items[:NOTES_SHOWN]]
        if len(items) > NOTES_SHOWN:
            shown.append(f"- And {len(items) - NOTES_SHOWN} more pull requests.")
        if unmerged:
            shown.append(f"- And {unmerged} {'change' if unmerged == 1 else 'changes'} made without a pull request.")
        body = "\n".join(shown) + "\n" if shown else "No changes since the last release.\n"
    Path(args.output).write_text(body, encoding="utf-8", newline="\n")
    print(f"Wrote {args.output}.")


def manifest(args):
    """Tauri's static update manifest, plus the commit it was built from, which the next
    nightly compares main against."""
    document = {
        "version": args.version,
        "notes": Path(args.notes).read_text(encoding="utf-8").strip(),
        "pub_date": datetime.datetime.now(datetime.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "commit": args.commit,
        "platforms": {"windows-x86_64": {
            "signature": Path(args.signature).read_text(encoding="utf-8").strip(),
            "url": args.url,
        }},
    }
    Path(args.output).write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"Wrote {args.output}.")


def minisign_lines(encoded, what):
    """A Tauri key or signature: base64 of minisign's text form."""
    try:
        return base64.b64decode(encoded).decode("utf-8").splitlines()
    except ValueError as error:
        fail(f"The {what} is not base64 minisign text: {error}")


def verify(args):
    """The installer verifies, as both apps will verify it, against the public key they carry."""
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    key = base64.b64decode(minisign_lines(PUBLIC_KEY.read_text(encoding="utf-8").strip(), "public key")[1])
    if key[:2] != b"Ed":
        fail("The updater public key is not an Ed25519 minisign key.")
    lines = minisign_lines(Path(args.signature).read_text(encoding="utf-8").strip(), "signature")
    signature = base64.b64decode(lines[1])
    trusted = lines[2].removeprefix("trusted comment: ")
    global_signature = base64.b64decode(lines[3])
    if signature[2:10] != key[2:10]:
        fail("The installer was signed with another key than the apps carry; the signing secret does not match.")
    public = Ed25519PublicKey.from_public_bytes(key[10:42])
    data = Path(args.installer).read_bytes()
    message = hashlib.blake2b(data, digest_size=64).digest() if signature[:2] == b"ED" else data
    try:
        public.verify(signature[10:74], message)
        public.verify(global_signature, signature[10:74] + trusted.encode("utf-8"))
    except InvalidSignature:
        fail("The installer's signature does not verify against the apps' public key.")
    signed = next((field.removeprefix("version:") for field in trusted.split("\t") if field.startswith("version:")), None)
    if signed != args.version:
        fail(f"The signature is bound to version {signed}, not {args.version}; the apps require that binding.")
    print(f"Verified: signed by key {signature[2:10][::-1].hex().upper()} for version {signed}.")


def precedence(version):
    """Semver's order: core numbers, then a release after its prereleases, then each prerelease
    field, numbers before words and numbers by value."""
    core, _, pre = version.partition("+")[0].partition("-")
    if not STABLE.match(core):
        fail(f"{version} is not a release version.")
    fields = [(0, int(field), "") if field.isdigit() else (1, 0, field) for field in pre.split(".")] if pre else []
    return tuple(int(part) for part in core.split(".")), not pre, fields


def newer(args):
    if precedence(args.version) <= precedence(args.than):
        fail(f"{args.version} is not newer than {args.than}, which is already published.")
    print(f"{args.version} is newer than {args.than}.")


def prune(args):
    """After a build on the self-hosted runner: its installers go, and a build folder grown past
    the cap goes whole, so the folder stays bounded while the next build reuses what it can."""
    target = Path(args.target)
    if not target.is_absolute():
        fail(f"The build folder {args.target!r} must be an absolute path.")
    if not target.exists():
        print(f"{target} does not exist; nothing to prune.")
        return
    for bundle in target.glob("*/bundle"):
        shutil.rmtree(bundle)
        print(f"Removed {bundle}.")
    size = sum(entry.stat().st_size for entry in target.rglob("*") if entry.is_file())
    print(f"{target} holds {size / 2**30:.1f} GB of a {args.cap_gb} GB cap.")
    if size > args.cap_gb * 2**30:
        shutil.rmtree(target)
        print(f"Removed {target}: past its cap, the next build starts it again.")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("version")
    command.add_argument("--channel", required=True, choices=["stable", "nightly", "preview"])
    command.add_argument("--tag")
    command.add_argument("--date")
    command.add_argument("--run")
    command.add_argument("--github-output", action="store_true")
    command.set_defaults(handler=version)
    command = commands.add_parser("stamp")
    command.add_argument("version")
    command.set_defaults(handler=stamp)
    command = commands.add_parser("notes")
    command.add_argument("--since", default="")
    command.add_argument("--until", required=True)
    command.add_argument("--channel", required=True)
    command.add_argument("--output", required=True)
    command.set_defaults(handler=notes)
    command = commands.add_parser("manifest")
    for name in ("version", "signature", "url", "notes", "commit", "output"):
        command.add_argument(f"--{name}", required=True)
    command.set_defaults(handler=manifest)
    command = commands.add_parser("verify")
    for name in ("installer", "signature", "version"):
        command.add_argument(f"--{name}", required=True)
    command.set_defaults(handler=verify)
    command = commands.add_parser("newer")
    command.add_argument("--than", required=True)
    command.add_argument("version")
    command.set_defaults(handler=newer)
    command = commands.add_parser("prune")
    command.add_argument("--target", required=True)
    command.add_argument("--cap-gb", required=True, type=int)
    command.set_defaults(handler=prune)
    args = parser.parse_args()
    args.handler(args)


if __name__ == "__main__":
    main()
