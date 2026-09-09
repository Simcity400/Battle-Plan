<#
.SYNOPSIS
Milestone 1.1 game input and screenshots for the sampling probe.
.DESCRIPTION
The full product smoke check is not implemented yet. Run explicit actions to
drive the game during the sampling proof. This script does not inject, read
game memory, modify settings, or certify that a save finished loading.

Screenshot and click coordinates are relative to the outer game window.
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
#>
[CmdletBinding()]
param(
    [ValidateSet('Smoke', 'Launch', 'Screenshot', 'Key', 'Click', 'Text', 'Console')]
    [string]$Action = 'Smoke',
    [string]$GameExe = 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe',
    [string]$UserDirectory = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV',
    [ValidateRange(0, 2147483647)][int]$GameId = 0,
    [switch]$ContinueLastSave,
    [string]$ExpectedSave,
    [string]$Out,
    [ValidateSet('Space', 'Escape', 'Enter', 'Grave', 'Tab', 'Backspace', 'Delete',
        'Left', 'Right', 'Up', 'Down', 'Home', 'End', '1', '2', '3', '4', '5',
        'F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12')]
    [string]$Key,
    [ValidateRange(-1, 32767)][int]$X = -1,
    [ValidateRange(-1, 32767)][int]$Y = -1,
    [ValidateLength(1, 512)][string]$Text,
    [ValidateRange(1, 254)][int]$ConsoleKey = 0xC0
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($Action -eq 'Smoke') {
    throw 'The full product smoke check awaits Milestone 2.4. Milestone 1.1 currently supports explicit Launch, Screenshot, Key, Click, Text, and Console actions only.'
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
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
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
[void][Hoi4ProbeInput]::SetForegroundWindow($window)
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
    'Click' {
        if ($X -lt 0 -or $Y -lt 0 -or $X -ge $width -or $Y -ge $height) { throw 'Click coordinates must lie inside the screenshot rectangle.' }
        $client = New-Object Hoi4ProbeInput+Rect
        $origin = New-Object Hoi4ProbeInput+Point
        if (-not [Hoi4ProbeInput]::GetClientRect($window, [ref]$client) -or -not [Hoi4ProbeInput]::ClientToScreen($window, [ref]$origin)) {
            throw 'Cannot locate the game client area.'
        }
        $screenX = $rect.Left + $X
        $screenY = $rect.Top + $Y
        if ($screenX -lt $origin.X -or $screenY -lt $origin.Y -or $screenX -ge $origin.X + $client.Right -or $screenY -ge $origin.Y + $client.Bottom) {
            throw 'Clicks must be inside the game client area, not the window frame.'
        }
        [Hoi4ProbeInput]::Click($window, [uint32]$game.Id, $screenX, $screenY)
        [pscustomobject]@{ Action = $Action; GameId = $game.Id; X = $X; Y = $Y }
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
