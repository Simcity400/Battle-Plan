"""Refuse nextest filters that name packages the workspace does not have (Python 3.11+).

usage: python tools/check-nextest-config.py

Reads every `filter` and `default-filter` in .config/nextest.toml and checks that each package
selector in them, `package(...)`, `deps(...)` or `rdeps(...)`, matches at least one workspace
member, the way nextest matches it: `=name` exactly, `~text` as a substring, `/regex/` anywhere
in the name, and `#glob` or a bare matcher as a glob; a regex Python cannot read, or a glob with alternatives or escapes, counts as matching. Nextest refuses the whole run when one
does not, before it compiles anything. Members come from the root Cargo.toml's `members` and
`exclude`; nothing is compiled or resolved. Prints `<selector>: no workspace package matches`
per finding and exits 1 on any.
"""
import fnmatch
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONFIG = Path(".config") / "nextest.toml"
FUNCTION = re.compile(r"\b(package|deps|rdeps|test|kind|binary|binary_id|platform|group)\s*\(\s*")
PACKAGES = {"package", "deps", "rdeps"}
FILTER_KEYS = {"filter", "default-filter"}


def members(root):
    """The package name of each workspace member."""
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    excluded = {(root / path).resolve() for path in workspace.get("exclude", [])}
    names = set()
    for pattern in workspace.get("members", []):
        for folder in sorted(root.glob(pattern)):
            manifest = folder / "Cargo.toml"
            if folder.resolve() in excluded or not manifest.is_file():
                continue
            package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package")
            if package:
                names.add(package["name"])
    return names


def filters(config):
    """Every filterset string in a parsed nextest config."""
    if isinstance(config, dict):
        for key, value in config.items():
            if key in FILTER_KEYS and isinstance(value, str):
                yield value
            else:
                yield from filters(value)
    elif isinstance(config, list):
        for item in config:
            yield from filters(item)


def selectors(text):
    """Each package selector in a filterset: (its text, its matcher). Every predicate's matcher is
    read whole, so a selector's name inside another's is not one; operators and groups are not
    predicates."""
    at = 0
    while found := FUNCTION.search(text, at):
        start = found.end()
        if text.startswith("/", start):
            end = start + 1
            while end < len(text) and text[end] != "/":
                end += 2 if text[end] == "\\" else 1
            close = text.find(")", end)
        else:
            close = text.find(")", start)
        if close < 0:
            return
        if found[1] in PACKAGES:
            yield text[found.start():close + 1], text[start:close].strip()
        at = close + 1


def matches(matcher, name):
    if matcher.startswith("="):
        return name == matcher[1:]
    if matcher.startswith("~"):
        return matcher[1:] in name
    if len(matcher) > 1 and matcher.startswith("/") and matcher.endswith("/"):
        try:
            return re.search(matcher[1:-1], name) is not None
        except re.error:
            return True
    glob = matcher.removeprefix("#")
    if "{" in glob or "\\" in glob:
        return True
    return fnmatch.fnmatchcase(name, glob)


def findings(config, names):
    """Each selector in the config that matches no package in `names`, once."""
    seen = []
    for text in filters(config):
        for selector, matcher in selectors(text):
            if selector not in seen and not any(matches(matcher, name) for name in names):
                seen.append(selector)
    return seen


def main(root=ROOT):
    path = root / CONFIG
    if not path.is_file():
        return 0
    found = findings(tomllib.loads(path.read_text(encoding="utf-8")), members(root))
    for selector in found:
        print(f"{CONFIG.as_posix()}: {selector}: no workspace package matches")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
