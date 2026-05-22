# =============================================================
# YAWREC · make-icons.ps1
# Génère toutes les icônes requises par Tauri depuis docs/icon-source.svg
#
# Étape 1 : SVG → PNG 1024×1024 via rsvg-convert ou Inkscape
# Étape 2 : `cargo tauri icon` génère toutes les tailles dans src-tauri/icons/
#
# Pré-requis (au choix) :
#   - rsvg-convert : `winget install gnome.librsvg` (recommandé, le + rapide)
#   - Inkscape     : `winget install Inkscape.Inkscape`
# =============================================================

#Requires -Version 5.1
$ErrorActionPreference = "Stop"

$root       = Split-Path $PSScriptRoot -Parent
$source_svg = Join-Path $root "docs\icon-source.svg"
$temp_png   = Join-Path $env:TEMP "yawrec-icon-1024.png"

if (-not (Test-Path $source_svg)) {
    throw "Source introuvable : $source_svg"
}

Write-Host "→ Conversion SVG → PNG 1024×1024..." -ForegroundColor Cyan

# Détection automatique du convertisseur disponible
$converter = $null
if (Get-Command rsvg-convert -ErrorAction SilentlyContinue) {
    $converter = "rsvg-convert"
} elseif (Get-Command inkscape -ErrorAction SilentlyContinue) {
    $converter = "inkscape"
} else {
    Write-Host ""
    Write-Host "Aucun convertisseur SVG trouvé." -ForegroundColor Yellow
    Write-Host "Installer l'un des deux :"
    Write-Host "  winget install gnome.librsvg     # rsvg-convert (recommandé)"
    Write-Host "  winget install Inkscape.Inkscape # Inkscape"
    Write-Host ""
    Write-Host "Ou : convertir manuellement docs/icon-source.svg en"
    Write-Host "PNG 1024×1024, le placer en $temp_png, puis relancer."
    exit 1
}

switch ($converter) {
    "rsvg-convert" {
        & rsvg-convert -w 1024 -h 1024 -f png $source_svg -o $temp_png
    }
    "inkscape" {
        & inkscape $source_svg `
            --export-type=png `
            --export-filename=$temp_png `
            --export-width=1024 `
            --export-height=1024
    }
}

if (-not (Test-Path $temp_png)) {
    throw "La conversion a échoué — $temp_png n'a pas été créé"
}

Write-Host "✓ PNG temporaire : $temp_png" -ForegroundColor Green

# Étape 2 : cargo tauri icon génère tout ce qu'il faut
Write-Host "→ Génération des tailles via cargo tauri icon..." -ForegroundColor Cyan
Push-Location $root
try {
    & cargo tauri icon $temp_png
} finally {
    Pop-Location
}

# Cleanup
Remove-Item $temp_png -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "✓ Icônes générées dans src-tauri/icons/" -ForegroundColor Green
Get-ChildItem (Join-Path $root "src-tauri\icons") -Filter "*.png","*.ico" | ForEach-Object {
    Write-Host "  - $($_.Name)" -ForegroundColor DarkGray
}
