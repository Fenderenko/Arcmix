#!/usr/bin/env python3
"""Draws the app icon (assets/app.ico + assets/app.png): a rounded square with a starry sky, layers of
a concave triangle (an arch, the "A" of Arcmix) stepping from violet to pink, and a planet below.
Sizes under 40 px are plainer: three layers, no stars and no planet. Every size is rendered separately
(SDF), so small sizes stay sharp. No third-party dependencies.
Usage: python3 tools/mkicon.py         — writes the icon files
       python3 tools/mkicon.py --svg   — prints the vector icon for "About" (#aboutMark in web/index.html)"""
import math, os, struct, sys, zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]

POW = 1.5  # the sides are superellipse quarters (2 would be a quarter ellipse, deeper)
LAYERS = ['#3B2A9A', '#5631C4', '#7638DD', '#9D44E6', '#C855E2', '#EE74D8', '#FFCDEB']
LAYERS_SMALL = ['#6A36D8', '#C24FE0', '#FFCDEB']
# wing tip inset, back layer apex and wing height, step between layers, bottom edge bow
BIG = (0.16, 0.13, 0.5, 0.036, 0.08)
SMALL = (0.13, 0.15, 0.62, 0.075, 0.08)
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


def sd_tri(x, y, l, apex, wing, sag):
    """Approximate signed distance to one layer: outside both corner superellipses, above the bottom."""
    a, b = 0.5 - l, wing - apex
    d = -1e9
    for cx in (l, 1 - l):
        dx, dy = abs(x - cx) / a, abs(y - apex) / b
        e = dx ** POW + dy ** POW
        g = math.hypot(POW * dx ** (POW - 1) / a, POW * dy ** (POW - 1) / b) or 1e-9
        d = max(d, -(e - 1) / g)
    t = (x - 0.5) / (0.5 - l)
    slope = 2 * sag * t / (0.5 - l)
    bottom = wing - sag * (1 - t * t)
    return max(d, (y - bottom) / math.hypot(1, slope), l - x, x - (1 - l), apex - y)


def render(n):
    """RGBA rows, top to bottom (straight alpha)."""
    small = n < 40
    pal = [hexc(c) for c in (LAYERS_SMALL if small else LAYERS)]
    l, a0, w0, step, sag = SMALL if small else BIG
    front_bottom = w0 + (len(pal) - 1) * step - sag
    pcy = front_bottom + 0.012 + PLANET_R
    out = bytearray(n * n * 4)
    for yy in range(n):
        y = (yy + 0.5) / n
        for xx in range(n):
            x = (xx + 0.5) / n
            ta = cov(sd_box(x, y, 0.5, 0.5, 0.47, 0.47, 0.215), n)
            if ta <= 0:
                continue
            # sky: darker at the top, a faint light low in the middle
            c = mix(hexc('#0C0E2C'), hexc('#1E2460'), y)
            c = mix(c, hexc('#3B3F92'), 0.35 * max(0.0, 1 - math.hypot(x - 0.5, y - 0.95) / 0.6))
            if not small:
                for sx, sy, r in STARS:
                    dx, dy = abs(x - sx), abs(y - sy)
                    d = min(dx / r + dy / (r * 0.22), dx / (r * 0.22) + dy / r) - 1  # a four-point sparkle
                    if d < 0.5:
                        c = mix(c, hexc('#C9C6F2'), cov(d * r * 0.3, n) * 0.85)
            for i, col in enumerate(pal):
                a = cov(sd_tri(x, y, l, a0 + i * step, w0 + i * step, sag), n)
                if a > 0:
                    c = mix(c, col, a * (0.88 + 0.12 * i / (len(pal) - 1)))
            if not small:
                d = math.hypot(x - 0.5, y - pcy) - PLANET_R
                if d > 0:
                    rim = max(0.0, 1 - d / 0.045)  # thin air above the horizon
                    c = mix(c, hexc('#9A97EA'), rim * rim * 0.32)
                pa = cov(d, n)
                if pa > 0:
                    c = mix(c, mix(hexc('#2D3080'), hexc('#1F225A'), min(1.0, max(0.0, (y - front_bottom) / 0.3))), pa)
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
