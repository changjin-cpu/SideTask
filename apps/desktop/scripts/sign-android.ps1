param(
    [Parameter(Mandatory=$true)][string]$SdkRoot,
    [Parameter(Mandatory=$true)][string]$UnsignedApk,
    [Parameter(Mandatory=$true)][string]$SigningDirectory,
    [Parameter(Mandatory=$true)][string]$OutputApk,
    [string]$BuildToolsVersion = '35.0.0'
)
$ErrorActionPreference = 'Stop'
$toolsDirectory = Join-Path (Resolve-Path -LiteralPath $SdkRoot).Path "build-tools/$BuildToolsVersion"
$keyPath = Join-Path (Resolve-Path -LiteralPath $SigningDirectory).Path 'sidetask-release.jks'
$passwordPath = Join-Path $SigningDirectory 'password.dpapi.xml'
if (-not (Test-Path -LiteralPath $keyPath)) { throw 'Release key is missing; do not replace the identity of an existing release' }
$securePassword = Import-Clixml -LiteralPath $passwordPath
if ($securePassword -isnot [Security.SecureString]) { throw 'Expected a current-user DPAPI encrypted password' }
$plainPassword = [Net.NetworkCredential]::new('', $securePassword).Password
$outputPath = [IO.Path]::GetFullPath($OutputApk)
$alignedPath = "$outputPath.aligned"
New-Item -ItemType Directory -Path (Split-Path -Parent $outputPath) -Force | Out-Null
try {
    & (Join-Path $toolsDirectory 'zipalign.exe') -f -P 16 4 (Resolve-Path -LiteralPath $UnsignedApk).Path $alignedPath
    if ($LASTEXITCODE -ne 0) { throw 'APK alignment failed' }
    $env:SIDETASK_RELEASE_STORE_PASSWORD = $plainPassword
    & (Join-Path $toolsDirectory 'apksigner.bat') sign --ks $keyPath --ks-key-alias sidetask-release --ks-pass env:SIDETASK_RELEASE_STORE_PASSWORD --key-pass env:SIDETASK_RELEASE_STORE_PASSWORD --out $outputPath $alignedPath
    if ($LASTEXITCODE -ne 0) { throw 'APK signing failed' }
    & (Join-Path $toolsDirectory 'apksigner.bat') verify --verbose --print-certs $outputPath
    if ($LASTEXITCODE -ne 0) { throw 'APK signature verification failed' }
    & (Join-Path $toolsDirectory 'zipalign.exe') -c -P 16 4 $outputPath
    if ($LASTEXITCODE -ne 0) { throw 'Signed APK alignment failed' }
    Get-FileHash -LiteralPath $outputPath -Algorithm SHA256
} finally {
    Remove-Item Env:SIDETASK_RELEASE_STORE_PASSWORD -ErrorAction SilentlyContinue
    $plainPassword = $null
    if (Test-Path -LiteralPath $alignedPath) { Remove-Item -LiteralPath $alignedPath }
}
