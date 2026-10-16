"""Run Node and all of its children in a retained Windows Job; no PID cleanup."""
import ctypes as c
from ctypes import wintypes as w
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

k = c.WinDLL('kernel32', use_last_error=True)

class Limits(c.Structure):
    _fields_ = [('process_time', c.c_int64), ('job_time', c.c_int64), ('flags', w.DWORD),
                ('min_ws', c.c_size_t), ('max_ws', c.c_size_t), ('active_limit', w.DWORD),
                ('affinity', c.c_size_t), ('priority', w.DWORD), ('scheduling', w.DWORD)]

class IO(c.Structure):
    _fields_ = [(name, c.c_uint64) for name in ('read_ops', 'write_ops', 'other_ops', 'read_bytes', 'write_bytes', 'other_bytes')]

class Extended(c.Structure):
    _fields_ = [('basic', Limits), ('io', IO), ('process_memory', c.c_size_t),
                ('job_memory', c.c_size_t), ('peak_process', c.c_size_t), ('peak_job', c.c_size_t)]

class Accounting(c.Structure):
    _fields_ = [(name, c.c_int64) for name in ('user', 'kernel', 'period_user', 'period_kernel')] + [
        (name, w.DWORD) for name in ('faults', 'total', 'active', 'terminated')]

def checked(value):
    if not value:
        raise c.WinError(c.get_last_error())
    return value

k.CreateJobObjectW.argtypes = [c.c_void_p, w.LPCWSTR]
k.CreateJobObjectW.restype = w.HANDLE
k.SetInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD]
k.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
k.TerminateJobObject.argtypes = [w.HANDLE, w.UINT]
k.QueryInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.c_void_p]
k.ResumeThread.argtypes = [w.HANDLE]
k.CloseHandle.argtypes = [w.HANDLE]

class Job:
    def __init__(self):
        self.handle = checked(k.CreateJobObjectW(None, None))
        limits = Extended()
        # No breakaway flags: every descendant inherits membership, including
        # detached browser/encoder children. Supervisor death closes this handle.
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        try:
            checked(k.SetInformationJobObject(self.handle, 9, c.byref(limits), c.sizeof(limits)))
        except BaseException:
            k.CloseHandle(self.handle)
            raise

    def assign(self, process_handle):
        checked(k.AssignProcessToJobObject(self.handle, process_handle))

    def terminate(self, code=130):
        checked(k.TerminateJobObject(self.handle, code))

    def active(self):
        info = Accounting()
        checked(k.QueryInformationJobObject(self.handle, 1, c.byref(info), c.sizeof(info), None))
        return info.active

    def close(self):
        if self.handle:
            self.terminate()
            deadline = time.monotonic() + 10
            while self.active() and time.monotonic() < deadline:
                time.sleep(.05)
            remaining = self.active()
            k.CloseHandle(self.handle)
            self.handle = None
            if remaining:
                raise RuntimeError(f'Owned job did not reap {remaining} processes')


def main():
    args = sys.argv[1:]
    if not args:
        raise ValueError('owned_job.py requires a command')
    if os.name != 'nt':
        raise RuntimeError('This harness requires Windows and installed Edge')
    import msvcrt
    class Startup(c.Structure):
        _fields_ = [('cb', w.DWORD), ('reserved', w.LPWSTR), ('desktop', w.LPWSTR), ('title', w.LPWSTR),
          ('x', w.DWORD), ('y', w.DWORD), ('width', w.DWORD), ('height', w.DWORD), ('xchars', w.DWORD), ('ychars', w.DWORD),
          ('fill', w.DWORD), ('flags', w.DWORD), ('show', w.WORD), ('reserved_size', w.WORD), ('reserved2', c.c_void_p),
          ('stdin', w.HANDLE), ('stdout', w.HANDLE), ('stderr', w.HANDLE)]
    class Process(c.Structure):
        _fields_ = [('process', w.HANDLE), ('thread', w.HANDLE), ('pid', w.DWORD), ('tid', w.DWORD)]
    k.CreateProcessW.argtypes = [w.LPCWSTR, w.LPWSTR, c.c_void_p, c.c_void_p, w.BOOL, w.DWORD, c.c_void_p, w.LPCWSTR, c.POINTER(Startup), c.POINTER(Process)]
    k.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
    k.GetExitCodeProcess.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
    args[0] = shutil.which(args[0]) or args[0]
    token = str(uuid.uuid4())
    stop = pathlib.Path(tempfile.gettempdir()) / ('hoi4-owned-job-' + token)
    guard = pathlib.Path(__file__).with_name('process-guard.cjs')
    os.environ['BATTLE_PLAN_OWNED_JOB'] = token
    os.environ['BATTLE_PLAN_JOB_STOP_FILE'] = str(stop)
    os.environ['NODE_OPTIONS'] = (os.environ.get('NODE_OPTIONS', '') + f' --require "{guard.as_posix()}"').strip()
    startup = Startup(); startup.cb = c.sizeof(startup); startup.flags = 0x101; startup.show = 0
    streams = [sys.stdin, sys.stdout, sys.stderr]
    handles = [msvcrt.get_osfhandle(stream.fileno()) for stream in streams]
    for handle in handles:
        os.set_handle_inheritable(handle, True)
    startup.stdin, startup.stdout, startup.stderr = handles
    assigned = False
    job = Job(); info = Process(); began = time.monotonic(); code = w.DWORD(1)
    try:
        checked(k.CreateProcessW(args[0], c.create_unicode_buffer(subprocess.list2cmdline(args)), None, None, True,
            0x08000004, None, None, c.byref(startup), c.byref(info)))
        job.assign(info.process)
        assigned = True
        if k.ResumeThread(info.thread) == 0xffffffff:
            raise c.WinError(c.get_last_error())
        print(json.dumps({'ownedJob': token, 'stopFile': str(stop)}), file=sys.stderr, flush=True)
        while k.WaitForSingleObject(info.process, 100) == 258:
            if stop.exists() or time.monotonic() - began > 7200:
                job.terminate(124)
        checked(k.GetExitCodeProcess(info.process, c.byref(code)))
        if stop.exists():
            print('Owned job stopped: ' + stop.read_text(), file=sys.stderr)
            return 124
        return code.value
    finally:
        # The retained process and job handles are the only termination authority.
        reaped = False
        try:
            if info.process and not assigned:
                k.TerminateProcess.argtypes = [w.HANDLE, w.UINT]
                checked(k.TerminateProcess(info.process, 130))
            job.close()
            reaped = True
        finally:
            for handle in [info.thread, info.process]:
                if handle: k.CloseHandle(handle)
            stop.unlink(missing_ok=True)
            print(json.dumps({'ownedJob': token, 'activeAfterClose': 0 if reaped else None, 'wallSeconds': round(time.monotonic() - began, 3)}), file=sys.stderr)

if __name__ == '__main__':
    sys.exit(main())
