"""Exercise the real Windows desktop app on a separate, inactive desktop.

Python 3; Pillow is needed only for capture.
Never switches desktops or moves the system cursor.
"""
import ctypes as c
from ctypes import wintypes as w
import argparse, hashlib, json, os, pathlib, shutil, socket, subprocess, msvcrt, sys, time

u = c.WinDLL('user32', use_last_error=True)
k = c.WinDLL('kernel32', use_last_error=True)
u.SetProcessDpiAwarenessContext.argtypes=[c.c_void_p]
u.SetProcessDpiAwarenessContext(c.c_void_p(-4))
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--session', required=True, type=pathlib.Path, help='Capture/profile directory outside the repository')
parser.add_argument('--port', type=int, default=9227, help='Loopback WebView2 debugging port, used by launch')
parser.add_argument('action', choices=['launch', 'list', 'children', 'capture', 'resize', 'choose', 'cancel-dialog', 'message', 'text', 'close'])
parser.add_argument('arguments', nargs='*')
cli = parser.parse_args()
ROOT = cli.session.resolve()
repository = pathlib.Path(__file__).resolve().parents[1]
if ROOT == repository or repository in ROOT.parents:
    parser.error('Keep test profiles, binaries and captures outside the repository')
if not 1024 <= cli.port <= 65535:
    parser.error('Choose a port between 1024 and 65535')
NAME = 'HOI4ReplayTest-' + hashlib.sha256(str(ROOT).casefold().encode()).hexdigest()[:16]

class Startup(c.Structure):
    _fields_ = [('cb',w.DWORD),('reserved',w.LPWSTR),('desktop',w.LPWSTR),('title',w.LPWSTR),
      ('x',w.DWORD),('y',w.DWORD),('width',w.DWORD),('height',w.DWORD),('xchars',w.DWORD),('ychars',w.DWORD),
      ('fill',w.DWORD),('flags',w.DWORD),('show',w.WORD),('reserved2size',w.WORD),('reserved2',c.c_void_p),
      ('stdin',w.HANDLE),('stdout',w.HANDLE),('stderr',w.HANDLE)]
class Process(c.Structure):
    _fields_ = [('process',w.HANDLE),('thread',w.HANDLE),('pid',w.DWORD),('tid',w.DWORD)]
u.CreateDesktopW.argtypes=[w.LPCWSTR,w.LPCWSTR,c.c_void_p,w.DWORD,w.DWORD,c.c_void_p]
u.CreateDesktopW.restype=w.HANDLE
u.OpenDesktopW.argtypes=[w.LPCWSTR,w.DWORD,w.BOOL,w.DWORD]
u.OpenDesktopW.restype=w.HANDLE
u.SetThreadDesktop.argtypes=[w.HANDLE]
u.SetThreadDesktop.restype=w.BOOL
u.CloseDesktop.argtypes=[w.HANDLE]
k.CreateProcessW.argtypes=[w.LPCWSTR,w.LPWSTR,c.c_void_p,c.c_void_p,w.BOOL,w.DWORD,c.c_void_p,w.LPCWSTR,c.POINTER(Startup),c.POINTER(Process)]
k.CreateProcessW.restype=w.BOOL
k.CloseHandle.argtypes=[w.HANDLE]
k.WaitForSingleObject.argtypes=[w.HANDLE,w.DWORD]
k.GetExitCodeProcess.argtypes=[w.HANDLE,c.POINTER(w.DWORD)]

def checked(value):
    if not value: raise c.WinError(c.get_last_error())
    return value

def desktop(create=False):
    desk = checked(u.CreateDesktopW(NAME,None,None,0,0x1ff,None) if create else u.OpenDesktopW(NAME,0,False,0x1ff))
    checked(u.SetThreadDesktop(desk))
    return desk

