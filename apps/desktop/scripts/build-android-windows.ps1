param(
    [Parameter(Mandatory=$true)][string]$SdkRoot,
    [Parameter(Mandatory=$true)][string]$JdkRoot,
    [Parameter(Mandatory=$true)][string]$NativeTargetDir,
    [string]$NdkVersion = '28.0.13004108',
    [ValidateSet('debug','release')][string]$Profile = 'release',
    [string[]]$Targets = @('aarch64','x86_64'),
    [switch]$Smoke
)
$ErrorActionPreference = 'Stop'
if ($Smoke -and $Profile -ne 'debug') { throw 'Smoke instrumentation cannot be packaged for release' }
$desktopRoot = Split-Path -Parent $PSScriptRoot
$projectRoot = Join-Path $desktopRoot 'src-tauri/gen/android'
if ($NativeTargetDir -match '[^\x00-\x7F]') { throw 'NativeTargetDir must contain ASCII characters only for Windows NDK linking' }
$env:ANDROID_HOME = (Resolve-Path -LiteralPath $SdkRoot).Path
$env:ANDROID_SDK_ROOT = $env:ANDROID_HOME
$env:NDK_HOME = Join-Path $env:ANDROID_HOME "ndk/$NdkVersion"
$env:JAVA_HOME = (Resolve-Path -LiteralPath $JdkRoot).Path
$env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath($NativeTargetDir)
$env:Path = "$env:JAVA_HOME/bin;$env:USERPROFILE/.cargo/bin;$env:Path"
$logDir = Join-Path $desktopRoot '../../artifacts/android'
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
$abis = @{ aarch64='arm64-v8a'; x86_64='x86_64' }
Push-Location $desktopRoot
try {
    foreach ($target in $Targets) {
        if (-not $abis.ContainsKey($target)) { throw "Unsupported Android target: $target" }
        $buildLog = Join-Path $logDir "native-$target-$Profile.log"
        $cliArgs = @('tauri','android','build','--target',$target,'--apk')
        if ($Profile -eq 'debug') { $cliArgs += '--debug' }
        if ($Smoke) { $cliArgs += @('--features','mobile-smoke') }
        # Tauri normally links the freshly built .so into the Gradle project.
        # Windows without symlink privilege can instead copy this exact output.
        & npx @cliArgs > $buildLog 2>&1
        $nativeExit = $LASTEXITCODE
        if ($nativeExit -ne 0 -and -not (Select-String -LiteralPath $buildLog -SimpleMatch 'Creation symbolic link is not allowed for this system.' -Quiet)) {
            Get-Content -LiteralPath $buildLog -Tail 25
            throw "Android native compilation failed ($nativeExit)"
        }
        $nativeFile = Join-Path $env:CARGO_TARGET_DIR "$target-linux-android/$Profile/libsidetask_lib.so"
        if (-not (Test-Path -LiteralPath $nativeFile)) { throw 'Compiled native library not found' }
        $jniDir = Join-Path $projectRoot "app/src/main/jniLibs/$($abis[$target])"
        New-Item -ItemType Directory -Path $jniDir -Force | Out-Null
        Copy-Item -LiteralPath $nativeFile -Destination (Join-Path $jniDir 'libsidetask_lib.so') -Force
        if ((Get-FileHash -LiteralPath $nativeFile).Hash -ne (Get-FileHash -LiteralPath (Join-Path $jniDir 'libsidetask_lib.so')).Hash) { throw 'Native library copy verification failed' }
    }
    $version = (Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw | ConvertFrom-Json).version
    $parts = $version.Split('.')
    $versionCode = [int]$parts[0] * 1000000 + [int]$parts[1] * 1000 + [int]$parts[2]
    @("tauri.android.versionCode=$versionCode", "tauri.android.versionName=$version") | Set-Content -LiteralPath (Join-Path $projectRoot 'app/tauri.properties') -Encoding ascii
    $gradleArgs = @('-p',$projectRoot,"assembleUniversal$($Profile.Substring(0,1).ToUpper()+$Profile.Substring(1))",'-x',"rustBuildUniversal$($Profile.Substring(0,1).ToUpper()+$Profile.Substring(1))",'--console=plain',"-PabiList=$(($Targets | ForEach-Object { $abis[$_] }) -join ',')")
    & (Join-Path $projectRoot 'gradlew.bat') @gradleArgs
    if ($LASTEXITCODE -ne 0) { throw "Gradle packaging failed ($LASTEXITCODE)" }
} finally { Pop-Location }
