<#
.SYNOPSIS
Full live recording/save/oracle/render smoke and guarded game-driving actions.
.DESCRIPTION
Smoke uses receipt-verified final binaries with an explicit save, calibrated UiConfig,
and a new Out directory. An explicit Checkpoint runs the workspace checks
before building the three release artifacts.
It preserves the input save and restores the isolated user
profile selection/settings after normal exit. Explicit actions remain available. Baseline reads only the game's clock,
through recorder.exe's clock command on any game build; it sends no input and does not inject.
Launch does not modify settings or certify that a save finished loading.
Reload requires a held GameLock and its exact LockOwner token, an explicit GameId,
and UiConfig naming expected_save, expected_hour, expected_seed, menu, load_game,
load_local, load_row, load_row_step, load_visible_rows, cursor_park and load_button,
with calibrated pause-menu, Load-dialog and Load-label references. It requires the background shim
and a changed load timestamp with the expected stable clock and seed.
Launch and Smoke require a physical user directory without subst drives, junction or
symbolic-link ancestors or hyphens in its path. ContinueLastSave requires ExpectedSave;
the driver writes a complete ASCII-safe, BOM-free UTF-8 continue_game.json for that
local save before launching. Smoke restores the original selection bytes after exit.

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
.PARAMETER GameExe
Path to the installed hoi4.exe, or set BATTLEPLAN_GAME_DIR. A build this release does not recognise is a warning.
.PARAMETER CaptureRoot
User-selected output root; defaults to LOCALAPPDATA/BattlePlan/captures.
.PARAMETER GameLock
Shared lock directory used to prevent concurrent full smoke runs.
.PARAMETER ContinueLastSave
Launch directly into ExpectedSave, writing the game's continuation selection first.
.PARAMETER ExpectedSave
Existing local .hoi4 filename in the user directory's save games folder.
Required with ContinueLastSave and for Smoke.
.PARAMETER UiConfig
JSON containing width/height, expected_hour/expected_seed, calibrated two-element
pixel coordinates for menu, speed_up, speed_down, pause, save_menu, save_name,
save_confirm, save_back, menu_close, exit_game, exit_confirm, map_center and
capture_cursor, and calibrated reference images and origins for menu_patch,
save_dialog_patch and exit_confirm_patch. The capture cursor must lie over inert
UI chrome without a tooltip.
.PARAMETER WaitForUiReview
Before saving, pause for a visual review of dynamic informational notifications.
The guarded driving actions can dismiss them. A ui-review-approved.json file in Out
must then contain the exact hour and unobscured=true. Timeout is ten minutes.
This is a graphical pre-save checkpoint, not a waiver of any correctness check.
.PARAMETER AllowHeldCaptures
Accept captures held on the game thread. Without it the smoke fails when the capture status
shows captures held instead of beside the game, for example because the command-batch hook
is missing or its publication unproven: the smoke certifies the shipped path.
.PARAMETER InputShimDir
Directory holding inputshim.dll and inputshim-inject.exe; supply an existing shim build.
In background mode Launch attaches the shim, and Click
and Wheel then work with no foreground change at all: while the messages are posted the shim
answers the game's user32 GetCursorPos, GetAsyncKeyState, GetKeyState, GetKeyboardState,
GetForegroundWindow, GetActiveWindow and GetFocus imports from a shared control block.
Missing binaries are not an error; the driver falls back to the brief foreground borrow.
ShimStatus prints the per-hook call counters and their one-second deltas. ShimMask -Mask <n>
sets hook_mask (bit per hook, 0x7f enables all) to bisect what the click path depends on.
.PARAMETER RecordHours
Live advance in game hours at top speed. The smoke then requests a full refresh (recorder
refresh) and steps one hour at speed 1 to the saved hour, whose full sample is taken as the
next hourly update is entered, and compares that sample with the save.
.PARAMETER BuildReceipt
JSON with the current source_commit and exactly three binaries entries
(name, sha256, bytes) for recorder.exe, collector.dll and viewer.exe. A checkpoint
writes build-receipt.json in Out; existing smoke-result.json has the same fields.
.PARAMETER BinaryDirectory
Directory of those binaries; defaults to this checkout's .cargo-target/release.
All source and copied hashes and the clean current HEAD are verified before launch.
The other actions read the game clock with the recorder.exe in this directory (same default):
recorder.exe clock locates the game's sites on any build and names each field it withholds.
Smoke reads it with its own receipt-verified copy.
The viewer's pinned shader compiler, dxcompiler.dll and dxil.dll, is copied from beside
them and checked before launch.
.PARAMETER Checkpoint
Run workspace checks and build the release binaries before the live smoke.
Mutually exclusive with BuildReceipt/BinaryDirectory. Subsequent smokes use the
receipt and do not repeat the workspace checks.
.EXAMPLE
powershell -NoProfile -File tools/smoke.ps1 -Action Smoke -BuildReceipt "$env:LOCALAPPDATA\BattlePlan\captures\checkpoint\build-receipt.json" -BinaryDirectory "$env:LOCALAPPDATA\BattlePlan\captures\checkpoint\bin" -ExpectedSave save.hoi4 -UiConfig "$env:LOCALAPPDATA\BattlePlan\captures\smoke_config\ui.json" -Out "$env:LOCALAPPDATA\BattlePlan\captures\smoke_new" -WaitForUiReview
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Launch -ContinueLastSave -ExpectedSave baseline.hoi4
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Screenshot -Out "$env:LOCALAPPDATA\BattlePlan\captures\probe\menu.png"
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Key -Key Space
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Click -X 640 -Y 480
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Move -GameId 1234 -X 1280 -Y 720
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Wheel -X 1280 -Y 720 -Delta -3
.EXAMPLE
powershell -File tools/smoke.ps1 -Action Baseline -Seconds 120 -Out "$env:LOCALAPPDATA\BattlePlan\captures\baseline"
.EXAMPLE
powershell -File tools/smoke.ps1 -Action ShimStatus
.EXAMPLE
powershell -File tools/smoke.ps1 -Action ShimMask -Mask 127
#>
[CmdletBinding()]
param(
    [ValidateSet('Smoke', 'Launch', 'Reload', 'Screenshot', 'Key', 'Click', 'Move', 'Wheel', 'Text', 'Console', 'Baseline', 'Clock',
        'ShimStatus', 'ShimMask')]
    [string]$Action = 'Smoke',
    [string]$GameExe = '',
    [string]$UserDirectory = (Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Paradox Interactive\Hearts of Iron IV'),
    [ValidateRange(0, 2147483647)][int]$GameId = 0,
    [switch]$ContinueLastSave,
    [string]$ExpectedSave,
    [string]$Out,
    [string]$CaptureRoot = (Join-Path $env:LOCALAPPDATA 'BattlePlan\captures'),
    [string]$GameLock = (Join-Path $env:TEMP 'claude\hoi4_game.lock'),
    [string]$LockOwner,
    [ValidateRange(1, 2147483647)][int]$Seconds = 120,
    [ValidateSet('Space', 'Escape', 'Enter', 'Grave', 'Tab', 'Backspace', 'Delete',
        'Left', 'Right', 'Up', 'Down', 'Home', 'End', '1', '2', '3', '4', '5',
        'CtrlA', 'F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12')]
    [string]$Key,
    [ValidateRange(-1, 32767)][int]$X = -1,
    [ValidateRange(-1, 32767)][int]$Y = -1,
    [ValidateRange(-50, 50)][int]$Delta = 0,
    [ValidateLength(1, 512)][string]$Text,
    [string]$UiConfig,
    [string]$MenuPatch,
    [int[]]$MenuPatchOrigin = @(1150, 440),
    [string]$BuildReceipt,
    [string]$BinaryDirectory,
    [switch]$Checkpoint,
    [switch]$WaitForUiReview,
    [switch]$AllowHeldCaptures,
    [string]$Perspective = 'recorded',
    [ValidateRange(120, 10000)][int]$RecordHours = 120,
    [ValidateRange(300, 14400)][int]$TimeoutSeconds = 3600,
    [ValidateRange(1, 254)][int]$ConsoleKey = 0xC0,
    [switch]$Foreground,
    [switch]$KeepForeground,
    [switch]$Posted,
    [string]$InputShimDir = '',
    [switch]$RequireInputShim,
    [ValidateRange(-1, 255)][int]$Mask = -1,
    [string[]]$ExtraArgs = @()
)

$script:ShimMapping = 'Local\hoi4_inputshim_v1'
$script:ShimMagic = [uint32]0x4D485348
$script:ShimHooks = @('GetCursorPos', 'GetAsyncKeyState', 'GetKeyState', 'GetKeyboardState',
    'GetForegroundWindow', 'GetActiveWindow', 'GetFocus')
$script:ShimMaskOffset = 8
$script:ShimFlagsOffset = 12
$script:ShimCursorOffset = 16
$script:ShimWindowOffset = 24
$script:ShimCallsOffset = 32
$script:ShimCursorFlag = 1
$script:ShimFocusFlag = 2
$script:ShimLeftFlag = 4

function Get-GameBuild([string]$Path) {
    $source = [IO.File]::ReadAllText((Join-Path (Split-Path -Parent $PSScriptRoot) 'crates\engine-executable\src\build.rs'))
    $pinned = [regex]::Match($source, '(?s)pub const PINNED_BUILD: Build = Build \{\s*version: "([^"]+)",\s*exe_size: ([\d_]+),\s*exe_sha256: \[([^\]]+)\]')
    if (-not $pinned.Success) { throw 'PINNED_BUILD not found in crates/engine-executable/src/build.rs.' }
    $wantedSize = [int64]($pinned.Groups[2].Value -replace '_', '')
    $wantedHash = (-join ([regex]::Matches($pinned.Groups[3].Value, '0x([0-9a-fA-F]{2})') | ForEach-Object { $_.Groups[1].Value })).ToLowerInvariant()
    $bytes = [IO.File]::ReadAllBytes($Path)
    $hash = ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($bytes)) -replace '-', '').ToLowerInvariant()
    $text = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
    $banners = @([regex]::Matches($text, '(?<=\x00)[A-Z][A-Za-z ]{0,63} v\d{1,4}(?:\.\d{1,4}){2,}\.[0-9a-f]{1,8}(?=\x00)') | ForEach-Object Value | Sort-Object -Unique)
    $version = if ($banners.Count -eq 1) { $banners[0] } else { $null }
    $recognised = $bytes.LongLength -eq $wantedSize -and $hash -eq $wantedHash
    $found = "hoi4.exe of $($bytes.LongLength) bytes, SHA-256 $($hash.Substring(0, 12))..."
    $warning = if ($recognised) { $null } else {
        "Unrecognised game build: this release was made for version $($pinned.Groups[1].Value) (hoi4.exe of $wantedSize bytes, SHA-256 $($wantedHash.Substring(0, 12))...), " +
        "but the game is $(if ($version) { "$version ($found)" } else { $found }). Parts that fail their own checks on this build are withheld."
    }
    [pscustomobject]@{ Size = $bytes.LongLength; Sha256 = $hash; Version = $version; Recognised = $recognised; Warning = $warning }
}

function Write-GameBuildWarning([string]$Path) {
    $build = Get-GameBuild $Path
    if ($build.Warning) { Write-Warning $build.Warning }
    $build
}

. (Join-Path $PSScriptRoot 'game-clock.ps1')

function Open-InputShim {
    try { $map = [IO.MemoryMappedFiles.MemoryMappedFile]::OpenExisting($script:ShimMapping) } catch { return $null }
    $view = $map.CreateViewAccessor(0, 4096)
    if ($view.ReadUInt32(0) -ne $script:ShimMagic) { $view.Dispose(); $map.Dispose(); return $null }
    [pscustomobject]@{ Map = $map; View = $view }
}

function Close-InputShim($shim) {
    if ($shim) { $shim.View.Dispose(); $shim.Map.Dispose() }
}

function Read-ShimCounters($shim) {
    $counters = [ordered]@{}
    for ($index = 0; $index -lt $script:ShimHooks.Count; $index++) {
        $counters[$script:ShimHooks[$index]] = $shim.View.ReadUInt32($script:ShimCallsOffset + 4 * $index)
    }
    $counters
}

function Install-InputShim([int]$processId) {
    if (-not $InputShimDir) { return 'absent' }
    $injector = Join-Path $InputShimDir 'inputshim-inject.exe'
    $dll = Join-Path $InputShimDir 'inputshim.dll'
    if (-not (Test-Path -LiteralPath $injector) -or -not (Test-Path -LiteralPath $dll)) { return 'absent' }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $output = (& $injector attach $processId $dll 2>&1 | Out-String).Trim() }
    finally { $ErrorActionPreference = $previous }
    if ($LASTEXITCODE -ne 0) { return "failed: $output" }
    'attached'
}

function Get-SubstDriveNames {
    & (Join-Path $env:SystemRoot 'System32\subst.exe') | ForEach-Object {
        if ($_ -match '^([A-Za-z]:)\\:') { $Matches[1] }
    }
}

function Resolve-LaunchUserDirectory([string]$path) {
    $resolved = (Resolve-Path -LiteralPath $path).ProviderPath
    $drive = [IO.Path]::GetPathRoot($resolved).TrimEnd('\')
    if (@(Get-SubstDriveNames) -contains $drive) {
        throw "UserDirectory is on subst drive ${drive}. Copy the profile into a physical directory and use that same path for the game and recorder."
    }
    $directory = Get-Item -LiteralPath $resolved -Force
    if (-not $directory.PSIsContainer) { throw 'UserDirectory must be a directory.' }
    for ($entry = $directory; $null -ne $entry; $entry = $entry.Parent) {
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "UserDirectory traverses a junction or symbolic link: $($entry.FullName). Copy the profile into a physical directory and use that same path for the game and recorder."
        }
    }
    if ($resolved.Contains('"')) { throw 'The user directory contains an invalid quote.' }
    if ($resolved.Contains('-')) { throw 'This game build splits command-line values at hyphens. Use a user directory whose entire path contains no hyphen.' }
    $resolved
}

function Write-LaunchSelection([string]$userPath, [string]$saveName) {
    if ([string]::IsNullOrWhiteSpace($saveName) -or $saveName.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0 -or $saveName -notlike '*.hoi4') {
        throw 'ExpectedSave must be a local .hoi4 save filename.'
    }
    $savePath = Join-Path (Join-Path $userPath 'save games') $saveName
    if (-not (Test-Path -LiteralPath $savePath -PathType Leaf)) { throw "ExpectedSave is missing: $saveName" }
    $modified = (Get-Item -LiteralPath $savePath).LastWriteTime
    $culture = [Globalization.CultureInfo]::InvariantCulture
    $date = $modified.ToString('ddd MMM', $culture) + (' {0,2} ' -f $modified.Day) + $modified.ToString('HH:mm:ss yyyy', $culture) + "`n"
    $json = [ordered]@{ title=$saveName; desc=''; date=$date; filename=$saveName; is_remote=$false } | ConvertTo-Json -Compress
    $json = [regex]::Replace($json, '[^\x00-\x7F]', { param($match) '\u{0:x4}' -f [int][char]$match.Value })
    [IO.File]::WriteAllText((Join-Path $userPath 'continue_game.json'), $json, [Text.UTF8Encoding]::new($false))
}

function Copy-ShaderCompiler([string]$source, [string]$destination) {
    $names = @('dxcompiler.dll', 'dxil.dll')
    foreach ($name in $names) {
        if (-not (Test-Path -LiteralPath (Join-Path $source $name) -PathType Leaf)) {
            throw "The viewer needs the pinned shader compiler: install it beside the smoke binaries (tools/install-shader-compiler.ps1 -Destination '$source')."
        }
    }
    foreach ($name in $names) { Copy-Item -LiteralPath (Join-Path $source $name) -Destination (Join-Path $destination $name) }
    Join-Path $destination 'dxcompiler.dll'
}

