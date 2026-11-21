$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if (-not (Test-Path 'gen/android/keystore.properties')) {
    throw 'gen/android/keystore.properties is missing; copy it from the backed-up signing key folder.'
}
$jdk21 = 'C:\Program Files\BellSoft\LibericaJDK-21'
if (Test-Path $jdk21) { $env:JAVA_HOME = $jdk21 }

cargo tauri android build --apk --target aarch64 --ci
if ($LASTEXITCODE -ne 0) { throw 'Android build failed.' }

$apk = 'gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk'
$out = Join-Path $PSScriptRoot 'target/CebeIndiren.apk'
Copy-Item $apk $out -Force

$buildTools = Get-ChildItem "$env:LOCALAPPDATA\Android\Sdk\build-tools" | Sort-Object Name | Select-Object -Last 1
& (Join-Path $buildTools.FullName 'apksigner.bat') verify --print-certs $out
if ($LASTEXITCODE -ne 0) { throw 'The APK is not signed.' }

$hash = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
Write-Host "`n$out`nSHA-256: $hash"
