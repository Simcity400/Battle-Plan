[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir '..\..\..')).Path
$builderSource = Join-Path $scriptDir 'New-PortableProductPackage.ps1'
$verifierSource = Join-Path $scriptDir 'Test-PortableProductPackageReceipt.ps1'
$neutralScriptDir = Join-Path $repoRoot 'portable_replay\native\scripts'
$attestationSource = Join-Path $neutralScriptDir 'ProductionArtifact.Attestation.ps1'
. (Join-Path $scriptDir 'PortableProduct.InteractionClosure.ps1')
. (Join-Path $scriptDir 'ProductionCapture.Promotion.ps1')
$script:PortablePackageSelfTestCases = 0

# Per-case wall time, written only when BATTLE_PLAN_SELFTEST_CASE_TIMING names a file.
# The script is one straight line of fixture construction punctuated by
# assertions, so there is no "case body" to wrap: the honest unit is the time
# from the previous case's verdict to this one's, which is the work that case
# needed done before it could be decided. Off by default, and it changes no
# case name, order or verdict.
$script:PortablePackageSelfTestTimingPath = $(if (Test-Path Env:BATTLE_PLAN_SELFTEST_CASE_TIMING) { $env:BATTLE_PLAN_SELFTEST_CASE_TIMING } else { $env:HOI4_SELFTEST_CASE_TIMING })
$script:PortablePackageSelfTestClock = $null
if ($script:PortablePackageSelfTestTimingPath) {
    $script:PortablePackageSelfTestClock = [Diagnostics.Stopwatch]::StartNew()
    [IO.File]::WriteAllText($script:PortablePackageSelfTestTimingPath, "ms`tordinal`tname`n")
}

function Write-SelfTestCaseTiming {
    param([Parameter(Mandatory = $true)][string]$Name)
    if (-not $script:PortablePackageSelfTestClock) { return }
    $ms = $script:PortablePackageSelfTestClock.Elapsed.TotalMilliseconds
    $script:PortablePackageSelfTestClock.Restart()
    [IO.File]::AppendAllText(
        $script:PortablePackageSelfTestTimingPath,
        ('{0:F1}{1}{2}{1}{3}{4}' -f $ms, "`t", $script:PortablePackageSelfTestCases, $Name, "`n")
    )
}

function Assert-Case {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Condition
    )
    if (-not $Condition) { throw "portable package self-test failed: $Name" }
    $script:PortablePackageSelfTestCases++
    Write-SelfTestCaseTiming -Name $Name
}

function Invoke-ExpectFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Body
    )
    try {
        & $Body
    }
    catch {
        $script:PortablePackageSelfTestCases++
        Write-SelfTestCaseTiming -Name $Name
        return
    }
    throw "portable package self-test expected failure: $Name"
}

function Set-U16 {
    param([byte[]]$Bytes, [int]$Offset, [uint16]$Value)
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Bytes, $Offset, 2)
}

function Set-U32 {
    param([byte[]]$Bytes, [int]$Offset, [uint32]$Value)
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Bytes, $Offset, 4)
}

function New-MinimalAmd64Pe {
    param(
        [Parameter(Mandatory = $true)][bool]$Dll,
        [Parameter(Mandatory = $true)][bool]$Gui,
        [Parameter(Mandatory = $true)][byte]$Marker,
        [string[]]$EmbeddedText = @()
    )
    $bytes = [byte[]]::new(1024)
    Set-U16 $bytes 0 0x5a4d
    Set-U32 $bytes 0x3c 0x80
    Set-U32 $bytes 0x80 0x00004550
    Set-U16 $bytes 0x84 0x8664
    Set-U16 $bytes 0x86 1
    Set-U16 $bytes 0x94 0xf0
    $characteristics = if ($Dll) { 0x2002 } else { 0x0002 }
    Set-U16 $bytes 0x96 $characteristics
    Set-U16 $bytes 0x98 0x020b
    Set-U16 $bytes 0xdc $(if ($Gui) { 2 } else { 3 })
    $textOffset = 0x200
    foreach ($text in $EmbeddedText) {
        $encoded = [Text.Encoding]::ASCII.GetBytes($text + [char]0)
        if ($textOffset + $encoded.Length -ge $bytes.Length - 1) {
            throw "minimal PE fixture has insufficient marker capacity"
        }
        [Array]::Copy($encoded, 0, $bytes, $textOffset, $encoded.Length)
        $textOffset += $encoded.Length
    }
    $bytes[$bytes.Length - 1] = $Marker
    return ,$bytes
}

function Write-Bytes {
    param([string]$Path, [byte[]]$Bytes)
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Path)) | Out-Null
    [IO.File]::WriteAllBytes($Path, $Bytes)
}

function Get-FixtureRelativePath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )
    $repo = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($repo + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "self-test fixture input is outside the repository: $full"
    }
    return $full.Substring($repo.Length + 1)
}

function Copy-FixtureInput {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Destination)) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
}

function Write-InertFixtureSource {
    param([Parameter(Mandatory = $true)][string]$Destination)
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Destination)) | Out-Null
    [IO.File]::WriteAllText(
        $Destination,
        "// synthetic production source fixture`n",
        [Text.UTF8Encoding]::new($false)
    )
}

function Get-SelfTestTreeManifest {
    # Relative path -> SHA256, over EVERY file in the tree.
    #
    # The source-tree authority's own digest covers only .rs/.c/.h/.cc/.cpp
    # under the production roots. Everything else a gate reads -- the gate's own
    # PowerShell, the manifests, the lockfile, the accepted baseline -- is
    # outside it. A copy that matched that digest and nothing else would still
    # be free to carry a rewritten gate, so proving a copy faithful needs the
    # whole tree.
    param([Parameter(Mandatory = $true)][string]$Root)

    $full = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    # Ordinal, not OrdinalIgnoreCase. Case-insensitive keys make 'Dir\File.rs'
    # and 'dir\file.rs' the same entry, so a copy that renamed a file's case
    # would compare equal, and on a case-sensitive directory two genuinely
    # different files would collapse into one and hide a missing or extra file.
    # Copy-Item preserves casing, so a faithful copy still matches exactly.
    $manifest = [Collections.Generic.Dictionary[string, string]]::new(
        [StringComparer]::Ordinal
    )
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        foreach ($entry in ([IO.DirectoryInfo]$full).EnumerateFileSystemInfos(
            '*', [IO.SearchOption]::AllDirectories)) {
            if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "self-test tree manifest reached a reparse point: $($entry.FullName)"
            }
            if (($entry.Attributes -band [IO.FileAttributes]::Directory) -ne 0) {
                continue
            }
            $relative = $entry.FullName.Substring($full.Length + 1)
            $manifest[$relative] = ([BitConverter]::ToString(
                $sha.ComputeHash([IO.File]::ReadAllBytes($entry.FullName))
            )).Replace('-', '').ToLowerInvariant()
        }
    }
    finally {
        $sha.Dispose()
    }
    return $manifest
}

function Compare-SelfTestTreeManifest {
    # Returns the empty string when the two manifests are identical, or a
    # description of the FIRST difference in a stable order. Ordered so the
    # message is reproducible rather than dependent on enumeration order.
    param(
        [Parameter(Mandatory = $true)]
        [Collections.Generic.Dictionary[string, string]]$Expected,
        [Parameter(Mandatory = $true)]
        [Collections.Generic.Dictionary[string, string]]$Actual
    )

    foreach ($relative in @($Expected.Keys | Sort-Object)) {
        if (-not $Actual.ContainsKey($relative)) {
            return "missing file '$relative'"
        }
        if ([string]$Actual[$relative] -cne [string]$Expected[$relative]) {
            return "content of '$relative' differs"
        }
    }
    foreach ($relative in @($Actual.Keys | Sort-Object)) {
        if (-not $Expected.ContainsKey($relative)) {
            return "unexpected extra file '$relative'"
        }
    }
    return ''
}

# One private-tree set, shared by every parallel segment in this suite.
#
# Measured on this repository the suite's wall time is not one hot loop: 54% of
# it is forty-odd package-builder child processes run one after another, each
# paying the same fourteen seconds of cold source-tree derivation, and 36% is
# the source-rejection roster. Both are the same shape of work -- mutate one
# file, ask a gate, put the file back -- and both are only safe to run
# concurrently on trees that have exactly one writer.
#
# So the trees are made ONCE and handed to every segment, rather than each
# segment copying the fixture again. A copy costs about a second of walking and
# hashing 966 files, which is cheap once and not cheap five times over.
#
# Nothing here is a cache: the trees exist only for the lifetime of one run,
# under one temporary container that is proved empty at teardown, and every
# gate evaluation below is a real evaluation against a real tree. What is
# reused is the COPY, never an answer.
function Get-SelfTestGateWorkerCount {
    # Gate workers are one CPU-bound process each, so this may go wide. Two
    # threads are left for the parent and the child it is waiting on.
    return [Math]::Max(2, [Math]::Min(12, [Environment]::ProcessorCount - 4))
}

function Get-SelfTestBuilderWorkerCount {
    # Builder workers are TWO processes each -- the worker and the package
    # builder it spawns -- so this is deliberately narrower than the gate pool.
    return [Math]::Max(2, [Math]::Min(8, [int]([Environment]::ProcessorCount / 2)))
}

function Stop-SelfTestWorkerProcessTree {
    # Ends the worker AND the child it is blocked on.
    #
    # Process.Kill() ends exactly one process. Every worker in this suite spends
    # nearly all of its time inside a `& powershell.exe` child running the
    # package builder or the receipt verifier, and that child holds open handles
    # on the very tree the pool is about to remove. Killing only the worker
    # leaves its child reading and writing a fixture that is being deleted
    # underneath it, which turns a timeout in one worker into unrelated failures
    # and leftover directories.
    #
    # Process.Kill($true) is the .NET Core overload and does not exist on the
    # .NET Framework this shell runs on, so the tree is ended with taskkill /T,
    # which walks the live parent-PID chain. Kill() stays as the fallback for a
    # machine where taskkill cannot be run at all.
    param([Parameter(Mandatory = $true)]$Process)

    if ($Process.HasExited) { return }
    $treeEnded = $false
    try {
        # Deliberately NOT redirected. On this shell, redirecting a native
        # command's stderr wraps each line in an ErrorRecord, which under
        # $ErrorActionPreference = 'Stop' would make a janitorial message
        # terminating -- inside a finally, that would replace the real failure.
        $null = & taskkill.exe /PID $Process.Id /T /F
        $treeEnded = ($LASTEXITCODE -eq 0)
    }
    catch {
        $treeEnded = $false
    }
    if (-not $treeEnded -and -not $Process.HasExited) {
        $Process.Kill()
    }
}

function New-SelfTestPrivateTreeSet {
    # Full copies of the fixture tree, each PROVED faithful by the parent before
    # any worker is allowed to speak from it.
    #
    # The proof has to be the parent's, file by file, over every file in the
    # tree. A digest a worker computes with code living inside its own copy
    # cannot establish that the copy is what we think it is, and the source
    # tree's own digest covers only the five source extensions under the
    # production roots -- not the gate's PowerShell, its manifests, its
    # lockfile or its accepted baseline, all of which a copy is equally free to
    # have rewritten.
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$ContainerRoot,
        [Parameter(Mandatory = $true)][int]$Count,
        [Parameter(Mandatory = $true)]
        [Collections.Generic.Dictionary[string, string]]$SharedManifest
    )

    if ($Count -lt 1) {
        throw "self-test private tree set needs at least one tree, asked for $Count"
    }
    $sourceFull = [IO.Path]::GetFullPath($SourceRoot).TrimEnd('\')
    # Directories are reproduced from the source's own enumeration, not implied
    # by the files: an empty directory is part of what the fixture is, and a
    # file-driven copy would silently drop it.
    $sourceDirectories = @(
        ([IO.DirectoryInfo]$sourceFull).EnumerateDirectories(
            '*', [IO.SearchOption]::AllDirectories) |
            ForEach-Object { $_.FullName.Substring($sourceFull.Length + 1) }
    )
    $sourceDirectorySet = [Collections.Generic.HashSet[string]]::new(
        [string[]]$sourceDirectories,
        [StringComparer]::Ordinal
    )
    $relatives = @($SharedManifest.Keys)
    $trees = [Collections.Generic.List[string]]::new()
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        for ($index = 0; $index -lt $Count; $index++) {
            # The name is kept SHORT on purpose -- these roots hold a full copy
            # of a tree that already sits under a long fixture path, and a
            # longer prefix would push the deepest files past MAX_PATH and
            # produce a partial copy.
            $treeRoot = Join-Path $ContainerRoot "t$index"
            # Refused rather than reused, and the ROOT ITSELF is inspected. A
            # junction or symlink at t<n> would be followed by every copy and by
            # every worker below, so two trees that are supposed to be private
            # could alias one directory and each would then read the other's
            # planted symbol -- and the descendant walk cannot see that, because
            # through a junction it only ever sees the target's ordinary
            # children. CreateDirectory succeeds on a directory that is already
            # there, so the attribute and emptiness of what it returned are what
            # actually decides, not the Test-Path before it.
            if (Test-Path -LiteralPath $treeRoot) {
                throw "self-test private tree root already exists: $treeRoot"
            }
            [IO.Directory]::CreateDirectory($treeRoot) | Out-Null
            $treeRootInfo = [IO.DirectoryInfo]::new($treeRoot)
            if (($treeRootInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "self-test private tree root is a reparse point: $treeRoot"
            }
            if (@($treeRootInfo.EnumerateFileSystemInfos(
                    '*', [IO.SearchOption]::TopDirectoryOnly)).Count -ne 0) {
                throw "self-test private tree root was not empty: $treeRoot"
            }
            foreach ($relative in $sourceDirectories) {
                [IO.Directory]::CreateDirectory((Join-Path $treeRoot $relative)) | Out-Null
            }
            # Copied and PROVED in one pass. The proof is a digest of the bytes
            # that landed in the copy, compared against the digest of the bytes
            # the fixture had -- not an assumption that a copy cmdlet did what
            # it said. Doing it here rather than in a second full walk is why
            # this can afford to make a dozen trees.
            foreach ($relative in $relatives) {
                $destination = Join-Path $treeRoot $relative
                [IO.File]::Copy((Join-Path $sourceFull $relative), $destination, $true)
                $observed = ([BitConverter]::ToString(
                    $sha.ComputeHash([IO.File]::ReadAllBytes($destination))
                )).Replace('-', '').ToLowerInvariant()
                if ($observed -cne [string]$SharedManifest[$relative]) {
                    throw (
                        "self-test private tree $index is not a faithful copy of " +
                        "the fixture tree: content of '$relative' differs"
                    )
                }
            }
            # The loop above can only see the entries the fixture HAD, so the
            # copy is walked once as well. This is what the full manifest walk
            # it replaced provided and hashing alone does not: an EXTRA file or
            # directory, and any reparse point, in the destination. A file the
            # copy did not write, or an entry that is really a link to somewhere
            # else, is as much a difference as a wrong byte -- and a linked tree
            # is the one difference that would let two workers share a writer.
            $copiedFiles = 0
            foreach ($entry in ([IO.DirectoryInfo]$treeRoot).EnumerateFileSystemInfos(
                '*', [IO.SearchOption]::AllDirectories)) {
                if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw (
                        "self-test private tree $index contains a reparse point: " +
                        $entry.FullName
                    )
                }
                $copiedRelative = $entry.FullName.Substring($treeRoot.Length + 1)
                if (($entry.Attributes -band [IO.FileAttributes]::Directory) -ne 0) {
                    if (-not $sourceDirectorySet.Contains($copiedRelative)) {
                        throw (
                            "self-test private tree $index has an unexpected extra " +
                            "directory: '$copiedRelative'"
                        )
                    }
                    continue
                }
                if (-not $SharedManifest.ContainsKey($copiedRelative)) {
                    throw (
                        "self-test private tree $index has an unexpected extra " +
                        "file: '$copiedRelative'"
                    )
                }
                $copiedFiles++
            }
            if ($copiedFiles -ne $relatives.Count) {
                throw (
                    "self-test private tree $index holds $copiedFiles file(s) where " +
                    "the fixture holds $($relatives.Count)"
                )
            }
            $trees.Add($treeRoot)
        }
    }
    finally {
        $sha.Dispose()
    }
    return @($trees)
}

function Assert-SelfTestPrivateTreesUnchanged {
    # Re-proved between segments, because the builder pool is the one segment
    # that WRITES to a private tree and puts it back. A revert that silently
    # lost a byte would leave every later segment measuring a tree nobody
    # authorised, and the later segments would still report verdicts.
    param(
        [Parameter(Mandatory = $true)][string[]]$Trees,
        [Parameter(Mandatory = $true)]
        [Collections.Generic.Dictionary[string, string]]$SharedManifest,
        [Parameter(Mandatory = $true)][string]$Stage
    )

    for ($index = 0; $index -lt $Trees.Count; $index++) {
        $difference = Compare-SelfTestTreeManifest `
            -Expected $SharedManifest `
            -Actual (Get-SelfTestTreeManifest -Root $Trees[$index])
        if ($difference) {
            throw (
                "self-test private tree $index no longer matches the fixture " +
                "tree after $Stage`: $difference"
            )
        }
    }
}

# Body of the builder-case worker, materialised beside the private trees at run
# time. It lives here rather than as its own file under scripts/ so it cannot
# be copied into a fixture tree and cannot drift from the pool that dispatches
# it.
#
# One case is: write these files into MY tree, run the package builder, record
# whether it refused, put the files back. That is exactly what the serial
# `Invoke-ExpectFailure { Invoke-JsonScript ... }` sites did, including how a
# refusal is recognised -- a non-zero exit, or output the parent could not
# parse as the builder's JSON. The builder is still a child process, so the
# exit-code path stays proved rather than replaced by an in-process call.
$script:PortablePackageBuilderCaseWorkerScript = @'
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RequestPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$request = Get-Content -LiteralPath $RequestPath -Raw | ConvertFrom-Json
$root = [string]$request.Root
$outPath = [string]$request.OutPath
$builder = Join-Path $root 'extractor\native\scripts\New-PortableProductPackage.ps1'
$utf8 = [Text.UTF8Encoding]::new($false)

if (-not (Test-Path -LiteralPath $builder -PathType Leaf)) {
    throw "self-test builder case worker has no package builder: $builder"
}

function Invoke-FixtureBuilderOnce {
    # The parent's Invoke-JsonScript, restated for one script: a non-zero exit
    # is a refusal, and so is output that does not parse as the builder's JSON.
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    $stdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $builder @Arguments)
    if ($LASTEXITCODE -ne 0) {
        return [pscustomobject]@{
            Succeeded = $false
            Message = "package builder exited $LASTEXITCODE"
        }
    }
    try {
        (($stdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json) | Out-Null
    }
    catch {
        return [pscustomobject]@{
            Succeeded = $false
            Message = "package builder output did not parse: $($_.Exception.Message)"
        }
    }
    return [pscustomobject]@{ Succeeded = $true; Message = '' }
}

$results = [Collections.Generic.List[object]]::new()
$restoreFailures = [Collections.Generic.List[string]]::new()

foreach ($item in @($request.Items)) {
    $name = [string]$item.Name
    $originals = [Collections.Generic.List[object]]::new()
    foreach ($write in @($item.Writes)) {
        $path = Join-Path $root ([string]$write.RelativePath)
        $existed = [IO.File]::Exists($path)
        $originals.Add([pscustomobject]@{
            Path = $path
            Existed = $existed
            Bytes = $(if ($existed) { [IO.File]::ReadAllBytes($path) } else { $null })
        })
    }
    foreach ($write in @($item.Writes)) {
        $path = Join-Path $root ([string]$write.RelativePath)
        [IO.Directory]::CreateDirectory((Split-Path -Parent $path)) | Out-Null
        if ([string]$write.Encoding -ceq 'base64') {
            [IO.File]::WriteAllBytes(
                $path,
                [Convert]::FromBase64String([string]$write.Content)
            )
        }
        else {
            [IO.File]::WriteAllText($path, [string]$write.Content, $utf8)
        }
    }
    $arguments = @(
        @($item.Arguments) | ForEach-Object { ([string]$_).Replace('@ROOT@', $root) }
    )
    $outcome = $null
    try {
        $outcome = Invoke-FixtureBuilderOnce -Arguments ([string[]]$arguments)
    }
    finally {
        # Restored, then RE-READ. A revert this worker only believes it did
        # would leave the tree mutated for every later case on it, and those
        # cases would still return verdicts.
        foreach ($original in $originals) {
            if ($original.Existed) {
                [IO.File]::WriteAllBytes($original.Path, [byte[]]$original.Bytes)
                $observed = [IO.File]::ReadAllBytes($original.Path)
                $restored = ($observed.Length -eq ([byte[]]$original.Bytes).Length)
                if ($restored) {
                    for ($byteIndex = 0; $byteIndex -lt $observed.Length; $byteIndex++) {
                        if ($observed[$byteIndex] -ne $original.Bytes[$byteIndex]) {
                            $restored = $false
                            break
                        }
                    }
                }
                if (-not $restored) {
                    $restoreFailures.Add(
                        "case '$name' could not restore $($original.Path)"
                    )
                }
            }
            else {
                if ([IO.File]::Exists($original.Path)) {
                    [IO.File]::Delete($original.Path)
                }
                if ([IO.File]::Exists($original.Path)) {
                    $restoreFailures.Add(
                        "case '$name' could not remove $($original.Path)"
                    )
                }
            }
        }
        # Anything the BUILDER published, as opposed to anything this worker
        # wrote. A published package left in the tree is a real difference from
        # the fixture, and the parent re-pins the trees against that fixture
        # once this pool is done.
        foreach ($remove in @($item.Removes)) {
            $removePath = Join-Path $root ([string]$remove)
            if ([IO.File]::Exists($removePath)) {
                [IO.File]::Delete($removePath)
            }
            if ([IO.File]::Exists($removePath)) {
                $restoreFailures.Add("case '$name' could not remove $removePath")
            }
        }
    }
    $results.Add([ordered]@{
        name = $name
        rejected = [bool](-not $outcome.Succeeded)
        message = [string]$outcome.Message
    })
}

$response = [ordered]@{
    schema = 'battle-plan.selftest_builder_case_worker.v1'
    results = @($results)
    restore_failures = @($restoreFailures)
}
[IO.File]::WriteAllText(
    $outPath,
    ($response | ConvertTo-Json -Depth 6),
    $utf8
)
'@

