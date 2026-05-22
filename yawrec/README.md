# YawREC — squelette Tauri 2

Mini enregistreur d'écran cross-plateforme. Cœur Rust, UI WebView2 (HTML/CSS/JS).
Ce squelette cible Windows en priorité ; macOS/Linux suivront en réutilisant le même socle.

## Architecture

```
yawrec/
├── index.html             ← entrée frontend
├── src/                   ← UI vanilla JS + CSS (design YawREC Windows)
│   ├── main.js
│   ├── recorder.js
│   └── style.css
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/
│   └── src/
│       ├── main.rs
│       ├── lib.rs         ← Builder, plugins, probe encoder au démarrage, tick loop
│       ├── commands.rs    ← #[tauri::command] + worker thread écran
│       ├── state.rs       ← état partagé (phase, timer, atomics, worker)
│       ├── error.rs
│       ├── capture/
│       │   ├── mod.rs     ← trait Capturer + Frame
│       │   ├── screen.rs  ← ✅ Windows.Graphics.Capture (D1+D2)
│       │   ├── audio.rs   ← cpal (stubs)
│       │   └── webcam.rs  ← nokhwa (stubs)
│       └── encoder/
│           └── mod.rs     ← ✅ FFmpeg wrapper (E1) + détection HW (E2)
└── package.json
```

## Prérequis Windows

| Outil | Version | Vérif |
|---|---|---|
| Rust | stable, toolchain `x86_64-pc-windows-msvc` | `rustc -V` |
| Node.js | LTS ≥ 20 | `node -v` |
| Visual Studio Build Tools | 2022, workload "Desktop development with C++" | `cl` |
| WebView2 Runtime | préinstallé sur Win 11 | — |
| Tauri CLI | ^2.0 | `cargo install tauri-cli --version "^2.0"` |
| FFmpeg dev libs | 7.x | voir ci-dessous |

### Installer FFmpeg dev libs sur Windows

1. Télécharger un build **shared + dev** depuis https://www.gyan.dev/ffmpeg/builds/
   (`ffmpeg-release-full-shared.7z`).
2. Extraire vers `C:\ffmpeg` (doit contenir `include/`, `lib/`, `bin/`).
3. PowerShell admin :
   ```powershell
   setx FFMPEG_DIR "C:\ffmpeg"
   setx PATH "$env:PATH;C:\ffmpeg\bin"
   ```
4. Fermer/rouvrir le terminal pour prendre en compte les variables.
5. Vérifier : `ffmpeg -version` (doit lister `--enable-libx264` au minimum,
   et idéalement `--enable-nvenc`, `--enable-libmfx`, `--enable-amf`).

## Installation et lancement

```powershell
npm install
cargo tauri dev
cargo tauri build
```

Sortie release : `src-tauri/target/release/bundle/{nsis,msi}/`.

## État du squelette

### ✅ Branché et fonctionnel

- Fenêtre 800×240 Mica acrylique avec drag region
- Boutons système (réduire / agrandir / fermer)
- **D1** Énumération écrans (Monitor::enumerate)
- **D2** Capture frames via Windows.Graphics.Capture (BGRA8)
- **D3** Audio cpal — micro + loopback système (Windows), conversion vers
  48 kHz stéréo f32, mixage par somme avec écrêtage, worker dédié
- **D4** Webcam via `nokhwa` (MediaFoundation sur Windows) — énumération
  + worker dédié qui capture en RGB, convertit/redimensionne en BGRA 400×225,
  publie dans un buffer partagé. OFF par défaut (privacy first), toggle via
  bouton webcam dans l'UI ou commande `set_webcam_enabled`.
- **D5** Compositing PiP — la frame webcam est blittée en bas-droite de
  chaque frame écran (marge 24px) avec une bordure blanche 2px,
  avant push à l'encoder. Pas d'alpha blend (overwrite), ~22 MB/s de coût.
- **E1** Pipeline FFmpeg : sws_scaler BGRA→YUV420P, mux MP4, flush propre
- **E2** Détection automatique de l'encodeur matériel : NVENC → QuickSync
  → AMF → x264. Résultat caché en OnceLock. Override : `YAWREC_FORCE_ENCODER=libx264`.
- **E3 bis** Stream audio AAC (48 kHz stéréo, 128 kbps, FLTP planar) dans
  le même MP4 ; framing AAC 1024 samples géré côté encoder
- **F1** Raccourci global `Ctrl + Shift + R` — toggle recording
  (start ou stop selon la phase) même quand la fenêtre n'a pas le focus
