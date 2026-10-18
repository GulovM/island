# Builds the release exe and produces dist\island-<version>-portable.zip and (if Inno Setup is found) dist\island-<version>-setup.exe
# Usage: powershell -ExecutionPolicy Bypass -File tools\package.ps1 [-NoBuild] [-NoInstaller]
# Signing (optional, skipped when nothing is configured): set ISLAND_SIGN_PFX (+ ISLAND_SIGN_PASSWORD)
# or ISLAND_SIGN_THUMBPRINT (cert in the store); ISLAND_SIGN_TIMESTAMP overrides the timestamp server.
# (ASCII only on purpose: Windows PowerShell 5.1 reads BOM-less files as ANSI.)
param([switch]$NoBuild, [switch]$NoInstaller)

$ErrorActionPreference = "Stop"

function Find-SignTool {
    $c = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($c) { return $c.Source }
    $kits = "${env:ProgramFiles(x86)}\Windows Kitsin"
    if (Test-Path $kits) {
        return Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -like "*d\*" } | Sort-Object FullName -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
}

# Authenticode-signs a file when a certificate is configured; otherwise does nothing.
function Sign-File($path) {
    if (-not $env:ISLAND_SIGN_PFX -and -not $env:ISLAND_SIGN_THUMBPRINT) { return }
    $tool = Find-SignTool
    if (-not $tool) { throw "signtool.exe not found (install Windows SDK)" }
    $ts = if ($env:ISLAND_SIGN_TIMESTAMP) { $env:ISLAND_SIGN_TIMESTAMP } else { "http://timestamp.digicert.com" }
    $a = @("sign", "/fd", "SHA256", "/tr", $ts, "/td", "SHA256")
    if ($env:ISLAND_SIGN_PFX) {
        $a += @("/f", $env:ISLAND_SIGN_PFX)
        if ($env:ISLAND_SIGN_PASSWORD) { $a += @("/p", $env:ISLAND_SIGN_PASSWORD) }
    } else {
        $a += @("/sha1", $env:ISLAND_SIGN_THUMBPRINT)
    }
    & $tool @a $path
    if ($LASTEXITCODE -ne 0) { throw "signtool failed for $path" }
    Write-Host "Signed: $path"
}

$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (-not $NoBuild) {
        cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    }
    $exe = Join-Path $root "target\release\island.exe"
    if (-not (Test-Path $exe)) { throw "island.exe not found - build first" }

    Sign-File $exe

    $version = "0.1.0"
    $m = Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($m) { $version = $m.Matches[0].Groups[1].Value }

    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force $dist | Out-Null
    # Old artifacts of other versions would end up in SHA256SUMS - remove them.
    Get-ChildItem $dist -File | Where-Object { $_.Extension -in ".zip", ".exe" -and $_.Name -notlike "island-$version-*" } | Remove-Item -Force

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
            Sign-File $setup
            Write-Host ("Done: {0} ({1} KB)" -f $setup, [math]::Round((Get-Item $setup).Length / 1KB))
        }
    }

    # SHA256 of every artifact
    $sums = Get-ChildItem $dist -File | Where-Object { $_.Extension -in ".zip", ".exe" } |
        ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name }
    # LF line endings (sha256sum -c on Linux chokes on CR).
    [IO.File]::WriteAllText((Join-Path $dist "SHA256SUMS.txt"), ($sums -join "`n") + "`n", (New-Object Text.ASCIIEncoding))
    $sums | ForEach-Object { Write-Host $_ }
}
finally {
    Pop-Location
}
