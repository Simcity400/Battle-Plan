#!/usr/bin/env python3
import base64
import errno
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
HOST = 'x86_64-unknown-linux-gnu'
TRANSIENT_ERRNOS = {errno.EAGAIN, errno.ENOMEM, errno.EINTR}
BROKEN_TOOLCHAIN = 72
UNPROVEN_TOOLCHAIN = 71
SRC = ROOT / 'src'
CLAIM = ROOT / 'claim'
STRANDED = ROOT / 'stranded'
MANIFEST = ROOT / 'source-manifest.json'
TARGET = 'x86_64-pc-windows-msvc'
TEST_TARGET = ROOT / 'target-test'


METADATA = ROOT / 'cache-mtimes.json.gz'
BOOT_MARKER = ROOT / 'cache-mtimes-boot'
SCCACHE_VERSION = 'v0.18.0'
SCCACHE_SHA256 = '45f1447fbe231e3037bde351ef70677dd212216c8d62ae7ca409fecc4d6acc89'
SCCACHE_HOME = ROOT / 'sccache'
SCCACHE = SCCACHE_HOME / SCCACHE_VERSION / 'sccache'
SCCACHE_ENV = {'SCCACHE_DIR': str(SCCACHE_HOME / 'cache'), 'SCCACHE_CACHE_SIZE': '16G', 'SCCACHE_IDLE_TIMEOUT': '900'}

def checkpoint():
    mtimes = {}
    for root in (SRC, ROOT / 'target', TEST_TARGET, Path('/usr/local/cargo/registry/src')):
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

def cargo_command(args, target, structured):
    """Cargo's command line, with Boat's own options ahead of any arguments for the program after `--`."""
    split = args.index('--') if '--' in args else len(args)
    owned = ['--target', target, *(['--message-format=json-render-diagnostics'] if structured else [])]
    return ['cargo', 'xwin', *args[:split], *owned, *args[split:]]


def prune_incremental():
    usage = shutil.disk_usage(ROOT)
    if usage.used < usage.total * 0.7:
        return
    failed = []
    for incremental in [path for target in (ROOT / 'target', TEST_TARGET) for path in (*target.glob('*/incremental'), *target.glob('*/*/incremental'))]:
        shutil.rmtree(incremental, onerror=lambda function, path, error: failed.append(path))
    after = shutil.disk_usage(ROOT)
    note = f'; {len(failed)} paths could not be removed' if failed else ''
    print(f'Pruned incremental caches: disk {usage.used * 100 // usage.total}% -> {after.used * 100 // after.total}%{note}', file=sys.stderr, flush=True)

def prune_image():
    """Leaves what a warm check reads: the dev profile's outputs without their incremental sessions; release and test
    builds and the compile caches go, so the image stays small enough to hydrate quickly on a default machine."""
    target = ROOT / 'target'
    gone = [TEST_TARGET, SCCACHE_HOME / 'cache', SCCACHE_HOME / 'cache-test']
    for folder, keep in ((target, {'debug', TARGET}), (target / TARGET, {'debug'})):
        gone += [path for path in folder.glob('*') if path.is_dir() and not path.is_symlink() and path.name not in keep]
    gone += [target / 'debug' / 'incremental', target / TARGET / 'debug' / 'incremental']
    for path in gone:
        shutil.rmtree(path, ignore_errors=True)
    left = [str(path) for path in gone if path.exists()]
    if left:
        raise RuntimeError(f'Could not remove {left}')
    checkpoint()
    usage = shutil.disk_usage(ROOT)
    print(f'Pruned the image to what a check reads: {usage.used / 1024 ** 3:.1f} GiB used', flush=True)

def clear_targets():
    """Removes every build output and compile cache, so an image refresh keeps only what the current tree builds."""
    for path in (ROOT / 'target', TEST_TARGET, SCCACHE_HOME / 'cache', SCCACHE_HOME / 'cache-test'):
        shutil.rmtree(path, ignore_errors=True)
        if path.exists():
            raise RuntimeError(f'Could not remove {path}')
    (ROOT / 'target').mkdir()

def provenance():
    commands = [['rustc', '-vV'], ['cargo', 'xwin', '--version'], ['clang-cl', '--version'], ['lld-link', '--version'], ['node', '--version'], ['cargo', 'xwin', 'env', '--target', TARGET]]
    text = ''
    for command in commands:
        result = subprocess.run(command, cwd=SRC, capture_output=True, text=True, check=True)
        text += '$ ' + ' '.join(command) + '\n' + result.stdout + result.stderr + '\n'
    text += (ROOT / 'xwin' / 'xwin' / 'DONE').read_text()
    (ROOT / 'toolchain.txt').write_text(text)
    print(text)

