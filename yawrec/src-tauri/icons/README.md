# Icons

Placez ici les icônes de l'application (ces fichiers seront référencés
par `tauri.conf.json` → `bundle.icon`).

Fichiers attendus :

- `32x32.png`         — icône PNG 32×32
- `128x128.png`       — icône PNG 128×128
- `128x128@2x.png`    — icône PNG 256×256 (Retina)
- `icon.icns`         — bundle macOS
- `icon.ico`          — bundle Windows

## Génération rapide

À partir d'un PNG source 1024×1024 :

```bash
cargo tauri icon path/to/source.png
```

Cette commande produit l'ensemble des formats requis dans ce dossier.
