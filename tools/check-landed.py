"""Refuse map work that can change a paused picture without telling anyone (Python 3.11+).

usage: python tools/check-landed.py

A paused map draws only when something tells it to. Below the map's side, a result reaches the
picture by moving `viewer_layer_core::landed` (`landed::channel` or `landed::moved()`); in the
window, by an egui repaint. Every first-party file of the map crates, tests aside, that starts a
thread (named or not, scoped or not) or hands work to a pool must therefore use `landed::`
(side crates) or `landed::` or `request_repaint` (window crates). `EXEMPT` names each file that
does neither and why its work cannot change the paused picture or is learned another way. This is
a text check: it proves a signal is present in the file, not that every publication moves it.
Exits 1 on any failure.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
STARTS = re.compile(r"thread::Builder::new\(\)|thread::spawn\(|spawn_carried\(|spawn_scoped\(|try_submit\(")
SIDE = ["viewer-layer-core", "viewer-layers-*", "viewer-playback"]
WINDOW = ["viewer", "viewer-native-gui", "viewer-combat"]
EXEMPT = {
    "crates/viewer-layers-infrastructure/src/sounds.rs": "the battle mix makes sound only",
    "crates/viewer-layer-core/src/playhead_host/views/apart.rs": "a test and probe driver that steps its own store",
    "crates/viewer-playback/src/overlay.rs": "the overlay maker moves its own count, Shared::published",
    "crates/viewer-playback/src/engaged_index/lanes.rs": "coverage is compared by the side each rest, MapSide::covering",
    "crates/viewer-playback/src/perf_log.rs": "writes the perf log",
    "crates/viewer-playback/src/worker_pool.rs": "the pool itself; each submitter signals",
    "crates/viewer/src/app/fly_aircraft.rs": "the caller passes the window repaint in",
    "crates/viewer/src/app/playback_cost.rs": "the playback cost probe drives its own frames",
    "crates/viewer/src/app/playback_cost/extra_maps.rs": "the playback cost probe drives its own frames",
    "crates/viewer/src/video_export.rs": "renders an export, not the window",
    "crates/viewer-playback/src/side.rs": "the side thread; each still turn calls the window wake, Shared::wake",
    "crates/viewer/src/host.rs": "starts the map's own thread",
    "crates/viewer-native-gui/src/lazy_icons.rs": "the art loader calls the wake the viewer sets, LazyIcons::set_wake",
}


def crates(globs):
    for pattern in globs:
        yield from sorted((ROOT / "crates").glob(pattern))


def checked(crate):
    source = crate / "src"
    for path in sorted(source.rglob("*.rs")):
        parts = path.relative_to(source).parts
        if path.name.endswith("tests.rs") or "tests" in parts or "test_support" in parts:
            continue
        yield path


def main():
    failed = []
    for globs, signals in ((SIDE, ("landed::",)), (WINDOW, ("landed::", "request_repaint"))):
        for crate in crates(globs):
            for path in checked(crate):
                name = path.relative_to(ROOT).as_posix()
                text = path.read_text(encoding="utf-8")
                if name in EXEMPT or not STARTS.search(text) or any(signal in text for signal in signals):
                    continue
                failed.append(f"{name}: starts work without {' or '.join(signals)}")
    for line in failed:
        print(line)
    print(f"landed: {len(failed)} file(s) start work that can land unseen")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