IMAGE_STEPS = {'clear': clear_targets, 'prune': prune_image, 'provenance': provenance}

EMBEDS_UI = {'replay-desktop', 'recorder-app'}

def embeds_ui(args):
    """Whether a build enables a Tauri package's `custom-protocol`, which embeds ui/dist."""
    if any('custom-protocol' in arg for arg in args):
        return True
    if '--all-features' not in args:
        return False
    pairs = list(zip(args, args[1:]))
    excluded = {value for flag, value in pairs if flag == '--exclude'}
    excluded |= {arg.split('=', 1)[1] for arg in args if arg.startswith('--exclude=')}
    packages = {value for flag, value in pairs if flag in ('-p', '--package')}
    packages |= {arg.split('=', 1)[1] for arg in args if arg.startswith('--package=')}
    packages |= {arg[2:] for arg in args if arg.startswith('-p') and len(arg) > 2}
    if not packages or '--workspace' in args or '--all' in args:
        return bool(EMBEDS_UI - excluded)
    return bool(packages & EMBEDS_UI)

def start_cache(token):
    """Starts the compile cache's server outside the job's process group; the environment of its clients, or None and a warning."""
    try:
        if not SCCACHE.is_file():
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
        STRANDED.write_text(f'{token}\n{boot_id()}')
        subprocess.run([str(SCCACHE), '--stop-server'], env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
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

def toolchain_fault(channel):
    """(exit code, why) when the installed toolchain of `channel`, the incoming tree's, does not run Cargo: BROKEN_TOOLCHAIN
    when it fails or cannot launch, UNPROVEN_TOOLCHAIN when the probe was cut short; (None, None) when it runs, or the VM
    lacks the toolchain, which is rustup's to install."""
    if not channel:
        return None, None
    toolchains = Path(os.environ.get('RUSTUP_HOME', '/usr/local/rustup')) / 'toolchains'
    if not any((toolchains / name).is_dir() for name in (channel, f'{channel}-{HOST}')):
        return None, None
    try:
        result = subprocess.run(['cargo', '--version'], cwd=ROOT, env={**os.environ, 'RUSTUP_TOOLCHAIN': channel, 'RUSTUP_AUTO_INSTALL': '0'}, capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        return UNPROVEN_TOOLCHAIN, 'cargo --version did not answer within 60 s'
    except OSError as error:
        return (UNPROVEN_TOOLCHAIN if error.errno in TRANSIENT_ERRNOS else BROKEN_TOOLCHAIN), repr(error)
    if result.returncode < 0:
        return UNPROVEN_TOOLCHAIN, f'cargo --version was killed by signal {-result.returncode}'
    if result.returncode:
        return BROKEN_TOOLCHAIN, (result.stderr or result.stdout).strip() or f'exit {result.returncode}'
    return None, None

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

def job_file(job, kind):
    """A job's `started` file, naming its process, and `outcome` file, its exit code and a build's artifacts: for a run that lost its connection to it."""
    return ROOT / f'job-{job}.{kind}'

def leave(job, kind, value):
    partial = job_file(job, 'part')
    partial.write_text(json.dumps(value))
    partial.replace(job_file(job, kind))

def process_start(pid):
    """When a live process started, which tells it from a later one given the same id; None once it has ended."""
    try:
        fields = (Path('/proc') / str(pid) / 'stat').read_text().rsplit(')', 1)[1].split()
    except (FileNotFoundError, ProcessLookupError):
        return None
    return None if fields[0] == 'Z' else fields[19]

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

def test_programs(events, packages, wanted, pdb):
    """From Cargo's messages and `cargo metadata` packages by id: the test programs, each with the package facts a test run sets; the binaries tests name; the files to send."""
    scripts = {event['package_id']: event.get('env', []) for event in events if event['reason'] == 'build-script-executed'}
    built = [event for event in events if event['reason'] == 'compiler-artifact' and event.get('executable')]
    programs = [event for event in built if event['profile'].get('test')]
    binaries = [event for event in built if not event['profile'].get('test') and 'bin' in event['target']['kind']]

    def package(event):
        found = packages[event['package_id']]
        facts = {key: found.get(key) for key in ('name', 'version', 'authors', 'description', 'homepage', 'repository', 'license', 'license_file', 'rust_version', 'readme')}
        return {**facts, 'manifest': Path(found['manifest_path']).relative_to(SRC).as_posix(), 'env': scripts.get(event['package_id'], [])}

    tests = [{'package': package(event), 'target': event['target']['name'], 'kind': event['target']['kind'], 'executable': Path(event['executable']).name, 'path': event['executable']} for event in programs]
    named = [{'package': packages[event['package_id']]['name'], 'name': event['target']['name'], 'executable': Path(event['executable']).name, 'path': event['executable']} for event in binaries]
    named = [binary for binary in named if f"{binary['package']}/{binary['name']}" in wanted]
    files = [item['path'] for item in tests + named]
    if pdb:
        files += [str(Path(name).with_suffix('.pdb')) for name in files if Path(name).with_suffix('.pdb').exists()]
    return files, tests, named

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
elif mode in IMAGE_STEPS:
    job = sys.argv[3] if len(sys.argv) > 3 else None
    if job:
        leave(job, 'started', {'pid': os.getpid(), 'start': process_start(os.getpid()), 'boot': boot_id()})
    outcome = {'exit': 1}
    try:
        IMAGE_STEPS[mode]()
        outcome['exit'] = 0
    finally:
        if job:
            leave(job, 'outcome', outcome)
elif mode == 'acquire':
    token = sys.argv[2]
    try:
        CLAIM.mkdir()
    except FileExistsError:
        holder = (CLAIM / 'token').read_text() if (CLAIM / 'token').exists() else None
        with (ROOT / 'running.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                print('busy')
                print('VM is still finishing a detached job', file=sys.stderr)
                sys.exit(73)
            if STRANDED.exists() and STRANDED.read_text() == f'{holder}\n{boot_id()}':
                print('VM holds processes a job could not stop; refusing to overlap', file=sys.stderr)
                sys.exit(73)
            if compiling():
                print('busy')
                print('VM is still finishing a detached job', file=sys.stderr)
                sys.exit(73)
            MANIFEST.unlink(missing_ok=True)
            shutil.rmtree(CLAIM)
            CLAIM.mkdir()
            print('Took over the claim of a run that ended without releasing it', file=sys.stderr)
    STRANDED.unlink(missing_ok=True)
    (CLAIM / 'token').write_text(token)
    for leftover in [*ROOT.glob('artifacts-*'), *ROOT.glob('incoming-*'), *ROOT.glob('runner-*.b64-*'), *ROOT.glob('job-*')]:
        if leftover.is_dir() and not leftover.is_symlink():
            shutil.rmtree(leftover)
        else:
            leftover.unlink()
    restore_metadata()
    fault, why = toolchain_fault(sys.argv[3] if len(sys.argv) > 3 else '')
    if fault:
        print(f'The toolchain does not run: {why}' if fault == BROKEN_TOOLCHAIN else f'The toolchain could not be checked: {why}', file=sys.stderr)
        with (ROOT / 'running.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                sys.exit(fault)
            if CLAIM.exists() and owned(token):
                shutil.rmtree(CLAIM)
        sys.exit(fault)
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
elif mode == 'outcome':
    job = sys.argv[2]
    try:
        started = json.loads(job_file(job, 'started').read_text())
    except FileNotFoundError:
        print('The job has not started', file=sys.stderr)
        sys.exit(78)
    if started['boot'] == boot_id() and process_start(started['pid']) == started['start']:
        sys.exit(74)
    path = job_file(job, 'outcome')
    if not path.exists():
        print('The job ended without leaving its outcome', file=sys.stderr)
        sys.exit(1)
    print(path.read_text())
elif mode in {'check', 'build', 'test', 'ui'}:
    token, job = sys.argv[2], sys.argv[3]
    leave(job, 'started', {'pid': os.getpid(), 'start': process_start(os.getpid()), 'boot': boot_id()})
    assert owned(token)
    args = sys.argv[4:]
    source = None
    if args[:1] == ['--sync']:
        source, args = args[1], args[2:]
    wanted, pdb = set(), False
    if mode == 'test':
        options, args = args[:args.index('--')], args[args.index('--') + 1:]
        pdb = '--pdb' in options
        wanted = {option.split('=', 1)[1] for option in options if option.startswith('--bin-exe=')}
        os.environ['CARGO_TARGET_DIR'] = str(TEST_TARGET)
        SCCACHE_ENV.update(SCCACHE_DIR=str(SCCACHE_HOME / 'cache-test'), SCCACHE_CACHE_SIZE='8G')
    memory_gib = os.sysconf('SC_PAGE_SIZE') * os.sysconf('SC_PHYS_PAGES') / (1024 ** 3)
    os.environ['CARGO_BUILD_JOBS'] = str(max(1, min(os.cpu_count() or 1, int(memory_gib / (2.2 if mode in {'build', 'test'} else 1.2)))))
    with (ROOT / 'running.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = None
        launched = False
        cache = None
        cache_started = False
        outcome = {'exit': 1}
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
            if mode == 'ui' or (mode == 'build' and embeds_ui(args)):
                package_hash = hashlib.sha256((SRC / 'package-lock.json').read_bytes()).hexdigest()
                stamp = ROOT / 'node-lock.sha256'
                modules = ROOT / 'npm' / 'node_modules'
                previous = ROOT / 'node_modules' / 'installed'
                modules.parent.mkdir(exist_ok=True)
                if previous.exists() and not modules.exists():
                    previous.rename(modules)
                link = SRC / 'node_modules'
                if not modules.exists() or not stamp.exists() or stamp.read_text().strip() != package_hash:
                    stamp.unlink(missing_ok=True)
                    if link.is_symlink():
                        link.unlink()
                    subprocess.run(['npm', 'ci', '--no-audit', '--no-fund'], cwd=SRC, check=True)
                    if modules.exists():
                        shutil.rmtree(modules)
                    link.rename(modules)
                    stamp.write_text(package_hash)
                if link.is_symlink() and link.resolve() != modules:
                    link.unlink()
                elif link.exists() and not link.is_symlink():
                    shutil.rmtree(link)
                if not link.is_symlink():
                    link.symlink_to(modules, target_is_directory=True)
                command = ['npm', 'run', 'check:ui']
                if mode == 'build':
                    subprocess.run(['npm', 'run', 'build:ui'], cwd=SRC, check=True)
            if mode != 'ui':
                prune_incremental()
                if args[:1] == ['clippy'] and subprocess.run(['cargo', 'clippy', '--version'], cwd=SRC, capture_output=True).returncode:
                    if subprocess.run(['rustup', 'component', 'add', 'clippy'], cwd=SRC).returncode:
                        print('Clippy could not be installed on this VM', file=sys.stderr, flush=True)
                        sys.exit(80)
                command = cargo_command(args, TARGET, mode in {'build', 'test'})
                if mode in {'build', 'test'}:
                    cache_started = True
                    cache = start_cache(token)
            print('Remote command: ' + ' '.join(command), file=sys.stderr, flush=True)
            if mode not in {'build', 'test'}:
                launched = True
                result = subprocess.Popen(command, cwd=SRC, start_new_session=True)
                result.wait()
            else:
                artifacts, events = [], []
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
                    elif mode == 'test' and event.get('reason') in {'compiler-artifact', 'build-script-executed'}:
                        events.append(event)
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
                    if mode == 'test':
                        listing = subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps', '--offline'], cwd=SRC, capture_output=True, text=True, check=True)
                        packages = {package['id']: package for package in json.loads(listing.stdout)['packages']}
                        artifacts, outcome['tests'], outcome['binaries'] = test_programs(events, packages, wanted, pdb)
                    out = []
                    for name in sorted(set(artifacts)):
                        path = Path(name)
                        out.append({'path': name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'bytes': path.stat().st_size})
                    bundle = ROOT / ('artifacts-' + token + '.tar.xz')
                    with bundle.open('wb') as compressed:
                        packer = subprocess.Popen(['xz', '-T0', '-2', '-c'], stdin=subprocess.PIPE, stdout=compressed)
                        with tarfile.open(fileobj=packer.stdin, mode='w|') as archive:
                            for artifact in out:
                                path = Path(artifact['path'])
                                archive.add(path, arcname=path.name, recursive=False)
                        packer.stdin.close()
                        if packer.wait():
                            raise RuntimeError('Artifact compression failed')
                    outcome.update(artifacts=out, bundle={'path': str(bundle), 'size': bundle.stat().st_size, 'sha256': hashlib.sha256(bundle.read_bytes()).hexdigest()})
            sys.exit(result.returncode)
        except SystemExit as stop:
            outcome['exit'] = stop.code & 0xFF if isinstance(stop.code, int) else 1
            raise
        finally:
            drained = not launched or (result is not None and drain(result))
            if cache_started:
                stopped = stop_cache()
                if stopped and not drained and result is not None:
                    drained = drain(result)
                drained = drained and stopped
            leave(job, 'outcome', outcome)
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
