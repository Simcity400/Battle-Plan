"""Retired: `python tools/ci.py check` replaced it. This forwards its arguments there."""
import subprocess
import sys
from pathlib import Path

if __name__ == "__main__":
    sys.exit(subprocess.call([sys.executable, str(Path(__file__).with_name("ci.py")), "check", *sys.argv[1:]]))
