#!/usr/bin/env python3
import base64
import fcntl
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tarfile
import time

ROOT = Path('/home/user/hoi4')
SRC = ROOT / 'src'
CLAIM = ROOT / 'claim'
# Beside the claim, which older runners release with rmdir: the token and boot of a job whose processes outlived it.
STRANDED = ROOT / 'stranded'
MANIFEST = ROOT / 'source-manifest.json'
TARGET = 'x86_64-pc-windows-msvc'


METADATA = ROOT / 'cache-mtimes.json.gz'
BOOT_MARKER = ROOT / 'cache-mtimes-boot'
# Release builds reuse any crate this VM compiled before from the same inputs, whichever lane built it:
# Cargo keeps one build of each crate, so 47-102 crates were rebuilt per build after another lane's (29 Oct).
SCCACHE_VERSION = 'v0.18.0'
SCCACHE_SHA256 = '45f1447fbe231e3037bde351ef70677dd212216c8d62ae7ca409fecc4d6acc89'
SCCACHE_HOME = ROOT / 'sccache'
SCCACHE = SCCACHE_HOME / SCCACHE_VERSION / 'sccache'
SCCACHE_ENV = {'SCCACHE_DIR': str(SCCACHE_HOME / 'cache'), 'SCCACHE_CACHE_SIZE': '16G', 'SCCACHE_IDLE_TIMEOUT': '900'}

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

def prune_incremental():
    # Every lane's checks leave incremental sessions behind (106 of 147 GB seen on one VM).
    usage = shutil.disk_usage(ROOT)
    if usage.used < usage.total * 0.7:
        return
    failed = []
    target = ROOT / 'target'
    for incremental in [*target.glob('*/incremental'), *target.glob('*/*/incremental')]:
        shutil.rmtree(incremental, onerror=lambda function, path, error: failed.append(path))
    after = shutil.disk_usage(ROOT)
    note = f'; {len(failed)} paths could not be removed' if failed else ''
    print(f'Pruned incremental caches: disk {usage.used * 100 // usage.total}% -> {after.used * 100 // after.total}%{note}', file=sys.stderr, flush=True)

