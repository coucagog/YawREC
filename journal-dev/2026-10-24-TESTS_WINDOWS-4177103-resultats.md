# Résultats `TESTS_WINDOWS.md` — branche `fix/revue-complete` @ 4177103

**Version testée** : `417710349f553769bb8a01e618498672a6241742`, **compilée depuis les sources**
(`git log -1 --format=%h` → `4177103`) avec `yawrec\scripts\build.ps1 -SkipNpm`. Exe et DLL FFmpeg 8.1.2
sont copiés dans `C:\Users\hp\yawrec-test\`. L'APK `yawrec-android-debug-4177103…` vient du run CI
40123957437. Tests du 24/10/2026, de 20:00 à 23:26 (heure UTC des journaux).

Cette campagne refait les tests demandés : 14, 7, 5, 3, puis 11 et 12. Les autres tests gardent leur
résultat sur f369546 (voir `2026-10-24-TESTS_WINDOWS-f369546-resultats.md`).

## Configuration testée

Identique à f369546 :
- Windows 11 Pro 10.0.22631, Intel Core i5-7500T, 1 écran 1920×1080.
- `Encodeur prêt : libx264`.
- Audio : micro interne Synaptics, webcam Logitech C270, écouteurs filaires (sortie « Casque (Synaptics HD Audio) », 48 kHz).
- Téléphone Android sur le même Wi-Fi, IP 10.120.214.122.

## Tests

| # | Test | Résultat | Commentaire |
|---|---|---|---|
| 1 | Lancement | OK (f369546) | Non refait. |
| 2 | Enregistrement simple | OK (f369546) | Non refait. Remarques de f369546 toujours valables : écho par fuite des écouteurs, pas de limiteur. |
| 3 | Synchro | **OK avec Décalage = 50 ms** | Mesures sur un **fichier de référence** (flash + bip alignés à +8 ms, toutes les 2 s), lu en plein écran dans le navigateur. **0 ms : −51 ms puis −42 ms** (2 passes), sans dérive. **50 ms : −4 ms** (de −5 ms au début à +8 ms à la fin) → ±1 image ✓. Le journal indique bien `Audio sources : … (retard N ms)`. Les vidéos YouTube « audio video sync test » ne servent **pas** de référence : avec https://youtu.be/ucZl6vQ_8Uo on mesure −339 ms à 0 ms, contre −115 ms avec la vidéo utilisée sur f369546. |
| 4 | Pause | OK (f369546) | Non refait localement. Pause et reprise **depuis le téléphone** OK (test 11). |
| 5 | Écran statique + webcam | **OK** | 4 coins, **plus de déformation**. 20,0 s pour 20 s, 0 gel de capture. `Webcam · ouverte (640x360@1FPS, NV12 Format) → PiP 384×216` et `Webcam · 1re image décodée : 640×360`. **Le libellé `@1FPS` est trompeur** : on mesure environ 16 changements d'image par seconde dans la vignette. |
| 6 | Raccourcis | OK (f369546) | Non refait. Absence de notifications désormais documentée comme attendue avec l'exe portable. |
| 7 | Coupure d'accès à l'écran | **OK** | Invite UAC et écran sécurisé : `DXGI · accès perdu`, puis 15 lignes `bureau sécurisé actif, attente : Accès refusé (0x80070005)` en 3 s, puis `DXGI · duplication récupérée après 3.16s`. **Aucune image noire** (luminosité stable de 39 à 41) et l'image repart. Ce sont 15 lignes au lieu des 1 627 de f369546. |
| 8 | Quitter en plein enregistrement | OK (f369546) | Non refait. |
| 9 | 2 écrans | Non testé | Un seul écran. |
| 10 | Dossier de sortie | OK (f369546) | Non refait. |
| 11 | Téléphone | **OK** | Connexion par QR code et par adresse IP ✓. Démarrage, pause, reprise et arrêt à distance ✓ (MP4 de 39,0 s pour 48 s moins 7,4 s de pause, aucun trou). Après une coupure Wi-Fi de 10 s, le téléphone garde le contrôle ✓. Le PC n'a journalisé aucune déconnexion (la connexion TCP a tenu), mais 5 déconnexions/ré-authentifications antérieures ont réussi. **Caméra** : en télécommande, la webcam est celle réglée sur le PC (le protocole ne connaît que `start\|stop\|pause\|resume\|status`). En enregistrement sur le téléphone, la tuile « Caméra » ne fait rien (`RecordingService.handleToggleCamera` : `// Camera PiP compositing: future sprint`) et il n'y a aucun choix de caméra. |
| 12 | Sécurité du téléphone | **OK** | Il suffit de changer la dernière lettre du code pour bloquer l'accès : `handshake refusé : HTTP error: 401 Unauthorized`, **une seule fois, sans réessai en boucle**. Le libellé du message affiché sur le téléphone n'a pas été relevé. |
| 13 | Instance unique | OK (f369546) | Non refait. |
| 14 | Script automatique | **KO 13/14** | 2e passage : seul échec « Pas de gel d'image (MP4 #2, pause) », un gel de 352 ms à 0,03 s. MP4 #1 : max 78 ms, 29,5 fps ✓. Le 1er passage (5/12) n'est pas exploitable : l'exe neuf a réagi avec 6 s de retard au premier lancement, comme sur f369546. Lignes demandées : voir plus bas. |
| 15 | Matériel varié | OK partiel (f369546) | Non refait. |

