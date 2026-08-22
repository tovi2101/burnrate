"""Generate Burnrate's multi-resolution Windows icon without third-party tools."""

from __future__ import annotations

import binascii
import struct
import zlib
from pathlib import Path


SIZES = (16, 20, 24, 32)
SUPERSAMPLE = 4
COLORS = (
    (220, 139, 102, 255),
    (143, 203, 155, 255),
    (190, 150, 237, 255),
    (102, 202, 209, 255),
    (242, 109, 120, 255),
)


def chunk(kind: bytes, payload: bytes) -> bytes:
    body = kind + payload
    return struct.pack(">I", len(payload)) + body + struct.pack(">I", binascii.crc32(body) & 0xFFFFFFFF)


def inside_rounded_rect(x: float, y: float, left: float, top: float, right: float, bottom: float, radius: float) -> bool:
    nearest_x = min(max(x, left + radius), right - radius)
    nearest_y = min(max(y, top + radius), bottom - radius)
    return (x - nearest_x) ** 2 + (y - nearest_y) ** 2 <= radius ** 2


def render(size: int) -> bytes:
    scale = size / 32
    high = size * SUPERSAMPLE
    pixels = [(0, 0, 0, 0)] * (high * high)

    def paint_rect(left: float, top: float, right: float, bottom: float, color: tuple[int, int, int, int]) -> None:
        for y in range(high):
            py = (y + 0.5) / SUPERSAMPLE
            if not top <= py < bottom:
                continue
            for x in range(high):
                px = (x + 0.5) / SUPERSAMPLE
                if left <= px < right:
                    pixels[y * high + x] = color

    for y in range(high):
        py = (y + 0.5) / SUPERSAMPLE
        for x in range(high):
            px = (x + 0.5) / SUPERSAMPLE
            if inside_rounded_rect(px, py, 1.5 * scale, 1.5 * scale, size - 1.5 * scale, size - 1.5 * scale, 6 * scale):
                pixels[y * high + x] = (242, 244, 248, 255)
            if inside_rounded_rect(px, py, 3 * scale, 3 * scale, size - 3 * scale, size - 3 * scale, 4.7 * scale):
                pixels[y * high + x] = (16, 17, 21, 255)

    centers = (8.2, 12.1, 16.0, 19.9, 23.8)
    heights = (7.0, 11.0, 15.0, 9.0, 13.0)
    baseline = 25.0
    for center, height, color in zip(centers, heights, COLORS):
        left = (center - 1.45) * scale
        right = (center + 1.45) * scale
        top = (baseline - height) * scale
        bottom = baseline * scale
        paint_rect(left, top, right, bottom, (235, 238, 244, 255))
        inset = max(0.55 * scale, 0.45)
        paint_rect(left + inset, top + inset, right - inset, bottom - inset, color)

    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            samples = [pixels[(y * SUPERSAMPLE + sy) * high + x * SUPERSAMPLE + sx] for sy in range(SUPERSAMPLE) for sx in range(SUPERSAMPLE)]
            rows.extend(sum(sample[channel] for sample in samples) // len(samples) for channel in range(4))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(rows), 9)) + chunk(b"IEND", b"")


def render_template(size: int) -> bytes:
    high = size * SUPERSAMPLE
    pixels = [0] * (high * high)
    scale = size / 18
    for y in range(high):
        py = (y + 0.5) / SUPERSAMPLE
        for x in range(high):
            px = (x + 0.5) / SUPERSAMPLE
            outer = inside_rounded_rect(px, py, 1 * scale, 1 * scale, size - scale, size - scale, 3.2 * scale)
            inner = inside_rounded_rect(px, py, 2.15 * scale, 2.15 * scale, size - 2.15 * scale, size - 2.15 * scale, 2.2 * scale)
            if outer and not inner:
                pixels[y * high + x] = 255
    centers = (4.7, 6.85, 9, 11.15, 13.3)
    heights = (4, 6.2, 8.4, 5.2, 7.2)
    for center, height in zip(centers, heights):
        left, right = (center - .65) * scale, (center + .65) * scale
        top, bottom = (14 - height) * scale, 14 * scale
        for y in range(high):
            py = (y + 0.5) / SUPERSAMPLE
            if not top <= py < bottom:
                continue
            for x in range(high):
                px = (x + 0.5) / SUPERSAMPLE
                if left <= px < right:
                    pixels[y * high + x] = 255
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            alpha = sum(pixels[(y * SUPERSAMPLE + sy) * high + x * SUPERSAMPLE + sx] for sy in range(SUPERSAMPLE) for sx in range(SUPERSAMPLE)) // (SUPERSAMPLE ** 2)
            rows.extend((0, 0, 0, alpha))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(rows), 9)) + chunk(b"IEND", b"")


def main() -> None:
    frames = [(size, render(size)) for size in SIZES]
    offset = 6 + 16 * len(frames)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(frames)))
    payload = bytearray()
    for size, png in frames:
        directory.extend(struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(png), offset))
        payload.extend(png)
        offset += len(png)
    output = Path(__file__).resolve().parents[1] / "src-tauri" / "icons" / "icon.ico"
    output.write_bytes(directory + payload)
    template = output.with_name("trayTemplate.png")
    template_2x = output.with_name("trayTemplate@2x.png")
    template.write_bytes(render_template(18))
    template_2x.write_bytes(render_template(36))
    print(f"generated {output} with sizes {', '.join(map(str, SIZES))}")
    print(f"generated macOS templates {template.name} (18px) and {template_2x.name} (36px)")


if __name__ == "__main__":
    main()
