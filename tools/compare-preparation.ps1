param(
    [Parameter(Mandatory = $true)][string]$SerialEvidence,
    [Parameter(Mandatory = $true)][string]$ParallelEvidence
)
$ErrorActionPreference = 'Stop'
function Read-Commit([IO.StreamReader]$Reader) {
    while ($null -ne ($line = $Reader.ReadLine())) {
        $row = $line | ConvertFrom-Json
        if ($row.recordKind -eq 'storage') { return $row }
    }
    return $null
}
$left = [IO.StreamReader]::new((Join-Path $SerialEvidence 'blocks.jsonl'))
$right = [IO.StreamReader]::new((Join-Path $ParallelEvidence 'blocks.jsonl'))
$blocks = 0
$payloads = 0L
try {
    while ($true) {
        $serial = Read-Commit $left
        $parallel = Read-Commit $right
        if ($null -eq $serial -or $null -eq $parallel) {
            if ($null -ne $serial -or $null -ne $parallel) { throw 'Committed block counts differ.' }
            break
        }
        if ($serial.ordinal -ne $parallel.ordinal -or $serial.firstHour -ne $parallel.firstHour -or $serial.lastHour -ne $parallel.lastHour -or $serial.gameHours -ne $parallel.gameHours) {
            throw "Block bounds/hours differ at ordinal $($serial.ordinal)."
        }
        $serialPayloads = [IO.StreamReader]::new((Join-Path $SerialEvidence $serial.payloadChangesFile))
        $parallelPayloads = [IO.StreamReader]::new((Join-Path $ParallelEvidence $parallel.payloadChangesFile))
        try {
            while ($true) {
                $a = $serialPayloads.ReadLine()
                $b = $parallelPayloads.ReadLine()
                if ($null -eq $a -or $null -eq $b) {
                    if ($null -ne $a -or $null -ne $b) { throw "Changed section ranges differ in block $($serial.ordinal)." }
                    break
                }
                $a = $a | ConvertFrom-Json
                $b = $b | ConvertFrom-Json
                if ($a.section -cne $b.section -or $a.range -ne $b.range -or $a.bytes -ne $b.bytes -or $a.blake3 -cne $b.blake3) {
                    throw "Payload differs: block=$($serial.ordinal) section=$($a.section) range=$($a.range)."
                }
                $payloads += 1
            }
        } finally {
            $serialPayloads.Dispose()
            $parallelPayloads.Dispose()
        }
        $blocks += 1
        Write-Output "PARITY block=$($serial.ordinal) changed_payloads=$payloads"
    }
} finally {
    $left.Dispose()
    $right.Dispose()
}
if ($blocks -eq 0) { throw 'No completed blocks to compare.' }
[pscustomobject]@{ matchingBlocks = $blocks; matchingChangedPayloads = $payloads; scope = 'All changed live section/range payload lengths and BLAKE3 digests at each commit; initial commit covers every live payload, so matching deltas preserve whole-prefix equality. Manifest UUIDs and physical names excluded.' }
