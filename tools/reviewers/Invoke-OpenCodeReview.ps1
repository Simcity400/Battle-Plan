<#
.SYNOPSIS
  Run one OpenCode job non-interactively and return its reply, session id and exit code.

.DESCRIPTION
  Mirrors the Codex reviewer call shape used in AGENTS.md: one fresh session per job,
  the prompt fed on stdin, the reply read from a file (never from stdout), and the
  session id persisted the moment it appears so a hard kill still leaves a resumable
  handle. Pass -SessionId to continue an existing session instead of starting a new one.

  OpenCode sometimes halts before it has finished. When a turn ends on anything other
  than a clean 'stop', the script sends "continue" into the same session and keeps
  going, up to -MaxContinues times, then returns the whole reply joined together.
  It gives up early if a continue produces no new text at all.

.EXAMPLE
  $r = & .\tools\reviewers\Invoke-OpenCodeReview.ps1 -Prompt $Prompt -RunDir .\runs\lane-XX
  if ($r.ExitCode -ne 0 -or -not $r.ReplyText) { throw 'opencode job failed' }
  $r.ReplyText

.EXAMPLE
  $r2 = & .\tools\reviewers\Invoke-OpenCodeReview.ps1 -Prompt $FixNote -SessionId $r.SessionId -RunDir .\runs\lane-XX
#>
[CmdletBinding(DefaultParameterSetName = 'Inline')]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'Inline')]
    [string] $Prompt,

    [Parameter(Mandatory = $true, ParameterSetName = 'FromFile')]
    [string] $PromptPath,

    # Continue an existing session. Resume by id only - never --continue.
    [string] $SessionId,

    # Ox Alpha at max reasoning effort is the standing default (owner, 2026-08-21).
    [string] $Model = 'opencode/x-preview-f-free',

    # Provider-specific reasoning effort. Empty means "provider default".
    [string] $Variant = 'max',

    # Skip the variant sanity check (saves one catalog lookup on the first call of the day).
    [switch] $SkipVariantCheck,

    # OpenCode sometimes halts before it has finished. The script then says "continue"
    # into the same session and keeps going, up to this many times.
    [int] $MaxContinues = 20,

    # 'plan' is OpenCode's built-in read-only agent: it cannot edit files.
    [string] $Agent = 'plan',

    # Where to persist the session id and a copy of the transcript.
    [string] $RunDir,

    [string] $Title
)

$ErrorActionPreference = 'Stop'

if ($PSCmdlet.ParameterSetName -eq 'FromFile') {
    if (-not (Test-Path -LiteralPath $PromptPath)) { throw "prompt file not found: $PromptPath" }
    $Prompt = Get-Content -LiteralPath $PromptPath -Raw
}
if ([string]::IsNullOrWhiteSpace($Prompt)) { throw 'prompt is empty' }

$exe = Join-Path $env:APPDATA 'npm\opencode.cmd'
if (-not (Test-Path -LiteralPath $exe)) {
    $cmd = Get-Command opencode -ErrorAction SilentlyContinue
    if ($null -eq $cmd) { throw 'opencode CLI not found; install with: npm install -g opencode-ai' }
    $exe = $cmd.Source
}

$uniq       = [guid]::NewGuid().ToString('N')
$tmp        = [IO.Path]::GetTempPath()
$EventsPath = Join-Path $tmp ("oc_{0}.jsonl" -f $uniq)
$ReplyPath  = Join-Path $tmp ("oc_{0}.txt"   -f $uniq)

# The CLI accepts an unknown --variant silently, which would quietly downgrade the
# reasoning effort with no error. Confirm the model actually declares this variant.
if ($Variant -and -not $SkipVariantCheck) {
    $catalogPath = Join-Path $tmp ("oc_models_{0}.txt" -f (Get-Date -Format 'yyyyMMdd'))
    if (-not (Test-Path -LiteralPath $catalogPath)) {
        (& $exe models --verbose 2>&1 | Out-String) | Set-Content -LiteralPath $catalogPath -Encoding utf8
    }
    $blocks = @{}
    $key = $null
    $buf = $null
    foreach ($line in ((Get-Content -LiteralPath $catalogPath -Raw) -split "`r?`n")) {
        if ($line -match '^[a-z0-9\-]+/\S+$') {
            if ($key) { $blocks[$key] = $buf.ToString() }
            $key = $line.Trim()
            $buf = New-Object System.Text.StringBuilder
        } elseif ($null -ne $buf) { [void]$buf.AppendLine($line) }
    }
    if ($key) { $blocks[$key] = $buf.ToString() }

    if (-not $blocks.ContainsKey($Model)) {
        throw "model '$Model' is not in the opencode catalog; run: opencode models"
    }
    $block = $blocks[$Model]
    $vi = $block.IndexOf('"variants"')
    $declares = ($vi -ge 0) -and ($block.Substring($vi) -match ('"{0}"\s*:' -f [regex]::Escape($Variant)))
    if (-not $declares) {
        throw "model '$Model' does not declare variant '$Variant'; the CLI would ignore it and silently lower the reasoning effort"
    }
}

