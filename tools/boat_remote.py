#!/usr/bin/env python3
import fcntl
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

ROOT = Path('/home/user/hoi4')
SRC = ROOT / 'src'
CLAIM = ROOT / 'claim'
MANIFEST = ROOT / 'source-manifest.json'
TARGET = 'x86_64-pc-windows-msvc'


METADATA = ROOT / 'cache-mtimes.json.gz'
BOOT_MARKER = ROOT / 'cache-mtimes-boot'

def checkpoint():
    mtimes = {}
    for root in (SRC, ROOT / 'target', Path('/usr/local/cargo/registry/src')):
        for base, dirs, files in os.walk(root, followlinks=False):
            if root == SRC:
                dirs[:] = [name for name in dirs if name not in {'.git', 'node_modules', '.cargo-target'}]
            for name in ['', *dirs, *files]:
                path = Path(base) / name if name else Path(base)
                try:
                    mtimes[str(path)] = path.lstat().st_mtime_ns
                except FileNotFoundError:
                    pass
    with gzip.open(str(METADATA) + '.tmp', 'wt') as output:
        json.dump(mtimes, output)
    Path(str(METADATA) + '.tmp').replace(METADATA)
    BOOT_MARKER.write_text(Path('/proc/sys/kernel/random/boot_id').read_text())

def restore_metadata():
    boot = Path('/proc/sys/kernel/random/boot_id').read_text()
    if METADATA.exists() and (not BOOT_MARKER.exists() or BOOT_MARKER.read_text() != boot):
        with gzip.open(METADATA, 'rt') as source:
            mtimes = json.load(source)
        for name, stamp in mtimes.items():
            path = Path(name)
            if path.exists() or path.is_symlink():
                try:
                    os.utime(path, ns=(stamp, stamp), follow_symlinks=False)
                except FileNotFoundError:
                    pass
        BOOT_MARKER.write_text(boot)
        print(f'Restored {len(mtimes)} cached mtimes after clone/resume', file=sys.stderr)

