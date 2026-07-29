[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Binary,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Commit,
    [Parameter(Mandatory)][string]$LicenseNotices,
    [Parameter(Mandatory)][string]$Sbom,
    [ValidateSet('unsigned-verification', 'production')][string]$SignatureMode = 'unsigned-verification',
    [string]$ExpectedSignerThumbprint = ''
)

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'invalid version' }
if ($Commit -notmatch '^[0-9a-f]{40}$') { throw 'invalid commit' }
foreach ($path in @($Binary, $LicenseNotices, $Sbom, (Join-Path $PSScriptRoot '..\..\LICENSE'))) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "missing release input: $path" }
}

$signature = Get-AuthenticodeSignature -LiteralPath $Binary
if ($SignatureMode -eq 'production') {
    if ($signature.Status -ne 'Valid') { throw "production binary signature is $($signature.Status)" }
    $actual = $signature.SignerCertificate.Thumbprint
    if (-not $ExpectedSignerThumbprint -or $actual -ne $ExpectedSignerThumbprint) {
        throw 'production signer thumbprint does not match the protected release binding'
    }
}
elseif ($signature.Status -ne 'NotSigned') {
    throw "verification binary unexpectedly has signature status $($signature.Status)"
}

$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
$name = "codex-notifier-v$Version-windows-x86_64"
$stage = Join-Path $output $name
$archive = Join-Path $output "$name.zip"
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
[IO.Directory]::CreateDirectory($stage) | Out-Null

Copy-Item -LiteralPath $Binary -Destination (Join-Path $stage 'codex-notifier.exe')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\..\LICENSE') -Destination (Join-Path $stage 'LICENSE')
Copy-Item -LiteralPath $LicenseNotices -Destination (Join-Path $stage 'THIRD-PARTY-LICENSES.html')
Copy-Item -LiteralPath $Sbom -Destination (Join-Path $stage 'codex-notifier.spdx.json')
$metadata = [ordered]@{
    schema_version = 1
    product = 'codex-notifier'
    version = $Version
    target = 'x86_64-pc-windows-msvc'
    commit = $Commit
    signature = $SignatureMode
}
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText(
    (Join-Path $stage 'RELEASE-METADATA.json'),
    ($metadata | ConvertTo-Json -Compress),
    $utf8
)

$epoch = if ($env:SOURCE_DATE_EPOCH -match '^\d+$') {
    [DateTimeOffset]::FromUnixTimeSeconds([int64]$env:SOURCE_DATE_EPOCH).UtcDateTime
} else {
    [DateTime]::UnixEpoch
}
Get-ChildItem -LiteralPath $stage -Recurse | ForEach-Object { $_.LastWriteTimeUtc = $epoch }
(Get-Item -LiteralPath $stage).LastWriteTimeUtc = $epoch
Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal
Remove-Item -LiteralPath $stage -Recurse -Force

$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText(
    "$archive.sha256",
    "$hash  $([IO.Path]::GetFileName($archive))`n",
    [Text.Encoding]::ASCII
)
Write-Output $archive