def launch(args):
    if not args:
        parser.error('launch requires the built replay-desktop.exe path')
    with socket.socket() as probe:
        if probe.connect_ex(('127.0.0.1', cli.port)) == 0:
            raise RuntimeError('Debugging port is already in use; choose another --port')
    existing = u.OpenDesktopW(NAME, 0, False, 0x1ff)
    if existing:
        u.CloseDesktop(existing)
        raise RuntimeError('This session desktop is already open; close its app first')
    source = pathlib.Path(args[0]).resolve(strict=True)
    ROOT.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, ROOT / source.name)
    # The app's map loads the pinned shader compiler from beside the app.
    for filename in ['dxcompiler.dll', 'dxil.dll']:
        if (source.parent / filename).is_file():
            shutil.copy2(source.parent / filename, ROOT / filename)
    args = [str(ROOT / source.name), *args[1:]]
    desk = desktop(True)
    for key,subdir in [('APPDATA','roaming'),('LOCALAPPDATA','local'),('WEBVIEW2_USER_DATA_FOLDER','webview')]:
        folder=ROOT/subdir; folder.mkdir(exist_ok=True); os.environ[key]=str(folder)
    os.environ['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS']=f'--remote-debugging-port={cli.port} --disable-backgrounding-occluded-windows --disable-renderer-backgrounding --disable-background-timer-throttling'
    log=open(ROOT/'desktop.log','wb');null=open(os.devnull,'rb')
    for stream in (log,null):os.set_handle_inheritable(msvcrt.get_osfhandle(stream.fileno()),True)
    startup=Startup();startup.cb=c.sizeof(startup);startup.desktop='winsta0\\'+NAME;startup.flags=0x180
    startup.stdout=startup.stderr=msvcrt.get_osfhandle(log.fileno());startup.stdin=msvcrt.get_osfhandle(null.fileno())
    info=Process()
    command=c.create_unicode_buffer(subprocess.list2cmdline(args))
    checked(k.CreateProcessW(args[0],command,None,None,True,0x08000000,None,str(ROOT),c.byref(startup),c.byref(info)))
    data={'pid':info.pid,'desktop':NAME,'exe':args[0],'port':cli.port}
    (ROOT/'session.json').write_text(json.dumps(data))
    print(json.dumps(data),flush=True)
    k.CloseHandle(info.thread)
    try:
        while k.WaitForSingleObject(info.process, 1000) == 258:
            pass
    except KeyboardInterrupt:
        u.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
        for item in windows():
            if item['pid'] == info.pid:
                u.PostMessageW(item['hwnd'], 0x10, 0, 0)
        if k.WaitForSingleObject(info.process, 15000) == 258:
            k.TerminateProcess.argtypes = [w.HANDLE, w.UINT]
            checked(k.TerminateProcess(info.process, 130))
            k.WaitForSingleObject(info.process, 5000)
    code=w.DWORD();k.GetExitCodeProcess(info.process,c.byref(code));print(json.dumps({'exitCode':code.value}),flush=True)
    k.CloseHandle(info.process);u.CloseDesktop(desk)
    return code.value

def windows(parent=None):
    items=[]
    callback=c.WINFUNCTYPE(w.BOOL,w.HWND,w.LPARAM)
    u.GetWindowTextW.argtypes=[w.HWND,w.LPWSTR,c.c_int]
    u.GetClassNameW.argtypes=[w.HWND,w.LPWSTR,c.c_int]
    u.GetWindowThreadProcessId.argtypes=[w.HWND,c.POINTER(w.DWORD)]
    u.IsWindowVisible.argtypes=[w.HWND]
    u.GetWindowRect.argtypes=[w.HWND,c.POINTER(w.RECT)]
    u.GetDlgCtrlID.argtypes=[w.HWND]
    def collect(hwnd,_):
        name=c.create_unicode_buffer(1024);cls=c.create_unicode_buffer(256);pid=w.DWORD();r=w.RECT()
        u.GetWindowTextW(hwnd,name,1024);u.GetClassNameW(hwnd,cls,256);u.GetWindowThreadProcessId(hwnd,c.byref(pid));u.GetWindowRect(hwnd,c.byref(r))
        if name.value or u.IsWindowVisible(hwnd):items.append({'hwnd':hwnd,'pid':pid.value,'title':name.value,'class':cls.value,'id':u.GetDlgCtrlID(hwnd),'visible':bool(u.IsWindowVisible(hwnd)),'rect':[r.left,r.top,r.right,r.bottom]})
        return True
    if parent:
        u.EnumChildWindows.argtypes=[w.HWND,callback,w.LPARAM]
        u.EnumChildWindows(parent,callback(collect),0)
    else:u.EnumWindows(callback(collect),0)
    return items

