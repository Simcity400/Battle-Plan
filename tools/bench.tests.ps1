# Offline parser regressions only; no game, recorder, build or benchmark is launched.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'bench.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw $errors[0] }
foreach ($definition in $ast.FindAll({param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -in @('Write-Json', 'Get-SizeSummary')
}, $false)) { . ([scriptblock]::Create($definition.Extent.Text)) }
$root = Join-Path ([IO.Path]::GetTempPath()) ('hoi4-bench-summary-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $root
try {
    $log = Join-Path $root 'recorder.stdout.log'
    $header = 'finalize_s=12.500 replay_bytes=10000 spool_bytes=20000'
    $evidence = 'hours=720 total_p50_ms=not_summarized total_p95_ms=not_summarized'
    $ready = 'Stop to save-ready: {"save_ready":true,"stop_to_save_ready_s":16.25,"pack_s":null}'
    [IO.File]::WriteAllLines($log, @($header, $evidence, $ready), $utf8)
    $size = Get-SizeSummary $root
    if ($size.stop_to_save_ready_s -ne 16.25 -or $null -ne $size.raw_bytes_per_hour -or $null -ne $size.replay_bytes_per_hour -or $null -ne $size.stop_diagnostics.pack_s) { throw 'Unknown evidence became a measurement.' }
    if ([IO.File]::ReadAllText((Join-Path $root 'tables.json')) -ne '[]') { throw 'Deferred tables are not an empty JSON array.' }
    foreach ($bad in @('', 'Stop to save-ready: {"save_ready":false,"stop_to_save_ready_s":null}')) {
        [IO.File]::WriteAllLines($log, @($header, $evidence, $bad), $utf8)
        $refused = $false
        try { $null = Get-SizeSummary $root } catch { $refused = $true }
        if (-not $refused) { throw 'Missing or failed publication was accepted.' }
    }
    $unknown = 'Stop to save-ready: {"save_ready":true,"stop_to_save_ready_s":null,"pack_s":null}'
    [IO.File]::WriteAllLines($log, @($header, $evidence, $unknown), $utf8)
    $size = Get-SizeSummary $root
    if ($null -ne $size.stop_to_save_ready_s) { throw 'An unavailable CLI timestamp became a Stop measurement.' }
    [IO.File]::WriteAllLines($log, @($header, $evidence, 'save_pause_s=300', $ready), $utf8)
    $size = Get-SizeSummary $root
    if ($size.stop_to_save_ready_s -ne 16.25) { throw 'The plan-save pause changed Stop to save-ready.' }
    [IO.File]::WriteAllLines($log, @('finalize_s=not_observed replay_bytes=10000 spool_bytes=20000', 'hours=0 total_p50_ms=not_observed', $ready), $utf8)
    $size = Get-SizeSummary $root
    if ($null -ne $size.finalize_s -or $null -ne $size.spool_bytes_per_hour) { throw 'Unobserved timing or coverage became zero.' }
    [IO.File]::WriteAllLines($log, @($header, 'hours=2 total_p50_ms=1.0', 'table raw_encoded_bytes bytes_per_hour read_p50_ms read_p95_ms read_max_ms snapshots deltas unchanged truncated unavailable', 'Clock 100 50 1 2 3 1 1 0 0 0'), $utf8)
    $size = Get-SizeSummary $root
    if ($size.raw_bytes_per_hour -ne 50 -or $null -ne $size.stop_to_save_ready_s) { throw 'Legacy summary was changed or given a fabricated Stop timer.' }
} finally {
    Remove-Item -LiteralPath (Join-Path $root 'recorder.stdout.log')
    Remove-Item -LiteralPath (Join-Path $root 'tables.json') -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $root
}
