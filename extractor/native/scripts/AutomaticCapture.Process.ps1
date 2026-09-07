# No game write handles or game controls. The only message targets an owned
# recorder process and its specific product window class, for graceful stop.
function Initialize-ScenarioProcessAccess {
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Collections.Generic;
public static class AutomaticScenarioProcess {
    public sealed class Identity {
        public int pid; public string creation_filetime; public int session_id;
        public string user_sid; public string exe_path;
    }
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetProcessTimes(IntPtr h, out long c, out long e, out long k, out long u);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool QueryFullProcessImageName(IntPtr h, int f, StringBuilder p, ref int n);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool ProcessIdToSessionId(int pid, out int session);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenProcessToken(IntPtr h, uint a, out IntPtr token);
    delegate bool EnumWindow(IntPtr w, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindow callback, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr w, out int pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr w, StringBuilder name, int count);
    [DllImport("user32.dll", SetLastError=true)] static extern bool PostMessage(IntPtr w, uint message, IntPtr wp, IntPtr lp);
    static Exception Error(string operation) { return new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), operation); }
    public static Identity Read(int pid) {
        IntPtr h=OpenProcess(0x1000, false, pid); // QUERY_LIMITED_INFORMATION only
        if(h==IntPtr.Zero) throw Error("query process identity");
        IntPtr token=IntPtr.Zero;
        try {
            long c,e,k,u; int session; int length=32768; var path=new StringBuilder(length);
            if(!GetProcessTimes(h,out c,out e,out k,out u) || !ProcessIdToSessionId(pid,out session) || !QueryFullProcessImageName(h,0,path,ref length)) throw Error("read process identity");
            if(!OpenProcessToken(h,8,out token)) throw Error("read process user");
            using(var user=new WindowsIdentity(token)) return new Identity { pid=pid, creation_filetime=c.ToString(System.Globalization.CultureInfo.InvariantCulture), session_id=session, user_sid=user.User.Value, exe_path=path.ToString() };
        } finally { if(token!=IntPtr.Zero) CloseHandle(token); CloseHandle(h); }
    }
    public static bool RequestRecorderStop(int pid, string creation, string path) {
        var actual=Read(pid);
        if(actual.creation_filetime!=creation || !String.Equals(actual.exe_path,path,StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("recorder ownership changed; no close request sent");
        var windows=new List<IntPtr>();
        EnumWindows(delegate(IntPtr window,IntPtr unused) {
            int owner; GetWindowThreadProcessId(window,out owner);
            var name=new StringBuilder(256); GetClassName(window,name,name.Capacity);
            if(owner==pid && name.ToString()=="Hoi4ReplayRecorderWindow") windows.Add(window);
            return true;
        },IntPtr.Zero);
        if(windows.Count!=1) return false;
        actual=Read(pid);
        int currentOwner; GetWindowThreadProcessId(windows[0],out currentOwner);
        if(actual.creation_filetime!=creation || currentOwner!=pid) throw new InvalidOperationException("recorder/window ownership changed");
        // WM_CLOSE transfers stop ownership to the recorder coordinator.
        if(!PostMessage(windows[0],0x0010,IntPtr.Zero,IntPtr.Zero)) throw Error("request owned recorder graceful stop");
        return true;
    }
}
'@
}

function Get-ScenarioGames {
    param([string]$ExpectedExe, $Owner)
    foreach ($process in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
        $identity = [AutomaticScenarioProcess]::Read($process.Id)
        if ($identity.exe_path -ine $ExpectedExe -or $identity.session_id -ne $Owner.session_id -or $identity.user_sid -cne $Owner.user_sid) {
            throw 'the game census includes another executable, user or Windows session'
        }
        $identity
    }
}