def start_cache(token):
    """Starts the compile cache's server outside the job's process group; the environment of its clients, or None and a warning."""
    try:
        if not SCCACHE.is_file():
            # A failed download is retried an hour later, not by every job.
            failed = SCCACHE_HOME / 'download-failed'
            if failed.exists() and time.time() - failed.stat().st_mtime < 3600:
                raise RuntimeError(f'the download failed within the hour: {failed.read_text().strip()}')
            name = f'sccache-{SCCACHE_VERSION}-x86_64-unknown-linux-musl'
            url = f'https://github.com/mozilla/sccache/releases/download/{SCCACHE_VERSION}/{name}.tar.gz'
            try:
                data = subprocess.run(['curl', '-fsSL', '--connect-timeout', '10', '--max-time', '120', url], capture_output=True, check=True).stdout
                if hashlib.sha256(data).hexdigest() != SCCACHE_SHA256:
                    raise RuntimeError('checksum mismatch')
                with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
                    binary = archive.extractfile(f'{name}/sccache').read()
            except Exception as error:
                SCCACHE_HOME.mkdir(exist_ok=True)
                failed.write_text(repr(error))
                raise
            SCCACHE.parent.mkdir(parents=True, exist_ok=True)
            partial = SCCACHE.with_suffix('.part')
            partial.write_bytes(binary)
            partial.chmod(0o755)
            partial.replace(SCCACHE)
        environment = {**os.environ, **SCCACHE_ENV}
        # Until its stop is confirmed the server may write into target outside Cargo's group, where
        # older helpers' busy checks miss it: the claim is marked stranded ahead, which no helper
        # takes over once this runner is gone.
        STRANDED.write_text(f'{token}\n{boot_id()}')
        # A server an interrupted job left behind would hold the port with its own settings.
        subprocess.run([str(SCCACHE), '--stop-server'], env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
        # The server inherits these handles: a pipe would keep this job's output open while it runs.
        with (SCCACHE_HOME / 'server.log').open('w') as log:
            subprocess.run([str(SCCACHE), '--start-server'], env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=log, timeout=60, check=True)
        return environment
    except Exception as error:
        print(f'WARNING: no compile cache this run: {error!r}', file=sys.stderr, flush=True)
        return None

def cache_processes():
    """Live cache servers and the compilers they run, outside any job's process group."""
    found = []
    for stat_file in Path('/proc').glob('[0-9]*/stat'):
        try:
            # comm is field 2, in parentheses; the state follows it.
            text = stat_file.read_text()
            comm, state = text[text.index('(') + 1:text.rindex(')')], text[text.rindex(')') + 2:][:1]
        except (OSError, ValueError):
            continue
        if comm in {'sccache', 'rustc'} and state != 'Z':
            found.append(int(stat_file.parent.name))
    return found

def stop_cache():
    """Stops the cache's server, printing its hits and misses; False while it or a compiler it ran still runs."""
    try:
        if SCCACHE.is_file():
            environment = {**os.environ, **SCCACHE_ENV}
            with (SCCACHE_HOME / 'stop.log').open('w') as log:
                subprocess.run([str(SCCACHE), '--stop-server'], env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=log, timeout=60)
            stats = [line for line in (SCCACHE_HOME / 'stop.log').read_text().splitlines() if line.startswith(('Compile requests ', 'Cache hits ', 'Cache misses ', 'Non-cacheable calls', 'Cache size'))]
            print('Compile cache: ' + '; '.join(' '.join(line.split()) for line in stats), file=sys.stderr, flush=True)
    except Exception as error:
        print(f'WARNING: the compile cache did not stop cleanly: {error!r}', file=sys.stderr, flush=True)
    # The stop does not wait: a server restoring a hit still writes into target, and its compilers
    # run outside Cargo's group. Only this job runs on the VM, so every one of them is its own.
    for wait, kill in ((30, True), (5, False)):
        deadline = time.monotonic() + wait
        while cache_processes() and time.monotonic() < deadline:
            time.sleep(0.2)
        for pid in cache_processes() if kill else ():
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    return not cache_processes()

def drain(process):
    """True once the job's process group is gone within five seconds; it is killed only while its leader is unreaped."""
    # A reaped leader frees its ID for reuse, so a group outliving its leader is never signalled, only kept claimed.
    deadline = time.monotonic() + 5
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=max(0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            return False
    while time.monotonic() < deadline:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            return True
        except PermissionError:
            pass
        time.sleep(0.1)
    return False

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

def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text()

def compiling():
    """True while Cargo, a compiler or a compile cache a job left behind still runs."""
    for comm in Path('/proc').glob('[0-9]*/comm'):
        try:
            if comm.read_text().strip() in {'cargo', 'cargo-xwin', 'rustc', 'sccache'}:
                return True
        except OSError:
            pass
    return False

def receive(spec, token):
    """The uploaded source bundle, verified and unpacked; returns its directory."""
    kind, name, digest = spec.split(':')
    if kind == 'parts':
        parts = sorted(ROOT.glob(name + '.b64-*'))
        data = base64.b64decode(''.join(part.read_text() for part in parts))
    else:
        parts = [ROOT / name]
        data = parts[0].read_bytes()
    for part in parts:
        part.unlink()
    if hashlib.sha256(data).hexdigest() != digest:
        raise RuntimeError('Source bundle checksum mismatch')
    incoming = ROOT / ('incoming-' + token)
    incoming.mkdir(exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:xz') as archive:
        for member in archive.getmembers():
            path = Path(member.name)
            if not member.isfile() or path.is_absolute() or '..' in path.parts or path.parts[0] not in {'files', 'delta.json'}:
                raise RuntimeError(f'Unexpected source bundle entry: {member.name}')
            destination = incoming / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, destination.open('wb') as output:
                shutil.copyfileobj(source, output)
    return incoming

def apply(incoming):
    """Brings the tree to the uploaded manifest; the caller holds the claim and the running lock."""
    delta = json.loads((incoming / 'delta.json').read_text())
    old = manifest()
    new = {name: digest for name, digest in old.items() if name not in set(delta['deleted'])}
    new.update(delta['changed'])
    if hashlib.sha256(json.dumps(new, sort_keys=True).encode()).hexdigest() != delta['digest']:
        raise RuntimeError('The tree and the uploaded changes do not add up to the worktree')
    changed = [name for name, digest in delta['changed'].items() if old.get(name) != digest]
    for name in changed:
        if hashlib.sha256((incoming / 'files' / name).read_bytes()).hexdigest() != new[name]:
            raise RuntimeError(f'Upload checksum mismatch: {name}')
    for name in new:
        safe_path(name)
    # Nothing has changed yet; from here an interrupted sync leaves no manifest, so the next run rehashes the tree.
    MANIFEST.unlink(missing_ok=True)
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
        shutil.copyfile(incoming / 'files' / name, path)
    temporary = MANIFEST.with_suffix('.tmp')
    temporary.write_text(json.dumps(new))
    temporary.replace(MANIFEST)
    print(f'Source synchronized: {len(changed)} changed, {len(old.keys() - new.keys())} deleted, {len(new)} total', file=sys.stderr, flush=True)

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
        # The caller holds the VM's local lock, so the claim's own run is gone; only its remote work may remain.
        holder = (CLAIM / 'token').read_text() if (CLAIM / 'token').exists() else None
        with (ROOT / 'running.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                # Its runner still runs, and releases the claim or marks it stranded itself.
                # Older helpers quarantine on 73; this one reads the marker and only passes the VM by.
                print('busy')
                print('VM is still finishing a detached job', file=sys.stderr)
                sys.exit(73)
            # Its runner is gone. A claim it marked stranded, ahead of a compile cache or for
            # processes it could not stop, is never taken over: they may still write into target.
            if STRANDED.exists() and STRANDED.read_text() == f'{holder}\n{boot_id()}':
                print('VM holds processes a job could not stop; refusing to overlap', file=sys.stderr)
                sys.exit(73)
            if compiling():
                print('busy')
                print('VM is still finishing a detached job', file=sys.stderr)
                sys.exit(73)
            # Its run may have stopped mid-sync: without a manifest the tree is rehashed below.
            MANIFEST.unlink(missing_ok=True)
            shutil.rmtree(CLAIM)
            CLAIM.mkdir()
            print('Took over the claim of a run that ended without releasing it', file=sys.stderr)
    STRANDED.unlink(missing_ok=True)
    (CLAIM / 'token').write_text(token)
    # Earlier runs' downloads and uploads are done with by now.
    for leftover in [*ROOT.glob('artifacts-*'), *ROOT.glob('incoming-*'), *ROOT.glob('runner-*.b64-*')]:
        if leftover.is_dir() and not leftover.is_symlink():
            shutil.rmtree(leftover)
        else:
            leftover.unlink()
    restore_metadata()
    print(json.dumps(manifest()))
elif mode == 'release':
    token = sys.argv[2]
    if CLAIM.exists() and owned(token):
        with (ROOT / 'running.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                sys.exit(73)
            shutil.rmtree(CLAIM)
elif mode in {'check', 'build', 'ui'}:
    token = sys.argv[2]
    assert owned(token)
    args = sys.argv[3:]
    source = None
    if args[:1] == ['--sync']:
        source, args = args[1], args[2:]
    memory_gib = os.sysconf('SC_PAGE_SIZE') * os.sysconf('SC_PHYS_PAGES') / (1024 ** 3)
    # Releasing recorder, collector and viewer with 8 jobs left 1.2 of 15.6 GiB free on a large VM, so builds run 7;
    # a workspace check with 7 left 8.3 GiB free (28 Oct), so checks run one per vCPU.
    os.environ['CARGO_BUILD_JOBS'] = str(max(1, min(os.cpu_count() or 1, int(memory_gib / (2.2 if mode == 'build' else 1.2)))))
    with (ROOT / 'running.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = None
        launched = False
        cache = None
        cache_started = False
        try:
            if source:
                incoming = None
                try:
                    incoming = receive(source, token)
                    apply(incoming)
                except Exception as error:
                    print(f'Source synchronization failed: {error!r}', file=sys.stderr, flush=True)
                    sys.exit(79)
                finally:
                    if incoming:
                        shutil.rmtree(incoming, ignore_errors=True)
            if mode == 'ui' or (mode == 'build' and any('custom-protocol' in arg for arg in args)):
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
                if mode == 'build':
                    subprocess.run(['npm', 'run', 'build:ui'], cwd=SRC, check=True)
            if mode != 'ui':
                prune_incremental()
                command = ['cargo', 'xwin', *args, '--target', TARGET]
                if mode == 'build':
                    command.append('--message-format=json-render-diagnostics')
                    # Checks compile workspace crates incrementally, which the cache passes through.
                    cache_started = True
                    cache = start_cache(token)
            print('Remote command: ' + ' '.join(command), file=sys.stderr, flush=True)
            if mode != 'build':
                launched = True
                result = subprocess.Popen(command, cwd=SRC, start_new_session=True)
                result.wait()
            else:
                artifacts = []
                launched = True
                environment = {**cache, 'RUSTC_WRAPPER': str(SCCACHE)} if cache else None
                result = subprocess.Popen(command, cwd=SRC, stdout=subprocess.PIPE, text=True, start_new_session=True, env=environment)
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
                        # Level 6 on four threads took 28 s for the viewer; level 2 on every core is 16% larger and takes seconds.
                        packer = subprocess.Popen(['xz', '-T0', '-2', '-c'], stdin=subprocess.PIPE, stdout=compressed)
                        with tarfile.open(fileobj=packer.stdin, mode='w|') as archive:
                            for artifact in out:
                                path = Path(artifact['path'])
                                archive.add(path, arcname=path.name, recursive=False)
                        packer.stdin.close()
                        if packer.wait():
                            raise RuntimeError('Artifact compression failed')
                    print('BOAT_ARTIFACTS=' + json.dumps(out), flush=True)
                    print('BOAT_BUNDLE=' + json.dumps({'path': str(bundle), 'size': bundle.stat().st_size, 'sha256': hashlib.sha256(bundle.read_bytes()).hexdigest()}), flush=True)
            sys.exit(result.returncode)
        finally:
            # The job's command group (Cargo, npm check) must not outlive its claim: the next job prunes incremental files under it.
            # A launch interrupted before Popen returned may have left Cargo running without a handle.
            drained = not launched or (result is not None and drain(result))
            # A start that failed part way may still have left a server.
            if cache_started:
                stopped = stop_cache()
                # A server a compile started in Cargo's place, had ours gone, would keep Cargo's group alive.
                if stopped and not drained and result is not None:
                    drained = drain(result)
                drained = drained and stopped
            checkpoint()
            if not drained:
                print('WARNING: processes of this job are still running; the claim stays for quarantine', file=sys.stderr, flush=True)
                if CLAIM.exists() and owned(token):
                    STRANDED.write_text(f'{token}\n{boot_id()}')
            elif CLAIM.exists() and owned(token):
                STRANDED.unlink(missing_ok=True)
                shutil.rmtree(CLAIM)
else:
    raise RuntimeError(f'Unknown mode: {mode}')
