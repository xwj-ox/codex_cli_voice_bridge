Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseName = "CodexCliVoiceBridgeRust_Windows"
$releaseRoot = Join-Path $projectRoot "packaging\release"
$outputDir = Join-Path $releaseRoot $releaseName
$zipPath = Join-Path $releaseRoot "$releaseName.zip"

Write-Host "[BUILD] project root: $projectRoot"
Write-Host "[BUILD] release dir:  $outputDir"

Push-Location $projectRoot
try {
    cargo build --release --bin voice_bridge --bin doubao_asr_demo --bin mai_demo --bin configure_credentials --bin voice_bridge_doctor
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

if (Test-Path $outputDir) {
    Remove-Item -Path $outputDir -Recurse -Force
}
New-Item -Path $outputDir -ItemType Directory -Force | Out-Null

$releaseBinDir = Join-Path $projectRoot "target\\release"
$templateFiles = @(
    @{ Source = (Join-Path $releaseBinDir "voice_bridge.exe"); Target = "voice_bridge.exe" },
    @{ Source = (Join-Path $releaseBinDir "doubao_asr_demo.exe"); Target = "doubao_asr_demo.exe" },
    @{ Source = (Join-Path $releaseBinDir "mai_demo.exe"); Target = "mai_demo.exe" },
    @{ Source = (Join-Path $releaseBinDir "configure_credentials.exe"); Target = "configure_credentials.exe" },
    @{ Source = (Join-Path $releaseBinDir "voice_bridge_doctor.exe"); Target = "voice_bridge_doctor.exe" },
    @{ Source = (Join-Path $projectRoot "doubao_credentials.example.json"); Target = "doubao_credentials.example.json" },
    @{ Source = (Join-Path $projectRoot "mai_credentials.example.json"); Target = "mai_credentials.example.json" },
    @{ Source = (Join-Path $projectRoot "packaging\\README_windows_release.md"); Target = "README_windows_release.md" }
)

foreach ($file in $templateFiles) {
    Copy-Item -Path $file.Source -Destination (Join-Path $outputDir $file.Target) -Force
}

$blankCredentials = @'
{
  "app_id": "",
  "access_token": ""
}
'@
Set-Content -Path (Join-Path $outputDir "doubao_credentials.json") -Value $blankCredentials -Encoding utf8

$blankMaiCredentials = @'
{
  "endpoint": "",
  "key": ""
}
'@
Set-Content -Path (Join-Path $outputDir "mai_credentials.json") -Value $blankMaiCredentials -Encoding utf8

$launchers = @{
    "voice_bridge.cmd" = "@echo off`r`ncd /d ""%~dp0""`r`nvoice_bridge.exe %*`r`n"
    "start_voice_bridge.cmd" = "@echo off`r`ncd /d ""%~dp0""`r`nvoice_bridge.exe %*`r`n"
    "doubao_asr_demo.cmd" = "@echo off`r`ncd /d ""%~dp0""`r`ndoubao_asr_demo.exe %*`r`n"
    "mai_demo.cmd" = "@echo off`r`ncd /d ""%~dp0""`r`nmai_demo.exe %*`r`n"
    "configure_credentials.cmd" = "@echo off`r`ncd /d ""%~dp0""`r`nconfigure_credentials.exe %*`r`n"
    "voice_bridge_doctor.cmd" = "@echo off`r`ncd /d ""%~dp0""`r`nvoice_bridge_doctor.exe %*`r`n"
}

foreach ($name in $launchers.Keys) {
    Set-Content -Path (Join-Path $outputDir $name) -Value $launchers[$name] -Encoding ascii
}

if (Test-Path $zipPath) {
    Remove-Item -Path $zipPath -Force
}
Compress-Archive -Path $outputDir -DestinationPath $zipPath -CompressionLevel Optimal

Write-Host "[DONE] output dir: $outputDir"
Write-Host "[DONE] zip:        $zipPath"
