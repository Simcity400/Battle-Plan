#!/usr/bin/env bash
set -euo pipefail
# Builds the hoi4-check image tools/boat.py clones: boat new --type large --ttl 14400 --no-env;
# boat push ID CLEAN_MAIN_WORKTREE --to /home/user/hoi4/src; boat scp tools/boat_setup.sh ID:/home/user/setup.sh;
# boat exec ID --timeout 0 --stream -- bash /home/user/setup.sh SOURCE_SHA; boat snapshot ID hoi4-check; boat extend ID --ttl 120
ROOT=/home/user/hoi4
SRC=$ROOT/src
[[ -f $SRC/rust-toolchain.toml ]] || { echo "Push the clean main worktree to $SRC first" >&2; exit 1; }
sudo apt-get update
sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y clang-19 lld-19 llvm-19 cmake ninja-build pkg-config curl python3-venv xz-utils
export CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup
export PATH=/usr/local/cargo/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH
TOOLCHAIN=$(python3 -c 'import tomllib; print(tomllib.load(open("/home/user/hoi4/src/rust-toolchain.toml", "rb"))["toolchain"]["channel"])')
rustup toolchain install "$TOOLCHAIN" --profile minimal --target x86_64-pc-windows-msvc
rustup component add --toolchain "$TOOLCHAIN" llvm-tools
python3 -m venv "$ROOT/tools"
"$ROOT/tools/bin/pip" install cargo-xwin==0.23.1
[[ ! -L "$ROOT/tools/bin/clang-cl" ]] || unlink "$ROOT/tools/bin/clang-cl"
cat > "$ROOT/tools/bin/clang-cl" <<'BOAT_CLANG_CL'
#!/usr/bin/env python3
import os
import sys
# libudis86 requests GCC's forced-include spelling even with the CL driver.
args = ['/FIstring.h' if arg == '-includestring.h' else arg for arg in sys.argv[1:]]
os.execv('/usr/bin/clang-19', ['clang-cl', '--driver-mode=cl', *args])
BOAT_CLANG_CL
chmod +x "$ROOT/tools/bin/clang-cl"
mkdir -p "$ROOT/target" "$ROOT/xwin" "$ROOT/npm"
cat > "$ROOT/env.sh" <<'ENV'
export PATH=/home/user/hoi4/node/bin:/home/user/hoi4/tools/bin:/usr/lib/llvm-19/bin:/usr/local/cargo/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH
export CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup
export CARGO_TARGET_DIR=/home/user/hoi4/target
export XWIN_CACHE_DIR=/home/user/hoi4/xwin
export XWIN_ARCH=x86_64
export CARGO_BUILD_JOBS=6

ENV

cp "$SRC/tools/boat_remote.py" "$ROOT/remote.py"
cat > "$ROOT/run.sh" <<'BOAT_REMOTE_RUN'
#!/usr/bin/env bash
set -euo pipefail
source /home/user/hoi4/env.sh
exec python3 /home/user/hoi4/remote.py "$@"
BOAT_REMOTE_RUN
source "$ROOT/env.sh"
cd "$SRC"
python3 - <<'MANIFEST'
import hashlib, json, os
from pathlib import Path
root = Path('/home/user/hoi4/src')
result = {}
for base, dirs, files in os.walk(root):
    dirs[:] = [name for name in dirs if name not in {'.git', 'node_modules', '.cargo-target', 'target'}]
    for name in files:
        path = Path(base) / name
        result[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
Path('/home/user/hoi4/source-manifest.json').write_text(json.dumps(result))
MANIFEST
cargo xwin --version
cargo xwin cache xwin
if ! node -e 'const [a,b]=process.versions.node.split(".").map(Number); process.exit(a>22||(a===22&&b>=12)?0:1)' 2>/dev/null; then
  mkdir -p "$ROOT/node"
  curl -fsSLo "$ROOT/node.tar.xz" https://nodejs.org/dist/v24.22.1/node-v24.22.1-linux-x64.tar.xz
  curl -fsS https://nodejs.org/dist/v24.22.1/SHASUMS256.txt | grep ' node-v24.22.1-linux-x64.tar.xz$' | sed "s@node-v24.22.1-linux-x64.tar.xz@$ROOT/node.tar.xz@" | sha256sum --check -
  tar -xJf "$ROOT/node.tar.xz" -C "$ROOT/node" --strip-components=1
fi
node --version
npm ci --no-audit --no-fund
npm run check:ui
if [[ ! -L node_modules ]]; then
  [[ ! -e "$ROOT/npm/node_modules" ]] || { echo "Existing node modules: use a fresh VM" >&2; exit 1; }
  mv node_modules "$ROOT/npm/node_modules"
fi
[[ -L node_modules ]] || ln -s "$ROOT/npm/node_modules" node_modules
sha256sum package-lock.json | cut -d" " -f1 > "$ROOT/node-lock.sha256"
/usr/bin/time -v cargo xwin check --workspace --all-targets --target x86_64-pc-windows-msvc
# The release snapshot covers all four operational artifacts, with Cargo's normal release profile.
/usr/bin/time -v cargo xwin build --release -p dashboard --bin preparation-cost -p viewer --bin viewer -p recorder --bin recorder -p collector --lib --target x86_64-pc-windows-msvc
python3 "$ROOT/remote.py" checkpoint
printf '%s\n' "${1:-source revision not supplied}" > "$ROOT/source-revision"
{ rustc -vV; cargo xwin --version; clang-cl --version; lld-link --version; node --version; cat "$ROOT/xwin/xwin/DONE"; cargo xwin env --target x86_64-pc-windows-msvc; } > "$ROOT/toolchain.txt"
# Save after this exits: boat snapshot ID hoi4-check; boat extend ID --ttl 120.