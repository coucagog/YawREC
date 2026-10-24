# ============================================================
# YawREC · test_audio.ps1
# Tests automatisés : enregistrement audio, pause, qualité MP4
#
# Usage :
#   .\scripts\test_audio.ps1                         # exe de target\release
#   .\scripts\test_audio.ps1 -ExePath C:\yawrec\yawrec.exe
# ffprobe est cherché dans le PATH, puis dans $env:FFMPEG_DIR\bin.
#
# Fichier en UTF-8 avec BOM : sans BOM, Windows PowerShell 5.1 le lit en
# ANSI et les caractères accentués cassent le parsing.
# ============================================================

param(
    [string]$ExePath = (Join-Path $PSScriptRoot "..\yawrec\src-tauri\target\release\yawrec.exe"),
    [string]$FfprobePath = "",
    [string]$OutputDir = "$env:USERPROFILE\Videos\YawREC"
)

if (-not $FfprobePath) {
    $cmd = Get-Command ffprobe -ErrorAction SilentlyContinue
    if ($cmd) { $FfprobePath = $cmd.Source }
    elseif ($env:FFMPEG_DIR) { $FfprobePath = Join-Path $env:FFMPEG_DIR "bin\ffprobe.exe" }
}
if (-not $FfprobePath -or -not (Test-Path $FfprobePath)) {
    Write-Host "[TEST] ffprobe introuvable : l'ajouter au PATH ou passer -FfprobePath" -ForegroundColor Red
    exit 1
}

# Seuls les MP4 écrits après ce instant comptent : un ancien fichier du
# dossier de sortie ne doit pas faire passer le test.
$testStart = Get-Date

# Horodatages (s) des paquets d'un flux ("v:0" / "a:0"), triés.
function Get-PacketTimes($File, $Stream) {
    & $FfprobePath -v error -select_streams $Stream -show_entries packet=pts_time -of csv=p=0 $File |
        Where-Object { $_ -match '^[0-9.]+' } |
        ForEach-Object { [double](($_ -split ',')[0]) } |
        Sort-Object
}

# Synchro et régularité d'un MP4, mesurées sur les paquets : en VFR, la
# « duration » des flux rapportée par ffprobe ne reflète pas l'étendue réelle.
function Test-Timing($File, $Label) {
    $vt = @(Get-PacketTimes $File "v:0")
    $at = @(Get-PacketTimes $File "a:0")
    if ($vt.Count -lt 2 -or $at.Count -lt 2) {
        Fail "Horodatages $Label" "vidéo=$($vt.Count) paquets, audio=$($at.Count) paquets"
        return
    }
    # Fin des deux flux à moins de 300 ms l'une de l'autre.
    $endDiff = [math]::Round([math]::Abs($vt[-1] - $at[-1]), 3)
    if ($endDiff -lt 0.3) { Pass "Fins A/V alignées $Label (écart ${endDiff}s)" }
    else { Fail "Fins A/V alignées $Label" "vidéo finit à $($vt[-1])s, audio à $($at[-1])s" }

    # Aucun gel d'image : la dernière frame est répétée à la cadence demandée,
    # un écart > 250 ms entre deux frames signale une boucle de capture bloquée.
    $maxGap = 0.0; $at_s = 0.0
    for ($i = 1; $i -lt $vt.Count; $i++) {
        $gap = $vt[$i] - $vt[$i - 1]
        if ($gap -gt $maxGap) { $maxGap = $gap; $at_s = $vt[$i - 1] }
    }
    $maxGapMs = [math]::Round($maxGap * 1000)
    $fps = [math]::Round(($vt.Count - 1) / ($vt[-1] - $vt[0]), 1)
    if ($maxGap -lt 0.25) { Pass "Pas de gel d'image $Label (max ${maxGapMs} ms, ${fps} fps)" }
    else { Fail "Pas de gel d'image $Label" "gel de ${maxGapMs} ms à ${at_s}s, ${fps} fps moyen (voir 'Vidéo · gel' dans yawrec.log)" }
}

# Renvoie le MP4 le plus récent créé pendant le test (hors $Exclude).
function Get-NewMp4($Exclude) {
    Get-ChildItem -Path $OutputDir -Filter "*.mp4" -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -gt $testStart -and $_.FullName -ne $Exclude } |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}

$errors = @()
$results = @()

function Log($msg) { Write-Host "[TEST] $msg" }
function Pass($test) { $script:results += "✓ $test"; Log "PASS : $test" }
function Fail($test, $reason) { $script:results += "✗ $test — $reason"; $script:errors += $test; Log "FAIL : $test — $reason" }

# ---- 1. Vérifier l'exécutable ----
Log "=== 1. Vérification exécutable ==="
if (Test-Path $ExePath) { Pass "yawrec.exe présent" }
else { Fail "yawrec.exe présent" "Fichier introuvable : $ExePath"; exit 1 }

# ---- 2. Démarrer l'app ----
Log "=== 2. Démarrage ==="
# L'app est mono-instance : une instance déjà ouverte recevrait les
# raccourcis et la nouvelle se fermerait aussitôt.
if (Get-Process -Name "yawrec" -ErrorAction SilentlyContinue) {
    Fail "App démarrée" "YawREC tourne déjà : le fermer (icône de la barre des tâches → Quitter) avant le test"
    exit 1
}
$proc = Start-Process $ExePath -PassThru
Start-Sleep -Seconds 3

