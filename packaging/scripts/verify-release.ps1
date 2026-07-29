[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][ValidateSet('windows-x86_64', 'macos-universal', 'linux-x86_64', 'linux-aarch64')][string]$Package,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Commit,
    [ValidateSet('unsigned-verification', 'production', 'ad-hoc-verification', 'checksums-and-provenance')][string]$SignatureMode
)

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'invalid version' }
if ($Commit -notmatch '^[0-9a-f]{40}$') { throw 'invalid commit' }
$archivePath = [IO.Path]::GetFullPath($Archive)
if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) { throw 'archive is missing' }
$sidecar = "$archivePath.sha256"
if (-not (Test-Path -LiteralPath $sidecar -PathType Leaf)) { throw 'checksum sidecar is missing' }
$checksumLine = (Get-Content -LiteralPath $sidecar -Raw -Encoding ascii).Trim()
$expectedName = [IO.Path]::GetFileName($archivePath)
if ($checksumLine -notmatch "^(?<hash>[0-9a-f]{64})  $([regex]::Escape($expectedName))$") {
    throw 'checksum sidecar has an invalid shape or filename'
}
$actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $Matches.hash) { throw 'archive checksum mismatch' }

if ($Package -in @('windows-x86_64', 'macos-universal')) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            if ($name -match '(^/|(^|/)\.\.(/|$)|:)') { throw 'archive contains an unsafe ZIP path' }
        }
    }
    finally {
        $zip.Dispose()
    }
}
else {
    $entries = & tar -tzf $archivePath
    if ($LASTEXITCODE -ne 0) { throw 'Linux archive table is unreadable' }
    foreach ($entry in $entries) {
        if ($entry -match '(^/|(^|/)\.\.(/|$))') { throw 'archive contains an unsafe tar path' }
    }
    $verboseEntries = & tar -tvzf $archivePath
    if ($LASTEXITCODE -ne 0) { throw 'Linux archive metadata is unreadable' }
    if ($verboseEntries | Where-Object { $_ -match '^[lh]' }) { throw 'Linux archive contains a link entry' }
}