$SessionPath = $null
if ($RunDir) {
    if (-not (Test-Path -LiteralPath $RunDir)) { New-Item -ItemType Directory -Path $RunDir -Force | Out-Null }
    $SessionPath = Join-Path $RunDir 'opencode_session.txt'
}

$ocBase = @('run', '--format', 'json', '--auto', '--agent', $Agent, '--model', $Model)
if ($Variant) { $ocBase += @('--variant', $Variant) }

$events       = New-Object System.IO.StreamWriter($EventsPath, $false, [Text.UTF8Encoding]::new($false))
$turns        = New-Object System.Collections.Generic.List[string]
$seenSession  = $SessionId
$exitCode     = $null
$finishReason = $null
$continues    = 0
$turnPrompt   = $Prompt
$isContinue   = $false

try {
    while ($true) {
        $ocArgs = $ocBase
        if ($seenSession) { $ocArgs += @('--session', $seenSession) }
        elseif ($Title)   { $ocArgs += @('--title', $Title) }

        $texts        = New-Object System.Collections.Generic.List[string]
        $finishReason = $null

        $turnPrompt | & $exe @ocArgs 2>&1 | ForEach-Object {
            $line = [string]$_
            $events.WriteLine($line)
            $events.Flush()

            if ($line -notmatch '^\s*\{') { return }
            try { $ev = $line | ConvertFrom-Json } catch { return }

            # Persist the session id the instant it appears - a hard kill skips exit-time code.
            if (-not $seenSession -and $ev.sessionID) {
                $seenSession = [string]$ev.sessionID
                if ($SessionPath) { Set-Content -LiteralPath $SessionPath -Value $seenSession -Encoding utf8 }
            }
            if ($ev.type -eq 'text' -and $ev.part -and $ev.part.text) { $texts.Add([string]$ev.part.text) }
            if ($ev.type -eq 'step_finish' -and $ev.part -and $ev.part.reason) { $finishReason = [string]$ev.part.reason }
        }
        $exitCode = $LASTEXITCODE

        $turnText = ($texts -join '')
        if ($turnText) { $turns.Add($turnText) }

        # A clean finish is exit 0, a 'stop' reason, and something actually said.
        if ($exitCode -eq 0 -and $finishReason -eq 'stop' -and $turnText) { break }
        if (-not $seenSession)               { break }   # no session to resume into
        if ($isContinue -and -not $turnText) { break }   # continuing is getting us nowhere
        if ($continues -ge $MaxContinues)    { break }

        $continues++
        $isContinue = $true
        $turnPrompt = 'continue'
        $events.WriteLine(('{{"type":"wrapper_continue","attempt":{0},"afterReason":"{1}","afterExit":"{2}"}}' -f $continues, $finishReason, $exitCode))
        $events.Flush()
    }
}
finally {
    $events.Close()
}

$replyText = ($turns -join "`n")
Set-Content -LiteralPath $ReplyPath -Value $replyText -Encoding utf8

if ($RunDir) {
    Copy-Item -LiteralPath $EventsPath -Destination (Join-Path $RunDir 'opencode_events.jsonl') -Force
    Copy-Item -LiteralPath $ReplyPath  -Destination (Join-Path $RunDir 'opencode_reply.txt')    -Force
}

[pscustomobject]@{
    SessionId    = $seenSession
    ExitCode     = $exitCode
    ReplyText    = $replyText
    ReplyPath    = $ReplyPath
    EventsPath   = $EventsPath
    SessionPath  = $SessionPath
    Model        = $Model
    Variant      = $Variant
    Agent        = $Agent
    Continues    = $continues
    FinishReason = $finishReason
}
