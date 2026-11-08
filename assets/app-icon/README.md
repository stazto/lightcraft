# Lightspot app icon

The **"Ls" mark**: the letters L and s in cyan (`#2dc0fe`) on a full-bleed navy tile (`#02173b`),
no roundel or border.

## Geometry

A rounded-square tile with `rx=112` on a 512 grid, clipped; the navy fills the whole tile.
macOS files (`.icns`, `lightcraft-macos-512.png`) add Apple's transparent margin (an 824/1024 body);
Windows and Linux files use the full-bleed tile so the mark reads at 16-48 px.

## Provenance

The owner's original artwork, supplied as a 1254 px raster. Licence: `LICENSE.txt`
(MIT OR Apache-2.0, like the repo).

## Files

| File | What |
|---|---|
| `lightcraft.svg` | the tile with the artwork (1024 px) embedded and clipped round |
| `lightcraft-small.svg` | the same with a 512 px embed (Linux scalable icon) |
| `lightcraft-1024.png` | 1024 px render (store listings, docs) |
| `lightcraft-macos-512.png` | 512 px with Apple's margin: the runtime window / Dock icon on macOS |
| `lightcraft.icns` | macOS bundle icon (Apple's margin, 16-512 px and @2x) |
| `lightcraft.ico` | Windows icon (16-256 px) |
| `hicolor/<size>/apps/ai.storyteller.lightcraft.png` | Linux hicolor theme (also the runtime window icon on Windows and Linux: 256x256) |
| `hicolor/scalable/apps/ai.storyteller.lightcraft.svg` | Linux scalable icon (the small SVG) |

## Regenerating

The files above are rendered from the owner's artwork with Pillow and `iconutil` (the tile mask,
Apple's margin, the hicolor sizes, the `.ico` and the `.icns`). `packaging/icons.sh` renders the
same set from `lightcraft.svg` with `resvg`.
