#!/usr/bin/env python3
"""Generate VSParallel application icon variants from the canonical PNG."""

import io
import struct
from pathlib import Path

from PIL import Image


REPOSITORY = Path(__file__).resolve().parent.parent
SOURCE = REPOSITORY / "assets" / "icon.png"
TAURI_ICONS = REPOSITORY / "src-tauri" / "icons"
UI_ICON = REPOSITORY / "ui" / "vsparallel-icon.png"
COMPANION_ICON = REPOSITORY / "companion" / "icon.png"
LANCZOS = getattr(Image, "Resampling", Image).LANCZOS


def resized(source: Image.Image, size: int) -> Image.Image:
    # Resample premultiplied pixels so transparent edges stay blue instead of
    # acquiring a dark fringe at taskbar, Dock, and tray sizes.
    return source.convert("RGBa").resize((size, size), LANCZOS).convert("RGBA")


def tray_template(source: Image.Image, size: int) -> Image.Image:
    """Extract the bright logo mark for an adaptive monochrome macOS tray icon."""
    alpha = Image.new("L", source.size)
    alpha.putdata(
        [
            int(
                source_alpha
                * max(
                    0,
                    min(1, min((green - red - 55) / 35, (blue - 150) / 70)),
                )
            )
            for red, green, blue, source_alpha in source.getdata()
        ]
    )
    extracted = Image.new("RGBA", source.size, (255, 255, 255, 0))
    extracted.putalpha(alpha)
    extracted = resized(extracted, size)
    cleaned_alpha = extracted.getchannel("A").point(
        lambda value: 0 if value < 72 else min(255, round((value - 72) * 255 / 183))
    )
    extracted.putalpha(cleaned_alpha)
    return extracted


def png_bytes(image: Image.Image) -> bytes:
    output = io.BytesIO()
    image.save(output, format="PNG", optimize=True)
    return output.getvalue()


def save_icns(source: Image.Image, destination: Path) -> None:
    """Write the complete macOS 1x/2x desktop icon matrix.

    Pillow 9 omits the native 16 px and 32 px layers. Those two legacy layers
    use separate RGB and alpha-mask chunks; the remaining Retina-era layers
    are lossless PNG chunks.
    """

    icon_16 = resized(source, 16)
    icon_32 = resized(source, 32)
    chunks = [
        (b"is32", icon_16.convert("RGB").tobytes()),
        (b"s8mk", icon_16.getchannel("A").tobytes()),
        (b"ic11", png_bytes(icon_32)),
        (b"il32", icon_32.convert("RGB").tobytes()),
        (b"l8mk", icon_32.getchannel("A").tobytes()),
        (b"ic12", png_bytes(resized(source, 64))),
        (b"ic07", png_bytes(resized(source, 128))),
        (b"ic13", png_bytes(resized(source, 256))),
        (b"ic08", png_bytes(resized(source, 256))),
        (b"ic14", png_bytes(resized(source, 512))),
        (b"ic09", png_bytes(resized(source, 512))),
        (b"ic10", png_bytes(resized(source, 1024))),
    ]

    table = b"".join(code + struct.pack(">I", len(payload) + 8) for code, payload in chunks)
    entries = [(b"TOC ", table), *chunks]
    total_size = 8 + sum(len(payload) + 8 for _, payload in entries)
    with destination.open("wb") as output:
        output.write(b"icns")
        output.write(struct.pack(">I", total_size))
        for code, payload in entries:
            output.write(code)
            output.write(struct.pack(">I", len(payload) + 8))
            output.write(payload)


def main() -> None:
    source = Image.open(SOURCE).convert("RGBA")
    if source.width != source.height:
        raise SystemExit(f"icon source must be square, got {source.size}")

    corner_alpha = [
        source.getpixel((0, 0))[3],
        source.getpixel((source.width - 1, 0))[3],
        source.getpixel((0, source.height - 1))[3],
        source.getpixel((source.width - 1, source.height - 1))[3],
    ]
    if any(corner_alpha):
        raise SystemExit("icon source must have transparent corners")

    TAURI_ICONS.mkdir(parents=True, exist_ok=True)
    for obsolete_name in ("tray-icon.png", "tray-icon-template.png"):
        (TAURI_ICONS / obsolete_name).unlink(missing_ok=True)

    variants = {
        "16x16.png": 16,
        "24x24.png": 24,
        "32x32.png": 32,
        "48x48.png": 48,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "256x256.png": 256,
        "512x512.png": 512,
        "icon.png": 512,
    }
    for filename, size in variants.items():
        resized(source, size).save(TAURI_ICONS / filename, optimize=True)

    resized(source, 64).save(TAURI_ICONS / "tray-icon-windows.png", optimize=True)
    resized(source, 64).save(TAURI_ICONS / "tray-icon-linux.png", optimize=True)
    tray_template(source, 36).save(TAURI_ICONS / "tray-icon-macos.png", optimize=True)
    resized(source, 128).save(UI_ICON, optimize=True)
    resized(source, 128).save(COMPANION_ICON, optimize=True)
    source.save(
        TAURI_ICONS / "icon.ico",
        format="ICO",
        sizes=[
            (32, 32),
            (16, 16),
            (20, 20),
            (24, 24),
            (30, 30),
            (36, 36),
            (40, 40),
            (48, 48),
            (60, 60),
            (64, 64),
            (72, 72),
            (80, 80),
            (96, 96),
            (128, 128),
            (256, 256),
        ],
    )
    save_icns(source, TAURI_ICONS / "icon.icns")

    print(f"Generated {len(variants) + 7} icon assets from {SOURCE}")


if __name__ == "__main__":
    main()
