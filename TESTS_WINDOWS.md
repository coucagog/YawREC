# Procédure de test Windows — branche `fix/revue-complete`

> **Fichier temporaire** : à supprimer une fois la campagne de test terminée.

## Récupérer l'exécutable

1. Ouvrir les runs CI de la branche (connecté à GitHub) :
   https://github.com/coucagog/YawREC/actions?query=branch%3Afix%2Frevue-complete
   → dernier run vert → section *Artifacts* → `yawrec-windows-portable-…` (≈ 70 Mo, conservé 14 jours).
2. Dézipper dans un dossier : l'exe et les DLL FFmpeg doivent rester ensemble.
   Prendre un dossier **hors de l'arborescence de build** (par exemple `C:\Users\<vous>\yawrec-test\`) :
   le test E2E du 24/10 a montré qu'un exe lancé depuis `src-tauri\target\release` ne pouvait
   ni écrire son journal ni créer sa WebView2 sur cette machine.
3. Lancer `YawREC.exe`.
   - Exe non signé, SmartScreen bloque : *Informations complémentaires → Exécuter quand même*.
   - Pare-feu : autoriser les **réseaux privés** (nécessaire pour le téléphone).

**Journal** : `%TEMP%\yawrec.log`. **Il est effacé à chaque lancement** : le copier avant de relancer l'app.
Relancer l'app pendant qu'elle tourne déjà n'efface plus le journal en cours.
En cas de gel d'image, les lignes `Vidéo · gel de capture …` et `Encodeur · appel lent …` détaillent où le temps est passé.

Sauf indication contraire, les vidéos vont dans `Vidéos\YawREC`.

## Configuration testée

