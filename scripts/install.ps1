#Requires -Version 5.1
<#
.SYNOPSIS
  Installs the latest release of Scrooge on Windows.
.DESCRIPTION
  Downloads the latest NSIS installer (or portable archive with -Portable) from
  GitHub Releases, verifies its SHA-256 checksum against SHA256SUMS.txt when
  present, and installs Scrooge for the current user.
.EXAMPLE
  irm https://raw.githubusercontent.com/DraconDeSo/scrooge/main/scripts/install.ps1 | iex
#>
param(
  [string]$Repo = "DraconDeSo/scrooge",
  [string]$Version = "latest",
  [switch]$Portable,
  [string]$InstallDir = "$env:LOCALAPPDATA\Programs\Scrooge"
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$apiUrl = if ($Version -eq "latest") {
  "https://api.github.com/repos/$Repo/releases/latest"
} else {
  $tag = if ($Version.StartsWith("v")) { $Version } else { "v$Version" }
  "https://api.github.com/repos/$Repo/releases/tags/$tag"
}

Write-Host "==> Fetching Scrooge release metadata from $Repo..." -ForegroundColor Cyan
$release = Invoke-RestMethod -Uri $apiUrl -Headers @{
  "Accept"     = "application/vnd.github+json"
  "User-Agent" = "scrooge-installer"
}

$assetPattern = if ($Portable) { "*_x64-portable.zip" } else { "*_x64-setup.exe" }
$asset = $release.assets | Where-Object { $_.name -like $assetPattern } | Select-Object -First 1
if (-not $asset) {
  throw "Could not find a matching Windows asset ($assetPattern) in release $($release.tag_name)."
}

$sumsAsset = $release.assets | Where-Object { $_.name -eq "SHA256SUMS.txt" } | Select-Object -First 1
$tempDir = Join-Path ([IO.Path]::GetTempPath()) ("scrooge-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

try {
  $targetFile = Join-Path $tempDir $asset.name
  Write-Host "==> Downloading $($asset.name) ($($release.tag_name))..." -ForegroundColor Cyan
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $targetFile -UseBasicParsing

  if ($sumsAsset) {
    $sumsFile = Join-Path $tempDir "SHA256SUMS.txt"
    Invoke-WebRequest -Uri $sumsAsset.browser_download_url -OutFile $sumsFile -UseBasicParsing
    $expectedLine = Get-Content -LiteralPath $sumsFile | Where-Object { $_ -match ([regex]::Escape($asset.name) + '$') } | Select-Object -First 1
    if ($expectedLine) {
      $expectedHash = ($expectedLine -split '\s+')[0].Trim().ToLowerInvariant()
      $actualHash = (Get-FileHash -LiteralPath $targetFile -Algorithm SHA256).Hash.ToLowerInvariant()
      if ($expectedHash -ne $actualHash) {
        throw "SHA-256 mismatch for $($asset.name): expected $expectedHash, got $actualHash."
      }
      Write-Host "==> Verified SHA-256 checksum." -ForegroundColor Green
    }
  }

  if ($Portable) {
    Write-Host "==> Extracting portable archive to $InstallDir..." -ForegroundColor Cyan
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Expand-Archive -LiteralPath $targetFile -DestinationPath $InstallDir -Force
    Write-Host "==> Installed portable binaries to $InstallDir" -ForegroundColor Green
  } else {
    Write-Host "==> Running installer..." -ForegroundColor Cyan
    $proc = Start-Process -FilePath $targetFile -ArgumentList "/P" -Wait -PassThru
    if ($proc.ExitCode -ne 0) {
      throw "Installer exited with code $($proc.ExitCode)."
    }
    Write-Host "==> Scrooge $($release.tag_name) installed successfully." -ForegroundColor Green
  }
} finally {
  if (Test-Path -LiteralPath $tempDir) {
    Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
  }
}