def capture(hwnd, filename):
    from PIL import Image
    g=c.WinDLL('gdi32',use_last_error=True)
    u.GetDC.argtypes=[w.HWND];u.GetDC.restype=w.HDC
    u.ReleaseDC.argtypes=[w.HWND,w.HDC]
    u.GetWindowRect.argtypes=[w.HWND,c.POINTER(w.RECT)]
    u.PrintWindow.argtypes=[w.HWND,w.HDC,w.UINT]
    g.CreateCompatibleDC.argtypes=[w.HDC];g.CreateCompatibleDC.restype=w.HDC
    g.CreateCompatibleBitmap.argtypes=[w.HDC,c.c_int,c.c_int];g.CreateCompatibleBitmap.restype=w.HBITMAP
    g.SelectObject.argtypes=[w.HDC,w.HANDLE];g.SelectObject.restype=w.HANDLE
    g.GetDIBits.argtypes=[w.HDC,w.HBITMAP,w.UINT,w.UINT,c.c_void_p,c.c_void_p,w.UINT]
    g.DeleteObject.argtypes=[w.HANDLE];g.DeleteDC.argtypes=[w.HDC]
    r=w.RECT();checked(u.GetWindowRect(hwnd,c.byref(r)));width=r.right-r.left;height=r.bottom-r.top
    dc=u.GetDC(hwnd);mem=g.CreateCompatibleDC(dc);bitmap=g.CreateCompatibleBitmap(dc,width,height);old=g.SelectObject(mem,bitmap)
    try:
        checked(u.PrintWindow(hwnd,mem,2))
        class Header(c.Structure):
            _fields_=[('size',w.DWORD),('width',w.LONG),('height',w.LONG),('planes',w.WORD),('bits',w.WORD),('compression',w.DWORD),('sizeImage',w.DWORD),('x',w.LONG),('y',w.LONG),('colors',w.DWORD),('important',w.DWORD)]
        header=Header(c.sizeof(Header),width,-height,1,32,0,0,0,0,0,0);pixels=c.create_string_buffer(width*height*4)
        checked(g.GetDIBits(mem,bitmap,0,height,pixels,c.byref(header),0))
        Image.frombuffer('RGB',(width,height),pixels.raw,'raw','BGRX',0,1).save(filename)
        print(json.dumps({'path':filename,'width':width,'height':height}))
    finally:
        g.SelectObject(mem,old);g.DeleteObject(bitmap);g.DeleteDC(mem);u.ReleaseDC(hwnd,dc)

if __name__=='__main__':
    action=cli.action
    arguments=cli.arguments
    if action=='launch':
        code=launch(arguments)
        # Signed, so a crash status such as 0xC0000005 is the script's own exit code.
        sys.exit(code-(1<<32) if code>=1<<31 else code)
    else:
        desk=desktop()
        def handle(index=0):
            hwnd=int(arguments[index])
            top=windows()
            owned=top + [child for item in top for child in windows(item['hwnd'])]
            if not any(item['hwnd'] == hwnd for item in owned):
                raise RuntimeError('Window does not belong to this isolated desktop')
            return hwnd
        if action=='list':print(json.dumps(windows()))
        elif action=='children':print(json.dumps(windows(handle())))
        elif action=='capture':capture(handle(),arguments[1])
        elif action=='resize':
            u.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT]
            checked(u.SetWindowPos(handle(),None,0,0,int(arguments[1]),int(arguments[2]),0x16))
        elif action in ('choose','cancel-dialog'):
            for attempt in range(50):
                dialogs=[item for item in windows() if item['class']=='#32770' and item['visible']]
                if dialogs:break
                time.sleep(.1)
            if len(dialogs)!=1:raise RuntimeError(f'Expected one dialog: {dialogs}')
            dialog=dialogs[0];items=windows(dialog['hwnd'])
            if action=='choose':
                # Folder picker (1152) and recording file picker (1148).
                field=next(item for item in items if item['class']=='Edit' and item['id'] in (1152,1148))
                u.SendMessageW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPCWSTR];u.SendMessageW.restype=w.LPARAM
                checked(u.SendMessageW(field['hwnd'],0xc,0,str(pathlib.Path(arguments[0]))))
            button=next(item for item in items if item['class']=='Button' and item['id']==(1 if action=='choose' else 2))
            u.SendMessageW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPARAM]
            u.SendMessageW(button['hwnd'],0xf5,0,0)
            print(json.dumps({'dialog':dialog['title'],'action':action}))
        elif action=='message':
            u.SendMessageW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPARAM];u.SendMessageW.restype=w.LPARAM
            print(u.SendMessageW(handle(),int(arguments[1]),int(arguments[2]),int(arguments[3])))
        elif action=='text':
            u.SendMessageW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPCWSTR];u.SendMessageW.restype=w.LPARAM
            print(u.SendMessageW(handle(),0xc,0,arguments[1]))
        elif action=='close':
            u.PostMessageW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPARAM]
            checked(u.PostMessageW(handle(),0x10,0,0))
        u.CloseDesktop(desk)
