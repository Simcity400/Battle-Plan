# Canonical documents and closed ZIP roster shared by publication and receipt verification.
function Get-PortableProductPackageDocuments {
    param(
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$')][string]$Version,
        [Parameter(Mandatory = $true)][System.Collections.IDictionary]$BinaryHashes
    )

    $binaryNames = @('Battle Plan Recorder.exe', 'Battle Plan.exe', 'collector.dll')
    if ($BinaryHashes.Count -ne $binaryNames.Count) {
        throw 'portable package requires exactly three binary hashes'
    }
    $hashLines = foreach ($name in $binaryNames) {
        if (-not $BinaryHashes.Contains($name) -or
            [string]$BinaryHashes[$name] -cnotmatch '^[0-9a-f]{64}$') {
            throw "portable package binary hash is missing or invalid: $name"
        }
        "$($BinaryHashes[$name])  $name"
    }
    $readme = @"
Battle Plan $Version

This is a portable Windows package. It has no installer, service, tray process,
administrator requirement, automatic updater, or network dependency.

Run "Battle Plan Recorder.exe" once and leave its window open while you play.
It watches for supported the game sessions automatically. Use "Stop and
finish recordings" (or close the window) to stop cooperatively and verify any
active recording before exit. The window reports recording start, verified
completion, or failure and shows the exact output path. Use "Open recordings
folder" to reveal the current output directory.

This recorder supports one the game version. When Paradox patches the
game, the recorder tells you that this game version is not supported yet and
that an update to the recorder will add it. Nothing is changed in your game and
no recording is started; wait for a recorder update that adds that version.

The recorder is a passive injected observer. It does not pause the game, change game
speed, send input, move or focus the game window, or start/stop the game process.
Evidence-backed observation hooks are allowed only for values that cannot be
sampled reliably; they do not grant authority to mutate the simulation. Capture
is designed to remain passive while the player runs at speed five.

Run "Battle Plan.exe" to open and view a replay. Its picker starts in the
recorder output root when that folder exists. On first use, select the Hearts of
Iron IV installation folder and user-data folder when prompted.

Recorder data is stored under the current user's LocalAppData in
"Battle Plan Recorder" with a protected user/SYSTEM ACL. SHA256SUMS.txt detects
accidental package corruption; it is not a publisher signature.
"@
    $utf8 = [Text.UTF8Encoding]::new($false)
    $readmeBytes = $utf8.GetBytes($readme.Replace("`r`n", "`n"))
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $readmeHash = [BitConverter]::ToString($sha.ComputeHash($readmeBytes)).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
    $hashLines += "$readmeHash  README.txt"
    return [ordered]@{
        'README.txt' = $readmeBytes
        'SHA256SUMS.txt' = $utf8.GetBytes((($hashLines -join "`n") + "`n"))
    }
}

function Assert-PortableProductPackageContents {
    param(
        [Parameter(Mandatory = $true)][IO.Compression.ZipArchive]$Archive,
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][System.Collections.IDictionary]$BinaryHashes
    )

    $expectedNames = @('Battle Plan Recorder.exe', 'Battle Plan.exe', 'collector.dll', 'README.txt', 'SHA256SUMS.txt')
    if ($Archive.Entries.Count -ne $expectedNames.Count) {
        throw 'portable package does not contain exactly five entries'
    }
    foreach ($name in $expectedNames) {
        if (@($Archive.Entries | Where-Object FullName -CEQ $name).Count -ne 1) {
            throw "portable package entry is missing or duplicated: $name"
        }
    }
    $documents = Get-PortableProductPackageDocuments -Version $Version -BinaryHashes $BinaryHashes
    foreach ($name in $documents.Keys) {
        $entry = @($Archive.Entries | Where-Object FullName -CEQ $name)[0]
        $expected = [byte[]]$documents[$name]
        if ($entry.Length -ne $expected.Length) {
            throw "portable package document length mismatch: $name"
        }
        # Bound decompression by the canonical document length, including a
        # terminal read: never trust the ZIP's advertised uncompressed size.
        $stream = $entry.Open()
        try {
            $actual = [byte[]]::new($expected.Length)
            $offset = 0
            while ($offset -lt $actual.Length) {
                $read = $stream.Read($actual, $offset, $actual.Length - $offset)
                if ($read -le 0) { throw "portable package document is truncated: $name" }
                $offset += $read
            }
            if ($stream.ReadByte() -ne -1) { throw "portable package document exceeds canonical length: $name" }
            if ([Convert]::ToBase64String($actual) -cne [Convert]::ToBase64String($expected)) {
                throw "portable package document bytes mismatch: $name"
            }
        }
        finally { $stream.Dispose() }
    }
}
