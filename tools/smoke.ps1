<#
.SYNOPSIS
Milestone 1.3 game input, screenshots, and passive free-run clock measurement.
.DESCRIPTION
The full product smoke check is not implemented yet. Run explicit actions to
drive the game during local validation. Baseline verifies the pinned game build,
then reads only its clock; it sends no input and does not inject.
The script does not modify settings or certify that a save finished loading.

Screenshot, Click and Move coordinates are relative to the outer game window.
Move positions the cursor without clicking or pressing a key. Wheel positions the
cursor and turns the mouse wheel by Delta notches (negative zooms the map out). After setting
speed and unpausing, move well inside the map before each timed run to avoid
edge scrolling during either the baseline or the capture.
Interactive actions briefly borrow foreground focus through a thread-attached
handoff and restore the previous foreground window afterward. They never stop
or minimize that application, including when it is TeamViewer.
Console opens the console with the supplied key, types one line, presses Enter,
and closes it. Use Text for an already-open console or a save-name field.
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Launch -ContinueLastSave
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Screenshot -Out captures\probe\menu.png
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Key -Key Space
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Click -X 640 -Y 480
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Move -GameId 1234 -X 1280 -Y 720
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Wheel -X 1280 -Y 720 -Delta -3
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Baseline -Seconds 120 -Out captures\baseline
#>
[CmdletBinding()]
param(
    [ValidateSet('Smoke', 'Launch', 'Screenshot', 'Key', 'Click', 'Move', 'Wheel', 'Text', 'Console', 'Baseline')]
    [string]$Action = 'Smoke',
    [string]$GameExe = 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe',
    [string]$UserDirectory = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV',
    [ValidateRange(0, 2147483647)][int]$GameId = 0,
    [switch]$ContinueLastSave,
    [string]$ExpectedSave,
    [string]$Out,
    [ValidateRange(1, 2147483647)][int]$Seconds = 120,
    [ValidateSet('Space', 'Escape', 'Enter', 'Grave', 'Tab', 'Backspace', 'Delete',
        'Left', 'Right', 'Up', 'Down', 'Home', 'End', '1', '2', '3', '4', '5',
        'F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12')]
    [string]$Key,
    [ValidateRange(-1, 32767)][int]$X = -1,
    [ValidateRange(-1, 32767)][int]$Y = -1,
    [ValidateRange(-50, 50)][int]$Delta = 0,
    [ValidateLength(1, 512)][string]$Text,
    [ValidateRange(1, 254)][int]$ConsoleKey = 0xC0
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($Action -eq 'Smoke') {
    throw 'The full product smoke check awaits Milestone 2.4. Milestone 1.3 supports explicit Launch, Screenshot, Key, Click, Move, Text, Console, and Baseline actions only.'
}
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'The game driver requires Windows and an interactive desktop.'
}
$gamePath = (Resolve-Path -LiteralPath $GameExe).ProviderPath
if ([IO.Path]::GetFileName($gamePath) -ine 'hoi4.exe') {
    throw 'GameExe must identify hoi4.exe.'
}
$gameProcesses = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
if ($gameProcesses.Count -gt 1) {
    throw 'Two game processes are unsupported. Close the extra game before driving the test.'
}

