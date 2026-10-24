# Procédure de test Windows — branche `fix/revue-complete`

> **Fichier temporaire** : à supprimer une fois la campagne de test terminée.

## Récupérer l'exécutable

1. Ouvrir les runs CI de la branche (connecté à GitHub) :
   https://github.com/coucagog/YawREC/actions?query=branch%3Afix%2Frevue-complete
   → dernier run vert → section *Artifacts* → `yawrec-windows-portable-…` (≈ 70 Mo, conservé 14 jours).
2. Dézipper dans un dossier : l'exe et les DLL FFmpeg doivent rester ensemble.
3. Lancer `YawREC.exe`.
   - Exe non signé, SmartScreen bloque : *Informations complémentaires → Exécuter quand même*.
   - Pare-feu : autoriser les **réseaux privés** (nécessaire pour le téléphone).

**Journal** : `%TEMP%\yawrec.log`. **Il est effacé à chaque lancement** : le copier avant de relancer l'app.

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
| 3 | **Synchro.** Enregistrer 1 min d'une vidéo « audio video sync test » de YouTube (flash + bip). | Bip et flash ensemble à ±1 image, **au début comme à la fin** : aucune dérive. | |
| 4 | **Pause.** 10 s d'enregistrement, `Ctrl+Shift+F10`, 10 s de pause en parlant, `Ctrl+Shift+F10`, 10 s, puis arrêt. Refaire avec le bouton de l'interface. | La vidéo dure environ 20 s, rien de la pause n'apparaît (ni image ni son), et la synchro est intacte après la reprise. | |
| 5 | **Écran statique.** 20 s sur un bureau immobile, webcam activée, à 30 FPS. | La vidéo dure bien 20 s et la webcam reste animée. L'incrustation fait environ 1/5 de la largeur, sans déformation ; essayer les 4 coins. | |
| 6 | **Raccourcis.** App réduite, `Ctrl+Shift+F9` deux fois. Puis démarrer/arrêter depuis l'icône de la barre des tâches. | Démarrage puis arrêt, avec une notification. L'icône de la barre des tâches marche aussi. | |
| 7 | **Coupure d'accès à l'écran** pendant un enregistrement : `Ctrl+Alt+Suppr` puis `Échap`, et une invite UAC (clic droit *Exécuter en tant qu'administrateur*). | L'enregistrement **continue**. L'image se fige pendant l'écran sécurisé puis repart. Le journal contient `DXGI · accès perdu`. | |
| 8 | **Quitter en plein enregistrement** : fermer la fenêtre, puis recommencer avec *Quitter* de l'icône de la barre des tâches. | L'app se ferme après une ou deux secondes, et **le MP4 se lit**. | |
| 9 | **Fenêtre et région** (avec 2 écrans) : capturer une fenêtre placée sur l'écran 2, puis une région. Essayer aussi de démarrer en mode fenêtre sans en choisir une. | La fenêtre est cadrée juste, **sans bande noire ou transparente** autour. Le sélecteur de région s'ouvre **sur l'écran de la souris**, et la zone capturée correspond. Sans fenêtre choisie, une erreur s'affiche. | |
| 10 | **Dossier de sortie.** Le changer, puis relancer l'app. | Le dossier choisi est conservé. | |
| 11 | **Téléphone.** Ouvrir le QR code, le scanner avec l'APK, puis démarrer / pause / arrêt depuis le téléphone. Couper ensuite le Wi-Fi du téléphone 10 s. | Ça se connecte, les commandes répondent et le chrono est à jour. Après la coupure, **la reconnexion est automatique**. | |
| 12 | **Sécurité du téléphone.** Modifier une lettre du code d'appairage sur le téléphone. | Le message « Code d'appairage invalide » s'affiche et l'app ne réessaie pas en boucle. | |
| 13 | **Matériel varié**, selon ce qui est disponible : casque Bluetooth, sortie 48 kHz, son 5.1, micro seul, son système seul. | Le son n'est ni accéléré, ni haché, ni muet. | |

## À renvoyer

- Ce tableau rempli (OK / KO + commentaire).
- Le fichier `yawrec.log` d'une session où un KO est survenu.
- Pour les problèmes de synchro ou de durée, si possible la sortie de cette commande (ffprobe n'est pas dans le zip ; MediaInfo convient aussi) :
  ```
  ffprobe -v error -show_entries stream=codec_name,avg_frame_rate,nb_frames,duration -of compact fichier.mp4
  ```