def manifest():
    if MANIFEST.exists():
        expected = json.loads(MANIFEST.read_text())
        return {name: digest if (SRC / name).is_file() and not (SRC / name).is_symlink() and hashlib.sha256((SRC / name).read_bytes()).hexdigest() == digest else None for name, digest in expected.items()}
    result = {}
    for base, dirs, files in os.walk(SRC):
        dirs[:] = [d for d in dirs if d not in {'.git', 'node_modules', '.cargo-target', 'target'}]
        for name in files:
            path = Path(base) / name
            result[path.relative_to(SRC).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    MANIFEST.write_text(json.dumps(result))
    return result

def owned(token):
    return (CLAIM / 'token').read_text() == token

def safe_path(name):
    p = SRC / name
    if not name or name.startswith('/') or '..' in Path(name).parts or p.is_symlink():
        raise RuntimeError(f'Unsafe source path: {name}')
    parent = p.parent
    while parent != SRC:
        if parent.is_symlink():
            raise RuntimeError(f'Symlinked source directory: {parent}')
        parent = parent.parent
    return p

mode = sys.argv[1]
if mode == 'checkpoint':
    checkpoint()
    print('Cache metadata checkpoint saved')
elif mode == 'provenance':
    commands = [['rustc', '-vV'], ['cargo', 'xwin', '--version'], ['clang-cl', '--version'], ['lld-link', '--version'], ['node', '--version'], ['cargo', 'xwin', 'env', '--target', TARGET]]
    text = ''
    for command in commands:
        result = subprocess.run(command, cwd=SRC, capture_output=True, text=True, check=True)
        text += '$ ' + ' '.join(command) + '\n' + result.stdout + result.stderr + '\n'
    text += (ROOT / 'xwin' / 'xwin' / 'DONE').read_text()
    (ROOT / 'toolchain.txt').write_text(text)
    print(text)
elif mode == 'acquire':
    token = sys.argv[2]
    try:
        CLAIM.mkdir()
    except FileExistsError:
        print('VM has an existing remote claim; refusing to overlap', file=sys.stderr)
        sys.exit(73)
    (CLAIM / 'token').write_text(token)
    restore_metadata()
    print(json.dumps(manifest()))
elif mode == 'split-bundle':
    token = sys.argv[2]
    if len(token) != 32 or any(char not in '0123456789abcdef' for char in token):
        raise RuntimeError('Invalid bundle token')
    bundle = ROOT / ('artifacts-' + token + '.tar.xz')
    chunk = max(1, (bundle.stat().st_size + 3) // 4)
    parts = []
    with bundle.open('rb') as source:
        for index in range(4):
            data = source.read(chunk)
            if not data:
                break
            part = ROOT / ('artifacts-' + token + '.part-' + str(index))
            part.write_bytes(data)
            parts.append(str(part))
    print(json.dumps(parts))
elif mode == 'discard-bundle':
    token = sys.argv[2]
    if len(token) != 32 or any(char not in '0123456789abcdef' for char in token):
        raise RuntimeError('Invalid bundle token')
    (ROOT / ('artifacts-' + token + '.tar.xz')).unlink(missing_ok=True)
    for index in range(4):
        (ROOT / ('artifacts-' + token + '.part-' + str(index))).unlink(missing_ok=True)
elif mode == 'release':
    token = sys.argv[2]
    if CLAIM.exists() and owned(token):
        with (ROOT / 'running.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                sys.exit(73)
            (CLAIM / 'token').unlink()
            CLAIM.rmdir()
elif mode == 'sync':
    token, incoming = sys.argv[2:4]
    assert owned(token)
    incoming = Path(incoming)
    new = json.loads((incoming / '__boat_manifest__.json').read_text())
    old = manifest()
    changed = [name for name, digest in new.items() if old.get(name) != digest]
    for name in changed:
        path = incoming / name
        if hashlib.sha256(path.read_bytes()).hexdigest() != new[name]:
            raise RuntimeError(f'Upload checksum mismatch: {name}')
    for name in new:
        safe_path(name)
    for name in old.keys() - new.keys():
        path = safe_path(name)
        if path.exists():
            path.unlink()
    old_dirs = {parent.as_posix() for name in old for parent in Path(name).parents if parent != Path('.')}
    new_dirs = {parent.as_posix() for name in new for parent in Path(name).parents if parent != Path('.')}
    for name in sorted(old_dirs - new_dirs, key=lambda name: name.count('/'), reverse=True):
        path = safe_path(name)
        if path.is_dir():
            shutil.rmtree(path)
    for name in changed:
        path = safe_path(name)
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(incoming / name, path)
    temporary = MANIFEST.with_suffix('.tmp')
    temporary.write_text(json.dumps(new))
    temporary.replace(MANIFEST)
    shutil.rmtree(incoming)
    print(f'Source synchronized: {len(changed)} changed, {len(old.keys() - new.keys())} deleted, {len(new)} total')
elif mode in {'check', 'build', 'ui'}:
    token = sys.argv[2]
    assert owned(token)
    args = sys.argv[3:]
    memory_gib = os.sysconf('SC_PAGE_SIZE') * os.sysconf('SC_PHYS_PAGES') / (1024 ** 3)
    os.environ['CARGO_BUILD_JOBS'] = str((4 if memory_gib > 10 else 2) if mode == 'build' else (6 if memory_gib > 10 else 3))
    with (ROOT / 'running.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            if mode == 'ui':
                package_hash = hashlib.sha256((SRC / 'package-lock.json').read_bytes()).hexdigest()
                stamp = ROOT / 'node-lock.sha256'
                modules = ROOT / 'npm' / 'node_modules'
                previous = ROOT / 'node_modules' / 'installed'
                modules.parent.mkdir(exist_ok=True)
                if previous.exists() and not modules.exists():
                    previous.rename(modules)
                link = SRC / 'node_modules'
                if link.is_symlink() and link.resolve() != modules:
                    link.unlink()
                if not stamp.exists() or stamp.read_text().strip() != package_hash:
                    if (SRC / 'node_modules').is_symlink():
                        (SRC / 'node_modules').unlink()
                    subprocess.run(['npm', 'ci', '--no-audit', '--no-fund'], cwd=SRC, check=True)
                    if modules.exists():
                        shutil.rmtree(modules)
                    (SRC / 'node_modules').rename(modules)
                    stamp.write_text(package_hash)
                if not (SRC / 'node_modules').exists():
                    (SRC / 'node_modules').symlink_to(modules, target_is_directory=True)
                command = ['npm', 'run', 'check:ui']
            else:
                command = ['cargo', 'xwin', *args, '--target', TARGET]
                if mode == 'build':
                    command.append('--message-format=json-render-diagnostics')
            print('Remote command: ' + ' '.join(command), file=sys.stderr, flush=True)
            if mode != 'build':
                result = subprocess.run(command, cwd=SRC)
            else:
                artifacts = []
                result = subprocess.Popen(command, cwd=SRC, stdout=subprocess.PIPE, text=True)
                for line in result.stdout:
                    try:
                        event = json.loads(line)
                    except json.JSONDecodeError:
                        print(line, end='', flush=True)
                        continue
                    if event.get('reason') == 'compiler-message':
                        print(event['message'].get('rendered', ''), end='', flush=True)
                    elif event.get('reason') == 'compiler-artifact':
                        has_debug = event.get('profile', {}).get('debuginfo') not in (None, 0, False, 'none')
                        for name in event.get('filenames', []):
                            path = Path(name)
                            if path.suffix.lower() in {'.exe', '.dll', '.pdb'} and 'deps' not in path.relative_to(ROOT / 'target').parts:
                                artifacts.append(str(path))
                                pdb = path.with_suffix('.pdb')
                                if has_debug and pdb.exists():
                                    artifacts.append(str(pdb))
                        executable = event.get('executable')
                        if has_debug and executable:
                            path = Path(executable).with_suffix('.pdb')
                            if path.exists():
                                artifacts.append(str(path))
                result.wait()
                if result.returncode == 0:
                    out = []
                    for name in sorted(set(artifacts)):
                        path = Path(name)
                        out.append({'path': name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'bytes': path.stat().st_size})
                    bundle = ROOT / ('artifacts-' + token + '.tar.xz')
                    with bundle.open('wb') as compressed:
                        packer = subprocess.Popen(['xz', '-T4', '-6', '-c'], stdin=subprocess.PIPE, stdout=compressed)
                        with tarfile.open(fileobj=packer.stdin, mode='w|') as archive:
                            for artifact in out:
                                path = Path(artifact['path'])
                                archive.add(path, arcname=path.name, recursive=False)
                        packer.stdin.close()
                        if packer.wait():
                            raise RuntimeError('Artifact compression failed')
                    print('BOAT_ARTIFACTS=' + json.dumps(out), flush=True)
                    print('BOAT_BUNDLE=' + str(bundle), flush=True)
            sys.exit(result.returncode)
        finally:
            checkpoint()
            if CLAIM.exists() and owned(token):
                (CLAIM / 'token').unlink()
                CLAIM.rmdir()
else:
    raise RuntimeError(f'Unknown mode: {mode}')