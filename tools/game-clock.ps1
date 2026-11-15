# The game's clock for the game-driving tools, read by `recorder.exe clock` through the located
# sites with read-only rights on any game build; nothing here reads game memory itself.
# Dot-source it, open a reader with Open-GameClock, then call .Sample() and .Dispose().

# recorder.exe in Directory, or in this checkout's release build when none is given.
function Resolve-ClockRecorder([string]$Directory) {
    if (-not $Directory) { $Directory = Join-Path (Split-Path -Parent $PSScriptRoot) '.cargo-target\release' }
    $recorder = Join-Path $Directory 'recorder.exe'
    if (-not (Test-Path -LiteralPath $recorder -PathType Leaf)) {
        throw "The game clock is read by recorder.exe, and there is none at $recorder. Pass the directory of a recorder build."
    }
    (Resolve-Path -LiteralPath $recorder).ProviderPath
}

function Read-ClockLine($Reader, [int]$Seconds) {
    $line = $Reader.Process.StandardOutput.ReadLineAsync()
    if (-not $line.Wait($Seconds * 1000)) {
        $Reader.Broken = $true
        throw "recorder.exe clock (PID $($Reader.Process.Id)) gave no answer within $Seconds s."
    }
    if ($null -eq $line.Result) {
        $Reader.Broken = $true
        $Reader.Process.WaitForExit(5000) | Out-Null
        throw "recorder.exe clock (PID $($Reader.Process.Id)) ended: $($Reader.Errors.GetAwaiter().GetResult().Trim())"
    }
    $line.Result | ConvertFrom-Json
}

function ConvertTo-GameClock($Value, $Withheld) {
    if ($null -eq $Value) { return $null }
    $names = @($Value.PSObject.Properties.Name)
    $camera = $names -contains 'eye'
    $clock = [pscustomobject]@{
        game_state = [uint64]$Value.game_state; hour = [int]$Value.hour; play_time = [uint64]$Value.play_time
        seed = [int]$Value.seed; speed = [int]$Value.speed; tick_busy = [int]$Value.tick_busy
        play_session_started_at = [int64]$Value.play_session_started_at
        # Absent, never zero: the pause state and camera were not read or are withheld on this build.
        paused = $(if ($names -contains 'paused') { [int][bool]$Value.paused } else { $null })
        presentation_valid = $camera
        eye = $(if ($camera) { [double[]]$Value.eye } else { $null })
        look_at = $(if ($camera) { [double[]]$Value.look_at } else { $null })
        viewport = $(if ($camera) { [int[]]$Value.viewport } else { $null })
        withheld = $Withheld
    }
    $clock | Add-Member -MemberType ScriptMethod -Name Same -Value {
        param($other)
        $null -ne $other -and $this.game_state -eq $other.game_state -and $this.hour -eq $other.hour -and
            $this.play_time -eq $other.play_time -and $this.play_session_started_at -eq $other.play_session_started_at -and
            $this.seed -eq $other.seed -and $this.speed -eq $other.speed -and $this.tick_busy -eq $other.tick_busy -and
            $this.paused -eq $other.paused
    }
    $clock
}

# A reader of the game process ProcessId's clock. Throws "Clock reading is withheld: ..." with the
# recorder's named reason when this build withholds the clock, and other errors as they come.
function Open-GameClock([int]$ProcessId, [string]$Recorder) {
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $Recorder; $info.Arguments = "clock --pid $ProcessId"
    $info.UseShellExecute = $false; $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true; $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false); $info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::Start($info)
    $reader = [pscustomobject]@{
        Process = $process; Errors = $process.StandardError.ReadToEndAsync(); Broken = $false
        Frequency = [Diagnostics.Stopwatch]::Frequency; Withheld = $null
    }
    $reader | Add-Member -MemberType ScriptMethod -Name Dispose -Value {
        if ($this.Process.HasExited) { return }
        try { $this.Process.StandardInput.Close() } catch {}
        if (-not $this.Process.WaitForExit(5000)) { $this.Process.Kill(); $this.Process.WaitForExit(5000) | Out-Null }
    }
    $reader | Add-Member -MemberType ScriptMethod -Name Sample -Value {
        if ($this.Broken) { throw "recorder.exe clock (PID $($this.Process.Id)) stopped answering." }
        $this.Process.StandardInput.WriteLine()
        $this.Process.StandardInput.Flush()
        ConvertTo-GameClock (Read-ClockLine $this 10).clock $this.Withheld
    }
    try {
        # Locating the sites reads and checks the whole executable once.
        $ready = Read-ClockLine $reader 120
        if (-not $ready.ready) {
            $named = @($ready.withheld.PSObject.Properties | ForEach-Object { "$($_.Name): $($_.Value)" }) -join '; '
            throw "Clock reading is withheld: $named"
        }
        $reader.Withheld = $ready.withheld
    } catch { $reader.Dispose(); throw }
    $reader
}

# The fields the reader's build withholds, by name, as "name: reason" lines; none when all read.
function Get-ClockWithheld($Reader) {
    @($Reader.Withheld.PSObject.Properties | ForEach-Object { "$($_.Name): $($_.Value)" })
}
