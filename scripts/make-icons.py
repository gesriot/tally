"""Готовит значки приложения из `assets/icon.png` (нужен Python с Pillow).

Исходник – квадратная картинка без прозрачности. Скрипт скругляет углы и
пишет рядом файлы, которые лежат в репозитории и нужны сборке:

  assets/icon-window.png  256x256 – значок окон в `.slint` (запасной вариант);
  assets/icon-tray.png     64x64  – значок в трее;
  assets/icon.ico          16..256 – значок exe для Windows: те размеры, что
                                      Windows берёт при масштабе 100–400 %
                                      (16, 20, 24, 32, 40, 48, 64, 96, 128, 256).
                                      Его вшивает `scripts/package-windows.ps1`,
                                      а окно берёт из него размер под свой DPI.

В `.ico` кадры до 128 px – обычные 32-битные BMP (их читает любой код Windows),
256 px – PNG, как принято в значках Windows.

Запуск из корня репозитория: python scripts/make-icons.py
Менять `assets/icon.png` нужно только вместе с повторным запуском скрипта.
"""

import struct
import sys
from io import BytesIO
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ModuleNotFoundError:
    sys.exit("ошибка: нужен пакет Pillow (pip install pillow)")

ASSETS = Path(__file__).resolve().parent.parent / "assets"
SOURCE = ASSETS / "icon.png"
# Радиус скругления – доля стороны; около 22 % – как у значков macOS и iOS.
CORNER_RATIO = 0.2237
# Размеры значка Windows: 16/24/32 – панель задач и заголовки, 20/40 – масштаб 125 %,
# 48/64/96 – крупные значки и масштаб 150–300 %, 128/256 – проводник.
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
# До какого размера кадры чуть подчёркиваются, чтобы мелкие детали не «плыли».
SHARPEN_UP_TO = 48
# Начиная с какого размера кадр пишется как PNG (а не BMP).
PNG_FROM = 256


def rounded_mask(size: int) -> Image.Image:
    scale = 4  # сглаживание краёв: рисуем в 4 раза крупнее и уменьшаем
    big = size * scale
    mask = Image.new("L", (big, big), 0)
    radius = round(size * CORNER_RATIO * scale)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, big - 1, big - 1), radius=radius, fill=255)
    return mask.resize((size, size), Image.Resampling.LANCZOS)


def frame(master: Image.Image, side: int) -> Image.Image:
    image = master.resize((side, side), Image.Resampling.LANCZOS)
    if side <= SHARPEN_UP_TO:
        image = image.filter(ImageFilter.UnsharpMask(radius=0.6, percent=70, threshold=0))
    return image


def bmp_frame(image: Image.Image) -> bytes:
    """Кадр ICO в виде DIB: 32-битные BGRA снизу вверх и маска AND."""
    side = image.width
    rgba = image.tobytes()
    pixels = bytearray()
    mask_rows = []
    mask_stride = ((side + 31) // 32) * 4
    for y in range(side - 1, -1, -1):
        row = rgba[y * side * 4:(y + 1) * side * 4]
        mask = bytearray(mask_stride)
        for x in range(side):
            r, g, b, a = row[x * 4:x * 4 + 4]
            pixels += bytes((b, g, r, a))
            if a == 0:
                mask[x // 8] |= 0x80 >> (x % 8)
        mask_rows.append(bytes(mask))
    mask_data = b"".join(mask_rows)
    header = struct.pack(
        "<IiiHHIIiiII", 40, side, side * 2, 1, 32, 0, len(pixels) + len(mask_data), 0, 0, 0, 0
    )
    return header + bytes(pixels) + mask_data


def write_ico(path: Path, master: Image.Image) -> None:
    frames = []
    for side in ICO_SIZES:
        image = frame(master, side)
        if side >= PNG_FROM:
            buffer = BytesIO()
            image.save(buffer, format="PNG", optimize=True)
            frames.append((side, buffer.getvalue()))
        else:
            frames.append((side, bmp_frame(image)))

    out = bytearray(struct.pack("<HHH", 0, 1, len(frames)))
    offset = 6 + 16 * len(frames)
    for side, data in frames:
        dimension = 0 if side >= 256 else side  # 0 означает 256
        out += struct.pack("<BBBBHHII", dimension, dimension, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    for _, data in frames:
        out += data
    path.write_bytes(bytes(out))


def main() -> None:
    if not SOURCE.is_file():
        sys.exit(f"ошибка: нет исходника {SOURCE}")
    image = Image.open(SOURCE).convert("RGBA")
    if image.width != image.height:
        sys.exit(f"ошибка: значок должен быть квадратным, а он {image.width}x{image.height}")

    master = image.resize((1024, 1024), Image.Resampling.LANCZOS)
    master.putalpha(rounded_mask(1024))

    for name, side in (("icon-window.png", 256), ("icon-tray.png", 64)):
        master.resize((side, side), Image.Resampling.LANCZOS).save(ASSETS / name, optimize=True)
    write_ico(ASSETS / "icon.ico", master)
    for name in ("icon-window.png", "icon-tray.png", "icon.ico"):
        print(f"{name}: {(ASSETS / name).stat().st_size} байт")


if __name__ == "__main__":
    main()