function Invoke-SelfTestBuilderCasePool {
    # Runs a set of package-builder cases across the private trees, one worker
    # process per tree, and returns name -> "the builder refused this".
    #
    # The guards are the ones the serial form got for free and a pool has to
    # state: worker exit status, response presence, an exact bijection between
    # dispatched and returned cases on an ORDINAL key, a boolean verdict type,
    # and every mutation proved reverted. Counting alone would let two cases
    # swap verdicts, or one case run twice while another never ran, and still
    # add up.
    param(
        [Parameter(Mandatory = $true)][string[]]$Trees,
        [Parameter(Mandatory = $true)][object[]]$Cases,
        [Parameter(Mandatory = $true)][string]$ContainerRoot,
        [Parameter(Mandatory = $true)][string]$Label,
        [int]$BudgetMs = 3600000
    )

    if ($Cases.Count -lt 1) {
        throw "self-test builder case pool '$Label' was given no cases"
    }
    if ($Trees.Count -lt 1) {
        throw "self-test builder case pool '$Label' was given no private trees"
    }
    $dispatched = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($case in $Cases) {
        if (-not $dispatched.Add([string]$case.Name)) {
            throw (
                "self-test builder case pool '$Label' has a duplicate case name: " +
                [string]$case.Name
            )
        }
    }
    $workerScriptPath = Join-Path $ContainerRoot 'builder-case-worker.ps1'
    [IO.File]::WriteAllText(
        $workerScriptPath,
        $script:PortablePackageBuilderCaseWorkerScript,
        [Text.UTF8Encoding]::new($false)
    )
    $workerCount = [Math]::Min($Trees.Count, $Cases.Count)
    $workers = [Collections.Generic.List[object]]::new()
    $cleanupProblems = [Collections.Generic.List[string]]::new()
    $verdicts = [Collections.Generic.Dictionary[string, bool]]::new(
        [StringComparer]::Ordinal
    )
    try {
        for ($workerIndex = 0; $workerIndex -lt $workerCount; $workerIndex++) {
            # Round robin, not contiguous blocks: cases are not all the same
            # cost, and a contiguous block can leave one worker with every
            # expensive case while another has already finished.
            $slice = @(
                0..($Cases.Count - 1) |
                    Where-Object { ($_ % $workerCount) -eq $workerIndex } |
                    ForEach-Object { $Cases[$_] }
            )
            if ($slice.Count -eq 0) { continue }
            $requestPath = Join-Path $ContainerRoot ("$Label-request$workerIndex.json")
            $responsePath = Join-Path $ContainerRoot ("$Label-response$workerIndex.json")
            if (Test-Path -LiteralPath $responsePath -PathType Leaf) {
                # A stale response from an earlier pool would be read as this
                # pool's answer if this worker died before writing its own.
                Remove-Item -LiteralPath $responsePath -Force -ErrorAction Stop
            }
            [IO.File]::WriteAllText(
                $requestPath,
                ([ordered]@{
                    Root = $Trees[$workerIndex]
                    OutPath = $responsePath
                    Items = @($slice)
                } | ConvertTo-Json -Depth 8),
                [Text.UTF8Encoding]::new($false)
            )
            # Quoted. Start-Process flattens -ArgumentList into one command
            # line, so an unquoted path splits on the first space in %TEMP%.
            $process = Start-Process -FilePath 'powershell.exe' -PassThru -WindowStyle Hidden `
                -ArgumentList @(
                    '-NoProfile', '-ExecutionPolicy', 'Bypass',
                    '-File', ('"{0}"' -f $workerScriptPath),
                    '-RequestPath', ('"{0}"' -f $requestPath)
                )
            $workers.Add([pscustomobject]@{
                Index = $workerIndex
                Process = $process
                ResponsePath = $responsePath
                Expected = @($slice)
            })
        }
        # Bounded: a wedged worker fails the suite with a named reason instead
        # of holding the run open until something outside it intervenes.
        $clock = [Diagnostics.Stopwatch]::StartNew()
        foreach ($worker in $workers) {
            $remaining = $BudgetMs - [int]$clock.Elapsed.TotalMilliseconds
            if ($remaining -lt 1) { $remaining = 1 }
            if (-not $worker.Process.WaitForExit($remaining)) {
                try { Stop-SelfTestWorkerProcessTree -Process $worker.Process } catch { }
                throw (
                    "self-test builder case worker $($worker.Index) of '$Label' did " +
                    "not finish within the $BudgetMs ms budget"
                )
            }
            # The bool overload can return before the exit-code cache is filled.
            $worker.Process.WaitForExit()
            if ($worker.Process.ExitCode -ne 0) {
                throw (
                    "self-test builder case worker $($worker.Index) of '$Label' " +
                    "exited $($worker.Process.ExitCode)"
                )
            }
        }
        foreach ($worker in $workers) {
            if (-not (Test-Path -LiteralPath $worker.ResponsePath -PathType Leaf)) {
                throw (
                    "self-test builder case worker $($worker.Index) of '$Label' " +
                    'produced no response'
                )
            }
            $response = Get-Content -LiteralPath $worker.ResponsePath -Raw |
                ConvertFrom-Json
            if (@($response.restore_failures).Count -ne 0) {
                throw (
                    "self-test builder case worker $($worker.Index) of '$Label' left " +
                    'its tree mutated: ' + (@($response.restore_failures) -join '; ')
                )
            }
            $outstanding = [Collections.Generic.HashSet[string]]::new(
                [StringComparer]::Ordinal
            )
            foreach ($case in @($worker.Expected)) {
                [void]$outstanding.Add([string]$case.Name)
            }
            foreach ($row in @($response.results)) {
                $name = [string]$row.name
                # Remove returns false for a name that was never dispatched AND
                # for one already consumed, so this is the duplicate check too.
                if (-not $outstanding.Remove($name)) {
                    throw (
                        "self-test builder case worker $($worker.Index) of '$Label' " +
                        "returned a verdict for unrequested or duplicated case '$name'"
                    )
                }
                if ($row.rejected -isnot [bool]) {
                    throw (
                        "self-test builder case worker $($worker.Index) of '$Label' " +
                        "returned a non-boolean verdict for '$name'"
                    )
                }
                $verdicts[$name] = [bool]$row.rejected
            }
            if ($outstanding.Count -ne 0) {
                throw (
                    "self-test builder case worker $($worker.Index) of '$Label' " +
                    "returned no verdict for $($outstanding.Count) dispatched case(s)"
                )
            }
        }
        if ($verdicts.Count -ne $Cases.Count) {
            throw (
                "self-test builder case pool '$Label' dispatched $($Cases.Count) " +
                "case(s) and collected $($verdicts.Count) verdict(s)"
            )
        }
    }
    finally {
        # Cleanup problems are RECORDED here, not thrown: throwing from a
        # finally would replace the real failure with a janitorial one. They are
        # raised after the block instead, where they can only be reached if the
        # main path succeeded. The private trees are NOT removed here -- they
        # outlive this pool and are torn down once, with the container.
        foreach ($worker in $workers) {
            try {
                if (-not $worker.Process.HasExited) {
                    Stop-SelfTestWorkerProcessTree -Process $worker.Process
                }
            }
            catch {
                $cleanupProblems.Add(
                    "worker $($worker.Index) could not be terminated: $($_.Exception.Message)"
                )
            }
        }
        foreach ($worker in $workers) {
            try {
                if (-not $worker.Process.WaitForExit(30000)) {
                    $cleanupProblems.Add(
                        "worker $($worker.Index) was still running 30000 ms after being terminated"
                    )
                }
            }
            catch {
                $cleanupProblems.Add(
                    "worker $($worker.Index) could not be waited on: $($_.Exception.Message)"
                )
            }
        }
        # Warned here as well as thrown below, because those are different
        # paths: if the block above threw, that exception propagates the moment
        # this finally returns and the check below is never reached.
        if ($cleanupProblems.Count -ne 0) {
            Write-Warning (
                "self-test builder case pool '$Label' cleanup failed: " +
                ($cleanupProblems -join '; ')
            ) -WarningAction Continue
        }
    }
    if ($cleanupProblems.Count -ne 0) {
        throw (
            "self-test builder case pool '$Label' cleanup failed: " +
            ($cleanupProblems -join '; ')
        )
    }
    # Comma on purpose: the dictionary is written as the single element of an
    # array, which the pipeline unrolls back to the dictionary. Without it the
    # caller is at the mercy of how PowerShell decides to enumerate the type.
    return ,$verdicts
}

function New-SelfTestBuilderCase {
    # One pooled builder case. Writes are relative to the private tree, and
    # every argument that names a path is written with the @ROOT@ token so the
    # worker rebases it onto ITS tree -- an absolute path from the parent's
    # fixture would send every worker back to the one shared tree and make the
    # whole pool unsound.
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Writes,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [AllowEmptyCollection()][string[]]$Removes = @()
    )

    foreach ($argument in $Arguments) {
        if ([IO.Path]::IsPathRooted([string]$argument)) {
            throw (
                "self-test builder case '$Name' carries an absolute path argument " +
                "instead of an @ROOT@-relative one: $argument"
            )
        }
    }
    return [pscustomobject]@{
        Name = $Name
        Writes = @($Writes)
        Arguments = @($Arguments)
        Removes = @($Removes)
    }
}

function New-SelfTestBuilderCaseTextWrite {
    param(
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Content
    )
    return [pscustomobject]@{
        RelativePath = $RelativePath
        Encoding = 'text'
        Content = $Content
    }
}

function New-SelfTestBuilderCaseByteWrite {
    param(
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][byte[]]$Bytes
    )
    return [pscustomobject]@{
        RelativePath = $RelativePath
        Encoding = 'base64'
        Content = [Convert]::ToBase64String($Bytes)
    }
}

# Body of the receipt-case worker, materialised beside the fixture at run time.
# It lives here rather than as its own file under scripts/ so it cannot be
# copied into a fixture tree and cannot drift from the pool that dispatches it.
#
# One case is: put my own corruption in my own file, ask the receipt verifier
# (or the package builder), record whether it refused, take my file away again.
# That is exactly what the serial sites did, including how a refusal is
# recognised -- a non-zero exit, or output the parent could not parse as the
# script's JSON. Both scripts are still child processes, so the exit-code path
# stays proved rather than replaced by an in-process call.
$script:PortablePackageReceiptCaseWorkerScript = @'
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RequestPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$request = Get-Content -LiteralPath $RequestPath -Raw | ConvertFrom-Json
$root = [string]$request.Root
$outPath = [string]$request.OutPath
$releaseRoot = Join-Path $root '.cargo-target\release'
$canonicalReceipt = Join-Path $releaseRoot 'hoi4-portable-package-receipt.json'
$verifier = Join-Path $root 'extractor\native\scripts\Test-PortableProductPackageReceipt.ps1'
$builder = Join-Path $root 'extractor\native\scripts\New-PortableProductPackage.ps1'
$utf8 = [Text.UTF8Encoding]::new($false)

foreach ($required in @($verifier, $builder, $canonicalReceipt)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "self-test receipt case worker is missing an input: $required"
    }
}

function Invoke-FixtureScriptOnce {
    # The parent's Invoke-JsonScript, restated for one script: a non-zero exit
    # is a refusal, and so is output that does not parse as the script's JSON.
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $stdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Path @Arguments)
    if ($LASTEXITCODE -ne 0) {
        return [pscustomobject]@{
            Accepted = $false
            Message = "fixture script exited $LASTEXITCODE"
            Output = $null
        }
    }
    $parsed = $null
    try {
        $parsed = (($stdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
    }
    catch {
        return [pscustomobject]@{
            Accepted = $false
            Message = "fixture script output did not parse: $($_.Exception.Message)"
            Output = $null
        }
    }
    return [pscustomobject]@{ Accepted = $true; Message = ''; Output = $parsed }
}

function Set-SelfTestReceiptProperty {
    param(
        [Parameter(Mandatory = $true)]$Receipt,
        [Parameter(Mandatory = $true)][string]$Property,
        [Parameter(Mandatory = $true)][string]$ValueKind,
        $Value
    )

    $segments = @($Property -split '\.')
    $node = $Receipt
    for ($index = 0; $index -lt $segments.Count - 1; $index++) {
        # Strict mode makes a segment this receipt does not have a terminating
        # error here, rather than a mutation that silently plants nothing. A
        # rejection case that stopped planting its corruption is the one way
        # this family could keep reporting refusals that mean nothing.
        $node = $node.($segments[$index])
    }
    $leaf = $segments[$segments.Count - 1]
    # Reading it first is the same guard for the LAST segment, which the loop
    # above never touches.
    [void]$node.$leaf
    if ($ValueKind -ceq 'string') { $node.$leaf = [string]$Value }
    elseif ($ValueKind -ceq 'bool') { $node.$leaf = [bool]$Value }
    elseif ($ValueKind -ceq 'releasePath') {
        $node.$leaf = [IO.Path]::GetFullPath((Join-Path $releaseRoot ([string]$Value)))
    }
    elseif ($ValueKind -ceq 'emptyArray') {
        # Assigned from a variable, not inline: an @() produced by a switch or
        # a pipeline is dropped to $null on its way out, and a receipt whose
        # marker roster went missing instead of empty is a different mutation
        # from the one this case exists to plant.
        $emptyRoster = @()
        $node.$leaf = $emptyRoster
    }
    else { throw "self-test receipt mutation has an unknown value kind: $ValueKind" }
}

$results = [Collections.Generic.List[object]]::new()
$cleanupFailures = [Collections.Generic.List[string]]::new()
$canonicalText = [IO.File]::ReadAllText($canonicalReceipt)

foreach ($item in @($request.Items)) {
    $name = [string]$item.Name
    $kind = [string]$item.Kind
    $outcome = $null
    $written = [Collections.Generic.List[string]]::new()
    try {
        if ($kind -ceq 'builder') {
            $arguments = @(
                @($item.Arguments) | ForEach-Object { ([string]$_).Replace('@ROOT@', $root) }
            )
            $outcome = Invoke-FixtureScriptOnce -Path $builder -Arguments ([string[]]$arguments)
        }
        elseif ($kind -ceq 'verifier_canonical') {
            $outcome = Invoke-FixtureScriptOnce -Path $verifier -Arguments @(
                '-RepoRoot', $root, '-ReceiptPath', $canonicalReceipt
            )
        }
        elseif ($kind -ceq 'verifier_candidate') {
            # Both sides of the comparison are round-trips of the same text, so
            # the difference this reports is the mutation and nothing else --
            # not ConvertTo-Json's formatting.
            $baselineText = ($canonicalText | ConvertFrom-Json | ConvertTo-Json -Depth 12)
            $mutated = $canonicalText | ConvertFrom-Json
            foreach ($mutation in @($item.Mutations)) {
                Set-SelfTestReceiptProperty `
                    -Receipt $mutated `
                    -Property ([string]$mutation.Property) `
                    -ValueKind ([string]$mutation.ValueKind) `
                    -Value $mutation.Value
            }
            $mutatedText = ($mutated | ConvertTo-Json -Depth 12)
            if (@($item.Mutations).Count -eq 0) {
                if ($mutatedText -cne $baselineText) {
                    throw (
                        "self-test receipt control '$name' changed the receipt it " +
                        'is supposed to leave alone'
                    )
                }
            }
            elseif ($mutatedText -ceq $baselineText) {
                throw (
                    "self-test receipt case '$name' planted a mutation that " +
                    'changed nothing'
                )
            }
            if ($null -ne $item.ZipCopy) {
                $copySource = Join-Path $releaseRoot ([string]$item.ZipCopy.SourceName)
                $copyDestination =
                    Join-Path $releaseRoot ([string]$item.ZipCopy.DestinationName)
                $copyBytes = [byte[]][IO.File]::ReadAllBytes($copySource)
                if ($copyBytes.Length -lt 1) {
                    throw "self-test receipt case '$name' has an empty package to copy"
                }
                if ($item.ZipCopy.TamperLastByte) {
                    $copyBytes[$copyBytes.Length - 1] =
                        $copyBytes[$copyBytes.Length - 1] -bxor 0xff
                }
                [IO.File]::WriteAllBytes($copyDestination, $copyBytes)
                $written.Add($copyDestination)
                # Proved planted, not assumed. A tamper that produced the same
                # bytes would leave this case asking the verifier to reject a
                # package that is byte-identical to its receipt.
                $observedCopy = [IO.File]::ReadAllBytes($copyDestination)
                $sourceBytes = [IO.File]::ReadAllBytes($copySource)
                $copyDiffers = ($observedCopy.Length -ne $sourceBytes.Length)
                if (-not $copyDiffers) {
                    for ($byteIndex = 0; $byteIndex -lt $observedCopy.Length; $byteIndex++) {
                        if ($observedCopy[$byteIndex] -ne $sourceBytes[$byteIndex]) {
                            $copyDiffers = $true
                            break
                        }
                    }
                }
                if ([bool]$item.ZipCopy.TamperLastByte -ne $copyDiffers) {
                    throw (
                        "self-test receipt case '$name' did not plant the package-byte " +
                        'difference it declares'
                    )
                }
            }
            $candidate = Join-Path $releaseRoot ([string]$item.CandidateName)
            if ([IO.File]::Exists($candidate)) {
                throw "self-test receipt candidate already exists: $candidate"
            }
            [IO.File]::WriteAllText($candidate, $mutatedText, $utf8)
            $written.Add($candidate)
            $outcome = Invoke-FixtureScriptOnce -Path $verifier -Arguments @(
                '-RepoRoot', $root, '-ReceiptPath', $candidate, '-AllowStagedReceipt'
            )
        }
        else {
            throw "self-test receipt case '$name' has an unknown kind: $kind"
        }
    }
    finally {
        # Every file this case put into the shared release directory is taken
        # back out and proved gone. A candidate left behind is a receipt at the
        # canonical location that nobody published.
        #
        # Each removal is guarded. An exception raised here would replace
        # whatever failure is already propagating out of the try above, abandon
        # the remaining files, and never reach cleanup_failures -- so the parent
        # would be told a janitorial story instead of the real one and would not
        # learn that the release directory is still dirty.
        foreach ($path in $written) {
            try {
                if ([IO.File]::Exists($path)) { [IO.File]::Delete($path) }
                if ([IO.File]::Exists($path)) {
                    $cleanupFailures.Add("case '$name' could not remove $path")
                }
            }
            catch {
                $cleanupFailures.Add(
                    "case '$name' could not remove $path ($($_.Exception.Message))"
                )
            }
        }
    }
    $status = ''
    $packageSha256 = ''
    if ($outcome.Accepted -and $null -ne $outcome.Output) {
        # Both reads are guarded, because the two scripts this worker runs emit
        # different documents. The package builder's JSON has neither `status`
        # nor `package_sha256`, and under StrictMode reading a property a
        # PSObject does not have is terminating -- so an unguarded read here
        # would kill the worker whenever a BUILDER case was accepted. That is
        # precisely the case where the builder stopped refusing something it
        # must refuse, and the parent would then report "worker N exited 1"
        # instead of naming the case that regressed.
        if ($null -ne $outcome.Output.PSObject.Properties['status']) {
            $status = [string]$outcome.Output.status
        }
        if ($null -ne $outcome.Output.PSObject.Properties['package_sha256']) {
            $packageSha256 = [string]$outcome.Output.package_sha256
        }
    }
    $results.Add([ordered]@{
        name = $name
        accepted = [bool]$outcome.Accepted
        message = [string]$outcome.Message
        output_status = $status
        output_package_sha256 = $packageSha256
    })
}

$response = [ordered]@{
    schema = 'battle-plan.selftest_receipt_case_worker.v1'
    results = @($results)
    cleanup_failures = @($cleanupFailures)
}
[IO.File]::WriteAllText(
    $outPath,
    ($response | ConvertTo-Json -Depth 6),
    $utf8
)
'@

function Invoke-SelfTestReceiptCasePool {
    # Runs the receipt-verifier family across worker processes on ONE fixture
    # root, and returns name -> the row that case produced.
    #
    # Unlike the builder pool above there are no private trees here, because
    # these cases do not need one: a receipt case's corruption lives entirely in
    # its OWN candidate file (and, for the package-bytes case, its own copy of
    # the package), and a builder case's output is its own uniquely named ZIP.
    # Nothing any case writes is read by another, and neither the verifier nor
    # the builder writes anything else under the root. What made these serial
    # was one shared receipt file, not shared state in general.
    #
    # The guards are the ones the serial form got for free and a pool has to
    # state: worker exit status, response presence, an exact bijection between
    # dispatched and returned cases on an ORDINAL key, a boolean verdict type,
    # and every planted file proved removed.
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][object[]]$Cases,
        [Parameter(Mandatory = $true)][string]$ContainerRoot,
        [Parameter(Mandatory = $true)][string]$Label,
        [int]$BudgetMs = 3600000
    )

    if ($Cases.Count -lt 1) {
        throw "self-test receipt case pool '$Label' was given no cases"
    }
    $dispatched = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($case in $Cases) {
        if (-not $dispatched.Add([string]$case.Name)) {
            throw (
                "self-test receipt case pool '$Label' has a duplicate case name: " +
                [string]$case.Name
            )
        }
    }
    $candidateNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($case in $Cases) {
        $candidateName = [string]$case.CandidateName
        if ([string]::IsNullOrEmpty($candidateName)) { continue }
        # Two cases sharing a candidate name would be two cases writing one
        # file, which is exactly the serialisation this pool exists to remove.
        if (-not $candidateNames.Add($candidateName)) {
            throw (
                "self-test receipt case pool '$Label' reuses a staged candidate " +
                "name: $candidateName"
            )
        }
        # The verifier's staged-receipt rule is the guard being relied on, so it
        # is restated where the names are minted: anything else is refused by
        # the verifier for its NAME, and every case in the pool would then be
        # rejected for a reason that has nothing to do with its corruption.
        if ($candidateName -cnotmatch
            '^hoi4-portable-package-receipt\.json\.[0-9a-f]{32}\.candidate$') {
            throw (
                "self-test receipt case pool '$Label' minted a noncanonical staged " +
                "candidate name: $candidateName"
            )
        }
    }
    $workerScriptPath = Join-Path $ContainerRoot 'receipt-case-worker.ps1'
    [IO.File]::WriteAllText(
        $workerScriptPath,
        $script:PortablePackageReceiptCaseWorkerScript,
        [Text.UTF8Encoding]::new($false)
    )
    $workerCount = [Math]::Min((Get-SelfTestGateWorkerCount), $Cases.Count)
    $workers = [Collections.Generic.List[object]]::new()
    $cleanupProblems = [Collections.Generic.List[string]]::new()
    $rows = [Collections.Generic.Dictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    try {
        for ($workerIndex = 0; $workerIndex -lt $workerCount; $workerIndex++) {
            # Round robin, not contiguous blocks: two of these cases are decided
            # by the verifier's first few lines and the rest re-derive the whole
            # source tree, so a contiguous block can leave one worker with every
            # expensive case while another has already finished.
            $slice = @(
                0..($Cases.Count - 1) |
                    Where-Object { ($_ % $workerCount) -eq $workerIndex } |
                    ForEach-Object { $Cases[$_] }
            )
            if ($slice.Count -eq 0) { continue }
            $requestPath = Join-Path $ContainerRoot ("$Label-request$workerIndex.json")
            $responsePath = Join-Path $ContainerRoot ("$Label-response$workerIndex.json")
            if (Test-Path -LiteralPath $responsePath -PathType Leaf) {
                # A stale response from an earlier pool would be read as this
                # pool's answer if this worker died before writing its own.
                Remove-Item -LiteralPath $responsePath -Force -ErrorAction Stop
            }
            [IO.File]::WriteAllText(
                $requestPath,
                ([ordered]@{
                    Root = $Root
                    OutPath = $responsePath
                    Items = @($slice)
                } | ConvertTo-Json -Depth 8),
                [Text.UTF8Encoding]::new($false)
            )
            # Quoted. Start-Process flattens -ArgumentList into one command
            # line, so an unquoted path splits on the first space in %TEMP%.
            $process = Start-Process -FilePath 'powershell.exe' -PassThru -WindowStyle Hidden `
                -ArgumentList @(
                    '-NoProfile', '-ExecutionPolicy', 'Bypass',
                    '-File', ('"{0}"' -f $workerScriptPath),
                    '-RequestPath', ('"{0}"' -f $requestPath)
                )
            $workers.Add([pscustomobject]@{
                Index = $workerIndex
                Process = $process
                ResponsePath = $responsePath
                Expected = @($slice)
            })
        }
        # Bounded: a wedged worker fails the suite with a named reason instead
        # of holding the run open until something outside it intervenes.
        $clock = [Diagnostics.Stopwatch]::StartNew()
        foreach ($worker in $workers) {
            $remaining = $BudgetMs - [int]$clock.Elapsed.TotalMilliseconds
            if ($remaining -lt 1) { $remaining = 1 }
            if (-not $worker.Process.WaitForExit($remaining)) {
                try { Stop-SelfTestWorkerProcessTree -Process $worker.Process } catch { }
                throw (
                    "self-test receipt case worker $($worker.Index) of '$Label' did " +
                    "not finish within the $BudgetMs ms budget"
                )
            }
            # The bool overload can return before the exit-code cache is filled.
            $worker.Process.WaitForExit()
            if ($worker.Process.ExitCode -ne 0) {
                throw (
                    "self-test receipt case worker $($worker.Index) of '$Label' " +
                    "exited $($worker.Process.ExitCode)"
                )
            }
        }
        foreach ($worker in $workers) {
            if (-not (Test-Path -LiteralPath $worker.ResponsePath -PathType Leaf)) {
                throw (
                    "self-test receipt case worker $($worker.Index) of '$Label' " +
                    'produced no response'
                )
            }
            $response = Get-Content -LiteralPath $worker.ResponsePath -Raw |
                ConvertFrom-Json
            if (@($response.cleanup_failures).Count -ne 0) {
                throw (
                    "self-test receipt case worker $($worker.Index) of '$Label' left " +
                    'the release directory mutated: ' +
                    (@($response.cleanup_failures) -join '; ')
                )
            }
            $outstanding = [Collections.Generic.HashSet[string]]::new(
                [StringComparer]::Ordinal
            )
            foreach ($case in @($worker.Expected)) {
                [void]$outstanding.Add([string]$case.Name)
            }
            foreach ($row in @($response.results)) {
                $name = [string]$row.name
                # Remove returns false for a name that was never dispatched AND
                # for one already consumed, so this is the duplicate check too.
                if (-not $outstanding.Remove($name)) {
                    throw (
                        "self-test receipt case worker $($worker.Index) of '$Label' " +
                        "returned a verdict for unrequested or duplicated case '$name'"
                    )
                }
                if ($row.accepted -isnot [bool]) {
                    throw (
                        "self-test receipt case worker $($worker.Index) of '$Label' " +
                        "returned a non-boolean verdict for '$name'"
                    )
                }
                $rows[$name] = $row
            }
            if ($outstanding.Count -ne 0) {
                throw (
                    "self-test receipt case worker $($worker.Index) of '$Label' " +
                    "returned no verdict for $($outstanding.Count) dispatched case(s)"
                )
            }
        }
        if ($rows.Count -ne $Cases.Count) {
            throw (
                "self-test receipt case pool '$Label' dispatched $($Cases.Count) " +
                "case(s) and collected $($rows.Count) verdict(s)"
            )
        }
    }
    finally {
        # Cleanup problems are RECORDED here, not thrown: throwing from a
        # finally would replace the real failure with a janitorial one. They are
        # raised after the block instead, where they can only be reached if the
        # main path succeeded.
        foreach ($worker in $workers) {
            try {
                if (-not $worker.Process.HasExited) {
                    Stop-SelfTestWorkerProcessTree -Process $worker.Process
                }
            }
            catch {
                $cleanupProblems.Add(
                    "worker $($worker.Index) could not be terminated: $($_.Exception.Message)"
                )
            }
        }
        foreach ($worker in $workers) {
            try {
                if (-not $worker.Process.WaitForExit(30000)) {
                    $cleanupProblems.Add(
                        "worker $($worker.Index) was still running 30000 ms after being terminated"
                    )
                }
            }
            catch {
                $cleanupProblems.Add(
                    "worker $($worker.Index) could not be waited on: $($_.Exception.Message)"
                )
            }
        }
        # Warned here as well as thrown below, because those are different
        # paths: if the block above threw, that exception propagates the moment
        # this finally returns and the check below is never reached.
        if ($cleanupProblems.Count -ne 0) {
            Write-Warning (
                "self-test receipt case pool '$Label' cleanup failed: " +
                ($cleanupProblems -join '; ')
            ) -WarningAction Continue
        }
    }
    if ($cleanupProblems.Count -ne 0) {
        throw (
            "self-test receipt case pool '$Label' cleanup failed: " +
            ($cleanupProblems -join '; ')
        )
    }
    # Comma on purpose: the dictionary is written as the single element of an
    # array, which the pipeline unrolls back to the dictionary. Without it the
    # caller is at the mercy of how PowerShell decides to enumerate the type.
    return ,$rows
}

function New-SelfTestReceiptCandidateName {
    # The one shape the verifier's staged-receipt rule admits. Minted per case
    # so no two cases can name the same file.
    return 'hoi4-portable-package-receipt.json.{0}.candidate' -f
        [guid]::NewGuid().ToString('N')
}

function New-SelfTestReceiptMutation {
    param(
        [Parameter(Mandatory = $true)][string]$Property,
        [Parameter(Mandatory = $true)]
        [ValidateSet('string', 'bool', 'releasePath', 'emptyArray')]
        [string]$ValueKind,
        $Value = $null
    )
    return [pscustomobject]@{
        Property = $Property
        ValueKind = $ValueKind
        Value = $Value
    }
}

function New-SelfTestReceiptCase {
    # One pooled receipt case. Builder arguments carry the @ROOT@ token so the
    # worker rebases them onto the root it was given, exactly as the builder
    # pool above requires.
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)]
        [ValidateSet('verifier_canonical', 'verifier_candidate', 'builder')]
        [string]$Kind,
        [AllowEmptyCollection()][object[]]$Mutations = @(),
        [AllowEmptyCollection()][string[]]$Arguments = @(),
        $ZipCopy = $null
    )

    foreach ($argument in $Arguments) {
        if ([IO.Path]::IsPathRooted([string]$argument)) {
            throw (
                "self-test receipt case '$Name' carries an absolute path argument " +
                "instead of an @ROOT@-relative one: $argument"
            )
        }
    }
    if ($Kind -cne 'verifier_candidate' -and
        (@($Mutations).Count -ne 0 -or $null -ne $ZipCopy)) {
        throw (
            "self-test receipt case '$Name' plants a receipt mutation but is not a " +
            'staged-candidate case'
        )
    }
    if ($Kind -cne 'builder' -and @($Arguments).Count -ne 0) {
        throw (
            "self-test receipt case '$Name' carries builder arguments but is not a " +
            'builder case'
        )
    }
    return [pscustomobject]@{
        Name = $Name
        Kind = $Kind
        CandidateName = $(
            if ($Kind -ceq 'verifier_candidate') { New-SelfTestReceiptCandidateName }
            else { '' }
        )
        Mutations = @($Mutations)
        Arguments = @($Arguments)
        ZipCopy = $ZipCopy
    }
}

# Body of the source-rejection worker, materialised beside the worker trees at
# run time. It lives here rather than as its own file under scripts/ so it
# cannot be copied into a fixture tree and cannot drift from the loop that
# dispatches it.
$script:PortablePackageSourceRejectionWorkerScript = @'
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RequestPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$request = Get-Content -LiteralPath $RequestPath -Raw | ConvertFrom-Json
$root = [string]$request.Root
$outPath = [string]$request.OutPath

. (Join-Path $root 'extractor\native\scripts\PortableProduct.InteractionClosure.ps1')

$response = [ordered]@{
    schema = 'battle-plan.selftest_source_rejection_worker.v1'
    clean_tree_sha256 = ''
    clean_tree_accepted = $false
    clean_tree_message = ''
    results = @()
}

# The control, on THIS worker's own copy. The gate has to accept the unmutated
# private tree, or every rejection below could be the copy being broken rather
# than the planted symbol being caught. This is necessary but NOT sufficient on
# its own -- it is computed by code living inside the copy -- so the parent
# separately proves the copy is faithful before trusting anything here.
try {
    $cleanAuthority = Get-ProductionInteractionSourceTreeAuthority -RepoRoot $root
    $response.clean_tree_sha256 = [string]$cleanAuthority.sha256
    Assert-ProductionInteractionSourceTreeClosure `
        -RepoRoot $root `
        -Authority $cleanAuthority
    $response.clean_tree_accepted = $true
}
catch {
    $response.clean_tree_message = [string]$_.Exception.Message
}

