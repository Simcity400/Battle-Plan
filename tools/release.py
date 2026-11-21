"""Release Battle Plan and Battle Plan Recorder, which share one version (Python 3.11+).

The steps of the release workflows (docs/adr/0001-release-channels.md). Main's CI builds one
release candidate per commit (.github/workflows/release-candidate.yml). The release workflow
(.github/workflows/release.yml) verifies, signs, smoke-tests and publishes one, without compiling.

  version   --channel stable|nightly|preview [--sha SHA] [--version V] [--github-output]
  stamp     VERSION
  candidate --installer FILE --channel C --version V --commit SHA --output FILE
  resolve   --channel C [--sha SHA] [--version V] [--scheduled] [--github-output]
  publish   --channel C --candidate DIR [--draft] [--public-key FILE]
  finalize  --version V
  verify    --installer FILE --signature FILE --version V [--public-key FILE]
  newer     --than OLD NEW   (fails unless NEW is a later version than OLD)

The workspace version in Cargo.toml is the last stable release. Nightly and preview candidates are
its next patch with `-<channel>.<yyyymmdd>.<n>`: the UTC date of the build and the commit's position
on main's first-parent line, so candidates built in main's order sort in it. Stable is that next
patch, or `--version` for a larger step. `finalize` then makes it main's workspace version, through
a pull request. The version is compiled in, so a candidate belongs to one channel.

`resolve` decides whether a release is due and from what. A nightly is due when main has changed
since the nightly feed's commit and main's CI is green on it, unless PAUSE_NIGHTLY is true. A stable
takes the commit the nightly feed offers unless `--sha` names one. A preview, a draft with no feed,
takes main's head or `--sha`. `publish --draft` is the dry run: everything goes to a new draft
release, its own latest.json included, and the channel's feed is left alone. `--public-key` stands
in for the apps' key in a draft only.

Environment: GITHUB_REPOSITORY, whose releases carry the installers and feeds; GH_TOKEN;
TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD; AZURE_TENANT_ID, AZURE_CLIENT_ID,
AZURE_CLIENT_SECRET, AZURE_SIGNING_ENDPOINT, AZURE_SIGNING_ACCOUNT and AZURE_SIGNING_PROFILE, all or
none, for Authenticode; LEAK_GATE_TERMS; PAUSE_NIGHTLY.
"""
import argparse
import base64
import binascii
import contextlib
import datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ROOT = TOOLS.parent
sys.path.insert(0, str(TOOLS))
import release_smoke

PUBLIC_KEY = ROOT / "crates" / "app-update" / "updater.pub"
STABLE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
VERSION_LINE = re.compile(r'(?ms)^(\[workspace\.package\]\s*?\n(?:(?!\[).*\n)*?version = ")([^"]+)(")')
MERGE = re.compile(r"^Merge pull request #(\d+) from ")
SQUASHED = re.compile(r"\(#(\d+)\)$")
NOTES_SHOWN = 40
CHANNELS = ("stable", "nightly", "preview")
PUBLISHED = ("stable", "nightly")
AZURE = ("AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET", "AZURE_SIGNING_ENDPOINT",
         "AZURE_SIGNING_ACCOUNT", "AZURE_SIGNING_PROFILE")
GB = 2**30
ASSET_LIMIT = 2 * GB
INSTALLER = "battle-plan-{version}-x64-setup.exe"
CANDIDATE_WORKFLOW = "release-candidate.yml"
SIGNED = re.compile(r"Signed installer SHA-256: ([0-9a-f]{64})\.")
SETTLED = ("success", "skipped", "neutral")


class Refused(Exception):
    """A named reason a release step does not go on."""



def say(text):
    print(text, flush=True)



def run(command, cwd=ROOT, env=None, check=True, capture=True):
    done = subprocess.run([str(part) for part in command], cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                          capture_output=capture, text=True, encoding="utf-8", errors="replace")
    if check and done.returncode != 0:
        told = (done.stderr or done.stdout or "").strip()[-2000:] if capture else "see the output above"
        raise Refused(f"{Path(str(command[0])).stem} {command[1]} failed (exit {done.returncode}): {told}")
    return done



def git(*args, cwd=ROOT):
    return run(["git", *args], cwd=cwd).stdout.strip()



def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()



def workspace_version(text):
    version = tomllib.loads(text)["workspace"]["package"]["version"]
    if not STABLE.match(version):
        raise Refused(f"The workspace version {version} in Cargo.toml must be a stable release, like 1.2.3.")
    return version



def candidate_version(channel, base, built, position, stable=None):
    """The version of a candidate built on `built` (UTC) from the commit at `position` on main's
    first-parent line, whose workspace version, the last stable release, is `base`. Stable is
    `stable`, by default the next patch, which the nightlies preview. Nightly and preview are that
    next patch prereleased on the build date and the position: built in main's order, they sort in it."""
    major, minor, patch = STABLE.match(base).groups()
    following = f"{major}.{minor}.{int(patch) + 1}"
    if channel != "stable":
        return f"{following}-{channel}.{built:%Y%m%d}.{position}"
    chosen = stable or following
    if not STABLE.match(chosen):
        raise Refused(f"{chosen} is not a stable version, like 1.2.3.")
    refuse_older(chosen, base, "the workspace's last stable release")
    return chosen