function Invoke-WithFocusRetry {
    param(
        [Parameter(Mandatory)][scriptblock]$Operation,
        [int]$MaxAttempts = 5,
        [bool]$AllowFocusLost = $true,
        [scriptblock]$OnRetry = { Start-Sleep -Milliseconds 250 }
    )
    if ($MaxAttempts -lt 1) { $MaxAttempts = 1 }
    for ($retryIndex = 1; ; $retryIndex++) {
        try { return (& $Operation) }
        catch {
            $retryMessage = $_.Exception.Message
            $refused = $retryMessage -like '*Windows refused to focus the game*'
            $lostBeforeInput = $AllowFocusLost -and $retryMessage -like '*Game focus was lost; input was not sent.*'
            $isRetryable = ($refused -or $lostBeforeInput) -and ($retryMessage -notlike '*partial input*')
            if (-not $isRetryable -or $retryIndex -ge $MaxAttempts) { throw }
            & $OnRetry $retryIndex $retryMessage
        }
    }
}

function Invoke-SequentialFocusInput {
    param(
        [Parameter(Mandatory)][object[]]$Items,
        [Parameter(Mandatory)][scriptblock]$SendItem,
        [Parameter(Mandatory)][scriptblock]$Reacquire,
        [int]$MaxAttemptsPerItem = 5,
        [scriptblock]$Pause = { Start-Sleep -Milliseconds 150 }
    )
    if ($MaxAttemptsPerItem -lt 1) { $MaxAttemptsPerItem = 1 }
    $deliveredCount = 0
    $totalCount = $Items.Count
    foreach ($seqItem in $Items) {
        for ($itemAttempt = 1; ; $itemAttempt++) {
            try {
                & $SendItem $seqItem
                $deliveredCount++
                break
            } catch {
                $itemMessage = $_.Exception.Message
                $focusError = $itemMessage -like '*Game focus was lost*' -or $itemMessage -like '*Windows refused to focus the game*'
                if (-not $focusError) { throw }
                if ($itemAttempt -ge $MaxAttemptsPerItem) {
                    if ($deliveredCount -gt 0) {
                        throw "Game focus was lost after delivering partial input ($deliveredCount of $totalCount)."
                    }
                    throw
                }
                & $Pause $itemAttempt $itemMessage
                try {
                    & $Reacquire
                } catch {
                    if ($deliveredCount -gt 0) {
                        throw "Game focus was lost after delivering partial input ($deliveredCount of $totalCount): $($_.Exception.Message)"
                    }
                    throw
                }
            }
        }
    }
}

function Get-UiRegionBox([int[]]$Point, [int]$Width, [int]$Height, [int]$HalfWidth = 120, [int]$HalfHeight = 60) {
    $x0 = [Math]::Max(0, [int]$Point[0] - $HalfWidth)
    $y0 = [Math]::Max(0, [int]$Point[1] - $HalfHeight)
    $x1 = [Math]::Min($Width, [int]$Point[0] + $HalfWidth)
    $y1 = [Math]::Min($Height, [int]$Point[1] + $HalfHeight)
    @($x0, $y0, $x1, $y1)
}

function Test-UiPatchMatch {
    param(
        [Parameter(Mandatory)][string]$ImagePath,
        [Parameter(Mandatory)][string]$PatchPath,
        [Parameter(Mandatory)][int[]]$Origin,
        [double]$Tolerance = 22.0,
        [switch]$Label
    )
    Add-Type -AssemblyName System.Drawing
    $reference = [Drawing.Bitmap]::FromFile($PatchPath)
    $image = [Drawing.Bitmap]::FromFile($ImagePath)
    try {
        if ($Origin.Count -ne 2 -or $Origin[0] -lt 0 -or $Origin[1] -lt 0 -or
            $Origin[0] + $reference.Width -gt $image.Width -or $Origin[1] + $reference.Height -gt $image.Height) {
            throw 'Menu reference lies outside the calibrated window.'
        }
        $sum = 0.0; $count = 0; $glyphs = 0; $difference = 0; $union = 0
        $stride = if ($Label) { 1 } else { 4 }
        $threshold = 0.0
        if ($Label) {
            $minimum = 765; $maximum = 0
            for ($py = 0; $py -lt $reference.Height; $py++) {
                for ($px = 0; $px -lt $reference.Width; $px++) {
                    $pixel = $reference.GetPixel($px, $py)
                    $brightness = $pixel.R + $pixel.G + $pixel.B
                    $minimum = [Math]::Min($minimum, $brightness)
                    $maximum = [Math]::Max($maximum, $brightness)
                }
            }
            if ($minimum -eq $maximum) { return $false }
            $threshold = ($minimum + $maximum) / 2.0
        }
        for ($py = 0; $py -lt $reference.Height; $py += $stride) {
            for ($px = 0; $px -lt $reference.Width; $px += $stride) {
                $a = $reference.GetPixel($px, $py)
                $b = $image.GetPixel($Origin[0] + $px, $Origin[1] + $py)
                if ($Label) {
                    $expected = ($a.R + $a.G + $a.B -ge $threshold)
                    $actual = ($b.R + $b.G + $b.B -ge $threshold)
                    if ($expected) { $glyphs++ }
                    if ($expected -or $actual) { $union++ }
                    if ($expected -ne $actual) { $difference++ }
                }
                $sum += [Math]::Abs($a.R - $b.R) + [Math]::Abs($a.G - $b.G) + [Math]::Abs($a.B - $b.B)
                $count++
            }
        }
        if ($Label) {
            if ($glyphs -gt $count / 2) { $glyphs = $count - $glyphs; $union = $count - $union + $difference }
            return ($glyphs -gt 0 -and $union -gt 0 -and 100.0 * $difference / $union -lt $Tolerance)
        }
        return ($count -gt 0 -and $sum / (3.0 * $count) -lt $Tolerance)
    } finally {
        $reference.Dispose()
        $image.Dispose()
    }
}

function Resolve-UiDialogProbe {
    param(
        [Parameter(Mandatory)]$Ui,
        [Parameter(Mandatory)][string]$PatchProperty,
        [Parameter(Mandatory)][string]$OriginProperty,
        [Parameter(Mandatory)][string]$DialogLabel,
        [double]$Tolerance = 22.0,
        [switch]$Label,
        [string]$OverridePatch = '',
        [int[]]$OverrideOrigin = @()
    )
    if ($OverridePatch) {
        $patch = $OverridePatch
        $origin = [int[]]@($OverrideOrigin)
    } else {
        $patch = if ($Ui.PSObject.Properties[$PatchProperty] -and $Ui.$PatchProperty) { [string]$Ui.$PatchProperty } else { '' }
        $origin = [int[]]@(if ($Ui.PSObject.Properties[$OriginProperty] -and $null -ne $Ui.$OriginProperty) { $Ui.$OriginProperty })
    }
    if (-not $patch -or -not (Test-Path -LiteralPath $patch -PathType Leaf) -or @($origin).Count -ne 2 -or $origin[0] -lt 0 -or $origin[1] -lt 0) {
        throw "Smoke requires a calibrated $DialogLabel reference image and origin ($PatchProperty / $OriginProperty)."
    }
    $patch = (Resolve-Path -LiteralPath $patch).ProviderPath
    Add-Type -AssemblyName System.Drawing
    $bitmap = $null
    try {
        try {
            $bitmap = [Drawing.Bitmap]::FromFile($patch)
        } catch {
            throw "Smoke requires a readable calibrated $DialogLabel reference image ($PatchProperty): $($_.Exception.Message)"
        }
        $patchWidth = [int]$bitmap.Width
        $patchHeight = [int]$bitmap.Height
    } finally {
        if ($null -ne $bitmap) { $bitmap.Dispose() }
    }
    $uiWidth = if ($Ui.PSObject.Properties['width'] -and $null -ne $Ui.width) { [int]$Ui.width } else { 0 }
    $uiHeight = if ($Ui.PSObject.Properties['height'] -and $null -ne $Ui.height) { [int]$Ui.height } else { 0 }
    if ($patchWidth -le 0 -or $patchHeight -le 0 -or $uiWidth -le 0 -or $uiHeight -le 0 -or
        ($origin[0] + $patchWidth) -gt $uiWidth -or
        ($origin[1] + $patchHeight) -gt $uiHeight) {
        throw "Smoke calibrated $DialogLabel reference image ($PatchProperty / $OriginProperty) exceeds the calibrated window bounds (${uiWidth}x${uiHeight})."
    }
    [pscustomobject]@{ Mode = 'Patch'; Patch = $patch; Origin = $origin; Tolerance = $Tolerance; Label = [bool]$Label }
}

function Resolve-SmokeUiProbes {
    param(
        [Parameter(Mandatory)]$Ui,
        [string]$MenuPatch = '',
        [int[]]$MenuPatchOrigin = @()
    )
    [pscustomobject]@{
        Menu = Resolve-UiDialogProbe -Ui $Ui -PatchProperty 'menu_patch' -OriginProperty 'menu_patch_origin' -DialogLabel 'pause-menu' -Tolerance 22 -OverridePatch $MenuPatch -OverrideOrigin $MenuPatchOrigin
        SaveDialog = Resolve-UiDialogProbe -Ui $Ui -PatchProperty 'save_dialog_patch' -OriginProperty 'save_dialog_patch_origin' -DialogLabel 'save-dialog' -Tolerance 22
        ExitConfirm = Resolve-UiDialogProbe -Ui $Ui -PatchProperty 'exit_confirm_patch' -OriginProperty 'exit_confirm_patch_origin' -DialogLabel 'exit-confirmation' -Tolerance 22
    }
}

function Test-UiDialogState {
    param(
        [Parameter(Mandatory)][string]$ShotPath,
        [Parameter(Mandatory)]$Probe
    )
    if ($Probe.Mode -ne 'Patch') {
        throw "Unsupported dialog probe mode: $($Probe.Mode)"
    }
    Test-UiPatchMatch -ImagePath $ShotPath -PatchPath $Probe.Patch -Origin $Probe.Origin -Tolerance $Probe.Tolerance -Label:$Probe.Label
}

function Reset-UiToCleanMap {
    param(
        [Parameter(Mandatory)][scriptblock]$IsAnyDialogOpen,
        [Parameter(Mandatory)][scriptblock]$SendEscape,
        [int]$MaxEscapes = 2,
        [scriptblock]$ShouldStop = { $false },
        [scriptblock]$Pause = { Start-Sleep -Milliseconds 300 }
    )
    for ($escapeStep = 0; $escapeStep -lt $MaxEscapes; $escapeStep++) {
        if (& $ShouldStop) { return $false }
        if (-not (& $IsAnyDialogOpen)) { return $true }
        & $SendEscape
        & $Pause $escapeStep
    }
    if (& $ShouldStop) { return $false }
    return (-not (& $IsAnyDialogOpen))
}

function Invoke-ObservedUiStep {
    param(
        [Parameter(Mandatory)][scriptblock]$Act,
        [Parameter(Mandatory)][scriptblock]$Observe,
        [int]$MaxPolls = 20,
        [scriptblock]$Pause = { Start-Sleep -Milliseconds 250 }
    )
    if ($MaxPolls -lt 1) { $MaxPolls = 1 }
    & $Act
    for ($pollIndex = 1; $pollIndex -le $MaxPolls; $pollIndex++) {
        if (& $Observe $pollIndex) { return $true }
        if ($pollIndex -lt $MaxPolls) { & $Pause $pollIndex }
    }
    $false
}

function Invoke-BoundedUiSequence {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$FailureMessage,
        [Parameter(Mandatory)][scriptblock]$Attempt,
        [scriptblock]$Reset = {},
        [int]$MaxAttempts = 3
    )
    if ($MaxAttempts -lt 1) { $MaxAttempts = 1 }
    $seqFailures = @()
    for ($seqTry = 1; $seqTry -le $MaxAttempts; $seqTry++) {
        if ($seqTry -gt 1) { & $Reset $seqTry }
        $seqResult = & $Attempt $seqTry
        if ($seqResult -and $seqResult.Done) {
            return [pscustomobject]@{ Name = $Name; Done = $true; Attempts = $seqTry; Failures = $seqFailures; Result = $seqResult }
        }
        $seqReason = if ($seqResult -and $seqResult.PSObject.Properties['Reason'] -and $seqResult.Reason) { [string]$seqResult.Reason } else { 'effect not observed' }
        $seqFailures += "attempt ${seqTry}: $seqReason"
    }
    throw "$FailureMessage ($($seqFailures -join '; '))"
}

function Invoke-UiSaveFlow {
    param(
        [Parameter(Mandatory)][scriptblock]$IsSaveComplete,
        [Parameter(Mandatory)][scriptblock]$CheckClock,
        [Parameter(Mandatory)][scriptblock]$PauseGame,
        [Parameter(Mandatory)][scriptblock]$Click,
        [Parameter(Mandatory)][scriptblock]$TypeSaveName,
        [Parameter(Mandatory)][scriptblock]$SendEscape,
        [Parameter(Mandatory)][scriptblock]$IsMenuOpen,
        [Parameter(Mandatory)][scriptblock]$IsSaveDialogOpen,
        [Parameter(Mandatory)][scriptblock]$WaitForSave,
        [scriptblock]$OnMenuOpened = {},
        [int]$MaxAttempts = 3,
        [int]$MaxPolls = 20,
        [scriptblock]$Pause = { Start-Sleep -Milliseconds 250 }
    )
    $anyOpen = { (& $IsSaveDialogOpen) -or (& $IsMenuOpen) }
    $ensureClean = {
        $clean = Reset-UiToCleanMap -IsAnyDialogOpen $anyOpen -SendEscape $SendEscape -MaxEscapes 2 -ShouldStop $IsSaveComplete -Pause $Pause
        if (& $IsSaveComplete) { return $true }
        & $CheckClock
        & $PauseGame
        $clean
    }
    Invoke-BoundedUiSequence -Name 'save' -MaxAttempts $MaxAttempts -FailureMessage 'The UI did not produce a complete text save.' -Reset {
        if (& $IsSaveComplete) { return }
        $null = & $ensureClean
    } -Attempt {
        param($saveTry)
        if (& $IsSaveComplete) { return [pscustomobject]@{ Done = $true; Reason = $null } }
        & $CheckClock
        if ($saveTry -eq 1 -and (& $anyOpen)) {
            $null = & $ensureClean
            if (& $IsSaveComplete) { return [pscustomobject]@{ Done = $true; Reason = $null } }
        }
        if (& $anyOpen) {
            return [pscustomobject]@{ Done = $false; Reason = 'starting UI state was not the clean map' }
        }
        $menuOpened = Invoke-ObservedUiStep -MaxPolls $MaxPolls -Pause $Pause -Act { & $Click 'menu' } -Observe { & $IsMenuOpen }
        if (-not $menuOpened) {
            return [pscustomobject]@{ Done = $false; Reason = 'menu click did not open the pause menu' }
        }
        & $OnMenuOpened
        $dialogOpened = Invoke-ObservedUiStep -MaxPolls $MaxPolls -Pause $Pause -Act { & $Click 'save_menu' } -Observe { & $IsSaveDialogOpen }
        if (-not $dialogOpened) {
            return [pscustomobject]@{ Done = $false; Reason = 'save_menu click did not open the save dialog' }
        }
        & $Click 'save_name'
        & $TypeSaveName
        & $Click 'save_confirm'
        & $WaitForSave $saveTry
    }
}

