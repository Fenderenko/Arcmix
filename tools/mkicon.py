#!/usr/bin/env python3
"""Draws the app icon (assets/app.ico + assets/app.png): the app mark — a concave triangle, the "A" of
Arcmix — light on a dark rounded tile (the same mark as the tray icon, rust/src/icon.rs, and the window
header). Every size is rendered separately (SDF), so small sizes stay sharp. No third-party dependencies.
"About" keeps the earlier starry icon: --svg prints it (#aboutMark in web/index.html).
Usage: python3 tools/mkicon.py         — writes the icon files
       python3 tools/mkicon.py --svg   — prints the vector icon for "About"
"""
import math, os, struct, sys, zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]

POW = 1.5  # the sides are superellipse quarters (2 would be a quarter ellipse, deeper)
LAYERS = ['#3B2A9A', '#5631C4', '#7638DD', '#9D44E6', '#C855E2', '#EE74D8', '#FFCDEB']
# wing tip inset, back layer apex and wing height, step between layers, bottom edge bow
BIG = (0.16, 0.13, 0.5, 0.036, 0.08)
PLANET_R = 0.68
STARS = [(0.19, 0.19, 0.022), (0.8, 0.15, 0.016), (0.87, 0.34, 0.02), (0.11, 0.4, 0.016), (0.9, 0.57, 0.013), (0.33, 0.1, 0.012)]


def hexc(h):
    return tuple(int(h[i:i + 2], 16) / 255 for i in (1, 3, 5))


def mix(a, b, t):
    return tuple(x + (y - x) * t for x, y in zip(a, b))


def cov(d, n):
    return min(max(0.5 - d * n, 0.0), 1.0)


def sd_box(px, py, cx, cy, hw, hh, r):
    qx = abs(px - cx) - hw + r
    qy = abs(py - cy) - hh + r
    return (max(qx, 0) ** 2 + max(qy, 0) ** 2) ** 0.5 + min(max(qx, qy), 0) - r


# the mark on a 24-unit box: apex (12, 3), wing tips (3, 20) and (21, 20), control point of the bowed
# bottom (12, 14) — in units of the side: wing inset, apex, wing tips, control point
MARK = (3 / 24, 3 / 24, 20 / 24, 14 / 24)
MARK_Z = 0.8  # size of the mark on the tile
TILE, INK = '#1C1D20', '#E9EAEE'


def sd_mark(x, y, z):
    """Approximate signed distance to the mark scaled by z about the centre (its middle between the apex
    and the wing tips at the centre): inside both sides and above the bottom curve."""
    l, apex, wing, ctl = MARK
    x, y = 0.5 + (x - 0.5) / z, (apex + wing) / 2 + (y - 0.5) / z
    side = (abs(x - 0.5) * (wing - apex) - (y - apex) * (0.5 - l)) / math.hypot(wing - apex, 0.5 - l)
    t = (x - l) / (1 - 2 * l)  # the control point is over the middle: x is linear in the curve parameter
    bottom = wing - 2 * t * (1 - t) * (wing - ctl)
    slope = -2 * (wing - ctl) * (1 - 2 * t) / (1 - 2 * l)
    return max(side, (y - bottom) / math.hypot(1, slope)) * z


def render(n):
    """RGBA rows, top to bottom (straight alpha)."""
    out = bytearray(n * n * 4)
    tile, ink, rim = hexc(TILE), hexc(INK), hexc('#3A3C41')
    for yy in range(n):
        y = (yy + 0.5) / n
        for xx in range(n):
            x = (xx + 0.5) / n
            d = sd_box(x, y, 0.5, 0.5, 0.47, 0.47, 0.215)
            ta = cov(d, n)
            if ta <= 0:
                continue
            c = mix(tile, rim, cov(abs(d + 0.6 / n) - 0.5 / n, n) * 0.6)  # a faint edge on dark taskbars
            c = mix(c, ink, cov(sd_mark(x, y, MARK_Z), n))
            o = (yy * n + xx) * 4
            out[o:o + 4] = bytes([int(v * 255 + 0.5) for v in c] + [int(ta * 255 + 0.5)])
    return out