def release_name(channel, version):
    label = {"stable": "", "nightly": " Nightly", "preview": " Preview (pipeline test, do not install)"}[channel]
    return f"Battle Plan{label} {version}"



def precedence(version):
    """Semver's order: core numbers, then a release after its prereleases, then each prerelease
    field, numbers before words and numbers by value."""
    core, _, pre = version.partition("+")[0].partition("-")
    if not STABLE.match(core):
        raise Refused(f"{version} is not a release version.")
    fields = [(0, int(field), "") if field.isdigit() else (1, 0, field) for field in pre.split(".")] if pre else []
    return tuple(int(part) for part in core.split(".")), not pre, fields



def refuse_older(version, than, what):
    if than and precedence(version) <= precedence(than):
        raise Refused(f"{version} is not newer than {than}, {what}.")



def stamp_lock(text, names, version):
    """Cargo.lock with the version of each workspace package in `names` set to `version`; registry,
    git and vendored packages keep theirs."""
    crlf = "\r\n" in text
    blocks = text.replace("\r\n", "\n").split("\n[[package]]\n")

    def stamped(block):
        name = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
        if not name or name[1] not in names or re.search(r"^source = ", block, re.MULTILINE):
            return block
        return re.sub(r'^version = "[^"]+"$', f'version = "{version}"', block, count=1, flags=re.MULTILINE)

    result = "\n[[package]]\n".join(stamped(block) for block in blocks)
    return result.replace("\n", "\r\n") if crlf else result



