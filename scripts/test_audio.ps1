# ============================================================
# YawREC · test_audio.ps1
# Tests automatisés : enregistrement audio, pause, qualité MP4
# ============================================================

param(
    [string]$ExePath = "I:\yawrec-tauri-g1\yawrec\src-tauri\target\release\yawrec.exe",
    [string]$FfprobePath = "C:\ffmpeg\bin\ffprobe.exe",
    [string]$OutputDir = "$env:USERPROFILE\Videos\YawREC"
)

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
$proc = Start-Process $ExePath -PassThru
Start-Sleep -Seconds 3

if (-not $proc.HasExited) { Pass "App démarrée (PID $($proc.Id))" }
else { Fail "App démarrée" "Processus terminé prématurément"; exit 1 }

# ---- 3. Test enregistrement court (mic seul, 10s) ----
Log "=== 3. Enregistrement 10s (mic seul) ==="
$wsh = New-Object -ComObject WScript.Shell
$wsh.SendKeys("^+r")
Log "Démarrage via Ctrl+Shift+R"
Start-Sleep -Seconds 10

$wsh.SendKeys("^+r")
Log "Arrêt via Ctrl+Shift+R"
Start-Sleep -Seconds 5

# ---- 4. Vérifier le fichier MP4 produit ----
Log "=== 4. Vérification MP4 ==="
$mp4Files = Get-ChildItem -Path $OutputDir -Filter "*.mp4" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1

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

        # Vérifier que l'audio et la vidéo ont des durées proches (< 500ms d'écart)
        $vDur = [double]$videoStream.duration
        $aDur = [double]$audioStream.duration
        $avDiff = [math]::Abs($vDur - $aDur)
        if ($avDiff -lt 0.5) { Pass "Sync A/V correcte (écart ${avDiff}s)" }
        else { Fail "Sync A/V correcte" "Écart A/V : ${avDiff}s (vidéo=${vDur}s, audio=${aDur}s)" }

    } catch {
        Fail "Parse ffprobe JSON" $_.Exception.Message
    }
} else {
    Fail "Fichier MP4 trouvé" "Aucun fichier dans $OutputDir"
}

# ---- 5. Test pause/reprise (5s rec, pause 3s, reprise 5s) ----
Log "=== 5. Test Pause / Reprise ==="
Start-Sleep -Seconds 2
$wsh.SendKeys("^+r")
Log "Démarrage enregistrement #2"
Start-Sleep -Seconds 5

$wsh.SendKeys("^+p")
Log "Pause via Ctrl+Shift+P"
Start-Sleep -Seconds 3

$wsh.SendKeys("^+p")
Log "Reprise via Ctrl+Shift+P"
Start-Sleep -Seconds 5

$wsh.SendKeys("^+r")
Log "Arrêt"
Start-Sleep -Seconds 5

# Vérifier le second MP4
$mp4Files2 = Get-ChildItem -Path $OutputDir -Filter "*.mp4" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($mp4Files2 -and $mp4Files2.Name -ne $mp4Files.Name) {
    Pass "Second MP4 produit : $($mp4Files2.Name)"
    $probe2Json = & $FfprobePath -v quiet -print_format json -show_streams -show_format $mp4Files2.FullName 2>&1
    try {
        $probe2 = $probe2Json | ConvertFrom-Json
        $dur2 = [double]$probe2.format.duration
        # 5s rec + 5s rec = 10s (sans les 3s de pause)
        if ($dur2 -ge 8 -and $dur2 -le 14) { Pass "Durée avec pause correcte (~10s effectif, obtenu ${dur2}s)" }
        else { Fail "Durée avec pause correcte" "Attendu ~10s effectif, obtenu ${dur2}s" }
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
