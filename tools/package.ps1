# Builds the release exe and produces dist\island-<version>-portable.zip and (if Inno Setup is found) dist\island-<version>-setup.exe
# Usage: powershell -ExecutionPolicy Bypass -File tools\package.ps1 [-NoBuild] [-NoInstaller]
# (ASCII only on purpose: Windows PowerShell 5.1 reads BOM-less files as ANSI.)
param([switch]$NoBuild, [switch]$NoInstaller)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (-not $NoBuild) {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    }
    $exe = Join-Path $root "target\release\island.exe"
    if (-not (Test-Path $exe)) { throw "island.exe not found - build first" }

    $version = "0.1.0"
    $m = Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($m) { $version = $m.Matches[0].Groups[1].Value }

    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force $dist | Out-Null

    # Portable zip
    $stage = Join-Path $dist "island-$version"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item $exe $stage
    Copy-Item (Join-Path $root "README.md") $stage
    Copy-Item (Join-Path $root "LICENSE") $stage

    $zip = Join-Path $dist "island-$version-portable.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
    Remove-Item -Recurse -Force $stage
    Write-Host ("Done: {0} ({1} KB)" -f $zip, [math]::Round((Get-Item $zip).Length / 1KB))

    # Installer (Inno Setup 6)
    if (-not $NoInstaller) {
        $iscc = @(
            (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
        ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
        if (-not $iscc) {
            Write-Warning "Inno Setup not found - installer skipped. Install: winget install JRSoftware.InnoSetup"
        } else {
            & $iscc "/DAppVersion=$version" (Join-Path $root "installer\island.iss")
            if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
            $setup = Join-Path $dist "island-$version-setup.exe"
            Write-Host ("Done: {0} ({1} KB)" -f $setup, [math]::Round((Get-Item $setup).Length / 1KB))
        }
    }

    # SHA256 of every artifact
    $sums = Get-ChildItem $dist -File | Where-Object { $_.Extension -in ".zip", ".exe" } |
        ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name }
    $sums | Set-Content -Encoding ascii (Join-Path $dist "SHA256SUMS.txt")
    $sums | ForEach-Object { Write-Host $_ }
}
finally {
    Pop-Location
}