- **F2** Dialog dossier de sortie (plugin-dialog)
- **F3** Notifications système au start (« Enregistrement démarré ») et au
  stop (« Fichier : <chemin> ») via tauri-plugin-notification
- **F4** Tray icon avec menu : Afficher YawREC / Démarrer-Arrêter / Quitter
  + clic gauche sur l'icône qui ramène la fenêtre devant
- **G1** Build & packaging :
  - `scripts/build.ps1` — pré-flight (Rust/Node/FFMPEG_DIR/icônes) + cargo tauri build
  - `scripts/make-icons.ps1` — SVG source → toutes les tailles via cargo tauri icon
  - `scripts/sign.ps1` — signing self-signed / thumbprint / PFX
  - `.github/workflows/release.yml` — CI Windows automatique sur tag v*
  - Installateurs MSI (WiX) + NSIS produits par défaut, langues fr-FR + en-US
- Architecture : encoder partagé dans `Arc<Mutex<Option<Encoder>>>`,
  trois workers (`yawrec-video-worker`, `yawrec-audio-worker`,
  `yawrec-webcam-worker`) coordonnés via Arc<AtomicBool> stop_flag
- Helpers `do_start_recording` / `do_stop_recording` / `do_toggle_recording`
  partagés entre les commandes IPC, le raccourci F1 et le menu tray F4
- Stop graceful complet (stop_flag + CaptureControl::stop + JoinHandle::join via spawn_blocking)
- Tick loop 250 ms qui émet `recorder://tick`

### 🚧 À faire

- Dropdowns dans l'UI pour choisir le device audio / la webcam / l'écran
  (les énumérations Rust sont déjà là, il manque juste l'UI de sélection)
- Acheter un cert CodeSigning EV pour distribuer sans warning SmartScreen

---

## Build & distribution (G1)

### Build local rapide

```powershell
# Une seule fois : générer les icônes
.\scripts\make-icons.ps1

# Build release complet (~3-5 min en cold cache)
.\scripts\build.ps1
```

Produit :
- `src-tauri\target\release\yawrec.exe` — binaire portable
- `src-tauri\target\release\bundle\msi\YawREC_0.1.0_x64_en-US.msi` — installeur Windows Installer
- `src-tauri\target\release\bundle\nsis\YawREC_0.1.0_x64-setup.exe` — installeur NSIS

### Signing

**Mode dev (self-signed, pour tests internes) :**
```powershell
.\scripts\sign.ps1 -SelfSigned
```
Crée un certificat jetable `CN=YawREC Dev`, signe tout. Windows SmartScreen
affichera quand même un warning aux utilisateurs — c'est normal pour un cert
non vérifié par une autorité reconnue.

**Mode production (cert acheté) :**
```powershell
# Si ton cert est déjà dans Cert:\CurrentUser\My
.\scripts\sign.ps1 -Thumbprint "ABCD1234..."

# Sinon, depuis un fichier .pfx
$pw = Read-Host -AsSecureString "Mot de passe PFX"
.\scripts\sign.ps1 -PfxPath ".\cert.pfx" -PfxPassword $pw
```

Pour une distribution publique sans warning SmartScreen, il faut un cert
**CodeSigning EV (Extended Validation)** chez DigiCert, Sectigo, ou
SSL.com — comptes ~250–400€/an. Les certs OV (Organization Validation)
standards continuent d'afficher le warning au début (réputation construite
au fur et à mesure que des utilisateurs cliquent « Exécuter quand même »).

### CI automatique sur GitHub

`.github/workflows/release.yml` se déclenche sur push de tag `v*.*.*`.
Il installe FFmpeg dev libs depuis GyanD/codexffmpeg, build, signe
si les secrets `WINDOWS_PFX_BASE64` et `WINDOWS_PFX_PASSWORD` sont
définis, et crée une release draft GitHub avec les artefacts.

Pour ajouter le PFX en secret :
```powershell
$pfx_b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))
$pfx_b64 | clip
# Puis : GitHub → Settings → Secrets → New : WINDOWS_PFX_BASE64
# Et : WINDOWS_PFX_PASSWORD = mot de passe du PFX
```

### Publier une release

```powershell
# Bumper la version dans :
#   - src-tauri/tauri.conf.json (version)
#   - src-tauri/Cargo.toml      (version)
#   - package.json              (version)

git commit -am "v0.2.0"
git tag v0.2.0
git push --tags
# → la CI prend le relais, draft release apparaît sur GitHub
```

