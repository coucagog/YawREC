# YawREC — Rapport d'améliorations

**Date** : 2026-05-22  
**Version** : 0.1.0  
**Plateforme** : Windows 10 Pro x64  
**Build** : Release (cargo tauri build)

---

## Résumé

Suite à la demande de rendre fonctionnels tous les contrôles UI et d'améliorer la qualité A/V, 10 modifications ont été apportées à l'application. Tous les tests automatisés passent (10/10).

---

## Modifications apportées

### Audio

#### A1 — Synchronisation du mixer (bruit corrigé)
**Problème** : `pull_chunk()` drainait le buffer mic OU loopback indépendamment, selon lequel se remplissait en premier. Résultat : des chunks mono-source alternaient, produisant du bruit et une perte de signal.  
**Correction** : Vérification que les deux buffers actifs ont au moins `CHUNK_SAMPLES` avant de drainer l'un ou l'autre. Si l'un est en retard, on attend.  
**Fichier** : `src-tauri/src/capture/audio.rs` — `pull_chunk()`

#### A2 — Loopback désactivé par défaut
**Problème** : Le loopback (son système) était activé par défaut. Quand activé seul (sans mic actif), il mixait du silence au signal mic, réduisant de moitié le volume effectif.  
**Correction** : `loopback_enabled: false` dans `RecorderState::Default`.  
**Fichier** : `src-tauri/src/state.rs`

---

### Vidéo

#### V1 — Suppression de `tune=zerolatency` (qualité améliorée)
**Problème** : L'option x264 `zerolatency` est conçue pour le streaming live (supprime le B-frame lookahead). Elle dégrade la qualité dans un contexte d'enregistrement fichier.  
**Correction** : Option supprimée des paramètres x264.  
**Fichier** : `src-tauri/src/encoder/mod.rs` — `VideoEncoder::encoder_options()`

#### V2 — Correction de la dérive A/V (CFR → VFR)
**Problème** : L'encodeur utilisait `frame_index` comme PTS avec une base de temps `Rational(1, 30)`. Si la capture réelle tournait à ~27-28 fps (DXGI timeout 100 ms), la vidéo était écourtée (~9.2 s pour 10 s enregistrées) tandis que l'audio restait exact → dérive de 0.687 s.  
**Correction** : PTS vidéo calculé depuis le vrai timestamp de la frame (`frame.timestamp.as_millis()`) avec base de temps `Rational(1, 1000)` (millisecondes). L'encodeur devient VFR, aligné sur le flux audio.  
**Fichiers** : `src-tauri/src/encoder/mod.rs` — `push_video_frame()`, `try_open_video()`, `add_stream_with_params()`

#### V3 — Correction des timestamps pendant pause/reprise
**Problème** : Après la correction VFR, la pause révélait un nouveau bug : `frame.timestamp` est mesuré depuis le démarrage du capturer (horloge murale continue). Pendant une pause de 4 s, le capteur continuait de livrer des frames avec des timestamps incluant le temps de pause → vidéo de 14 s pour 10 s effectives, et dérive A/V égale à la durée de pause (4.026 s).  
**Correction** :
  1. Le worker vidéo **ignore** les frames arrivées pendant la pause (`pause_flag`).
  2. À chaque reprise, la durée de pause est accumulée dans `paused_total_ms` (Arc<AtomicU64>).
  3. Le PTS effectif = `raw_timestamp_ms − paused_total_ms`, garantissant des timestamps continus.  
**Fichiers** : `src-tauri/src/state.rs`, `src-tauri/src/commands.rs` — `VideoWorkerCtx`, `run_video_worker()`, `do_pause_recording()`

---

### UI / Contrôles

#### U1 — Bouton Pause fonctionnel
**Ajout** : Bouton ⏸/⏵ visible pendant l'enregistrement, raccourci global `Ctrl+Shift+P`.  
**Comportement** : Bascule entre `Recording` et `Paused` — audio coupé, vidéo figée (frames ignorées).  
**Fichiers** : `index.html`, `src/style.css`, `src/main.js`, `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`

#### U2 — Compteur de frames dans le footer
**Ajout** : Span `frame-count-item` visible uniquement pendant enregistrement/pause, affichant le nombre de frames capturées en temps réel.  
**Fichiers** : `index.html`, `src/main.js`, `src/recorder.js`

#### U3 — Popovers Audio, Webcam, Écran
**Ajout** : Trois popovers overlay dans `.win-body` :
- **Audio** : toggle mic/loopback, sélection du périphérique d'entrée
- **Webcam** : toggle ON/OFF, sélection de la caméra
- **Écran** : sélecteur d'écran (auto si écran unique)

Fermeture au clic en dehors. Anchors correctement gérés pour éviter double-toggle.  
**Fichiers** : `index.html`, `src/style.css`, `src/main.js`

#### U4 — Dossier de sortie configurable
**Ajout** : Click sur le dossier ouvre un sélecteur natif (Tauri dialog). Valeur initiale récupérée depuis le backend au démarrage.  
**Fichiers** : `src/main.js`, `src/recorder.js`, `src-tauri/src/commands.rs`

#### U5 — Modes non implémentés grisés
**Ajout** : Boutons `window` et `region` dans le segmented control sont grisés (`opacity: 0.28`, `pointer-events: none`) avec tooltip explicatif.  
**Fichier** : `src/main.js` — `init()`

---

## Résultats des tests

| Test | Résultat | Détail |
|------|----------|--------|
| App démarre | ✓ | PID stable, pas de crash |
| MP4 produit (10 s) | ✓ | `YawREC-*.mp4` dans `Videos\YawREC` |
| Codec vidéo H264 | ✓ | h264 confirmé par ffprobe |
| Codec audio AAC | ✓ | aac confirmé par ffprobe |
| Durée ~10 s | ✓ | 9.967 s (tolérance ±2 s) |
| Sync A/V enregistrement simple | ✓ | Écart **0.055 s** (seuil 0.5 s) |
| Second MP4 après pause/reprise | ✓ | Nouveau fichier distinct |
| Durée effective ~10 s (5 s + pause + 5 s) | ✓ | 10.011 s |
| Sync A/V pause/reprise | ✓ | Écart **0.021 s** |
| App fermée proprement | ✓ | Process terminé |

**Score global : 10 / 10**

---

## Métriques avant / après

| Métrique | Avant | Après |
|----------|-------|-------|
| Dérive A/V (enregistrement simple) | 0.687 s ✗ | 0.055 s ✓ |
| Dérive A/V (pause/reprise 4 s) | 4.026 s ✗ | 0.021 s ✓ |
| Durée vidéo avec pause | 14.04 s ✗ | 10.01 s ✓ |
| Qualité audio (bruit) | Bruit / signal inaudible | Signal mic clair ✓ |

---

## Artefacts

```
target\release\yawrec.exe                              — 6.3 MB
target\release\bundle\nsis\YawREC_0.1.0_x64-setup.exe — 63.9 MB
target\release\bundle\msi\YawREC_0.1.0_x64_en-US.msi  — 94.3 MB
target\release\bundle\msi\YawREC_0.1.0_x64_fr-FR.msi  — 94.3 MB
```

---

## Points restants (hors scope de cette session)

- **Mode fenêtre** (`window`) : capture d'une fenêtre spécifique — non implémenté (bouton grisé).
- **Mode région** (`region`) : capture d'une zone arbitraire — non implémenté (bouton grisé).
- **Signature des installeurs** : le script `scripts/sign.ps1` est disponible pour une signature code optionnelle.
