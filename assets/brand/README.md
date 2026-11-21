# kuverta brand

## The mark

An envelope with a stem rising from its left edge. The stem and the envelope's
flap together make a lowercase **k**: the flap is the k's arm. A narrow gap
splits the flap from the body, like a crease. The body is close to √2 : 1,
the DIN A-series ratio used for German paper and envelopes. Every straight
edge of the body sits on a 32 px grid of the 1024 canvas, so the icon stays
sharp at 32×32.

## Palette

| Name   | Hex       | Use                                              |
|--------|-----------|--------------------------------------------------|
| Petrol | `#1B4A4E` | Tile, flat fills (gradient `#22585C` → `#0F2F32`) |
| Paper  | `#F4EFE6` | The envelope; the wordmark on dark               |
| Amber  | `#E9A23B` | The flap only (gradient `#F2AE48` → `#E0922F`)   |
| Ink    | `#16292B` | The wordmark on light                            |

Use amber only on the flap.

## Files

| File                    | Use                                                                 |
|-------------------------|---------------------------------------------------------------------|
| `kuverta-mark.svg`      | App icon master (1024, 824 px superellipse body on Apple's grid). Source for `apps/desktop/icons/`. |
| `kuverta-glyph.svg`     | Single-colour mark in `currentColor` (24×24). Inline in the UI next to the name; it follows the text colour in light and dark mode. |
| `kuverta-logo.svg`      | Horizontal lockup for light backgrounds.                            |
| `kuverta-logo-dark.svg` | Horizontal lockup for dark backgrounds.                             |
| `favicon.svg`           | Website favicon (full-bleed tile, 32 grid).                         |

The wordmark is drawn as outlines, so it needs no font and renders the same
everywhere.

## Sizes and spacing

- App icon: 16 px minimum. The flap gap disappears below 24 px, but the k still reads.
- Glyph: 14 px minimum. Built for 16 to 20 px beside UI text.
- Lockup: 20 px tall minimum.
- Clear space: at least ¼ of the tile's height on every side. For the glyph,
  the width of its stem.
- Do not recolour, rotate, outline or add effects to the mark. Do not set the
  wordmark in a font.

## Regenerating the app icons

Render `kuverta-mark.svg` to a 1024 px PNG with a real browser engine
(Chromium or WebKit). ImageMagick's built-in SVG renderer draws it as a solid
black tile. Then run:

```sh
cargo tauri icon mark-1024.png -o /tmp/icons
cp /tmp/icons/{32x32.png,128x128.png,128x128@2x.png,icon.png,icon.icns,icon.ico} apps/desktop/icons/
```