def svg():
    """The big icon as SVG on a 100 x 100 box, with more stars, a glow behind the layers and air
    above the planet."""
    l, a0, w0, step, sag = BIG
    a, b, k = 0.5 - l, w0 - a0, 24
    f = lambda v: ('%.2f' % (v * 100)).rstrip('0').rstrip('.')
    # one layer at the back position; the others are the same shape moved down
    left = [(l + a * math.cos(t) ** (2 / POW), a0 + b * math.sin(t) ** (2 / POW)) for t in (math.pi / 2 * i / k for i in range(k + 1))]
    pts = left[::-1] + [(1 - x, y) for x, y in left[1:]]  # wing → apex → wing
    tri = 'M' + ' '.join(f(x) + ' ' + f(y) for x, y in pts) + ' Q50 ' + f(w0 - 2 * sag) + ' ' + f(l) + ' ' + f(w0) + 'Z'
    front_bottom = w0 + (len(LAYERS) - 1) * step - sag
    stars = STARS + [(0.62, 0.08, 0.01), (0.07, 0.25, 0.009), (0.94, 0.45, 0.009), (0.25, 0.3, 0.008), (0.72, 0.27, 0.008), (0.45, 0.06, 0.007)]
    # each star twinkles slowly at its own pace (class "tw", the animation is in the page CSS)
    star = ''.join('<path class="tw" style="--o:{o};animation-duration:{dur}s;animation-delay:-{dl}s" d="M{x} {t}Q{x} {y} {r} {y}Q{x} {y} {x} {bt}Q{x} {y} {lf} {y}Q{x} {y} {x} {t}Z"/>'.format(
        x=f(sx), y=f(sy), t=f(sy - r), bt=f(sy + r), r=f(sx + r), lf=f(sx - r), o='%.2f' % (0.55 + 0.4 * min(1, r / 0.02)),
        dur='%.1f' % (3.2 + (i * 7 % 5) * 0.6), dl='%.1f' % (i * 1.7 % 4.5)) for i, (sx, sy, r) in enumerate(stars))
    layers = ''.join('<use href="#amTri" y="%s" fill="%s" fill-opacity="%.2f"/>' % (f(i * step), c, 0.88 + 0.12 * i / (len(LAYERS) - 1)) for i, c in enumerate(LAYERS))
    return ('<svg id="aboutMark" width="64" height="64" viewBox="0 0 100 100" aria-hidden="true">\n'
            '<defs>\n'
            '<clipPath id="amClip"><rect x="3" y="3" width="94" height="94" rx="21.5"/></clipPath>\n'
            '<linearGradient id="amSky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0C0E2C"/><stop offset="1" stop-color="#1E2460"/></linearGradient>\n'
            '<radialGradient id="amLow" cx=".5" cy=".95" r=".6"><stop offset="0" stop-color="#3B3F92" stop-opacity=".45"/><stop offset="1" stop-color="#3B3F92" stop-opacity="0"/></radialGradient>\n'
            '<radialGradient id="amNeb" cx=".72" cy=".22" r=".35"><stop offset="0" stop-color="#7A4BD6" stop-opacity=".16"/><stop offset="1" stop-color="#7A4BD6" stop-opacity="0"/></radialGradient>\n'
            '<linearGradient id="amPlanet" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2D3080"/><stop offset=".45" stop-color="#1F225A"/></linearGradient>\n'
            '<path id="amTri" d="%s"/>\n'
            '<filter id="amBlur" x="-30%%" y="-30%%" width="160%%" height="160%%"><feGaussianBlur stdDeviation="3"/></filter>\n'
            '<filter id="amAir" x="-10%%" y="-30%%" width="120%%" height="160%%"><feGaussianBlur stdDeviation="1.4"/></filter>\n'
            '<filter id="amTile" x="-15%%" y="-15%%" width="130%%" height="135%%"><feDropShadow dx="0" dy="2.5" stdDeviation="2.6" flood-color="#0C0E2C" flood-opacity=".4"/></filter>\n'
            '</defs>\n'
            '<rect x="3" y="3" width="94" height="94" rx="21.5" fill="url(#amSky)" filter="url(#amTile)"/>\n'
            '<g clip-path="url(#amClip)">\n'
            '<rect width="100" height="100" fill="url(#amLow)"/><rect width="100" height="100" fill="url(#amNeb)"/>\n'
            '<g fill="#C9C6F2">%s</g>\n'
            '<use href="#amTri" y="%s" fill="#C855E2" opacity=".45" filter="url(#amBlur)"/>\n'
            '%s\n'
            '<circle cx="50" cy="%s" r="%s" fill="none" stroke="#9A97EA" stroke-width="2.2" stroke-opacity=".55" filter="url(#amAir)"/>\n'
            '<circle cx="50" cy="%s" r="%s" fill="url(#amPlanet)"/>\n'
            '<circle cx="50" cy="%s" r="%s" fill="none" stroke="#8C89E0" stroke-opacity=".35" stroke-width=".6"/>\n'
            '</g>\n'
            '<rect x="3.4" y="3.4" width="93.2" height="93.2" rx="21.1" fill="none" stroke="#fff" stroke-opacity=".08" stroke-width=".8"/>\n'
            '</svg>') % (tri, star, f(3 * step), layers,
                         f(front_bottom + 0.012 + PLANET_R), f(PLANET_R), f(front_bottom + 0.012 + PLANET_R), f(PLANET_R),
                         f(front_bottom + 0.012 + PLANET_R), f(PLANET_R - 0.003))