function Invoke-UiExitFlow {
    param(
        [Parameter(Mandatory)][scriptblock]$HasExited,
        [Parameter(Mandatory)][scriptblock]$GetExitCode,
        [Parameter(Mandatory)][scriptblock]$WaitForExit,
        [Parameter(Mandatory)][scriptblock]$PauseGame,
        [Parameter(Mandatory)][scriptblock]$Click,
        [Parameter(Mandatory)][scriptblock]$SendEscape,
        [Parameter(Mandatory)][scriptblock]$IsMenuOpen,
        [Parameter(Mandatory)][scriptblock]$IsConfirmOpen,
        [scriptblock]$OnMenuOpened = {},
        [scriptblock]$OnConfirmationSent = {},
        [int]$MaxAttempts = 3,
        [int]$MaxPolls = 20,
        [scriptblock]$Pause = { Start-Sleep -Milliseconds 250 }
    )
    $exitState = @{ ConfirmationSent = $false }
    $checkExitState = {
        if (& $HasExited) {
            $code = & $GetExitCode
            if ($exitState.ConfirmationSent) {
                if ($code -eq 0) {
                    return [pscustomobject]@{ Done = $true; Reason = $null; ConfirmationSent = $true; ExitCode = 0 }
                }
                throw "The game exited with non-zero exit code $code after exit confirmation."
            }
            throw "The game exited unexpectedly before exit confirmation was sent (exit code $code)."
        }
        $null
    }
    $anyOpen = {
        if (& $HasExited) { return $false }
        (& $IsConfirmOpen) -or (& $IsMenuOpen)
    }
    $null = & $checkExitState
    $seq = Invoke-BoundedUiSequence -Name 'exit' -MaxAttempts $MaxAttempts -FailureMessage 'The game did not exit through its confirmation dialog.' -Reset {
        if (& $HasExited) {
            if (-not $exitState.ConfirmationSent) { $null = & $checkExitState }
            return
        }
        $null = Reset-UiToCleanMap -IsAnyDialogOpen $anyOpen -SendEscape $SendEscape -MaxEscapes 2 -ShouldStop { & $HasExited } -Pause $Pause
        if (& $HasExited) {
            if (-not $exitState.ConfirmationSent) { $null = & $checkExitState }
            return
        }
        & $PauseGame
    } -Attempt {
        param($exitTry)
        $exited = & $checkExitState
        if ($null -ne $exited) { return $exited }
        & $PauseGame
        $exited = & $checkExitState
        if ($null -ne $exited) { return $exited }
        if ($exitTry -eq 1 -and (& $anyOpen)) {
            $null = Reset-UiToCleanMap -IsAnyDialogOpen $anyOpen -SendEscape $SendEscape -MaxEscapes 2 -ShouldStop { & $HasExited } -Pause $Pause
            $exited = & $checkExitState
            if ($null -ne $exited) { return $exited }
        }
        if (& $anyOpen) {
            return [pscustomobject]@{ Done = $false; Reason = 'starting UI state was not the clean map' }
        }
        $menuOpened = Invoke-ObservedUiStep -MaxPolls $MaxPolls -Pause $Pause -Act {
            if (& $HasExited) { return }
            & $Click 'menu'
        } -Observe {
            if (& $HasExited) { return $true }
            & $IsMenuOpen
        }
        $exited = & $checkExitState
        if ($null -ne $exited) { return $exited }
        if (-not $menuOpened) {
            return [pscustomobject]@{ Done = $false; Reason = 'menu click did not open the pause menu' }
        }
        & $OnMenuOpened
        $confirmOpened = Invoke-ObservedUiStep -MaxPolls $MaxPolls -Pause $Pause -Act {
            if (& $HasExited) { return }
            & $Click 'exit_game'
        } -Observe {
            if (& $HasExited) { return $true }
            & $IsConfirmOpen
        }
        $exited = & $checkExitState
        if ($null -ne $exited) { return $exited }
        if (-not $confirmOpened) {
            return [pscustomobject]@{ Done = $false; Reason = 'exit_game click did not open the confirmation dialog' }
        }
        & $Click 'exit_confirm'
        $exitState.ConfirmationSent = $true
        & $OnConfirmationSent
        foreach ($waitMs in @(20000, 40000)) {
            if ((& $WaitForExit $waitMs) -or (& $HasExited)) {
                $code = & $GetExitCode
                if ($code -ne 0) {
                    throw "The game exited with non-zero exit code $code after exit confirmation."
                }
                return [pscustomobject]@{ Done = $true; Reason = $null; ConfirmationSent = $true; ExitCode = 0 }
            }
        }
        throw 'The game did not exit through its confirmation dialog (game did not exit after exit_confirm).'
    }
    [pscustomobject]@{
        Name = 'exit'
        Done = $true
        Attempts = $seq.Attempts
        Failures = $seq.Failures
        ConfirmationSent = [bool]$seq.Result.ConfirmationSent
        ExitCode = [int]$seq.Result.ExitCode
    }
}

function Invoke-ExitThenVerify {
    param(
        [Parameter(Mandatory)][scriptblock]$ExitGame,
        [Parameter(Mandatory)][scriptblock]$Verify,
        [Parameter(Mandatory)]$Report
    )
    $exitFailure = $null
    try {
        & $ExitGame
        $Report.normal_game_exit = $true
    } catch {
        $exitFailure = $_
        $Report.normal_game_exit = $false
        $Report.exit_error = $_.Exception.Message
    }
    & $Verify
    if ($null -ne $exitFailure) { throw $exitFailure }
}

function Read-SharedUtf8Text([string]$Path) {
    if (-not $Path -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) { return '' }
    $stream = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    try { [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8).ReadToEnd() } finally { $stream.Dispose() }
}

function Start-SmokeTool {
    param(
        [Parameter(Mandatory)][string]$Exe,
        [string[]]$Arguments = @(),
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [Parameter(Mandatory)][string]$OutputDir,
        [Collections.IList]$Tracker = $null
    )
    $stdoutPath = Join-Path $OutputDir ($Name + '.stdout.log')
    $stderrPath = Join-Path $OutputDir ($Name + '.stderr.log')
    $logPath = Join-Path $OutputDir ($Name + '.log')
    $joinedArgs = ($Arguments | ForEach-Object {
        if ($_ -match '"') { throw 'Native argument contains a quote.' }
        '"' + ($_ -replace '(\\+)$', '$1$1') + '"'
    }) -join ' '
    $startParams = @{
        FilePath = $Exe
        WorkingDirectory = $WorkingDirectory
        WindowStyle = 'Hidden'
        PassThru = $true
        RedirectStandardOutput = $stdoutPath
        RedirectStandardError = $stderrPath
    }
    if ($joinedArgs.Length -gt 0) { $startParams.ArgumentList = $joinedArgs }
    $process = Microsoft.PowerShell.Management\Start-Process @startParams
    $null = $process.Handle
    $run = [pscustomobject]@{
        Process = $process
        StdoutPath = $stdoutPath
        StderrPath = $stderrPath
        LogPath = $logPath
        Name = $Name
        Started = [DateTime]::UtcNow
        Command = @($Exe) + $Arguments
    }
    if ($null -ne $Tracker) { $null = $Tracker.Add($run) }
    $run
}

function Get-ActiveSmokeTools($Tracker) {
    if ($null -eq $Tracker) { return @() }
    $active = @()
    foreach ($entry in $Tracker) {
        if ($null -eq $entry -or $null -eq $entry.Process) { continue }
        try { $entry.Process.Refresh() } catch {}
        if (-not $entry.Process.HasExited) { $active += $entry }
    }
    $active
}

function Invoke-FailureGamePause {
    param(
        $Game,
        $Reader,
        [bool]$ExitConfirmationSent,
        [Parameter(Mandatory)]$Report,
        [Parameter(Mandatory)][scriptblock]$SendEscape,
        [Parameter(Mandatory)][scriptblock]$PauseGame
    )
    if ($null -eq $Game -or $null -eq $Reader -or $ExitConfirmationSent) { return }
    try { if ($Game.PSObject.Methods['Refresh']) { $Game.Refresh() } } catch {}
    if ($Game.HasExited) { return }
    try {
        & $SendEscape
        & $PauseGame
    } catch {
        $Report.pause_cleanup_error = $_.Exception.Message
    }
}

function Invoke-FailureRecorderCleanup {
    param(
        $RecorderRun,
        $ToolTracker,
        [Parameter(Mandatory)]$Report,
        [Parameter(Mandatory)][scriptblock]$RunDetach,
        [Parameter(Mandatory)][scriptblock]$WaitRecorder
    )
    if ($null -eq $RecorderRun -or $null -eq $RecorderRun.Process) { return }
    try { if ($RecorderRun.Process.PSObject.Methods['Refresh']) { $RecorderRun.Process.Refresh() } } catch {}
    if ($RecorderRun.Process.HasExited) { return }
    $activeDetach = @(Get-ActiveSmokeTools $ToolTracker | Where-Object { $_.Name -like '*detach*' })
    if ($activeDetach.Count -gt 0) {
        $pids = ($activeDetach | ForEach-Object { $_.Process.Id }) -join ', '
        $Report.recorder_cleanup_error = "Skipped failure-detach because detach process ($pids) is still running."
        return
    }
    try {
        & $RunDetach
        & $WaitRecorder
    } catch {
        $Report.recorder_cleanup_error = $_.Exception.Message
    }
}

function Wait-SmokeTool {
    param(
        [Parameter(Mandatory)]$Run,
        [int]$Seconds = 240,
        [int]$TimeoutMs = -1,
        [Parameter(Mandatory)]$Report,
        [Parameter(Mandatory)][scriptblock]$WriteReport,
        [Parameter(Mandatory)][Text.Encoding]$Utf8
    )
    $waitMs = if ($TimeoutMs -ge 0) { $TimeoutMs } else { $Seconds * 1000 }
    if (-not $Run.Process.WaitForExit($waitMs)) {
        $partialOut = Read-SharedUtf8Text $Run.StdoutPath
        $partialErr = Read-SharedUtf8Text $Run.StderrPath
        [IO.File]::WriteAllText($Run.LogPath, $partialOut + "`n" + $partialErr, $Utf8)
        $Report.checks += [pscustomobject]@{
            name = $Run.Name
            exit = $null
            timed_out = $true
            pid = $Run.Process.Id
            seconds = ([DateTime]::UtcNow - $Run.Started).TotalSeconds
            command = $Run.Command
            stdout_log = $Run.StdoutPath
            stderr_log = $Run.StderrPath
            log = $Run.LogPath
        }
        & $WriteReport
        throw "Timed out: $($Run.Name); process $($Run.Process.Id) retained for safe cleanup. Logs: $($Run.StdoutPath), $($Run.StderrPath)"
    }
    $Run.Process.WaitForExit()
    $text = Read-SharedUtf8Text $Run.StdoutPath
    $errors = Read-SharedUtf8Text $Run.StderrPath
    [IO.File]::WriteAllText($Run.LogPath, $text + "`n" + $errors, $Utf8)
    $Report.checks += [pscustomobject]@{
        name = $Run.Name
        exit = $Run.Process.ExitCode
        seconds = ([DateTime]::UtcNow - $Run.Started).TotalSeconds
        command = $Run.Command
        stdout_log = $Run.StdoutPath
        stderr_log = $Run.StderrPath
        log = $Run.LogPath
    }
    & $WriteReport
    if ($Run.Process.ExitCode -ne 0) { throw "$($Run.Name) failed with exit $($Run.Process.ExitCode). See $($Run.Name).log" }
    return $text
}

function Set-ShimFlags($shim, [int]$flags) { $shim.View.Write($script:ShimFlagsOffset, [uint32]$flags) }

function Invoke-ShimPointer($shim, $window, [int]$logicalX, [int]$logicalY, [int]$virtualX, [int]$virtualY, [scriptblock]$press) {
    $view = $shim.View
    $view.Write($script:ShimWindowOffset, [uint64]$window.ToInt64())
    $view.Write($script:ShimCursorOffset, [int]$virtualX)
    $view.Write($script:ShimCursorOffset + 4, [int]$virtualY)
    Set-ShimFlags $shim ($script:ShimCursorFlag -bor $script:ShimFocusFlag)
    try {
        [Hoi4ProbeInput]::PostMove($window, $logicalX, $logicalY)
        Start-Sleep -Milliseconds 60
        & $press
    } finally {
        Set-ShimFlags $shim 0
    }
}

