# Résultats `TESTS_WINDOWS.md` — branche `fix/revue-complete` @ f369546

**Version testée** : `f369546712622362d704481951f363f0763596e8`, **compilée depuis les sources**
(`git log -1 --format=%h` → `f369546`) avec `yawrec\scripts\build.ps1 -SkipNpm`, et non avec l'artifact
CI (le zip reçu était l'archive des sources). Exe + DLL FFmpeg 8.1.2 copiés dans
`C:\Users\hp\yawrec-test\`. Tests du 24/10/2026, 15:30 → 18:50 (heure UTC des journaux).

## Configuration testée

- Windows : Windows 11 Pro 10.0.22631 (23H2)
- Carte graphique : Intel intégrée (CPU Core i5-7500T, 4 cœurs / 4 threads) ; pas de NVIDIA ni d'AMD
- Écrans : 1 écran, 1920×1080
- Encodeur indiqué dans le journal : `Encodeur prêt : libx264` (NVENC : `nvcuda.dll` absent ; QSV refuse
  yuv420p ; AMF absent)
- Audio : micro interne « Microphone externe (Synaptics HD Audio) », webcam Logitech C270 (vidéo + micro),
  écouteurs filaires (sortie « Casque (Synaptics HD Audio) », 48 kHz)

## Tests

| # | Test | Résultat | Commentaire |
|---|---|---|---|
| 1 | Lancement | **OK** | Interface normale, pas de page blanche. VU-mètre (3 barres à côté du micro) : bouge à la voix. Journal : `Encodeur prêt : libx264`. Remarque : l'échelle linéaire rend le VU peu réactif avec un micro faible (passer en dB). |
| 2 | Enregistrement simple | **OK** | Curseur visible et bien placé. Son système **stéréo conservé** (test contrôlé : bip 440 Hz à gauche seule → G 390 / D 51, bip 880 Hz à droite seule → D 439 / G 55). Micro et son système à volume normal. Remarques : écho d'environ 79 ms = fuite des écouteurs recaptée par le micro (pas d'annulation d'écho) ; crêtes proches de 0 dBFS quand micro et son système sont forts en même temps (+1,5 dB mesuré) → limiteur conseillé. |
| 3 | Synchro | **KO (critère ±1 image)** | **Aucune dérive** (≈ −130 ms au début, ≈ −100 ms à la fin sur 43 s), mais un décalage **constant d'environ −115 ms** (le bip arrive ~3,5 images avant le flash). Non perçu par l'utilisateur. Hypothèse : le loopback capte le son avant la latence de sortie, alors que le navigateur synchronise ce qu'on entend. Les gels décalent ponctuellement un flash (−185 ms pendant un gel de 235 ms). |
| 4 | Pause | **OK** | Bouton de l'interface : 9,5 s retirées, MP4 de 20,3 s, rien de la pause (ni image ni voix), aucun trou au point de reprise. Raccourci `Ctrl+Shift+F10` : OK (test 14). |
| 5 | Écran statique + webcam | **KO** | Durée ≈ 20 s ✓, webcam animée sur un bureau figé ✓, taille ≈ 1/5 de la largeur ✓ (384×216 sur 1920), 4 coins fonctionnels ✓. **Tête aplatie** : la webcam en 4:3 (`Webcam · ouverte (1280×960) → PiP 384×216`) est étirée dans un cadre 16:9. |
| 6 | Raccourcis | **KO partiel** | `Ctrl+Shift+F9` avec l'app réduite ✓, démarrage/arrêt depuis l'icône de la barre des tâches ✓. **Aucune notification affichée** ; aucune erreur dans le journal (`show()` réussit). Exe portable sans raccourci dans le menu Démarrer → Windows ignore probablement les notifications d'une app non installée (pas d'AppUserModelID). Non vérifié avec l'installeur. |
| 7 | Coupure d'accès à l'écran | **KO** | `Ctrl+Alt+Suppr` : OK (`DXGI · accès perdu`, puis `Accès refusé (0x80070005)` pendant l'écran sécurisé, puis reprise). **Invite UAC** : boucle de **1 627** `accès perdu` / `duplication récupérée après ~1 ms` en 6 s (jusqu'à 570/s), **vidéo noire de l'invite jusqu'à la fin**, sans reprise après sa fermeture. L'enregistrement, lui, continue. |
| 8 | Quitter en plein enregistrement | **OK** | Croix de la fenêtre : l'app se ferme, le MP4 se lit (fin vidéo 0,6 s avant la fin audio). *Quitter* de l'icône : fermeture propre en ~1 s, MP4 lisible, fins alignées (12,33 / 12,33 s). |
| 9 | Fenêtre et région (2 écrans) | **Non testé** | Un seul écran. Le mode région, utilisé sur un écran, a produit des MP4 corrects. |
| 10 | Dossier de sortie | **OK** | `set_output_directory → C:\Users\hp\Videos\yc` restauré au relancement, MP4 écrit dans ce dossier. |
| 11 | Téléphone | **Non testé** | Pas d'APK sur le téléphone ; pas de SDK Android ni de Java sur la machine pour le compiler. |
| 12 | Sécurité du téléphone | **Non testé** | Idem 11. |
| 13 | Instance unique | **OK** | 2e double-clic → la fenêtre existante revient, un seul `yawrec.exe`. Le journal de la 1re instance est conservé (0 octet nul ; corrige le défaut vu sur fe313a6). |
| 14 | Script automatique | **KO 12/14** | `scripts\test_audio.ps1 -ExePath C:\Users\hp\yawrec-test\yawrec.exe`, 3 passages. Seul échec stable : **« Pas de gel d'image »** (gels de 1,0 à 1,5 s, 21 à 25 fps en moyenne). Synchro, fins A/V et pause OK. Le 1er passage (5/14) a été ralenti par le premier lancement de l'exe neuf. Les exclusions Defender (dossier de sortie + exe) ne changent rien. |
| 15 | Matériel varié | **OK (partiel)** | Micro seul et son système seul : ni accéléré, ni haché, ni muet (durées conformes, 0 trou audio) ; confirmé à l'écoute. Micro C270 : **ronflement 50/100 Hz** (≈ 70 % de l'énergie), venant du matériel ; un filtre passe-haut sur la voie micro aiderait. Non testés : casque Bluetooth, sortie à une autre fréquence que 48 kHz, 5.1. |

## Défaut principal transversal : gels d'image (tests 3, 5, 7, 14)

Les gels apparaissent dans **tous** les enregistrements : 2 à 8 par enregistrement, de 0,2 à 1,8 s, surtout
dans les premières secondes. L'instrumentation de `c4e5e86` les attribue toujours à **un seul appel
d'encodage** :
```
Vidéo · gel de 1350 ms à t=7064 ms : encodage frame précédente 1343 ms, puis capture 7 ms, verrou encodeur 11 ms
Vidéo · 212 frames, encodage moyen 23.7 ms (max 1343 ms), 3 gel(s) > 200 ms
```
Le CPU total reste entre 4 et 48 % pendant ces gels : l'appel **attend**, il ne calcule pas. Defender est
écarté : c'est identique avec le dossier de sortie et l'exe exclus.
Pistes : chronométrer séparément `send_frame`, `receive_packet` et `write_interleaved` ; essayer
`tune=zerolatency` (ou `rc-lookahead=0`, `bframes=0`, `sliced-threads`) quand il n'y a pas d'encodeur
matériel ; créer l'encodeur au démarrage plutôt qu'à la première image.

## Sorties ffprobe

```
# T14 passage 2 (f369546)
YawREC_2026-10-24_15-33-53.mp4 : ✓ Fins A/V alignées (écart 0.003s) ; ✗ gel de 1350 ms à 5.714s, 21.2 fps moyen
YawREC_2026-10-24_15-34-12.mp4 : ✓ Fins A/V alignées (écart 0.15s) ; ✗ gel de 1049 ms à 1.272s, 24.9 fps moyen

# T5 écran statique + webcam
YawREC_2026-10-24_17-02-47.mp4 (1920×1080) : vidéo 0,03 → 19,04 s, 446 img ; audio → 19,75 s
YawREC_2026-10-24_17-03-35.mp4 (1920×1080) : vidéo 0,05 → 21,32 s, 562 img ; audio → 21,29 s

# T3 synchro
YawREC_2026-10-24_17-37-15.mp4
stream|codec_name=h264|duration=42.938000|nb_frames=1120
stream|codec_name=aac|duration=42.986000|nb_frames=2016

# T7 UAC (luminosité moyenne, une valeur par seconde : 38 de 1 à 28 s, puis 0 de 29 à 34 s)
YawREC_2026-10-24_17-54-27.mp4
stream|codec_name=h264|width=1920|height=1080|duration=34.734000|nb_frames=710
stream|codec_name=aac|duration=34.794000|nb_frames=1632

# T4 pause (bouton)
YawREC_2026-10-24_17-44-52.mp4 : vidéo 0,06 → 20,37 s (523 img) ; audio −0,02 → 20,33 s
```

## Journaux joints (`yawrec.log` copiés après chaque session)

| Fichier | Session |
|---|---|
| `yawrec-T14.log`, `yawrec-T14-run2.log`, `yawrec-T14-run3-exclusion.log` | **KO 14** (gels), passages 1 à 3 |
| `yawrec-T5.log`, `yawrec-T1-T5-user.log` | **KO 5** (webcam) |
| `yawrec-T7.log` | **KO 7** (UAC) |
| `yawrec-T6.log` | **KO 6** (notifications) |
| `yawrec-T3.log` | **KO 3** (décalage constant) |
| `yawrec-T2.log`, `yawrec-T4.log`, `yawrec-T8.log`, `yawrec-T8-quitter.log`, `yawrec-T10.log`, `yawrec-T15.log` | tests OK |

## Autres remarques

- `build.ps1` corrigé : il fonctionne sous PowerShell 5.1 sans `cargo tauri` (repli sur `npx tauri`) et
  vérifie que FFmpeg est en 7.x ou 8.x.
- Le journal n'est (ré)initialisé qu'environ 8 s après le premier lancement d'un exe neuf (analyse
  Defender probable) : jusque-là, `yawrec.log` montre encore la session précédente.
- Le message `[RDP/VM non supporté ?]` affiché sur `Accès refusé (0x80070005)` est trompeur : on le voit
  aussi sur l'écran sécurisé de `Ctrl+Alt+Suppr`.