if ($Action -eq 'Launch') {
    if ($gameProcesses.Count -ne 0) { throw 'the game is already running.' }
    $userPath = (Resolve-Path -LiteralPath $UserDirectory).ProviderPath
    if ($userPath.Contains('"')) { throw 'The user directory contains an invalid quote.' }
    # This build joins argv then splits on every '-' (FUN_140124b80 -> FUN_1424b8120).
    # A split path can fail PHYSFS setup and silently fall back to the owner's Documents directory.
    if ($userPath.Contains('-')) { throw 'This game build splits command-line values at hyphens. Use a user directory whose entire path contains no hyphen.' }
    if ($ExpectedSave -and -not $ContinueLastSave) {
        throw 'ExpectedSave requires ContinueLastSave.'
    }
    $arguments = @('--gdpr-compliant', ('"-userdir=' + $userPath + '"'))
    if ($ContinueLastSave) {
        $selection = Get-Content -LiteralPath (Join-Path $userPath 'continue_game.json') -Raw | ConvertFrom-Json
        $saveName = [string]$selection.filename
        if ([string]::IsNullOrWhiteSpace($saveName) -or $saveName.IndexOfAny([char[]]'\/:') -ge 0) {
            throw 'continue_game.json must identify a save filename without directory components.'
        }
        if ($ExpectedSave -and $ExpectedSave -cne $saveName) {
            throw "The launcher selected '$saveName', not the requested '$ExpectedSave'. Select the save in the game first."
        }
        if ([bool]$selection.is_remote) {
            throw 'This local probe driver requires a local save; select it in the game first.'
        }
        if (-not (Test-Path -LiteralPath (Join-Path (Join-Path $userPath 'save games') $saveName) -PathType Leaf)) {
            throw "The launcher-selected save is missing: $saveName"
        }
        $arguments += '--continuelastsave'
        Write-Host "Launching the existing continue selection: $saveName"
    }
    # The game is the interactive application being tested, so its window is visible.
    $started = Start-Process -FilePath $gamePath -WorkingDirectory (Split-Path -Parent $gamePath) -ArgumentList $arguments -PassThru
    [pscustomobject]@{ Action = 'Launch'; GameId = $started.Id; GameExe = $gamePath; RequestedUserDirectory = $userPath }
    return
}

