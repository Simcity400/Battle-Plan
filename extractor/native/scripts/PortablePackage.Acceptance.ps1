# Package custody around the existing publication verifier and viewer CLI seam.
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression
. (Join-Path $PSScriptRoot 'PortableProduct.PackageContents.ps1')
. (Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1')
# The driver's dot-source guard loads functions only; it launches nothing.
. (Join-Path $PSScriptRoot 'Invoke-ViewerDesktopPlaybackValidation.ps1')

if (-not ('PpaFileLease' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class PpaFileLease {
    [StructLayout(LayoutKind.Sequential)] struct Info {
        public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Created, Accessed, Written;
        public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string path, uint access, uint share, IntPtr sa, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetFileInformationByHandle(SafeFileHandle h, out Info info);
    public static SafeFileHandle Open(string path, bool directory) {
        // No delete sharing: held ancestors cannot be renamed beneath a leased input.
        var h = CreateFileW(path, directory ? 0x80u : 0x80000000u, directory ? 3u : 1u,
            IntPtr.Zero, 3, 0x02200000, IntPtr.Zero);
        if (h.IsInvalid) { int error=Marshal.GetLastWin32Error(); h.Dispose(); throw new Win32Exception(error, "PPA_FILE_OPEN: " + path + " (Win32 " + error + ")"); }
        Info info;
        if (!GetFileInformationByHandle(h, out info)) { int error=Marshal.GetLastWin32Error(); h.Dispose(); throw new Win32Exception(error); }
        if ((info.Attributes & 0x400) != 0 || ((info.Attributes & 0x10) != 0) != directory) {
            h.Dispose(); throw new IOException("PPA_REPARSE_OR_TYPE: " + path);
        }
        return h;
    }
}
'@
}

function Assert-PpaLocalPath {
    param([string]$Path)
    if ($Path -notmatch '^[A-Za-z]:[\\/]') { throw 'PPA_LOCAL_PATH: require an explicit local drive path, not UNC/relative input.' }
    if ($Path.Substring(2).Contains(':')) { throw 'PPA_LOCAL_PATH: alternate streams are forbidden.' }
    $full = [IO.Path]::GetFullPath($Path)
    $drive=[IO.DriveInfo]::new([IO.Path]::GetPathRoot($full))
    if ($drive.DriveType -notin @([IO.DriveType]::Fixed,[IO.DriveType]::Removable)) { throw 'PPA_LOCAL_PATH: require an available local disk, not a mapped network drive.' }
    return $full
}

function Lock-PpaDirectory {
    param([string]$Path, [Collections.Generic.List[object]]$Leases)
    $full = Assert-PpaLocalPath $Path
    $parent = [IO.Directory]::GetParent($full)
    if ($null -ne $parent) { Lock-PpaDirectory $parent.FullName $Leases }
    $Leases.Add([PpaFileLease]::Open($full, $true))
}

function Open-PpaFile {
    param([string]$Path, [string]$ExpectedSha256, [Collections.Generic.List[object]]$Leases)
    Assert-VdpSha256 $ExpectedSha256 'parent-pinned file'
    $file=Open-PpaReadLease $Path $Leases
    if ($file.reference.sha256 -cne $ExpectedSha256) { throw "PPA_HASH_MISMATCH: $Path" }
    return $file
}

function Open-PpaReadLease {
    param([string]$Path, [Collections.Generic.List[object]]$Leases, [long]$MaxBytes=[long]::MaxValue)
    $full = Assert-PpaLocalPath $Path
    Lock-PpaDirectory ([IO.Path]::GetDirectoryName($full)) $Leases
    $handle = [PpaFileLease]::Open($full, $false)
    try { $stream = [IO.FileStream]::new($handle, [IO.FileAccess]::Read) }
    catch { $handle.Dispose(); throw }
    $Leases.Add($stream)
    if ($stream.Length -gt $MaxBytes) { throw 'PPA_READ_BOUND: leased output exceeds its byte limit.' }
    $identity = Get-WindowsPathIdentity -LiteralPath $full -OpenReparsePoint
    $hash = Get-PpaStreamHash $stream
    [pscustomobject]@{ stream=$stream; reference=[ordered]@{ path=$identity.final_path; bytes=$stream.Length; sha256=$hash;
        volume_serial_number=$identity.volume_serial_number; file_id_128=$identity.file_id_128 } }
}

function Get-PpaStreamHash {
    param([IO.Stream]$Stream)
    $Stream.Position = 0; $sha = [Security.Cryptography.SHA256]::Create()
    try { [BitConverter]::ToString($sha.ComputeHash($Stream)).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose(); $Stream.Position = 0 }
}

function Read-PpaJson {
    param($File)
    if ($File.stream.Length -gt 8MB) { throw 'PPA_JSON_BOUND: authority document exceeds 8 MiB.' }
    $reader = [IO.StreamReader]::new($File.stream, [Text.Encoding]::UTF8, $true, 4096, $true)
    try { $reader.ReadToEnd() | ConvertFrom-Json }
    finally { $reader.Dispose(); $File.stream.Position = 0 }
}

function Get-PpaPackageHashes {
    param($Receipt, $Verification, [string]$ReceiptSha, [string]$ZipSha)
    if ($Receipt.schema -cne 'battle-plan.central_portable_package.v1' -or
        $Receipt.status -cne 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
        $Receipt.successful_exit -isnot [bool] -or -not $Receipt.successful_exit -or
        $Receipt.package.schema -cne 'hoi4-portable-package.v1' -or
        $Receipt.package.admissible_for_production -isnot [bool] -or -not $Receipt.package.admissible_for_production -or
        $Verification.schema -cne 'battle-plan.central_portable_package_verification.v1' -or
        $Verification.status -cne 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
        $Verification.canonical_receipt -isnot [bool] -or -not $Verification.canonical_receipt -or
        $Verification.static_vcruntime_admission -isnot [bool] -or -not $Verification.static_vcruntime_admission -or
        $Verification.receipt_sha256 -cne $ReceiptSha -or $Verification.package_sha256 -cne $ZipSha -or
        $Receipt.package.sha256 -cne $ZipSha -or
        $Verification.build_environment_sha256 -cne $Receipt.build_environment.sha256) {
        throw 'PPA_PACKAGE_AUTHORITY: require parent-pinned successful canonical receipt and its existing verifier result.'
    }
    Assert-VdpSha256 $Verification.build_environment_sha256 'approved build environment'
    $hashes = $Receipt.package.input_sha256
    if ((@($hashes.PSObject.Properties.Name | Sort-Object) -join ',') -cne 'collector_dll,recorder_host,replay_app') {
        throw 'PPA_BINARY_ROLES: require exactly the three shipping roles.'
    }
    [ordered]@{ 'Battle Plan Recorder.exe'=[string]$hashes.recorder_host;
        'Battle Plan.exe'=[string]$hashes.replay_app; 'collector.dll'=[string]$hashes.collector_dll }
}

function Test-PpaArchive {
    param([IO.Compression.ZipArchive]$Archive, [string]$Version, [Collections.IDictionary]$Hashes,
        [string]$ExtractTo = '', [Collections.Generic.List[object]]$Leases)
    # This is the same closed five-entry/document verifier used by publication.
    Assert-PortableProductPackageContents -Archive $Archive -Version $Version -BinaryHashes $Hashes
    $references = [Collections.Generic.List[object]]::new(); $total = 0L
    foreach ($entry in $Archive.Entries) {
        if ($entry.Length -lt 1 -or $entry.Length -gt 256MB) { throw 'PPA_ENTRY_BOUND: entry exceeds 256 MiB or is empty.' }
        $total += $entry.Length
        if ($total -gt 512MB) { throw 'PPA_TOTAL_BOUND: decompressed package exceeds 512 MiB.' }
        $inputStream = $entry.Open(); $output = $null; $sha = [Security.Cryptography.SHA256]::Create()
        try {
            if ($ExtractTo) {
                $destination = Join-Path $ExtractTo $entry.FullName
                if (-not (Test-WindowsPathContainedByRoot -Candidate $destination -Root $ExtractTo) -or
                    [IO.Path]::GetDirectoryName($destination) -ine $ExtractTo) { throw 'PPA_EXTRACTION_ESCAPE' }
                $output = [IO.FileStream]::new($destination, 'CreateNew', 'Write', 'Read')
            }
            $buffer = [byte[]]::new(65536); $count = 0L
            while (($read = $inputStream.Read($buffer,0,$buffer.Length)) -gt 0) {
                $count += $read
                if ($count -gt $entry.Length) { throw 'PPA_ENTRY_LENGTH: decompression exceeds advertised bounded length.' }
                $null = $sha.TransformBlock($buffer,0,$read,$buffer,0)
                if ($null -ne $output) { $output.Write($buffer,0,$read) }
            }
            $null = $sha.TransformFinalBlock([byte[]]::new(0),0,0)
            if ($count -ne $entry.Length) { throw 'PPA_ENTRY_LENGTH: truncated entry.' }
            $hash = [BitConverter]::ToString($sha.Hash).Replace('-','').ToLowerInvariant()
            if ($Hashes.Contains($entry.FullName) -and $hash -cne $Hashes[$entry.FullName]) { throw "PPA_BINARY_HASH: $($entry.FullName)" }
            if ($null -ne $output) { $output.Flush($true) }
        } finally { if ($null -ne $output) { $output.Dispose() }; $inputStream.Dispose(); $sha.Dispose() }
        if ($ExtractTo) {
            $locked = Open-PpaFile $destination $hash $Leases
            $references.Add($locked.reference)
        } else { $references.Add([ordered]@{ entry=$entry.FullName; bytes=$count; sha256=$hash }) }
    }
    return @($references.ToArray())
}

function Get-PpaEnvironment {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        [ordered]@{ utc=[DateTime]::UtcNow.ToString('o'); machine=[Environment]::MachineName;
            user_sid=$identity.User.Value; non_elevated=(-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator));
            is_system=$identity.IsSystem; network_available=[Net.NetworkInformation.NetworkInterface]::GetIsNetworkAvailable() }
    } finally { $identity.Dispose() }
}

function Assert-PpaEnvironment {
    param($Sample)
    if ($Sample.non_elevated -isnot [bool] -or -not $Sample.non_elevated -or
        $Sample.is_system -isnot [bool] -or $Sample.is_system) { throw 'PPA_STANDARD_USER: require a non-elevated user token.' }
    if ($Sample.network_available -isnot [bool] -or $Sample.network_available) { throw 'PPA_OFFLINE: network must already be unavailable; this runner does not change network state.' }
}

function Invoke-PpaAcceptance {
    param([Collections.IDictionary]$Request)
    $repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
    $leases = [Collections.Generic.List[object]]::new(); $cases = [Collections.Generic.List[object]]::new()
    $samples = [Collections.Generic.List[object]]::new(); $archive=$null; $zip=$null; $root=$null
    $receipt = [ordered]@{ schema='hoi4.portable-package-acceptance.v1'; status='fail'; mode=$Request.Mode;
        machine=[Environment]::MachineName; inputs=@{}; harness=@(); plan=@(); entries=@(); extracted=@(); cases=@(); environment_samples=@();
        cli_playback_qualified=$false; package_after=$null; desktop_handoff=$null; error=$null;
        limitations=@('This Windows machine only; not clean-machine qualification.',
            'Consumes the explicitly approved canonical verification result; does not rerun source/build/capture promotion gates.',
            'Offline state is sampled before and after each bounded CLI case, not enforced by firewall or packet observation.',
            'CLI admission is not desktop/offscreen rendering or interaction qualification; desktop uses the separate existing driver.',
            'Single .battleplan only; package-set custody requires a separate all-member implementation.') }
    try {
        $receipt.harness=@(foreach($name in @('Invoke-PortablePackageAcceptance.ps1','PortablePackage.Acceptance.ps1',
            'PortableProduct.PackageContents.ps1','WindowsPath.Identity.ps1','Invoke-ViewerDesktopPlaybackValidation.ps1')) {
            Get-VdpReference (Join-Path $PSScriptRoot $name)
        })
        foreach ($key in @('ExpectedPackageSha256','ExpectedPackageReceiptSha256','ExpectedPackageVerificationSha256',
            'ExpectedRecordingSha256','ExpectedGameExeSha256','ExpectedDlcLoadSha256','ExpectedContentAuthoritySha256','ExpectedEffectiveVfsManifestSha256')) {
            Assert-VdpSha256 $Request[$key] $key
        }
        if ($Request.Mode -cnotin @('Prepare','Run') -or $Request.TimeoutSeconds -lt 5 -or $Request.TimeoutSeconds -gt 900) { throw 'PPA_REQUEST_INVALID' }
        $recording = Assert-PpaLocalPath $Request.RecordingPath
        if ([IO.Path]::GetExtension($recording) -ine '.battleplan') { throw 'PPA_RECORDING_SET_UNSUPPORTED' }
        $game = Assert-PpaLocalPath $Request.GameRoot; $user = Assert-PpaLocalPath $Request.UserDataRoot
        $outputPath = Assert-PpaLocalPath $Request.OutputDirectory
        if (-not $outputPath.StartsWith('C:\',[StringComparison]::OrdinalIgnoreCase)) { throw 'PPA_OUTPUT_LOCATION: use new C: scratch.' }
        foreach ($protected in @($repo,$game,$user)) {
            if (Test-WindowsPathContainedByRoot -Candidate (Resolve-WindowsPathThroughExistingAncestor $outputPath) -Root (Resolve-WindowsPathThroughExistingAncestor $protected)) { throw 'PPA_OUTPUT_LOCATION: output may not modify repository/content inputs.' }
        }
        if (Test-Path -LiteralPath $outputPath) { throw 'PPA_OUTPUT_EXISTS' }
        Lock-PpaDirectory ([IO.Path]::GetDirectoryName($outputPath)) $leases
        $null = [IO.Directory]::CreateDirectory($outputPath); $root=$outputPath
        Lock-PpaDirectory $root $leases
        foreach ($spec in @(
            @('package',$Request.PackageZip,$Request.ExpectedPackageSha256),
            @('package_receipt',$Request.PackageReceiptPath,$Request.ExpectedPackageReceiptSha256),
            @('package_verification',$Request.PackageVerificationPath,$Request.ExpectedPackageVerificationSha256),
            @('recording',$recording,$Request.ExpectedRecordingSha256),
            @('game_executable',(Join-Path $game 'hoi4.exe'),$Request.ExpectedGameExeSha256),
            @('dlc_load',(Join-Path $user 'dlc_load.json'),$Request.ExpectedDlcLoadSha256))) {
            $file = Open-PpaFile $spec[1] $spec[2] $leases
            $receipt.inputs[$spec[0]]=$file.reference
            switch ($spec[0]) {
                'package' { $zip=$file }
                'package_receipt' { $package=Read-PpaJson $file }
                'package_verification' { $verification=Read-PpaJson $file }
            }
        }
        $hashes = Get-PpaPackageHashes $package $verification $Request.ExpectedPackageReceiptSha256 $Request.ExpectedPackageSha256
        if ($zip.stream.Length -gt 512MB) { throw 'PPA_ZIP_BOUND: closed ZIP exceeds 512 MiB.' }
        $archive = [IO.Compression.ZipArchive]::new($zip.stream,[IO.Compression.ZipArchiveMode]::Read,$true)
        $receipt.entries = @(Test-PpaArchive $archive $package.package.version $hashes -Leases $leases)
        $installed = Join-Path $root 'installed'; $viewer = Join-Path $installed 'Battle Plan.exe'
        $profile = Join-Path $root 'playback-profile.json'; $authorityPath=Join-Path $root 'content-authority.json'
        $receipt.plan = @([ordered]@{ name='configure-roots'; argv=@('--configure','--game',$game,'--userdata',$user,'--config',$profile,'--non-interactive') }) +
            @(Get-VdpCliPlan $recording $game $user $authorityPath | Where-Object { $_.name -cne 'desktop' })
        $receipt.desktop_handoff = [ordered]@{ script=(Join-Path $PSScriptRoot 'Invoke-ViewerDesktopPlaybackValidation.ps1');
            script_sha256=(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'Invoke-ViewerDesktopPlaybackValidation.ps1') -Algorithm SHA256).Hash.ToLowerInvariant();
            parameters=[ordered]@{ Mode='Run'; ViewerExecutable=$viewer; ExpectedViewerSha256=$hashes['Battle Plan.exe'];
                RecordingPath=$recording; ExpectedRecordingSha256=$Request.ExpectedRecordingSha256; GameRoot=$game; UserDataRoot=$user;
                ExpectedContentAuthoritySha256=$Request.ExpectedContentAuthoritySha256; OutputDirectory=(Join-Path $root 'desktop'); TimeoutSeconds=60 };
            still_required=@('ExpectedHour','SeekHour','FirstCountryTag','SecondCountryTag','TooltipWindowId','TooltipElementId');
            custody_required_before_desktop=[ordered]@{ game_executable=$receipt.inputs.game_executable; dlc_load=$receipt.inputs.dlc_load;
                expected_effective_vfs_manifest_sha256=$Request.ExpectedEffectiveVfsManifestSha256;
                requirement='Reacquire these read leases for the entire separate desktop run and compare its admitted VFS manifest. The existing driver does not accept these additional pins.' };
            invoked=$false; limitation='Parent supplies admitted interaction targets and schedules the existing driver separately.' }
        $sample = Get-PpaEnvironment; $samples.Add($sample)
        if ($Request.Mode -ceq 'Prepare') { $receipt.status='prepared_not_run' }
        else {
            Assert-PpaEnvironment $sample
            $null = [IO.Directory]::CreateDirectory($installed); Lock-PpaDirectory $installed $leases
            $receipt.extracted = @(Test-PpaArchive $archive $package.package.version $hashes -ExtractTo $installed -Leases $leases)
            foreach ($case in $receipt.plan) {
                $sample=Get-PpaEnvironment; $samples.Add($sample); Assert-PpaEnvironment $sample
                # Existing bounded CLI execution: scrubbed child PATH/environment,
                # exact PID/creation/image custody, whole-playback receipts, raw logs.
                Invoke-VdpProcess $case $viewer $root $Request.TimeoutSeconds $null 0 $cases
                $sample=Get-PpaEnvironment; $samples.Add($sample); Assert-PpaEnvironment $sample
                if ($case.name -ceq 'configure-roots') {
                    $profileFile=Open-PpaReadLease $profile $leases 8MB
                    $stored=Read-PpaJson $profileFile
                    if ($stored.schema_version -ne 1 -or [IO.Path]::GetFullPath($stored.game_install) -ine $game -or
                        [IO.Path]::GetFullPath($stored.user_data) -ine $user) { throw 'PPA_ROOT_PROFILE_MISMATCH' }
                    $receipt['root_profile']=$profileFile.reference
                }
                if ($case.name -ceq 'content-authority') {
                    $authorityFile=Open-PpaReadLease $authorityPath $leases 8MB
                    $authority=Assert-VdpAuthority (Read-PpaJson $authorityFile) $recording $Request.ExpectedContentAuthoritySha256
                    if ($authority.effective_vfs_manifest_sha256 -cne $Request.ExpectedEffectiveVfsManifestSha256) { throw 'PPA_ROOT_MANIFEST_MISMATCH' }
                    $receipt['content_authority']=$authorityFile.reference
                }
            }
            if ($cases.Count -ne 4 -or @($cases | Where-Object { $_.exit_code -ne 0 -or $_.error -or $_.forced_exit -or -not $_.pipe_drain_complete -or $_.cleanup_errors.Count }).Count) { throw 'PPA_CLI_CASES_INCOMPLETE' }
            $receipt.cli_playback_qualified=$true; $receipt.status='pass'
        }
    } catch { $receipt.error=$_.Exception.Message }
    finally {
        if ($null -ne $archive) { $archive.Dispose() }
        if ($null -ne $zip) {
            try {
                $after=Get-PpaStreamHash $zip.stream
                $receipt.package_after=[ordered]@{ sha256=$after; bytes=$zip.stream.Length; same_bytes=($after -ceq $Request.ExpectedPackageSha256) }
                if (-not $receipt.package_after.same_bytes) { throw 'PPA_POST_PACKAGE_CHANGED' }
            } catch { $receipt.status='fail'; $receipt.cli_playback_qualified=$false; $receipt.error=$_.Exception.Message }
        }
        $receipt.cases=@($cases.ToArray()); $receipt.environment_samples=@($samples.ToArray())
        $receipt['finished_utc']=[DateTime]::UtcNow.ToString('o')
        try {
            if ($root) {
                $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 30))
                $null=Publish-WindowsDurableNewFileBytes -LiteralPath (Join-Path $root 'PACKAGE_ACCEPTANCE.json') -Bytes $bytes
            }
        } finally { foreach ($lease in $leases) { $lease.Dispose() } }
    }
    return $receipt
}
