# Vérifie les patterns dans yawrec.log et retourne un résumé
param([int]$TailLines = 50)
$log = Get-Content "$env:TEMP\yawrec.log" -Tail $TailLines -ErrorAction SilentlyContinue
if (-not $log) { Write-Host "Pas de log trouvé"; return }

Write-Host "=== yawrec.log (dernières $TailLines lignes) ==="
$log | ForEach-Object {
    # Décoder les caractères mal encodés (UTF-8 lu en Latin-1)
    $_ -replace 'Ã©','é' -replace 'Ã¨','è' -replace 'Ã ','à' -replace 'Ã»','û' -replace 'â€"','—' -replace 'â†'','→' -replace 'Â·','·' -replace 'â–¶','▶' -replace 'â–ˆ','■' -replace 'â–ˆ','■' -replace 'â–ˆ','⏸' -replace 'â¸','⏸' -replace 'â¹','⏵'
}
