# 2026-10-24 — Campagne `TESTS_WINDOWS.md` sur `fix/revue-complete` @ fe313a6

Version testée : branche `fix/revue-complete`, commit `fe313a639d6f654f8b09e9e0237d6d5360f0b167`
(24/10 13:40 UTC, « docs: compléter la procédure de test Windows (instance unique, script) »).
`master` reste à `eb4ed16` (version du test E2E précédent, cf. `2026-10-24-test-e2e.md`).

Exe : artifact CI `yawrec-windows-portable-fe313a639d6f654f8b09e9e0237d6d5360f0b167`
(run https://github.com/coucagog/YawREC/actions/runs/40099196890, 69,6 Mo), à dézipper dans
`C:\Users\hp\yawrec-test\` (hors arborescence de build).

## Journal

- `git fetch origin` : `fix/revue-complete` 03d41ef → fe313a6. Procédure lue via
  `git show origin/fix/revue-complete:TESTS_WINDOWS.md` (15 tests).
- API GitHub : run `completed / success`, head_sha = fe313a6…, artifact présent, non expiré.
- Téléchargement de l'artifact : nécessite d'être authentifié (`gh` non connecté sur la machine)
  → en attente de l'utilisateur.
- L'utilisateur fournit `YawREC-fix-revue-complete.zip` (racine du dépôt) : c'est l'archive **des
  sources** (« Download ZIP », commentaire = fe313a639d6f…, 178 fichiers, aucun .exe/.dll), pas
  l'artifact CI. → Bascule sur la voie « test à partir des sources ».
- `git checkout fix/revue-complete` → `fe313a6` ✓. 40 fichiers changés depuis eb4ed16.
- La branche versionne désormais `Cargo.lock` (Tauri 2.12.5, plugins 2.8.3 / 2.7.0 / 2.4.1,
  `tauri-plugin-single-instance` 2.5.4) et `package-lock.json` (`@tauri-apps/*` 2.12.4 / 2.8.3 /
  2.7.0 / 2.4.1) : versions alignées, les épinglages de la session précédente ne servent plus.
- `build.ps1` de la branche : repli sur `npx.cmd tauri` si `cargo tauri` absent, contrôle
  FFmpeg 7.x/8.x (`avcodec-61/62.dll`), fichier en UTF-8 avec BOM.
- `npm ci` (node_modules de master obsolète ; `build.ps1` ne réinstalle que si absent) → CLI 2.12.4.
- Build : `scripts\build.ps1 -SkipNpm` → **KO au pré-flight** (bug de la branche) :
  ```
  cargo : error: no such command: `tauri`
  Au caractère ...\yawrec\scripts\build.ps1:51 : 9
  +     if (cargo tauri --version 2>$null) {
      + FullyQualifiedErrorId : NativeCommandError
  ```
  Sous Windows PowerShell 5.1 avec `$ErrorActionPreference = "Stop"`, le stderr d'une commande
  native redirigé par `2>$null` devient une erreur terminante : le repli `npx.cmd tauri` n'est jamais
  atteint quand `cargo tauri` est absent (le cas qu'il devait justement couvrir).
  Correctif suggéré : tester via `Get-Command`/`$LASTEXITCODE` dans un bloc où
  `$ErrorActionPreference = "Continue"`, par ex.
  `$ErrorActionPreference="Continue"; cargo tauri --version *> $null; $hasCargoTauri = $LASTEXITCODE -eq 0; $ErrorActionPreference="Stop"`.
- Contournement : étapes du script reproduites à la main (copie DLL FFmpeg 8.1.2 → resources +
  release, `npx tauri build --no-bundle`).
- 1er essai KO : `build.rs` de yawrec → `os error 32` (fichier utilisé par un autre processus) et
  `Copy-Item` refusé sur `target\release\avcodec-62.dll`. Cause : une instance de l'**ancien** exe
  eb4ed16 (PID 5624, lancée depuis l'Explorateur à 13:29 depuis `target\release`) tournait en
  tâche de fond. Arrêtée, build relancé.
- 2e essai KO, même erreur : `target\release\yawrec-copy.exe` (PID 7888, 13:30:03) tenait
  `avcodec-62.dll`. C'était la copie créée par l'agent pour le test d'emplacement (session
  précédente) : son `Remove-Item` avait échoué silencieusement (fichier en cours d'utilisation), puis
  elle a été relancée. Arrêtée et supprimée ; plus aucun `yawrec*` en cours. Build relancé (3e essai).

- 3e essai : build OK (`Finished release in 4m 35s`, `yawrec.exe` 7 491 072 o, 14:37).
  Copié avec les DLL FFmpeg 8.1.2 dans `C:\Users\hp\yawrec-test\`.

## Configuration testée

- Windows 11 Pro 10.0.22631 (23H2)
- CPU Intel Core i5-7500T 2,7 GHz, 4 cœurs / 4 threads ; pas de NVENC (`nvcuda.dll` absent),
  QSV refuse yuv420p, AMF absent
- 1 écran, 1920×1080
- Encodeur : `Encodeur prêt : libx264`
- Micro : « Microphone externe (Synaptics HD Audio) », 48 kHz stéréo F32

## Résultats des tests automatisés (agent)

### T1 (partie journal) — OK
`Encodeur prêt : libx264`, `YawREC · fenêtre prête`, `WS remote control server listening on :9799`,
aucune erreur. Journal : `C:\Users\hp\yawrec-test\yawrec-T1.log`.
(Affichage de l'UI / VU-mètre : à vérifier par l'utilisateur.)

### T13 instance unique — OK, avec un défaut
- 2e lancement : le processus se termine immédiatement (code 0), journal
  `2e instance lancée — fenêtre existante ramenée`, une seule instance restante.
- **Défaut** : la 2e instance ouvre `%TEMP%\yawrec.log` avec `truncate(true)` *avant* que
  `tauri-plugin-single-instance` ne la fasse quitter → le journal de l'instance en cours est effacé ;
  celle-ci continue d'écrire à son ancien offset → fichier rempli d'octets NUL puis reprise.
  Or la procédure demande justement de renvoyer `yawrec.log` d'une session KO.
  Piste : initialiser le logger (ou ouvrir le fichier) après le contrôle d'instance unique, ou
  n'ouvrir qu'en `append` et tronquer seulement dans l'instance principale.

### T14 script automatique — **KO 10/11** (sync A/V)
```
✗ Sync A/V correcte — Écart A/V : 0.618s (vidéo=9.141s, audio=9.759s)
✓ Second MP4 produit : YawREC_2026-10-24_14-38-31.mp4
✓ Durée avec pause correcte (~10s effectif, obtenu 10.04s)
```
Journal : `C:\Users\hp\yawrec-test\yawrec-T14.log`.

ffprobe :
```
YawREC_2026-10-24_14-38-14.mp4
stream|codec_name=h264|avg_frame_rate=200000/9141|duration=9.141000|nb_frames=200
stream|codec_name=aac|sample_rate=48000|channels=2|duration=9.759000|nb_frames=458
YawREC_2026-10-24_14-38-31.mp4
stream|codec_name=h264|avg_frame_rate=263000/10017|duration=10.008000|nb_frames=263
stream|codec_name=aac|sample_rate=48000|channels=2|duration=9.749000|nb_frames=458
```

Analyse des horodatages vidéo (pts des paquets) :

| Fichier | Version | Images | 1re → dernière | Écart moyen | Trous > 100 ms |
|---|---|---|---|---|---|
| 13-20-59 | eb4ed16 | 366 | 0,008 → 9,98 s | 27,3 ms | 3 (max 1 735 ms) |
| 13-21-17 | eb4ed16 | 375 | 0,012 → 10,02 s | 26,8 ms | 2 (max 1 562 ms) |
| 14-38-14 | fe313a6 | 200 | **0,273** → 10,01 s | **48,9 ms** (~20 fps) | 4 : 141, **1 622**, 716, 750 ms |
| 14-38-31 | fe313a6 | 263 | 0,032 → 10,04 s | ~38 ms | 2 : 549, 766 ms |

Constats :
- Le « 0,618 s » vient de la *durée de piste* rapportée par ffprobe (9,141 s) ; la vidéo s'étend en
  réalité de 0,273 à 10,01 s. Le critère du script (différence des durées de flux) est fragile en VFR.
- Mais il y a de **vrais gels d'image** de 0,5 à 1,6 s, et la cadence moyenne tombe à ~20–26 fps
  au lieu de 30. La 1re image arrive à 0,273 s (encodeur créé paresseusement à la 1re frame).
- Le code de fe6dc79 (`commands.rs` ~l.240–327) est censé répéter la dernière image à la cadence
  demandée (timeout `AcquireNextFrame` ≈ 34 ms, `WAIT_TIMEOUT` → `None`, implémentation DXGI
  correcte, `screen.rs` ~l.539). Les trous signifient donc que **la boucle elle-même est bloquée**,
  vraisemblablement dans `push_video_frame` : clone BGRA 8 Mo + conversion YUV + libx264 1080p
  synchrone + `Mutex` de l'encodeur partagé avec le worker audio, sur un i5-7500T 4 cœurs sans
  encodeur matériel. L'ancienne version montrait des trous aux mêmes endroits.
- Pistes : mesurer la durée de `push_video_frame` et l'attente du `Mutex` (log debug), découpler
  capture et encodage (canal + thread encodeur), preset x264 plus rapide quand il n'y a pas de HW,
  créer l'encodeur au démarrage plutôt qu'à la 1re image.
- Ce point impacte aussi T3 (synchro) et T5 (écran statique / webcam animée) : à vérifier.

### T6 (raccourcis) / T4 (pause) — partiel via T14
Ctrl+Shift+F9 (start/stop) et Ctrl+Shift+F10 (pause/reprise) fonctionnent ; pause correctement
retirée (10,04 s effectifs pour 5 s + 3 s de pause + 5 s), audio stéréo 48 kHz.
Reste à vérifier : app réduite, icône de barre des tâches, notification, absence d'image/son de pause.

## Nouvelle version : f369546 (24/10, 14:49 UTC)

- Run CI vert annoncé : https://github.com/coucagog/YawREC/actions/runs/40104201953
  (artifact `yawrec-windows-portable-f369546712622362d704481951f363f0763596e8`).
- Le zip fourni (`YawREC-fix-revue-complete.zip`, 15:17, aussi dans Téléchargements) est de nouveau
  l'archive **des sources** (commentaire f369546…, 178 fichiers, aucun .exe/.dll) → build depuis les sources.
- `git pull` → `f369546`. Commits depuis fe313a6 :
  - `c4e5e86` fix(desktop): gels d'image avec x264 et fin d'audio tronquée
  - `69fc97b` fix(desktop): une 2e instance n'efface plus le journal de la 1re
  - `c292ca1` fix(scripts): build.ps1 sous PowerShell 5.1, test_audio mesure gels et synchro
  - `f369546` docs: procédure de test Windows — journal, gels, script 14/14
- Aucun `yawrec*` en cours. Build via `scripts\build.ps1 -SkipNpm` (teste aussi son correctif PS 5.1).
- **build.ps1 OK sous PS 5.1** : `✓ tauri CLI (npx tauri, cargo tauri absent)`,
  `✓ Version FFmpeg compatible (avcodec-62.dll)`, release en 4m 21s + 2 bundles (MSI/NSIS).
  Une ligne `NativeCommandError` (stderr npm) apparaît pendant le build mais n'est pas bloquante.
- `C:\Users\hp\yawrec-test\` vidé puis rempli : `yawrec.exe` f369546 (7 495 680 o, 15:30) + DLL 8.1.2.

### f369546 — T1 (journal) : OK
`Encodeur prêt : libx264`, fenêtre prête, WS `:9799`, aucune erreur.
Remarque : 8 s après le lancement, `yawrec.log` contenait encore l'ancienne session ; le journal n'est
(ré)initialisé qu'à 15:30:50 pour un lancement vers 15:30:42 (1er lancement de l'exe neuf, dossier non
exclu de Defender — probablement l'analyse ; non compté comme défaut).

### f369546 — T13 instance unique : OK, défaut de journal corrigé
2e lancement → code 0, une seule instance. Journal de l'instance principale intact (1 913 o, **0 octet
NUL**), contient `2e instance lancée — fenêtre existante ramenée`. Le fichier est désormais ouvert sans
partage en lecture simple (`ReadAllBytes` refuse ; lecture OK avec `FileShare.ReadWrite`, ce que fait
`Get-Content`/l'Explorateur).

### f369546 — T14 script (nouvelle version, 14 critères dont gels et fins A/V)

**Passage 1 : KO 5/14** (journal `yawrec-test\yawrec-T14.log`)
```
✗ Taille MP4 raisonnable — Trop petit : 0 octets
✗ Fins A/V alignées (MP4 #1) — vidéo finit à 8.282s, audio à 11.443333s
✗ Pas de gel d'image (MP4 #1) — gel de 1247 ms à 1.965s, 25.1 fps moyen
✓ Durée avec pause correcte (~10s effectif, obtenu 13.653s)
✗ Fins A/V alignées (MP4 #2, pause) — vidéo finit à 12.429s, audio à 13.632s
✗ Pas de gel d'image (MP4 #2, pause) — gel de 2503 ms à 4.948s, 19.1 fps moyen
```
Journal : tout le processus est lent pendant ce passage —
`flush + close` 5 s (15:31:44 → :49, d'où le MP4 de 0 o au moment du contrôle) ; 2e démarrage 3 s
(commande 15:31:51, DXGI 15:31:54) ; pause prise en compte ~2 s après Ctrl+Shift+F10 et durée 0,6 s
au lieu de 3 s (`⏸ pause (7421 ms)` / `⏵ reprise (8030 ms)`) → le 13,65 s ne vient pas d'une pause
mal retirée mais du raccourci reçu en retard ; encodage d'une frame jusqu'à 3 121 ms.

**Passage 2 (immédiatement après) : KO 12/14** (journal `yawrec-test\yawrec-T14-run2.log`)
```
✓ Fins A/V alignées (MP4 #1) (écart 0.003s)
✗ Pas de gel d'image (MP4 #1) — gel de 1350 ms à 5.714s, 21.2 fps moyen
✓ Durée avec pause correcte (~10s effectif, obtenu 10.517s)
✓ Fins A/V alignées (MP4 #2, pause) (écart 0.15s)
✗ Pas de gel d'image (MP4 #2, pause) — gel de 1049 ms à 1.272s, 24.9 fps moyen
```
Journal : pause exacte (5016 → 8029 ms), flush instantané, mais gels répétés :
```
Vidéo · gel de 1090 ms à t=2309 ms : encodage frame précédente 1083 ms, puis capture 7 ms, verrou encodeur 1 ms
Vidéo · gel de 1350 ms à t=7064 ms : encodage frame précédente 1343 ms, puis capture 7 ms, verrou encodeur 11 ms
Vidéo · gel de 519 ms à t=9554 ms : encodage frame précédente 509 ms, ...
Vidéo · 212 frames, encodage moyen 23.7 ms (max 1343 ms), 3 gel(s) > 200 ms
Vidéo · gel de 1049 ms à t=2321 ms : encodage frame précédente 1042 ms, ...
Vidéo · gel de 581 ms à t=6175 ms : encodage frame précédente 575 ms, ...
Vidéo · gel de 238 ms à t=8380 ms : encodage frame précédente 230 ms, ...
Vidéo · 258 frames, encodage moyen 17.3 ms (max 1042 ms), 3 gel(s) > 200 ms
```
CPU total échantillonné chaque seconde pendant le passage 2 : 4 à 25 % (pic à 39 %), jamais saturé.
Au repos : CPU 2–9 %, disque 0 %, 9,4 Go de RAM libres sur 16 Go.

Analyse :
- Les gels viennent toujours d'**un** appel d'encodage de ~0,5–1,3 s, la capture (7 ms) et le verrou
  (≤ 11 ms) sont hors de cause — l'instrumentation de c4e5e86 le montre bien.
- Le CPU étant peu chargé, ce n'est pas un manque de puissance mais une **attente** dans
  `push_video_frame`. Le chronomètre « encodage » englobe vraisemblablement aussi l'écriture
  muxée dans le MP4 (`Vidéos\YawREC`, non exclu de Defender) : hypothèse d'écritures freinées par
  l'analyse temps réel. Autre piste : x264 en `threads=auto` (frame threads + lookahead) qui bloque
  l'appel quand son pipeline est plein.
- Pistes pour trancher : chronométrer séparément `send_frame`/`receive_packet` et
  `write_interleaved` dans le journal ; refaire le test avec le dossier de sortie exclu de Defender ;
  essayer `tune=zerolatency` (pas de lookahead ni de B-frames, threads par tranches) quand il n'y a
  pas d'encodeur matériel.
- Le 1er passage montre en plus que le premier lancement d'un exe neuf (DLL de 300 Mo non exclues)
  peut tout ralentir pendant plusieurs secondes.

**Passage 3, avec exclusions Defender** (ajoutées par l'utilisateur en admin :
`C:\Users\hp\Videos\YawREC` et `C:\Users\hp\yawrec-test`) : **KO 12/14, inchangé**
(journal `yawrec-test\yawrec-T14-run3-exclusion.log`)
```
✓ Fins A/V alignées (MP4 #1) (écart 0.003s)
✗ Pas de gel d'image (MP4 #1) — gel de 1166 ms à 1.594s, 25 fps moyen
✓ Durée avec pause correcte (~10s effectif, obtenu 10.796s)
✓ Fins A/V alignées (MP4 #2, pause) (écart 0.045s)
✗ Pas de gel d'image (MP4 #2, pause) — gel de 1459 ms à 2.158s, 25.5 fps moyen
```
```
Vidéo · gel de 1166 ms à t=2760 ms : encodage frame précédente 1157 ms, puis capture 9 ms, verrou encodeur 1 ms
Vidéo · gel de 434 ms à t=7301 ms : encodage frame précédente 427 ms, ...
Vidéo · 234 frames, encodage moyen 17.3 ms (max 1157 ms), 2 gel(s) > 200 ms
Vidéo · gel de 250 ms à t=1425 ms : encodage frame précédente 242 ms, ...
Vidéo · gel de 1459 ms à t=3617 ms : encodage frame précédente 1452 ms, ...
Vidéo · 276 frames, encodage moyen 16.2 ms (max 1452 ms), 2 gel(s) > 200 ms
```
CPU total 1–48 %, jamais saturé.

→ **Defender est écarté** : les gels (un appel d'encodage de 0,4–1,5 s, CPU peu chargé) persistent
avec le dossier de sortie et l'exe exclus. Ils apparaissent surtout dans les ~3 premières secondes de
chaque enregistrement (t = 1,4 / 2,3 / 2,8 / 3,6 s), puis plus rarement. Cause la plus probable :
le comportement de x264 lui-même (threads=auto : pipeline de frame threads + lookahead qui se remplit
et bloque l'appel), à confirmer en chronométrant séparément `send_frame` / `receive_packet` /
`write_interleaved`, et en essayant `tune=zerolatency` (ou `sliced-threads`, `rc-lookahead=0`,
`bframes=0`) et/ou un preset plus rapide sans encodeur matériel.
Remarque mineure : la pause de ce passage est prise en compte à 5 799 ms au lieu de ~5 000
(raccourci reçu ~0,8 s en retard) → 2,2 s retirées au lieu de 3 s, d'où 10,8 s.

### f369546 — tests manuels (utilisateur)

**T1 lancement : OK.** Interface affichée normalement (pas de page blanche). VU-mètre = les 3 barres
à droite de l'icône micro (« trois petits points » au repos, `scaleY(0.04)`) : elles bougent quand
l'utilisateur parle.
Remarque : l'échelle est linéaire en RMS (1re barre visible au-dessus de ~0,018, pleine à 0,45 ≈ −7 dBFS) ;
avec ce micro (ambiance −48 dB moyen dans les MP4 des tests), le VU ne réagit qu'à une voix assez forte.
Piste : échelle en dB.

**Webcam Logitech branchée (début T5)** : détectée (`webcam::list_devices → 1 caméra(s)`), aperçu
affiché, choix du coin (haut-droit, bas-droit…) fonctionnel. Journal : `yawrec-test\yawrec-T1-T5-user.log`.

Enregistrements de l'utilisateur, tous en mode **Région** :

| MP4 | Vidéo (1re → dernière image) | Audio | fps | Gels > 200 ms |
|---|---|---|---|---|
| 16-10-27 (1100×680) | 0,237 → 5,404 s | 0,000 → 5,397 s | 20,1 | 1 |
| 16-16-55 (818×666) | 1,290 → 17,686 s | 1,008 → 17,648 s | 28,2 | 1 |
| 16-44-00 (1070×1022, webcam + loopback) | 1,867 → **35,816 s** | 1,578 → **37,418 s** | 22,4 | **7** |

Journal de 16-44-00 :
```
▶ start_recording (mode=Region, screen=0, webcam=true)
Worker audio · démarrage (mic=true, loopback=true)
Webcam · ouverte (1280×960) → PiP 214×120
Vidéo · gel de 1431 ms à t=4565 ms : encodage frame précédente 1421 ms, ...
... (7 gels de 867 à 1666 ms)
Vidéo · 761 frames, encodage moyen 19.5 ms (max 1656 ms), 7 gel(s) > 200 ms
```
Constats :
- **Fin vidéo 1,6 s avant la fin audio** (35,82 s vs 37,42 s) sur l'enregistrement webcam + loopback.
- **Écrêtage** : crête +1,54 dB, RMS −24,5 dB (micro + son système additionnés sans limiteur).
- **PiP au mauvais ratio** : webcam 1280×960 (4:3) → PiP 214×120 (16:9) → image déformée ou rognée
  (à confirmer visuellement). Largeur ≈ 1/5 de la zone (214/1070) ✓.
- Gels toujours présents (cf. T14).

**T5 écran statique + webcam (plein écran, 30 fps) : KO (déformation de l'incrustation)**
Journal : `yawrec-test\yawrec-T5.log`.

| MP4 | Coin | Vidéo | Audio | Zone webcam : images distinctes | Centre écran : images distinctes |
|---|---|---|---|---|---|
| 17-02-47 | bas-gauche | 0,03 → 19,04 s, 446 img | → 19,75 s | 292 (plus long figement 1,78 s) | 4 |
| 17-03-35 | haut-gauche | 0,05 → 21,32 s, 562 img | → 21,29 s | 372 (plus long figement 1,29 s) | 4 |

(images distinctes comptées avec `crop` + `mpdecimate` sur la zone de l'incrustation et sur une zone
centrale du bureau)
- Durée ≈ 20 s ✓ (19,7 s / 21,3 s, chronométrage manuel), le bureau reste figé et la vidéo continue ✓.
- **Webcam animée sur écran statique** ✓ — ses plus longs figements (1,78 / 1,29 s) correspondent
  exactement aux gels d'encodage du journal (`gel de 1771 ms`, `gel de 1265 ms`).
- Taille : `Webcam · ouverte (1280×960) → PiP 384×216` = 1/5 de 1920 ✓.
- Coins testés : bas-gauche, haut-gauche (plein écran), haut-droit (région, plus tôt) ✓.
- **KO : tête aplatie** (constat utilisateur, confirmé sur des captures à t = 10 s) : la source 4:3
  (1280×960) est étirée dans un cadre 16:9 (384×216) au lieu d'être recadrée ou letterboxée.
  Piste : calculer la hauteur du PiP depuis le ratio de la source, ou rogner la source au 16:9.
- 3 gels d'encodage par enregistrement (cf. T14).

**T2 enregistrement simple (plein écran, micro + son système, YouTube) : partiel, points à confirmer**
Journal : `yawrec-test\yawrec-T2.log`. Retour utilisateur : curseur visible ✓, tous les sons
audibles, « un semblant d'écho » sur les voix YouTube, le reste stable.

| MP4 | Micro | Loopback | Durée | L / R RMS | L−R | corr(L,R) |
|---|---|---|---|---|---|---|
| 17-13-59 | Microphone externe (Synaptics HD Audio) | Casque (Synaptics HD Audio), mode Stereo | 21,4 s | −15,4 / −15,4 dB | −59,2 dB | 1,0000 |
| 17-15-43 | Microphone (Logi C270 HD WebCam) | idem | 16,4 s | −10,7 / −10,7 dB | −60,8 dB | 1,0000 |

(PCM 16 kHz extrait avec ffmpeg, analyse numpy : `scratchpad\echo.py`, `scratchpad\spec.py`)
- **Stéréo : KO probable** — les deux canaux sont identiques (L−R à −60 dB, corrélation 1,0000) alors que
  le journal annonce le loopback en `mode Stereo`. À confirmer selon la vidéo YouTube (était-elle stéréo ?).
- **Écho (17-13-59)** : pic d'autocorrélation à **79 ms** (force 0,185) → une copie du son retardée
  de ~79 ms. Cohérent avec le micro qui recapte les haut-parleurs (décalage = latence micro vs loopback),
  pas avec un doublon logiciel du loopback (qui donnerait une corrélation proche de 1). Sans annulation
  d'écho, c'est attendu si l'écoute se fait sur haut-parleurs. À confirmer : haut-parleurs ou casque ?
- **Ronflement secteur (17-15-43, micro C270)** : 50 Hz + 100 Hz = ~70 % de l'énergie du signal sur toute
  la durée (autocorrélation 0,95 à 20/40/60 ms), RMS −10,7 dB → bourdonnement fort. À confirmer à l'écoute.
- Crête −0,39 dBFS sur 17-15-43 (proche de l'écrêtage, cf. région + loopback plus haut).
- Gels toujours présents (3 et 2 par enregistrement, jusqu'à 1,7 s).

Précisions de l'utilisateur : écoute **aux écouteurs** ; vidéo YouTube = **test stéréo** ; bourdonnement
entendu avec le micro de la **webcam**, pas avec le micro interne ; « son décalé de quelques ms ».

Code vérifié : loopback ouvert en `ChannelMode::Stereo` (`(2, Stereo)` = copie telle quelle,
`capture/audio.rs` ~l.501), mixage micro + loopback échantillon par échantillon (`pull_chunk`),
encodeur AAC `ChannelLayout::STEREO` avec désentrelacement manuel L/R (`encoder/mod.rs` ~l.448) ;
option Windows « Audio mono » désactivée (`AccessibilityMonoMixState` absent).

**Test stéréo contrôlé (agent)** : WAV généré (440 Hz 2 s à **gauche seule**, puis 880 Hz 2 s à
**droite seule**), joué via `System.Media.SoundPlayer` pendant un enregistrement Ctrl+Shift+F9
(micro C270 + loopback « Casque (Synaptics HD Audio) ») → `YawREC_2026-10-24_17-32-56.mp4` :
```
t= 4.5s  440 Hz : G   388.7  D    50.6   |  880 Hz : G     5.0  D     5.0
t= 6.0s  440 Hz : G   389.6  D    51.3   |  880 Hz : G     4.0  D     4.0
t= 7.5s  440 Hz : G    15.7  D    15.7   |  880 Hz : G    62.7  D   433.8
t= 9.0s  440 Hz : G     5.7  D     5.7   |  880 Hz : G    55.1  D   438.8
```
→ **stéréo conservée** (séparation ≈ 18 dB). Le résidu égal sur les deux canaux (~13 %) est le micro
(mono dupliqué L/R) qui capte la fuite sonore des écouteurs.

Conclusions T2 :
- Curseur visible et bien placé ✓ (utilisateur).
- **Stéréo du son système ✓** (test contrôlé). Le mono strict des enregistrements YouTube vient de la
  source (flux joué en mono, ou passage gauche/droite hors de l'enregistrement), pas de YawREC.
- **Écho de ~79 ms** : recapture par le micro de la fuite des écouteurs (même mécanisme que le résidu
  ci-dessus), décalée de la latence micro vs loopback. Pas un doublon logiciel. Comportement attendu
  sans annulation d'écho ; piste d'amélioration : AEC, ou conseiller de couper le micro quand on ne parle pas.
- **Bourdonnement 50/100 Hz** : propre au micro de la webcam C270 (absent avec le micro interne) →
  matériel / masse USB, pas YawREC. Piste : filtre passe-haut ~80–100 Hz sur la voie micro.
- Volume micro et son système normaux à l'écoute ✓ (utilisateur) ; attention à l'écrêtage quand micro
  et son système sont forts en même temps (crêtes −0,4 et +1,5 dB mesurées) → limiteur sur le mix.
- **Verdict T2 : OK**, avec remarques (gels vidéo, écrêtage possible, écho/ronflement côté matériel).

**T3 synchro (vidéo YouTube « audio video sync test », plein écran) : pas de dérive ✓ ; décalage
constant ~115 ms (audio en avance) à confirmer**
Fichier `YawREC_2026-10-24_17-37-15.mp4` (42,9 s, 1 120 images, micro C270 + loopback),
journal `yawrec-test\yawrec-T3.log` (8 gels de 220 à 1 287 ms). Ressenti utilisateur : « le son et le
flash correspondent bien ».

Mesure (agent) : luminosité de la zone qui clignote le plus (images 32×18 en niveaux de gris, pts exacts
via `best_effort_timestamp_time`) ; bips = attaques de l'enveloppe audio après **passe-haut 300 Hz**
(sans filtre, le ronflement 50 Hz du micro C270, crêtes à −6 dB, masquait tout). Bip identifié :
780 Hz, 15 ms, une fois par seconde (à x,38 s). Scripts : `scratchpad\sync.py`, `scratchpad\beeps.py`.
```
flash   4.51 s   bip - flash =  -133 ms
flash  10.51 s   bip - flash =  -133 ms
flash  20.49 s   bip - flash =  -112 ms
flash  26.56 s   bip - flash =  -185 ms   <- pendant le gel de 235 ms à t=26560 ms
flash  30.52 s   bip - flash =  -144 ms
flash  36.48 s   bip - flash =  -102 ms
flash  42.47 s   bip - flash =   -98 ms
```
(20 flashs appariés ; les 2 premiers — début du fichier et gel de 852 ms à 3,3 s — sont aberrants)
- **Aucune dérive** : ≈ −130 ms au début, ≈ −100 ms à la fin ✓.
- **Décalage constant ≈ −115 ms** (audio en avance de ~3,5 images) → hors du critère « ±1 image »
  de la procédure, bien que non perçu par l'utilisateur. Une partie peut être structurelle : le loopback
  capte le son au moment où il est envoyé à la sortie audio, avant sa latence de sortie, alors que le
  navigateur synchronise image et son *audibles*. À vérifier avec une source dont la synchro est connue,
  et en regardant la compensation `OUTPUT_DELAY` (250 ms) de `capture/audio.rs`.
- Les gels décalent ponctuellement le flash (26,56 s : −185 ms).

**T4 pause (bouton de l'interface) : OK**
`YawREC_2026-10-24_17-44-52.mp4`, journal `yawrec-test\yawrec-T4.log`. Ressenti utilisateur : 20 s,
rien de la pause (ni image ni voix).
```
⏸ pause (10685 ms)
⏵ reprise (20213 ms)
Vidéo · 523 frames, encodage moyen 14.6 ms (max 1219 ms), 3 gel(s) > 200 ms
```
- 9,5 s de pause retirées ; vidéo 0,06 → 20,37 s, audio −0,02 → 20,33 s ✓.
- Aucun trou ni saut au point de pause (~10,7 s) ; audio continu (aucun trou > 30 ms) ✓.
- La pause par raccourci Ctrl+Shift+F10 était déjà validée par T14 (passages 2 et 3).
- Gels : 501, 1 225 et 1 043 ms, tous dans les 6 premières secondes.

**T6 raccourcis (app réduite) + icône de la barre des tâches : KO partiel (notifications absentes)**
Journal `yawrec-test\yawrec-T6.log`. Retour utilisateur : les notifications n'apparaissent pas,
l'icône fonctionne.
```
17:47:20 ▶ start_recording ... 17:47:30 ■ stop_recording → YawREC_2026-10-24_17-47-20.mp4   (Ctrl+Shift+F9, app réduite)
17:47:48 ▶ start_recording ... 17:48:00 ■ stop_recording → YawREC_2026-10-24_17-47-48.mp4   (menu de l'icône)
```
- Raccourcis avec l'app réduite ✓, démarrage/arrêt depuis l'icône ✓ (2 MP4 : 2,3 et 3,2 Mo).
- **Notifications : KO**. `notify()` (`commands.rs` l.866) n'a journalisé aucune erreur → `show()`
  réussit mais Windows n'affiche rien. Réglage Windows `Notifications\Settings\app.yawrec.desktop`
  présent sans `Enabled=0` (donc autorisé). **Aucun raccourci YawREC dans le menu Démarrer** :
  cause probable = exe portable sans AppUserModelID enregistré (Windows ignore silencieusement les
  toasts d'une app non installée ; l'installeur MSI/NSIS crée le raccourci qui porte l'identifiant).
  À confirmer en installant le bundle NSIS/MSI. Pour la version portable : créer le raccourci
  (avec `System.AppUserModel.ID` = identifiant) au 1er lancement, ou documenter la limite.

**T7 coupure d'accès à l'écran : KO (UAC → vidéo noire jusqu'à la fin)**
`YawREC_2026-10-24_17-54-27.mp4` (34,7 s), journal `yawrec-test\yawrec-T7.log`. Retour utilisateur :
l'enregistrement continue, mais « tout est noir dans la vidéo après Exécuter en tant qu'admin ».

| Heure | Événement | Journal |
|---|---|---|
| 17:54:27 | début | `▶ start_recording` |
| 17:54:53 | Ctrl+Alt+Suppr | `DXGI · accès perdu`, puis `récupération en attente : DuplicateOutput : Accès refusé (0x80070005) [RDP/VM non supporté ?]` (attendu sur l'écran sécurisé), puis récupération ✓ |
| 17:54:56 → 17:55:02 | invite UAC puis « Non » | **1 627 cycles** `accès perdu` / `duplication récupérée après ~0,5–1,4 ms` (jusqu'à 570/s), jusqu'à l'arrêt |

Luminosité moyenne du MP4, une valeur par seconde : 38 de 1 s à 28 s, puis **0 de 29 s à 34 s**.

- Ctrl+Alt+Suppr : OK (accès perdu, attente, reprise, image continue).
- **UAC : KO**. `DuplicateOutput` réussit mais le 1er `AcquireNextFrame` renvoie aussitôt
  `DXGI_ERROR_ACCESS_LOST` → boucle serrée sans délai (CPU), aucune image valide, vidéo noire, et
  pas de reprise après fermeture de l'invite (pendant les ~5 s restantes).
- Pistes : après une perte, attendre (backoff) au lieu de réessayer immédiatement ; recréer le device
  D3D11 et ré-énumérer l'adaptateur/la sortie, pas seulement `DuplicateOutput` ; détecter le bureau
  sécurisé (`OpenInputDesktop`) et répéter la dernière image valide plutôt qu'un cadre noir ; ne pas
  remplacer `last_base` par une image noire. Le message `[RDP/VM non supporté ?]` est trompeur ici.

**T8 quitter en plein enregistrement : OK pour la fermeture de fenêtre ; « Quitter » de l'icône à confirmer**
Retour utilisateur : l'app se ferme, le MP4 se lit.
- Un seul MP4 produit depuis 17:56 : `YawREC_2026-10-24_18-01-50.mp4` (3,8 Mo, 11,56 s) ; décodage
  complet sans corruption (les messages `non monotonically increasing dts` viennent du muxer `null`
  de la commande de vérification ; on les retrouve aussi sur un MP4 arrêté normalement).
- Vidéo 0,03 → 10,95 s, audio −0,02 → 11,54 s : **la vidéo s'arrête 0,6 s avant l'audio** à la
  fermeture (dernières images non vidées ?).
- Le journal de cette session a été écrasé par le relancement de 18:02:07 (journal effacé à chaque
  lancement, comme documenté) → pas de trace de la séquence d'arrêt.
- 2e scénario fait ensuite par l'utilisateur (« Quitter » depuis l'icône pendant un enregistrement,
  sans relancer) : ✓. Journal `yawrec-test\yawrec-T8-quitter.log` :
  ```
  18:04:52 ▶ start_recording (mode=Fullscreen, screen=0, webcam=false)
  18:05:04 Vidéo · 250 frames, encodage moyen 21.3 ms (max 1570 ms), 3 gel(s) > 200 ms
  18:05:04 Worker vidéo · terminé / Worker audio · terminé / Encoder · flush + close
  18:05:04 Encoder · MP4 fermé — 250 frames vidéo, 5310854 octets, codec libx264
  18:05:05 ■ stop_recording → YawREC_2026-10-24_18-04-52.mp4
  ```
  Vidéo 0,06 → 12,33 s, audio −0,02 → 12,33 s : fins alignées ✓. Processus terminé ✓.
- **Verdict T8 : OK** (remarque : fin vidéo 0,6 s plus courte lors de la fermeture par la croix).

**T10 dossier de sortie : OK**
Journal `yawrec-test\yawrec-T10.log`. Dossier choisi par l'utilisateur : `C:\Users\hp\Videos\yc`.
```
18:37:34 YawREC · démarrage
18:37:34 set_output_directory → C:\Users\hp\Videos\yc        <- restauré au lancement (localStorage WebView)
18:37:46 ■ stop_recording → C:\Users\hp\Videos\yc\YawREC_2026-10-24_18-37-41.mp4
```
Dossier conservé après Quitter + relance ✓, MP4 écrit dans le nouveau dossier ✓.
Remarque : la persistance repose sur le `localStorage` de la WebView (profil
`AppData\Local\app.yawrec.desktop\EBWebView`), pas sur un fichier de config côté Rust.

**T9 fenêtre et région sur 2 écrans : non testable** (machine à un seul écran, `list_screens (DXGI) → 1 écran(s)`).
Le mode région a été utilisé sur un écran par l'utilisateur (cf. début T5) sans anomalie signalée.

**T11 / T12 téléphone : en attente** — l'utilisateur n'a pas l'APK. Machine sans SDK Android ni Java
(`ANDROID_HOME`/`JAVA_HOME` vides, pas de `%LOCALAPPDATA%\Android\Sdk`) → build local de l'APK
impossible sans installation. Options : artifact Android du run CI f369546, ou installer JDK 17 +
SDK Android. Reportés en fin de campagne.

**T15 matériel varié : configurations 1 et 2 OK (mesures) ; Bluetooth et autre fréquence non faits**
Journal `yawrec-test\yawrec-T15.log`, dossier de sortie `C:\Users\hp\Videos\yc`.

| Config | Journal | Durée audio / vidéo | Trous audio > 30 ms | Moyen / crête |
|---|---|---|---|---|
| Micro seul (18-46-57) | `mic[Microphone externe (Synaptics HD Audio)] 48000 Hz, MonoFold`, `loopback=false` | 7,13 / 7,13 s | 0 | −27,0 / −2,6 dB |
| Son système seul (18-47-43) | `loop[Casque (Synaptics HD Audio)] 48000 Hz, Stereo`, `micro=false` | 11,61 / 11,65 s | 0 | −31,0 / −9,2 dB |

Durées cohérentes avec l'intervalle start → stop du journal (8 s et 12 s, moins le délai de démarrage)
→ ni accéléré, ni haché, ni muet. Déjà couverts par ailleurs : micro interne + loopback (T2),
micro C270 + loopback (T2/T3, ronflement 50 Hz propre au C270), écouteurs filaires (sortie).
Non faits : casque Bluetooth, sortie à une autre fréquence que 48 kHz, son 5.1.

## Répartition prévue

- Automatisables par l'agent : 1 (partie journal), 13 (instance unique), 14 (script), 10 (partiel),
  6 (raccourcis via SendKeys, partiel), 2/4 (vérifs ffprobe : durée, canaux audio).
- Manuels (utilisateur) : 1 (affichage, VU-mètre), 2 (curseur, écoute), 3 (synchro YouTube),
  5 (webcam), 7 (Ctrl+Alt+Suppr / UAC), 8, 9 (2 écrans), 11–12 (téléphone + APK), 15 (matériel).