if ($response.clean_tree_accepted) {
    $originals = @{}
    $results = [Collections.Generic.List[object]]::new()
    foreach ($item in @($request.Items)) {
        $relative = [string]$item.RelativePath
        $path = Join-Path $root $relative
        if (-not $originals.ContainsKey($relative)) {
            $originals[$relative] = [IO.File]::ReadAllText($path)
        }
        $original = [string]$originals[$relative]
        $symbol = [string]$item.Symbol
        [IO.File]::WriteAllText(
            $path,
            ($original + "`nunsafe extern `"system`" { fn $symbol(); }`n"),
            [Text.UTF8Encoding]::new($false)
        )
        $rejected = $false
        $message = ''
        try {
            Assert-ProductionInteractionSourceTreeClosure `
                -RepoRoot $root `
                -Authority (Get-ProductionInteractionSourceTreeAuthority -RepoRoot $root)
        }
        catch {
            $rejected = $true
            $message = [string]$_.Exception.Message
        }
        finally {
            [IO.File]::WriteAllText(
                $path,
                $original,
                [Text.UTF8Encoding]::new($false)
            )
        }
        # The whole dispatched tuple is echoed back, not just the role. The
        # parent matches these against exactly what it sent, so a dropped,
        # duplicated or misattributed verdict cannot hide behind a correct
        # per-role count.
        $results.Add([ordered]@{
            role = [string]$item.Role
            relative_path = $relative
            symbol = $symbol
            rejected = [bool]$rejected
            message = $message
        })
    }
    $response.results = @($results)
}

[IO.File]::WriteAllText(
    $outPath,
    ($response | ConvertTo-Json -Depth 6),
    [Text.UTF8Encoding]::new($false)
)
'@

function Invoke-JsonScript {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    $childClock = $null
    if ($script:PortablePackageSelfTestClock) { $childClock = [Diagnostics.Stopwatch]::StartNew() }
    $stdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Path @Arguments)
    if ($childClock) {
        [IO.File]::AppendAllText(
            $script:PortablePackageSelfTestTimingPath,
            ('{0:F1}{1}child{1}{2}{3}' -f $childClock.Elapsed.TotalMilliseconds, "`t",
                [IO.Path]::GetFileName($Path), "`n")
        )
    }
    if ($LASTEXITCODE -ne 0) {
        throw "fixture script exited $LASTEXITCODE`: $Path"
    }
    return (($stdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
}

# The builder's binary enforcer, lifted out of the fixture builder by AST and
# made callable on its own.
#
# The roster loops below used to spend one whole package build per forbidden
# symbol, and a package build re-derives the entire production source tree -
# about fourteen seconds - before it ever looks at the binary. Nothing in that
# tree changes between two symbols of the same roster: the mutation is a byte
# in a release artifact. What each of those cases establishes is that the
# builder's enforcer rejects that symbol, so that is what is driven directly.
#
# Lifting the definition rather than restating it is the point. There is still
# exactly one enforcer, in the builder, and if it is edited this probe changes
# with it; a hand-written second copy here would agree with itself forever.
# The builder is still run end to end once per role, so the wiring from
# "builder invoked" to "enforcer rejected" stays proved rather than assumed.
# This is the same lifting technique Test-ProductionInteractionClosure.ps1
# already uses on the same two functions.
function New-FixtureInteractionEnforcerProbe {
    param(
        [Parameter(Mandatory = $true)][string]$BuilderPath,
        [Parameter(Mandatory = $true)][string]$HelperPath
    )

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile(
        $BuilderPath,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if (@($parseErrors).Count -ne 0) {
        throw "fixture package builder source does not parse: $BuilderPath"
    }
    $lifted = [System.Collections.Generic.List[string]]::new()
    foreach ($name in @('Find-AsciiByteSequence', 'Assert-ProductionInteractionClosure')) {
        $definitions = @($ast.FindAll(
            {
                param($node)
                $node -is [System.Management.Automation.Language.FunctionDefinitionAst]
            },
            $true
        ) | Where-Object { [string]$_.Name -ceq $name })
        if ($definitions.Count -ne 1) {
            throw "fixture package builder does not define exactly one $name`: $BuilderPath"
        }
        $lifted.Add("function $name $($definitions[0].Body.Extent.Text)")
    }
    # The path is pasted into generated source, so its quotes are escaped for
    # the single-quoted literal it lands in. %TEMP% is whatever the caller's
    # environment says it is, and a directory named O'Brien would otherwise
    # terminate the string and fail to parse.
    $quotedHelperPath = "'" + $HelperPath.Replace("'", "''") + "'"
    $moduleText = @(
        ". $quotedHelperPath"
        $lifted
        @'
function Invoke-FixtureInteractionEnforcer {
    param(
        [byte[]]$Bytes,
        [string]$Name,
        [string[]]$RequiredMarkers,
        [uint16]$ExpectedPinnedHookCount,
        [string]$PinnedHookContractSourceSha256,
        [string[]]$PermittedSelfUiCapabilities,
        [string[]]$PermittedSelfWorkerCapabilities,
        [string[]]$PermittedInjectionCapabilities
    )
    return Assert-ProductionInteractionClosure `
        -Bytes $Bytes `
        -Name $Name `
        -RequiredMarkers $RequiredMarkers `
        -ExpectedPinnedHookCount $ExpectedPinnedHookCount `
        -PinnedHookContractSourceSha256 $PinnedHookContractSourceSha256 `
        -PermittedSelfUiCapabilities $PermittedSelfUiCapabilities `
        -PermittedSelfWorkerCapabilities $PermittedSelfWorkerCapabilities `
        -PermittedInjectionCapabilities $PermittedInjectionCapabilities
}
'@
    ) -join [Environment]::NewLine
    return (New-Module -ScriptBlock ([scriptblock]::Create($moduleText)) -AsCustomObject)
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("hoi4-portable-package-selftest-{0}" -f [guid]::NewGuid().ToString('N'))
try {
    $fixtureScripts = Join-Path $root 'extractor\native\scripts'
    # The attestation helper and the two files it dot-sources are neutral,
    # shared build machinery under portable_replay\native\scripts, so the
    # fixture repository has to reproduce that layout rather than putting every
    # script under extractor\native\scripts. Getting this wrong is not silent:
    # the helper resolves its own siblings through $PSScriptRoot.
    $fixtureNeutralScripts = Join-Path $root 'portable_replay\native\scripts'
    $releaseRoot = Join-Path $root '.cargo-target\release'
    [IO.Directory]::CreateDirectory($fixtureScripts) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $root 'portable_replay\native')) | Out-Null
    [IO.Directory]::CreateDirectory($fixtureNeutralScripts) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $root 'replay_interface\native')) | Out-Null
    [IO.Directory]::CreateDirectory($releaseRoot) | Out-Null

    foreach ($name in @(
        'PortableProduct.InteractionClosure.ps1',
        'ProductionCapture.Promotion.ps1',
        'New-PortableProductPackage.ps1',
        'Test-PortableProductPackageReceipt.ps1',
        'Read-Hoi4GameState.ps1',
        'Test-PassiveCapturePolicy.ps1',
        'PassiveCapture.Lifecycle.ps1'
    )) {
        Copy-Item -LiteralPath (Join-Path $scriptDir $name) -Destination (Join-Path $fixtureScripts $name)
    }
    foreach ($name in @(
        'Hoi4ClientDriving.OptIn.ps1',
        'PortableProduct.PeImports.ps1',
        'ProductionArtifact.Attestation.ps1'
    )) {
        Copy-Item -LiteralPath (Join-Path $neutralScriptDir $name) -Destination (Join-Path $fixtureNeutralScripts $name)
    }
    # Deriving the fixture from the authority's own walk means this file no
    # longer double-checks that walk by restating it. The pin below is the
    # compensation: a hand-written manifest graph with a hand-written expected
    # answer, so a traversal that stops following lane-to-lane edges, resolves a
    # workspace-inherited path against the wrong base, or starts reporting a
    # production unit as one of its own dependencies still fails by name.
    $pinnedGraphRoot = Join-Path $root '.pinned_manifest_graph'
    $pinnedGraphManifests = [ordered]@{
        'extractor\native\Cargo.toml' = @'
[workspace]
members = ["collector", "recorder_host", "lanes/lane_alpha", "lanes/lane_beta"]

[workspace.dependencies]
pinned_workspace_dep = { path = "pinned/workspace_dep" }
'@
        'extractor\native\Cargo.lock' = @'
version = 3
'@
        'extractor\native\recorder_host\Cargo.toml' = @'
[package]
name = "recorder_host"
'@
        'extractor\native\collector\Cargo.toml' = @'
[package]
name = "collector"

[dependencies]
lane_alpha = { path = "../lanes/lane_alpha" }
shared_alpha = { path = "../shared_alpha" }
'@
        'extractor\native\lanes\lane_alpha\Cargo.toml' = @'
[package]
name = "lane_alpha"

[dependencies]
lane_beta = { path = "../lane_beta" }
pinned_workspace_dep = { workspace = true }
'@
        'extractor\native\lanes\lane_beta\Cargo.toml' = @'
[package]
name = "lane_beta"
'@
        'extractor\native\shared_alpha\Cargo.toml' = @'
[package]
name = "shared_alpha"
'@
        'extractor\native\pinned\workspace_dep\Cargo.toml' = @'
[package]
name = "pinned_workspace_dep"
'@
        'replay_interface\native\Cargo.toml' = @'
[workspace]
members = ["replay_app"]
'@
        'replay_interface\native\Cargo.lock' = @'
version = 3
'@
        'replay_interface\native\replay_app\Cargo.toml' = @'
[package]
name = "replay_app"

[dependencies]
replay_shared = { path = "../replay_shared" }
'@
        'replay_interface\native\replay_shared\Cargo.toml' = @'
[package]
name = "replay_shared"
'@
    }
    foreach ($pinnedManifest in $pinnedGraphManifests.GetEnumerator()) {
        $pinnedManifestPath = Join-Path $pinnedGraphRoot $pinnedManifest.Key
        [IO.Directory]::CreateDirectory((Split-Path -Parent $pinnedManifestPath)) | Out-Null
        [IO.File]::WriteAllText(
            $pinnedManifestPath,
            ([string]$pinnedManifest.Value + "`n"),
            [Text.UTF8Encoding]::new($false)
        )
    }
    $pinnedGraphClosure = @(
        Get-ProductionUnitDependencyClosure -RepoRoot $pinnedGraphRoot
    )
    $pinnedGraphUnits = @(
        $pinnedGraphClosure | ForEach-Object { [string]$_.unit } | Sort-Object
    )
    Assert-Case 'pinned manifest graph resolves the exact production unit closure' (
        ($pinnedGraphUnits -join '|') -ceq (@(
            'extractor/native/collector',
            'extractor/native/lanes/lane_alpha',
            'extractor/native/lanes/lane_beta',
            'extractor/native/recorder_host',
            'replay_interface/native/replay_app'
        ) -join '|')
    )
    $pinnedGraphDependencyRoots = @(
        $pinnedGraphClosure |
            ForEach-Object { @($_.dependencies) } |
            Where-Object { -not $_.production_unit } |
            ForEach-Object {
                Get-FixtureRelativePath `
                    -RepoRoot $pinnedGraphRoot `
                    -Path ([string]$_.root)
            } |
            Sort-Object -Unique
    )
    Assert-Case 'pinned manifest graph resolves the exact dependency roots' (
        ($pinnedGraphDependencyRoots -join '|') -ceq (@(
            'extractor\native\pinned\workspace_dep',
            'extractor\native\shared_alpha',
            'replay_interface\native\replay_shared'
        ) -join '|')
    )
    $pinnedGraphWorkspaceInputs = @(
        $pinnedGraphClosure | ForEach-Object {
            '{0}=>{1}+{2}' -f
                [string]$_.unit,
                (Get-FixtureRelativePath -RepoRoot $pinnedGraphRoot -Path ([string]$_.lock_path)),
                (Get-FixtureRelativePath -RepoRoot $pinnedGraphRoot -Path ([string]$_.workspace_cargo_path))
        } | Sort-Object
    )
    Assert-Case 'pinned manifest graph binds every unit to its owning lock and workspace manifest' (
        ($pinnedGraphWorkspaceInputs -join '|') -ceq (@(
            'extractor/native/collector=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'extractor/native/lanes/lane_alpha=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'extractor/native/lanes/lane_beta=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'extractor/native/recorder_host=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'replay_interface/native/replay_app=>replay_interface\native\Cargo.lock+replay_interface\native\Cargo.toml'
        ) -join '|')
    )

    # This fixture used to name every file it created. The production authority
    # does not work from a list: it walks each production unit's Cargo manifest
    # and reads whatever those path edges reach. The two drifted, and the walk
    # then demanded a crate the list had never heard of
    # (extractor\native\compact_stream, pulled in by verbatim-copied lane
    # manifests), which aborted this self-test before its first case ran.
    #
    # The SHAPE of the fixture is now derived from the authority's own walk -
    # Get-ProductionUnitDependencyClosure, the function the classification
    # itself calls - so there is one traversal and no transcription of it. The
    # cost is that this fixture is no longer an independent check of that walk;
    # a pinned case below feeds the same function a hand-written manifest graph
    # with a hand-written expected answer, so a traversal regression still has a
    # named failing case.
    #
    # CONTENT stays the fixture's own decision. The files listed here are the
    # ones the authority makes content assertions about, so they are mirrored
    # byte for byte; every other source file the walk reaches is created at its
    # real relative path with an inert body. The fixture is a product tree that
    # has to pass, not a copy of the repository - copying the repository's own
    # lane sources would import their forbidden client-capability symbols and
    # this self-test would only ever restate that the repository is what it is.
    $verbatimFixtureSources = @(
        'extractor\native\recorder_host\src\main.rs',
        'extractor\native\recorder_host\src\early_render_launch.rs',
        'extractor\native\recorder_host\src\auto_attach.rs',
        'extractor\native\recorder_host\src\auto_attach\product_runtime.rs',
        'extractor\native\collector\src\lib.rs',
        'extractor\native\compact_contract\src\passive_ready.rs',
        'extractor\native\compact_contract\src\build_profile.rs',
        'extractor\native\compact_contract\src\control_transport\payloads.rs',
        'replay_interface\native\replay_app\src\main.rs',
        'replay_interface\native\replay_app\src\recording_open.rs',
        'replay_interface\native\replay_app\src\win32.rs'
    )
    $repoRootFull = [IO.Path]::GetFullPath($repoRoot).TrimEnd('\')
    $productionClosureUnits = @(
        Get-ProductionUnitDependencyClosure -RepoRoot $repoRoot
    )
    $productionLaneRoots = @(Get-CollectorProductionLaneRoots -RepoRoot $repoRoot)
    # Every production unit must have an owning Cargo.lock and the workspace
    # manifest beside it: the capability closure walks up from the unit to the
    # lock to find its workspace root and to resolve inherited dependency
    # paths and locked versions. Those come out of the walk, not out of a list.
    $fixtureManifestInputs = [System.Collections.Generic.List[string]]::new()
    $fixtureSourceRoots = [System.Collections.Generic.List[string]]::new()
    $fixtureUnitRoots = [System.Collections.Generic.List[string]]::new()
    foreach ($closureUnit in $productionClosureUnits) {
        $fixtureUnitRoots.Add([string]$closureUnit.unit_root)
    }
    foreach ($closureUnit in $productionClosureUnits) {
        foreach ($manifestInput in @(
            [string]$closureUnit.manifest_path,
            [string]$closureUnit.lock_path,
            [string]$closureUnit.workspace_cargo_path
        )) {
            if ((Test-Path -LiteralPath $manifestInput -PathType Leaf) -and
                -not $fixtureManifestInputs.Contains($manifestInput)) {
                $fixtureManifestInputs.Add($manifestInput)
            }
        }
        $closureUnitRoot = [string]$closureUnit.unit_root
        if (-not $fixtureSourceRoots.Contains($closureUnitRoot)) {
            $fixtureSourceRoots.Add($closureUnitRoot)
        }
        # Transitive, because the capability closure the authority computes is:
        # an in-repository crate is expanded through its own manifest, so
        # `extractor/native/snapshot` reaching `portable_replay/native/*` puts
        # those crates in the tree the authority reads. Materialising only the
        # units' DIRECT path dependencies built a fixture the authority then
        # threw on, and the queue is seeded and grown from the same resolver the
        # authority uses so the two cannot part company again.
        $pendingDependencies = [System.Collections.Generic.Queue[string]]::new()
        foreach ($dependency in @($closureUnit.dependencies)) {
            # A dependency that is itself a production unit is already a root
            # here, and a registry crate is read out of the real cargo source
            # cache by both the authority and this fixture, so neither is
            # materialised.
            if ($dependency.production_unit -or -not $dependency.path_dependency) {
                continue
            }
            $pendingDependencies.Enqueue([string]$dependency.root)
        }
        while ($pendingDependencies.Count -ne 0) {
            $dependencyRoot = [string]$pendingDependencies.Dequeue()
            if (-not $dependencyRoot.StartsWith(
                    $repoRootFull + '\',
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                continue
            }
            if ($fixtureSourceRoots.Contains($dependencyRoot)) {
                continue
            }
            $fixtureSourceRoots.Add($dependencyRoot)
            $dependencyManifest = Join-Path $dependencyRoot 'Cargo.toml'
            if (-not (Test-Path -LiteralPath $dependencyManifest -PathType Leaf)) {
                continue
            }
            if (-not $fixtureManifestInputs.Contains($dependencyManifest)) {
                $fixtureManifestInputs.Add($dependencyManifest)
            }
            foreach ($nested in @((Get-CrateManifestDependencyRoots `
                        -CrateRoot $dependencyRoot `
                        -LockPath ([string]$closureUnit.lock_path)).dependencies)) {
                if (-not [bool]$nested.path_dependency) {
                    continue
                }
                $pendingDependencies.Enqueue([string]$nested.root)
            }
        }
    }
    if ($fixtureSourceRoots.Count -eq 0 -or $fixtureManifestInputs.Count -eq 0) {
        throw 'self-test fixture derived no production roots from the source authority'
    }
    foreach ($manifestInput in $fixtureManifestInputs) {
        Copy-FixtureInput `
            -Source $manifestInput `
            -Destination (Join-Path $root (
                Get-FixtureRelativePath -RepoRoot $repoRoot -Path $manifestInput
            ))
    }
    # An in-repository DEPENDENCY that measures as capability-bearing is carried
    # verbatim, and which ones those are is measured, not listed. The capability
    # digest the committed pin accepts for such a crate is computed from that
    # crate's own sources, so an inert body would make the fixture's digest for
    # `inline_detour` differ from the repository's for no reason but the
    # fixture, and the case below - which compares the two - could then only be
    # weakened until it stopped saying anything. Unit sources stay inert: those
    # ARE read by the forbidden-symbol rules, and copying them would turn this
    # self-test into a restatement of the repository.
    $verbatimFixtureRoots = [System.Collections.Generic.List[string]]::new()
    $fixtureSuspensionEntryPoints = @(Get-ProductionThreadSuspensionEntryPoints)
    $fixtureMutationEntryPoints = @(Get-ProductionCodeMutationEntryPoints)
    foreach ($fixtureSourceRoot in $fixtureSourceRoots) {
        if ($fixtureUnitRoots.Contains($fixtureSourceRoot)) {
            continue
        }
        $dependencyScan = Get-ProductionCrateSourceCapabilityScan `
            -Root $fixtureSourceRoot `
            -SuspensionEntryPoints $fixtureSuspensionEntryPoints `
            -CodeMutationEntryPoints $fixtureMutationEntryPoints
        if ([bool]$dependencyScan.suspends -or [bool]$dependencyScan.mutates_code) {
            $verbatimFixtureRoots.Add($fixtureSourceRoot)
        }
    }
    $fixtureSourceExtensions = @(Get-ProductionInteractionSourceExtensions)
    $fixtureMaterialisedSources = [System.Collections.Generic.List[string]]::new()
    foreach ($fixtureSourceRoot in $fixtureSourceRoots) {
        $verbatimRoot = $verbatimFixtureRoots.Contains($fixtureSourceRoot)
        foreach ($sourceFile in @(
            Get-ChildItem -LiteralPath $fixtureSourceRoot -Recurse -Force -File
        )) {
            if ($fixtureSourceExtensions -notcontains $sourceFile.Extension) {
                continue
            }
            $relativeSource =
                Get-FixtureRelativePath -RepoRoot $repoRoot -Path $sourceFile.FullName
            $destination = Join-Path $root $relativeSource
            if ($verbatimRoot -or $verbatimFixtureSources -ccontains $relativeSource) {
                Copy-FixtureInput -Source $sourceFile.FullName -Destination $destination
            }
            else {
                Write-InertFixtureSource -Destination $destination
            }
            $fixtureMaterialisedSources.Add($relativeSource)
        }
    }
    # A cited file the walk never reaches is the failure that started this: it
    # used to surface as Copy-Item throwing on a path that does not exist, with
    # nothing to say which authority was supposed to read it.
    $unreachedVerbatimSources = @(
        $verbatimFixtureSources |
            Where-Object { $fixtureMaterialisedSources -cnotcontains $_ }
    )
    if ($unreachedVerbatimSources.Count -ne 0) {
        throw (
            'self-test fixture cites a source the production traversal never ' +
            "reaches: $($unreachedVerbatimSources -join ', ')"
        )
    }

    # The rejection loop far below mutates one source per role, and those four
    # are named by hand rather than derived. A name that is not in the fixture
    # used to surface as ReadAllText on a missing path three hours into the run,
    # behind every case above it - which is exactly how the citation of a lib.rs
    # in replay_app, a crate whose root is main.rs and which has never had a
    # lib.rs in any commit, survived. They are resolved and checked here, beside
    # the fixture that has to contain them.
    $sourceRejectionRoles = @(
        [pscustomobject]@{
            Name = 'recorder'
            Path = Join-Path $root 'extractor\native\recorder_host\src\main.rs'
        },
        [pscustomobject]@{
            Name = 'collector'
            Path = Join-Path $root 'extractor\native\collector\src\lib.rs'
        },
        [pscustomobject]@{
            Name = 'replay'
            # main.rs carries no self-UI exception, so the whole roster still
            # has to be rejected in it.
            Path = Join-Path $root 'replay_interface\native\replay_app\src\main.rs'
        },
        [pscustomobject]@{
            Name = 'production-lane'
            Path = Join-Path $root 'extractor\native\lanes\active_command_abilities_live\src\lib.rs'
        }
    )
    $missingSourceRejectionRoles = @(
        $sourceRejectionRoles |
            Where-Object {
                -not (Test-Path -LiteralPath ([string]$_.Path) -PathType Leaf)
            } |
            ForEach-Object { "$($_.Name)=$([string]$_.Path)" }
    )
    if ($missingSourceRejectionRoles.Count -ne 0) {
        throw (
            'self-test source-rejection role names a fixture source that does ' +
            "not exist: $($missingSourceRejectionRoles -join ', ')"
        )
    }
    $fixtureProductionLaneRoots = @(
        Get-CollectorProductionLaneRoots -RepoRoot $root
    )
    $expectedProductionLaneNames = @(
        $productionLaneRoots | ForEach-Object { Split-Path -Leaf $_ } | Sort-Object
    )
    $fixtureProductionLaneNames = @(
        $fixtureProductionLaneRoots |
            ForEach-Object { Split-Path -Leaf $_ } |
            Sort-Object
    )
    if (($expectedProductionLaneNames -join '|') -cne
            ($fixtureProductionLaneNames -join '|') -or
        $fixtureProductionLaneNames -cnotcontains
            'production_render_catalog_capture_contract') {
        throw 'self-test fixture does not reproduce the transitive production lane closure'
    }
    $fixtureCollectorCargoPath =
        Join-Path $root 'extractor\native\collector\Cargo.toml'
    $fixtureCollectorCargoOriginal =
        [IO.File]::ReadAllText($fixtureCollectorCargoPath)
    $activeLaneInline =
        'active_command_abilities_live_lane = { path = "../lanes/active_command_abilities_live" }'
    $activeLaneDotted =
        'active_command_abilities_live_lane.path = "../lanes/active_command_abilities_live"'
    $fixtureCollectorCargoDotted =
        $fixtureCollectorCargoOriginal.Replace($activeLaneInline, $activeLaneDotted)
    if ($fixtureCollectorCargoDotted -ceq $fixtureCollectorCargoOriginal) {
        throw 'self-test fixture could not create dotted production dependency syntax'
    }
    [IO.File]::WriteAllText(
        $fixtureCollectorCargoPath,
        $fixtureCollectorCargoDotted,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        $dependencyAliasInline = @(
            Get-TomlProductionDependencyRecords -Toml @'
[dependencies]
dependencies = { package = "active_command_abilities_live_lane", path = "../lanes/active_command_abilities_live" }
'@
        )
        $dependencyAliasDotted = @(
            Get-TomlProductionDependencyRecords -Toml @'
[dependencies]
dependencies.path = "../lanes/active_command_abilities_live"
'@
        )
        Assert-Case 'dotted dependency path remains in production lane closure' (
            @(
                Get-CollectorProductionLaneRoots -RepoRoot $root |
                    ForEach-Object { Split-Path -Leaf $_ }
            ) -ccontains 'active_command_abilities_live' -and
            $dependencyAliasInline.Count -eq 1 -and
            [string]$dependencyAliasInline[0].name -ceq 'dependencies' -and
            [string]$dependencyAliasInline[0].path -ceq
                '../lanes/active_command_abilities_live' -and
            $dependencyAliasDotted.Count -eq 1 -and
            [string]$dependencyAliasDotted[0].name -ceq 'dependencies' -and
            [string]$dependencyAliasDotted[0].path -ceq
                '../lanes/active_command_abilities_live'
        )
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureCollectorCargoPath,
            $fixtureCollectorCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }
    $fixtureWorkspaceCargoPath = Join-Path $root 'extractor\native\Cargo.toml'
    $fixtureWorkspaceCargoOriginal =
        [IO.File]::ReadAllText($fixtureWorkspaceCargoPath)
    $fixtureCollectorCargoWorkspace = $fixtureCollectorCargoOriginal.Replace(
        $activeLaneInline,
        'active_command_abilities_live_lane = { workspace = true }'
    )
    if ($fixtureCollectorCargoWorkspace -ceq $fixtureCollectorCargoOriginal) {
        throw 'self-test fixture could not create workspace production dependency syntax'
    }
    [IO.File]::WriteAllText(
        $fixtureWorkspaceCargoPath,
        ($fixtureWorkspaceCargoOriginal +
            "`n[workspace.dependencies]`n" +
            'active_command_abilities_live_lane = { path = "lanes/active_command_abilities_live" }' +
            "`n"),
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        $fixtureCollectorCargoPath,
        $fixtureCollectorCargoWorkspace,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Assert-Case 'workspace dependency path remains in production lane closure' (
            @(
                Get-CollectorProductionLaneRoots -RepoRoot $root |
                    ForEach-Object { Split-Path -Leaf $_ }
            ) -ccontains 'active_command_abilities_live'
        )
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureCollectorCargoPath,
            $fixtureCollectorCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
        [IO.File]::WriteAllText(
            $fixtureWorkspaceCargoPath,
            $fixtureWorkspaceCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }
    [IO.File]::WriteAllText(
        (Join-Path $root 'extractor\native\collector\src\attach_log.rs'),
        "// synthetic contract fixture: every production domain has an admitted passive source`n",
        [Text.UTF8Encoding]::new($false)
    )
    Copy-Item `
        -LiteralPath (Join-Path $repoRoot 'extractor\native\domain_registry.json') `
        -Destination (Join-Path $root 'extractor\native\domain_registry.json')
    $fixturePassiveReadyPath =
        Join-Path $root 'extractor\native\compact_contract\src\passive_ready.rs'
    $fixturePassiveReady = Get-Content -LiteralPath $fixturePassiveReadyPath -Raw
    $fixtureRosterDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &['
    $fixtureCountDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;'
    $fixtureRosterStart =
        $fixturePassiveReady.IndexOf($fixtureRosterDeclaration, [StringComparison]::Ordinal)
    $fixtureRosterBodyStart = $fixtureRosterStart + $fixtureRosterDeclaration.Length
    $fixtureRosterEnd = if ($fixtureRosterStart -ge 0) {
        $fixturePassiveReady.IndexOf('];', $fixtureRosterBodyStart, [StringComparison]::Ordinal)
    }
    else { -1 }
    if ($fixtureRosterStart -lt 0 -or
        -not $fixturePassiveReady.Contains($fixtureCountDeclaration) -or
        $fixtureRosterEnd -le $fixtureRosterBodyStart) {
        throw 'self-test fixture lacks a bounded slice roster and derived hook count'
    }
    $fixtureRosterBody = $fixturePassiveReady.Substring(
        $fixtureRosterBodyStart,
        $fixtureRosterEnd - $fixtureRosterBodyStart
    )
    $expectedPinnedHookCount = [regex]::Matches(
        $fixtureRosterBody,
        '(?m)^\s*pinned_(?:import|rva)!\('
    ).Count
    if ($expectedPinnedHookCount -le 0 -or $expectedPinnedHookCount -gt [uint16]::MaxValue) {
        throw 'self-test fixture has an invalid mechanically counted hook roster'
    }
    $expectedPinnedHookSourceSha256 =
        (Get-FileHash -LiteralPath $fixturePassiveReadyPath -Algorithm SHA256).Hash.ToLowerInvariant()
    # The attestation binds these exact production paths. Their fixture bodies
    # are inert but content-addressed, so the real helper still validates every
    # path/hash/source-digest relationship.
    [IO.File]::WriteAllText(
        (Join-Path $fixtureScripts 'Invoke-CentralRustValidation.ps1'),
        "throw 'fixture runner must never execute'`n",
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        (Join-Path $fixtureScripts 'Start-Hoi4ProductionCaptureSafe.ps1'),
        "throw 'fixture driver must never execute'`n",
        [Text.UTF8Encoding]::new($false)
    )
    Copy-Item `
        -LiteralPath (Join-Path $scriptDir 'verify_speed5_stream.py') `
        -Destination (Join-Path $fixtureScripts 'verify_speed5_stream.py')

    # The accepted suspending-code-mutation inventory is pinned per tree: the
    # repository's own pin names the exact files and call counts of the lanes
    # that patch live code, and this fixture deliberately does not carry those
    # sources. So the fixture measures its own inventory with the authority's
    # own functions and writes it as the fixture's pin. On its own that would be
    # a gate that cannot fail, so it is bracketed: the case below pins that the
    # fixture reaches exactly the units the committed repository pin says
    # delegate, with the digest that pin accepted, and a later case drives the
    # builder against a fixture pin that has drifted and requires a rejection.
    $fixtureSuspendingBaselinePath =
        Get-ProductionSuspendingCodeMutationBaselinePath -RepoRoot $root
    $fixtureSuspendingBaseline = Get-ProductionSuspendingCodeMutationBaseline `
        -Findings (Get-ProductionSuspendingCodeMutationFindings `
            -RepoRoot $root `
            -Authority (Get-ProductionInteractionSourceTreeAuthority -RepoRoot $root))
    [IO.File]::WriteAllText(
        $fixtureSuspendingBaselinePath,
        ($fixtureSuspendingBaseline | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $repoSuspendingBaseline = Read-ProductionSuspendingCodeMutationBaseline `
        -Path (Get-ProductionSuspendingCodeMutationBaselinePath -RepoRoot $repoRoot)
    $repoDelegatingUnits = @(
        $repoSuspendingBaseline.units |
            Where-Object { [bool]$_.delegates_suspending_code_mutation } |
            ForEach-Object { [string]$_.unit } |
            Sort-Object
    )
    # The expected delegated-patcher signature per unit is READ OUT of the
    # committed pin, never written down here. It used to be the literal
    # 'minhook', count exactly one - which was true only while every unit that
    # patched live code named minhook itself. It stopped being true the moment
    # lanes started taking their freeze from the shared in-repository detour
    # crate, and a hand-written expectation would have had to be edited to match
    # rather than reporting that the closure had changed shape.
    $repoPatcherSignatures =
        [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
    foreach ($repoUnit in @($repoSuspendingBaseline.units)) {
        if (-not [bool]$repoUnit.delegates_suspending_code_mutation) {
            continue
        }
        $repoPatcherSignatures[[string]$repoUnit.unit] = (@(
            @($repoUnit.delegated_patchers) | ForEach-Object {
                "$([string]$_.package)|$([bool]$_.reaches_suspension)|" +
                "$([bool]$_.reaches_code_mutation)|$([string]$_.capability_digest)"
            } | Sort-Object
        ) -join ';')
    }
    $fixtureBaselineUnits = @($fixtureSuspendingBaseline.units)
    $fixtureBaselineUnitNames = @(
        $fixtureBaselineUnits | ForEach-Object { [string]$_.unit } | Sort-Object
    )
    Assert-Case 'fixture reaches the exact accepted delegating unit closure' (
        $repoDelegatingUnits.Count -gt 0 -and
        ($fixtureBaselineUnitNames -join '|') -ceq ($repoDelegatingUnits -join '|') -and
        @($fixtureBaselineUnits | Where-Object {
            # The fixture's sources are inert, so no unit in it can perform the
            # mutation itself and none can carry direct evidence. What it does
            # carry is the manifest graph, so the delegation edges - and the
            # capability each one contributes - have to be the accepted ones.
            [bool]$_.performs_suspending_code_mutation -or
            -not [bool]$_.delegates_suspending_code_mutation -or
            @($_.direct_evidence).Count -ne 0 -or
            -not $repoPatcherSignatures.Contains([string]$_.unit) -or
            (@(
                @($_.delegated_patchers) | ForEach-Object {
                    "$([string]$_.package)|$([bool]$_.reaches_suspension)|" +
                    "$([bool]$_.reaches_code_mutation)|$([string]$_.capability_digest)"
                } | Sort-Object
            ) -join ';') -cne [string]$repoPatcherSignatures[[string]$_.unit]
        }).Count -eq 0
    )

    # Regression, mutation-proven by construction: the two halves of a
    # suspending code mutation split across the unit/dependency boundary. A unit
    # that writes live code itself and takes its thread freeze from a crate it
    # depends on is doing exactly what this gate inventories, and the rule that
    # required one scan subject to spell both halves reported nothing at all for
    # it. Both sides are synthesised into a unit the fixture measures as clean,
    # so the case can only pass because the halves were composed.
    $splitClosureUnits = @(Get-ProductionUnitDependencyClosure -RepoRoot $root)
    $splitCandidate = $null
    foreach ($splitUnit in $splitClosureUnits) {
        if ($fixtureBaselineUnitNames -ccontains [string]$splitUnit.unit) {
            continue
        }
        $splitDependency = @(
            @($splitUnit.dependencies) | Where-Object {
                [bool]$_.path_dependency -and
                -not [bool]$_.production_unit -and
                ([string]$_.root).StartsWith(
                    ([IO.Path]::GetFullPath($root).TrimEnd('\')) + '\',
                    [StringComparison]::OrdinalIgnoreCase
                )
            } | Select-Object -First 1
        )
        if ($splitDependency.Count -ne 1) {
            continue
        }
        $splitUnitSource = @(
            Get-ChildItem -LiteralPath ([string]$splitUnit.unit_root) -Recurse -Force -File |
                Where-Object { $_.Extension -ceq '.rs' } |
                Sort-Object FullName |
                Select-Object -First 1
        )
        $splitDependencySource = @(
            Get-ChildItem -LiteralPath ([string]$splitDependency[0].root) -Recurse -Force -File |
                Where-Object { $_.Extension -ceq '.rs' } |
                Sort-Object FullName |
                Select-Object -First 1
        )
        if ($splitUnitSource.Count -ne 1 -or $splitDependencySource.Count -ne 1) {
            continue
        }
        $splitCandidate = [pscustomobject]@{
            Unit = [string]$splitUnit.unit
            UnitSource = [string]$splitUnitSource[0].FullName
            DependencyName = [string]$splitDependency[0].name
            DependencySource = [string]$splitDependencySource[0].FullName
        }
        break
    }
    if ($null -eq $splitCandidate) {
        throw (
            'self-test fixture has no clean production unit with an in-repository ' +
            'path dependency to compose a split suspending code mutation from'
        )
    }
    $splitUnitOriginal = [IO.File]::ReadAllText($splitCandidate.UnitSource)
    $splitDependencyOriginal = [IO.File]::ReadAllText($splitCandidate.DependencySource)
    try {
        [IO.File]::WriteAllText(
            $splitCandidate.UnitSource,
            ($splitUnitOriginal + @'

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(a: *mut u8, b: usize, c: u32, d: *mut u32) -> i32;
}
pub unsafe fn split_fixture_mutates_code(a: *mut u8, d: *mut u32) -> i32 {
    VirtualProtect(a, 1, 0x40, d)
}
'@),
            [Text.UTF8Encoding]::new($false)
        )
        [IO.File]::WriteAllText(
            $splitCandidate.DependencySource,
            ($splitDependencyOriginal + @'

#[link(name = "kernel32")]
extern "system" {
    fn SuspendThread(a: *mut u8) -> u32;
}
pub unsafe fn split_fixture_suspends(a: *mut u8) -> u32 {
    SuspendThread(a)
}
'@),
            [Text.UTF8Encoding]::new($false)
        )
        $splitFindings = Get-ProductionSuspendingCodeMutationFindings `
            -RepoRoot $root `
            -Authority (Get-ProductionInteractionSourceTreeAuthority -RepoRoot $root)
        $splitUnitFinding = @(
            @($splitFindings.non_conforming_units) |
                Where-Object { [string]$_.unit -ceq $splitCandidate.Unit }
        )
        $splitPatcher = @(
            if ($splitUnitFinding.Count -eq 1) {
                @($splitUnitFinding[0].delegated_patchers) | Where-Object {
                    [string]$_.dependency -ceq $splitCandidate.DependencyName
                }
            }
        )
        Assert-Case 'a freeze in a dependency and a patch in the unit compose into one finding' (
            $splitUnitFinding.Count -eq 1 -and
            # Neither side spells both halves, so neither side is reportable on
            # its own - which is precisely why this used to measure as clean.
            -not [bool]$splitUnitFinding[0].performs_suspending_code_mutation -and
            [bool]$splitUnitFinding[0].delegates_suspending_code_mutation -and
            @($splitUnitFinding[0].suspension_sites).Count -eq 0 -and
            @($splitUnitFinding[0].code_mutation_sites).Count -gt 0 -and
            $splitPatcher.Count -eq 1 -and
            [bool]$splitPatcher[0].reaches_suspension -and
            -not [bool]$splitPatcher[0].reaches_code_mutation
        )
    }
    finally {
        [IO.File]::WriteAllText(
            $splitCandidate.UnitSource,
            $splitUnitOriginal,
            [Text.UTF8Encoding]::new($false)
        )
        [IO.File]::WriteAllText(
            $splitCandidate.DependencySource,
            $splitDependencyOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }

    # The same split, but with the freeze in a crate OUTSIDE the repository.
    # A registry crate is not expanded through its manifest, so the only thing
    # that can be said about it is what its own sources do - and a name that is
    # merely mentioned there may belong to something else entirely, which is why
    # the rule asks whether the half is exercised rather than merely present.
    # Both halves of that distinction are pinned here: a crate that CALLS the
    # entry point contributes its half, a crate that only names it does not.
    $externalCrateRoot = Join-Path ([IO.Path]::GetTempPath()) (
        'hoi4-selftest-external-' + [guid]::NewGuid().ToString('N')
    )
    $externalCallingRoot = Join-Path $externalCrateRoot 'calling'
    $externalNamingRoot = Join-Path $externalCrateRoot 'naming'
    $externalPointerRoot = Join-Path $externalCrateRoot 'pointer'
    try {
        [IO.Directory]::CreateDirectory((Join-Path $externalCallingRoot 'src')) | Out-Null
        [IO.Directory]::CreateDirectory((Join-Path $externalNamingRoot 'src')) | Out-Null
        [IO.Directory]::CreateDirectory((Join-Path $externalPointerRoot 'src')) | Out-Null
        [IO.File]::WriteAllText(
            (Join-Path $externalCallingRoot 'src\lib.rs'),
            @'
#[link(name = "kernel32")]
extern "system" {
    fn SuspendThread(a: *mut u8) -> u32;
}
pub unsafe fn external_fixture_suspends(a: *mut u8) -> u32 {
    SuspendThread(a)
}
'@,
            [Text.UTF8Encoding]::new($false)
        )
        # The exact shape the unrestricted rule reported as a thread freeze:
        # libc-0.2.189/src/psp.rs:505 is `SuspendThread = 66`, a variant of a
        # PlayStation-Portable enum.
        [IO.File]::WriteAllText(
            (Join-Path $externalNamingRoot 'src\lib.rs'),
            @'
#[repr(i32)]
pub enum ExternalFixtureThreadOp {
    SleepThread = 64,
    SuspendThread = 66,
}
'@,
            [Text.UTF8Encoding]::new($false)
        )
        # The ordinary indirect call: the entry point is bound to a local and
        # the local is called. Nothing about this is a definition of the name,
        # and a rule that discounted it would suppress a genuine freeze in a
        # third-party crate.
        [IO.File]::WriteAllText(
            (Join-Path $externalPointerRoot 'src\lib.rs'),
            @'
#[link(name = "kernel32")]
extern "system" {
    fn SuspendThread(a: *mut u8) -> u32;
}
pub unsafe fn external_fixture_suspends_indirectly(a: *mut u8) -> u32 {
    let suspend = SuspendThread;
    suspend(a)
}
'@,
            [Text.UTF8Encoding]::new($false)
        )
        $externalLockPath = Join-Path $root 'extractor\native\Cargo.lock'
        $externalSuspension = @(Get-ProductionThreadSuspensionEntryPoints)
        $externalMutation = @(Get-ProductionCodeMutationEntryPoints)
        $externalCallingClosure = Get-ProductionDependencyCapabilityClosure `
            -RepoRoot $root `
            -Root $externalCallingRoot `
            -LockPath $externalLockPath `
            -SuspensionEntryPoints $externalSuspension `
            -CodeMutationEntryPoints $externalMutation
        $externalNamingClosure = Get-ProductionDependencyCapabilityClosure `
            -RepoRoot $root `
            -Root $externalNamingRoot `
            -LockPath $externalLockPath `
            -SuspensionEntryPoints $externalSuspension `
            -CodeMutationEntryPoints $externalMutation
        Assert-Case 'a crate outside the repository that calls the freeze contributes its half' (
            [bool]$externalCallingClosure.reaches_suspension -and
            -not [bool]$externalCallingClosure.reaches_code_mutation -and
            @($externalCallingClosure.suspension_sites).Count -eq 1 -and
            [string]@($externalCallingClosure.suspension_sites)[0].kind -ceq 'direct_call'
        )
        $externalPointerClosure = Get-ProductionDependencyCapabilityClosure `
            -RepoRoot $root `
            -Root $externalPointerRoot `
            -LockPath $externalLockPath `
            -SuspensionEntryPoints $externalSuspension `
            -CodeMutationEntryPoints $externalMutation
        Assert-Case 'a crate outside the repository that calls the freeze through a pointer contributes its half' (
            [bool]$externalPointerClosure.reaches_suspension -and
            -not [bool]$externalPointerClosure.reaches_code_mutation -and
            # The binding is a reference, not a call, and it still counts: the
            # only kind that does not is a DEFINITION of the name.
            @(@($externalPointerClosure.suspension_sites) | Where-Object {
                [string]$_.kind -ceq 'capability_reference'
            }).Count -eq 1
        )
        Assert-Case 'a crate outside the repository that only names the freeze contributes nothing' (
            -not [bool]$externalNamingClosure.reaches_suspension -and
            -not [bool]$externalNamingClosure.reaches_code_mutation -and
            @($externalNamingClosure.suspension_sites).Count -eq 0 -and
            # The site IS measured, as a definition of the name - it is the
            # contribution rule that declines it, not the scanner failing to see
            # it. If the scan stopped seeing the enum variant this case would
            # pass for the wrong reason.
            @(@((Get-ProductionCrateSourceCapabilityScan `
                -Root $externalNamingRoot `
                -SuspensionEntryPoints $externalSuspension `
                -CodeMutationEntryPoints $externalMutation).suspension_sites) |
                    Where-Object { [string]$_.kind -ceq 'capability_definition' }
            ).Count -eq 1
        )
    }
    finally {
        if (Test-Path -LiteralPath $externalCrateRoot) {
            Remove-Item -LiteralPath $externalCrateRoot -Recurse -Force
        }
    }

    # A pinned reachability flag has to be a real boolean in the file. [bool] on
    # the string "false" is $true, so a pin that spelled it as a string would
    # compare equal to a measured $true while reading as its opposite.
    $booleanPinUnit = [pscustomobject]@{
        unit = 'extractor/native/lanes/fixture_boolean_pin'
        performs_suspending_code_mutation = $false
        delegates_suspending_code_mutation = $true
        direct_evidence = @()
        delegated_patchers = @(
            [pscustomobject]@{
                dependency = 'fixture_patcher'
                package = 'fixture_patcher'
                reaches_suspension = 'false'
                reaches_code_mutation = $true
                capability_digest = 'f' * 64
            }
        )
    }
    $booleanPinDefects = @(Get-ProductionSuspendingCodeMutationBaselineDefects -Pinned (
        [pscustomobject]@{
            schema = 'hoi4.production-suspending-code-mutation-baseline.v1'
            units = @($booleanPinUnit)
        }
    ))
    Assert-Case 'a pinned reachability flag spelled as a string is rejected' (
        @($booleanPinDefects | Where-Object {
            ([string]$_).Contains('reaches_suspension') -and
            ([string]$_).Contains('is not a boolean')
        }).Count -eq 1
    )

    # The lane-root derivation and cargo's own resolution of the collector have
    # to name the same lanes, and removing a lane from the manifest walk has to
    # be what makes them disagree. Without this, a dependency shape the TOML
    # reader stops understanding silently shrinks the tree every other case
    # above measures, and every one of them still passes.
    Assert-Case 'fixture lane roots agree with the resolved collector' (
        @(Get-CollectorProductionLaneRootCoverageDifferences -RepoRoot $root).Count -eq 0
    )
    $coverageCollectorCargoPath =
        Join-Path $root 'extractor\native\collector\Cargo.toml'
    $coverageCollectorCargoOriginal =
        [IO.File]::ReadAllText($coverageCollectorCargoPath)
    # Which lane is dropped is derived, not chosen: a lane the collector also
    # reaches through another lane's manifest stays in the walk when the
    # collector's own edge is deleted, and asserting on a hand-picked name would
    # make this case depend on which lane happened to be first in the file.
    $coverageMutationProven = $false
    $coverageDroppedLaneName = $null
    try {
        foreach ($coverageMatch in @([regex]::Matches(
                    $coverageCollectorCargoOriginal,
                    '(?m)^[^\r\n]*path\s*=\s*"\.\./lanes/(?<lane>[A-Za-z0-9_]+)"[^\r\n]*\r?\n'
                ))) {
            [IO.File]::WriteAllText(
                $coverageCollectorCargoPath,
                $coverageCollectorCargoOriginal.Remove(
                    $coverageMatch.Index,
                    $coverageMatch.Length
                ),
                [Text.UTF8Encoding]::new($false)
            )
            $coverageLane = [string]$coverageMatch.Groups['lane'].Value
            $coverageDifferences = @(
                Get-CollectorProductionLaneRootCoverageDifferences -RepoRoot $root
            )
            if (@($coverageDifferences | Where-Object {
                        ([string]$_).EndsWith(
                            '\' + $coverageLane,
                            [StringComparison]::OrdinalIgnoreCase
                        ) -and
                        ([string]$_).Contains('is not in the scanned lane roots')
                    }).Count -ge 1) {
                $coverageMutationProven = $true
                $coverageDroppedLaneName = $coverageLane
                break
            }
        }
    }
    finally {
        [IO.File]::WriteAllText(
            $coverageCollectorCargoPath,
            $coverageCollectorCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }
    Assert-Case 'a lane dropped from the manifest walk fails the coverage cross-check' (
        $coverageMutationProven -and
        -not [string]::IsNullOrWhiteSpace($coverageDroppedLaneName)
    )
    Assert-Case 'the coverage cross-check is green again once the lane is restored' (
        @(Get-CollectorProductionLaneRootCoverageDifferences -RepoRoot $root).Count -eq 0
    )

    $fixtureBuilder = Join-Path $fixtureScripts 'New-PortableProductPackage.ps1'
    # The fixture's receipt verifier is not named here: every case that runs it
    # is dispatched to the receipt-case pool, whose workers resolve it from the
    # root they are given. A second spelling of that path in this file would be
    # free to disagree with the one the workers actually run.
    $fixtureHelper = Join-Path $fixtureNeutralScripts 'ProductionArtifact.Attestation.ps1'
    . $fixtureHelper
    . (Join-Path $fixtureScripts 'PassiveCapture.Lifecycle.ps1')
    $freshness = Clear-ProductionArtifactOutputs -RepoRoot $root

    $recorder = Join-Path $releaseRoot 'collector_record.exe'
    $replay = Join-Path $releaseRoot 'replay_app.exe'
    $collector = Join-Path $releaseRoot 'collector.dll'
    Write-Bytes $recorder (New-MinimalAmd64Pe -Dll $false -Gui $true -Marker 1 -EmbeddedText @(
        'BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1',
        'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
    ))
    Write-Bytes $replay (New-MinimalAmd64Pe -Dll $false -Gui $true -Marker 2)
    Write-Bytes $collector (New-MinimalAmd64Pe -Dll $true -Gui $false -Marker 3 -EmbeddedText @(
        'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
    ))
    foreach ($name in @('fold_check.exe', 'compact_replay_gate.exe', 'pack_replay.exe')) {
        Write-Bytes (Join-Path $releaseRoot $name) ([Text.Encoding]::ASCII.GetBytes("fixture-$name"))
    }

    $zip = Join-Path $releaseRoot 'battle-plan-0.1.0-windows-x86_64.zip'
    $builderResult = Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
        '-RecorderExe', $recorder,
        '-ReplayExe', $replay,
        '-CollectorDll', $collector,
        '-OutputZip', $zip,
        '-Version', '0.1.0'
    )
    Assert-Case 'builder emits exact schema' ([string]$builderResult.schema -ceq 'hoi4-portable-package.v1')
    Assert-Case 'builder emits exact package path' ([IO.Path]::GetFullPath([string]$builderResult.path) -ieq [IO.Path]::GetFullPath($zip))
    Assert-Case 'builder emits two GUI executables' ([int]$builderResult.executable_count -eq 2)
    Assert-Case 'builder without real capture evidence is explicitly non-production' (
        $builderResult.admissible_for_production -is [bool] -and
        -not $builderResult.admissible_for_production -and
        $null -eq $builderResult.production_capture_evidence
    )
    Assert-Case 'builder reverified passive interaction closure' (
        [string]$builderResult.interaction_closure.policy -ceq 'passive_injected_observer' -and
        $builderResult.interaction_closure.zip_entry_bytes_reverified -is [bool] -and
        $builderResult.interaction_closure.zip_entry_bytes_reverified -and
        [string]$builderResult.interaction_closure.policy_inputs.read_only_game_state_witness_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.policy_inputs.passive_capture_policy_test_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.policy_inputs.passive_capture_lifecycle_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.policy_inputs.passive_ready_contract_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.source_capability.schema -ceq
            'hoi4.production-source-capability-closure.v1' -and
        $builderResult.interaction_closure.source_capability.canonical_observer_feature_isolated -and
        $builderResult.interaction_closure.source_capability.target_lifecycle_actuators_evidence_only -and
        $builderResult.interaction_closure.source_capability.collector_fixture_hooks_default_off -and
        $builderResult.interaction_closure.source_capability.structural_dependency_feature_parse -and
        $builderResult.interaction_closure.source_capability.production_source_tree_closed -and
        $builderResult.interaction_closure.source_capability.canonical_hook_name_families_closed -and
        [int]$builderResult.interaction_closure.source_capability.exact_pinned_hook_count -eq
            $expectedPinnedHookCount -and
        [string]$builderResult.interaction_closure.source_capability.pinned_hook_contract_source_sha256 -ceq
            $expectedPinnedHookSourceSha256 -and
        [string]$builderResult.interaction_closure.recorder_host.schema -ceq 'hoi4.interaction-closure.v1' -and
        [string]$builderResult.interaction_closure.replay_app.schema -ceq 'hoi4.interaction-closure.v1' -and
        [string]$builderResult.interaction_closure.collector_dll.schema -ceq 'hoi4.interaction-closure.v1' -and
        [int]$builderResult.interaction_closure.recorder_host.exact_pinned_hook_count -eq
            $expectedPinnedHookCount -and
        [int]$builderResult.interaction_closure.collector_dll.exact_pinned_hook_count -eq
            $expectedPinnedHookCount -and
        (@($builderResult.interaction_closure.recorder_host.permitted_self_ui_capabilities) -join '|') -ceq
            'SendMessageW|ShellExecuteW|EnableWindow|SetWindowTextW|ShowWindow|DestroyWindow' -and
        (@($builderResult.interaction_closure.replay_app.permitted_self_ui_capabilities) -join '|') -ceq
            'SetWindowTextW|DestroyWindow' -and
        @($builderResult.interaction_closure.collector_dll.permitted_self_ui_capabilities).Count -eq 0 -and
        (@($builderResult.interaction_closure.replay_app.permitted_self_worker_capabilities) -join '|') -ceq
            'CreateProcessW|TerminateProcess|CreateJobObjectW|AssignProcessToJobObject|SetInformationJobObject' -and
        @($builderResult.interaction_closure.recorder_host.permitted_self_worker_capabilities).Count -eq 0 -and
        @($builderResult.interaction_closure.collector_dll.permitted_self_worker_capabilities).Count -eq 0 -and
        $builderResult.interaction_closure.replay_app.forbidden_symbols_absent -and
        (@($builderResult.interaction_closure.recorder_host.permitted_injection_capabilities) -join '|') -ceq
            'OpenProcess|VirtualAllocEx|VirtualFreeEx|WriteProcessMemory|CreateRemoteThread|LoadLibraryW|FreeLibrary'
    )

    # ----------------------------------------------------------------------
    # The private trees every parallel segment below runs on.
    #
    # Made HERE, at the one point where the fixture is known good: the builder
    # has just accepted this tree end to end, so a copy of it is a copy of
    # something that builds. Every segment after this point either mutates one
    # file and asks the builder, or mutates one file and asks the source-tree
    # gate, and both are only sound with exactly one writer per tree.
    #
    # This is not a fixture store and nothing survives the run: the container is
    # created here, proved removed below, and the trees inside it are proved
    # byte-identical to the fixture both when they are made and after the
    # builder pool has finished writing to them.
    # ----------------------------------------------------------------------
    # Short name on purpose: these roots hold a full copy of a tree that already
    # sits under a long fixture path, and a longer prefix would push the deepest
    # files past MAX_PATH. Short also means fewer bits of uniqueness, so the
    # path is required to be NEW rather than merely created -- reusing whatever
    # is already there would let a leftover junction stand in for a private
    # tree.
    # Set only once this run OWNS the container, and it is the only thing any
    # teardown path is allowed to remove. A run that draws a name another run
    # already holds must fail without touching it: deleting the winner's trees
    # out from under its workers would give that run unrelated refusals, which
    # is a wrong verdict rather than a failed run.
    $privateTreeContainerOwned = $null
    $privateTreeContainer = Join-Path ([IO.Path]::GetTempPath()) (
        'hoi4pt' + [guid]::NewGuid().ToString('N').Substring(0, 8)
    )
    if (Test-Path -LiteralPath $privateTreeContainer) {
        throw "self-test private tree container already exists: $privateTreeContainer"
    }
    [IO.Directory]::CreateDirectory($privateTreeContainer) | Out-Null
    $privateTreeContainerInfo = [IO.DirectoryInfo]::new($privateTreeContainer)
    if (($privateTreeContainerInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "self-test private tree container is a reparse point: $privateTreeContainer"
    }
    # ONE filesystem operation that can only succeed for one caller.
    # CreateDirectory does not fail on a directory that already exists, so
    # "check then create" is a race: two runs that drew the same short name
    # would both proceed, their workers would write into each other's trees, and
    # a refusal caused by another case's mutation could be attributed to this
    # one. CreateNew on a file is atomic and refuses the second claimant, which
    # turns that race into a named failure.
    $privateTreeClaimPath = Join-Path $privateTreeContainer '.selftest-owner'
    $privateTreeClaim = [IO.File]::Open(
        $privateTreeClaimPath,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None
    )
    # Recorded the instant the claim is won, not once the rest of the checks
    # pass: from here on this run owns the container whether or not it goes on
    # to accept it, and anything that fails below has to leave a container the
    # teardown will remove rather than an orphan nobody claims.
    $privateTreeContainerOwned = $privateTreeContainer
    try { $privateTreeClaim.WriteByte(0) } finally { $privateTreeClaim.Dispose() }
    $privateTreeContainerEntries = @(
        $privateTreeContainerInfo.EnumerateFileSystemInfos(
            '*', [IO.SearchOption]::TopDirectoryOnly) |
            Where-Object { $_.Name -cne '.selftest-owner' }
    )
    if ($privateTreeContainerEntries.Count -ne 0) {
        throw (
            'self-test private tree container was not empty: ' +
            $privateTreeContainer
        )
    }
    $privateTreeManifest = Get-SelfTestTreeManifest -Root $root
    $privateTrees = @(New-SelfTestPrivateTreeSet `
        -SourceRoot $root `
        -ContainerRoot $privateTreeContainer `
        -Count (Get-SelfTestGateWorkerCount) `
        -SharedManifest $privateTreeManifest)
    $builderTrees = @(
        $privateTrees | Select-Object -First (Get-SelfTestBuilderWorkerCount)
    )
    if ($builderTrees.Count -lt 1) {
        throw 'self-test private tree set produced no builder trees'
    }
    # The control, once per tree, before any of them is asked to refuse
    # anything. A tree that cannot build is a tree that refuses every mutation,
    # and without this every pooled case below would pass for that reason.
    $privateTreeControlZip = '.cargo-target\release\private-tree-control.zip'
    $privateTreeControlVerdicts = Invoke-SelfTestBuilderCasePool `
        -Trees $privateTrees `
        -Cases @(
            0..($privateTrees.Count - 1) | ForEach-Object {
                New-SelfTestBuilderCase `
                    -Name ("private tree $_ builds") `
                    -Writes @() `
                    -Arguments @(
                        '-RecorderExe', '@ROOT@\.cargo-target\release\collector_record.exe',
                        '-ReplayExe', '@ROOT@\.cargo-target\release\replay_app.exe',
                        '-CollectorDll', '@ROOT@\.cargo-target\release\collector.dll',
                        '-OutputZip', ('@ROOT@\' + $privateTreeControlZip),
                        '-Version', '0.1.0'
                    ) `
                    -Removes @($privateTreeControlZip)
            }
        ) `
        -ContainerRoot $privateTreeContainer `
        -Label 'control'
    if ($privateTreeControlVerdicts.Count -ne $privateTrees.Count) {
        throw (
            'self-test private tree control returned ' +
            "$($privateTreeControlVerdicts.Count) verdict(s) for " +
            "$($privateTrees.Count) tree(s)"
        )
    }
    foreach ($controlName in @($privateTreeControlVerdicts.Keys)) {
        if ($privateTreeControlVerdicts[$controlName]) {
            throw (
                'self-test private tree control failed: the package builder ' +
                "refused an unmutated private tree ($controlName)"
            )
        }
    }
    # The control build published a package into every tree and the worker took
    # it back out again. That the trees are still byte-identical to the fixture
    # is proved once, after the LAST segment that writes to them -- a full
    # manifest walk of every tree is not free, and a leftover from here would
    # be caught by that same check before any pooled verdict is trusted by the
    # source-rejection roster. The workers additionally re-read every file they
    # restore, so a failed revert is reported by the worker that made it.
    Write-SelfTestCaseTiming -Name 'private fixture trees (copy and control build)'

    # The fixture's accepted inventory is measured, so on its own it can only
    # agree with itself. This drives the drift gate the other way: one accepted
    # unit is taken out of the pin while the tree still reaches it, which is
    # exactly the shape of "a unit started suspending and nobody accepted it".
    $fixtureBaselineOriginal = [IO.File]::ReadAllText($fixtureSuspendingBaselinePath)
    $driftedBaseline = $fixtureBaselineOriginal | ConvertFrom-Json
    $driftedBaseline.units = @(
        @($driftedBaseline.units) | Select-Object -Skip 1
    )
    if (@($driftedBaseline.units).Count -ne ($fixtureBaselineUnits.Count - 1)) {
        throw 'self-test could not drift the fixture suspending-code-mutation pin'
    }

    # The five cases below are one shape: put something the builder must refuse
    # into a tree, run the whole builder, and require a refusal. Serially they
    # cost five cold builder processes back to back, and each of those spends
    # about fourteen seconds re-deriving a source tree that four of the five do
    # not touch at all. They are dispatched across the private trees instead.
    #
    # What each case proves is unchanged: the real package builder, as a child
    # process, refusing that exact input. The mutation is the same mutation,
    # the arguments are the same arguments, and the refusal is recognised the
    # same way -- a non-zero exit or unparsable output.
    $evidenceMarker = [Text.Encoding]::ASCII.GetBytes('BATTLE_PLAN_CLIENT_DRIVING_EVIDENCE_V1')
    $evidenceRecorderBytes = [byte[]][IO.File]::ReadAllBytes($recorder).Clone()
    [Array]::Copy($evidenceMarker, 0, $evidenceRecorderBytes, 0x300, $evidenceMarker.Length)
    $suspendedLaunchBytes = [byte[]][IO.File]::ReadAllBytes($recorder).Clone()
    $suspendedLaunchMarker = [Text.Encoding]::ASCII.GetBytes('CREATE_SUSPENDED')
    [Array]::Copy(
        $suspendedLaunchMarker,
        0,
        $suspendedLaunchBytes,
        0x340,
        $suspendedLaunchMarker.Length
    )
    $targetLifecycleBytes = [byte[]][IO.File]::ReadAllBytes($recorder).Clone()
    $targetLifecycleMarker = [Text.Encoding]::ASCII.GetBytes('early_render_capture_receipt')
    [Array]::Copy($targetLifecycleMarker, 0, $targetLifecycleBytes, 0x380, $targetLifecycleMarker.Length)
    $evidenceCollectorBytes = [byte[]][IO.File]::ReadAllBytes($collector).Clone()
    [Array]::Copy($evidenceMarker, 0, $evidenceCollectorBytes, 0x300, $evidenceMarker.Length)

    $releaseRelative = '.cargo-target\release'
    $recorderRelative = "$releaseRelative\collector_record.exe"
    $replayRelative = "$releaseRelative\replay_app.exe"
    $collectorRelative = "$releaseRelative\collector.dll"
    $markerRejectionVerdicts = Invoke-SelfTestBuilderCasePool `
        -Trees $builderTrees `
        -ContainerRoot $privateTreeContainer `
        -Label 'markers' `
        -Cases @(
            (New-SelfTestBuilderCase `
                -Name 'accepted suspending-code-mutation drift is rejected' `
                -Writes @(
                    New-SelfTestBuilderCaseTextWrite `
                        -RelativePath (Get-FixtureRelativePath `
                            -RepoRoot $root -Path $fixtureSuspendingBaselinePath) `
                        -Content ($driftedBaseline | ConvertTo-Json -Depth 8)
                ) `
                -Arguments @(
                    '-RecorderExe', "@ROOT@\$recorderRelative",
                    '-ReplayExe', "@ROOT@\$replayRelative",
                    '-CollectorDll', "@ROOT@\$collectorRelative",
                    '-OutputZip', "@ROOT@\$releaseRelative\drifted-suspending-baseline.zip",
                    '-Version', '0.1.0'
                )),
            (New-SelfTestBuilderCase `
                -Name 'evidence recorder marker is rejected' `
                -Writes @(
                    New-SelfTestBuilderCaseByteWrite `
                        -RelativePath "$releaseRelative\collector_record_evidence_marker.exe" `
                        -Bytes $evidenceRecorderBytes
                ) `
                -Arguments @(
                    '-RecorderExe', "@ROOT@\$releaseRelative\collector_record_evidence_marker.exe",
                    '-ReplayExe', "@ROOT@\$replayRelative",
                    '-CollectorDll', "@ROOT@\$collectorRelative",
                    '-OutputZip', "@ROOT@\$releaseRelative\forbidden-evidence-recorder.zip",
                    '-Version', '0.1.0'
                )),
            (New-SelfTestBuilderCase `
                -Name 'target suspended-launch marker is rejected' `
                -Writes @(
                    New-SelfTestBuilderCaseByteWrite `
                        -RelativePath "$releaseRelative\collector_record_suspended_launch_marker.exe" `
                        -Bytes $suspendedLaunchBytes
                ) `
                -Arguments @(
                    '-RecorderExe', "@ROOT@\$releaseRelative\collector_record_suspended_launch_marker.exe",
                    '-ReplayExe', "@ROOT@\$replayRelative",
                    '-CollectorDll', "@ROOT@\$collectorRelative",
                    '-OutputZip', "@ROOT@\$releaseRelative\forbidden-suspended-launch-recorder.zip",
                    '-Version', '0.1.0'
                )),
            (New-SelfTestBuilderCase `
                -Name 'target early-render lifecycle marker is rejected' `
                -Writes @(
                    New-SelfTestBuilderCaseByteWrite `
                        -RelativePath "$releaseRelative\collector_record_target_lifecycle_marker.exe" `
                        -Bytes $targetLifecycleBytes
                ) `
                -Arguments @(
                    '-RecorderExe', "@ROOT@\$releaseRelative\collector_record_target_lifecycle_marker.exe",
                    '-ReplayExe', "@ROOT@\$replayRelative",
                    '-CollectorDll', "@ROOT@\$collectorRelative",
                    '-OutputZip', "@ROOT@\$releaseRelative\forbidden-target-lifecycle-recorder.zip",
                    '-Version', '0.1.0'
                )),
            (New-SelfTestBuilderCase `
                -Name 'evidence collector marker is rejected' `
                -Writes @(
                    New-SelfTestBuilderCaseByteWrite `
                        -RelativePath "$releaseRelative\collector_evidence_marker.dll" `
                        -Bytes $evidenceCollectorBytes
                ) `
                -Arguments @(
                    '-RecorderExe', "@ROOT@\$recorderRelative",
                    '-ReplayExe', "@ROOT@\$replayRelative",
                    '-CollectorDll', "@ROOT@\$releaseRelative\collector_evidence_marker.dll",
                    '-OutputZip', "@ROOT@\$releaseRelative\forbidden-evidence-collector.zip",
                    '-Version', '0.1.0'
                ))
        )
    foreach ($markerCaseName in @(
            'accepted suspending-code-mutation drift is rejected',
            'evidence recorder marker is rejected',
            'target suspended-launch marker is rejected',
            'target early-render lifecycle marker is rejected',
            'evidence collector marker is rejected'
        )) {
        # Named individually, in the order the serial form ran them, so the case
        # names, their order and their count are exactly what they were.
        Assert-Case $markerCaseName ([bool]$markerRejectionVerdicts[$markerCaseName])
    }

    # Repinned deliberately, three times. The roster gained the thread-handle
    # acquisition and consumption surface it was missing: the Zw
    # suspend/resume/terminate aliases, Nt/Zw SetContextThread, the alert and
    # alert-resume pairs, Nt/Zw SetInformationThread and ImpersonateThread, the
    # QueueUserAPC2 and QueueApcThreadEx2 forms, SetThreadToken,
    # SetThreadInformation, CancelSynchronousIo, the Nt/Zw OpenThread pair, and
    # the scheduling setters. NtOpenThread in particular meant the ban on
    # obtaining another thread's handle had never been in force at all, since
    # only the Win32 spelling was listed.
    #
    # Each of three review rounds found more of this surface, so expect this
    # census to move again. It is a census, not a proof that the thread-control
    # surface is closed. Two files pin it and both have to move together.
    $fixedForbiddenMarkerCount = 147
    $fixedForbiddenMarkerRosterSha256 =
        '3068b0722bf931718c471cbeb998998b05ff9cca2ae72a4ecb2ce8a9a8bcf617'
    $forbiddenMarkerRoster = @(Get-ForbiddenProductionInteractionMarkers)
    Assert-Case 'forbidden interaction marker roster is the fixed reviewed census' (
        $forbiddenMarkerRoster.Count -eq $fixedForbiddenMarkerCount -and
        (Get-ProductionInteractionMarkerRosterSha256) -ceq
            $fixedForbiddenMarkerRosterSha256
    )
    $binaryEnforcerProbe = New-FixtureInteractionEnforcerProbe `
        -BuilderPath $fixtureBuilder `
        -HelperPath (Join-Path $fixtureScripts 'PortableProduct.InteractionClosure.ps1')
    # The exact argument sets the builder binds for each role. They are not a
    # relaxation of anything: the builder passes these same three literals at
    # its three enforcer callsites, and a role that gains or loses a permitted
    # capability there has to be changed here too or the roster case stops
    # matching the builder's own behaviour.
    $binaryRoleEnforcerArguments = @{
        recorder = [pscustomobject]@{
            Receipt = 'recorder_host'
            CleanBytes = [IO.File]::ReadAllBytes($recorder)
            Name = 'Battle Plan Recorder.exe'
            RequiredMarkers = @(
                'BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1',
                'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
            )
            Ui = @(
                'SendMessageW',
                'ShellExecuteW',
                'EnableWindow',
                'SetWindowTextW',
                'ShowWindow',
                'DestroyWindow'
            )
            Worker = @()
            Injection = @(
                'OpenProcess',
                'VirtualAllocEx',
                'VirtualFreeEx',
                'WriteProcessMemory',
                'CreateRemoteThread',
                'LoadLibraryW',
                'FreeLibrary'
            )
        }
        replay = [pscustomobject]@{
            Receipt = 'replay_app'
            CleanBytes = [IO.File]::ReadAllBytes($replay)
            Name = 'Battle Plan.exe'
            RequiredMarkers = @()
            Ui = @('SetWindowTextW', 'DestroyWindow')
            Worker = @(
                'CreateProcessW',
                'TerminateProcess',
                'CreateJobObjectW',
                'AssignProcessToJobObject',
                'SetInformationJobObject'
            )
            Injection = @()
        }
        collector = [pscustomobject]@{
            Receipt = 'collector_dll'
            CleanBytes = [IO.File]::ReadAllBytes($collector)
            Name = 'collector.dll'
            RequiredMarkers = @('BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1')
            Ui = @()
            Worker = @()
            Injection = @()
        }
    }
    # A rejection is only evidence if the probe ACCEPTS the unmutated binary for
    # the same role under the same arguments. Without this, a probe that threw
    # on every input - a mis-bound required-marker list would do it - would read
    # exactly like a probe rejecting every forbidden symbol, and the four
    # hundred cases below would all pass while proving nothing.
    #
    # The control is stricter than "does not throw": the probe's receipt for the
    # clean binary has to equal, field for field, the receipt the real builder
    # just published for that role. That pins the arguments too, so a role whose
    # permitted capabilities drift out of step with the builder's own callsite
    # is caught here rather than silently narrowing the roster.
    foreach ($role in @('recorder', 'replay', 'collector')) {
        $roleEnforcerArguments = $binaryRoleEnforcerArguments[$role]
        $cleanReceipt = $binaryEnforcerProbe.'Invoke-FixtureInteractionEnforcer'(
            [byte[]]$roleEnforcerArguments.CleanBytes,
            [string]$roleEnforcerArguments.Name,
            [string[]]@($roleEnforcerArguments.RequiredMarkers),
            [uint16]$expectedPinnedHookCount,
            [string]$expectedPinnedHookSourceSha256,
            [string[]]@($roleEnforcerArguments.Ui),
            [string[]]@($roleEnforcerArguments.Worker),
            [string[]]@($roleEnforcerArguments.Injection)
        )
        $builderReceipt =
            $builderResult.interaction_closure.([string]$roleEnforcerArguments.Receipt)
        $receiptSummaries = foreach ($receipt in @($cleanReceipt, $builderReceipt)) {
            @(
                [string]$receipt.schema
                [string]$receipt.policy
                [string][int]$receipt.forbidden_symbol_count
                [string]$receipt.forbidden_symbol_roster_sha256
                [string][bool]$receipt.forbidden_symbols_absent
                [string][bool]$receipt.semantic_engine_state_writes
                [string][int]$receipt.exact_pinned_hook_count
                [string]$receipt.exact_pinned_hook_contract
                [string]$receipt.pinned_hook_contract_source_sha256
                (@($receipt.required_markers) -join ',')
                (@($receipt.forbidden_target_client_capabilities) -join ',')
                (@($receipt.permitted_self_ui_capabilities) -join ',')
                (@($receipt.permitted_self_worker_capabilities) -join ',')
                (@($receipt.permitted_injection_capabilities) -join ',')
            ) -join '|'
        }
        if (@($receiptSummaries).Count -ne 2 -or
            [string]@($receiptSummaries)[0] -cne [string]@($receiptSummaries)[1]) {
            throw (
                "self-test enforcer probe does not reproduce the builder receipt for " +
                "$role`: probe=$(@($receiptSummaries)[0]) builder=$(@($receiptSummaries)[1])"
            )
        }
    }
    # Derived once, up here, because the pool below and the loop after it have
    # to agree on which symbol is "the first symbol of this role" -- the pool
    # drives that one through the whole builder and the loop must then not
    # drive it again.
    $binaryRoleRosters = [ordered]@{}
    foreach ($role in @('recorder', 'replay', 'collector')) {
        $binaryRoleRosters[$role] = if ($role -ceq 'recorder') {
            @($forbiddenMarkerRoster | Where-Object {
                $_ -cnotin @(
                    'SendMessageW',
                    'ShellExecuteW',
                    'EnableWindow',
                    'SetWindowTextW',
                    'ShowWindow',
                    'DestroyWindow'
                )
            })
        }
        elseif ($role -ceq 'replay') {
            @($forbiddenMarkerRoster | Where-Object {
                $_ -cnotin @(
                    'SetWindowTextW',
                    'DestroyWindow',
                    'CreateProcessW',
                    'TerminateProcess',
                    'CreateJobObjectW',
                    'AssignProcessToJobObject',
                    'SetInformationJobObject'
                )
            })
        }
        else {
            @($forbiddenMarkerRoster)
        }
        if (@($binaryRoleRosters[$role]).Count -lt 2) {
            throw "self-test binary roster for $role is too small to exercise the gate"
        }
    }
    # The first symbol of every role is driven through the real package builder,
    # so "the builder rejects a forbidden binary symbol for this role" is
    # measured end to end and not inferred from the enforcer alone. Three cold
    # builder processes; nothing about them is sequential, so they are not run
    # sequentially.
    $binaryRoleBuilderCases = foreach ($role in @('recorder', 'replay', 'collector')) {
        $symbol = [string]@($binaryRoleRosters[$role])[0]
        $baseBytes = switch ($role) {
            'recorder' { [IO.File]::ReadAllBytes($recorder) }
            'replay' { [IO.File]::ReadAllBytes($replay) }
            'collector' { [IO.File]::ReadAllBytes($collector) }
            default { throw "unexpected binary interaction-closure role: $role" }
        }
        $mutatedBytes = [byte[]]$baseBytes.Clone()
        $marker = [Text.Encoding]::ASCII.GetBytes($symbol)
        [Array]::Copy($marker, 0, $mutatedBytes, 0x300, $marker.Length)
        $mutatedRelative = "$releaseRelative\" + ("forbidden-{0}-{1}.bin" -f $role, $symbol)
        New-SelfTestBuilderCase `
            -Name $role `
            -Writes @(
                New-SelfTestBuilderCaseByteWrite `
                    -RelativePath $mutatedRelative `
                    -Bytes $mutatedBytes
            ) `
            -Arguments @(
                '-RecorderExe', $(if ($role -ceq 'recorder') { "@ROOT@\$mutatedRelative" } else { "@ROOT@\$recorderRelative" }),
                '-ReplayExe', $(if ($role -ceq 'replay') { "@ROOT@\$mutatedRelative" } else { "@ROOT@\$replayRelative" }),
                '-CollectorDll', $(if ($role -ceq 'collector') { "@ROOT@\$mutatedRelative" } else { "@ROOT@\$collectorRelative" }),
                '-OutputZip', ("@ROOT@\$releaseRelative\" + ("forbidden-{0}-{1}.zip" -f $role, $symbol)),
                '-Version', '0.1.0'
            )
    }
    $binaryRoleBuilderVerdicts = Invoke-SelfTestBuilderCasePool `
        -Trees $builderTrees `
        -Cases @($binaryRoleBuilderCases) `
        -ContainerRoot $privateTreeContainer `
        -Label 'binroster'
    Write-SelfTestCaseTiming -Name 'binary roster builder path (one symbol per role)'

    foreach ($role in @('recorder', 'replay', 'collector')) {
        $allRejected = $true
        $roleEnforcerArguments = $binaryRoleEnforcerArguments[$role]
        $roleBuilderPathProved = $false
        $roleForbiddenMarkerRoster = @($binaryRoleRosters[$role])
        foreach ($symbol in $roleForbiddenMarkerRoster) {
            $rejected = $false
            if (-not $roleBuilderPathProved) {
                $rejected = [bool]$binaryRoleBuilderVerdicts[$role]
                $roleBuilderPathProved = $true
            }
            else {
                # Built here rather than before the branch: the first symbol's
                # bytes were planted by the pooled builder case, and reading the
                # clean binary again for every one of the remaining symbols was
                # pure waste even in the serial form.
                $baseBytes = switch ($role) {
                    'recorder' { [IO.File]::ReadAllBytes($recorder) }
                    'replay' { [IO.File]::ReadAllBytes($replay) }
                    'collector' { [IO.File]::ReadAllBytes($collector) }
                    default { throw "unexpected binary interaction-closure role: $role" }
                }
                $mutatedBytes = [byte[]]$baseBytes.Clone()
                $marker = [Text.Encoding]::ASCII.GetBytes($symbol)
                [Array]::Copy($marker, 0, $mutatedBytes, 0x300, $marker.Length)
                try {
                    $binaryEnforcerProbe.'Invoke-FixtureInteractionEnforcer'(
                        $mutatedBytes,
                        [string]$roleEnforcerArguments.Name,
                        [string[]]@($roleEnforcerArguments.RequiredMarkers),
                        [uint16]$expectedPinnedHookCount,
                        [string]$expectedPinnedHookSourceSha256,
                        [string[]]@($roleEnforcerArguments.Ui),
                        [string[]]@($roleEnforcerArguments.Worker),
                        [string[]]@($roleEnforcerArguments.Injection)
                    ) | Out-Null
                }
                catch {
                    $rejected = $true
                }
            }
            if (-not $rejected) {
                $allRejected = $false
                break
            }
        }
        Assert-Case "$role binary rejects the complete lifecycle/UI API roster" $allRejected
    }

    $fixtureCollectorCargo =
        Join-Path $root 'extractor\native\collector\Cargo.toml'
    $collectorCargoOriginal = [IO.File]::ReadAllText($fixtureCollectorCargo)
    $cargoMutations = [ordered]@{
        'multiline character dependency features are rejected' =
            $collectorCargoOriginal.Replace(
                'character_presentation_live_lane = { path = "../lanes/character_presentation_live" }',
                @'
character_presentation_live_lane = {
    path = "../lanes/character_presentation_live",
    features = [
        "a729-fixture-hook",
    ],
}
'@
            )
        'multiline focus dependency table features are rejected' =
            $collectorCargoOriginal.Replace(
                'focus_research_decisions_depth_live = { path = "../lanes/focus_research_decisions_depth_live" }',
                @'
[dependencies.focus_research_decisions_depth_live]
path = "../lanes/focus_research_decisions_depth_live"
features = [
    "a729-focus-fixture-hook",
]
[dependencies]
'@
            )
        'multiline aliased fixture feature is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
hidden_fixture_alias = {
    package = "runtime_text_inputs_live_lane",
    path = "../lanes/runtime_text_inputs_live",
    features = [
        "a729-fixture-hook",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
            )
        'escaped aliased fixture feature is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
escaped_fixture_alias = {
    package = "runtime_text_inputs_live_lane",
    path = "../lanes/runtime_text_inputs_live",
    features = [
        "a729\u002dfixture\u002dhook",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
            )
        'escaped inline package alias is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
escaped_package_alias = {
    package = "character\u005fpresentation\u005flive\u005flane",
    path = "../lanes/character_presentation_live",
    features = [
        "ordinary-looking-feature",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
            )
        'aliased character dependency subtable features are rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies.character_alias]
package = "character_presentation_live_lane"
path = "../lanes/character_presentation_live"
features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'dotted aliased focus dependency features are rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
focus_alias.package = "focus_research_decisions_depth_live"
focus_alias.path = "../lanes/focus_research_decisions_depth_live"
focus_alias.features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'mixed dotted and subtable aliases are one dependency record' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
mixed_alias.package = "character_presentation_live_lane"
mixed_alias.path = "../lanes/character_presentation_live"
[dependencies.mixed_alias]
features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'features preceding package retain alias state' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies.stateful_alias]
features = [
    "ordinary-looking-feature",
]
package = "character_presentation_live_lane"
path = "../lanes/character_presentation_live"
[target.'cfg(windows)'.dependencies]
'@
            )
        'features preceding escaped subtable package are rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies.escaped_stateful_alias]
features = [
    "ordinary-looking-feature",
]
package = "focus\u005fresearch\u005fdecisions\u005fdepth\u005flive"
path = "../lanes/focus_research_decisions_depth_live"
[target.'cfg(windows)'.dependencies]
'@
            )
        'quoted dotted dependency alias is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies]
"quoted.alias".package = "focus_research_decisions_depth_live"
"quoted.alias".path = "../lanes/focus_research_decisions_depth_live"
"quoted.alias".features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'target dependency subtable retains quoted dotted alias state' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[target.'cfg(windows)'.dependencies."target.alias"]
features = [
    "ordinary-looking-feature",
]
package = "character_presentation_live_lane"
path = "../lanes/character_presentation_live"
[target.'cfg(windows)'.dependencies]
'@
            )
    }
    # ----------------------------------------------------------------------
    # Everything from here to the lane-root scan is the same shape as the five
    # marker cases above: write one file (or two) into a tree, run the whole
    # package builder, require a refusal. Serially that is twenty-one cold
    # builder processes, about 40% of this suite's wall time, for work with no
    # order dependency in it whatsoever.
    #
    # So the mutations are all BUILT first and then dispatched across the
    # private trees in one pool. The construction below is the construction
    # that used to sit beside each case, moved up unchanged: what each case
    # writes, what it runs, and what it requires are identical. Only where the
    # write lands -- a private tree instead of the shared fixture -- and when it
    # runs have changed.
    # ----------------------------------------------------------------------
    $collectorCargoRelative = 'extractor\native\collector\Cargo.toml'
    $workspaceCargoRelative = 'extractor\native\Cargo.toml'
    $characterCargoRelative =
        'extractor\native\lanes\character_presentation_live\Cargo.toml'
    $fixtureWorkspaceCargo = Join-Path $root $workspaceCargoRelative
    $workspaceCargoOriginal = [IO.File]::ReadAllText($fixtureWorkspaceCargo)
    $workspaceInheritedCargo = $collectorCargoOriginal.Replace(
        "[target.'cfg(windows)'.dependencies]",
        @'
workspace_character_alias = {
    workspace = true,
    features = [
        "ordinary-looking-feature",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
    )
    $workspaceCargoMutation = $workspaceCargoOriginal + @'

[workspace.dependencies]
workspace_character_alias = {
    package = "character_presentation_live_lane",
    path = "lanes/character_presentation_live",
}
'@
    $workspaceAdditiveCollectorCargo = $collectorCargoOriginal.Replace(
        "[target.'cfg(windows)'.dependencies]",
        @'
workspace_additive_alias = {
    workspace = true,
    features = [
        "ordinary-looking-feature",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
    )
    $workspaceAdditiveRootCargo = $workspaceCargoOriginal + @'

[workspace.dependencies]
workspace_additive_alias = {
    package = "runtime_text_inputs_live_lane",
    path = "lanes/runtime_text_inputs_live",
    features = [
        "a729-fixture-hook",
    ],
}
'@
    $fixtureCharacterCargo = Join-Path $root $characterCargoRelative
    $characterCargoOriginal = [IO.File]::ReadAllText($fixtureCharacterCargo)
    $characterCargoDefaultMutation = $characterCargoOriginal.Replace(
        'default = []',
        'default = ["a729-fixture-hook"]'
    )
    if ($characterCargoDefaultMutation -ceq $characterCargoOriginal) {
        throw 'portable package self-test could not mutate protected lane defaults'
    }

    # The evidence-only actuator region is excised from the recorder's own
    # early_render_launch.rs and from nowhere else. A lane carrying a file with
    # the same LEAF name - here with a shouted extension, because the exclusion
    # used to be compared case-insensitively on the path - must not inherit the
    # exclusion.
    $fixtureShadowEarlyRenderPath = Join-Path (
        $fixtureProductionLaneRoots |
            Where-Object { (Split-Path -Leaf $_) -ceq 'active_command_abilities_live' } |
            Select-Object -First 1
    ) 'src\early_render_launch.RS'
    $fixtureShadowEarlyRenderRelative = Get-FixtureRelativePath `
        -RepoRoot $root -Path $fixtureShadowEarlyRenderPath
    $fixtureShadowEarlyRenderContent = @'
#[cfg(all(windows, feature = "evidence-client-driving"))]
unsafe extern "system" { fn SuspendThread(); }
#[cfg(all(windows, not(feature = "evidence-client-driving")))]
pub fn observer_only() {}
'@

    # This used to append `use minhook::MinHook; ... MinHook::enable_hook(..)`
    # to a lane that already declares minhook and require a rejection. It could
    # never get one, and it had never been run: the authority binds a unit to a
    # delegated patcher by the MANIFEST EDGE, not by the callsite text - by
    # design, because a re-export lets a crate use a patcher without ever
    # writing its name - and that lane's edge was already in the accepted
    # inventory, so the mutation changed nothing the gate keys on.
    #
    # The mutation now has the shape the authority actually enforces: a lane
    # with no minhook edge takes one on, which is a new delegated patcher and
    # has to be accepted again. air_loss_stats_live is in the production
    # closure and declares no minhook today; its owning lock resolves minhook
    # 0.9.0, so the classification reaches the same registry source as the
    # lanes that do.
    #
    # Limit worth stating: a lane that reaches minhook only through another
    # in-repo crate's re-export still has no minhook edge of its own, and a
    # pure re-exporter has no suspend/mutate site of its own to be classified
    # on. That path is covered neither here nor by the authority.
    $fixtureProductionLaneRelative =
        'extractor\native\lanes\air_loss_stats_live\Cargo.toml'
    $fixtureProductionLane = Join-Path $root $fixtureProductionLaneRelative
    $fixtureProductionLaneOriginal = [IO.File]::ReadAllText($fixtureProductionLane)
    if ($fixtureProductionLaneOriginal.Contains('minhook')) {
        throw 'self-test lane already declares the suspending primitive it plants'
    }
    $fixtureProductionLaneMutation = $fixtureProductionLaneOriginal +
        "`n[target.'cfg(windows)'.dependencies]`nminhook = `"0.9.0`"`n"

    # The first symbol of every source-rejection role, and the first production
    # lane root, still go through the whole package builder - so the builder is
    # still proved to reach the source-tree gate rather than that being inferred
    # from calling the gate directly. Those five builds are in this pool too.
    $sourceRejectionSymbols = @(Get-ForbiddenProductionClientCapabilitySymbols)
    if ($sourceRejectionSymbols.Count -lt 2) {
        throw 'self-test source-rejection roster is too small to exercise the gate'
    }
    $sourceRejectionFirstSymbol = [string]$sourceRejectionSymbols[0]
    $laneRootBuilderRoot = @($fixtureProductionLaneRoots)[0]
    $laneRootBuilderLib = Join-Path $laneRootBuilderRoot 'src\lib.rs'
    $laneRootBuilderRelative = Get-FixtureRelativePath `
        -RepoRoot $root -Path $laneRootBuilderLib

    $pooledBuilderCases = [Collections.Generic.List[object]]::new()
    $cargoMutationIndex = 0
    foreach ($mutation in $cargoMutations.GetEnumerator()) {
        if ([string]$mutation.Value -ceq $collectorCargoOriginal) {
            throw "portable package self-test could not apply Cargo mutation: $($mutation.Key)"
        }
        $cargoZipRelative =
            "$releaseRelative\forbidden-cargo-$cargoMutationIndex.zip"
        $pooledBuilderCases.Add((New-SelfTestBuilderCase `
            -Name ([string]$mutation.Key) `
            -Writes @(
                New-SelfTestBuilderCaseTextWrite `
                    -RelativePath $collectorCargoRelative `
                    -Content ([string]$mutation.Value)
            ) `
            -Arguments @(
                '-RecorderExe', "@ROOT@\$recorderRelative",
                '-ReplayExe', "@ROOT@\$replayRelative",
                '-CollectorDll', "@ROOT@\$collectorRelative",
                '-OutputZip', "@ROOT@\$cargoZipRelative",
                '-Version', '0.1.0'
            ) `
            -Removes @($cargoZipRelative)))
        $cargoMutationIndex++
    }
    foreach ($pairedCase in @(
            [pscustomobject]@{
                Name = 'workspace-inherited protected alias features are rejected'
                Collector = $workspaceInheritedCargo
                Workspace = $workspaceCargoMutation
                Zip = 'forbidden-workspace-inherited-feature.zip'
            },
            [pscustomobject]@{
                Name = 'workspace and local dependency features are scanned additively'
                Collector = $workspaceAdditiveCollectorCargo
                Workspace = $workspaceAdditiveRootCargo
                Zip = 'forbidden-workspace-additive-feature.zip'
            }
        )) {
        if ([string]$pairedCase.Collector -ceq $collectorCargoOriginal -or
            [string]$pairedCase.Workspace -ceq $workspaceCargoOriginal) {
            throw (
                'portable package self-test could not apply workspace mutation: ' +
                [string]$pairedCase.Name
            )
        }
        $pairedZipRelative = "$releaseRelative\" + [string]$pairedCase.Zip
        $pooledBuilderCases.Add((New-SelfTestBuilderCase `
            -Name ([string]$pairedCase.Name) `
            -Writes @(
                (New-SelfTestBuilderCaseTextWrite `
                    -RelativePath $collectorCargoRelative `
                    -Content ([string]$pairedCase.Collector)),
                (New-SelfTestBuilderCaseTextWrite `
                    -RelativePath $workspaceCargoRelative `
                    -Content ([string]$pairedCase.Workspace))
            ) `
            -Arguments @(
                '-RecorderExe', "@ROOT@\$recorderRelative",
                '-ReplayExe', "@ROOT@\$replayRelative",
                '-CollectorDll', "@ROOT@\$collectorRelative",
                '-OutputZip', "@ROOT@\$pairedZipRelative",
                '-Version', '0.1.0'
            ) `
            -Removes @($pairedZipRelative)))
    }
    foreach ($singleCase in @(
            [pscustomobject]@{
                Name = 'protected lane forbidden hook cannot enter default features'
                RelativePath = $characterCargoRelative
                Content = $characterCargoDefaultMutation
                Zip = 'forbidden-lane-default-feature.zip'
            },
            [pscustomobject]@{
                Name = 'evidence interval exclusion is exact recorder path only'
                RelativePath = $fixtureShadowEarlyRenderRelative
                Content = $fixtureShadowEarlyRenderContent
                Zip = 'forbidden-shadow-early-render.zip'
            },
            [pscustomobject]@{
                Name = 'production-linked lane cannot take on the suspending MinHook primitive'
                RelativePath = $fixtureProductionLaneRelative
                Content = $fixtureProductionLaneMutation
                Zip = 'forbidden-lane-minhook.zip'
            },
            [pscustomobject]@{
                Name = 'lane-root builder path'
                RelativePath = $laneRootBuilderRelative
                Content = ([IO.File]::ReadAllText($laneRootBuilderLib) +
                    "`nunsafe extern `"system`" { fn SuspendThread(); }`n")
                Zip = ('forbidden-lane-root-{0}.zip' -f (Split-Path -Leaf $laneRootBuilderRoot))
            }
        )) {
        $singleZipRelative = "$releaseRelative\" + [string]$singleCase.Zip
        $pooledBuilderCases.Add((New-SelfTestBuilderCase `
            -Name ([string]$singleCase.Name) `
            -Writes @(
                New-SelfTestBuilderCaseTextWrite `
                    -RelativePath ([string]$singleCase.RelativePath) `
                    -Content ([string]$singleCase.Content)
            ) `
            -Arguments @(
                '-RecorderExe', "@ROOT@\$recorderRelative",
                '-ReplayExe', "@ROOT@\$replayRelative",
                '-CollectorDll', "@ROOT@\$collectorRelative",
                '-OutputZip', "@ROOT@\$singleZipRelative",
                '-Version', '0.1.0'
            ) `
            -Removes @($singleZipRelative)))
    }
    foreach ($sourceRole in $sourceRejectionRoles) {
        $sourceRoleRelative = Get-FixtureRelativePath `
            -RepoRoot $root -Path ([string]$sourceRole.Path)
        $sourceRoleZipRelative = "$releaseRelative\" + (
            'forbidden-source-{0}-{1}.zip' -f
                [string]$sourceRole.Name, $sourceRejectionFirstSymbol
        )
        $pooledBuilderCases.Add((New-SelfTestBuilderCase `
            -Name ('source-rejection builder path: ' + [string]$sourceRole.Name) `
            -Writes @(
                New-SelfTestBuilderCaseTextWrite `
                    -RelativePath $sourceRoleRelative `
                    -Content ([IO.File]::ReadAllText([string]$sourceRole.Path) +
                        "`nunsafe extern `"system`" { fn $sourceRejectionFirstSymbol(); }`n")
            ) `
            -Arguments @(
                '-RecorderExe', "@ROOT@\$recorderRelative",
                '-ReplayExe', "@ROOT@\$replayRelative",
                '-CollectorDll', "@ROOT@\$collectorRelative",
                '-OutputZip', "@ROOT@\$sourceRoleZipRelative",
                '-Version', '0.1.0'
            ) `
            -Removes @($sourceRoleZipRelative)))
    }
    $pooledBuilderVerdicts = Invoke-SelfTestBuilderCasePool `
        -Trees $builderTrees `
        -Cases @($pooledBuilderCases) `
        -ContainerRoot $privateTreeContainer `
        -Label 'builders'
    # The pool is the only segment that writes to a private tree. Everything
    # after this point - including the source-rejection roster, which runs on
    # the same trees - depends on those writes having been taken back out, so
    # that is proved here rather than assumed.
    Assert-SelfTestPrivateTreesUnchanged `
        -Trees $privateTrees `
        -SharedManifest $privateTreeManifest `
        -Stage 'the pooled builder cases'
    Write-SelfTestCaseTiming -Name 'pooled package-builder rejection cases'

    # Named individually, in the order the serial form ran them, so the case
    # names, their order and their count are exactly what they were.
    foreach ($mutation in $cargoMutations.GetEnumerator()) {
        Assert-Case ([string]$mutation.Key) (
            [bool]$pooledBuilderVerdicts[[string]$mutation.Key]
        )
    }
    Assert-Case 'workspace-inherited protected alias features are rejected' (
        [bool]$pooledBuilderVerdicts['workspace-inherited protected alias features are rejected']
    )
    Assert-Case 'workspace and local dependency features are scanned additively' (
        [bool]$pooledBuilderVerdicts['workspace and local dependency features are scanned additively']
    )
    Assert-Case 'protected lane forbidden hook cannot enter default features' (
        [bool]$pooledBuilderVerdicts['protected lane forbidden hook cannot enter default features']
    )

    # As with the binary roster above, the first symbol of every role is driven
    # through the whole package builder and the rest are driven through the
    # source-tree authority the builder calls. The gate is the same function
    # either way - the package builder reaches it through
    # Get-ProductionSourceCapabilityClosure - and it is the only thing in the
    # builder that reads these files, so a symbol it does not reject here is a
    # symbol the builder does not reject either.
    #
    # This is the shared helper's own function, called against the fixture root
    # exactly as the fixture's own copy would be: the fixture's copy of
    # PortableProduct.InteractionClosure.ps1 is a verbatim copy of the one this
    # file dot-sourced, and the scenarios that mutate the fixture's copy of a
    # gate do not run in this loop.
    # Same control as the binary probe: the in-process gate has to accept the
    # unmutated fixture, or every rejection below could be the gate failing on
    # something that has nothing to do with the planted symbol.
    try {
        Assert-ProductionInteractionSourceTreeClosure `
            -RepoRoot $root `
            -Authority (Get-ProductionInteractionSourceTreeAuthority -RepoRoot $root)
    }
    catch {
        throw (
            'self-test source-tree gate rejects the unmutated fixture: ' +
            $_.Exception.Message
        )
    }
    # The roster loop below is the suite's dominant cost: four roles times the
    # full forbidden-capability roster is several hundred gate evaluations, and
    # every one of them is a genuinely fresh derivation, because the planted
    # symbol really does change what the classification sees -- planting
    # SuspendThread makes a crate become a suspending code patcher, which is the
    # whole point of the case. No memo makes distinct inputs cheap, so the only
    # lever left is to stop running them one after another. Measured serially at
    # roughly 4.8 s per evaluation, this segment was 72% of the suite.
    #
    # Each worker gets its OWN copy of the fixture tree. That is what makes the
    # parallelism sound: the loop's mutation is an in-place write to one file,
    # so workers sharing a tree would read each other's planted symbols and
    # every verdict would be meaningless. It also means exactly one process
    # mutates any given tree, so nothing here widens the per-file memo's
    # edit-and-revert window -- each private tree has a single writer, where the
    # serial version had one writer on one tree.
    #
    # An unfaithful copy is the failure this has to be able to see: a worker
    # whose tree was truncated, or whose copy of the gate was altered, would
    # reject every symbol and pass the case for the wrong reason. THE PARENT
    # proves the copy, file by file, before it will read any verdict from it --
    # a digest the worker computes with code living inside the copy cannot
    # establish that the copy is what we think it is. The worker's own control
    # (the gate must accept its unmutated tree) is kept as well, because it
    # catches a tree that is a faithful copy of something that was already
    # broken.
    #
    # One behaviour does change: the serial loop stopped at the first symbol a
    # role failed to reject. Workers evaluate their whole slice. That makes a
    # FAILING run do more work and report every unrejected symbol rather than
    # the first; it cannot change a verdict.
    # The builder path for the first symbol of every role was dispatched with
    # the other package-builder cases above; the verdicts are read back here so
    # the case below still requires the BUILDER to have refused, not just the
    # gate the builder calls.
    $sourceRoleBuilderVerdicts = [ordered]@{}
    foreach ($sourceRole in $sourceRejectionRoles) {
        $sourceRoleBuilderVerdicts[[string]$sourceRole.Name] = [bool](
            $pooledBuilderVerdicts[
                'source-rejection builder path: ' + [string]$sourceRole.Name
            ]
        )
    }

    $sourceRejectionWorkerCount = $privateTrees.Count
    $sourceRejectionWorkItems = [Collections.Generic.List[object]]::new()
    # Ordinal throughout. Rust symbol matching is case-sensitive, so
    # CreateProcessW and createprocessw are different work items and a
    # case-insensitive set would let a verdict for one satisfy the other.
    $sourceRejectionKeys = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($sourceRole in $sourceRejectionRoles) {
        $relative = Get-FixtureRelativePath -RepoRoot $root -Path ([string]$sourceRole.Path)
        foreach ($symbol in @($sourceRejectionSymbols | Select-Object -Skip 1)) {
            $key = "$([string]$sourceRole.Name)`0$relative`0$([string]$symbol)"
            # Stated rather than assumed: the per-worker bijection below proves
            # each worker answered its own slice, and only global uniqueness
            # makes those slices add up to the whole roster.
            if (-not $sourceRejectionKeys.Add($key)) {
                throw "self-test source-rejection work item is not unique: '$key'"
            }
            $sourceRejectionWorkItems.Add([pscustomobject]@{
                Role = [string]$sourceRole.Name
                RelativePath = $relative
                Symbol = [string]$symbol
                Key = $key
            })
        }
    }
    # The lane-root scan is the same work item under a different name: plant
    # SuspendThread in one lane's lib.rs and require the source-tree gate to
    # refuse the tree. It used to be its own serial loop over forty-odd lanes,
    # each one a fresh full-tree derivation in the parent process while every
    # worker core sat idle. Nothing about it differs from the roster items, so
    # it is dispatched with them.
    #
    # The first lane root still goes through the whole package builder - that
    # case is in the builder pool above - so "the builder reaches this gate for
    # a lane root" stays proved rather than inferred.
    $laneRootRoleName = 'lane-root'
    if (@($sourceRejectionRoles | Where-Object {
                [string]$_.Name -ceq $laneRootRoleName
            }).Count -ne 0) {
        throw "self-test source-rejection roles already use the reserved role '$laneRootRoleName'"
    }
    $laneRootScanItemCount = 0
    foreach ($fixtureLaneRoot in @($fixtureProductionLaneRoots | Select-Object -Skip 1)) {
        $laneRelative = Get-FixtureRelativePath `
            -RepoRoot $root `
            -Path (Join-Path $fixtureLaneRoot 'src\lib.rs')
        $laneKey = "$laneRootRoleName`0$laneRelative`0SuspendThread"
        if (-not $sourceRejectionKeys.Add($laneKey)) {
            throw "self-test source-rejection work item is not unique: '$laneKey'"
        }
        $sourceRejectionWorkItems.Add([pscustomobject]@{
            Role = $laneRootRoleName
            RelativePath = $laneRelative
            Symbol = 'SuspendThread'
            Key = $laneKey
        })
        $laneRootScanItemCount++
    }
    if ($laneRootScanItemCount -ne (@($fixtureProductionLaneRoots).Count - 1)) {
        throw 'self-test lane-root scan did not dispatch every lane root after the first'
    }
    $sourceRejectionWorkers = [Collections.Generic.List[object]]::new()
    $sourceRejectionCleanupProblems = [Collections.Generic.List[string]]::new()
    $sourceRejectionVerdicts = [ordered]@{}
    foreach ($sourceRole in $sourceRejectionRoles) {
        $sourceRejectionVerdicts[[string]$sourceRole.Name] =
            [Collections.Generic.List[object]]::new()
    }
    $sourceRejectionVerdicts[$laneRootRoleName] =
        [Collections.Generic.List[object]]::new()
    # The owned path, not the candidate: this segment's finally is the success
    # path's teardown for the whole container, and it must only ever remove a
    # container this run actually claimed.
    $sourceRejectionRoot = $privateTreeContainerOwned
    try {
        $workerScriptPath = Join-Path $sourceRejectionRoot 'worker.ps1'
        [IO.File]::WriteAllText(
            $workerScriptPath,
            $script:PortablePackageSourceRejectionWorkerScript,
            [Text.UTF8Encoding]::new($false)
        )
        for ($workerIndex = 0; $workerIndex -lt $sourceRejectionWorkerCount; $workerIndex++) {
            # Round robin, not contiguous blocks: every worker then carries the
            # same mix of roles, so one role being cheaper cannot leave a worker
            # idle while another still has a third of the roster left.
            $slice = @(
                0..($sourceRejectionWorkItems.Count - 1) |
                    Where-Object { ($_ % $sourceRejectionWorkerCount) -eq $workerIndex } |
                    ForEach-Object { $sourceRejectionWorkItems[$_] }
            )
            if ($slice.Count -eq 0) { continue }
            # The private tree this worker owns. Proved a faithful copy of the
            # fixture by the parent when it was made, and proved again after the
            # builder pool wrote to it and put it back -- file by file, over
            # every file in both trees, which covers the gate's own code and its
            # manifests, lockfiles and baselines, none of which the source-tree
            # digest reaches. A digest the worker computes with code living
            # inside its own copy could not establish any of that.
            $workerRoot = $privateTrees[$workerIndex]
            $requestPath = Join-Path $sourceRejectionRoot "request$workerIndex.json"
            $responsePath = Join-Path $sourceRejectionRoot "response$workerIndex.json"
            [IO.File]::WriteAllText(
                $requestPath,
                ([ordered]@{
                    Root = $workerRoot
                    OutPath = $responsePath
                    Items = @(
                        $slice | ForEach-Object {
                            [ordered]@{
                                Role = $_.Role
                                RelativePath = $_.RelativePath
                                Symbol = $_.Symbol
                            }
                        }
                    )
                } | ConvertTo-Json -Depth 6),
                [Text.UTF8Encoding]::new($false)
            )
            # Quoted. Start-Process flattens -ArgumentList into one command
            # line, so an unquoted path splits on the first space in %TEMP%.
            $process = Start-Process -FilePath 'powershell.exe' -PassThru -WindowStyle Hidden `
                -ArgumentList @(
                    '-NoProfile', '-ExecutionPolicy', 'Bypass',
                    '-File', ('"{0}"' -f $workerScriptPath),
                    '-RequestPath', ('"{0}"' -f $requestPath)
                )
            $sourceRejectionWorkers.Add([pscustomobject]@{
                Index = $workerIndex
                Process = $process
                ResponsePath = $responsePath
                Expected = @($slice)
            })
        }
        # Bounded: a wedged worker fails the suite with a named reason instead of
        # holding the run open until something outside it intervenes.
        $sourceRejectionBudgetMs = 3600000
        $sourceRejectionClock = [Diagnostics.Stopwatch]::StartNew()
        foreach ($worker in $sourceRejectionWorkers) {
            $remaining = $sourceRejectionBudgetMs -
                [int]$sourceRejectionClock.Elapsed.TotalMilliseconds
            if ($remaining -lt 1) { $remaining = 1 }
            if (-not $worker.Process.WaitForExit($remaining)) {
                try { Stop-SelfTestWorkerProcessTree -Process $worker.Process } catch { }
                throw (
                    "self-test source-rejection worker $($worker.Index) did not " +
                    "finish within the $sourceRejectionBudgetMs ms budget"
                )
            }
            # The bool overload can return before the exit-code cache is filled.
            $worker.Process.WaitForExit()
            if ($worker.Process.ExitCode -ne 0) {
                throw (
                    "self-test source-rejection worker $($worker.Index) exited " +
                    "$($worker.Process.ExitCode)"
                )
            }
        }
        foreach ($worker in $sourceRejectionWorkers) {
            if (-not (Test-Path -LiteralPath $worker.ResponsePath -PathType Leaf)) {
                throw (
                    "self-test source-rejection worker $($worker.Index) produced no response"
                )
            }
            $response = Get-Content -LiteralPath $worker.ResponsePath -Raw |
                ConvertFrom-Json
            if (-not $response.clean_tree_accepted) {
                throw (
                    "self-test source-rejection worker $($worker.Index) rejected its " +
                    'own unmutated tree: ' + [string]$response.clean_tree_message
                )
            }
            # Exact bijection between what was dispatched and what came back.
            # Counting alone lets two rows swap roles, or one tuple appear twice
            # while another never runs, and still add up.
            # Ordinal: a default hashtable would let a row for 'createprocessw'
            # consume the dispatched 'CreateProcessW', and the symbols this
            # plants into Rust are matched case-sensitively.
            $outstanding = [Collections.Generic.HashSet[string]]::new(
                [StringComparer]::Ordinal
            )
            foreach ($item in @($worker.Expected)) {
                [void]$outstanding.Add([string]$item.Key)
            }
            foreach ($row in @($response.results)) {
                $key = "$([string]$row.role)`0$([string]$row.relative_path)`0$([string]$row.symbol)"
                # Remove returns false for a key that was never dispatched AND
                # for one already consumed, so this is the duplicate check too.
                if (-not $outstanding.Remove($key)) {
                    throw (
                        "self-test source-rejection worker $($worker.Index) returned a " +
                        "verdict for unrequested or duplicated work item '$key'"
                    )
                }
                if ($row.rejected -isnot [bool]) {
                    throw (
                        "self-test source-rejection worker $($worker.Index) returned a " +
                        "non-boolean verdict for '$key'"
                    )
                }
                $sourceRejectionVerdicts[[string]$row.role].Add($row)
            }
            if ($outstanding.Count -ne 0) {
                throw (
                    "self-test source-rejection worker $($worker.Index) returned no " +
                    "verdict for $($outstanding.Count) dispatched work item(s)"
                )
            }
        }
    }
    finally {
        # Cleanup problems are RECORDED here, not thrown: throwing from a
        # finally would replace the real failure with a janitorial one. They are
        # raised after the block instead, where they can only be reached if the
        # main path succeeded.
        foreach ($worker in $sourceRejectionWorkers) {
            try {
                if (-not $worker.Process.HasExited) {
                    Stop-SelfTestWorkerProcessTree -Process $worker.Process
                }
            }
            catch {
                $sourceRejectionCleanupProblems.Add(
                    "worker $($worker.Index) could not be terminated: $($_.Exception.Message)"
                )
            }
        }
        # Wait before removing: on Windows a directory whose files a dying
        # process still has open cannot be removed, and the private trees would
        # be left behind in %TEMP%.
        foreach ($worker in $sourceRejectionWorkers) {
            try {
                if (-not $worker.Process.WaitForExit(30000)) {
                    $sourceRejectionCleanupProblems.Add(
                        "worker $($worker.Index) was still running 30000 ms after being terminated"
                    )
                }
            }
            catch {
                $sourceRejectionCleanupProblems.Add(
                    "worker $($worker.Index) could not be waited on: $($_.Exception.Message)"
                )
            }
        }
        try {
            Remove-Item -LiteralPath $sourceRejectionRoot -Recurse -Force `
                -ErrorAction Stop
            # Ownership ENDS here. The suite runs on for another twenty-odd
            # cases, and the container name is short; a later run that drew the
            # same name and claimed it would otherwise be deleted out from under
            # its own workers by this run's outer finally.
            $privateTreeContainerOwned = $null
        }
        catch {
            $sourceRejectionCleanupProblems.Add(
                "private worker trees under $sourceRejectionRoot were left behind: " +
                $_.Exception.Message
            )
        }
        # Warned here as well as thrown below, because those are different
        # paths. If the block above threw, that exception propagates the moment
        # this finally returns and the check below is never reached -- so a
        # leaked tree on a FAILING run would otherwise be recorded and then
        # silently discarded. A warning cannot displace the primary failure.
        # -WarningAction Continue is not the default here, it is the POINT.
        # This is an advanced script, so a caller's -WarningAction reaches this
        # line: Stop would make the warning terminating inside a finally and
        # replace the very exception it is trying to annotate, and
        # SilentlyContinue would drop the leaked path entirely. Pinning the
        # action makes the report independent of how the suite was invoked.
        if ($sourceRejectionCleanupProblems.Count -ne 0) {
            Write-Warning (
                'self-test source-rejection cleanup failed: ' +
                ($sourceRejectionCleanupProblems -join '; ')
            ) -WarningAction Continue
        }
    }
    # Only reachable when the block above did not throw. A run that leaves
    # worker processes or full tree copies behind in %TEMP% is a defect in this
    # suite, not something to discover later as unexplained disk use.
    if ($sourceRejectionCleanupProblems.Count -ne 0) {
        throw (
            'self-test source-rejection cleanup failed: ' +
            ($sourceRejectionCleanupProblems -join '; ')
        )
    }
    Write-SelfTestCaseTiming -Name 'source-rejection roster (parallel workers)'

    foreach ($sourceRole in $sourceRejectionRoles) {
        $roleName = [string]$sourceRole.Name
        $rows = @($sourceRejectionVerdicts[$roleName])
        # Every symbol after the first has to be accounted for. Without this a
        # worker that silently dropped its slice would leave "none unrejected"
        # true over an empty set, which is the shape of a case that cannot fail.
        $expectedRows = $sourceRejectionSymbols.Count - 1
        Assert-Case "$roleName source rejects the complete lifecycle/UI API roster" (
            [bool]$sourceRoleBuilderVerdicts[$roleName] -and
            $rows.Count -eq $expectedRows -and
            @($rows | Where-Object { -not $_.rejected }).Count -eq 0
        )
    }
    # Same split as the two rosters above: the first lane root went through the
    # whole builder (in the builder pool), the remaining forty-odd through the
    # source-tree authority the builder calls (in the roster pool). What varies
    # across these cases is which lane root the authority reaches, not which
    # gate decides.
    #
    # Every lane after the first has to be accounted for. Without the row count
    # a pool that silently dropped this role's slice would leave "none
    # unrejected" true over an empty set, which is the shape of a case that
    # cannot fail.
    $laneRootRows = @($sourceRejectionVerdicts[$laneRootRoleName])
    Assert-Case 'every direct and transitive production lane root is source-scanned' (
        [bool]$pooledBuilderVerdicts['lane-root builder path'] -and
        $laneRootRows.Count -eq $laneRootScanItemCount -and
        @($laneRootRows | Where-Object { -not $_.rejected }).Count -eq 0
    )
    Assert-Case 'evidence interval exclusion is exact recorder path only' (
        [bool]$pooledBuilderVerdicts['evidence interval exclusion is exact recorder path only']
    )
    Assert-Case 'production-linked lane cannot take on the suspending MinHook primitive' (
        [bool]$pooledBuilderVerdicts['production-linked lane cannot take on the suspending MinHook primitive']
    )

    $hexA = 'a' * 64
    $hexB = 'b' * 64
    $hexC = 'c' * 64
    $commitHash = 'd' * 40
    $buildEnvValues = [ordered]@{}
    foreach ($key in $script:ProductionBuildEnvironmentKeys) { $buildEnvValues[$key] = '' }
    $buildEnvValues.CARGO_INCREMENTAL = '0'
    $buildEnvValues.CARGO_TARGET_DIR = Join-Path $root '.cargo-target'
    $buildEnvValues.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'synthetic-lld-link.exe'
    $buildEnvValues.PATH_SHA256 = $hexC
    $buildEnvironment = New-ProductionBuildEnvironmentRecord `
        -RustcVerbose "rustc 1.96.0`nhost: x86_64-pc-windows-msvc`nrelease: 1.96.0`ncommit-hash: $commitHash" `
        -CargoVersion 'cargo 1.96.0' `
        -Linker ([pscustomobject][ordered]@{
            configured = 'synthetic-lld-link.exe'; version_output = 'LLD 22.1.0 synthetic'
            executable = [pscustomobject]@{ path = 'C:\synthetic\lld-link.exe'; bytes = 1; sha256 = $hexA; file_version = '22.1'; product_version = '22.1' }
        }) `
        -RustcWrapper ([pscustomobject][ordered]@{ active = $false; configured = ''; executable = $null }) `
        -MsvcToolset ([pscustomobject][ordered]@{
            discovery = 'synthetic'; version = '14.50.00000'; root = 'C:\synthetic\MSVC'
            compiler = [pscustomobject]@{ path = 'C:\synthetic\cl.exe'; bytes = 1; sha256 = $hexB; file_version = '19.50'; product_version = '19.50' }
            msvc_linker = [pscustomobject]@{ path = 'C:\synthetic\link.exe'; bytes = 1; sha256 = $hexC; file_version = '14.50'; product_version = '14.50' }
        }) `
        -WindowsSdk ([pscustomobject][ordered]@{
            root = 'C:\synthetic\Windows Kits\10'; version = '10.0.26100.0'
            windows_sdk_dir_env = ''; windows_sdk_version_env = ''; ucrt_version_env = ''
            kernel32_import_library = [pscustomobject]@{ path = 'C:\synthetic\kernel32.lib'; bytes = 1; sha256 = $hexA; file_version = ''; product_version = '' }
            ucrt_import_library = [pscustomobject]@{ path = 'C:\synthetic\ucrt.lib'; bytes = 1; sha256 = $hexB; file_version = ''; product_version = '' }
        }) `
        -Runner ([pscustomobject][ordered]@{
            image_os = 'synthetic'; image_version = '1'; runner_os = 'Windows'; runner_arch = 'X64'
            runner_environment = 'self-test'; os_description = 'synthetic Windows'
            os_architecture = 'X64'; process_architecture = 'X64'
        }) `
        -BuildEnv $buildEnvValues
    $runtimeLinkage = [pscustomobject][ordered]@{
        target = 'x86_64-pc-windows-msvc'; rust_target_feature = '+crt-static'; c_runtime = 'static'
        admission = 'pe_import_and_delay_import_tables'; forbidden_import = 'VCRUNTIME*.dll'
    }
    $sourceDigest = Get-ProductionRustInputDigest -RepoRoot $root
    $attestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
    Write-ProductionArtifactAttestation `
        -RepoRoot $root `
        -ReceiptPath $attestationPath `
        -RequestedWorkspace All `
        -SourceDigest $sourceDigest `
        -ReleaseRuntimeLinkage $runtimeLinkage `
        -BuildEnvironment $buildEnvironment `
        -ArtifactFreshness $freshness
    $driver = Join-Path $fixtureScripts 'Start-Hoi4ProductionCaptureSafe.ps1'
    $attested = Test-ProductionArtifactAttestation -RepoRoot $root -ReceiptPath $attestationPath -ExpectedDriverPath $driver

    # Build a small but exact synthetic receipt graph. This does not stand in
    # for live evidence; it exercises the package/promotion binding itself.
    $captureRun = Join-Path $root 'synthetic-production-capture'
    [IO.Directory]::CreateDirectory($captureRun) | Out-Null
    $retainedAttestationPath = Join-Path $captureRun 'prebuilt_artifact_attestation.json'
    Copy-Item -LiteralPath $attestationPath -Destination $retainedAttestationPath
    $registry = Get-Content -LiteralPath (Join-Path $root 'extractor\native\domain_registry.json') -Raw |
        ConvertFrom-Json
    $promotionStatuses = @('existing', 'production_complete', 'production_complete_base')
    # Must match Get-ProductionPromotionExpectedDomains in
    # ProductionCapture.Promotion.ps1 exactly, including its exclusion of
    # domains the owner ruled out of capture scope (`capture_scheduled: false`,
    # absent when a domain is still scheduled). A synthetic roster that carries
    # a domain the real gate drops fails the gate it is meant to exercise.
    $domains = @(
        $registry.domains |
            Where-Object {
                # Read through PSObject.Properties, exactly as the promotion
                # authority does: the key is ABSENT on the 83 of 88 domains that
                # are still scheduled, and a direct `$_.capture_scheduled` is a
                # terminating PropertyNotFoundStrict under the strict mode this
                # script sets.
                $descheduled = $_.PSObject.Properties['capture_scheduled']
                $_.schedulable -eq $true -and
                -not ($null -ne $descheduled -and $descheduled.Value -eq $false) -and
                $promotionStatuses -ccontains [string]$_.status
            } |
            Sort-Object { [int]$_.id }
    )
    $canonicalRoster = @(
        $domains | ForEach-Object { [ordered]@{ id = [int]$_.id; name = [string]$_.name } }
    )
    $scheduledRoster = @(
        $domains | ForEach-Object { [ordered]@{ id = [int]$_.id; wire_name = [string]$_.name } }
    )
    $scheduledNames = @($domains | ForEach-Object { [string]$_.name })
    $authority = @(
        $domains | ForEach-Object {
            [ordered]@{ id = [int]$_.id; name = [string]$_.name; wire_name = [string]$_.name }
        }
    )
    $perDomain = [ordered]@{}
    foreach ($domain in $domains) {
        $perDomain[[string]$domain.name] = [ordered]@{
            domain_id = [int]$domain.id
            record_count = 2
            data_record_count = 2
            status_flags_or = 0
            effective_failure_status_flags_or = 0
        }
    }
    $captureId = 'synthetic-capture'
    $sessionId = '11111111-1111-4111-8111-111111111111'
    $targetPid = 123
    $targetCreationFiletimeUtc = [uint64]132537600000000000
    $requestedDurationMs = 1000
    $schedulerPollMs = 10
    $schedulerQueueCapacity = 1
    $schedulerWorkerCount = 1
    $targetExecutableSha256 =
        '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
    $collectorDllSha256 =
        (Get-FileHash -LiteralPath $collector -Algorithm SHA256).Hash.ToLowerInvariant()
    $speedRootGatePath = Join-Path $captureRun 'speed_root_gate.json'
    [IO.File]::WriteAllText(
        $speedRootGatePath,
        '{"synthetic":"root-gate-retained-by-self-test"}',
        [Text.UTF8Encoding]::new($false)
    )
    $speedEvidenceTranscriptPath =
        Join-Path $captureRun 'speed_evidence_transcript.json'
    [IO.File]::WriteAllText(
        $speedEvidenceTranscriptPath,
        '{"synthetic":"raw-speed-evidence-transcript"}',
        [Text.UTF8Encoding]::new($false)
    )
    $speedEvidenceNonceLedgerRoot = Join-Path $captureRun 'speed_nonce_ledger'
    [IO.Directory]::CreateDirectory($speedEvidenceNonceLedgerRoot) | Out-Null
    $authorityPath = Join-Path $captureRun 'production_domain_roster_authority.json'
    [IO.File]::WriteAllText($authorityPath, ($authority | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false))
    $planPath = Join-Path $captureRun 'production_capture_plan.json'
    $plan = [ordered]@{
        schema = 'battle-plan.production_capture_plan.v1'
        capture_id = $captureId
        session_id = $sessionId
        pid = $targetPid
        process_creation_filetime_utc = $targetCreationFiletimeUtc
        requested_duration_ms = $requestedDurationMs
        read_only_scheduler_poll_ms = $schedulerPollMs
        read_only_scheduler_queue_cap = $schedulerQueueCapacity
        scheduler_worker_count = $schedulerWorkerCount
        exe_sha256 = $targetExecutableSha256
        staged_dll_sha256 = $collectorDllSha256
        speed_required_raw = 4
        unpaused_required = $true
        read_only_scheduler_required = $true
        domain57_game_speed_gate_v2 = $speedRootGatePath
        domain57_game_speed_gate_v2_sha256 =
            (Get-FileHash -LiteralPath $speedRootGatePath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_evidence_transcript_path = $speedEvidenceTranscriptPath
        speed_evidence_transcript_sha256 = $hexA
        speed_evidence_transcript_file_sha256 =
            (Get-FileHash -LiteralPath $speedEvidenceTranscriptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_evidence_nonce_ledger_root = $speedEvidenceNonceLedgerRoot
        speed_evidence_nonce_seal_sha256 = $hexC
        prebuilt_source_input_sha256 = [string]$attested.source_input_sha256
        production_profile_domain_roster = $canonicalRoster
        scheduled_domain_roster = $scheduledRoster
        scheduled_domain_names = $scheduledNames
        domain_roster_authority_path = $authorityPath
        domain_roster_authority_sha256 =
            (Get-FileHash -LiteralPath $authorityPath -Algorithm SHA256).Hash.ToLowerInvariant()
        interaction_closure = [ordered]@{
            policy = 'passive_injected_observer'
            client_commands_during_capture = 0
            semantic_engine_state_writes = 0
            game_process_lifecycle_commands = 0
            game_ui_commands = 0
        }
    }
    [IO.File]::WriteAllText($planPath, ($plan | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))
    $manifestPath = Join-Path $captureRun 'native_session_manifest.json'
    $manifest = [ordered]@{
        capture_id = $captureId
        session_id = $sessionId
        pid = $targetPid
        skipped_game_hours = 0
        unique_game_hour_count = 2
        first_game_hour_raw = 100
        last_game_hour_raw = 101
        scheduled_domains = $scheduledNames
        compact_stream = [ordered]@{
            frame_end_record_count = 2
            domain_gap_record_count = 0
            domain_gap_outcomes = @()
            failure_status_flags_or = 0
            failure_status_seen = $false
            # The promotion gate requires this block and reads exactly three
            # things out of it: the schema literal, undeclared_unit_count, and
            # the undeclared list, which it cross-checks against that count and
            # then requires to be empty. The recorder's producer
            # (recorder_host diagnostic_unit_accumulation_json) also publishes a
            # `classes` table; it is deliberately NOT synthesized here, because
            # reproducing the Rust Accumulation table in this fixture would be a
            # second copy of engine content free to drift from the first, and
            # nothing in the gate reads it.
            diagnostic_unit_accumulation = [ordered]@{
                schema = 'battle-plan.diagnostic_unit_accumulation.v1'
                undeclared_unit_count = 0
                undeclared = @()
            }
            per_domain = $perDomain
        }
    }
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))
    $collectorControlPath = Join-Path $captureRun 'collector_control.json'
    $pipeToken = 'synthetic-passive-pipe-token'
    $pipeName = "\\.\pipe\battle-plan_control_$targetPid`_$pipeToken"
    $collectorControl = [ordered]@{
        pipe_token = $pipeToken
        capture_ready_pipe_name_v1 = $pipeName
    }
    [IO.File]::WriteAllText(
        $collectorControlPath,
        ($collectorControl | ConvertTo-Json -Compress),
        [Text.UTF8Encoding]::new($false)
    )
    $hookAuthority =
        Assert-PassiveReadySourceContractParity 'vanilla_latest_2026_06_30'
    $clockHourRaw = 60006000
    $clockObservationOrdinal = [uint64]1
    $ready = [ordered]@{
        schema = 'hoi4-capture-ready-v2'
        status = 'ready'
        capture_policy = 'passive-observer-only'
        pid = $targetPid
        process_creation_filetime_utc = $targetCreationFiletimeUtc
        session_id = $sessionId
        capture_id = $captureId
        requested_duration_ms = $requestedDurationMs
        readiness_flags = 255
        validated_domain_count = $scheduledNames.Count
        scheduler_queue_capacity = $schedulerQueueCapacity
        scheduler_worker_count = $schedulerWorkerCount
        allowed_hook_count = [int]$hookAuthority.Count
        active_hook_count = [int]$hookAuthority.Count
        clock_hour_raw = $clockHourRaw
        clock_observation_ordinal = $clockObservationOrdinal
        target_executable_sha256 = $targetExecutableSha256
        collector_dll_sha256 = $collectorDllSha256
        hook_allowlist_sha256 = [string]$hookAuthority.Sha256
        hook_outcomes_sha256 = $hexA
        validated_domain_roster_sha256 =
            Get-PassiveUtf8Sha256 ($scheduledNames -join "`n")
        clock_witness_sha256 = Get-PassiveClockWitnessSha256 `
            -ClockHourRaw $clockHourRaw `
            -ObservationOrdinal $clockObservationOrdinal `
            -TargetExecutableSha256 $targetExecutableSha256 `
            -CollectorBuildProfileId 'vanilla_latest_2026_06_30'
        scheduler_config_sha256 = Get-PassiveSchedulerConfigSha256 `
            -PollMilliseconds $schedulerPollMs `
            -DomainQueueCapacity $schedulerQueueCapacity `
            -WorkerCount $schedulerWorkerCount
        output_pipe_identity_sha256 = Get-PassiveOutputPipeIdentitySha256 `
            -CanonicalPipeName $pipeName `
            -TargetPid $targetPid `
            -TargetCreationFiletimeUtc $targetCreationFiletimeUtc `
            -SessionId $sessionId
        zero_mutation_policy_sha256 = Get-PassiveZeroMutationPolicySha256
        session_id_sha256 = Get-PassiveUtf8Sha256 $sessionId
        capture_id_sha256 = Get-PassiveUtf8Sha256 $captureId
        control_sha256 =
            (Get-FileHash -LiteralPath $collectorControlPath -Algorithm SHA256).Hash.ToLowerInvariant()
        pipe_token_sha256 = Get-PassiveUtf8Sha256 $pipeToken
        ready_payload_sha256 = $hexA
        host_binding_sha256 = $hexA
        host_validated = $true
        public_receipt_core_sha256 = $hexB
        ready_ack_sha256 = $hexA
    }
    $ready.ready_payload_sha256 = Get-PassiveCaptureReadyPayloadSha256 `
        -Ready ([pscustomobject]$ready) `
        -PipeToken $pipeToken
    $hostBindingBytes = [Collections.Generic.List[byte]]::new()
    $hostBindingBytes.AddRange(
        [Text.UTF8Encoding]::new($false).GetBytes(
            "battle-plan.capture_ready.host_binding.v2`0"
        )
    )
    $hostBindingBytes.AddRange(
        (ConvertFrom-PassiveSha256 $ready.ready_payload_sha256)
    )
    Add-PassiveLittleEndianBytes $hostBindingBytes u32 $targetPid
    Add-PassiveLittleEndianBytes `
        $hostBindingBytes u64 $targetCreationFiletimeUtc
    Add-PassiveLittleEndianBytes $hostBindingBytes u32 $requestedDurationMs
    foreach ($digest in @(
        $targetExecutableSha256,
        $collectorDllSha256,
        $ready.session_id_sha256,
        $ready.capture_id_sha256,
        $ready.pipe_token_sha256
    )) {
        $hostBindingBytes.AddRange((ConvertFrom-PassiveSha256 $digest))
    }
    $ready.host_binding_sha256 =
        Get-PassiveBytesSha256 $hostBindingBytes.ToArray()
    $ready.ready_ack_sha256 = Get-PassiveCaptureReadyAckSha256 `
        -Ready ([pscustomobject]$ready) `
        -PipeToken $pipeToken
    $readyPath = Join-Path $captureRun 'CAPTURE_READY_V2.json'
    [IO.File]::WriteAllText(
        $readyPath,
        ($ready | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false)
    )
    $teardown = [ordered]@{
        schema = 'hoi4-capture-teardown-v2'
        status = 'collector-unloaded'
        capture_policy = 'passive-observer-only'
        pid = $targetPid
        process_creation_filetime_utc = $targetCreationFiletimeUtc
        session_id = $sessionId
        capture_id = $captureId
        session_id_sha256 = $ready.session_id_sha256
        capture_id_sha256 = $ready.capture_id_sha256
        pipe_token_sha256 = $ready.pipe_token_sha256
        collector_dll_sha256 = $collectorDllSha256
        capture_ready_payload_sha256 = $ready.ready_payload_sha256
        capture_ready_receipt_core_sha256 = $ready.public_receipt_core_sha256
        capture_ready_ack_sha256 = $ready.ready_ack_sha256
        capture_ready_receipt_sha256 =
            (Get-FileHash -LiteralPath $readyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        host_binding_sha256 = $ready.host_binding_sha256
        data_pipe_eof_observed = $true
        hook_lease_released_after_eof = $true
        collector_module_disposition = 'unloaded'
        collector_module_retention_policy = 'inline-detour.module.process-lifetime.v1'
        target_process_epoch_revalidated = $true
        target_code_temporarily_instrumented = $true
        pinned_hook_target_code_restored = $true
        semantic_game_state_mutated = $false
        pinned_hook_sites_restored = $true
        pinned_hook_site_count = [int]$hookAuthority.Count
        pinned_hook_site_baseline_sha256 = $hexC
        pinned_hook_site_restored_sha256 = $hexC
        # Lane AS added `pinned_hook_import_slot_count` to the recorder's
        # teardown record (recorder_host/src/main.rs) and to the exact-property
        # pin in Test-PassiveCaptureTeardownReceipt; this fixture was not
        # updated with it, so every promotion case here died 28-vs-29 before
        # reaching its own assertion. Take the count from the same canonical
        # roster authority the verifier compares against -- a literal would
        # stop tracking the roster and make the check vacuous.
        pinned_hook_import_slot_count = [int]$hookAuthority.ImportSlotCount
    }
    $teardownPath = Join-Path $captureRun 'CAPTURE_TEARDOWN_V1.json'
    [IO.File]::WriteAllText(
        $teardownPath,
        ($teardown | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false)
    )
    $speedMonitorPath = Join-Path $captureRun 'speed5_monitor.jsonl'
    [IO.File]::WriteAllText(
        $speedMonitorPath,
        (@(
            '{"game_hour_raw":100,"speed_raw":4,"paused":false}',
            '{"game_hour_raw":100,"speed_raw":4,"paused":false}',
            '{"game_hour_raw":101,"speed_raw":4,"paused":false}'
        ) -join "`n") + "`n",
        [Text.UTF8Encoding]::new($false)
    )
    $speedPath = Join-Path $captureRun 'speed5_proof.json'
    $speed = [ordered]@{
        schema = 'battle-plan.speed5_proof.v2'
        session_id = $sessionId
        acceptance_mode = 'production'
        pid = $targetPid
        exe_sha256 = $targetExecutableSha256
        monitor_sha256 =
            (Get-FileHash -LiteralPath $speedMonitorPath -Algorithm SHA256).Hash.ToLowerInvariant()
        collector_control_sha256 =
            (Get-FileHash -LiteralPath $collectorControlPath -Algorithm SHA256).Hash.ToLowerInvariant()
        domain57_game_speed_gate_v2_sha256 =
            (Get-FileHash -LiteralPath $speedRootGatePath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_raw = 4
        paused = $false
        sample_count = 3
        first_game_hour_raw = 100
        last_game_hour_raw = 101
        capture_first_game_hour_raw = 100
        capture_last_game_hour_raw = 101
        capture_plan_sha256 =
            (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [IO.File]::WriteAllText($speedPath, ($speed | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false))
    $streamPath = Join-Path $captureRun 'collector_stream.bin'
    [IO.File]::WriteAllBytes(
        $streamPath,
        [Text.Encoding]::ASCII.GetBytes('synthetic-stream-bound-by-speed-gate-report')
    )
    $speedGateReportPath =
        Join-Path $captureRun 'independent_validation\speed5_stream_gate.report.json'
    [IO.Directory]::CreateDirectory(
        (Split-Path -Parent $speedGateReportPath)
    ) | Out-Null
    $speedGateReport = [ordered]@{
        schema = 'battle-plan.speed5_stream_gate.v2'
        passed = $true
        completed_frames = 2
        first_game_hour_raw = 100
        last_game_hour_raw = 101
        monitor_samples = 3
        domain57_game_speed_gate_v2_sha256 =
            (Get-FileHash -LiteralPath $speedRootGatePath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_evidence_transcript_sha256 = $hexA
        speed_evidence_transcript_file_sha256 =
            (Get-FileHash -LiteralPath $speedEvidenceTranscriptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        nonce_seal_sha256 = $hexC
        collector_control_sha256 =
            (Get-FileHash -LiteralPath $collectorControlPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [IO.File]::WriteAllText(
        $speedGateReportPath,
        ($speedGateReport | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false)
    )
    $speedStreamVerifierPath =
        Join-Path $fixtureScripts 'verify_speed5_stream.py'
    $speedEvidenceVerifierPath = $speedStreamVerifierPath
    $speedEvidenceVerifierSha =
        (Get-FileHash -LiteralPath $speedEvidenceVerifierPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $speedGateArguments = @(
        $speedStreamVerifierPath,
        '--stream', $streamPath,
        '--manifest', $manifestPath,
        '--speed-proof', $speedPath,
        '--monitor', $speedMonitorPath,
        '--capture-plan', $planPath,
        '--root-gate', $speedRootGatePath,
        '--collector-control', $collectorControlPath,
        '--evidence-transcript', $speedEvidenceTranscriptPath,
        '--nonce-ledger-root', $speedEvidenceNonceLedgerRoot,
        '--evidence-verifier', $speedEvidenceVerifierPath,
        '--expected-verifier-sha256', $speedEvidenceVerifierSha,
        '--expected-probe-sha256', $hexB,
        '--expected-static-sha256', $hexC,
        '--out', $speedGateReportPath
    )
    $validationPath = Join-Path $captureRun 'production_validation_report.json'
    $validation = [ordered]@{
        schema = 'battle-plan.production_capture_validation.v2'
        ok = $true
        problems = @()
        manifest_sha256 =
            (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        expected_domain_count = $domains.Count
        expected_domain_roster = $canonicalRoster
        independent_gates = @(
            [ordered]@{
                name = 'compact_gate'
                passed = $true
                arguments = @('--run-dir', $captureRun, '--require-zero-loss')
            },
            [ordered]@{
                name = 'speed5_stream_gate'
                passed = $true
                started = $true
                exit_code = 0
                timed_out = $false
                problem = $null
                executable_sha256 = $hexA
                arguments = $speedGateArguments
            }
        )
    }
    [IO.File]::WriteAllText($validationPath, ($validation | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))
    $sinkPath = Join-Path $captureRun 'sink_reconciliation_verification.json'
    $sink = [ordered]@{
        schema = 'extractor.sink-reconciliation-verification.v2'
        run_dir = $captureRun
        expected_outcome = 'success'
        verdict = 'passed'
        problems = @()
        evidence = [ordered]@{
            manifest_sha256 =
                (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
            production_capture_plan_sha256 =
                (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
            speed5_proof_sha256 =
                (Get-FileHash -LiteralPath $speedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    [IO.File]::WriteAllText($sinkPath, ($sink | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false))
    $captureReceiptPath = Join-Path $captureRun 'capture_validation_receipt.json'
    $captureReceipt = [ordered]@{
        schema = 'battle-plan.production_capture_validation_receipt.v1'
        capture_id = $captureId
        session_id = $sessionId
        acceptance_mode = 'production'
        admissible_for_production = $true
        receipt_path = $captureReceiptPath
        capture_plan_path = $planPath
        capture_plan_sha256 =
            (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
        manifest_path = $manifestPath
        manifest_sha256 =
            (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        capture_ready_receipt_path = $readyPath
        capture_ready_receipt_sha256 =
            (Get-FileHash -LiteralPath $readyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        capture_teardown_receipt_path = $teardownPath
        capture_teardown_receipt_sha256 =
            (Get-FileHash -LiteralPath $teardownPath -Algorithm SHA256).Hash.ToLowerInvariant()
        validation_report_path = $validationPath
        validation_report_sha256 =
            (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        sink_reconciliation_report_path = $sinkPath
        sink_reconciliation_report_sha256 =
            (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
        prebuilt_artifact_attestation_path = $retainedAttestationPath
        prebuilt_artifact_attestation_sha256 =
            (Get-FileHash -LiteralPath $retainedAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        prebuilt_source_input_sha256 = [string]$attested.source_input_sha256
        interaction_closure = [ordered]@{
            policy = 'passive_injected_observer'
            client_commands_during_capture = 0
            semantic_engine_state_writes = 0
            game_process_lifecycle_commands = 0
            game_ui_commands = 0
        }
    }
    [IO.File]::WriteAllText($captureReceiptPath, ($captureReceipt | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))

    Remove-Item -LiteralPath $zip -Force
    $builderResult = Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
        '-RecorderExe', $recorder,
        '-ReplayExe', $replay,
        '-CollectorDll', $collector,
        '-OutputZip', $zip,
        '-Version', '0.1.0',
        '-ProductionCaptureReceiptPath', $captureReceiptPath,
        '-ProductionArtifactAttestationPath', $attestationPath
    )
    Assert-Case 'builder promotes only exact current real-capture evidence' (
        $builderResult.admissible_for_production -is [bool] -and
        $builderResult.admissible_for_production -and
        [string]$builderResult.production_capture_evidence.schema -ceq
            'battle-plan.production_capture_promotion_evidence.v1' -and
        $builderResult.production_capture_evidence.all_domains_real_source_complete -and
        $builderResult.production_capture_evidence.zero_loss -and
        [int]$builderResult.production_capture_evidence.speed_raw -eq 4 -and
        $builderResult.production_capture_evidence.unpaused
    )
    $fixtureAttachLog = Join-Path $root 'extractor\native\collector\src\attach_log.rs'
    $fixtureAttachLogOriginal = [IO.File]::ReadAllText($fixtureAttachLog)
    [IO.File]::WriteAllText(
        $fixtureAttachLog,
        ($fixtureAttachLogOriginal +
            @'

fn synthetic_structural_d71_passive_gate() {
    if passive_product_capture {
        if compact_explicit_domain_enabled(
            &control,
            RAW_DYNAMIC_COLLECTION_TRACE_LIVE_DOMAIN,
        ) {
            return Err("synthetic passive D71 rejection".into());
        }
    }
}
'@),
        [Text.UTF8Encoding]::new($false)
    )
    try {
        $blockedD71Closure =
            Get-ProductionPassiveSourceAdmissionClosure -RepoRoot $root
        Assert-Case 'statically impossible D71 passive source is exact and non-admissible' (
            -not $blockedD71Closure.no_statically_proven_impossible_production_sources -and
            @($blockedD71Closure.statically_impossible_domains).Count -eq 1 -and
            [int]$blockedD71Closure.statically_impossible_domains[0].id -eq 71
        )
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureAttachLog,
            $fixtureAttachLogOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }
    $manifestOriginal = [IO.File]::ReadAllText($manifestPath)
    $validationOriginal = [IO.File]::ReadAllText($validationPath)
    $sinkOriginal = [IO.File]::ReadAllText($sinkPath)
    $speedOriginal = [IO.File]::ReadAllText($speedPath)
    $readyOriginal = [IO.File]::ReadAllText($readyPath)
    $teardownOriginal = [IO.File]::ReadAllText($teardownPath)
    $captureReceiptOriginal = [IO.File]::ReadAllText($captureReceiptPath)
    try {
        $mutatedManifest = $manifestOriginal | ConvertFrom-Json
        $d71Domain = @($domains | Where-Object { [int]$_.id -eq 71 })
        if ($d71Domain.Count -ne 1) {
            throw 'production promotion self-test requires exact D71 registry membership'
        }
        $d71WireName = [string]$d71Domain[0].name
        $mutatedManifest.compact_stream.per_domain.$d71WireName.status_flags_or = 0x100
        [IO.File]::WriteAllText($manifestPath, ($mutatedManifest | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false))
        $mutatedManifestSha =
            (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $mutatedValidation = $validationOriginal | ConvertFrom-Json
        $mutatedValidation.manifest_sha256 = $mutatedManifestSha
        [IO.File]::WriteAllText($validationPath,
            ($mutatedValidation | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false))
        $mutatedSink = $sinkOriginal | ConvertFrom-Json
        $mutatedSink.evidence.manifest_sha256 = $mutatedManifestSha
        [IO.File]::WriteAllText($sinkPath, ($mutatedSink | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false))
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.manifest_sha256 = $mutatedManifestSha
        $mutatedReceipt.validation_report_sha256 =
            (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $mutatedReceipt.sink_reconciliation_report_sha256 =
            (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText($captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false))
        Invoke-ExpectFailure 'D71 source-unavailable status cannot be masked into promotion' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($manifestPath, $manifestOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($validationPath, $validationOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($sinkPath, $sinkOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    $invokeManifestPromotionFailure = {
        param(
            [Parameter(Mandatory = $true)][string]$Name,
            [Parameter(Mandatory = $true)][scriptblock]$Mutate
        )
        try {
            $mutatedManifest = $manifestOriginal | ConvertFrom-Json
            & $Mutate $mutatedManifest
            [IO.File]::WriteAllText(
                $manifestPath,
                ($mutatedManifest | ConvertTo-Json -Depth 10),
                [Text.UTF8Encoding]::new($false)
            )
            $mutatedManifestSha =
                (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
            $mutatedValidation = $validationOriginal | ConvertFrom-Json
            $mutatedValidation.manifest_sha256 = $mutatedManifestSha
            [IO.File]::WriteAllText(
                $validationPath,
                ($mutatedValidation | ConvertTo-Json -Depth 10),
                [Text.UTF8Encoding]::new($false)
            )
            $mutatedSink = $sinkOriginal | ConvertFrom-Json
            $mutatedSink.evidence.manifest_sha256 = $mutatedManifestSha
            [IO.File]::WriteAllText(
                $sinkPath,
                ($mutatedSink | ConvertTo-Json -Depth 8),
                [Text.UTF8Encoding]::new($false)
            )
            $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
            $mutatedReceipt.manifest_sha256 = $mutatedManifestSha
            $mutatedReceipt.validation_report_sha256 =
                (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
            $mutatedReceipt.sink_reconciliation_report_sha256 =
                (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
            [IO.File]::WriteAllText(
                $captureReceiptPath,
                ($mutatedReceipt | ConvertTo-Json -Depth 10),
                [Text.UTF8Encoding]::new($false)
            )
            Invoke-ExpectFailure $Name {
                Get-ProductionCapturePromotionEvidence `
                    -RepoRoot $root `
                    -CaptureReceiptPath $captureReceiptPath `
                    -CurrentArtifactAttestationPath $attestationPath | Out-Null
            }
        }
        finally {
            [IO.File]::WriteAllText(
                $manifestPath,
                $manifestOriginal,
                [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::WriteAllText(
                $validationPath,
                $validationOriginal,
                [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::WriteAllText(
                $sinkPath,
                $sinkOriginal,
                [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::WriteAllText(
                $captureReceiptPath,
                $captureReceiptOriginal,
                [Text.UTF8Encoding]::new($false)
            )
        }
    }
    & $invokeManifestPromotionFailure `
        'unknown compact status bit cannot be promoted' `
        {
            param($Value)
            $firstProperty = @($Value.compact_stream.per_domain.PSObject.Properties)[0]
            # Bit 16, not 15. Bit 15 became COMPACT_STATUS_BASELINE_CLEAR (the
            # assigned deliberate-resync primitive) and bit 14 is
            # COMPACT_STATUS_SOURCE_EMPTY (assigned informational), so promotion
            # correctly accepts both and either would stop this fixture
            # producing the failure it asserts. The rule under test is
            # unchanged: an UNASSIGNED status bit blocks promotion, and bit 16
            # is the lowest unassigned bit.
            $firstProperty.Value.status_flags_or = 0x10000
        }
    & $invokeManifestPromotionFailure `
        'missing manifest speed boundary cannot be promoted' `
        {
            param($Value)
            $Value.first_game_hour_raw = $null
        }

    try {
        $mutatedReady = $readyOriginal | ConvertFrom-Json
        $mutatedReady.host_validated = $false
        [IO.File]::WriteAllText(
            $readyPath,
            ($mutatedReady | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.capture_ready_receipt_sha256 =
            (Get-FileHash -LiteralPath $readyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'unauthenticated passive READY cannot be promoted' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($readyPath, $readyOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    try {
        $mutatedTeardown = $teardownOriginal | ConvertFrom-Json
        $mutatedTeardown.semantic_game_state_mutated = $true
        [IO.File]::WriteAllText(
            $teardownPath,
            ($mutatedTeardown | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.capture_teardown_receipt_sha256 =
            (Get-FileHash -LiteralPath $teardownPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'semantic game-state mutation in teardown cannot be promoted' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($teardownPath, $teardownOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    try {
        $mutatedValidation = $validationOriginal | ConvertFrom-Json
        $mutatedValidation.independent_gates = @(
            $mutatedValidation.independent_gates |
                Where-Object { [string]$_.name -cne 'speed5_stream_gate' }
        )
        [IO.File]::WriteAllText(
            $validationPath,
            ($mutatedValidation | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.validation_report_sha256 =
            (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'missing independent speed-five stream gate cannot be promoted' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($validationPath, $validationOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    try {
        $mutatedSpeed = $speedOriginal | ConvertFrom-Json
        $mutatedSpeed.capture_first_game_hour_raw = 99
        [IO.File]::WriteAllText(
            $speedPath,
            ($mutatedSpeed | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedSink = $sinkOriginal | ConvertFrom-Json
        $mutatedSink.evidence.speed5_proof_sha256 =
            (Get-FileHash -LiteralPath $speedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $sinkPath,
            ($mutatedSink | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.sink_reconciliation_report_sha256 =
            (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'speed-five proof from a different capture interval is rejected' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($speedPath, $speedOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($sinkPath, $sinkOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    $packageReceiptPath = Join-Path $releaseRoot 'hoi4-portable-package-receipt.json'
    $receipt = [ordered]@{
        schema = 'battle-plan.central_portable_package.v1'
        status = 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE'
        successful_exit = $true
        central_validation = [ordered]@{
            mode = 'Release'
            requested_workspace = 'All'
            release_runtime_linkage = $runtimeLinkage
            production_feature_closure = [ordered]@{
                schema = 'battle-plan.production_feature_closure.v1'
                rebuild_order = @('recorder_host', 'collector_dll')
                recorder_host = [ordered]@{
                    package = 'recorder_host'
                    target = 'collector_record'
                    default_features = $false
                    enabled_features = @()
                    artifact_sha256 = [string]$attested.artifact_sha256['recorder_host']
                }
                collector_dll = [ordered]@{
                    package = 'collector'
                    target = 'lib'
                    crate_types = @('cdylib', 'rlib')
                    default_features = $false
                    enabled_features = @()
                    # This belongs to collector_dll, which is where
                    # Invoke-CentralRustValidation emits it and where the
                    # receipt verifier reads it. The fixture had it on
                    # recorder_host, where nothing produces it and nothing
                    # reads it, so the verifier hit a missing property under
                    # Set-StrictMode instead of checking the closure.
                    all_non_default_features_disabled = $true
                    forbidden_features = @(
                        'test-support',
                        'unshipped-a25-runtime-text-evidence',
                        'unshipped-a39-character-presentation-evidence',
                        'unshipped-a31-focus-presentation-evidence'
                    )
                    built_last = $true
                    collector_cargo_sha256 =
                        [string]$builderResult.interaction_closure.policy_inputs.collector_cargo_sha256
                    pinned_hook_count =
                        [int]$builderResult.interaction_closure.source_capability.exact_pinned_hook_count
                    pinned_hook_contract_source_sha256 =
                        [string]$builderResult.interaction_closure.policy_inputs.passive_ready_contract_sha256
                    artifact_sha256 = [string]$attested.artifact_sha256['collector_dll']
                }
            }
        }
        artifact_attestation = [ordered]@{
            path = [string]$attested.receipt_path
            sha256 = [string]$attested.receipt_sha256
        }
        package_builder = [ordered]@{
            path = [IO.Path]::GetFullPath($fixtureBuilder)
            sha256 = (Get-FileHash -LiteralPath $fixtureBuilder -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        package = $builderResult
        source_input_sha256 = [string]$attested.source_input_sha256
        build_environment = $attested.build_environment
    }
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($receipt | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )

    # ----------------------------------------------------------------------
    # The receipt-verifier family.
    #
    # Twelve child processes -- nine receipt verifications and three package
    # builds -- against the SAME fixture root. Measured serially on this
    # repository they were 177 s of a 770 s suite, because almost every one of
    # them re-derives the whole production source tree before it reaches the
    # check it exists to exercise.
    #
    # What made them serial was one FILE, not shared state in general: the
    # canonical receipt under .cargo-target\release, which is the only receipt
    # the verifier will look at, so a case that wanted a corrupt receipt had to
    # corrupt that one and put it back.
    #
    # -AllowStagedReceipt removes that constraint without loosening anything.
    # It is not a switch invented here and it is not a debug affordance: it is
    # the mode the release pipeline uses to verify a receipt it has staged but
    # not yet published, and it is exactly as strict about where the file may
    # live -- the same release directory, and a name that must match
    # hoi4-portable-package-receipt.json.<32 hex>.candidate. Everything between
    # that path rule and the emitted status string -- the package, the artifact
    # attestation, the promotion evidence, the source-capability closure, the
    # ZIP entry bytes -- is the same code reading the same tree. Production
    # receipt verification is the invocation WITHOUT this switch, and neither it
    # nor any line of Test-PortableProductPackageReceipt.ps1 is changed.
    #
    # So each case gets its own candidate file, its own corruption, and its own
    # worker. The package-bytes case additionally gets its own PACKAGE: it
    # declares a distinct version, which is what the verifier derives the
    # expected ZIP path from, so the corrupt copy it plants is a file no other
    # case reads and the check it fires -- ZIP bytes against the receipt's
    # sha256 -- is the same one it fired serially. The three builder cases
    # already wrote to uniquely named output ZIPs and are dispatched unchanged.
    #
    # The control is what makes a refusal mean something. An unmutated candidate
    # is verified in the same batch and MUST be accepted. Without it, a
    # candidate rejected merely for being staged -- or a receipt that no longer
    # survives the ConvertFrom-Json/ConvertTo-Json round trip through which
    # every one of these mutations is applied -- would make each rejection below
    # true for a reason that has nothing to do with its planted corruption. That
    # hole was open while these cases were serial: they all round-tripped the
    # receipt and none of them ever asked whether a round trip alone still
    # verified. It is a precondition rather than a case, so it throws instead of
    # reporting a verdict, and it adds no name to the sequence.
    # ----------------------------------------------------------------------
    $receiptPoolControlName = 'control: staged candidate with no mutation'
    $stagedPackageVersion = '0.1.0-staged-package-bytes'
    $stagedPackageZipName =
        'battle-plan-{0}-windows-x86_64.zip' -f $stagedPackageVersion
    $receiptCases = @(
        (New-SelfTestReceiptCase `
            -Name $receiptPoolControlName `
            -Kind 'verifier_candidate'),
        (New-SelfTestReceiptCase `
            -Name 'verifier accepts authentic fixture' `
            -Kind 'verifier_canonical'),
        (New-SelfTestReceiptCase `
            -Name 'tampered package bytes are rejected' `
            -Kind 'verifier_candidate' `
            -Mutations @(
                (New-SelfTestReceiptMutation `
                    -Property 'package.version' `
                    -ValueKind 'string' `
                    -Value $stagedPackageVersion),
                (New-SelfTestReceiptMutation `
                    -Property 'package.path' `
                    -ValueKind 'releasePath' `
                    -Value $stagedPackageZipName)
            ) `
            -ZipCopy ([pscustomobject]@{
                SourceName = 'battle-plan-0.1.0-windows-x86_64.zip'
                DestinationName = $stagedPackageZipName
                TamperLastByte = $true
            })),
        (New-SelfTestReceiptCase `
            -Name 'tampered central receipt is rejected' `
            -Kind 'verifier_candidate' `
            -Mutations @(
                (New-SelfTestReceiptMutation `
                    -Property 'status' `
                    -ValueKind 'string' `
                    -Value 'PASS_WITHOUT_AUTHORITY')
            )),
        (New-SelfTestReceiptCase `
            -Name 'tampered production source-capability closure is rejected' `
            -Kind 'verifier_candidate' `
            -Mutations @(
                (New-SelfTestReceiptMutation `
                    -Property 'package.interaction_closure.source_capability.collector_fixture_hooks_default_off' `
                    -ValueKind 'bool' `
                    -Value $false)
            )),
        (New-SelfTestReceiptCase `
            -Name 'tampered production feature closure is rejected' `
            -Kind 'verifier_candidate' `
            -Mutations @(
                (New-SelfTestReceiptMutation `
                    -Property 'central_validation.production_feature_closure.collector_dll.built_last' `
                    -ValueKind 'bool' `
                    -Value $false)
            )),
        (New-SelfTestReceiptCase `
            -Name 'tampered interaction-closure verdict is rejected' `
            -Kind 'verifier_candidate' `
            -Mutations @(
                (New-SelfTestReceiptMutation `
                    -Property 'package.interaction_closure.recorder_host.forbidden_symbols_absent' `
                    -ValueKind 'bool' `
                    -Value $false)
            )),
        (New-SelfTestReceiptCase `
            -Name 'tampered interaction-closure marker roster is rejected' `
            -Kind 'verifier_candidate' `
            -Mutations @(
                (New-SelfTestReceiptMutation `
                    -Property 'package.interaction_closure.collector_dll.required_markers' `
                    -ValueKind 'emptyArray')
            )),
        (New-SelfTestReceiptCase `
            -Name 'tampered passive policy-input digest is rejected' `
            -Kind 'verifier_candidate' `
            -Mutations @(
                (New-SelfTestReceiptMutation `
                    -Property 'package.interaction_closure.policy_inputs.read_only_game_state_witness_sha256' `
                    -ValueKind 'string' `
                    -Value (('f' * 64) -join ''))
            )),
        (New-SelfTestReceiptCase `
            -Name 'DLL cannot masquerade as recorder executable' `
            -Kind 'builder' `
            -Arguments @(
                '-RecorderExe', "@ROOT@\$collectorRelative",
                '-ReplayExe', "@ROOT@\$replayRelative",
                '-CollectorDll', "@ROOT@\$collectorRelative",
                '-OutputZip', "@ROOT@\$releaseRelative\wrong-kind.zip",
                '-Version', '0.1.0'
            )),
        (New-SelfTestReceiptCase `
            -Name 'post-temp builder failure is rejected' `
            -Kind 'builder' `
            -Arguments @(
                '-RecorderExe', "@ROOT@\$recorderRelative",
                '-ReplayExe', "@ROOT@\$replayRelative",
                '-CollectorDll', "@ROOT@\$collectorRelative",
                '-OutputZip', "@ROOT@\$releaseRelative\post-temp-cleanup.zip",
                '-Version', '0.1.0',
                '-TestOnlyFailAfterTemporaryWrite'
            )),
        (New-SelfTestReceiptCase `
            -Name 'post-publication builder failure is rejected' `
            -Kind 'builder' `
            -Arguments @(
                '-RecorderExe', "@ROOT@\$recorderRelative",
                '-ReplayExe', "@ROOT@\$replayRelative",
                '-CollectorDll', "@ROOT@\$collectorRelative",
                '-OutputZip', "@ROOT@\$releaseRelative\post-publish-cleanup.zip",
                '-Version', '0.1.0',
                '-TestOnlyFailAfterPublishedWrite'
            ))
    )

    # The pool's request and response files are NOT put under the fixture root.
    # The production input digest the verifier recomputes covers every .json and
    # .ps1 under extractor\native, so a request file landing there would change
    # the tree these cases are measuring while they measure it.
    $receiptPoolContainer = Join-Path ([IO.Path]::GetTempPath()) (
        'hoi4pr' + [guid]::NewGuid().ToString('N').Substring(0, 8)
    )
    if (Test-Path -LiteralPath $receiptPoolContainer) {
        throw "self-test receipt pool container already exists: $receiptPoolContainer"
    }
    [IO.Directory]::CreateDirectory($receiptPoolContainer) | Out-Null
    $receiptPoolContainerInfo = [IO.DirectoryInfo]::new($receiptPoolContainer)
    if (($receiptPoolContainerInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "self-test receipt pool container is a reparse point: $receiptPoolContainer"
    }
    # ONE filesystem operation that can only succeed for one caller; see the
    # private-tree container above for why "check then create" is a race.
    $receiptPoolClaim = [IO.File]::Open(
        (Join-Path $receiptPoolContainer '.selftest-owner'),
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None
    )
    # Recorded the instant the claim is won, so anything that fails below leaves
    # a container the outer teardown will remove rather than an orphan.
    $receiptPoolContainerOwned = $receiptPoolContainer
    try { $receiptPoolClaim.WriteByte(0) } finally { $receiptPoolClaim.Dispose() }
    $receiptRows = $null
    $receiptPoolCleanupProblem = ''
    try {
        $receiptRows = Invoke-SelfTestReceiptCasePool `
            -Root $root `
            -Cases @($receiptCases) `
            -ContainerRoot $receiptPoolContainer `
            -Label 'receipts'
    }
    finally {
        # Recorded, not thrown: throwing from a finally would replace the real
        # failure with a janitorial one. Warned here as well as thrown below,
        # because a leak on a FAILING run would otherwise be recorded and then
        # silently discarded when the primary exception propagates.
        try {
            Remove-Item -LiteralPath $receiptPoolContainer -Recurse -Force `
                -ErrorAction Stop
            # Ownership ENDS here, so the outer teardown cannot remove a
            # container a later run has since claimed under the same short name.
            $receiptPoolContainerOwned = $null
        }
        catch {
            $receiptPoolCleanupProblem = [string]$_.Exception.Message
            Write-Warning (
                'self-test receipt pool container was left behind: ' +
                "$receiptPoolContainer ($receiptPoolCleanupProblem)"
            ) -WarningAction Continue
        }
    }
    if ($receiptPoolCleanupProblem) {
        throw (
            'self-test receipt pool container was left behind: ' +
            "$receiptPoolContainer ($receiptPoolCleanupProblem)"
        )
    }

    Write-SelfTestCaseTiming -Name 'receipt-verifier family (parallel workers)'

    # The control, before any verdict from this pool is trusted.
    $receiptControlRow = $receiptRows[$receiptPoolControlName]
    if (-not [bool]$receiptControlRow.accepted) {
        throw (
            'self-test receipt pool control failed: the receipt verifier refused an ' +
            'unmutated staged candidate (' + [string]$receiptControlRow.message + ')'
        )
    }
    if ([string]$receiptControlRow.output_status -cne
        'PASS_STAGED_CENTRAL_RELEASE_PORTABLE_PACKAGE') {
        throw (
            'self-test receipt pool control returned an unexpected staged verdict: ' +
            [string]$receiptControlRow.output_status
        )
    }

    # Named individually, in the order the serial form ran them, so the case
    # names, their order and their count are exactly what they were.
    $verified = $receiptRows['verifier accepts authentic fixture']
    Assert-Case 'verifier accepts authentic fixture' (
        [bool]$verified.accepted -and
        [string]$verified.output_status -ceq 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE'
    )
    Assert-Case 'verifier returns builder package hash' (
        [string]$verified.output_package_sha256 -ceq [string]$builderResult.sha256
    )
    foreach ($refusedCaseName in @(
        'tampered package bytes are rejected',
        'tampered central receipt is rejected',
        'tampered production source-capability closure is rejected',
        'tampered production feature closure is rejected',
        'tampered interaction-closure verdict is rejected',
        'tampered interaction-closure marker roster is rejected',
        'tampered passive policy-input digest is rejected',
        'DLL cannot masquerade as recorder executable',
        'post-temp builder failure is rejected'
    )) {
        Assert-Case $refusedCaseName (
            -not [bool]$receiptRows[$refusedCaseName].accepted
        )
    }
    $cleanupZip = Join-Path $releaseRoot 'post-temp-cleanup.zip'
    Assert-Case 'post-temp failure publishes no package' (-not (Test-Path -LiteralPath $cleanupZip))
    $cleanupTemporary = @(
        Get-ChildItem -LiteralPath $releaseRoot -Filter '.post-temp-cleanup.zip.*.tmp' -Force
    )
    Assert-Case 'post-temp failure removes unpublished temporary package' ($cleanupTemporary.Count -eq 0)
    Assert-Case 'post-publication builder failure is rejected' (
        -not [bool]$receiptRows['post-publication builder failure is rejected'].accepted
    )
    $publishedCleanupZip = Join-Path $releaseRoot 'post-publish-cleanup.zip'
    Assert-Case 'post-publication failure removes published package' (
        -not (Test-Path -LiteralPath $publishedCleanupZip)
    )

    [pscustomobject]@{
        schema = 'battle-plan.portable_package_selftest.v1'
        status = 'PASS'
        cases = $script:PortablePackageSelfTestCases
    } | ConvertTo-Json -Depth 3
}
finally {
    # The private-tree container is torn down on the success path by the
    # source-rejection segment, which THROWS if it cannot: leaving full tree
    # copies in %TEMP% is a defect in this suite, not something to discover
    # later as unexplained disk use. This is the failure path only -- a run that
    # died before that segment -- so it is best effort and must not displace the
    # exception that is already propagating.
    # privateTreeContainerOwned, not privateTreeContainer: the second is a
    # candidate path this run may never have owned, and removing another run's
    # container while its workers are reading it is exactly the corruption the
    # atomic claim exists to prevent.
    # Same reasoning for the receipt pool's own container, which is claimed the
    # same way, released the same way on its success path, and is therefore only
    # still owned here on a run that died inside the pool.
    foreach ($ownedVariableName in @(
        'privateTreeContainerOwned',
        'receiptPoolContainerOwned'
    )) {
        $containerVariable = Get-Variable -Name $ownedVariableName `
            -Scope Script -ErrorAction SilentlyContinue
        if ($containerVariable -and
            -not [string]::IsNullOrEmpty([string]$containerVariable.Value) -and
            (Test-Path -LiteralPath ([string]$containerVariable.Value))) {
            try {
                Remove-Item -LiteralPath ([string]$containerVariable.Value) `
                    -Recurse -Force -ErrorAction Stop
            }
            catch {
                Write-Warning (
                    'self-test temporary container was left behind: ' +
                    [string]$containerVariable.Value + ' (' + $_.Exception.Message + ')'
                ) -WarningAction Continue
            }
        }
    }
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