function Invoke-FullSmoke {
    if ($Checkpoint) {
        if ($BuildReceipt -or $BinaryDirectory) { throw 'Checkpoint cannot reuse a BuildReceipt or BinaryDirectory.' }
    } elseif (-not $BuildReceipt) {
        throw 'Smoke requires -BuildReceipt or an explicitly called -Checkpoint.'
    }
    if (-not $UiConfig -or -not $ExpectedSave -or -not $Out) {
        throw 'Smoke requires -UiConfig (calibrated screenshot pixels), -ExpectedSave and a new -Out directory.'
    }
    $captureRoot = [IO.Path]::GetFullPath($CaptureRoot).TrimEnd([char[]]'\/') + [IO.Path]::DirectorySeparatorChar
    $output = [IO.Path]::GetFullPath($Out)
    if (-not $output.StartsWith($captureRoot, [StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $output)) {
        throw 'Smoke output must be a NEW directory under CaptureRoot.'
    }
    $repo = Split-Path -Parent $PSScriptRoot
    $driver = $PSCommandPath
    $userPath = Resolve-LaunchUserDirectory $UserDirectory
    $gamePath = (Resolve-Path -LiteralPath $GameExe).ProviderPath
    if ($ExpectedSave.IndexOfAny([char[]]'\/:"') -ge 0 -or $ExpectedSave -notlike '*.hoi4') { throw 'ExpectedSave must be a local save filename.' }
    $sourceSave = Join-Path (Join-Path $userPath 'save games') $ExpectedSave
    $selectionPath = Join-Path $userPath 'continue_game.json'
    $settingsPath = Join-Path $userPath 'settings.txt'
    $settingsBytes = [IO.File]::ReadAllBytes($settingsPath)
    $settings = [IO.File]::ReadAllText($settingsPath)
    if ($settings -notmatch '(?m)^\s*save_as_binary\s*=\s*no\s*$') { throw 'Smoke requires save_as_binary=no; settings were not changed.' }
    $selection = if (Test-Path -LiteralPath $selectionPath) { ,[IO.File]::ReadAllBytes($selectionPath) } else { $null }
    if (-not (Test-Path -LiteralPath $sourceSave -PathType Leaf)) { throw 'Expected save is missing.' }
    $gameProcessName = [IO.Path]::GetFileNameWithoutExtension($gamePath)
    if (@(Get-Process -Name @($gameProcessName, 'recorder') -ErrorAction SilentlyContinue).Count) { throw 'Smoke will not take over another running game or recorder.' }
    $gameBuild = Write-GameBuildWarning $gamePath
    $ui = [IO.File]::ReadAllText((Resolve-Path -LiteralPath $UiConfig)) | ConvertFrom-Json
    foreach ($name in @('menu','speed_up','speed_down','pause','save_menu','save_name','save_confirm','save_back','menu_close','exit_game','exit_confirm','map_center','capture_cursor')) {
        $point = $ui.$name
        if ($point.Count -ne 2 -or $point[0] -lt 0 -or $point[1] -lt 0 -or $point[0] -ge $ui.width -or $point[1] -ge $ui.height) { throw "Invalid calibrated point: $name" }
    }
    $uiProbes = Resolve-SmokeUiProbes -Ui $ui -MenuPatch $MenuPatch -MenuPatchOrigin $MenuPatchOrigin
    $lock = [IO.Path]::GetFullPath($GameLock)
    $null = New-Item -ItemType Directory -Force -Path (Split-Path -Parent $lock)
    if (Test-Path -LiteralPath $lock) { throw "Shared game lock is held: $lock" }
    $null = New-Item -ItemType Directory -Path $lock -ErrorAction Stop
    $ownerToken = 'full-smoke-' + [Guid]::NewGuid().ToString()
    try {
        [IO.File]::WriteAllText((Join-Path $lock 'owner'), $ownerToken)
        $null = New-Item -ItemType Directory -Path $output
        $utf8 = New-Object Text.UTF8Encoding $false
        $recordRoot = Join-Path $output 'recording'
        $bin = Join-Path $output 'bin'
        $null = New-Item -ItemType Directory -Path $bin
        $started = [Diagnostics.Stopwatch]::StartNew()
        $report = [ordered]@{ passed=$false; phase='preflight'; checks=@(); source_save=$sourceSave; source_save_sha256=(Get-FileHash $sourceSave -Algorithm SHA256).Hash; started_utc=[DateTime]::UtcNow.ToString('o'); full_smoke=$true
            game_build=[ordered]@{ version=$gameBuild.Version; bytes=$gameBuild.Size; sha256=$gameBuild.Sha256; recognised=$gameBuild.Recognised; warning=$gameBuild.Warning } }
    } catch {
        Remove-Item -LiteralPath $lock -Recurse -Force -ErrorAction SilentlyContinue
        throw
    }
    $game = $null; $reader = $null; $recorderRun = $null; $foregroundState = @{ Handle = [IntPtr]::Zero }; $exitConfirmationState = @{ Sent = $false }
    $toolTracker = [Collections.Generic.List[object]]::new()
    $menuProbe = $uiProbes.Menu
    $saveDialogProbe = $uiProbes.SaveDialog
    $exitConfirmProbe = $uiProbes.ExitConfirm
    $statusPath = $null; $failure = $null; $cleaned = $false
    $selectionWritten = $false
    function Write-Report { [IO.File]::WriteAllText((Join-Path $output 'smoke-result.json'), ($report | ConvertTo-Json -Depth 14), $utf8) }
    function Stage([string]$name) { $report.phase=$name; Write-Report; Write-Host ("SMOKE {0}: {1:n1}s" -f $name,$started.Elapsed.TotalSeconds) }
    function Launch-Tool([string]$exe, [string[]]$arguments, [string]$name) {
        Start-SmokeTool -Exe $exe -Arguments $arguments -Name $name -WorkingDirectory $repo -OutputDir $output -Tracker $toolTracker
    }
    function Finish-Tool($run,[int]$seconds=240) {
        Wait-SmokeTool -Run $run -Seconds $seconds -Report $report -WriteReport { Write-Report } -Utf8 $utf8
    }
    function Run-Tool([string]$exe,[string[]]$arguments,[string]$name,[int]$seconds=240) {
        Finish-Tool (Launch-Tool $exe $arguments $name) $seconds
    }
    function Confirm-Source([string]$name,[string]$expected='') {
        $head=(Run-Tool $git @('rev-parse','HEAD') ($name+'-commit') 30).Trim()
        $dirty=Run-Tool $git @('status','--porcelain','--untracked-files=no') ($name+'-status') 30
        if ($head -notmatch '^[0-9a-f]{40}$' -or -not [string]::IsNullOrWhiteSpace($dirty)) {
            throw 'Smoke requires an exact clean committed source HEAD.'
        }
        if ($expected -and $head -cne $expected) { throw 'Source HEAD changed during artifact preparation.' }
        $head
    }
    function Read-Binaries([string]$directory,[object[]]$expected=@()) {
        $binaries=@()
        foreach ($name in @('recorder.exe','collector.dll','viewer.exe')) {
            $path=Join-Path $directory $name
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing smoke binary: $name" }
            $item=Get-Item -LiteralPath $path
            $hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
            if ($expected.Count) {
                $entries=@($expected | Where-Object { $_.PSObject.Properties['name'] -and $_.name -is [string] -and $_.name -ceq $name })
                if ($entries.Count -ne 1) { throw "Receipt must identify $name exactly once." }
                $entry=$entries[0]
                if (-not $entry.PSObject.Properties['sha256'] -or $entry.sha256 -isnot [string] -or
                    $entry.sha256 -notmatch '^[0-9a-fA-F]{64}$' -or -not $entry.PSObject.Properties['bytes'] -or
                    ($entry.bytes -isnot [int] -and $entry.bytes -isnot [long]) -or
                    $entry.bytes -ne $item.Length -or $entry.sha256 -ine $hash) {
                    throw "Receipt does not match the actual smoke binary: $name"
                }
            }
            if ($item.Length -le 0) { throw "Empty smoke binary: $name" }
            $binaries += [pscustomobject]@{name=$name;sha256=$hash;bytes=$item.Length}
        }
        $binaries
    }
    function Drive([string]$action,[hashtable]$extra=@{},[bool]$foregroundInput=$Foreground,[scriptblock]$onRetry={ Start-Sleep -Milliseconds 250 }) {
        $driverAction = $action
        Invoke-WithFocusRetry -OnRetry $onRetry -Operation {
            & $driver -CaptureRoot $CaptureRoot -InputShimDir $InputShimDir -Action $driverAction -GameId $game.Id -GameExe $gamePath -UserDirectory $userPath -Foreground:$foregroundInput -KeepForeground:($foregroundInput -and -not $Foreground) @extra
        }
    }
    function Click-Point([string]$name) {
        $point=$ui.$name
        $foregroundInput=$Foreground -or $name -in @('menu','save_menu','save_name','save_confirm','exit_game','exit_confirm')
        $null=Drive 'Click' @{X=[int]$point[0];Y=[int]$point[1]} $foregroundInput
    }
    function Clock {
        $limit=[DateTime]::UtcNow.AddSeconds(10)
        do {
            $game.Refresh(); if ($game.HasExited) { throw 'Game exited during smoke.' }
            $clock=$reader.Sample()
            if ($null -ne $clock -and $null -ne $clock.paused -and $clock.tick_busy -eq 0) { return $clock }
            Start-Sleep -Milliseconds 10
        } while ([DateTime]::UtcNow -lt $limit)
        throw 'No stable, authenticated game clock.'
    }
    function Pause-Game([bool]$paused) {
        if ((Clock).paused -ne [int]$paused) { $null=Drive 'Key' @{Key='Space'} }
        $limit=[DateTime]::UtcNow.AddSeconds(10)
        while ((Clock).paused -ne [int]$paused) {
            if ([DateTime]::UtcNow -gt $limit) { throw 'Pause input was not accepted.' }
            Start-Sleep -Milliseconds 20
        }
    }
    function Speed-Game([int]$speed) {
        for ($tries=0; $tries -lt 10; $tries++) {
            $actual=(Clock).speed
            if ($actual -eq $speed) { return }
            if ($actual -lt $speed) { Click-Point 'speed_up' } else { Click-Point 'speed_down' }
        }
        throw 'Speed input was not accepted.'
    }
    function Status {
        if (-not $script:smokeStatusPath) {
            $found=@(Get-ChildItem -LiteralPath $recordRoot -Filter status.json -Recurse -ErrorAction SilentlyContinue)
            if ($found.Count -eq 1) { $script:smokeStatusPath=$found[0].FullName }
        }
        $path=$script:smokeStatusPath
        if (-not $path) { return $null }
        try {
            $stream=[IO.FileStream]::new($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
            try { $text=[IO.StreamReader]::new($stream,[Text.Encoding]::UTF8).ReadToEnd() } finally { $stream.Dispose() }
            $text | ConvertFrom-Json
        } catch { return $null }
    }
    function Fresh-Status([DateTime]$since,[string]$what,[int]$seconds=30) {
        $limit=[DateTime]::UtcNow.AddSeconds($seconds)
        do {
            if (-not $script:smokeStatusPath) { $null=Status }
            $item=if ($script:smokeStatusPath) { Get-Item -LiteralPath $script:smokeStatusPath -ErrorAction SilentlyContinue } else { $null }
            if ($item -and $item.LastWriteTimeUtc -gt $since) {
                $status=Status
                if ($status) {
                    if ($null -eq $status.capture -or $null -eq $status.capture.held -or $null -eq $status.capture.beside -or $null -eq $status.last_hour -or $null -eq $status.hours) { throw "Status after $what lacks the capture counters." }
                    return $status
                }
            }
            if ([DateTime]::UtcNow -gt $limit) { throw "No readable status was written after $what." }
            Start-Sleep -Milliseconds 100
        } while ($true)
    }
    function Changed-Pixels([string]$first,[string]$second,[int[]]$box) {
        Add-Type -AssemblyName System.Drawing
        $a=[Drawing.Bitmap]::new($first); $b=[Drawing.Bitmap]::new($second)
        try {
            $count=0
            for ($y=$box[1]; $y -lt $box[3]; $y++) { for ($x=$box[0]; $x -lt $box[2]; $x++) {
                $p=$a.GetPixel($x,$y); $q=$b.GetPixel($x,$y)
                if ([Math]::Abs($p.R-$q.R) -gt 8 -or [Math]::Abs($p.G-$q.G) -gt 8 -or [Math]::Abs($p.B-$q.B) -gt 8) { $count++ }
            } }
            $count
        } finally { $a.Dispose(); $b.Dispose() }
    }
    function Park-Cursor([int]$index) {
        $point=$ui.capture_cursor
        $box=@([Math]::Max(0,[int]$point[0]-200),0,[Math]::Min([int]$ui.width,[int]$point[0]+200),([int]$point[1]+7))
        $shots=@(0,1,2 | ForEach-Object { Join-Path $output ("park-{0}-{1}.png" -f $index,$_) })
        $state=Clock
        $null=Drive 'Screenshot' @{Out=$shots[0]}
        for ($click=1; $click -le 2; $click++) {
            Click-Point 'capture_cursor'; Start-Sleep -Milliseconds 800
            $null=Drive 'Screenshot' @{Out=$shots[$click]}
        }
        $after=Clock
        $changed=(Changed-Pixels $shots[0] $shots[1] $box) + (Changed-Pixels $shots[1] $shots[2] $box)
        $held=$after.hour -eq $state.hour -and $after.paused -eq 1 -and $after.speed -eq $state.speed -and
            (($after.eye -join ',') -eq ($state.eye -join ',')) -and (($after.look_at -join ',') -eq ($state.look_at -join ','))
        if (-not $held -or $changed -ne 0) { throw "capture_cursor is not inert: $changed strip pixels changed, state held=$held; see park-$index-*.png" }
        Remove-Item -LiteralPath $shots
        $report.cursor_parks += @{index=$index;strip=$box;strip_changed=$changed}
    }
    function Test-SmokeDialog([object]$probe) {
        $probeShot = Join-Path $output ('ui-probe-' + [Guid]::NewGuid().ToString('N') + '.png')
        try {
            $null = Drive 'Screenshot' @{Out=$probeShot}
            return Test-UiDialogState -ShotPath $probeShot -Probe $probe
        } finally {
            Remove-Item -LiteralPath $probeShot -Force -ErrorAction SilentlyContinue
        }
    }
    function Test-CompleteTextSave([string]$path) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $false }
        try {
            $stream = [IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::None)
            try {
                $header = New-Object byte[] 8
                $n = $stream.Read($header,0,8)
                return $n -eq 8 -and [Text.Encoding]::ASCII.GetString($header).StartsWith('HOI4txt') -and $stream.Length -gt 10000
            } finally { $stream.Dispose() }
        } catch { return $false }
    }
    function Stop-OwnedGame {
        $result = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 20 `
            -HasExited { $game.Refresh(); $game.HasExited } `
            -GetExitCode { $game.Refresh(); $game.ExitCode } `
            -WaitForExit { param($waitMs) $game.WaitForExit($waitMs) } `
            -PauseGame { Pause-Game $true } `
            -Click { param($pointName) Click-Point $pointName } `
            -SendEscape { $null = Drive 'Key' @{Key='Escape'} $true } `
            -IsMenuOpen { Test-SmokeDialog $menuProbe } `
            -IsConfirmOpen { Test-SmokeDialog $exitConfirmProbe } `
            -OnConfirmationSent { $exitConfirmationState.Sent = $true }
        $report.exit_attempts = $result.Attempts
        $report.exit_confirmed = $result.ConfirmationSent
        $report.exit_code = $result.ExitCode
    }
    try {
        Stage 'static-checks'
        $git=(Get-Command git -ErrorAction Stop).Source
        $report.source_commit=Confirm-Source 'source'
        $report.source_clean=$true
        $formatMatch=[regex]::Match([IO.File]::ReadAllText((Join-Path $repo 'crates\replay-data\src\lib.rs')),'FORMAT_VERSION: u16 = (?:(\d+)|rows::compatibility::(\w+));')
        $formatText=$formatMatch.Groups[1].Value
        if ($formatMatch.Groups[2].Success) {
            $formatName=[regex]::Escape($formatMatch.Groups[2].Value)
            $formatCatalog=[IO.File]::ReadAllText((Join-Path $repo 'crates\replay-data\src\rows\compatibility.rs'))
            $formatText=[regex]::Match($formatCatalog,"const ${formatName}: u16 = (\d+);").Groups[1].Value
        }
        $format=[int]$formatText
        if ($format -lt 1) { throw 'FORMAT_VERSION not found in crates/replay-data/src/lib.rs.' }
        $binarySource=Join-Path $repo '.cargo-target\release'
        $expectedBinaries=@()
        if ($Checkpoint) {
            Stage 'checkpoint'
            if ($env:CARGO_BUILD_TARGET) { throw 'CARGO_BUILD_TARGET would change the checkpoint release path; use the native target.' }
            $cargo=(Get-Command cargo -ErrorAction Stop).Source
            $null=Run-Tool $cargo @('fmt','--all','--','--check') 'format' 2400
            $null=Run-Tool $cargo @('nextest','run','--workspace') 'checkpoint' 2400
            $summary=[regex]::Match([IO.File]::ReadAllText((Join-Path $output 'checkpoint.log')),'\d+ tests run: (\d+) passed')
            $report.tests_passed=[int]$summary.Groups[1].Value
            $null=Run-Tool $cargo @('build','-p','recorder','-p','collector','-p','viewer','--release','--target-dir','.cargo-target') 'release' 600
        } else {
            $receiptPath=(Resolve-Path -LiteralPath $BuildReceipt -ErrorAction Stop).ProviderPath
            if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) { throw 'BuildReceipt must identify an existing JSON file.' }
            $receipt=[IO.File]::ReadAllText($receiptPath) | ConvertFrom-Json
            if (-not $receipt.PSObject.Properties['source_commit'] -or $receipt.source_commit -isnot [string] -or
                $receipt.source_commit -cne $report.source_commit) {
                throw 'BuildReceipt source_commit does not match the exact current HEAD.'
            }
            if (-not $receipt.PSObject.Properties['binaries'] -or @($receipt.binaries).Count -ne 3) {
                throw 'BuildReceipt must contain exactly the three smoke binaries.'
            }
            $expectedBinaries=@($receipt.binaries)
            if ($BinaryDirectory) { $binarySource=(Resolve-Path -LiteralPath $BinaryDirectory -ErrorAction Stop).ProviderPath }
            $report.build_receipt=$receiptPath
        }
        $null=Confirm-Source 'pre-copy' $report.source_commit
        $sourceBinaries=@(Read-Binaries $binarySource $expectedBinaries)
        foreach ($file in $sourceBinaries) { Copy-Item -LiteralPath (Join-Path $binarySource $file.name) -Destination (Join-Path $bin $file.name) }
        $report.binaries=@(Read-Binaries $bin $sourceBinaries)
        $compiler=Copy-ShaderCompiler $binarySource $bin
        $pinned=[regex]::Match([IO.File]::ReadAllText((Join-Path $repo 'crates\shader-pipeline\src\lib.rs')),'pub const DXC_VERSION: &str = "([^"]+)";').Groups[1].Value
        if (-not $pinned) { throw 'DXC_VERSION not found in crates/shader-pipeline/src/lib.rs.' }
        if ((Get-Item -LiteralPath $compiler).VersionInfo.ProductVersion -cne $pinned) { throw "The shader compiler is not the pinned DXC $pinned." }
        $report.shader_compiler=@{path=$compiler;sha256=(Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash}
        if ($null -ne $selection) { [IO.File]::WriteAllBytes((Join-Path $output 'continue-before.json'),$selection) }
        [IO.File]::WriteAllBytes((Join-Path $output 'settings-before.txt'),$settingsBytes)
        Copy-Item -LiteralPath $UiConfig -Destination (Join-Path $output 'ui.json')
        Copy-Item -LiteralPath $driver -Destination (Join-Path $output 'smoke-driver.ps1')
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'game-clock.ps1') -Destination (Join-Path $output 'game-clock.ps1')
        $null=Confirm-Source 'pre-launch' $report.source_commit
        [IO.File]::WriteAllText((Join-Path $output 'build-receipt.json'),
            (@{source_commit=$report.source_commit;binaries=$report.binaries} | ConvertTo-Json -Depth 8),$utf8)
        $report.map_mode='controller'; $report.perspective=$Perspective
        $recorder=Join-Path $bin 'recorder.exe'; $viewer=Join-Path $bin 'viewer.exe'
        Write-LaunchSelection $userPath $ExpectedSave
        $selectionWritten = $true
        Stage 'launch-and-attach'
        $script:smokeStatusPath=$null
        $recorderRun=Launch-Tool $recorder @('record','--once','--out',$recordRoot,'--user-dir',$userPath) 'record'
        $launch=& $driver -CaptureRoot $CaptureRoot -InputShimDir $InputShimDir -BinaryDirectory $bin -Action Launch -GameExe $gamePath -UserDirectory $userPath -ContinueLastSave -ExpectedSave $ExpectedSave -Foreground:$Foreground
        $game=Get-Process -Id $launch.GameId
        $null=$game.Handle
        $limit=[DateTime]::UtcNow.AddSeconds(300)
        do {
            $game.Refresh(); if ($game.HasExited) { throw 'Game exited before loading.' }
            if ($null -eq $reader) {
                try { $reader=Open-GameClock $game.Id $recorder }
                catch { if ($_.Exception.Message -like 'Clock reading is withheld*') { throw } }
                if ($reader) { foreach ($withheld in Get-ClockWithheld $reader) { Write-Warning "Game clock field withheld: $withheld" } }
            }
            $clock=if ($reader) { $reader.Sample() } else { $null }
            if ($clock -and $clock.hour -eq $ui.expected_hour -and $clock.seed -eq $ui.expected_seed -and $null -ne $clock.paused -and $game.MainWindowHandle -ne [IntPtr]::Zero) { break }
            if ([DateTime]::UtcNow -gt $limit) { throw 'Expected save did not finish loading.' }
            Start-Sleep -Milliseconds 100
        } while ($true)
        $shot=Drive 'Screenshot' @{Out=(Join-Path $output 'loaded.png')}
        if ($shot.Width -ne $ui.width -or $shot.Height -ne $ui.height) { throw 'Window dimensions differ from calibrated UI.' }
        if ($Foreground) { $foregroundState.Handle=Invoke-WithFocusRetry -Operation { [Hoi4ProbeInput]::AcquireTargetFocus($game.MainWindowHandle,[uint32]$game.Id) } }
        Pause-Game $true
        $null=Drive 'Move' @{X=[int]$ui.map_center[0];Y=[int]$ui.map_center[1]}
        $limit=[DateTime]::UtcNow.AddSeconds(120)
        while (@($game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 1) {
            if ($recorderRun.Process.HasExited) { $null=Finish-Tool $recorderRun; throw 'Recorder exited before attaching.' }
            if ([DateTime]::UtcNow -gt $limit) { throw 'Collector did not attach.' }
            Start-Sleep -Milliseconds 200; $game.Refresh()
        }
        $clock=Clock; $report.initial_clock=$clock; $report.game_pid=$game.Id
        $first=[int]$clock.hour; $target=$first + $RecordHours
        $load=$clock.play_session_started_at; $seed=$clock.seed
        $live=[Diagnostics.Stopwatch]::StartNew(); $progressAt=0.0
        Stage 'recording'
        Speed-Game 4; Pause-Game $false
        while ($true) {
            $clock=Clock
            if ($clock.play_session_started_at -ne $load -or $clock.seed -ne $seed -or $clock.hour -lt $first) { throw 'Unexpected load or campaign change.' }
            if ($live.Elapsed.TotalSeconds -gt $TimeoutSeconds) { throw 'Live recording timed out.' }
            if ($clock.hour -ge $target) {
                Pause-Game $true; $target=[int](Clock).hour
                break
            }
            $status=Status
            if ($status) {
                if ($status.sample_boundary -ne 'after_hourly_update') { throw 'Unexpected sample boundary.' }
                if ($status.hours -gt 0 -and $status.last_hour-$status.hours+1 -ne $first) { throw 'Unexpected first recorded hour or discontinuity.' }
                if ($status.dropped_frames -ne 0 -or $status.write_errors -ne 0) { throw 'Recording reported drops or write errors.' }
                if ($live.Elapsed.TotalSeconds -ge $progressAt) {
                    Write-Host "H=$($clock.hour) target=$target samples=$($status.hours)"
                    $progressAt=$live.Elapsed.TotalSeconds+15
                    $report.live_progress=@{hour=$clock.hour;target=$target;samples=$status.hours}; Write-Report
                }
            }
            Start-Sleep -Milliseconds 20
        }
        $live.Stop(); $report.advance_seconds=$live.Elapsed.TotalSeconds; $advanced=$target-$first
        Stage 'step-to-saved-hour'
        $paused=$target; $since=[DateTime]::UtcNow; $limit=$since.AddSeconds(30)
        do { $status=Fresh-Status $since 'the pause'; if ($status.last_hour -ge $paused) { break }; if ([DateTime]::UtcNow -gt $limit) { throw 'The paused hour was not posted by the collector.' }; Start-Sleep -Milliseconds 100 } while ($true)
        $heldBefore=[long]$status.capture.held; $besideBefore=[long]$status.capture.beside
        if (-not $AllowHeldCaptures -and ($status.capture.placement -ne 'beside the game' -or -not ([string]$status.capture.batch_publication).StartsWith('proven'))) { throw ("Captures are held instead of beside the game: placement '{0}', batch_publication '{1}'. Pass -AllowHeldCaptures if that is expected." -f $status.capture.placement, $status.capture.batch_publication) }
        Speed-Game 1
        $null=Run-Tool $recorder @('refresh','--out',$recordRoot) 'refresh' 60
        Pause-Game $false
        $limit=[DateTime]::UtcNow.AddSeconds(30)
        while ((Clock).hour -le $paused) { if ([DateTime]::UtcNow -gt $limit) { throw 'The step to the saved hour did not advance.' }; Start-Sleep -Milliseconds 5 }
        Pause-Game $true
        $target=[int](Clock).hour
        if ($target -ne $paused+1) { throw 'The step to the saved hour overshot one hour.' }
        $report.save_hour=$target
        $status=Status
        [IO.File]::WriteAllText((Join-Path $output 'before-save-status.json'),($status | ConvertTo-Json -Depth 30),$utf8)
        if ($WaitForUiReview) {
            Stage 'awaiting-ui-review'
            $reviewShot=Join-Path $output 'before-save-ui-review.png'
            $null=Drive 'Screenshot' @{Out=$reviewShot}
            [IO.File]::WriteAllText((Join-Path $output 'ui-review-required.json'), (@{hour=$target;game_pid=$game.Id;screenshot=$reviewShot;requirement='Dismiss informational notifications through guarded driver actions, leave the map unobscured and paused; then write ui-review-approved.json with this hour.'} | ConvertTo-Json),$utf8)
            $ack=Join-Path $output 'ui-review-approved.json'; $limit=[DateTime]::UtcNow.AddSeconds(600)
            while (-not (Test-Path -LiteralPath $ack)) {
                if ((Clock).hour -ne $target -or (Clock).paused -ne 1) { throw 'Game advanced or unpaused during UI review.' }
                if ([DateTime]::UtcNow -gt $limit) { throw 'Unobscured-map review was not completed.' }
                Start-Sleep -Milliseconds 200
            }
            $approval=[IO.File]::ReadAllText($ack) | ConvertFrom-Json
            if ($approval.hour -ne $target -or $approval.unobscured -ne $true) { throw 'Invalid UI review acknowledgement.' }
            $report.ui_review=$approval
            $null=Drive 'Screenshot' @{Out=(Join-Path $output 'before-save-map.png')}
        }
        Stage 'save-through-game-ui'
        Start-Sleep -Seconds 2
        $before=Clock; if ($before.hour -ne $target -or $before.paused -ne 1) { throw 'Pause did not hold before save.' }
        $saveName='SMOKE_'+[DateTime]::Now.ToString('yyyyMMdd_HHmmss')+'_H'+$target
        $save=Join-Path (Join-Path $userPath 'save games') ($saveName+'.hoi4')
        if (Test-Path -LiteralPath $save) { throw 'Smoke save already exists; refusing to overwrite.' }
        $saveResult = Invoke-UiSaveFlow -MaxAttempts 3 -MaxPolls 20 `
            -IsSaveComplete { Test-CompleteTextSave $save } `
            -CheckClock {
                $nowClock = Clock
                if ($nowClock.hour -ne $target) { throw 'Game advanced during the save.' }
                if ($nowClock.paused -ne 1) { throw 'Pause did not hold before save.' }
            } `
            -PauseGame { Pause-Game $true } `
            -Click { param($pointName) Click-Point $pointName } `
            -TypeSaveName {
                $null = Drive 'Key' @{Key='CtrlA'} $true
                $null = Drive 'Text' @{Text=$saveName} $true
            } `
            -SendEscape { $null = Drive 'Key' @{Key='Escape'} $true } `
            -IsMenuOpen { Test-SmokeDialog $menuProbe } `
            -IsSaveDialogOpen { Test-SmokeDialog $saveDialogProbe } `
            -WaitForSave {
                $appearLimit = [DateTime]::UtcNow.AddSeconds(40)
                $completeLimit = [DateTime]::UtcNow.AddSeconds(120)
                do {
                    if (Test-CompleteTextSave $save) { return [pscustomobject]@{ Done=$true; Reason=$null } }
                    $writing = Test-Path -LiteralPath $save
                    $now = [DateTime]::UtcNow
                    if (-not $writing -and $now -gt $appearLimit) {
                        return [pscustomobject]@{ Done=$false; Reason='save file did not appear after save_confirm' }
                    }
                    if ($now -gt $completeLimit) {
                        return [pscustomobject]@{ Done=$false; Reason='save file did not finish writing in time' }
                    }
                    Start-Sleep -Milliseconds 250
                } while ($true)
            }
        $report.save_attempts = $saveResult.Attempts
        Start-Sleep -Seconds 2
        if ((Clock).hour -ne $target) { throw 'Game advanced during the save.' }
        $report.save=$save; $report.save_sha256=(Get-FileHash $save -Algorithm SHA256).Hash
        $null=Drive 'Screenshot' @{Out=(Join-Path $output 'after-save-ui.png')}
        $null=Drive 'Move' @{X=[int]$ui.map_center[0];Y=[int]$ui.map_center[1]}
        Stage 'same-hour-reference-captures'
        $references=@(); $steps=@(0,-3,6,6,-9); $report.cursor_parks=@()
        for ($i=0; $i -lt $steps.Count; $i++) {
            if ($steps[$i] -ne 0) { $null=Drive 'Wheel' @{X=[int]$ui.map_center[0];Y=[int]$ui.map_center[1];Delta=$steps[$i]} }
            Start-Sleep -Seconds 2
            Park-Cursor $i
            $camera=Clock
            if ($camera.hour -ne $target -or $camera.paused -ne 1 -or -not $camera.eye) { throw 'Reference capture lacks an authenticated paused camera.' }
            $image=Join-Path $output ("reference-{0}-game.png" -f $i)
            $null=Drive 'Screenshot' @{Out=$image}
            $references += @{index=$i;game_image=$image;clock=$camera;camera=@($camera.look_at[0],$camera.look_at[2],$camera.eye[1]);width=$camera.viewport[2];height=$camera.viewport[3]}
        }
        $report.references=$references
        $statusBeforeStep=Status
        $report.paused_observation=@{hour_before=$before.hour;hour_after=(Clock).hour;samples_before=$status.hours;samples_after=$statusBeforeStep.hours;scope='Observed save/camera interval only; not a universal paused-command proof.'}
        Stage 'end-of-hour-boundary'
        $status=Fresh-Status ([DateTime]::UtcNow) 'the reference captures'
        if ([long]$status.capture.held -ne $heldBefore -or [long]$status.capture.beside -ne $besideBefore -or $status.last_hour -ge $target) { throw 'A capture ran while paused: the saved hour may have been sampled before the save.' }
        Pause-Game $false
        $limit=[DateTime]::UtcNow.AddSeconds(30)
        while ((Clock).hour -lt $target+1) { if ([DateTime]::UtcNow -gt $limit) { throw 'The H+1 sampling step did not advance.' }; Start-Sleep -Milliseconds 5 }
        Pause-Game $true
        $report.after_sampling_clock=Clock; $reached=[int]$report.after_sampling_clock.hour
        $since=[DateTime]::UtcNow; $limit=$since.AddSeconds(30)
        do { $status=Fresh-Status $since 'the sampling step'; if ($status.last_hour -ge $reached) { break }; if ([DateTime]::UtcNow -gt $limit) { throw 'Saved hour was not posted by the collector.' }; Start-Sleep -Milliseconds 100 } while ($true)
        $heldGrowth=[long]$status.capture.held-$heldBefore; $besideGrowth=[long]$status.capture.beside-$besideBefore
        $report.saved_hour_capture=@{held=$heldGrowth;beside=$besideGrowth;reached=$reached;placement=$status.capture.placement}
        if ($heldGrowth -lt 1 -or $heldGrowth+$besideGrowth -ne 1+$reached-$target -or ($status.capture.placement -eq 'beside the game' -and $heldGrowth -ne 1)) { throw 'The saved hour was not read once, holding the game, after the save.' }
        [IO.File]::WriteAllText((Join-Path $output 'boundary-status.json'),($status | ConvertTo-Json -Depth 30),$utf8)
        Stage 'detach-drain-finalize'
        $finalize=[Diagnostics.Stopwatch]::StartNew()
        $null=Run-Tool $recorder @('detach','--out',$recordRoot) 'detach' 120
        $null=Finish-Tool $recorderRun 180
        $finalize.Stop(); $report.finalize_seconds=$finalize.Elapsed.TotalSeconds
        $game.Refresh()
        if (@($game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) { throw 'Collector remains loaded after detach.' }
        $completionFiles=@(Get-ChildItem -LiteralPath $recordRoot -Filter completed.json -Recurse)
        if ($completionFiles.Count -ne 1) { throw 'Expected one completed recording session.' }
        $completed=[IO.File]::ReadAllText($completionFiles[0].FullName) | ConvertFrom-Json
        if (-not $completed.detached -or -not $completed.drain_complete -or $completed.partial -or $completed.issues.Count -ne 0 -or $completed.warnings.Count -ne 0) { throw 'Recording did not finalize cleanly.' }
        $report.completion=$completed; $report.collector_absence_confirmed=$true
        Invoke-ExitThenVerify -Report $report -ExitGame {
            Stage 'normal-game-exit'
            try { Stop-OwnedGame }
            finally {
                if ($foregroundState.Handle -ne [IntPtr]::Zero -and ('Hoi4ProbeInput' -as [type])) {
                    try { [Hoi4ProbeInput]::RestoreForeground($foregroundState.Handle,[IntPtr]::Zero); $foregroundState.Handle=[IntPtr]::Zero }
                    catch { $report.foreground_cleanup_error=$_.Exception.Message }
                }
            }
        } -Verify {
            $replays=@(Get-ChildItem -LiteralPath $recordRoot -File | Where-Object { $_.Extension -in '.battleplan' })
            if ($replays.Count -ne 1) { throw 'Expected exactly one finalized replay.' }
            $replay=$replays[0].FullName; $report.replay=$replay; $report.replay_bytes=$replays[0].Length
            $stream=[IO.File]::OpenRead($replay)
            try { $header=New-Object byte[] 6; $n=$stream.Read($header,0,6); if ($n -ne 6 -or [Text.Encoding]::ASCII.GetString($header,0,4) -cne 'H4RP' -or [BitConverter]::ToUInt16($header,4) -ne $format) { throw "Smoke did not produce the expected format-$format replay." } } finally { $stream.Dispose() }
            Stage 'fresh-save-oracle'
            $inspection=Run-Tool $recorder @('inspect',$replay) 'inspect' 180
            if ($inspection -notmatch 'content_verified=true sessions=1 branches=1 blocks=\d+') { throw 'Replay identity/session/branch inspection failed.' }
            $oracle=Run-Tool $recorder @('oracle',$replay,$save,'--hour',[string]$target,'--branch','1','--game-dir',(Split-Path -Parent $gamePath),'--user-dir',$userPath) 'oracle' 240
            if ($oracle -notmatch '(?m)^Differences: 0\s*$' -or $oracle -notmatch 'coverage: complete; 0 table availability issues; 0 failed or skipped comparison groups') { throw 'Fresh-save oracle did not establish complete matching coverage.' }
            $report.oracle_comparisons=[int]([regex]::Match($oracle,'(\d+) exact field comparisons').Groups[1].Value)
            $report.oracle_differences=0; $report.recorded_format=$format
            Copy-Item -LiteralPath $save -Destination (Join-Path $output 'boundary-save.hoi4')
            Stage 'render-and-score-five-reference-pairs'
            $culture=[Globalization.CultureInfo]::InvariantCulture
            foreach ($reference in $references) {
                $cameraText=($reference.camera | ForEach-Object { ([double]$_).ToString('R',$culture) }) -join ','
                $common=@($replay,'--hour',[string]$target,'--branch','1','--camera',$cameraText,'--width',[string]$reference.width,'--height',[string]$reference.height,'--perspective',$Perspective,'--fog','on','--game-dir',(Split-Path -Parent $gamePath),'--user-dir',$userPath)
                $image=Join-Path $output ("reference-{0}-viewer.png" -f $reference.index)
                $null=Run-Tool $viewer (@('--dump')+$common+@('--out',$image)) ("render-{0}" -f $reference.index) 240
                $score=Join-Path $output ("reference-{0}-score.json" -f $reference.index)
                $scale=([double]$ui.width/$reference.width).ToString('R',$culture)
                $null=Run-Tool $viewer @('--compare',$reference.game_image,$image,'--upscale',$scale,'--exclude-game-ui','--json',$score) ("compare-{0}" -f $reference.index) 120
                $reference.viewer_image=$image; $reference.scores=[IO.File]::ReadAllText($score) | ConvertFrom-Json
                if ($reference.index -eq 0) {
                    $windowImage=Join-Path $output 'window-parity.png'
                    Add-Type -AssemblyName System.Windows.Forms
                    $screen=[Windows.Forms.SystemInformation]::VirtualScreen
                    [Windows.Forms.Cursor]::Position=New-Object Drawing.Point ($screen.Right-1),($screen.Bottom-1)
                    $null=Run-Tool $viewer (@('--window-dump')+$common+@('--out',$windowImage)) 'window-render' 240
                    $headlessHash=(Get-FileHash $image -Algorithm SHA256).Hash
                    $windowHash=(Get-FileHash $windowImage -Algorithm SHA256).Hash
                    $report.window_headless_identical=$headlessHash -eq $windowHash
                    $report.window_headless_sha256=$headlessHash
                    if (-not $report.window_headless_identical) { throw 'Headless and window render outputs differ.' }
                }
                Write-Report
            }
            $finalStatus=Status
            [IO.File]::WriteAllText((Join-Path $output 'final-status.json'),($finalStatus | ConvertTo-Json -Depth 30),$utf8)
            if ($finalStatus.hours -lt $RecordHours -or $finalStatus.dropped_frames -ne 0 -or $finalStatus.write_errors -ne 0) { throw 'Final transport counters failed.' }
            $report.performance=@{recorded_hours=$finalStatus.hours;advancing_game_hours=$advanced;advance_seconds=$live.Elapsed.TotalSeconds;advancing_hours_per_second=$advanced/$live.Elapsed.TotalSeconds;callback_p50_ms=$finalStatus.callback_work.p50_ns/1e6;callback_p95_ms=$finalStatus.callback_work.p95_ns/1e6;callback_max_ms=$finalStatus.callback_work.max_ns/1e6;pool_bytes=$finalStatus.pool_bytes;spool_bytes=$finalStatus.spool_bytes;scope='One live smoke at top speed. Not a paired baseline or performance acceptance.'}
            if ((Get-FileHash $sourceSave -Algorithm SHA256).Hash -cne $report.source_save_sha256) { throw 'Original input save changed.' }
            $report.source_save_unchanged=$true
        }
        $report.passed=$true; Stage 'validations-passed'
    } catch {
        $failure=$_; $report.passed=$false; $report.error=$_.Exception.Message
        Stage 'failed'
        Invoke-FailureGamePause -Game $game -Reader $reader -ExitConfirmationSent $exitConfirmationState.Sent -Report $report `
            -SendEscape { $null = Drive 'Key' @{Key='Escape'} } `
            -PauseGame { Pause-Game $true }
        Invoke-FailureRecorderCleanup -RecorderRun $recorderRun -ToolTracker $toolTracker -Report $report `
            -RunDetach { $null = Run-Tool $recorder @('detach','--out',$recordRoot) 'failure-detach' 120 } `
            -WaitRecorder { $null = Finish-Tool $recorderRun 180 }
    } finally {
        if ($reader) { $reader.Dispose() }
        if ($game) {
            $game.Refresh()
            if ($foregroundState.Handle -ne [IntPtr]::Zero -and ('Hoi4ProbeInput' -as [type])) {
                try { [Hoi4ProbeInput]::RestoreForeground($foregroundState.Handle,[IntPtr]::Zero) }
                catch { $report.foreground_cleanup_error=$_.Exception.Message }
            }
        }
        $ownedGameStopped=$null -eq $game -or $game.HasExited
        $ownedRecorderStopped=$null -eq $recorderRun -or $recorderRun.Process.HasExited
        $activeTools=@(if (Get-Variable -Name toolTracker -ErrorAction SilentlyContinue) { Get-ActiveSmokeTools $toolTracker })
        $ownedToolsStopped=$activeTools.Count -eq 0
        if ($ownedGameStopped -and $ownedRecorderStopped -and $ownedToolsStopped) {
            try {
                if ($null -ne $selection) { [IO.File]::WriteAllBytes($selectionPath,$selection) }
                elseif ($selectionWritten) { [IO.File]::Delete($selectionPath) }
                [IO.File]::WriteAllBytes($settingsPath,$settingsBytes)
                if ([IO.File]::ReadAllText((Join-Path $lock 'owner')) -ceq $ownerToken) { Remove-Item -LiteralPath (Join-Path $lock 'owner'); Remove-Item -LiteralPath $lock }
                $report.cleanup_complete=$true
            } catch { $report.passed=$false; $report.cleanup_complete=$false; $report.cleanup_error=$_.Exception.Message; if (-not $failure) { $failure=$_ } }
        } else {
            $report.cleanup_complete=$false; $report.retained_game_pid=if ($game) { $game.Id } else { $null }; $report.retained_lock=$lock
            if ($activeTools.Count -gt 0) {
                $report.retained_tool_pids=@($activeTools | ForEach-Object { [pscustomobject]@{ name = $_.Name; pid = $_.Process.Id } })
            }
        }
        $report.elapsed_seconds=$started.Elapsed.TotalSeconds; $report.finished_utc=[DateTime]::UtcNow.ToString('o'); Write-Report
    }
    if ($failure) { throw $failure }
    Write-Host 'FULL SMOKE PASSED, INCLUDING CLEANUP'
    $report | ConvertTo-Json -Depth 8
}

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $GameExe) {
    if ($(if (Test-Path Env:BATTLEPLAN_GAME_DIR) { $env:BATTLEPLAN_GAME_DIR } else { $env:HOI4_GAME_DIR })) { $GameExe = Join-Path $(if (Test-Path Env:BATTLEPLAN_GAME_DIR) { $env:BATTLEPLAN_GAME_DIR } else { $env:HOI4_GAME_DIR }) 'hoi4.exe' }
    else { throw 'Supply -GameExe or set BATTLEPLAN_GAME_DIR to the installed game directory.' }
}
if ($Action -eq 'Smoke') { Invoke-FullSmoke; return }
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'The game driver requires Windows and an interactive desktop.'
}
$gamePath = (Resolve-Path -LiteralPath $GameExe).ProviderPath
if ([IO.Path]::GetFileName($gamePath) -ine 'hoi4.exe') {
    throw 'GameExe must identify hoi4.exe.'
}
$gameProcessName = [IO.Path]::GetFileNameWithoutExtension($gamePath)
$gameProcesses = @(Get-Process -Name $gameProcessName -ErrorAction SilentlyContinue)
if ($gameProcesses.Count -gt 1) {
    throw 'Two game processes are unsupported. Close the extra game before driving the test.'
}