def png(n, rgba):
    raw = b''.join(b'\0' + bytes(rgba[y * n * 4:(y + 1) * n * 4]) for y in range(n))

    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', n, n, 8, 6, 0, 0, 0)) +
            chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))


def dib(n, rgba):
    """32-bit BMP without the file header (as in ICO resources): BITMAPINFOHEADER, BGRA bottom-up, AND mask."""
    rows = []
    for y in range(n - 1, -1, -1):
        r = rgba[y * n * 4:(y + 1) * n * 4]
        rows.append(bytes(b for i in range(0, len(r), 4) for b in (r[i + 2], r[i + 1], r[i], r[i + 3])))
    bgra = b''.join(rows)
    mask_row = ((n + 31) // 32) * 4
    hdr = struct.pack('<IiiHHIIiiII', 40, n, n * 2, 1, 32, 0, len(bgra) + mask_row * n, 0, 0, 0, 0)
    return hdr + bgra + b'\0' * (mask_row * n)


def main():
    if '--svg' in sys.argv:
        print(svg())
        return
    os.makedirs(os.path.join(ROOT, 'assets'), exist_ok=True)
    imgs = [(n, render(n)) for n in SIZES]
    entries, blobs = [], []
    off = 6 + 16 * len(imgs)
    for n, rgba in imgs:
        data = png(n, rgba) if n >= 256 else dib(n, rgba)
        entries.append(struct.pack('<BBBBHHII', n % 256, n % 256, 0, 0, 1, 32, len(data), off))
        blobs.append(data)
        off += len(data)
    with open(os.path.join(ROOT, 'assets/app.ico'), 'wb') as f:
        f.write(struct.pack('<HHH', 0, 1, len(imgs)) + b''.join(entries) + b''.join(blobs))
    with open(os.path.join(ROOT, 'assets/app.png'), 'wb') as f:
        f.write(png(*imgs[-1]))
    print('assets/app.ico:', len(imgs), 'sizes')


main()