if (-not $proc.HasExited) { Pass "App démarrée (PID $($proc.Id))" }
else { Fail "App démarrée" "Processus terminé prématurément"; exit 1 }

# ---- 3. Test enregistrement court (mic seul, 10s) ----
Log "=== 3. Enregistrement 10s (mic seul) ==="
$wsh = New-Object -ComObject WScript.Shell
$wsh.SendKeys("^+{F9}")
Log "Démarrage via Ctrl+Shift+F9"
Start-Sleep -Seconds 10

$wsh.SendKeys("^+{F9}")
Log "Arrêt via Ctrl+Shift+F9"
Start-Sleep -Seconds 5

# ---- 4. Vérifier le fichier MP4 produit ----
Log "=== 4. Vérification MP4 ==="
$mp4Files = Get-NewMp4 $null

if ($mp4Files) {
    Pass "Fichier MP4 trouvé : $($mp4Files.Name)"
    $sizeMB = [math]::Round($mp4Files.Length / 1MB, 2)
    if ($mp4Files.Length -gt 100000) { Pass "Taille MP4 raisonnable ($sizeMB MB)" }
    else { Fail "Taille MP4 raisonnable" "Trop petit : $($mp4Files.Length) octets" }

    # Analyse ffprobe
    $probeJson = & $FfprobePath -v quiet -print_format json -show_streams -show_format $mp4Files.FullName 2>&1
    try {
        $probe = $probeJson | ConvertFrom-Json
        $duration = [double]$probe.format.duration

        if ($duration -ge 8 -and $duration -le 15) { Pass "Durée MP4 correcte (${duration}s)" }
        else { Fail "Durée MP4 correcte" "Attendu ~10s, obtenu ${duration}s" }

        $videoStream = $probe.streams | Where-Object { $_.codec_type -eq "video" }
        $audioStream = $probe.streams | Where-Object { $_.codec_type -eq "audio" }

        if ($videoStream -and $videoStream.codec_name -eq "h264") { Pass "Flux vidéo H264 présent" }
        else { Fail "Flux vidéo H264 présent" "Stream: $($videoStream.codec_name)" }

        if ($audioStream -and $audioStream.codec_name -eq "aac") { Pass "Flux audio AAC présent" }
        else { Fail "Flux audio AAC présent" "Stream: $($audioStream.codec_name)" }

        Test-Timing $mp4Files.FullName "(MP4 #1)"

    } catch {
        Fail "Parse ffprobe JSON" $_.Exception.Message
    }
} else {
    Fail "Fichier MP4 trouvé" "Aucun nouveau fichier dans $OutputDir (voir $env:TEMP\yawrec.log)"
}

# ---- 5. Test pause/reprise (5s rec, pause 3s, reprise 5s) ----
Log "=== 5. Test Pause / Reprise ==="
Start-Sleep -Seconds 2
$wsh.SendKeys("^+{F9}")
Log "Démarrage enregistrement #2"
Start-Sleep -Seconds 5

$wsh.SendKeys("^+{F10}")
Log "Pause via Ctrl+Shift+F10"
Start-Sleep -Seconds 3

$wsh.SendKeys("^+{F10}")
Log "Reprise via Ctrl+Shift+F10"
Start-Sleep -Seconds 5

$wsh.SendKeys("^+{F9}")
Log "Arrêt"
Start-Sleep -Seconds 5

# Vérifier le second MP4
$mp4Files2 = Get-NewMp4 $mp4Files.FullName
if ($mp4Files2) {
    Pass "Second MP4 produit : $($mp4Files2.Name)"
    $probe2Json = & $FfprobePath -v quiet -print_format json -show_streams -show_format $mp4Files2.FullName 2>&1
    try {
        $probe2 = $probe2Json | ConvertFrom-Json
        $dur2 = [double]$probe2.format.duration
        # 5s rec + 5s rec = 10s (sans les 3s de pause)
        if ($dur2 -ge 8 -and $dur2 -le 14) { Pass "Durée avec pause correcte (~10s effectif, obtenu ${dur2}s)" }
        else { Fail "Durée avec pause correcte" "Attendu ~10s effectif, obtenu ${dur2}s" }
        Test-Timing $mp4Files2.FullName "(MP4 #2, pause)"
    } catch {
        Fail "Parse ffprobe second MP4" $_.Exception.Message
    }
} else {
    Fail "Second MP4 produit" "Pas de nouveau fichier dans $OutputDir"
}

# ---- 6. Fermer l'app ----
Log "=== 6. Arrêt app ==="
Stop-Process -Name "yawrec" -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
Pass "App fermée"

# ---- Résumé ----
Log ""
Log "============================================"
Log "RÉSUMÉ DES TESTS"
Log "============================================"
$results | ForEach-Object { Log $_ }
Log ""
if ($errors.Count -eq 0) {
    Log "TOUS LES TESTS PASSÉS ($($results.Count)/$($results.Count))"
} else {
    Log "ÉCHECS : $($errors.Count) / $($results.Count)"
    $errors | ForEach-Object { Log "  - $_" }
}
Log "============================================"

# Retourner les résultats pour le rapport
return @{
    Total = $results.Count
    Passed = ($results | Where-Object { $_ -like "✓*" }).Count
    Failed = $errors.Count
    Details = $results
    Errors = $errors
}
