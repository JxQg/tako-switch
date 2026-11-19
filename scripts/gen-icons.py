#!/usr/bin/env python3
"""Generate Tako Switch v2 app & tray PNG, ICNS, and ICO icons cleanly with zero external deps."""

import math
import pathlib
import shutil
import struct
import subprocess
import zlib


def encode_png(width: int, height: int, rgba: bytes) -> bytes:
    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)
        raw.extend(rgba[y * stride : (y + 1) * stride])
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(bytes(raw), level=9)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def render_app_icon(size: int) -> bytes:
    """Render a crisp rounded-square dark indigo badge with a modern switch + spark glyph."""
    buf = bytearray(size * size * 4)
    half = size / 2.0
    pad = size * 0.06
    box = half - pad
    radius = size * 0.22

    # Switch capsule + dual toggle dots in crisp white & aqua gradient
    cap_w = size * 0.56
    cap_h = size * 0.28
    knob_r = cap_h * 0.37

    for y in range(size):
        for x in range(size):
            px = x + 0.5
            py = y + 0.5
            dx = abs(px - half) - (box - radius)
            dy = abs(py - half) - (box - radius)
            qx = max(dx, 0.0)
            qy = max(dy, 0.0)
            dist = math.hypot(qx, qy) + min(max(dx, dy), 0.0) - radius

            alpha = max(0.0, min(1.0, 0.7 - dist))
            if alpha <= 0.0:
                continue

            # Subtle diagonal indigo/obsidian background gradient
            t = ((px + py) / (2.0 * size))
            r = 16.0 + 18.0 * t
            g = 24.0 + 36.0 * (1.0 - t * 0.4)
            b = 48.0 + 64.0 * t

            # Outer subtle border highlight
            if -1.5 * (size / 128.0) < dist <= 0.0:
                r = min(255.0, r + 42.0)
                g = min(255.0, g + 58.0)
                b = min(255.0, b + 88.0)

            # Switch track centered horizontally, slightly above middle + second bar below
            # Track 1 (upper switch, ON aqua pill)
            t1_cy = size * 0.37
            tx = abs(px - half) - (cap_w / 2.0 - cap_h / 2.0)
            ty = abs(py - t1_cy)
            tdist = math.hypot(max(tx, 0.0), max(ty, 0.0)) - (cap_h / 2.0) if tx > 0 else ty - (cap_h / 2.0)
            if tdist < 0.8:
                mix = max(0.0, min(1.0, 0.8 - tdist))
                tr, tg, tb = 45.0, 212.0, 191.0  # bright teal
                # Right active knob
                kdist = math.hypot(px - (half + cap_w * 0.23), py - t1_cy) - knob_r
                if kdist < 0.8:
                    kmix = max(0.0, min(1.0, 0.8 - kdist))
                    tr = tr * (1 - kmix) + 255.0 * kmix
                    tg = tg * (1 - kmix) + 255.0 * kmix
                    tb = tb * (1 - kmix) + 255.0 * kmix
                r = r * (1 - mix) + tr * mix
                g = g * (1 - mix) + tg * mix
                b = b * (1 - mix) + tb * mix

            # Track 2 (lower bar — triple Tako nodes representing 3 apps)
            t2_cy = size * 0.67
            node_r = size * 0.068
            for idx, nx_off in enumerate((-0.20, 0.0, 0.20)):
                ndist = math.hypot(px - (half + size * nx_off), py - t2_cy) - node_r
                if ndist < 0.8:
                    nmix = max(0.0, min(1.0, 0.8 - ndist))
                    palette = ((248, 250, 252), (125, 211, 252), (94, 234, 212))[idx]
                    r = r * (1 - nmix) + palette[0] * nmix
                    g = g * (1 - nmix) + palette[1] * nmix
                    b = b * (1 - nmix) + palette[2] * nmix

            i = (y * size + x) * 4
            buf[i] = max(0, min(255, int(round(r))))
            buf[i + 1] = max(0, min(255, int(round(g))))
            buf[i + 2] = max(0, min(255, int(round(b))))
            buf[i + 3] = max(0, min(255, int(round(alpha * 255.0))))
    return encode_png(size, size, bytes(buf))


def render_tray_icon(size: int = 32) -> bytes:
    """Monochrome white switch glyph for macOS template / Windows tray."""
    buf = bytearray(size * size * 4)
    half = size / 2.0
    cap_w = size * 0.72
    cap_h = size * 0.42
    cy = size * 0.48
    stroke = max(1.8, size * 0.085)
    knob_r = cap_h * 0.29

    for y in range(size):
        for x in range(size):
            px = x + 0.5
            py = y + 0.5
            tx = abs(px - half) - (cap_w / 2.0 - cap_h / 2.0)
            ty = abs(py - cy)
            odist = math.hypot(max(tx, 0.0), max(ty, 0.0)) - (cap_h / 2.0) if tx > 0 else ty - (cap_h / 2.0)
            ring = abs(odist) - stroke / 2.0
            knob = math.hypot(px - (half + cap_w * 0.21), py - cy) - knob_r
            d = min(ring, knob)
            alpha = max(0.0, min(1.0, 0.8 - d))
            i = (y * size + x) * 4
            buf[i : i + 3] = b"\xff\xff\xff"
            buf[i + 3] = int(round(alpha * 255.0))
    return encode_png(size, size, bytes(buf))


def write_ico(path: pathlib.Path, png_map: list[tuple[int, bytes]]) -> None:
    count = len(png_map)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    entries = bytearray()
    payload = bytearray()
    for sz, data in png_map:
        dim = 0 if sz >= 256 else sz
        entries.extend(struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset))
        payload.extend(data)
        offset += len(data)
    path.write_bytes(header + bytes(entries) + bytes(payload))


def main() -> None:
    icons_dir = pathlib.Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
    icons_dir.mkdir(parents=True, exist_ok=True)

    p32 = render_app_icon(32)
    p128 = render_app_icon(128)
    p256 = render_app_icon(256)
    p512 = render_app_icon(512)
    p1024 = render_app_icon(1024)

    (icons_dir / "32x32.png").write_bytes(p32)
    (icons_dir / "128x128.png").write_bytes(p128)
    (icons_dir / "128x128@2x.png").write_bytes(p256)
    (icons_dir / "icon.png").write_bytes(p512)
    (icons_dir / "tray.png").write_bytes(render_tray_icon(32))
    write_ico(icons_dir / "icon.ico", [(32, p32), (128, p128), (256, p256)])

    if shutil.which("iconutil"):
        iconset = icons_dir / "icon.iconset"
        shutil.rmtree(iconset, ignore_errors=True)
        iconset.mkdir()
        p16 = render_app_icon(16)
        (iconset / "icon_16x16.png").write_bytes(p16)
        (iconset / "icon_16x16@2x.png").write_bytes(p32)
        (iconset / "icon_32x32.png").write_bytes(p32)
        (iconset / "icon_32x32@2x.png").write_bytes(render_app_icon(64))
        (iconset / "icon_128x128.png").write_bytes(p128)
        (iconset / "icon_128x128@2x.png").write_bytes(p256)
        (iconset / "icon_256x256.png").write_bytes(p256)
        (iconset / "icon_256x256@2x.png").write_bytes(p512)
        (iconset / "icon_512x512.png").write_bytes(p512)
        (iconset / "icon_512x512@2x.png").write_bytes(p1024)
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(icons_dir / "icon.icns")], check=True)
        shutil.rmtree(iconset)


if __name__ == "__main__":
    main()
