#!/usr/bin/env python3
"""Build a transparent orb PNG and a legacy multi-size ICO without dependencies."""

import struct
import sys
import zlib
from pathlib import Path


def read_png(path):
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("input is not a PNG")
    pos, compressed = 8, bytearray()
    while pos < len(data):
        length = struct.unpack(">I", data[pos:pos + 4])[0]
        kind = data[pos + 4:pos + 8]
        chunk = data[pos + 8:pos + 8 + length]
        pos += length + 12
        if kind == b"IHDR":
            width, height, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", chunk)
            if depth != 8 or color not in (2, 6) or interlace:
                raise ValueError("expected a non-interlaced 8-bit RGB/RGBA PNG")
        elif kind == b"IDAT":
            compressed.extend(chunk)
        elif kind == b"IEND":
            break

    channels = 3 if color == 2 else 4
    stride = width * channels
    raw = zlib.decompress(compressed)
    rows, previous, offset = [], bytearray(stride), 0
    for _ in range(height):
        filter_type = raw[offset]
        scanline = bytearray(raw[offset + 1:offset + 1 + stride])
        offset += stride + 1
        for x in range(stride):
            left = scanline[x - channels] if x >= channels else 0
            up = previous[x]
            upper_left = previous[x - channels] if x >= channels else 0
            if filter_type == 1:
                scanline[x] = (scanline[x] + left) & 255
            elif filter_type == 2:
                scanline[x] = (scanline[x] + up) & 255
            elif filter_type == 3:
                scanline[x] = (scanline[x] + ((left + up) // 2)) & 255
            elif filter_type == 4:
                estimate = left + up - upper_left
                distances = (abs(estimate - left), abs(estimate - up), abs(estimate - upper_left))
                scanline[x] = (scanline[x] + (left, up, upper_left)[distances.index(min(distances))]) & 255
            elif filter_type != 0:
                raise ValueError(f"unsupported PNG filter: {filter_type}")
        rows.append(scanline)
        previous = scanline
    return width, height, channels, rows


def remove_magenta(width, height, channels, rows):
    pixels = []
    for row in rows:
        output = []
        for x in range(width):
            r, g, b = row[x * channels:x * channels + 3]
            source_alpha = row[x * channels + 3] if channels == 4 else 255
            distance = ((255 - r) ** 2 + g ** 2 + (255 - b) ** 2) ** 0.5
            alpha = 0 if distance <= 55 else 255 if distance >= 130 else round((distance - 55) * 255 / 75)
            alpha = alpha * source_alpha // 255
            if alpha:
                # Remove magenta spill from antialiased boundary pixels.
                key_mix = 1 - alpha / 255
                r = round(max(0, min(255, (r - 255 * key_mix) / (alpha / 255))))
                g = round(max(0, min(255, g / (alpha / 255))))
                b = round(max(0, min(255, (b - 255 * key_mix) / (alpha / 255))))
            output.append((r, g, b, alpha))
        pixels.append(output)
    return pixels


def png_chunk(kind, payload):
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))


def write_png(path, width, height, pixels):
    raw = b"".join(b"\0" + bytes(channel for pixel in row for channel in pixel) for row in pixels)
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", header) + png_chunk(b"IDAT", zlib.compress(raw, 9)) + png_chunk(b"IEND", b""))


def resize_box(pixels, source_width, source_height, size):
    output = []
    for target_y in range(size):
        y0, y1 = target_y * source_height // size, (target_y + 1) * source_height // size
        row = []
        for target_x in range(size):
            x0, x1 = target_x * source_width // size, (target_x + 1) * source_width // size
            samples = [pixels[y][x] for y in range(y0, y1) for x in range(x0, x1)]
            alpha_sum = sum(pixel[3] for pixel in samples)
            alpha = round(alpha_sum / len(samples))
            if alpha_sum:
                rgb = [round(sum(pixel[c] * pixel[3] for pixel in samples) / alpha_sum) for c in range(3)]
            else:
                rgb = [0, 0, 0]
            row.append((*rgb, alpha))
        output.append(row)
    return output


def dib(size, pixels):
    header = struct.pack("<IIIHHIIIIII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0)
    xor = b"".join(bytes((b, g, r, a)) for row in reversed(pixels) for r, g, b, a in row)
    mask_stride = ((size + 31) // 32) * 4
    mask = bytearray()
    for row in reversed(pixels):
        bits = [1 if pixel[3] < 128 else 0 for pixel in row]
        packed = bytearray(mask_stride)
        for index, bit in enumerate(bits):
            packed[index // 8] |= bit << (7 - index % 8)
        mask.extend(packed)
    return header + xor + mask


def write_ico(path, source_width, source_height, pixels):
    sizes = (16, 32, 48)
    images = [dib(size, resize_box(pixels, source_width, source_height, size)) for size in sizes]
    offset = 6 + 16 * len(images)
    entries = bytearray()
    for size, image in zip(sizes, images):
        entries.extend(struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(image), offset))
        offset += len(image)
    Path(path).write_bytes(struct.pack("<HHH", 0, 1, len(images)) + entries + b"".join(images))


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit("usage: build_favicon.py SOURCE.png OUTPUT.png OUTPUT.ico")
    width, height, channels, rows = read_png(sys.argv[1])
    rgba = remove_magenta(width, height, channels, rows)
    write_png(sys.argv[2], width, height, rgba)
    write_ico(sys.argv[3], width, height, rgba)
