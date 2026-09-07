[CmdletBinding()]
param([string]$HarnessPath = '')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($HarnessPath)) {
    $HarnessPath = Join-Path $PSScriptRoot 'Invoke-ReleaseManual.ps1'
}
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($HarnessPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw 'release driver does not parse' }
foreach ($name in @('Invoke-NativeChecked', 'ConvertFrom-GateJson', 'Test-JsonIntegerAtLeast')) {
    $matches = @($ast.FindAll({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true))
    if ($matches.Count -ne 1) { throw "expected one owning function: $name" }
    . ([scriptblock]::Create($matches[0].Extent.Text))
}
# Execute the shipped caller and admission, not a copy of its predicates.
$loops = @($ast.FindAll({ param($node)
    $node -is [Management.Automation.Language.ForEachStatementAst] -and
    $node.Variable.VariablePath.UserPath -eq 'gate' -and $node.Condition.Extent.Text -eq '$gateSpecs'
}, $true))
if ($loops.Count -ne 1) { throw 'expected one release gate loop' }
$gateLoop = [scriptblock]::Create($loops[0].Extent.Text)
$root = Join-Path ([IO.Path]::GetTempPath()) ('release-driver-units-' + [guid]::NewGuid().ToString('N'))
$null = [IO.Directory]::CreateDirectory($root)
$failures = [Collections.Generic.List[string]]::new()
$cases = 0
function New-OutputChild {
    param([string]$Text, [int]$Code = 0)
    $path = Join-Path $root ([guid]::NewGuid().ToString('N') + '.ps1')
    $base64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Text))
    $body = "[Console]::Out.Write([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$base64')))`r`nexit $Code`r`n"
    $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew)
    try { $bytes = [Text.Encoding]::UTF8.GetBytes($body); $stream.Write($bytes, 0, $bytes.Length) }
    finally { $stream.Dispose() }
    return $path
}
function Assert-GateCase {
    param([string]$Name, [string]$Text, [int]$Code, [bool]$Accepted, [bool]$Multiline = $false)
    $script:cases++
    $path = New-OutputChild -Text $Text -Code $Code
    $gateSpecs = @([pscustomobject]@{ Path = $path; Schema = 'hoi4-headless-raw-capture-unit-test-v1'; Status = 'pass'; MinCases = 200; Multiline = $Multiline })
    $caught = $null
    try { . $gateLoop } catch { $caught = $_ }
    if (($null -eq $caught) -ne $Accepted) {
        $failures.Add("$Name expected accepted=$Accepted; error=$caught")
        Write-Host "FAIL $Name"
    } else { Write-Host "PASS $Name" }
}
$good = '{"Schema":"hoi4-headless-raw-capture-unit-test-v1","Status":"pass","Cases":494}'
Assert-GateCase 'single receipt' $good 0 $true
Assert-GateCase 'supervision diagnostics before receipt' ("phase=supervision_contained step=probe_throws`nphase=supervision_contained step=probe_binding`n" + $good + "`n") 0 $true
Assert-GateCase 'blank lines after receipt' ($good + "`n`n  `n") 0 $true
Assert-GateCase 'exact case threshold' ($good.Replace('494', '200')) 0 $true
Assert-GateCase 'child nonzero despite PASS JSON' $good 7 $false
Assert-GateCase 'empty stdout' '' 0 $false
Assert-GateCase 'noise without receipt' 'phase=supervision_contained' 0 $false
Assert-GateCase 'malformed final JSON' '{"Schema":' 0 $false
Assert-GateCase 'diagnostic after valid JSON is not skipped' ($good + "`nphase=trailing") 0 $false
Assert-GateCase 'wrong schema' ($good.Replace('unit-test-v1', 'unit-test-v2')) 0 $false
Assert-GateCase 'wrong status' ($good.Replace('"pass"', '"fail"')) 0 $false
Assert-GateCase 'wrong status case' ($good.Replace('"pass"', '"PASS"')) 0 $false
Assert-GateCase 'too few cases' ($good.Replace('494', '199')) 0 $false
Assert-GateCase 'string case count' ($good.Replace('494', '"494"')) 0 $false
Assert-GateCase 'fractional case count' ($good.Replace('494', '494.5')) 0 $false
Assert-GateCase 'boolean case count' ($good.Replace('494', 'true')) 0 $false
Assert-GateCase 'missing case count' ($good.Replace(',"Cases":494', '')) 0 $false
Assert-GateCase 'null receipt' 'null' 0 $false
Assert-GateCase 'last receipt FAIL cannot use earlier PASS' ($good + "`n" + $good.Replace('"pass"', '"fail"')) 0 $false
$prettyGate = "{`n  `"Schema`": `"hoi4-headless-raw-capture-unit-test-v1`",`n  `"Status`": `"pass`",`n  `"Cases`": 494`n}"
Assert-GateCase 'multiline gate receipt' $prettyGate 0 $true $true
Assert-GateCase 'multiline gate wrong schema' ($prettyGate.Replace('unit-test-v1', 'unit-test-v2')) 0 $false $true
Assert-GateCase 'multiline gate wrong status' ($prettyGate.Replace('"pass"', '"FAIL"')) 0 $false $true
Assert-GateCase 'multiline gate too few cases' ($prettyGate.Replace('494', '199')) 0 $false $true
$script:cases++
try {
    $pretty = "{`n  `"schema`": `"multiline`",`n  `"values`": [1, 2]`n}`n"
    $path = New-OutputChild -Text $pretty
    $output = @(Invoke-NativeChecked -Program 'powershell.exe' -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $path) -Label 'multiline')
    if ($output.Count -ne 4 -or @($output | Where-Object { $_ -isnot [string] }).Count -ne 0) { throw 'stdout lines were nested or altered' }
    $receipt = ConvertFrom-GateJson -Output $output -Label 'multiline' -Multiline
    if ($receipt.schema -cne 'multiline' -or ($receipt.values -join ',') -cne '1,2') { throw 'multiline receipt changed' }
    Write-Host 'PASS multiline native stdout'
} catch { $failures.Add("multiline native stdout: $_"); Write-Host 'FAIL multiline native stdout' }
$script:cases++
try {
    $output = @(Invoke-NativeChecked -Program 'python' -Arguments @(
        '-B', (Join-Path $PSScriptRoot 'test_prebuilt_package_publication.py')
    ) -Label 'prebuilt package publication units')
    $publication = ConvertFrom-GateJson -Output $output -Label 'prebuilt package publication units'
    if ($publication.schema -cne 'hoi4-prebuilt-package-publication-tests.v1' -or
        $publication.status -cne 'pass' -or
        -not (Test-JsonIntegerAtLeast -Value $publication.cases -Minimum 16)) {
        throw 'prebuilt publication returned no complete unit receipt'
    }
    Write-Host 'PASS prebuilt package publication units'
}
catch { $failures.Add("prebuilt package publication units: $_"); Write-Host 'FAIL prebuilt package publication units' }
# Preserve the exact native child fixtures for reproducing failures.
$status = if ($failures.Count -eq 0) { 'pass' } else { 'fail' }
[ordered]@{ Schema = 'hoi4-release-manual-unit-test-v1'; Status = $status; Cases = $cases; Scratch = $root; Failures = @($failures.ToArray()) } | ConvertTo-Json -Depth 5 -Compress
if ($failures.Count -ne 0) { exit 1 }
exit 0