- Windows (10 / 11, build) :
- Carte graphique :
- Écrans (nombre, résolution, mise à l'échelle 100 % / 150 %…) :
- Encodeur indiqué dans le journal (`Encodeur prêt : …`) :

## Tests

Pour chaque test, noter **OK / KO**, avec un commentaire si KO.

| # | Test | Attendu | Résultat |
|---|---|---|---|
| 1 | **Lancement.** Parler dans le micro. | L'interface s'affiche normalement, pas de page blanche (vérifie la CSP). Le VU-mètre bouge. Le journal contient `Encodeur prêt : h264_nvenc`, `h264_qsv`, `h264_amf` ou `libx264`. | |
| 2 | **Enregistrement simple.** Plein écran, 30 s, avec une vidéo YouTube qui joue et en parlant au micro. | Le MP4 se lit. **Le curseur est visible**, à la bonne position. On entend le son système **en stéréo** et le micro, à un volume normal (plus divisé par deux). | |
| 3 | **Synchro.** Lire en plein écran dans le navigateur le fichier de référence `sync-reference.mp4` (commande ci-dessous ; les vidéos YouTube de test ne sont pas fiables), lancer la lecture **puis** démarrer l'enregistrement, 1 min. Garder *Son → Décalage* à sa nouvelle valeur par défaut, **+50 ms** (sur une installation neuve ; sinon le régler à 50). | Bip et flash ensemble à ±1 image, **au début comme à la fin**, sans dérive. Le journal indique `Audio sources : … (retard 50 ms)`. | |
| 4 | **Pause.** 10 s d'enregistrement, `Ctrl+Shift+F10`, 10 s de pause en parlant, `Ctrl+Shift+F10`, 10 s, puis arrêt. Refaire avec le bouton de l'interface. | La vidéo dure environ 20 s, rien de la pause n'apparaît (ni image ni son), et la synchro est intacte après la reprise. | |
| 5 | **Écran statique.** 20 s sur un bureau immobile, webcam activée, à 30 FPS. | La vidéo dure bien 20 s et la webcam reste animée. L'incrustation fait environ 1/5 de la largeur, **sans déformation** ; essayer les 4 coins. Le journal indique `Webcam · ouverte (640x360@30FPS, …)` (plus `@1FPS`), puis en fin d'enregistrement `Worker webcam · terminé — N images en X s (Y i/s)` : relever Y. | |
| 6 | **Raccourcis.** App réduite, `Ctrl+Shift+F9` deux fois. Puis démarrer/arrêter depuis l'icône de la barre des tâches. | Démarrage puis arrêt ; l'icône de la barre des tâches marche aussi. **Notifications** : Windows ne les affiche que pour une app installée (identifiant d'application posé par le raccourci de l'installeur) ; avec l'exe portable, leur absence est attendue. | |
| 7 | **Coupure d'accès à l'écran** pendant un enregistrement : `Ctrl+Alt+Suppr` puis `Échap`, et une invite UAC (clic droit *Exécuter en tant qu'administrateur*), laissée ouverte quelques secondes puis fermée. | L'enregistrement **continue**. L'image se fige (dernière image avant l'invite) pendant l'écran sécurisé, **puis repart** une fois l'invite fermée. Le journal contient `DXGI · accès perdu` puis `duplication récupérée`, sans rafale de centaines de lignes. Le résumé `Vidéo · capture : …` compte ces attentes à part (`N attente(s) d'écran inaccessible`) et non comme gels. | |
| 8 | **Quitter en plein enregistrement** : fermer la fenêtre, puis recommencer avec *Quitter* de l'icône de la barre des tâches. | L'app se ferme après une ou deux secondes, et **le MP4 se lit**. | |
| 9 | **Fenêtre et région** (avec 2 écrans) : capturer une fenêtre placée sur l'écran 2, puis une région. Essayer aussi de démarrer en mode fenêtre sans en choisir une. | La fenêtre est cadrée juste, **sans bande noire ou transparente** autour. Le sélecteur de région s'ouvre **sur l'écran de la souris**, et la zone capturée correspond. Sans fenêtre choisie, une erreur s'affiche. | |
| 10 | **Dossier de sortie.** Le changer, puis relancer l'app. | Le dossier choisi est conservé. | |
| 11 | **Téléphone.** Installer l'APK `yawrec-android-debug-<sha>` du même run CI (autoriser les sources inconnues ; désinstaller d'abord une version d'un autre run, la clé de signature change à chaque run). Ouvrir le QR code, le scanner avec l'APK, puis démarrer / pause / arrêt depuis le téléphone. Passer ensuite le téléphone en mode avion **1 min**, puis le rétablir. | Ça se connecte, les commandes répondent et le chrono est à jour. Après la coupure, **la reconnexion est automatique** (nouvelle ligne `WS: client authentifié` dans le journal). Sur l'écran d'enregistrement du téléphone, la tuile « Caméra · bientôt » est grisée. | |
| 12 | **Sécurité du téléphone.** Modifier une lettre du code d'appairage sur le téléphone. | Le message « Code d'appairage invalide » s'affiche et l'app ne réessaie pas en boucle. | |
| 13 | **Instance unique.** YawREC ouvert, relancer `YawREC.exe` (double-clic). | La fenêtre existante revient au premier plan. Il n'y a qu'un seul `yawrec.exe` dans le Gestionnaire des tâches. | |
| 14 | **Script automatique** (nécessite `ffprobe` dans le PATH). App fermée : `.\scripts\test_audio.ps1 -ExePath <dossier>\yawrec.exe` | 14/14, sur des MP4 créés pendant le test. Dans `yawrec.log`, relever les lignes `Vidéo · capture : …` (doit indiquer 0 gel, y compris au démarrage, et 0 frame abandonnée), `Vidéo · encodage : …` et les éventuels `Encodeur · appel lent …` (le temps y est désormais réparti entre `receive_packet` et `écriture`). Avec ffprobe, `start_time` de la piste audio doit être proche de 0 (plus de début muet) : `ffprobe -v error -show_entries stream=codec_name,start_time -of compact fichier.mp4`. | |
| 15 | **Matériel varié**, selon ce qui est disponible : casque Bluetooth, sortie 48 kHz, son 5.1, micro seul, son système seul. | Le son n'est ni accéléré, ni haché, ni muet. | |

## Fichier de référence pour la synchro (test 3)

Écran blanc 100 ms + bip 1 kHz 100 ms toutes les 2 s, noir et silence entre les deux, 62 s :
```
ffmpeg -f lavfi -i "color=c=black:s=1920x1080:r=30:d=62" -f lavfi -i "sine=frequency=1000:sample_rate=48000:duration=62" -filter_complex "[0:v]drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='lt(mod(t,2),0.1)'[v];[1:a]volume='if(lt(mod(t,2),0.1),0.5,0)':eval=frame,aformat=channel_layouts=stereo[a]" -map "[v]" -map "[a]" -c:v libx264 -preset veryfast -pix_fmt yuv420p -c:a aac -b:a 128k sync-reference.mp4
```

## À renvoyer

- Ce tableau rempli (OK / KO + commentaire).
- Le fichier `yawrec.log` d'une session où un KO est survenu.
- Pour les problèmes de synchro ou de durée, si possible la sortie de cette commande (ffprobe n'est pas dans le zip ; MediaInfo convient aussi) :
  ```
  ffprobe -v error -show_entries stream=codec_name,avg_frame_rate,nb_frames,duration -of compact fichier.mp4
  ```