## E2 — Hardware acceleration en détail

### Comment savoir quel encodeur a été choisi

Au lancement, la console Rust affiche :

```
INFO  yawrec_lib::encoder  Détection encodeur vidéo…
INFO  yawrec_lib::encoder  probe[h264_nvenc] : OK
INFO  yawrec_lib::encoder  Encodeur sélectionné : h264_nvenc (matériel)
INFO  yawrec_lib                                Encodeur prêt : h264_nvenc
```

Si tu n'as ni NVIDIA, ni Intel iGPU, ni AMD reconnu :

```
DEBUG yawrec_lib::encoder  probe[h264_nvenc] : codec absent du build FFmpeg
DEBUG yawrec_lib::encoder  probe[h264_qsv]   : codec absent du build FFmpeg
DEBUG yawrec_lib::encoder  probe[h264_amf]   : codec absent du build FFmpeg
INFO  yawrec_lib::encoder  probe[libx264]    : OK
INFO  yawrec_lib::encoder  Encodeur sélectionné : libx264 (logiciel)
```

Si un HW encoder est présent dans le build mais que le matériel ne suit pas
(ex. h264_nvenc compilé mais pas de carte NVIDIA), le probe descend au suivant
avec un warning :

```
WARN  yawrec_lib::encoder  probe[h264_nvenc] : open failed : Cannot load nvcuda.dll
INFO  yawrec_lib::encoder  probe[h264_qsv]   : OK
```

### Forcer un encodeur (debug)

```powershell
$env:YAWREC_FORCE_ENCODER="libx264"
cargo tauri dev
```

Valeurs acceptées : `h264_nvenc`, `h264_qsv`, `h264_amf`, `libx264`.

### Gain attendu

Sur un i7 + RTX 3060 à 1080p30 :

| Encodeur | CPU vidéo | Latence frame | Notes |
|---|---:|---:|---|
| `libx264 veryfast` | ~25-35 % | ~10 ms | Référence software |
| `h264_nvenc p4` | ~3-5 % | ~2 ms | Quasi gratuit |
| `h264_qsv veryfast` | ~5-8 % | ~3 ms | Bon si pas de NVIDIA |
| `h264_amf speed` | ~4-6 % | ~3 ms | GPU Radeon récents |

Le bitrate reste identique (8 Mbps par défaut) ; la qualité visuelle est
comparable, avec un léger avantage à x264 sur les scènes complexes mais un
avantage HW sur les bitrates élevés.

### Pourquoi probe à `640×360`

Le probe ouvre un encoder de test à basse résolution pour :
1. **Trier vite** : 640×360 alloue peu de mémoire GPU, ouverture en ~10 ms.
2. **Détecter le vrai HW** : NVENC/QSV/AMF échouent à `open()` si le driver
   ou le GPU n'est pas là, même si le codec est compilé dans FFmpeg.
3. **Caché** : `OnceLock` garantit que le probe ne tourne qu'une fois par
   process, peu importe combien d'enregistrements tu fais.

## Test end-to-end

```powershell
cargo tauri dev
```

1. Console terminal → cherche la ligne `Encodeur sélectionné : <nom>`.
2. F12 dans la WebView → `await __TAURI__.core.invoke("list_screens")`.
3. Choisir un dossier de sortie en cliquant sur le chemin du footer.
4. Cliquer sur le bouton REC central, attendre quelques secondes, recliquer.
5. Ouvrir le MP4 dans VLC pour vérifier.

Si l'encoder HW est utilisé, le Task Manager doit montrer une activité GPU
"Video Encode" sur l'onglet Performance pendant l'enregistrement.

## Roadmap actualisée

| # | Phase | État |
|---|---|---|
| B-C | Projet Tauri + IPC | ✅ |
| D1-D2 | Capture écran Windows | ✅ |
| D3 | Audio cpal | 🚧 énumération faite |
| D4 | Webcam nokhwa | 🚧 stub |
| D5 | Compositing PiP | 🚧 |
| **E1** | **FFmpeg wrapper + mux MP4** | ✅ |
| **E2** | **HW accel NVENC/QSV/AMF** | ✅ |
| E3 bis | Stream audio AAC | 🚧 attend D3 |
| F1-F4 | Raccourci/dialog/notif/tray | 🚧 |
| G1-G3 | Build & sign | ⏳ |

## Licence

GPL-3.0 (à confirmer).
