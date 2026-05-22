# =============================================================
# YAWREC · build.ps1
# Build de production complet pour Windows.
#
# Étapes :
#   1. Pré-flight : vérifie rust, node, FFMPEG_DIR, WebView2
#   2. npm install (si node_modules absent)
#   3. cargo tauri build → .exe + .msi + .exe NSIS
#   4. Affichage des fichiers produits avec tailles
#
# Usage :
#   .\scripts\build.ps1            # build release par défaut
#   .\scripts\build.ps1 -BuildDebug # build debug (rapide, gros binaire)
#   .\scripts\build.ps1 -SkipNpm   # ne pas re-run npm install
# =============================================================

#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$BuildDebug,
    [switch]$SkipNpm
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root

try {
    # =========================================================
    # Pré-flight
    # =========================================================
    Write-Host "═══ YawREC · pré-flight ═══" -ForegroundColor Cyan

    function Check-Tool($name, $hint) {
        if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
            Write-Host "✗ $name introuvable" -ForegroundColor Red
            Write-Host "  → $hint" -ForegroundColor Yellow
            exit 1
        }
        Write-Host "✓ $name" -ForegroundColor Green
    }

    Check-Tool "rustc" "winget install Rustlang.Rustup, puis rustup default stable-msvc"
    Check-Tool "cargo" "fourni avec rustc"
    Check-Tool "node"  "winget install OpenJS.NodeJS.LTS"
    Check-Tool "npm"   "fourni avec node"

    if (-not (cargo tauri --version 2>$null)) {
        Write-Host "✗ tauri CLI non installé" -ForegroundColor Red
        Write-Host "  → cargo install tauri-cli --version `"^2.0`"" -ForegroundColor Yellow
        exit 1
    }
    Write-Host "✓ tauri CLI" -ForegroundColor Green

    if (-not $env:FFMPEG_DIR) {
        Write-Host "✗ FFMPEG_DIR non défini" -ForegroundColor Red
        Write-Host "  → Télécharger ffmpeg-release-full-shared.7z depuis gyan.dev" -ForegroundColor Yellow
        Write-Host "  → Extraire vers C:\ffmpeg, puis :" -ForegroundColor Yellow
        Write-Host "       setx FFMPEG_DIR `"C:\ffmpeg`"" -ForegroundColor Yellow
        Write-Host "       setx PATH `"`$env:PATH;C:\ffmpeg\bin`"" -ForegroundColor Yellow
        Write-Host "  → Fermer/rouvrir le terminal" -ForegroundColor Yellow
        exit 1
    }
    Write-Host "✓ FFMPEG_DIR = $env:FFMPEG_DIR" -ForegroundColor Green

    if (-not (Test-Path (Join-Path $env:FFMPEG_DIR "include\libavcodec\avcodec.h"))) {
        Write-Host "✗ Headers FFmpeg manquants dans $env:FFMPEG_DIR\include\" -ForegroundColor Red
        Write-Host "  Le build télécharge `"release-full-shared`" doit contenir include/ + lib/" -ForegroundColor Yellow
        exit 1
    }
    Write-Host "✓ Headers FFmpeg présents" -ForegroundColor Green

    # Icônes
    $icon_ico = Join-Path $root "src-tauri\icons\icon.ico"
    if (-not (Test-Path $icon_ico)) {
        Write-Host "✗ src-tauri/icons/icon.ico absent" -ForegroundColor Red
        Write-Host "  → Lancer d'abord : .\scripts\make-icons.ps1" -ForegroundColor Yellow
        exit 1
    }
    Write-Host "✓ Icônes" -ForegroundColor Green

    # =========================================================
    # DLLs FFmpeg — copiées vers src-tauri/resources/ffmpeg-dlls/
    # pour que build.rs (et tauri.conf.json "resources") les bundlent
    # dans le MSI/NSIS. build.rs fait la même chose automatiquement,
    # mais on s'assure ici que le dossier est à jour avant le build.
    # =========================================================
    Write-Host ""
    Write-Host "═══ DLLs FFmpeg ═══" -ForegroundColor Cyan
    $dllDir = Join-Path $root "src-tauri\resources\ffmpeg-dlls"
    New-Item -ItemType Directory -Force $dllDir | Out-Null
    $dlls = Get-ChildItem (Join-Path $env:FFMPEG_DIR "bin") -Filter "*.dll"
    foreach ($dll in $dlls) {
        Copy-Item $dll.FullName (Join-Path $dllDir $dll.Name) -Force
    }
    # Also copy next to the release exe for direct testing without installer
    $releaseDir = Join-Path $root "src-tauri\target\release"
    if (Test-Path $releaseDir) {
        foreach ($dll in $dlls) {
            Copy-Item $dll.FullName (Join-Path $releaseDir $dll.Name) -Force
        }
    }
    Write-Host "✓ $($dlls.Count) DLLs copiées (resources + release)" -ForegroundColor Green

    # =========================================================
    # npm install
    # =========================================================
    if (-not $SkipNpm) {
        $node_modules = Join-Path $root "node_modules"
        if (-not (Test-Path $node_modules)) {
            Write-Host ""
            Write-Host "═══ npm install ═══" -ForegroundColor Cyan
            & npm install
            if ($LASTEXITCODE -ne 0) { throw "npm install a échoué" }
        } else {
            Write-Host "✓ node_modules présent (utiliser -SkipNpm pour forcer)" -ForegroundColor Green
        }
    }

    # =========================================================
    # Build Tauri
    # =========================================================
    Write-Host ""
    # PS 5.1 traite la stderr de cargo comme des erreurs avec ErrorActionPreference=Stop.
    # On relâche temporairement pour que cargo puisse écrire en stderr sans planter le script.
    $ErrorActionPreference = "Continue"
    if ($BuildDebug) {
        Write-Host "═══ cargo tauri build (DEBUG) ═══" -ForegroundColor Cyan
        & cargo tauri build --debug
    } else {
        Write-Host "═══ cargo tauri build (RELEASE) ═══" -ForegroundColor Cyan
        & cargo tauri build
    }
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    if ($buildExit -ne 0) { throw "cargo tauri build a échoué (code $buildExit)" }

    # =========================================================
    # Résultats
    # =========================================================
    $target_dir = if ($Debug) {
        Join-Path $root "src-tauri\target\debug"
    } else {
        Join-Path $root "src-tauri\target\release"
    }
    $bundle_dir = Join-Path $target_dir "bundle"

    Write-Host ""
    Write-Host "═══ Artefacts produits ═══" -ForegroundColor Cyan

    $exe = Join-Path $target_dir "yawrec.exe"
    if (Test-Path $exe) {
        $size_mb = [math]::Round((Get-Item $exe).Length / 1MB, 1)
        Write-Host "  Binaire   : $exe ($size_mb MB)" -ForegroundColor Green
    }

    if (Test-Path $bundle_dir) {
        Get-ChildItem $bundle_dir -Recurse -File -Include "*.msi","*.exe" |
            Where-Object { $_.FullName -ne $exe } |
            ForEach-Object {
                $size_mb = [math]::Round($_.Length / 1MB, 1)
                $kind = if ($_.Extension -eq ".msi") { "Installeur MSI" } else { "Installeur NSIS" }
                Write-Host "  $kind : $($_.FullName) ($size_mb MB)" -ForegroundColor Green
            }
    }

    Write-Host ""
    Write-Host "✓ Build terminé" -ForegroundColor Green
    Write-Host "  Étape suivante (optionnelle) : .\scripts\sign.ps1" -ForegroundColor DarkGray
}
finally {
    Pop-Location
}
