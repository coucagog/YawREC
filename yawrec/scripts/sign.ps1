# =============================================================
# YAWREC · sign.ps1
# Signature de code pour le .exe et les installateurs.
#
# Trois modes :
#
#   1. -SelfSigned        : génère un cert auto-signé jetable (dev/test).
#                           Le binaire sera signé mais SmartScreen affichera
#                           quand même un warning pour les utilisateurs.
#
#   2. -Thumbprint "..."  : utilise un cert déjà installé dans le store
#                           CurrentUser\My (typique pour un cert CodeSigning
#                           racheté chez DigiCert/Sectigo/etc.).
#
#   3. -PfxPath "..." -PfxPassword (sécurisé) : utilise un fichier .pfx
#                           chiffré. Préféré en CI/CD.
#
# Le timestamp server est obligatoire — sans lui, la signature expire
# avec le certificat.
# =============================================================

#Requires -Version 5.1
[CmdletBinding(DefaultParameterSetName="SelfSigned")]
param(
    [Parameter(ParameterSetName="SelfSigned")]
    [switch]$SelfSigned,

    [Parameter(ParameterSetName="Thumbprint", Mandatory)]
    [string]$Thumbprint,

    [Parameter(ParameterSetName="Pfx", Mandatory)]
    [string]$PfxPath,

    [Parameter(ParameterSetName="Pfx", Mandatory)]
    [SecureString]$PfxPassword,

    [string]$TimestampUrl = "http://timestamp.digicert.com",

    [switch]$Debug
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent

# =============================================================
# Localiser signtool.exe
# =============================================================
$signtool = $null
$candidates = @(
    "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe",
    "${env:ProgramFiles}\Windows Kits\10\bin\*\x64\signtool.exe"
)
foreach ($pattern in $candidates) {
    $found = Get-ChildItem $pattern -ErrorAction SilentlyContinue |
             Sort-Object FullName -Descending | Select-Object -First 1
    if ($found) { $signtool = $found.FullName; break }
}
if (-not $signtool) {
    Write-Host "✗ signtool.exe introuvable" -ForegroundColor Red
    Write-Host "  → Installer le Windows SDK : winget install Microsoft.WindowsSDK.10.0.22621" -ForegroundColor Yellow
    exit 1
}
Write-Host "✓ signtool : $signtool" -ForegroundColor Green

# =============================================================
# Préparer les paramètres de signature
# =============================================================
$sign_args = @("sign", "/fd", "SHA256", "/tr", $TimestampUrl, "/td", "SHA256")

switch ($PSCmdlet.ParameterSetName) {
    "SelfSigned" {
        Write-Host "→ Mode self-signed (développement / test uniquement)" -ForegroundColor Yellow

        # Vérifie si un cert YawREC existe déjà
        $existing = Get-ChildItem Cert:\CurrentUser\My |
            Where-Object { $_.Subject -eq "CN=YawREC Dev" -and $_.HasPrivateKey }

        $cert = if ($existing) {
            Write-Host "✓ Réutilisation du cert dev existant (thumbprint $($existing[0].Thumbprint))" -ForegroundColor Green
            $existing[0]
        } else {
            Write-Host "→ Création d'un nouveau cert self-signed..." -ForegroundColor Cyan
            New-SelfSignedCertificate `
                -Type CodeSigningCert `
                -Subject "CN=YawREC Dev" `
                -KeyAlgorithm RSA -KeyLength 2048 `
                -CertStoreLocation "Cert:\CurrentUser\My" `
                -NotAfter (Get-Date).AddYears(3) `
                -KeyUsage DigitalSignature `
                -KeyExportPolicy Exportable
        }

        $sign_args += @("/sha1", $cert.Thumbprint)
    }
    "Thumbprint" {
        Write-Host "→ Mode thumbprint : $Thumbprint" -ForegroundColor Cyan
        $sign_args += @("/sha1", $Thumbprint)
    }
    "Pfx" {
        if (-not (Test-Path $PfxPath)) {
            throw "Fichier PFX introuvable : $PfxPath"
        }
        Write-Host "→ Mode PFX : $PfxPath" -ForegroundColor Cyan
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($PfxPassword)
        $plain_pw = [Runtime.InteropServices.Marshal]::PtrToStringAuto($bstr)
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
        $sign_args += @("/f", $PfxPath, "/p", $plain_pw)
    }
}

if ($Debug) { $sign_args += "/v" }

# =============================================================
# Collecter les fichiers à signer
# =============================================================
$target_dir = Join-Path $root "src-tauri\target\release"
$bundle_dir = Join-Path $target_dir "bundle"

$files_to_sign = @()
$exe = Join-Path $target_dir "yawrec.exe"
if (Test-Path $exe) { $files_to_sign += $exe }

if (Test-Path $bundle_dir) {
    $files_to_sign += Get-ChildItem $bundle_dir -Recurse -File -Include "*.msi","*.exe" |
        ForEach-Object { $_.FullName }
}

if ($files_to_sign.Count -eq 0) {
    Write-Host "✗ Aucun fichier à signer trouvé. Lancer .\scripts\build.ps1 d'abord." -ForegroundColor Red
    exit 1
}

# =============================================================
# Signer chaque fichier
# =============================================================
Write-Host ""
Write-Host "═══ Signature de $($files_to_sign.Count) fichier(s) ═══" -ForegroundColor Cyan

$ok_count = 0
foreach ($file in $files_to_sign) {
    Write-Host "→ $([IO.Path]::GetFileName($file))" -ForegroundColor White
    & $signtool @sign_args $file
    if ($LASTEXITCODE -eq 0) {
        $ok_count++
        Write-Host "  ✓ signé" -ForegroundColor Green
    } else {
        Write-Host "  ✗ échec (code $LASTEXITCODE)" -ForegroundColor Red
    }
}

Write-Host ""
Write-Host "✓ $ok_count/$($files_to_sign.Count) fichier(s) signé(s)" `
    -ForegroundColor $(if ($ok_count -eq $files_to_sign.Count) { "Green" } else { "Yellow" })

if ($PSCmdlet.ParameterSetName -eq "SelfSigned") {
    Write-Host ""
    Write-Host "⚠ Signature dev : Windows SmartScreen affichera quand même" -ForegroundColor Yellow
    Write-Host "  un warning aux utilisateurs. Pour une distribution publique," -ForegroundColor Yellow
    Write-Host "  acheter un cert CodeSigning EV chez DigiCert/Sectigo/etc." -ForegroundColor Yellow
}