if ($Action -eq 'Launch') {
    if ($gameProcesses.Count -ne 0) { throw 'the game is already running.' }
    $userPath = Resolve-LaunchUserDirectory $UserDirectory
    if ($ExpectedSave -and -not $ContinueLastSave) {
        throw 'ExpectedSave requires ContinueLastSave.'
    }
    if ($ContinueLastSave -and -not $ExpectedSave) { throw 'ContinueLastSave requires ExpectedSave.' }
    $arguments = @('--gdpr-compliant', ('"-userdir=' + $userPath + '"'))
    if ($ContinueLastSave) {
        Write-LaunchSelection $userPath $ExpectedSave
        $arguments += '--continuelastsave'
        Write-Host "Launching the requested save: $ExpectedSave"
    }
    foreach ($extra in $ExtraArgs) { if ($extra -match '["\s]') { throw 'ExtraArgs entries must be single tokens without quotes.' }; $arguments += $extra }
    if (-not ('Hoi4ProbeInput' -as [type])) {
        $self = Get-Content -LiteralPath $PSCommandPath -Encoding UTF8 -Raw
        foreach ($definition in [regex]::Matches($self, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")) {
            if ($definition.Groups[1].Value -match 'class Hoi4ProbeInput') { Add-Type -TypeDefinition $definition.Groups[1].Value }
        }
    }
    $previousForeground = [Hoi4ProbeInput]::GetForegroundWindow()
    $started = Start-Process -FilePath $gamePath -WorkingDirectory (Split-Path -Parent $gamePath) -ArgumentList $arguments -PassThru
    $started.PriorityClass = [Diagnostics.ProcessPriorityClass]::Normal
    if (-not $Foreground) {
        $gameBuild = Write-GameBuildWarning $gamePath
        $clockUnavailable = $null
        try { $clockRecorder = Resolve-ClockRecorder $BinaryDirectory } catch { $clockUnavailable = $_.Exception.Message }
        $reader = $null; $quietSince = $null; $pushes = 0; $loaded = $false; $warned = $false
        $deadline = [DateTime]::UtcNow.AddSeconds(240)
        do {
            Start-Sleep -Milliseconds 100
            $started.Refresh()
            if ($started.HasExited) { throw 'The game exited during launch.' }
            $handle = $started.MainWindowHandle
            if ($handle -ne [IntPtr]::Zero) {
                if ([Hoi4ProbeInput]::GetForegroundWindow() -eq $handle) {
                    [void][Hoi4ProbeInput]::PushBehind($handle, $previousForeground); $pushes++; $quietSince = $null
                } elseif ($null -eq $quietSince) { $quietSince = [DateTime]::UtcNow }
                if ($null -eq $clockUnavailable -and $null -eq $reader) {
                    try {
                        $reader = Open-GameClock $started.Id $clockRecorder
                        $withheld = @(Get-ClockWithheld $reader)
                        if ($withheld.Count) { $clockUnavailable = "Game clock fields withheld: $($withheld -join '; ')."; $reader.Dispose(); $reader = $null }
                    } catch {
                        $reader = $null
                        if ($_.Exception.Message -like 'Clock reading is withheld*') { $clockUnavailable = $_.Exception.Message }
                    }
                }
            }
            $clock = $null
            if ($reader) {
                try { $clock = $reader.Sample() } catch { $clockUnavailable = $_.Exception.Message; $reader.Dispose(); $reader = $null }
            }
            if ($clockUnavailable -and -not $warned) { Write-Warning "$clockUnavailable The map load cannot be read from the clock; launch returns after the window stays in the background for 30 seconds."; $warned = $true }
            $loaded = [bool]($clock -and $clock.presentation_valid -and $null -ne $quietSince -and ([DateTime]::UtcNow - $quietSince).TotalSeconds -ge 5)
            $settled = $null -ne $clockUnavailable -and $null -ne $quietSince -and ([DateTime]::UtcNow - $quietSince).TotalSeconds -ge 30
        } while (-not $loaded -and -not $settled -and [DateTime]::UtcNow -lt $deadline)
        if ($reader) { $reader.Dispose() }
        $shim = if ($started.MainWindowHandle -ne [IntPtr]::Zero) { Install-InputShim $started.Id } else { 'no window' }
        [pscustomobject]@{ Action = 'Launch'; GameId = $started.Id; GameExe = $gamePath; RequestedUserDirectory = $userPath; Background = $true; ForegroundPushes = $pushes; LoadedBeforeReturn = $(if ($null -eq $clockUnavailable) { $loaded } else { $null }); InputShim = $shim; GameBuildWarning = $gameBuild.Warning; ClockUnavailable = $clockUnavailable }
        return
    }
    [pscustomobject]@{ Action = 'Launch'; GameId = $started.Id; GameExe = $gamePath; RequestedUserDirectory = $userPath }
    return
}

if ($gameProcesses.Count -ne 1) { throw 'Exactly one running hoi4.exe is required.' }
$game = $gameProcesses[0]
if ($GameId -ne 0 -and $game.Id -ne $GameId) { throw "The running game has PID $($game.Id), not $GameId." }
if ($game.Path -ine $gamePath) { throw "The running game executable differs from GameExe: $($game.Path)" }
if ($RequireInputShim -and $Action -in @('Click','Move','Wheel','Key','Text','Console')) {
    if ($Foreground) { throw 'RequireInputShim refuses foreground input.' }
    $requiredShim=Open-InputShim
    if (-not $requiredShim) { throw 'The required background input shim is absent; input was not sent.' }
    try { if ($requiredShim.View.ReadUInt32(80) -ne $game.Id) { throw 'The background input shim belongs to another process; input was not sent.' } }
    finally { Close-InputShim $requiredShim }
}
if ($Action -eq 'Reload') {
    . (Join-Path $PSScriptRoot 'recorder-guard.ps1')
    if (-not $LockOwner -or -not $GameId -or -not $UiConfig -or -not $ExpectedSave -or -not $Out) {
        throw 'Reload requires GameId, GameLock, LockOwner, UiConfig, ExpectedSave and Out.'
    }
    function Confirm-ReloadLock {
        if ([IO.File]::ReadAllText((Join-Path $GameLock 'owner')).Trim() -cne $LockOwner) { throw 'The shared game lock belongs to another screen.' }
        (Get-Item -LiteralPath $GameLock).LastWriteTimeUtc = [DateTime]::UtcNow
    }
    Confirm-ReloadLock
    $gameBuild = Write-GameBuildWarning $gamePath
    $clockRecorder = Resolve-ClockRecorder $BinaryDirectory
    function Confirm-ReloadIdle {
        Confirm-ReloadLock
        $game.Refresh()
        if ($game.HasExited) { throw 'Game exited during reload.' }
        if (@(Get-GameBlockingRecorder).Count) { throw 'Another recorder is running; Load was not requested.' }
        if (@($game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count) { throw 'Detach the collector before reloading.' }
    }
    Confirm-ReloadIdle
    $userPath = Resolve-LaunchUserDirectory $UserDirectory
    if ($ExpectedSave.IndexOfAny([char[]]'\/:"') -ge 0 -or $ExpectedSave -notlike '*.hoi4') { throw 'ExpectedSave must be a local save filename.' }
    $save = Join-Path (Join-Path $userPath 'save games') $ExpectedSave
    if (-not (Test-Path -LiteralPath $save -PathType Leaf)) { throw 'Expected save is missing.' }
    $saves=@(Get-ChildItem -LiteralPath (Split-Path -Parent $save) -File -Filter '*.hoi4' | Sort-Object LastWriteTimeUtc -Descending)
    $selected=Get-Item -LiteralPath $save
    $selectedSecond=([DateTimeOffset]$selected.LastWriteTimeUtc).ToUnixTimeSeconds()
    if (@($saves | Where-Object { ([DateTimeOffset]$_.LastWriteTimeUtc).ToUnixTimeSeconds() -eq $selectedSecond }).Count -ne 1) { throw 'The expected save has an ambiguous timestamp in the load-menu order.' }
    $row=@($saves | Where-Object LastWriteTimeUtc -gt $selected.LastWriteTimeUtc).Count
    $command = (Get-CimInstance Win32_Process -Filter "ProcessId=$GameId").CommandLine
    if ($command -notmatch ('(?i)(?:"-userdir=' + [regex]::Escape($userPath) + '"|-userdir="' + [regex]::Escape($userPath) + '"|-userdir=' + [regex]::Escape($userPath) + '(?=\s|$))')) { throw 'Running game uses a different user directory.' }
    $ui = Get-Content -LiteralPath $UiConfig -Raw | ConvertFrom-Json
    if (-not $MenuPatch -and $ui.PSObject.Properties['menu_patch']) { $MenuPatch=[string]$ui.menu_patch; $MenuPatchOrigin=[int[]]$ui.menu_patch_origin }
    if (-not $MenuPatch -or -not (Test-Path -LiteralPath $MenuPatch -PathType Leaf) -or $MenuPatchOrigin.Count -ne 2 -or $MenuPatchOrigin[0] -lt 0 -or $MenuPatchOrigin[1] -lt 0) { throw 'Reload requires a calibrated pause-menu reference image and origin.' }
    if (-not $ui.PSObject.Properties['load_button_patch'] -or -not (Test-Path -LiteralPath $ui.load_button_patch -PathType Leaf) -or $ui.load_button_patch_origin.Count -ne 2) { throw 'Reload requires a calibrated Load button reference image and origin.' }
    if (-not $ui.PSObject.Properties['load_menu_patch'] -or -not (Test-Path -LiteralPath $ui.load_menu_patch -PathType Leaf) -or $ui.load_menu_patch_origin.Count -ne 2) { throw 'Reload requires a calibrated Load dialog reference image and origin.' }
    if ($ui.expected_save -cne $ExpectedSave -or $null -eq $ui.expected_hour -or $null -eq $ui.expected_seed) { throw 'UiConfig must identify expected_save, expected_hour and expected_seed.' }
    foreach ($name in @('menu','load_game','load_local','load_row','load_button','cursor_park')) {
        $point = $ui.$name
        if ($point.Count -ne 2 -or $point[0] -lt 0 -or $point[1] -lt 0 -or $point[0] -ge $ui.width -or $point[1] -ge $ui.height) { throw "Invalid calibrated point: $name" }
    }
    if ($ui.load_row_step -le 0 -or $ui.load_visible_rows -le 0 -or $row -ge $ui.load_visible_rows) { throw "Expected save is outside the calibrated visible load rows: $ExpectedSave" }
    $rowY=[int]$ui.load_row[1]+$row*[int]$ui.load_row_step
    if ($rowY -ge $ui.height) { throw 'Expected save row lies outside the calibrated window.' }
    $shim = Open-InputShim
    if (-not $shim) { $null = Install-InputShim $game.Id; $shim = Open-InputShim }
    if (-not $shim) { throw 'Reload requires the background input shim; supply InputShimDir.' }
    Close-InputShim $shim
    $game.Refresh()
    if (@($game.Modules | Where-Object ModuleName -ieq 'inputshim.dll').Count -ne 1) { throw 'The background input shim is not loaded in the selected game.' }
    $driver = $PSCommandPath
    function Reload-Drive([string]$action, [hashtable]$extra) {
        Confirm-ReloadLock
        & $driver -Action $action -GameExe $gamePath -UserDirectory $userPath -GameId $GameId -InputShimDir $InputShimDir -RequireInputShim -CaptureRoot $CaptureRoot @extra
    }
    function Reload-Click([string]$name) { $point=$ui.$name; $null=Reload-Drive 'Click' @{X=[int]$point[0];Y=[int]$point[1]}; Start-Sleep -Milliseconds 500 }
    function Test-ReloadMenu([string]$name,[string]$patch=$MenuPatch,[int[]]$origin=$MenuPatchOrigin,[double]$tolerance=22,[switch]$Label) {
        $captureName=[IO.Path]::GetFileNameWithoutExtension($name)+'-'+[Guid]::NewGuid().ToString('N')+'.png'
        $shot=Reload-Drive 'Screenshot' @{Out=(Join-Path $Out $captureName)}
        if ($shot.Width -ne $ui.width -or $shot.Height -ne $ui.height) { throw 'Window dimensions differ from calibrated UI.' }
        Test-UiPatchMatch -ImagePath $shot.Path -PatchPath $patch -Origin $origin -Tolerance $tolerance -Label:$Label
    }
    function Wait-ReloadMenu([string]$name,[string]$patch,[int[]]$origin,[double]$tolerance,[switch]$Label) {
        $deadline=[DateTime]::UtcNow.AddSeconds(15)
        do {
            if (Test-ReloadMenu $name $patch $origin $tolerance -Label:$Label) { return }
            if ([DateTime]::UtcNow -ge $deadline) { throw "Reload witness failed: calibrated dialog did not become visible: $name" }
            Start-Sleep -Milliseconds 250
        } while ($true)
    }
    $reader = Open-GameClock $game.Id $clockRecorder
    try {
        $withheld = @(Get-ClockWithheld $reader)
        if ($withheld.Count) { throw "Reload needs the paused camera and load witness, and this build withholds: $($withheld -join '; ')" }
        $before = $reader.Sample()
        if (-not $before -or -not $before.presentation_valid -or $before.play_session_started_at -le 0) { throw 'No authenticated presentation/load witness before reload.' }
        $watch = [Diagnostics.Stopwatch]::StartNew()
        if (-not (Test-ReloadMenu 'menu-before.png')) {
            Reload-Click 'menu'
            Wait-ReloadMenu 'menu-open.png' $MenuPatch $MenuPatchOrigin 22
        }
        Reload-Click 'load_game'
        Wait-ReloadMenu 'load-menu.png' $ui.load_menu_patch ([int[]]$ui.load_menu_patch_origin) 5 -Label
        Reload-Click 'load_local'
        $null=Reload-Drive 'Click' @{X=[int]$ui.load_row[0];Y=$rowY}
        Start-Sleep -Milliseconds 500
        $null=Reload-Drive 'Move' @{X=[int]$ui.cursor_park[0];Y=[int]$ui.cursor_park[1]}
        Wait-ReloadMenu 'load-ready.png' $ui.load_button_patch ([int[]]$ui.load_button_patch_origin) 5 -Label
        Confirm-ReloadIdle
        [IO.File]::WriteAllText((Join-Path $Out 'load-requested.json'),(@{expected_save=$ExpectedSave;before_load=$before.play_session_started_at;requested_utc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json))
        Reload-Click 'load_button'
        $stable = $null
        while ($watch.Elapsed.TotalSeconds -lt 240) {
            Confirm-ReloadLock
            $game.Refresh(); if ($game.HasExited) { throw 'Game exited during reload.' }
            $clock=$reader.Sample()
            if ($clock -and $clock.play_session_started_at -gt 0 -and $clock.play_session_started_at -ne $before.play_session_started_at -and
                $clock.hour -eq $ui.expected_hour -and $clock.seed -eq $ui.expected_seed -and
                (-not $ui.PSObject.Properties['expected_play_time'] -or $clock.play_time -eq $ui.expected_play_time) -and
                $clock.tick_busy -eq 0 -and $clock.presentation_valid -and $clock.paused -eq 1) {
                if ($stable -and $clock.Same($stable) -and $watch.Elapsed.TotalSeconds - $stableAt -ge 2) {
                    return [pscustomobject]@{Action='Reload';GameId=$GameId;ExpectedSave=$ExpectedSave;Before=$before;Clock=$clock;Verified=$true;Seconds=$watch.Elapsed.TotalSeconds}
                }
                if (-not $stable -or -not $clock.Same($stable)) { $stable=$clock; $stableAt=$watch.Elapsed.TotalSeconds }
            } else { $stable=$null }
            Start-Sleep -Milliseconds 100
        }
        throw 'Reload witness failed: expected a new load timestamp and the expected stable clock and seed.'
    } finally { $reader.Dispose() }
}
if ($Action -eq 'Clock') {
    $gameBuild = Write-GameBuildWarning $gamePath
    $reader = Open-GameClock $game.Id (Resolve-ClockRecorder $BinaryDirectory)
    try { $reader.Sample() } finally { $reader.Dispose() }
    return
}
if ($Action -eq 'Baseline') {
    if (-not [Environment]::Is64BitProcess) { throw 'Baseline requires 64-bit PowerShell.' }
    $captureRoot = [IO.Path]::GetFullPath($CaptureRoot).TrimEnd([char[]]'\/')
    if (-not $Out) { throw 'Baseline requires -Out, an existing capture directory or a new .jsonl file in one.' }
    $capturePath = [IO.Path]::GetFullPath($Out)
    if (Test-Path -LiteralPath $capturePath -PathType Container) {
        $capturePath = Join-Path $capturePath ('baseline-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '-' + $game.Id + '.jsonl')
    }
    if (-not $capturePath.StartsWith($captureRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetExtension($capturePath) -ine '.jsonl') {
        throw "Baseline output must be a JSONL file under CaptureRoot."
    }
    if (-not (Test-Path -LiteralPath ([IO.Path]::GetDirectoryName($capturePath)) -PathType Container)) {
        throw 'Create the baseline capture directory before starting the measurement.'
    }
    if (Test-Path -LiteralPath $capturePath) { throw "Baseline already exists: $capturePath" }
    $gameBuild = Write-GameBuildWarning $gamePath
    $clockRecorder = Resolve-ClockRecorder $BinaryDirectory
    $actualSize = $gameBuild.Size
    $actualHash = $gameBuild.Sha256
    if (-not $gameBuild.Version) { throw 'The executable has no single version text.' }
    $exeVersion = $gameBuild.Version
    if (@($game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) {
        throw 'The free-run baseline requires collector.dll to be detached first.'
    }
    $reader = $null
    $stream = $null
    $writer = $null
    $summary = $null
    $stopReason = 'duration completed'
    try {
        $reader = Open-GameClock $game.Id $clockRecorder
        $processStart = $game.StartTime.ToUniversalTime().ToFileTimeUtc()
        $stream = [IO.File]::Open($capturePath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
        $writer.AutoFlush = $true
        $qpcStart = [Diagnostics.Stopwatch]::GetTimestamp()
        $lastFlush = $qpcStart
        $lastProcessCheck = $qpcStart
        $writer.WriteLine((@{ kind = 'baseline_start'; pid = $game.Id; process_start_time = $processStart;
            qpc = $qpcStart; qpc_frequency = $reader.Frequency; requested_seconds = $Seconds; sample_interval_ms = 125;
            exe_version = $exeVersion; exe_size = $actualSize; exe_sha256 = $actualHash.ToLowerInvariant();
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
        while (([Diagnostics.Stopwatch]::GetTimestamp() - $qpcStart) / [double]$reader.Frequency -lt $Seconds) {
            $game.Refresh()
            if ($game.HasExited) { $stopReason = 'game exited'; break }
            $clock = $reader.Sample()
            $qpc = [Diagnostics.Stopwatch]::GetTimestamp()
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
                if (@(Get-Process -Name $gameProcessName -ErrorAction SilentlyContinue).Count -gt 1) {
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
            observation_elapsed_s = ([Diagnostics.Stopwatch]::GetTimestamp() - $qpcStart) / [double]$reader.Frequency; stop_reason = $stopReason }
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
    [DllImport("user32.dll", SetLastError=true)] static extern bool PostMessageW(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", SetLastError=true)] static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
    static void Post(IntPtr window, uint message, UIntPtr w, IntPtr l) {
        if (!PostMessageW(window, message, w, l)) throw new Win32Exception(Marshal.GetLastWin32Error(), "PostMessage failed.");
    }
    static IntPtr PackPoint(int x, int y) { return (IntPtr)unchecked((long)((((uint)y & 0xFFFFu) << 16) | ((uint)x & 0xFFFFu))); }
    public static void PostKey(IntPtr window, ushort key) {
        uint scan = MapVirtualKeyW(key, 0);
        long down = 1 | ((long)(scan & 0xFF) << 16);
        long up = down | (1L << 30) | (1L << 31);
        Post(window, 0x0100, (UIntPtr)key, (IntPtr)down);
        System.Threading.Thread.Sleep(60);
        Post(window, 0x0101, (UIntPtr)key, (IntPtr)up);
    }
    public static void PostChar(IntPtr window, char character) { Post(window, 0x0102, (UIntPtr)character, (IntPtr)1); }
    public static void PostMove(IntPtr window, int clientX, int clientY) { Post(window, 0x0200, UIntPtr.Zero, PackPoint(clientX, clientY)); }
    public static void PostClick(IntPtr window, int clientX, int clientY) {
        Post(window, 0x0200, UIntPtr.Zero, PackPoint(clientX, clientY));
        System.Threading.Thread.Sleep(80);
        Post(window, 0x0201, (UIntPtr)1, PackPoint(clientX, clientY));
        System.Threading.Thread.Sleep(70);
        Post(window, 0x0202, UIntPtr.Zero, PackPoint(clientX, clientY));
    }
    public static void PostButton(IntPtr window, uint message, uint wParam, int clientX, int clientY) {
        Post(window, message, (UIntPtr)wParam, PackPoint(clientX, clientY));
    }
    public static void PostWheel(IntPtr window, int screenX, int screenY, int notches) {
        int step = notches < 0 ? -1 : 1;
        for (int sent = 0; sent != notches; sent += step) {
            Post(window, 0x020A, (UIntPtr)unchecked((uint)((step * 120) << 16)), PackPoint(screenX, screenY));
            System.Threading.Thread.Sleep(60);
        }
    }
    public static bool PushBehind(IntPtr gameWindow, IntPtr previous) {
        bool ok = SetWindowPos(gameWindow, (IntPtr)1, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
        if (previous != IntPtr.Zero && previous != gameWindow && IsWindow(previous)) FocusWindow(previous);
        return ok;
    }
    public static bool CaptureWindow(IntPtr window, IntPtr hdc) { return PrintWindow(window, hdc, 2); }
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] static extern uint GetDpiForSystem();
    public static int[] LogicalPoint(IntPtr window, int clientX, int clientY) {
        uint windowDpi = GetDpiForWindow(window), systemDpi = GetDpiForSystem();
        if (windowDpi == 0 || systemDpi == 0 || windowDpi >= systemDpi) return new int[] { clientX, clientY };
        return new int[] { (int)Math.Round(clientX * (double)windowDpi / systemDpi), (int)Math.Round(clientY * (double)windowDpi / systemDpi) };
    }
    public static int[] LogicalSize(IntPtr window, int physicalWidth, int physicalHeight) {
        uint windowDpi = GetDpiForWindow(window), systemDpi = GetDpiForSystem();
        if (windowDpi == 0 || systemDpi == 0 || windowDpi >= systemDpi) return new int[] { physicalWidth, physicalHeight };
        return new int[] { (int)Math.Round(physicalWidth * (double)windowDpi / systemDpi), (int)Math.Round(physicalHeight * (double)windowDpi / systemDpi) };
    }
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
        return AcquireTargetFocus(window, process, 5, 150);
    }
    public static IntPtr AcquireTargetFocus(IntPtr window, uint process, int attempts, int delayMs) {
        if (attempts < 1) attempts = 1;
        for (int attempt = 1; attempt <= attempts; attempt++) {
            uint actual;
            if (!IsWindow(window) || GetWindowThreadProcessId(window, out actual) == 0 || actual != process)
                throw new InvalidOperationException("The game window no longer belongs to the selected process.");
            if (IsIconic(window)) ShowWindow(window, 9);
            IntPtr previous = GetForegroundWindow();
            if (FocusWindow(window))
                return previous;
            if (attempt < attempts && delayMs > 0)
                System.Threading.Thread.Sleep(delayMs);
        }
        throw new InvalidOperationException("Windows refused to focus the game; input was not sent.");
    }
    public static void RestoreForeground(IntPtr previous, IntPtr gameWindow) {
        if (previous == IntPtr.Zero || previous == gameWindow || !IsWindow(previous)) return;
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
            if (SendInput(1, up, Marshal.SizeOf(typeof(Input))) != 1)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows did not release the game key.");
        }
    }
    public static void SelectAll(IntPtr window, uint process) {
        var down = new Input[1]; down[0].Type = 1;
        down[0].Data.Keyboard.Scan = 0x1d; down[0].Data.Keyboard.Flags = 8;
        var up = (Input[])down.Clone(); up[0].Data.Keyboard.Flags |= 2;
        Send(window, process, down);
        try { KeyPress(window, process, 0x41); }
        finally { if (SendInput(1, up, Marshal.SizeOf(typeof(Input))) != 1) throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows did not release Control."); }
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
    public static void WheelNotch(IntPtr window, uint process, int x, int y, int step) {
        CheckTarget(window, process);
        if (!SetCursorPos(x, y)) throw new Win32Exception(Marshal.GetLastWin32Error());
        var turn = new Input[1];
        turn[0].Data.Mouse.Flags = 0x0800;
        turn[0].Data.Mouse.Data = unchecked((uint)(step * 120));
        Send(window, process, turn);
        System.Threading.Thread.Sleep(60);
    }
    public static void Wheel(IntPtr window, uint process, int x, int y, int notches) {
        CheckTarget(window, process);
        if (!SetCursorPos(x, y)) throw new Win32Exception(Marshal.GetLastWin32Error());
        System.Threading.Thread.Sleep(100);
        int step = notches < 0 ? -1 : 1;
        int total = Math.Abs(notches);
        int delivered = 0;
        for (int sent = 0; sent != notches; sent += step) {
            for (int attempt = 1; ; attempt++) {
                try {
                    WheelNotch(window, process, x, y, step);
                    delivered++;
                    break;
                } catch (InvalidOperationException error) {
                    if (!error.Message.Contains("focus was lost")) throw;
                    if (attempt >= 5) {
                        if (delivered > 0)
                            throw new InvalidOperationException("Game focus was lost after delivering partial input (" + delivered + " of " + total + ").");
                        throw;
                    }
                    System.Threading.Thread.Sleep(150);
                    AcquireTargetFocus(window, process);
                    System.Threading.Thread.Sleep(100);
                }
            }
        }
    }
    public static void Click(IntPtr window, uint process, int x, int y) {
        CheckTarget(window, process);
        if (!SetCursorPos(x, y)) throw new Win32Exception(Marshal.GetLastWin32Error());
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
$previousForeground = [IntPtr]::Zero
if ($Foreground) {
    if ([Hoi4ProbeInput]::IsIconic($window)) { [void][Hoi4ProbeInput]::ShowWindow($window, 9) }
    $previousForeground = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
} elseif ([Hoi4ProbeInput]::IsIconic($window)) {
    [void][Hoi4ProbeInput]::ShowWindow($window, 4)
    [void][Hoi4ProbeInput]::PushBehind($window, [Hoi4ProbeInput]::GetForegroundWindow())
}
try {
    Start-Sleep -Milliseconds 200
    if ($Foreground) {
        Invoke-WithFocusRetry -OnRetry {
            Start-Sleep -Milliseconds 150
            $null = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
            Start-Sleep -Milliseconds 100
        } -Operation { [Hoi4ProbeInput]::CheckTarget($window, [uint32]$game.Id) }
    }
    if (-not [Hoi4ProbeInput]::IsWindowVisible($window)) { throw 'The game window is not visible.' }
    $rect = New-Object Hoi4ProbeInput+Rect
    if (-not [Hoi4ProbeInput]::GetWindowRect($window, [ref]$rect)) { throw 'Cannot read the game window rectangle.' }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) { throw 'The game window has an empty rectangle.' }

    switch ($Action) {
    'Screenshot' {
        $captureRoot = [IO.Path]::GetFullPath($CaptureRoot).TrimEnd([char[]]'\/')
        if (-not $Out) { $Out = Join-Path $captureRoot ('probe\game-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '.png') }
        $capturePath = [IO.Path]::GetFullPath($Out)
        if (-not $capturePath.StartsWith($captureRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetExtension($capturePath) -ine '.png') {
            throw "Screenshots must be PNG files under CaptureRoot."
        }
        if (Test-Path -LiteralPath $capturePath) { throw "Screenshot already exists: $capturePath" }
        Add-Type -AssemblyName System.Drawing
        Add-Type -AssemblyName System.Windows.Forms
        if ($Foreground) {
            $desktop = [Windows.Forms.SystemInformation]::VirtualScreen
            if ($rect.Left -lt $desktop.Left -or $rect.Top -lt $desktop.Top -or $rect.Right -gt $desktop.Right -or $rect.Bottom -gt $desktop.Bottom) {
                throw 'The game window must fit entirely on the desktop for a screenshot.'
            }
        }
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($capturePath))
        $bitmap = New-Object Drawing.Bitmap $width, $height
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            if ($Foreground) {
                [Hoi4ProbeInput]::CheckTarget($window, [uint32]$game.Id)
                $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
            } else {
                $logical = [Hoi4ProbeInput]::LogicalSize($window, $width, $height)
                $raw = New-Object Drawing.Bitmap ([int]$logical[0]), ([int]$logical[1])
                $rawGraphics = [Drawing.Graphics]::FromImage($raw)
                try {
                    $hdc = $rawGraphics.GetHdc()
                    try { if (-not [Hoi4ProbeInput]::CaptureWindow($window, $hdc)) { throw 'PrintWindow failed; the game window did not render into the capture.' } }
                    finally { $rawGraphics.ReleaseHdc($hdc) }
                    $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBilinear
                    $graphics.DrawImage($raw, 0, 0, $width, $height)
                } finally { $rawGraphics.Dispose(); $raw.Dispose() }
            }
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
        if ($Foreground) {
            if ($Key -eq 'CtrlA') { [Hoi4ProbeInput]::SelectAll($window, [uint32]$game.Id) }
            else { [Hoi4ProbeInput]::KeyPress($window, [uint32]$game.Id, [uint16]$keys[$Key]) }
        } elseif ($Key -eq 'CtrlA') {
            [Hoi4ProbeInput]::PostKey($window, 0x23)
            for ($n = 0; $n -lt 64; $n++) { [Hoi4ProbeInput]::PostKey($window, 8) }
        } else { [Hoi4ProbeInput]::PostKey($window, [uint16]$keys[$Key]) }
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
        if ($Action -eq 'Wheel' -and $Delta -eq 0) { throw 'Wheel requires a nonzero -Delta.' }
        if ($Foreground) {
            if ($Action -eq 'Move') { [Hoi4ProbeInput]::Move($window, [uint32]$game.Id, $screenX, $screenY) }
            elseif ($Action -eq 'Wheel') {
                $wheelStep = if ($Delta -lt 0) { -1 } else { 1 }
                $wheelSteps = @(1..([Math]::Abs($Delta)) | ForEach-Object { $wheelStep })
                Invoke-WithFocusRetry -OnRetry {
                    Start-Sleep -Milliseconds 150
                    $null = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
                    Start-Sleep -Milliseconds 100
                } -Operation { [Hoi4ProbeInput]::Move($window, [uint32]$game.Id, $screenX, $screenY) }
                Invoke-SequentialFocusInput -Items $wheelSteps -MaxAttemptsPerItem 5 -Pause { Start-Sleep -Milliseconds 150 } -Reacquire {
                    $null = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
                    Start-Sleep -Milliseconds 100
                } -SendItem { param($notchStep) [Hoi4ProbeInput]::WheelNotch($window, [uint32]$game.Id, $screenX, $screenY, [int]$notchStep) }
            } else { [Hoi4ProbeInput]::Click($window, [uint32]$game.Id, $screenX, $screenY) }
        } elseif ($Posted -or $Action -eq 'Move') {
            $logical = [Hoi4ProbeInput]::LogicalPoint($window, $screenX - $origin.X, $screenY - $origin.Y)
            if ($Action -eq 'Move') { [Hoi4ProbeInput]::PostMove($window, [int]$logical[0], [int]$logical[1]) }
            elseif ($Action -eq 'Wheel') { [Hoi4ProbeInput]::PostWheel($window, $screenX, $screenY, $Delta) }
            else { [Hoi4ProbeInput]::PostClick($window, [int]$logical[0], [int]$logical[1]) }
        } else {
            $shim = Open-InputShim
            if ($shim) {
                try {
                    $logical = [Hoi4ProbeInput]::LogicalPoint($window, $screenX - $origin.X, $screenY - $origin.Y)
                    $virtual = [Hoi4ProbeInput]::LogicalPoint($window, $screenX, $screenY)
                    Invoke-ShimPointer $shim $window ([int]$logical[0]) ([int]$logical[1]) ([int]$virtual[0]) ([int]$virtual[1]) {
                        if ($Action -eq 'Wheel') { [Hoi4ProbeInput]::PostWheel($window, [int]$virtual[0], [int]$virtual[1], $Delta) }
                        else {
                            Set-ShimFlags $shim ($script:ShimCursorFlag -bor $script:ShimFocusFlag -bor $script:ShimLeftFlag)
                            [Hoi4ProbeInput]::PostButton($window, 0x0201, 1, [int]$logical[0], [int]$logical[1])
                            Start-Sleep -Milliseconds 60
                            Set-ShimFlags $shim ($script:ShimCursorFlag -bor $script:ShimFocusFlag)
                            [Hoi4ProbeInput]::PostButton($window, 0x0202, 0, [int]$logical[0], [int]$logical[1])
                            Start-Sleep -Milliseconds 60
                        }
                    }
                } finally { Close-InputShim $shim }
            } else {
                if ($RequireInputShim) { throw 'The required background input shim disappeared; input was not sent.' }
                $flash = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
                try {
                    Start-Sleep -Milliseconds 120
                    if ($Action -eq 'Wheel') {
                        $wheelStep = if ($Delta -lt 0) { -1 } else { 1 }
                        $wheelSteps = @(1..([Math]::Abs($Delta)) | ForEach-Object { $wheelStep })
                        Invoke-WithFocusRetry -OnRetry {
                            Start-Sleep -Milliseconds 150
                            $null = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
                            Start-Sleep -Milliseconds 100
                        } -Operation { [Hoi4ProbeInput]::Move($window, [uint32]$game.Id, $screenX, $screenY) }
                        Invoke-SequentialFocusInput -Items $wheelSteps -MaxAttemptsPerItem 5 -Pause { Start-Sleep -Milliseconds 150 } -Reacquire {
                            $null = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
                            Start-Sleep -Milliseconds 100
                        } -SendItem { param($notchStep) [Hoi4ProbeInput]::WheelNotch($window, [uint32]$game.Id, $screenX, $screenY, [int]$notchStep) }
                    } else { [Hoi4ProbeInput]::Click($window, [uint32]$game.Id, $screenX, $screenY) }
                } finally {
                    [Hoi4ProbeInput]::RestoreForeground($flash, $window)
                    [void][Hoi4ProbeInput]::PushBehind($window, $flash)
                }
            }
        }
        [pscustomobject]@{ Action = $Action; GameId = $game.Id; X = $X; Y = $Y; Delta = $Delta }
    }
    'ShimStatus' {
        $shim = Open-InputShim
        if (-not $shim) { throw 'The input shim is not attached; no control block exists.' }
        try {
            $first = Read-ShimCounters $shim
            Start-Sleep -Seconds 1
            $second = Read-ShimCounters $shim
            $calls = [ordered]@{}
            foreach ($hook in $script:ShimHooks) { $calls[$hook] = [pscustomobject]@{ Calls = $second[$hook]; PerSecond = $second[$hook] - $first[$hook] } }
            [pscustomobject]@{ Action = $Action; GameId = $game.Id; HookMask = $shim.View.ReadUInt32($script:ShimMaskOffset);
                Flags = $shim.View.ReadUInt32($script:ShimFlagsOffset);
                Cursor = @($shim.View.ReadInt32($script:ShimCursorOffset), $shim.View.ReadInt32($script:ShimCursorOffset + 4));
                Window = '0x{0:x}' -f $shim.View.ReadUInt64($script:ShimWindowOffset);
                LastVkAsync = $shim.View.ReadUInt32(64); LastVkState = $shim.View.ReadUInt32(68); Hooks = [pscustomobject]$calls }
        } finally { Close-InputShim $shim }
    }
    'ShimMask' {
        if ($Mask -lt 0) { throw 'ShimMask requires -Mask, a value from 0 to 127 (bit per hook, 0x7f enables all).' }
        $shim = Open-InputShim
        if (-not $shim) { throw 'The input shim is not attached; no control block exists.' }
        try {
            $shim.View.Write($script:ShimMaskOffset, [uint32]$Mask)
            $enabled = @(for ($index = 0; $index -lt $script:ShimHooks.Count; $index++) { if ($Mask -band (1 -shl $index)) { $script:ShimHooks[$index] } })
            [pscustomobject]@{ Action = $Action; GameId = $game.Id; HookMask = $shim.View.ReadUInt32($script:ShimMaskOffset); Enabled = $enabled }
        } finally { Close-InputShim $shim }
    }
    { $_ -eq 'Text' -or $_ -eq 'Console' } {
        if (-not $Text -or $Text -match '[\r\n\x00-\x1F]') { throw 'Text and Console require one nonempty line without control characters.' }
        if ($Foreground) {
            $events = [Collections.Generic.List[object]]::new()
            if ($Action -eq 'Console') { $events.Add([pscustomobject]@{ Kind = 'key'; Value = [uint16]$ConsoleKey; Delay = 200 }) }
            foreach ($character in $Text.ToCharArray()) { $events.Add([pscustomobject]@{ Kind = 'char'; Value = [string]$character; Delay = 0 }) }
            if ($Action -eq 'Console') {
                $events.Add([pscustomobject]@{ Kind = 'key'; Value = [uint16]0x0D; Delay = 200 })
                $events.Add([pscustomobject]@{ Kind = 'key'; Value = [uint16]$ConsoleKey; Delay = 0 })
            }
            Invoke-SequentialFocusInput -Items $events.ToArray() -MaxAttemptsPerItem 5 -Pause { Start-Sleep -Milliseconds 400 } -Reacquire {
                $null = [Hoi4ProbeInput]::AcquireTargetFocus($window, [uint32]$game.Id)
                Start-Sleep -Milliseconds 150
            } -SendItem {
                param($ev)
                if ($ev.Kind -eq 'key') { [Hoi4ProbeInput]::KeyPress($window, [uint32]$game.Id, [uint16]$ev.Value) }
                else { [Hoi4ProbeInput]::TypeText($window, [uint32]$game.Id, [string]$ev.Value) }
                if ($ev.Delay -gt 0) { Start-Sleep -Milliseconds $ev.Delay }
            }
        } else {
            if ($Action -eq 'Console') { [Hoi4ProbeInput]::PostKey($window, [uint16]$ConsoleKey); Start-Sleep -Milliseconds 200 }
            foreach ($character in $Text.ToCharArray()) { [Hoi4ProbeInput]::PostChar($window, $character); Start-Sleep -Milliseconds 15 }
            if ($Action -eq 'Console') {
                Start-Sleep -Milliseconds 100
                [Hoi4ProbeInput]::PostKey($window, 0x0D)
                Start-Sleep -Milliseconds 200
                [Hoi4ProbeInput]::PostKey($window, [uint16]$ConsoleKey)
            }
        }
        [pscustomobject]@{ Action = $Action; GameId = $game.Id; Text = $Text }
    }
    }
} finally {
    if ($previousForeground -ne [IntPtr]::Zero -and -not $KeepForeground) { [Hoi4ProbeInput]::RestoreForeground($previousForeground, $window) }
}