## Test 14 — lignes du journal

```
MP4 #1  Vidéo · encodage : 255 frames, moyenne 9.5 ms (max 1639 ms), 1 appel(s) > 200 ms
        Vidéo · capture : 255 frames, conversion moyenne 8.3 ms, 0 gel(s) > 200 ms, 0 frame(s) abandonnée(s) (file pleine)
MP4 #2  Vidéo · encodage : 283 frames, moyenne 13.8 ms (max 1538 ms), 2 appel(s) > 200 ms
        Vidéo · capture : 283 frames, conversion moyenne 9.1 ms, 1 gel(s) > 200 ms, 0 frame(s) abandonnée(s) (file pleine)
        Vidéo · gel de capture de 352 ms à t=384 ms (capture 6 ms, conversion 8 ms)
Encodeur · appel lent de 1639 ms pour t=2262 ms (verrou 0 ms, send_frame 2 ms, écriture 1637 ms)
Encodeur · appel lent de 1439 ms pour t=1065 ms (verrou 0 ms, send_frame 2 ms, écriture 1437 ms)
Encodeur · appel lent de 1538 ms pour t=3130 ms (verrou 0 ms, send_frame 3 ms, écriture 1535 ms)
```
- Le thread d'encodage dédié corrige le défaut visible : plus de gels de 1 à 1,5 s dans la vidéo, et
  aucune frame abandonnée.
- Les appels lents existent encore, et leur temps est **entièrement dans « écriture »**
  (`receive_packet`/`write_interleaved`), pas dans `send_frame`. Ils surviennent dans les 3 premières
  secondes. Piste : écriture disque ou antivirus sur le MP4 en cours d'écriture.
- Gel restant : 352 ms côté capture à t=384 ms, au tout début. Capture et conversion sont normales,
  donc l'attente se situe ailleurs dans la boucle (1re image, démarrage).

## Autres constats

1. **Départ de la piste audio retardé** dans certains MP4 : `start_time` audio à **1,118 s** (23-07-25)
   et à 0,308 s (21-58-12), contre 0 ailleurs. La synchro reste correcte (les pts sont respectés), mais
   le début du MP4 est muet. Probablement parce que le loopback ne livre rien tant que rien ne joue.
2. **Les gels « attendus » sont comptés comme gels** : les 15 attentes de 200 ms pendant l'écran
   sécurisé (test 7) apparaissent dans `15 gel(s) > 200 ms`.
3. **Libellé `@1FPS`** de la webcam : la vignette est pourtant animée à environ 16 images/s.
4. **Procédure du test 3** : remplacer la vidéo YouTube par un fichier de référence. Celui de ce test
   s'obtient avec :
   ```
   ffmpeg -f lavfi -i "color=c=black:s=1920x1080:r=30:d=62" -f lavfi -i "sine=frequency=1000:sample_rate=48000:duration=62" -filter_complex "[0:v]drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='lt(mod(t,2),0.1)'[v];[1:a]volume='if(lt(mod(t,2),0.1),0.5,0)':eval=frame,aformat=channel_layouts=stereo[a]" -map "[v]" -map "[a]" -c:v libx264 -preset veryfast -pix_fmt yuv420p -c:a aac -b:a 128k sync-reference.mp4
   ```
   Envisager 50 ms comme valeur par défaut du *Décalage*, ou un calibrage guidé.
5. **Application Android** : la tuile « Caméra » donne l'impression de fonctionner alors que la
   fonction n'est pas implémentée. La masquer ou l'indiquer « bientôt ».
6. **Build local** : `cl : error D8018 : impossible de créer le fichier réponse de l'éditeur de liens`
   dans le build script de `ffmpeg-sys-next` (reproductible). Contourné en plaçant `TMP`/`TEMP` dans
   `src-tauri\target\tmp` (exclu de Defender). `build.ps1` pourrait aussi définir lui-même `FFMPEG_DIR`
   s'il trouve `C:\ffmpeg`.

## Journaux joints

| Fichier | Session |
|---|---|
| `yawrec-T14.log`, `yawrec-T14-run2.log` | **KO 14** (passages 1 et 2) |
| `yawrec-T7.log` | 7 (UAC) |
| `yawrec-T5.log` | 5 (webcam) |
| `yawrec-T3.log`, `yawrec-T3-pass3.log`, `yawrec-T3-reference.log`, `yawrec-T3-50ms.log` | 3 (YouTube 0 et 120 ms, référence 0 ms, 50 ms) |
| `yawrec-T11.log`, `yawrec-T11-pause-wifi.log`, `yawrec-T12-T3.log` | 11 et 12 (téléphone) |
