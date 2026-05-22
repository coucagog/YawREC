"""Génère les icônes YawREC dans tous les formats requis par Tauri.

Deux états visuels :
  - idle      : cercle rouge sur fond sombre arrondi (logo de l'app)
  - recording : disque rouge plein avec halo (icône tray pendant capture)

Le logo "idle" sert pour le bundle de l'app (Windows .ico, .png) et pour
la tray quand on n'enregistre pas. L'icône "recording" remplace celle de
la tray pendant la capture pour signaler l'état actif.
"""

from PIL import Image, ImageDraw
import os
import struct

ICONS_DIR = os.path.dirname(os.path.abspath(__file__))


def draw_idle(size):
    """Logo : carré sombre arrondi avec un cercle rouge contour épais."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Fond sombre arrondi
    radius = max(3, size // 7)
    d.rounded_rectangle(
        [(1, 1), (size - 1, size - 1)],
        radius=radius,
        fill=(30, 30, 30, 255),
        outline=(80, 80, 80, 255),
        width=max(1, size // 32),
    )

    # Cercle rouge centré
    margin = size // 4
    d.ellipse(
        [(margin, margin), (size - margin, size - margin)],
        outline=(239, 68, 68, 255),
        width=max(2, size // 12),
    )
    return img


def draw_recording(size):
    """Icône tray pendant capture : disque rouge plein + léger halo."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Halo rouge
    halo_margin = size // 6
    d.ellipse(
        [(halo_margin, halo_margin), (size - halo_margin, size - halo_margin)],
        fill=(239, 68, 68, 90),
    )
    # Disque plein
    margin = size // 4
    d.ellipse(
        [(margin, margin), (size - margin, size - margin)],
        fill=(239, 68, 68, 255),
    )
    return img


def main():
    # Tailles PNG nécessaires pour le bundle Tauri
    png_sizes = [32, 128, 256]
    for s in png_sizes:
        path = os.path.join(ICONS_DIR, f"{s}x{s}.png")
        draw_idle(s).save(path)
        print(f"  {path}")

    # Variante Retina 128x128@2x = 256x256
    draw_idle(256).save(os.path.join(ICONS_DIR, "128x128@2x.png"))
    print(f"  {os.path.join(ICONS_DIR, '128x128@2x.png')}")

    # Icône tray "recording"
    draw_recording(32).save(os.path.join(ICONS_DIR, "tray-recording.png"))
    print(f"  {os.path.join(ICONS_DIR, 'tray-recording.png')}")
    draw_idle(32).save(os.path.join(ICONS_DIR, "tray-idle.png"))
    print(f"  {os.path.join(ICONS_DIR, 'tray-idle.png')}")

    # icon.ico (multi-résolution Windows)
    sizes_ico = [16, 24, 32, 48, 64, 128, 256]
    images_ico = [draw_idle(s) for s in sizes_ico]
    images_ico[0].save(
        os.path.join(ICONS_DIR, "icon.ico"),
        format="ICO",
        sizes=[(s, s) for s in sizes_ico],
        append_images=images_ico[1:],
    )
    print(f"  {os.path.join(ICONS_DIR, 'icon.ico')}")

    # icon.icns minimal (macOS) — on stocke juste un PNG 256x256 dans un wrapper
    # Format ICNS : magic 'icns' + size + (OSType 'ic09' + size + PNG bytes)
    png_bytes = open(os.path.join(ICONS_DIR, "128x128@2x.png"), "rb").read()
    icns_block_type = b"ic09"  # 512x512 selon Apple — on triche un peu, c'est OK pour Tauri
    block = icns_block_type + struct.pack(">I", 8 + len(png_bytes)) + png_bytes
    icns_data = b"icns" + struct.pack(">I", 8 + len(block)) + block
    with open(os.path.join(ICONS_DIR, "icon.icns"), "wb") as f:
        f.write(icns_data)
    print(f"  {os.path.join(ICONS_DIR, 'icon.icns')}")


if __name__ == "__main__":
    main()