$temporary = Join-Path ([IO.Path]::GetTempPath()) ("codex-notifier-verify-" + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temporary) | Out-Null
try {
    if ($Package -eq 'windows-x86_64') {
        Expand-Archive -LiteralPath $archivePath -DestinationPath $temporary
        $root = Join-Path $temporary "codex-notifier-v$Version-windows-x86_64"
        $topLevel = Get-ChildItem -LiteralPath $temporary -Force
        if ($topLevel.Count -ne 1 -or $topLevel[0].Name -ne [IO.Path]::GetFileName($root)) { throw 'Windows archive has an unexpected top-level layout' }
        $metadataPath = Join-Path $root 'RELEASE-METADATA.json'
        $executable = Join-Path $root 'codex-notifier.exe'
        $required = @($executable, (Join-Path $root 'LICENSE'), (Join-Path $root 'THIRD-PARTY-LICENSES.html'), (Join-Path $root 'codex-notifier.spdx.json'), $metadataPath)
    }
    elseif ($Package -eq 'macos-universal') {
        & ditto -x -k $archivePath $temporary
        if ($LASTEXITCODE -ne 0) { throw 'macOS archive extraction failed' }
        $root = Join-Path $temporary 'Codex Notifier.app'
        $unexpected = Get-ChildItem -LiteralPath $temporary -Force | Where-Object { $_.Name -notin @('Codex Notifier.app', '__MACOSX') }
        if ($unexpected -or -not (Test-Path -LiteralPath $root -PathType Container)) { throw 'macOS archive has an unexpected top-level layout' }
        $resources = Join-Path $root 'Contents/Resources'
        $metadataPath = Join-Path $resources 'RELEASE-METADATA.json'
        $executable = Join-Path $root 'Contents/MacOS/codex-notifier'
        $required = @($executable, (Join-Path $root 'Contents/Info.plist'), (Join-Path $resources 'codex-notifier.icns'), (Join-Path $resources 'LICENSE'), (Join-Path $resources 'THIRD-PARTY-LICENSES.html'), (Join-Path $resources 'codex-notifier.spdx.json'), $metadataPath)
    }
    else {
        & tar -xzf $archivePath -C $temporary
        if ($LASTEXITCODE -ne 0) { throw 'Linux archive extraction failed' }
        $architecture = if ($Package -eq 'linux-x86_64') { 'x86_64' } else { 'aarch64' }
        $root = Join-Path $temporary "codex-notifier-v$Version-linux-$architecture"
        $topLevel = Get-ChildItem -LiteralPath $temporary -Force
        if ($topLevel.Count -ne 1 -or $topLevel[0].Name -ne [IO.Path]::GetFileName($root)) { throw 'Linux archive has an unexpected top-level layout' }
        $metadataPath = Join-Path $root 'RELEASE-METADATA.json'
        $executable = Join-Path $root 'codex-notifier'
        $required = @($executable, (Join-Path $root 'install.sh'), (Join-Path $root 'uninstall.sh'), (Join-Path $root 'systemd/codex-notifier.service.in'), (Join-Path $root 'examples/ssh-config.example'), (Join-Path $root 'LICENSE'), (Join-Path $root 'THIRD-PARTY-LICENSES.html'), (Join-Path $root 'codex-notifier.spdx.json'), $metadataPath)
    }

    foreach ($path in $required) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "required package file is missing: $path" }
    }
    $metadata = Get-Content -LiteralPath $metadataPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($metadata.schema_version -ne 1 -or $metadata.product -ne 'codex-notifier' -or $metadata.version -ne $Version -or $metadata.commit -ne $Commit -or $metadata.signature -ne $SignatureMode) {
        throw 'release metadata does not match the archive contract'
    }
    $sbom = Get-Content -LiteralPath ($required | Where-Object { $_ -like '*.spdx.json' }) -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($sbom.SPDXID -ne 'SPDXRef-DOCUMENT' -or -not $sbom.packages) { throw 'embedded SPDX SBOM is invalid' }

    if ($Package -eq 'windows-x86_64') {
        $reported = (& $executable --version).Trim()
        if ($LASTEXITCODE -ne 0 -or $reported -ne "codex-notifier $Version") { throw 'packaged Windows version mismatch' }
        $signature = Get-AuthenticodeSignature -LiteralPath $executable
        if ($SignatureMode -eq 'production' -and $signature.Status -ne 'Valid') { throw 'packaged Windows signature is invalid' }
        if ($SignatureMode -eq 'unsigned-verification' -and $signature.Status -ne 'NotSigned') { throw 'verification archive is not explicitly unsigned' }
    }
    elseif ($Package -eq 'macos-universal') {
        & lipo $executable -verify_arch x86_64 arm64
        if ($LASTEXITCODE -ne 0) { throw 'packaged macOS executable is not universal' }
        & codesign --verify --deep --strict --verbose=2 $root
        if ($LASTEXITCODE -ne 0) { throw 'packaged macOS signature is invalid' }
        $plistVersion = (& plutil -extract CFBundleShortVersionString raw (Join-Path $root 'Contents/Info.plist')).Trim()
        if ($plistVersion -ne $Version) { throw 'macOS bundle version mismatch' }
        $reported = (& $executable --version).Trim()
        if ($LASTEXITCODE -ne 0 -or $reported -ne "codex-notifier $Version") { throw 'packaged macOS version mismatch' }
        if ($SignatureMode -eq 'production') {
            & xcrun stapler validate $root
            if ($LASTEXITCODE -ne 0) { throw 'packaged macOS notarization ticket is missing' }
            & spctl --assess --type execute --verbose=2 $root
            if ($LASTEXITCODE -ne 0) { throw 'packaged macOS Gatekeeper assessment failed' }
        }
    }
    elseif ($Package -eq 'linux-x86_64') {
        $reported = (& $executable --version).Trim()
        if ($LASTEXITCODE -ne 0 -or $reported -ne "codex-notifier $Version") { throw 'packaged Linux version mismatch' }
        $description = (& file $executable)
        if ($description -notmatch 'x86-64|x86_64') { throw 'Linux x86-64 architecture mismatch' }
    }
    else {
        $description = (& file $executable)
        if ($description -notmatch 'aarch64|ARM aarch64') { throw 'Linux AArch64 architecture mismatch' }
    }
}
finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}

Write-Output "verified=$expectedName sha256=$actualHash"
