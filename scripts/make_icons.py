#!/usr/bin/env python3
"""Regenerate the Aurora logo and app icons.

Uses only the Python standard library. Run from the project root:

    python scripts/make_icons.py

The Windows build does NOT need this script. The generated files are already
included in the project; run this only if you want to change the logo design.
"""
import math
import os
import struct
import zlib

BLACK_HOLE_RADIUS = 0.36  # radius of the dark core (fraction of half the image)
PHOTON_RING = 0.37        # thin bright ring just outside the core
LENS_HALO = 0.40          # soft lensing glow around the core
DISK_PEAK = 0.62          # brightest radius of the accretion disk
DISK_SQUASH = 2.4         # vertical squash that makes the disk look tilted

_cache = {}


def _clamp(v, lo=0.0, hi=1.0):
    return lo if v < lo else hi if v > hi else v


def _byte(v):
    return max(0, min(255, int(v)))


def render(size):
    """Return an RGBA byte string with the Aurora emblem drawn at `size` pixels."""
    if size in _cache:
        return _cache[size]
    buf = bytearray(size * size * 4)
    c = (size - 1) / 2.0
    for py in range(size):
        ny = (py - c) / c
        for px in range(size):
            nx = (px - c) / c
            rr = math.hypot(nx, ny)                       # real-space radius
            dr = math.hypot(nx, ny * DISK_SQUASH)         # disk-space radius

            disk = math.exp(-((dr - DISK_PEAK) / 0.13) ** 2)
            glow = 0.35 * math.exp(-((dr - DISK_PEAK) / 0.38) ** 2)
            photon = 1.1 * math.exp(-((rr - PHOTON_RING) / 0.018) ** 2)
            halo = 0.5 * math.exp(-((rr - LENS_HALO) / 0.07) ** 2)
            # Soft saturation keeps the disk orange instead of flat yellow.
            t = 1.0 - math.exp(-1.6 * (disk + glow + photon + halo))

            # Colour ramp: deep red at the faint edges, orange, then pale yellow-white at the hot centre.
            r = 255 * (t ** 0.6)
            g = 170 * (t ** 2.0)
            b = 60 * (t ** 4.0)
            alpha = 255 * _clamp(t * 1.4)

            # Dark core with an anti-aliased edge.
            m = _clamp((BLACK_HOLE_RADIUS - rr) * c + 0.5)
            r = r * (1 - m) + 4 * m
            g = g * (1 - m) + 2 * m
            b = b * (1 - m) + 8 * m
            alpha = alpha * (1 - m) + 255 * m

            i = (py * size + px) * 4
            buf[i] = _byte(r)
            buf[i + 1] = _byte(g)
            buf[i + 2] = _byte(b)
            buf[i + 3] = _byte(alpha)
    _cache[size] = bytes(buf)
    return _cache[size]


def png_bytes(size, rgba):
    """Encode an RGBA buffer as a PNG file."""
    raw = bytearray()
    stride = size * 4
    for y in range(size):
        raw.append(0)  # filter type: none
        raw += rgba[y * stride:(y + 1) * stride]

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def ico_bytes(images):
    """Build a multi-size Windows ICO from (size, png_bytes) pairs."""
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    data = b""
    for size, png in images:
        w = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
        data += png
    return header + entries + data


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    png_targets = {
        "public/logo.png": 512,
        "assets/logo.png": 1024,
        "src-tauri/icons/icon.png": 512,
        "src-tauri/icons/32x32.png": 32,
        "src-tauri/icons/128x128.png": 128,
        "src-tauri/icons/128x128@2x.png": 256,
    }
    for rel, size in png_targets.items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(png_bytes(size, render(size)))
        print(f"wrote {rel} ({size}x{size})")

    ico_sizes = [16, 32, 48, 256]
    images = [(s, png_bytes(s, render(s))) for s in ico_sizes]
    ico_path = os.path.join(root, "src-tauri/icons/icon.ico")
    with open(ico_path, "wb") as f:
        f.write(ico_bytes(images))
    print(f"wrote src-tauri/icons/icon.ico (sizes {ico_sizes})")


if __name__ == "__main__":
    main()