if ($gameProcesses.Count -ne 1) { throw 'Exactly one running hoi4.exe is required.' }
$game = $gameProcesses[0]
if ($GameId -ne 0 -and $game.Id -ne $GameId) { throw "The running game has PID $($game.Id), not $GameId." }
if ($game.Path -ine $gamePath) { throw "The running game executable differs from GameExe: $($game.Path)" }
if ($Action -eq 'Baseline') {
    if (-not [Environment]::Is64BitProcess) { throw 'Baseline requires 64-bit PowerShell.' }
    $captureRoot = 'captures'
    if (-not $Out) { throw 'Baseline requires -Out, an existing capture directory or a new .jsonl file in one.' }
    $capturePath = [IO.Path]::GetFullPath($Out)
    if (Test-Path -LiteralPath $capturePath -PathType Container) {
        $capturePath = Join-Path $capturePath ('baseline-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '-' + $game.Id + '.jsonl')
    }
    if (-not $capturePath.StartsWith($captureRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetExtension($capturePath) -ine '.jsonl') {
        throw "Baseline output must be a JSONL file under $captureRoot."
    }
    if (-not (Test-Path -LiteralPath ([IO.Path]::GetDirectoryName($capturePath)) -PathType Container)) {
        throw 'Create the baseline capture directory before starting the measurement.'
    }
    if (Test-Path -LiteralPath $capturePath) { throw "Baseline already exists: $capturePath" }
    # engine::PINNED_BUILD, verified against the installed executable on 2026-09-09.
    $wantedVersion = 'Operation Postern v1.19.2.0.a729'
    $wantedSize = [int64]55680632
    $wantedHash = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
    $actualSize = (Get-Item -LiteralPath $gamePath).Length
    $actualHash = (Get-FileHash -LiteralPath $gamePath -Algorithm SHA256).Hash
    if ($actualSize -ne $wantedSize -or $actualHash -ine $wantedHash) {
        throw "Unsupported build: wanted $wantedVersion / $wantedSize bytes / $wantedHash; got $actualSize bytes / $actualHash."
    }
    if (@($game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) {
        throw 'The free-run baseline requires collector.dll to be detached first.'
    }
    if (-not ('Hoi4ClockBaseline' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public sealed class Hoi4ClockBaseline : IDisposable {
    public sealed class Clock {
        public ulong game_state, play_time;
        public long play_session_started_at;
        public int hour, seed;
        public bool Same(Clock other) {
            return other != null && game_state == other.game_state && hour == other.hour &&
                play_time == other.play_time && play_session_started_at == other.play_session_started_at && seed == other.seed;
        }
    }
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint process);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] bytes, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern bool QueryPerformanceCounter(out long value);
    [DllImport("kernel32.dll")] static extern bool QueryPerformanceFrequency(out long value);
    readonly ulong image;
    readonly byte[] bytes = new byte[8];
    IntPtr process;
    public readonly long Frequency;
    public Hoi4ClockBaseline(uint processId, ulong imageBase) {
        if (!QueryPerformanceFrequency(out Frequency) || Frequency <= 0)
            throw new InvalidOperationException("QueryPerformanceFrequency failed.");
        image = imageBase;
        process = OpenProcess(0x10, false, processId); // PROCESS_VM_READ only; this helper never writes or injects.
        if (process == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public static long Qpc() {
        long value;
        if (!QueryPerformanceCounter(out value)) throw new InvalidOperationException("QueryPerformanceCounter failed.");
        return value;
    }
    bool Read(ulong address, uint size) {
        UIntPtr read;
        return address > 0 && address <= long.MaxValue &&
            ReadProcessMemory(process, new IntPtr((long)address), bytes, new UIntPtr(size), out read) && read.ToUInt64() == size;
    }
    Clock Once() {
        // engine::cgame_state: confirmed CGameState singleton, native-width clock fields and load witness.
        if (!Read(image + 0x3316e90, 8)) return null;
        ulong state = BitConverter.ToUInt64(bytes, 0);
        if (state == 0 || state > long.MaxValue - 0x800) return null;
        var clock = new Clock();
        clock.game_state = state;
        if (!Read(state + 0x468, 4)) return null;
        clock.hour = BitConverter.ToInt32(bytes, 0);
        if (!Read(state + 0x490, 8)) return null;
        clock.play_time = BitConverter.ToUInt64(bytes, 0);
        if (!Read(state + 0x720, 4)) return null;
        clock.seed = BitConverter.ToInt32(bytes, 0);
        if (!Read(image + 0x3317048, 8)) return null;
        clock.play_session_started_at = BitConverter.ToInt64(bytes, 0);
        if (!Read(image + 0x3316e90, 8) || BitConverter.ToUInt64(bytes, 0) != state) return null;
        return clock;
    }
    public Clock Sample() {
        Clock first = Once(), second = Once();
        return first != null && first.Same(second) ? second : null;
    }
    public void Dispose() {
        if (process != IntPtr.Zero) { CloseHandle(process); process = IntPtr.Zero; }
    }
}
'@
    }
    $reader = $null
    $stream = $null
    $writer = $null
    $summary = $null
    $stopReason = 'duration completed'
    try {
        $reader = [Hoi4ClockBaseline]::new([uint32]$game.Id, [uint64]$game.MainModule.BaseAddress.ToInt64())
        $processStart = $game.StartTime.ToUniversalTime().ToFileTimeUtc()
        $stream = [IO.File]::Open($capturePath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
        $writer.AutoFlush = $true
        $qpcStart = [Hoi4ClockBaseline]::Qpc()
        $lastFlush = $qpcStart
        $lastProcessCheck = $qpcStart
        $writer.WriteLine((@{ kind = 'baseline_start'; pid = $game.Id; process_start_time = $processStart;
            qpc = $qpcStart; qpc_frequency = $reader.Frequency; requested_seconds = $Seconds; sample_interval_ms = 125;
            exe_version = $wantedVersion; exe_size = $actualSize; exe_sha256 = $actualHash.ToLowerInvariant();
            limitation = 'External clock observations measure free-run throughput only. Pointer, load timestamp, and seed are identity witnesses; campaign identity and table consistency are not verified. Forward jumps between samples may be normal progress or an undetected reload.' } | ConvertTo-Json -Compress))
        Write-Host "Free-run baseline started for ${Seconds}s; no input is sent. Output: $capturePath"
        $previous = $null
        $previousQpc = [int64]0
        $lastWitness = $null
        $hours = [int64]0
        $pairTicks = [int64]0
        $pairs = [int64]0
        $valid = [int64]0
        $missing = [int64]0
        $loadChanges = [int64]0
        $witnessChanges = [int64]0
        $discontinuities = [int64]0
        $invalidQpc = [int64]0
        while (([Hoi4ClockBaseline]::Qpc() - $qpcStart) / [double]$reader.Frequency -lt $Seconds) {
            $game.Refresh()
            if ($game.HasExited) { $stopReason = 'game exited'; break }
            $clock = $reader.Sample()
            $qpc = [Hoi4ClockBaseline]::Qpc()
            $elapsed = ($qpc - $qpcStart) / [double]$reader.Frequency
            $reason = $null
            if ($null -eq $clock) {
                $missing++
                $previous = $null
                $reason = 'clock unavailable or changed during external read'
            } else {
                $valid++
                if ($null -ne $lastWitness) {
                    if ($clock.play_session_started_at -ne $lastWitness.play_session_started_at) { $loadChanges++ }
                    if ($clock.game_state -ne $lastWitness.game_state -or $clock.seed -ne $lastWitness.seed) { $witnessChanges++ }
                    if ($clock.hour -lt $lastWitness.hour) { $discontinuities++ }
                }
                $lastWitness = $clock
                if ($null -ne $previous) {
                    $sameWitness = $clock.game_state -eq $previous.game_state -and
                        $clock.play_session_started_at -eq $previous.play_session_started_at -and $clock.seed -eq $previous.seed
                    $delta = [int64]$clock.hour - [int64]$previous.hour
                    if ($qpc -le $previousQpc) { $invalidQpc++ }
                    if ($sameWitness -and $delta -ge 0 -and $qpc -gt $previousQpc) {
                        # Zero-hour pairs retain their elapsed time, so pauses reduce the measured rate.
                        $hours += $delta
                        $pairTicks += $qpc - $previousQpc
                        $pairs++
                    }
                }
                $previous = $clock
                $previousQpc = $qpc
            }
            $writer.WriteLine((@{ kind = 'baseline_sample'; pid = $game.Id; qpc = $qpc; elapsed_s = $elapsed;
                clock = $clock; unavailable_reason = $reason } | ConvertTo-Json -Compress -Depth 4))
            if (($qpc - $lastFlush) / [double]$reader.Frequency -ge 5) { $stream.Flush($true); $lastFlush = $qpc }
            if (($qpc - $lastProcessCheck) / [double]$reader.Frequency -ge 2) {
                if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -gt 1) {
                    $stopReason = 'second hoi4.exe process detected'
                    break
                }
                if (@($game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) {
                    $stopReason = 'collector.dll appeared during the free-run baseline'
                    break
                }
                $lastProcessCheck = $qpc
            }
            Start-Sleep -Milliseconds 125
        }
        $pairSeconds = $pairTicks / [double]$reader.Frequency
        $rate = if ($pairSeconds -gt 0) { $hours / $pairSeconds } else { $null }
        $summary = [pscustomobject]@{ kind = 'baseline_summary'; pid = $game.Id; path = $capturePath;
            hours = $hours; elapsed_s = $pairSeconds; hours_per_s = $rate; valid_samples = $valid; missing_samples = $missing;
            continuous_pairs = $pairs; load_changes = $loadChanges; witness_changes = $witnessChanges;
            backwards_discontinuities = $discontinuities; invalid_qpc_pairs = $invalidQpc;
            observation_elapsed_s = ([Hoi4ClockBaseline]::Qpc() - $qpcStart) / [double]$reader.Frequency; stop_reason = $stopReason }
        $writer.WriteLine(($summary | ConvertTo-Json -Compress))
    } finally {
        try { if ($null -ne $writer) { $writer.Flush() }; if ($null -ne $stream) { $stream.Flush($true) } }
        finally {
            if ($null -ne $writer) { $writer.Dispose() } elseif ($null -ne $stream) { $stream.Dispose() }
            if ($null -ne $reader) { $reader.Dispose() }
        }
    }
    $summary
    if ($stopReason -eq 'second hoi4.exe process detected' -or $stopReason -eq 'collector.dll appeared during the free-run baseline') {
        throw "Baseline stopped: $stopReason. Inspect the retained observations; this run is not an uninterrupted free-run baseline."
    }
    return
}
$game.Refresh()
$window = $game.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'The game has no main window yet. Wait for it to open.' }

if (-not ('Hoi4ProbeInput' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class Hoi4ProbeInput {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Keyboard { public ushort Key, Scan; public uint Flags, Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Sequential)] struct Mouse { public int X, Y; public uint Data, Flags, Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Explicit)] struct InputData {
        [FieldOffset(0)] public Keyboard Keyboard;
        [FieldOffset(0)] public Mouse Mouse;
    }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint Type; public InputData Data; }
    [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint count, Input[] input, int size);
    [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint code, uint mapType);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint from, uint to, bool attach);
    [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr SetActiveWindow(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr SetFocus(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    static bool AttachIfDifferent(uint current, uint other) {
        return other != 0 && other != current && AttachThreadInput(current, other, true);
    }
    static void DetachIfAttached(uint current, uint other, bool attached) {
        if (attached) AttachThreadInput(current, other, false);
    }
    static bool FocusWindow(IntPtr window) {
        IntPtr foreground = GetForegroundWindow();
        uint ignored;
        uint foregroundThread = foreground == IntPtr.Zero ? 0 : GetWindowThreadProcessId(foreground, out ignored);
        uint targetThread = GetWindowThreadProcessId(window, out ignored);
        uint currentThread = GetCurrentThreadId();
        bool foregroundAttached = AttachIfDifferent(currentThread, foregroundThread);
        bool targetAttached = targetThread != foregroundThread && AttachIfDifferent(currentThread, targetThread);
        try {
            BringWindowToTop(window);
            SetForegroundWindow(window);
            SetActiveWindow(window);
            SetFocus(window);
            return GetForegroundWindow() == window;
        } finally {
            DetachIfAttached(currentThread, targetThread, targetAttached);
            DetachIfAttached(currentThread, foregroundThread, foregroundAttached);
        }
    }
    public static IntPtr AcquireTargetFocus(IntPtr window, uint process) {
        uint actual;
        if (!IsWindow(window) || GetWindowThreadProcessId(window, out actual) == 0 || actual != process)
            throw new InvalidOperationException("The game window no longer belongs to the selected process.");
        IntPtr previous = GetForegroundWindow();
        if (!FocusWindow(window))
            throw new InvalidOperationException("Windows refused to focus the game; input was not sent.");
        return previous;
    }
    public static void RestoreForeground(IntPtr previous, IntPtr gameWindow) {
        if (previous == IntPtr.Zero || previous == gameWindow || !IsWindow(previous)) return;
        // Restoration is best effort: the prior application may close or
        // deliberately retake focus while the guarded game action runs.
        FocusWindow(previous);
    }
    public static void CheckTarget(IntPtr window, uint process) {
        uint actual;
        if (!IsWindow(window) || GetWindowThreadProcessId(window, out actual) == 0 || actual != process)
            throw new InvalidOperationException("The game window no longer belongs to the selected process.");
        if (GetForegroundWindow() != window)
            throw new InvalidOperationException("Game focus was lost; input was not sent.");
    }
    static void Send(IntPtr window, uint process, Input[] input) {
        CheckTarget(window, process);
        if (SendInput((uint)input.Length, input, Marshal.SizeOf(typeof(Input))) != input.Length)
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows did not accept all game input.");
    }
    public static void KeyPress(IntPtr window, uint process, ushort key) {
        uint scan = MapVirtualKeyW(key, 4);
        if (scan == 0) throw new InvalidOperationException("Cannot map the requested key to a scan code.");
        var down = new Input[1];
        down[0].Type = 1;
        down[0].Data.Keyboard.Scan = (ushort)(scan & 0xFF);
        down[0].Data.Keyboard.Flags = 8u | ((scan & 0xFF00) != 0 ? 1u : 0u);
        var up = (Input[])down.Clone();
        up[0].Data.Keyboard.Flags |= 2;
        Send(window, process, down);
        try { System.Threading.Thread.Sleep(80); }
        finally {
            // Release even if focus changes during the hold; never leave a key down.
            if (SendInput(1, up, Marshal.SizeOf(typeof(Input))) != 1)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows did not release the game key.");
        }
    }
    public static void TypeText(IntPtr window, uint process, string text) {
        foreach (char character in text) {
            var input = new Input[2];
            input[0].Type = input[1].Type = 1;
            input[0].Data.Keyboard.Scan = input[1].Data.Keyboard.Scan = character;
            input[0].Data.Keyboard.Flags = 4;
            input[1].Data.Keyboard.Flags = 6;
            Send(window, process, input);
        }
    }
    public static void Move(IntPtr window, uint process, int x, int y) {
        CheckTarget(window, process);
        if (!SetCursorPos(x, y)) throw new Win32Exception(Marshal.GetLastWin32Error());
        System.Threading.Thread.Sleep(100);
        CheckTarget(window, process);
    }
    public static void Wheel(IntPtr window, uint process, int x, int y, int notches) {
        CheckTarget(window, process);
        if (!SetCursorPos(x, y)) throw new Win32Exception(Marshal.GetLastWin32Error());
        System.Threading.Thread.Sleep(100);
        int step = notches < 0 ? -1 : 1;
        for (int sent = 0; sent != notches; sent += step) {
            var turn = new Input[1];
            turn[0].Data.Mouse.Flags = 0x0800;
            turn[0].Data.Mouse.Data = unchecked((uint)(step * 120));
            Send(window, process, turn);
            System.Threading.Thread.Sleep(60);
        }
    }
    public static void Click(IntPtr window, uint process, int x, int y) {
        CheckTarget(window, process);
        if (!SetCursorPos(x, y)) throw new Win32Exception(Marshal.GetLastWin32Error());
        // Let the game's next input update observe the new hover target before pressing.
        System.Threading.Thread.Sleep(100);
        var down = new Input[1];
        down[0].Data.Mouse.Flags = 2;
        var up = new Input[1];
        up[0].Data.Mouse.Flags = 4;
        Send(window, process, down);
        try { System.Threading.Thread.Sleep(80); }
        finally {
            if (SendInput(1, up, Marshal.SizeOf(typeof(Input))) != 1)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows did not release the game mouse button.");
        }
    }
}
'@
}
[void][Hoi4ProbeInput]::SetProcessDPIAware()
if ([Hoi4ProbeInput]::IsIconic($window)) { [void][Hoi4ProbeInput]::ShowWindow($window, 9) }
$previousForeground = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
try {
    Start-Sleep -Milliseconds 200
    [Hoi4ProbeInput]::CheckTarget($window, [uint32]$game.Id)
    if (-not [Hoi4ProbeInput]::IsWindowVisible($window)) { throw 'The game window is not visible.' }
    $rect = New-Object Hoi4ProbeInput+Rect
    if (-not [Hoi4ProbeInput]::GetWindowRect($window, [ref]$rect)) { throw 'Cannot read the game window rectangle.' }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) { throw 'The game window has an empty rectangle.' }

    switch ($Action) {
    'Screenshot' {
        $captureRoot = 'captures'
        if (-not $Out) { $Out = Join-Path $captureRoot ('probe\game-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '.png') }
        $capturePath = [IO.Path]::GetFullPath($Out)
        if (-not $capturePath.StartsWith($captureRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetExtension($capturePath) -ine '.png') {
            throw "Screenshots must be PNG files under $captureRoot."
        }
        if (Test-Path -LiteralPath $capturePath) { throw "Screenshot already exists: $capturePath" }
        Add-Type -AssemblyName System.Drawing
        Add-Type -AssemblyName System.Windows.Forms
        $desktop = [Windows.Forms.SystemInformation]::VirtualScreen
        if ($rect.Left -lt $desktop.Left -or $rect.Top -lt $desktop.Top -or $rect.Right -gt $desktop.Right -or $rect.Bottom -gt $desktop.Bottom) {
            throw 'The game window must fit entirely on the desktop for a screenshot.'
        }
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($capturePath))
        $bitmap = New-Object Drawing.Bitmap $width, $height
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            [Hoi4ProbeInput]::CheckTarget($window, [uint32]$game.Id)
            $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
            $bitmap.Save($capturePath, [Drawing.Imaging.ImageFormat]::Png)
        } finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
        [pscustomobject]@{ Action = $Action; GameId = $game.Id; Path = $capturePath; Width = $width; Height = $height; Left = $rect.Left; Top = $rect.Top }
    }
    'Key' {
        if (-not $Key) { throw 'Key requires -Key.' }
        $keys = @{ Space=0x20; Escape=0x1B; Enter=0x0D; Grave=0xC0; Tab=9; Backspace=8; Delete=0x2E;
            Left=0x25; Right=0x27; Up=0x26; Down=0x28; Home=0x24; End=0x23 }
        for ($index = 1; $index -le 12; $index++) { $keys["F$index"] = 0x6F + $index }
        for ($index = 1; $index -le 5; $index++) { $keys[[string]$index] = 0x30 + $index }
        [Hoi4ProbeInput]::KeyPress($window, [uint32]$game.Id, [uint16]$keys[$Key])
        [pscustomobject]@{ Action = $Action; GameId = $game.Id; Key = $Key }
    }
    { $_ -eq 'Click' -or $_ -eq 'Move' -or $_ -eq 'Wheel' } {
        if ($X -lt 0 -or $Y -lt 0 -or $X -ge $width -or $Y -ge $height) { throw "$Action coordinates must lie inside the screenshot rectangle." }
        $client = New-Object Hoi4ProbeInput+Rect
        $origin = New-Object Hoi4ProbeInput+Point
        if (-not [Hoi4ProbeInput]::GetClientRect($window, [ref]$client) -or -not [Hoi4ProbeInput]::ClientToScreen($window, [ref]$origin)) {
            throw 'Cannot locate the game client area.'
        }
        $screenX = $rect.Left + $X
        $screenY = $rect.Top + $Y
        if ($screenX -lt $origin.X -or $screenY -lt $origin.Y -or $screenX -ge $origin.X + $client.Right -or $screenY -ge $origin.Y + $client.Bottom) {
            if ($Action -ne 'Click') { throw "$Action coordinates must be inside the game client area, not the window frame." }
            throw 'Clicks must be inside the game client area, not the window frame.'
        }
        if ($Action -eq 'Move') {
            [Hoi4ProbeInput]::Move($window, [uint32]$game.Id, $screenX, $screenY)
        } elseif ($Action -eq 'Wheel') {
            if ($Delta -eq 0) { throw 'Wheel requires a nonzero -Delta.' }
            [Hoi4ProbeInput]::Wheel($window, [uint32]$game.Id, $screenX, $screenY, $Delta)
        } else {
            [Hoi4ProbeInput]::Click($window, [uint32]$game.Id, $screenX, $screenY)
        }
        [pscustomobject]@{ Action = $Action; GameId = $game.Id; X = $X; Y = $Y; Delta = $Delta }
    }
    { $_ -eq 'Text' -or $_ -eq 'Console' } {
        if (-not $Text -or $Text -match '[\r\n\x00-\x1F]') { throw 'Text and Console require one nonempty line without control characters.' }
        if ($Action -eq 'Console') {
            [Hoi4ProbeInput]::KeyPress($window, [uint32]$game.Id, [uint16]$ConsoleKey)
            Start-Sleep -Milliseconds 200
        }
        [Hoi4ProbeInput]::TypeText($window, [uint32]$game.Id, $Text)
        if ($Action -eq 'Console') {
            [Hoi4ProbeInput]::KeyPress($window, [uint32]$game.Id, 0x0D)
            Start-Sleep -Milliseconds 200
            [Hoi4ProbeInput]::KeyPress($window, [uint32]$game.Id, [uint16]$ConsoleKey)
        }
        [pscustomobject]@{ Action = $Action; GameId = $game.Id; Text = $Text }
    }
    }
} finally {
    [Hoi4ProbeInput]::RestoreForeground($previousForeground, $window)
}