def members(tree):
    """Each workspace member's package table, by name."""
    workspace = tomllib.loads((tree / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    excluded = {Path(path).as_posix() for path in workspace.get("exclude", [])}
    found = {}
    for pattern in workspace.get("members", []):
        for manifest in sorted(tree.glob(f"{pattern}/Cargo.toml")):
            if manifest.parent.relative_to(tree).as_posix() in excluded:
                continue
            document = tomllib.loads(manifest.read_text(encoding="utf-8"))
            if "package" in document:
                found[document["package"]["name"]] = document
    return found



def stamp(tree, version):
    """The version into a checkout's Cargo.toml and Cargo.lock, as Cargo itself would carry it."""
    cargo = tree / "Cargo.toml"
    with open(cargo, encoding="utf-8", newline="") as file:
        text = file.read()
    stamped, count = VERSION_LINE.subn(lambda match: match[1] + version + match[3], text.replace("\r\n", "\n"), count=1)
    if count != 1:
        raise Refused("Cargo.toml has no [workspace.package] version to stamp.")
    with open(cargo, "w", encoding="utf-8", newline="") as file:
        file.write(stamped.replace("\n", "\r\n") if "\r\n" in text else stamped)
    inheriting = {name for name, document in members(tree).items()
                  if document["package"].get("version") == {"workspace": True}}
    lock = tree / "Cargo.lock"
    with open(lock, encoding="utf-8", newline="") as file:
        text = file.read()
    with open(lock, "w", encoding="utf-8", newline="") as file:
        file.write(stamp_lock(text, inheriting, version))



def write_notes(repository, channel, since, until, output):
    """The pull requests merged on main since the last release on this channel, by their titles,
    which GitHub gives; never commit text."""
    if not since:
        body = f"The first {channel} release.\n"
    else:
        subjects = git("log", "--first-parent", "--format=%s", f"{since}..{until}").splitlines()
        numbers, unmerged = [], 0
        for subject in subjects:
            merged = MERGE.match(subject) or SQUASHED.search(subject)
            if merged:
                numbers.append(int(merged[1]))
            elif subject.strip():
                unmerged += 1

        def title(number):
            return run(["gh", "api", f"repos/{repository}/pulls/{number}", "--jq", ".title"]).stdout.strip()

        with ThreadPoolExecutor(max_workers=8) as pool:
            items = [f"{name} (#{number})" for number, name in zip(numbers, pool.map(title, numbers))]
        shown = [f"- {item}" for item in items[:NOTES_SHOWN]]
        if len(items) > NOTES_SHOWN:
            shown.append(f"- And {len(items) - NOTES_SHOWN} more pull requests.")
        if unmerged:
            shown.append(f"- And {unmerged} {'change' if unmerged == 1 else 'changes'} made without a pull request.")
        body = "\n".join(shown) + "\n" if shown else "No changes since the last release.\n"
    Path(output).write_text(body, encoding="utf-8", newline="\n")



def write_manifest(version, signature, url, notes, commit, output):
    """Tauri's static update manifest, plus the commit it was built from, which the next promotion
    compares main against."""
    document = {
        "version": version,
        "notes": Path(notes).read_text(encoding="utf-8").strip(),
        "pub_date": datetime.datetime.now(datetime.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "commit": commit,
        "platforms": {"windows-x86_64": {
            "signature": Path(signature).read_text(encoding="utf-8").strip(),
            "url": url,
        }},
    }
    Path(output).write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8", newline="\n")



def minisign_lines(encoded, what):
    """A Tauri key or signature: base64 of minisign's text form."""
    try:
        return base64.b64decode(encoded, validate=True).decode("utf-8").splitlines()
    except (ValueError, UnicodeDecodeError) as error:
        raise Refused(f"The {what} is not base64 minisign text: {error}")



def verify_signature(installer, signature_file, version, public_key):
    """The installer verifies, as both apps will verify it, against the public key they carry; the
    signing key's ID."""
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    key = base64.b64decode(minisign_lines(public_key.strip(), "public key")[1])
    if key[:2] != b"Ed":
        raise Refused("The updater public key is not an Ed25519 minisign key.")
    lines = minisign_lines(Path(signature_file).read_text(encoding="utf-8").strip(), "signature")
    signature = base64.b64decode(lines[1])
    trusted = lines[2].removeprefix("trusted comment: ")
    global_signature = base64.b64decode(lines[3])
    if signature[2:10] != key[2:10]:
        raise Refused(f"The updater key is {signature[2:10][::-1].hex().upper()}, not the "
                      f"{key[2:10][::-1].hex().upper()} the apps carry; the signing key does not match.")
    public = Ed25519PublicKey.from_public_bytes(key[10:42])
    data = Path(installer).read_bytes()
    message = hashlib.blake2b(data, digest_size=64).digest() if signature[:2] == b"ED" else data
    try:
        public.verify(signature[10:74], message)
        public.verify(global_signature, signature[10:74] + trusted.encode("utf-8"))
    except InvalidSignature:
        raise Refused("The installer's signature does not verify against the apps' public key.")
    signed = next((field.removeprefix("version:") for field in trusted.split("\t") if field.startswith("version:")), None)
    if signed != version:
        raise Refused(f"The signature is bound to version {signed}, not {version}; the apps require that binding.")
    return signature[2:10][::-1].hex().upper()



def key_problem(text, source):
    """Why `text` is not a Tauri updater secret key as Tauri decodes one: one line of standard
    base64 around minisign's text form of an Ed25519 secret key. None when it is one."""
    try:
        lines = base64.b64decode(text, validate=True).decode("utf-8").splitlines()
    except (binascii.Error, ValueError):
        return f"{source} is not one line of standard base64, which is how Tauri reads it."
    except UnicodeDecodeError:
        return f"{source} decodes to bytes that are not minisign text."
    if len(lines) < 2 or not lines[0].startswith("untrusted comment:"):
        return f"{source} decodes to no minisign key."
    try:
        box = base64.b64decode(lines[1], validate=True)
    except (binascii.Error, ValueError):
        return f"{source} holds a minisign key whose key line is not base64."
    if len(box) == 42:
        return f"{source} holds the updater's public key, not its secret key."
    if len(box) != 158 or box[:2] != b"Ed":
        return f"{source} is not an Ed25519 minisign secret key."
    return None



def tauri_cli(tree=ROOT):
    return tree / "node_modules" / "@tauri-apps" / "cli" / "tauri.js"



def sign_for_updates(file, version, tree=ROOT):
    """<file>.sig, the updater signature bound to `version`, made by Tauri's own signer."""
    signature = Path(f"{file}.sig")
    signature.unlink(missing_ok=True)
    signed = run(["node", tauri_cli(tree), "signer", "sign", "--app-version", version, file], check=False)
    if signed.returncode != 0 or not signature.is_file():
        told = signed.stderr + signed.stdout
        if "password" in told.lower():
            raise Refused("The updater key's password, TAURI_SIGNING_PRIVATE_KEY_PASSWORD, is wrong.")
        raise Refused(f"Tauri's signer refused the updater key (exit {signed.returncode}): {told.strip()[-500:]}")
    return signature



def authenticode_enabled(required):
    """Whether to sign with Authenticode: every Azure setting is set. Some of them, or none when it
    is required, is refused by name."""
    present = [name for name in AZURE if os.environ.get(name)]
    if present and len(present) != len(AZURE):
        missing = [name for name in AZURE if name not in present]
        raise Refused(f"Authenticode needs every Azure setting; {', '.join(missing)} {'is' if len(missing) == 1 else 'are'} not set.")
    if required and not present:
        raise Refused(f"Authenticode was asked for, but none of {', '.join(AZURE)} are set.")
    return bool(present)



def release_by_tag(repo, tag):
    """The published release of `tag`, or None when it has none; any other answer is refused."""
    read = run(["gh", "api", f"repos/{repo}/releases/tags/{tag}"], check=False)
    if read.returncode == 0:
        return json.loads(read.stdout)
    if "HTTP 404" in read.stderr:
        return None
    raise Refused(f"GitHub's release {tag} could not be read: {read.stderr.strip()}")



def released(repo, tag):
    return release_by_tag(repo, tag) is not None



def tag_commit(repo, tag):
    """The commit the publishing repository's tag names, or None when it has no such tag."""
    read = run(["gh", "api", f"repos/{repo}/git/ref/tags/{tag}"], check=False)
    if read.returncode != 0:
        if "HTTP 404" in read.stderr:
            return None
        raise Refused(f"The tag {tag} could not be read: {read.stderr.strip()}")
    target = json.loads(read.stdout)["object"]
    while target["type"] == "tag":
        target = json.loads(run(["gh", "api", f"repos/{repo}/git/tags/{target['sha']}"]).stdout)["object"]
    return target["sha"]



def newest_release(repo, channel):
    """The newest version published on the channel, by semver, or None before the first."""
    tags = run(["gh", "api", "--paginate", f"repos/{repo}/releases?per_page=100",
                "--jq", ".[] | select(.draft | not) | .tag_name"]).stdout.split()
    versions = [tag[1:] for tag in tags if re.fullmatch(r"v\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", tag)
                and (f"-{channel}." in tag if channel != "stable" else "-" not in tag)]
    return max(versions, key=precedence, default=None)



def read_feed(repo, channel):
    """The channel's published latest.json, or None when the channel has no feed yet."""
    read = run(["gh", "release", "download", channel, "--repo", repo, "--pattern", "latest.json", "--output", "-"], check=False)
    if read.returncode == 0:
        try:
            return json.loads(read.stdout)
        except ValueError:
            say(f"The {channel} feed's latest.json is empty or damaged, as a failed upload leaves it; it reads as absent.")
            return None
    if any(said in read.stderr.lower() for said in ("release not found", "no assets match", "no assets to download")):
        return None
    raise Refused(f"The {channel} feed could not be read: {read.stderr.strip()}")



def leak_gate(paths, cwd):
    """The leak gate on `paths`; it also looks for the updater key TAURI_SIGNING_PRIVATE_KEY holds."""
    checked = run([sys.executable, TOOLS / "leak-gate.py", *paths], cwd=cwd, check=False)
    lines = checked.stdout.strip().splitlines()
    if checked.returncode != 0:
        raise Refused("\n".join(lines[-20:]) or checked.stderr.strip())
    return lines[-1] if lines else "checked"



class Clock:
    def __init__(self):
        self.began, self.steps = time.monotonic(), []

    @contextlib.contextmanager
    def step(self, name):
        say(f"== {name}")
        at = time.monotonic()
        yield
        self.steps.append((name, round(time.monotonic() - at, 1)))

    def total(self):
        return round(time.monotonic() - self.began, 1)



def release_of(repo, tag, draft):
    if not draft:
        release = release_by_tag(repo, tag)
        if release is None:
            raise Refused(f"{repo} has no release {tag}.")
        return release
    releases = json.loads(run(["gh", "api", f"repos/{repo}/releases?per_page=100"]).stdout)
    release = next((release for release in releases if release["tag_name"] == tag and release["draft"]), None)
    if release is None:
        raise Refused(f"The draft {tag} is not among {repo}'s releases.")
    return release



def download(repo, tag, name, into):
    run(["gh", "release", "download", tag, "--repo", repo, "--pattern", name, "--dir", into, "--clobber"])
    return into / name



def verify_published(repo, tag, draft, files):
    """Each published asset is the file it was made from, by GitHub's SHA-256 digest, or by
    downloading it where GitHub gives none; the release's page."""
    release = release_of(repo, tag, draft)
    assets = {asset["name"]: asset for asset in release["assets"]}
    for path in files:
        asset = assets.get(path.name)
        if asset is None:
            raise Refused(f"{tag} has no {path.name}.")
        if asset.get("digest"):
            published = asset["digest"].removeprefix("sha256:")
        else:
            with tempfile.TemporaryDirectory() as scratch:
                published = sha256(download(repo, tag, path.name, Path(scratch)))
        if published != sha256(path):
            raise Refused(f"{tag}'s {path.name} is not the file that was signed and checked.")
    return release["html_url"]



def authenticode_valid(path):
    checked = run(["powershell", "-NoProfile", "-Command",
                   "if ((Get-AuthenticodeSignature -LiteralPath $env:BATTLE_PLAN_SIGNED).Status -ne 'Valid') { exit 1 }"],
                  env={**os.environ, "BATTLE_PLAN_SIGNED": str(path)}, check=False)
    return checked.returncode == 0



def repository():
    return os.environ.get("GITHUB_REPOSITORY") or run(
        ["gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"]).stdout.strip()


def feed_url(channel, repo):
    return f"https://github.com/{repo}/releases" if channel in PUBLISHED else None


def outputs(values, github_output):
    lines = [f"{key}={value}" for key, value in values.items()]
    if github_output:
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write("\n".join(lines) + "\n")
    say("\n".join(lines))


def on_main(commit, channel):
    """`commit`, refused for a public channel unless it is on main's first-parent line, where each
    commit's position, and so its version, is its own."""
    if git("rev-parse", "--is-shallow-repository") == "true":
        raise Refused("This checkout is shallow, so commits' positions on main, and so their versions, are wrong.")
    commit = git("rev-parse", "--verify", f"{commit}^{{commit}}")
    if channel in PUBLISHED and commit not in git("rev-list", "--first-parent", "origin/main").split():
        raise Refused(f"{commit} is not on main's first-parent line; {channel} releases come only from it.")
    return commit


def commit_version(channel, commit, stable=None):
    base = workspace_version(git("show", f"{commit}:Cargo.toml"))
    position = int(git("rev-list", "--count", "--first-parent", commit))
    return candidate_version(channel, base, datetime.datetime.now(datetime.UTC), position, stable)


def version(args):
    commit = on_main(args.sha or "HEAD", "preview")
    resolved = commit_version(args.channel, commit, args.version)
    outputs({"version": resolved, "tag": f"v{resolved}", "name": release_name(args.channel, resolved)}, args.github_output)
    return 0


def stamp_command(args):
    stamp(ROOT, args.version)
    say(f"Stamped {args.version} into Cargo.toml and Cargo.lock.")
    return 0


def candidate(args):
    """The manifest a candidate's artifact carries: what it is, and the key its apps verify updates with."""
    installer = Path(args.installer)
    document = {
        "channel": args.channel, "version": args.version, "commit": args.commit,
        "feed": feed_url(args.channel, repository()),
        "built": datetime.datetime.now(datetime.UTC).replace(microsecond=0).isoformat(),
        "run": os.environ.get("GITHUB_RUN_ID"),
        "updater_key": PUBLIC_KEY.read_text(encoding="utf-8"),
        "installer": {"name": installer.name, "sha256": sha256(installer), "size": installer.stat().st_size},
        "shipped": release_smoke.shipped_binaries(),
    }
    Path(args.output).write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8", newline="\n")
    say(f"Candidate {args.version} of {args.commit[:12]}: {installer.name}, SHA-256 {document['installer']['sha256']}.")
    return 0


def ci_state(repo, commit):
    """Main's CI on `commit`: the id of the successful run that built its candidate, once every
    check on the commit has settled green; else None and why not."""
    runs = json.loads(run(["gh", "api", f"repos/{repo}/actions/workflows/{CANDIDATE_WORKFLOW}/runs"
                           f"?head_sha={commit}&event=push&per_page=20"]).stdout)["workflow_runs"]
    if not runs:
        return None, "no release candidate was built for it"
    latest = max(runs, key=lambda found: (found["run_number"], found["run_attempt"]))
    if latest["status"] != "completed":
        return None, "its release candidate is still building"
    if latest["conclusion"] != "success":
        return None, f"its release candidate run ended {latest['conclusion']}"
    checks = run(["gh", "api", "--paginate", f"repos/{repo}/commits/{commit}/check-runs?per_page=100",
                  "--jq", ".check_runs[] | [.name, .status, (.conclusion // \"\")] | @tsv"]).stdout.splitlines()
    for line in checks:
        name, status, conclusion = (line.split("\t") + ["", ""])[:3]
        if status != "completed":
            return None, f"its check {name} is still running"
        if conclusion not in SETTLED:
            return None, f"its check {name} ended {conclusion}"
    return latest["id"], None


def resolve(args):
    """Whether a release of the channel is due, from which commit, and where its candidate is."""
    repo = repository()
    git("fetch", "--quiet", "origin", "main")
    found = {"due": "false", "channel": args.channel, "commit": "", "version": "", "candidate_run": "", "build": "false"}
    if args.channel == "nightly":
        if args.scheduled and os.environ.get("PAUSE_NIGHTLY", "").strip().lower() in ("1", "true", "yes"):
            say("Nightlies are paused (PAUSE_NIGHTLY).")
            return outputs(found, args.github_output) or 0
        commit = on_main(args.sha or "origin/main", "nightly")
        feed = read_feed(repo, "nightly")
        if feed and run(["git", "merge-base", "--is-ancestor", commit, feed["commit"]], check=False).returncode == 0:
            say(f"Main has not changed since the nightly of {feed['commit'][:12]}.")
            return outputs(found, args.github_output) or 0
        candidate_run, why = ci_state(repo, commit)
        if candidate_run is None:
            if args.scheduled:
                say(f"No nightly of {commit[:12]} yet: {why}.")
                return outputs(found, args.github_output) or 0
            raise Refused(f"{commit[:12]} has no green release candidate: {why}.")
        found.update(due="true", commit=commit, candidate_run=str(candidate_run))
    elif args.channel == "stable":
        if not args.sha:
            feed = read_feed(repo, "nightly")
            if not feed:
                raise Refused("No nightly feed is published, so name the stable release's commit.")
        commit = on_main(args.sha or feed["commit"], "stable")
        candidate_run, why = ci_state(repo, commit)
        if candidate_run is None:
            raise Refused(f"{commit[:12]} has not passed main's CI: {why}.")
        resolved = commit_version("stable", commit, args.version or None)
        newest = newest_release(repo, "stable")
        if not (newest == resolved and released(repo, f"v{resolved}")):
            refuse_older(resolved, newest, "the newest stable release")
        found.update(due="true", commit=commit, version=resolved, build="true")
    else:
        commit = on_main(args.sha or "origin/main", "preview")
        found.update(due="true", commit=commit, version=commit_version("preview", commit), build="true")
    say(f"A {args.channel} release of {commit[:12]} is due.")
    outputs(found, args.github_output)
    return 0


def preflight(draft, public_key):
    """The named reasons publishing cannot go on, found before anything is signed."""
    problems = []
    text = os.environ.get("TAURI_SIGNING_PRIVATE_KEY", "").strip()
    if not text:
        problems.append("No updater key: the TAURI_SIGNING_PRIVATE_KEY secret is not set.")
    elif problem := key_problem(text, "TAURI_SIGNING_PRIVATE_KEY"):
        problems.append(problem)
    else:
        with tempfile.TemporaryDirectory() as scratch:
            probe = Path(scratch) / "probe.bin"
            probe.write_bytes(b"Battle Plan updater key probe\n")
            try:
                verify_signature(probe, sign_for_updates(probe, "0.0.0-probe"), "0.0.0-probe", public_key)
            except Refused as error:
                problems.append(str(error))
    try:
        authenticode_enabled(False)
    except Refused as error:
        problems.append(str(error))
    if not os.environ.get("LEAK_GATE_TERMS"):
        problems.append("No private terms: the LEAK_GATE_TERMS secret is not set.")
    repo = repository()
    if not draft and run(["gh", "api", f"repos/{repo}", "--jq", ".private"]).stdout.strip() != "false":
        problems.append(f"{repo} is private, so installed apps could not read its update feed; publish a draft to test.")
    if problems:
        raise Refused("Preflight refused:\n" + "\n".join(f"- {problem}" for problem in problems))
    say("Preflight: ready.")


def release_order(repo, channel, version, commit, tag, draft):
    """The channel's feed, and the release of `tag` when this version is already published from
    this commit. It refuses a version not newer than the feed's or the channel's newest release
    (that release itself excepted, when it is being taken up again), a commit not after the feed's,
    and a tag on another commit."""
    on_main(commit, channel)
    feed = read_feed(repo, channel) if channel in PUBLISHED else None
    previous = feed["commit"] if feed else ""
    if previous == commit and not draft:
        raise Refused(f"The {channel} feed already offers {commit[:12]}.")
    if previous and run(["git", "merge-base", "--is-ancestor", previous, commit], check=False).returncode != 0:
        raise Refused(f"The {channel} feed was built from {previous}, which {commit[:12]} does not contain.")
    refuse_older(version, feed and feed["version"], f"which the {channel} feed offers")
    if draft:
        return feed, None
    tagged = tag_commit(repo, tag)
    if tagged and tagged != commit:
        raise Refused(f"The tag {tag} already names {tagged[:12]}, not {commit[:12]}.")
    existing = release_by_tag(repo, tag) if tagged else None
    newest = newest_release(repo, channel)
    if not (existing and newest == version):
        refuse_older(version, newest, f"the newest {channel} release")
    if existing:
        say(f"{tag} is already published from {commit[:12]}; its feed is what remains.")
    return feed, existing


def taken_up(repo, tag, existing, chosen, dist, authenticode):
    """The installer and signature of a release published earlier from this candidate, refused unless
    its text records the signed installer it serves, its checksum holds and, with Authenticode set
    up, it carries a valid Authenticode signature."""
    recorded = SIGNED.search(existing.get("body") or "")
    if not recorded:
        raise Refused(f"{tag} records no signed installer, so it is not taken up; publish its feed by hand or not at all.")
    shipped = download(repo, tag, chosen["installer"]["name"], dist)
    if sha256(shipped) != recorded[1]:
        raise Refused(f"{tag}'s installer is not the one its release records.")
    signature = download(repo, tag, f"{shipped.name}.sig", dist)
    sums = download(repo, tag, "SHA256SUMS", dist).read_text(encoding="utf-8").split()
    if sums[:2] != [sha256(shipped), shipped.name]:
        raise Refused(f"{tag}'s SHA256SUMS does not match its installer.")
    if authenticode and not authenticode_valid(shipped):
        raise Refused(f"{tag}'s installer has no valid Authenticode signature, which this release requires.")
    return shipped, signature


def previous_commit(repo, channel):
    """The commit the channel's newest release was tagged on, which its notes run from."""
    newest = newest_release(repo, channel)
    return tag_commit(repo, f"v{newest}") if newest else None


def publish(args):
    clock = Clock()
    repo, channel = repository(), args.channel
    draft = args.draft or channel == "preview"
    if args.public_key and not draft:
        raise Refused("--public-key stands in for the apps' key in a draft only.")
    folder = Path(args.candidate)
    chosen = json.loads((folder / "candidate.json").read_text(encoding="utf-8"))
    public_key = Path(args.public_key).read_text(encoding="utf-8") if args.public_key else chosen["updater_key"]
    with clock.step("Preflight"):
        preflight(draft, public_key)
        authenticode = authenticode_enabled(False)
    version, commit = chosen["version"], chosen["commit"]
    work = Path(tempfile.mkdtemp(prefix="release-", dir=os.environ.get("RUNNER_TEMP")))
    dist = work / "dist"
    dist.mkdir()
    with clock.step("Candidate"):
        if chosen["channel"] != channel:
            raise Refused(f"The candidate is for {chosen['channel']}, not {channel}.")
        if chosen["feed"] != feed_url(channel, repo):
            raise Refused(f"The candidate's apps read the feed at {chosen['feed']}, not {feed_url(channel, repo)}.")
        shipped = dist / chosen["installer"]["name"]
        shutil.copy2(folder / shipped.name, shipped)
        if sha256(shipped) != chosen["installer"]["sha256"]:
            raise Refused(f"{shipped.name} is not the installer main's CI built and checked.")
        if shipped.stat().st_size >= ASSET_LIMIT:
            raise Refused(f"{shipped.name} is {shipped.stat().st_size / GB:.1f} GB; GitHub takes release assets under 2 GB.")
        say(f"{version} of {commit[:12]}, built {chosen['built']}.")
    stamp_now = datetime.datetime.now(datetime.UTC).strftime("%Y%m%d-%H%M%S")
    tag = f"dry-run-{version}-{stamp_now}" if args.draft else f"v{version}"
    with clock.step("Release order"):
        feed, existing = release_order(repo, channel, version, commit, tag, draft)
    if existing:
        with clock.step("Published installer"):
            shipped, signature = taken_up(repo, tag, existing, chosen, dist, authenticode)
    else:
        if authenticode:
            with clock.step("Authenticode"):
                run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", TOOLS / "sign-release.ps1",
                     "-Path", shipped, "-Scratch", work / "signing"])
                (work / "authenticode.bin").write_bytes(shipped.read_bytes()[chosen["installer"]["size"]:])
                say(leak_gate([work / "authenticode.bin"], work))
        with clock.step("Updater signature"):
            signature = sign_for_updates(shipped, version)
    with clock.step("Smoke of the signed installer"):
        say(f"Verified: signed by key {verify_signature(shipped, signature, version, public_key)} for version {version}.")
        release_smoke.installed_layout(shipped, work / "smoke", chosen["shipped"])
        (dist / "SHA256SUMS").write_text(f"{sha256(shipped)}  {shipped.name}\n", encoding="utf-8", newline="\n")
    url = f"https://github.com/{repo}/releases/download/{tag}/{shipped.name}"
    with clock.step("Notes and feed"):
        if existing:
            notes = existing["body"].split("\nBuilt from commit ")[0].rstrip() + "\n"
            (work / "notes.md").write_text(notes, encoding="utf-8", newline="\n")
        else:
            since = previous_commit(repo, channel) or (feed["commit"] if feed else "")
            write_notes(repo, channel, since, commit, work / "notes.md")
        write_manifest(version, signature, url, work / "notes.md", commit, work / "latest.json")
        (work / "body.md").write_text((work / "notes.md").read_text(encoding="utf-8") + f"\nBuilt from commit {commit}. "
                                      f"Signed installer SHA-256: {sha256(shipped)}.\n", encoding="utf-8", newline="\n")
        say(leak_gate(["latest.json", "notes.md", "body.md", "dist/SHA256SUMS", f"dist/{signature.name}"], work))
    if not existing:
        assets = [shipped, signature, dist / "SHA256SUMS"]
        name = release_name(channel, version)
        with clock.step("Publish the installer"):
            if draft:
                title = name if channel == "preview" else f"Dry run, delete: {name}"
                run(["gh", "release", "create", tag, "--repo", repo, "--draft", "--prerelease", "--target", commit,
                     "--title", title, "--notes-file", work / "body.md", *assets])
            else:
                flags = ["--prerelease", "--latest=false"] if channel == "nightly" else ["--latest"]
                run(["gh", "release", "create", tag, "--repo", repo, "--target", commit, "--title", name,
                     "--notes-file", work / "body.md", *flags, *assets])
            verify_published(repo, tag, draft, assets)
            if not draft and tag_commit(repo, tag) != commit:
                raise Refused(f"GitHub tagged {tag} on another commit than {commit[:12]}.")
    if channel == "preview":
        page = release_of(repo, tag, True)["html_url"]
    else:
        with clock.step("Publish the feed"):
            if feed != read_feed(repo, channel):
                raise Refused(f"The {channel} feed changed during this release; run it again to check against it.")
            feed_release = tag if args.draft else channel
            if not args.draft and not released(repo, channel):
                run(["gh", "release", "create", channel, "--repo", repo, "--target", commit, "--prerelease", "--latest=false",
                     "--title", f"{channel.capitalize()} update feed", "--notes",
                     f"latest.json, which Battle Plan and Battle Plan Recorder on the {channel} channel check for updates. Not an installer."])
            run(["gh", "release", "upload", feed_release, "--repo", repo, "--clobber", work / "latest.json"])
            page = verify_published(repo, feed_release, args.draft, [work / "latest.json"])
        if not args.draft:
            with clock.step("Feed check"):
                offered, newest = read_feed(repo, channel), newest_release(repo, channel)
                if not offered or offered["version"] != version or newest != version:
                    raise Refused(f"The {channel} feed offers {offered and offered['version']}, and its newest release is "
                                  f"{newest}, not both {version}.")
    shutil.rmtree(work, ignore_errors=True)
    lines = [f"{step}: {seconds} s" for step, seconds in clock.steps]
    lines.append(f"{'Drafted' if draft else 'Published'} {version} in {clock.total()} s: {page}")
    say("\n".join(lines))
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write("\n".join(f"- {line}" for line in lines) + "\n")
    return 0


def finalize(args):
    """After a stable release, main's workspace version becomes it, through a pull request merged at
    once, because main takes changes only through pull requests."""
    repo = repository()
    git("fetch", "--quiet", "origin", "main")
    if workspace_version(git("show", "origin/main:Cargo.toml")) == args.version:
        say(f"Main's workspace version is already {args.version}.")
        return 0
    branch = f"release/v{args.version}-workspace-version"
    git("switch", "--quiet", "--force-create", branch, "origin/main")
    stamp(ROOT, args.version)
    git("-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com",
        "commit", "--quiet", "--all", "--message", f"Release: v{args.version} is the workspace version")
    git("push", "--quiet", "--force", "origin", f"HEAD:refs/heads/{branch}")
    pull = run(["gh", "pr", "create", "--repo", repo, "--base", "main", "--head", branch,
                "--title", f"Release: v{args.version} is the workspace version",
                "--body", f"v{args.version} is released, so the nightlies now preview the version after it."]).stdout.strip()
    merged = run(["gh", "pr", "merge", pull, "--repo", repo, "--merge", "--delete-branch"], check=False)
    if merged.returncode != 0:
        say(f"::warning::{pull} could not be merged ({merged.stderr.strip()}); merge it to finish v{args.version}.")
        return 1
    say(f"Main's workspace version is now {args.version}: {pull}.")
    return 0


def verify(args):
    public_key = Path(args.public_key or PUBLIC_KEY).read_text(encoding="utf-8")
    say(f"Verified: signed by key {verify_signature(args.installer, args.signature, args.version, public_key)} "
        f"for version {args.version}.")
    return 0


def newer(args):
    refuse_older(args.version, args.than, "which is already published")
    say(f"{args.version} is newer than {args.than}.")
    return 0


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("version")
    command.add_argument("--channel", required=True, choices=CHANNELS)
    command.add_argument("--sha")
    command.add_argument("--version")
    command.add_argument("--github-output", action="store_true")
    command.set_defaults(handler=version)
    command = commands.add_parser("stamp")
    command.add_argument("version")
    command.set_defaults(handler=stamp_command)
    command = commands.add_parser("candidate")
    for name in ("installer", "version", "commit", "output"):
        command.add_argument(f"--{name}", required=True)
    command.add_argument("--channel", required=True, choices=CHANNELS)
    command.set_defaults(handler=candidate)
    command = commands.add_parser("resolve")
    command.add_argument("--channel", required=True, choices=CHANNELS)
    command.add_argument("--sha", default="")
    command.add_argument("--version", default="")
    command.add_argument("--scheduled", action="store_true")
    command.add_argument("--github-output", action="store_true")
    command.set_defaults(handler=resolve)
    command = commands.add_parser("publish")
    command.add_argument("--channel", required=True, choices=CHANNELS)
    command.add_argument("--candidate", required=True)
    command.add_argument("--draft", action="store_true")
    command.add_argument("--public-key")
    command.set_defaults(handler=publish)
    command = commands.add_parser("finalize")
    command.add_argument("--version", required=True)
    command.set_defaults(handler=finalize)
    command = commands.add_parser("verify")
    for name in ("installer", "signature", "version"):
        command.add_argument(f"--{name}", required=True)
    command.add_argument("--public-key")
    command.set_defaults(handler=verify)
    command = commands.add_parser("newer")
    command.add_argument("--than", required=True)
    command.add_argument("version")
    command.set_defaults(handler=newer)
    args = parser.parse_args()
    try:
        sys.exit(args.handler(args))
    except Refused as error:
        print(error, file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
