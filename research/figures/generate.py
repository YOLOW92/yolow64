#!/usr/bin/env python3
"""Generate the public v0.3.0 performance charts and architecture diagrams.

The SVGs are checked into docs/figures/ so they render cleanly on GitHub and crates.io.
Run from anywhere:

    python3 research/figures/generate.py
"""

from __future__ import annotations

import csv
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "figures"
OUT.mkdir(parents=True, exist_ok=True)

# Modern high-contrast palette
BG = "#ffffff"
CARD_BG = "#f8fafc"
PANEL_BG = "#f1f5f9"
BORDER = "#e2e8f0"
BORDER_STRONG = "#cbd5e1"
INK = "#0f172a"
INK_SOFT = "#334155"
MUTED = "#64748b"
GRID = "#e2e8f0"

TEAL = "#0d9488"
TEAL_DARK = "#0f766e"
TEAL_LIGHT = "#ccfbf1"
TEAL_BG = "#f0fdfa"

BLUE = "#2563eb"
BLUE_DARK = "#1d4ed8"
BLUE_LIGHT = "#dbeafe"
BLUE_BG = "#eff6ff"

INDIGO = "#4f46e5"
AMBER = "#d97706"
AMBER_DARK = "#b45309"
AMBER_LIGHT = "#fef3c7"
AMBER_BG = "#fffbeb"

GREEN = "#16a34a"
GREEN_DARK = "#15803d"
GREEN_LIGHT = "#dcfce7"

PURPLE = "#7c3aed"
ROSE = "#e11d48"
SLATE = "#64748b"
SLATE_LIGHT = "#cbd5e1"

FONT = "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif"
MONO = "JetBrains Mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"


def esc(value: str) -> str:
    return (
        value.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def text(
    x: float,
    y: float,
    value: str,
    size: int = 13,
    fill: str = INK,
    anchor: str = "start",
    weight: int = 400,
    family: str = FONT,
    opacity: float = 1.0,
) -> str:
    op = f' opacity="{opacity:.2f}"' if opacity < 0.999 else ""
    return (
        f'<text x="{x:.1f}" y="{y:.1f}" font-family="{family}" '
        f'font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
        f'font-weight="{weight}"{op}>{esc(value)}</text>'
    )


def rect(
    x: float,
    y: float,
    w: float,
    h: float,
    fill: str,
    rx: float = 6,
    stroke: str | None = None,
    sw: float = 1.0,
    opacity: float = 1.0,
    dash: str | None = None,
) -> str:
    border = f' stroke="{stroke}" stroke-width="{sw}"' if stroke else ""
    op = f' opacity="{opacity:.2f}"' if opacity < 0.999 else ""
    ds = f' stroke-dasharray="{dash}"' if dash else ""
    return (
        f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" '
        f'rx="{rx}" fill="{fill}"{border}{ds}{op}/>'
    )


def line(
    x1: float,
    y1: float,
    x2: float,
    y2: float,
    color: str = GRID,
    width: float = 1.0,
    dash: str | None = None,
) -> str:
    ds = f' stroke-dasharray="{dash}"' if dash else ""
    return (
        f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
        f'stroke="{color}" stroke-width="{width}"{ds}/>'
    )


def circle(
    cx: float,
    cy: float,
    r: float,
    fill: str,
    stroke: str | None = None,
    sw: float = 1.5,
) -> str:
    st = f' stroke="{stroke}" stroke-width="{sw}"' if stroke else ""
    return f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{r:.1f}" fill="{fill}"{st}/>'


def arrow(
    x1: float,
    y1: float,
    x2: float,
    y2: float,
    color: str = MUTED,
    width: float = 2.0,
    marker: str = "arrow",
) -> str:
    return (
        f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
        f'stroke="{color}" stroke-width="{width}" marker-end="url(#{marker})"/>'
    )


def badge(
    x: float,
    y: float,
    label: str,
    bg: str = TEAL_LIGHT,
    fg: str = TEAL_DARK,
    w: float | None = None,
    h: float = 22,
    size: int = 11,
    family: str = FONT,
) -> str:
    if w is None:
        w = len(label) * (size * 0.60) + 14
    return rect(x, y, w, h, bg, rx=h / 2) + text(
        x + w / 2, y + h / 2 + size * 0.35, label, size, fg, anchor="middle", weight=700, family=family
    )


def svg_wrap(width: int, height: int, body: str, label: str) -> str:
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="{esc(label)}">
<defs>
  <linearGradient id="grad-teal" x1="0%" y1="0%" x2="100%" y2="0%">
    <stop offset="0%" stop-color="#0f766e"/>
    <stop offset="100%" stop-color="#14b8a6"/>
  </linearGradient>
  <linearGradient id="grad-blue" x1="0%" y1="0%" x2="100%" y2="0%">
    <stop offset="0%" stop-color="#1d4ed8"/>
    <stop offset="100%" stop-color="#3b82f6"/>
  </linearGradient>
  <linearGradient id="grad-amber" x1="0%" y1="0%" x2="100%" y2="0%">
    <stop offset="0%" stop-color="#b45309"/>
    <stop offset="100%" stop-color="#f59e0b"/>
  </linearGradient>
  <linearGradient id="grad-slate" x1="0%" y1="0%" x2="100%" y2="0%">
    <stop offset="0%" stop-color="#64748b"/>
    <stop offset="100%" stop-color="#94a3b8"/>
  </linearGradient>
  <linearGradient id="grad-header" x1="0%" y1="0%" x2="100%" y2="0%">
    <stop offset="0%" stop-color="#0f172a"/>
    <stop offset="100%" stop-color="#1e293b"/>
  </linearGradient>
  <marker id="arrow" viewBox="0 0 10 10" refX="8.5" refY="5" markerWidth="6.5" markerHeight="6.5" orient="auto-start-reverse">
    <path d="M0,1 L9,5 L0,9 z" fill="{MUTED}"/>
  </marker>
  <marker id="arrow-teal" viewBox="0 0 10 10" refX="8.5" refY="5" markerWidth="6.5" markerHeight="6.5" orient="auto-start-reverse">
    <path d="M0,1 L9,5 L0,9 z" fill="{TEAL_DARK}"/>
  </marker>
  <marker id="arrow-blue" viewBox="0 0 10 10" refX="8.5" refY="5" markerWidth="6.5" markerHeight="6.5" orient="auto-start-reverse">
    <path d="M0,1 L9,5 L0,9 z" fill="{BLUE_DARK}"/>
  </marker>
</defs>
<rect width="100%" height="100%" rx="14" fill="{BG}" stroke="{BORDER}" stroke-width="1.5"/>
{body}
</svg>
"""


def header_band(
    width: int,
    title: str,
    subtitle: str,
    pills: list[tuple[str, str, str]] | None = None,
) -> str:
    body = rect(16, 16, width - 32, 74, "url(#grad-header)", rx=10)
    body += text(34, 46, title, 19, "#ffffff", weight=700)
    body += text(34, 70, subtitle, 12, "#cbd5e1", weight=400)
    if pills:
        px = width - 32
        for label, bg, fg in reversed(pills):
            pw = len(label) * 6.3 + 20
            px -= pw
            body += rect(px, 30, pw, 26, bg, rx=13)
            body += text(px + pw / 2, 47, label, 11, fg, anchor="middle", weight=700)
            px -= 8
    return body


def read_csv(name: str) -> list[dict[str, str]]:
    with (ROOT / "benchmarks" / "results" / name).open(newline="") as handle:
        return list(csv.DictReader(handle))


def hero_benchmark_chart() -> None:
    width, height = 1060, 520
    body = header_band(
        width,
        "yolow64 v0.3.0 — Performance Overview",
        "Intel Core i7-10700 (AVX2 + BMI2) · Rust 1.99 fat LTO · 100% Block Validation",
        [
            ("#1 Short Decode (9/9)", "#0f766e", "#ccfbf1"),
            ("#1 Full-Check Decode (6/6)", "#1d4ed8", "#dbeafe"),
            ("22.5 GB/s Encode", "#b45309", "#fef3c7"),
        ],
    )

    # Left panel: 1 MiB Bulk Encode & Full-Validation Decode (GB/s)
    lx, ly, lw, lh = 16, 104, 504, 376
    body += rect(lx, ly, lw, lh, CARD_BG, rx=10, stroke=BORDER)
    body += text(lx + 20, ly + 28, "1 MiB Bulk Throughput (GB/s)", 15, INK, weight=700)
    body += text(lx + 20, ly + 46, "Higher is better · Decode uses 100% validation (FULLCHECK=1)", 11, MUTED)

    # Legend in left panel
    body += rect(lx + lw - 158, ly + 16, 12, 12, "url(#grad-teal)", rx=3)
    body += text(lx + lw - 140, ly + 26, "Decode", 11, INK_SOFT, weight=600)
    body += rect(lx + lw - 84, ly + 16, 12, 12, "url(#grad-blue)", rx=3)
    body += text(lx + lw - 66, ly + 26, "Encode", 11, INK_SOFT, weight=600)

    libs_1m = [
        ("yolow64 v0.3", 20.40, 22.27, True),
        ("Turbo-Base64 (full)", 16.68, 21.04, False),
        ("encodify", 16.69, 19.41, False),
        ("base64-turbo", 16.50, 19.43, False),
        ("simdutf", 13.34, 17.10, False),
        ("base64 (SIMD)", 11.67, 9.51, False),
        ("base64-simd", 7.82, 10.38, False),
        ("base64 (scalar)", 2.07, 2.51, False),
    ]

    bar_x = lx + 148
    max_w = lw - 210
    max_gbps = 24.0
    start_y = ly + 66
    row_h = 37

    for g in [0, 5, 10, 15, 20]:
        gx = bar_x + (g / max_gbps) * max_w
        body += line(gx, start_y - 6, gx, start_y + len(libs_1m) * row_h - 4, GRID, 1, "3,3")
        body += text(gx, start_y + len(libs_1m) * row_h + 10, f"{g}", 10, MUTED, anchor="middle", family=MONO)

    for idx, (name, dec_v, enc_v, is_hi) in enumerate(libs_1m):
        ry = start_y + idx * row_h
        if is_hi:
            body += rect(lx + 8, ry - 3, lw - 16, row_h - 2, TEAL_BG, rx=6, stroke=TEAL, sw=1.2)
        body += text(
            bar_x - 10,
            ry + 18,
            name,
            12,
            TEAL_DARK if is_hi else INK_SOFT,
            anchor="end",
            weight=700 if is_hi else 500,
        )
        dw = (dec_v / max_gbps) * max_w
        body += rect(
            bar_x,
            ry + 2,
            dw,
            12,
            "url(#grad-teal)" if is_hi else "#0d9488",
            rx=3,
            opacity=1.0 if is_hi else 0.72,
        )
        body += text(
            bar_x + dw + 6,
            ry + 12,
            f"{dec_v:.1f}",
            10,
            TEAL_DARK if is_hi else INK_SOFT,
            weight=700 if is_hi else 500,
            family=MONO,
        )
        ew = (enc_v / max_gbps) * max_w
        body += rect(
            bar_x,
            ry + 16,
            ew,
            12,
            "url(#grad-blue)" if is_hi else "#64748b",
            rx=3,
            opacity=1.0 if is_hi else 0.55,
        )
        body += text(
            bar_x + ew + 6,
            ry + 26,
            f"{enc_v:.1f}",
            10,
            BLUE_DARK if is_hi else MUTED,
            weight=700 if is_hi else 500,
            family=MONO,
        )

    # Right panel: Short Decode Latency Trajectory (8 B – 128 B)
    rx, ry, rw, rh = 536, 104, 508, 376
    body += rect(rx, ry, rw, rh, CARD_BG, rx=10, stroke=BORDER)
    body += text(rx + 20, ry + 28, "Short Decode Latency (8 B – 128 B)", 15, INK, weight=700)
    body += text(rx + 20, ry + 46, "ns/call, lower is better · yolow64 v0.3 wins all 9 sizes", 11, MUTED)

    short_sizes = [8, 12, 16, 24, 32, 48, 64, 96, 128]
    short_rows = read_csv("8way_short_all.csv")
    s_data: dict[str, dict[int, float]] = {}
    for row in short_rows:
        s_data.setdefault(row["lib"], {})[int(row["size"])] = float(row["median_ns"])

    px0, py0 = rx + 48, ry + 72
    pw, ph = rw - 72, 248
    max_ns = 30.0

    for ns_val in [0, 5, 10, 15, 20, 25, 30]:
        gy = py0 + ph - (ns_val / max_ns) * ph
        body += line(px0, gy, px0 + pw, gy, GRID, 1, "3,3" if ns_val > 0 else None)
        body += text(px0 - 8, gy + 4, f"{ns_val}", 10, MUTED, anchor="end", family=MONO)

    xs = [px0 + i * (pw / (len(short_sizes) - 1)) for i in range(len(short_sizes))]
    for i, sz in enumerate(short_sizes):
        body += line(xs[i], py0, xs[i], py0 + ph, GRID, 0.7)
        body += text(xs[i], py0 + ph + 18, f"{sz}B", 11, INK_SOFT, anchor="middle", weight=600, family=MONO)

    series = [
        ("encodify", "#94a3b8", 1.8, "4,3"),
        ("base64-turbo", "#f59e0b", 2.0, None),
        ("base64-simd", "#6366f1", 2.0, None),
        ("turbo-base64", "#2563eb", 2.4, None),
        ("yolow64", "#0f766e", 3.4, None),
    ]

    for lib_key, color, sw, dash in series:
        pts: list[tuple[float, float, float]] = []
        for i, sz in enumerate(short_sizes):
            v = s_data[lib_key][sz]
            vc = min(v, max_ns)
            y_pt = py0 + ph - (vc / max_ns) * ph
            pts.append((xs[i], y_pt, v))
        path_d = " ".join(
            f"{'M' if i == 0 else 'L'}{x:.1f},{y:.1f}" for i, (x, y, _) in enumerate(pts)
        )
        ds = f' stroke-dasharray="{dash}"' if dash else ""
        body += f'<path d="{path_d}" fill="none" stroke="{color}" stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round"{ds}/>'
        for i, (x, y, v) in enumerate(pts):
            r_dot = 4.2 if lib_key == "yolow64" else 2.8
            body += circle(x, y, r_dot, color, "#ffffff", 1.2)
            if lib_key == "yolow64" and short_sizes[i] in (8, 16, 32, 64, 128):
                body += text(x, y + 15, f"{v:.1f}", 10, TEAL_DARK, anchor="middle", weight=700, family=MONO)

    leg_items = [
        ("yolow64 v0.3 (#1)", "#0f766e"),
        ("Turbo-Base64", "#2563eb"),
        ("base64-simd", "#6366f1"),
        ("base64-turbo", "#f59e0b"),
        ("encodify", "#94a3b8"),
    ]
    lx_cur = rx + 24
    ly_leg = ry + rh - 18
    for label, col in leg_items:
        body += circle(lx_cur + 5, ly_leg - 4, 4.5, col)
        body += text(lx_cur + 14, ly_leg, label, 11, INK_SOFT, weight=600 if "yolow64" in label else 500)
        lx_cur += len(label) * 6.5 + 24

    body += text(
        24,
        height - 16,
        "Source: benchmarks/results/8way_decode_all.csv, 8way_short_all.csv, 8way_encode_final.csv (6-process medians).",
        11,
        MUTED,
    )
    (OUT / "hero-benchmark.svg").write_text(svg_wrap(width, height, body, "yolow64 v0.3.0 hero benchmark"))


def short_all_chart() -> None:
    width, height = 1060, 620
    rows = read_csv("8way_short_all.csv")
    sizes = [8, 12, 16, 24, 32, 48, 64, 96, 128]
    data: dict[str, dict[int, float]] = {}
    for row in rows:
        data.setdefault(row["lib"], {})[int(row["size"])] = float(row["median_ns"])

    body = header_band(
        width,
        "Short Decode Latency — 8 B to 128 B",
        "ns/call (lower is better) · Six-process medians · yolow64 v0.3 ranks #1 in 9 out of 9 sizes",
        [
            ("8B: 5.47 ns (2.6×)", "#0f766e", "#ccfbf1"),
            ("32B: 6.89 ns (2.8×)", "#1d4ed8", "#dbeafe"),
            ("64B: 10.96 ns (2.1×)", "#b45309", "#fef3c7"),
        ],
    )

    sx, sy, sw, sh = 16, 104, width - 32, 174
    body += rect(sx, sy, sw, sh, CARD_BG, rx=10, stroke=BORDER)
    body += text(
        sx + 20,
        sy + 26,
        "v0.3.0 Padded Short Kernel Spotlight — Latency vs Fastest Competitor (ns/call)",
        14,
        INK,
        weight=700,
    )
    body += text(
        sx + 20,
        sy + 44,
        "Dedicated decode12 (XMM), decode24 (YMM), decode44 (YMM+XMM), decode88 (2×YMM+XMM+pext6), and decode_hash4_tail44 (128B)",
        11,
        MUTED,
    )

    spotlight = [
        (8, "8 B (12c)", "decode12", "Turbo-Base64", data["turbo-base64"][8]),
        (16, "16 B (24c)", "decode24", "Turbo-Base64", data["turbo-base64"][16]),
        (32, "32 B (44c)", "decode44", "base64-simd", data["base64-simd"][32]),
        (64, "64 B (88c)", "decode88", "Turbo-Base64", data["turbo-base64"][64]),
        (128, "128 B (172c)", "tail44", "Turbo-Base64", data["turbo-base64"][128]),
    ]

    card_w = (sw - 40 - 4 * 12) / 5
    for idx, (sz, title_lbl, kernel_lbl, comp_name, comp_ns) in enumerate(spotlight):
        cx = sx + 20 + idx * (card_w + 12)
        cy = sy + 56
        ch = 104
        y_ns = data["yolow64"][sz]
        body += rect(cx, cy, card_w, ch, "#ffffff", rx=8, stroke=BORDER_STRONG)
        body += text(cx + 12, cy + 20, title_lbl, 12, INK, weight=700)
        body += badge(cx + card_w - 74, cy + 6, kernel_lbl, TEAL_LIGHT, TEAL_DARK, w=66, h=18, size=9, family=MONO)

        max_local = max(comp_ns * 1.15, 16.0)
        bw_max = card_w - 74
        yw = (y_ns / max_local) * bw_max
        body += text(cx + 12, cy + 44, "yolow64", 10, TEAL_DARK, weight=700)
        body += rect(cx + 12, cy + 49, yw, 12, "url(#grad-teal)", rx=3)
        body += text(cx + 16 + yw, cy + 59, f"{y_ns:.2f} ns", 10, TEAL_DARK, weight=700, family=MONO)

        cw = (comp_ns / max_local) * bw_max
        body += text(cx + 12, cy + 75, comp_name, 10, MUTED, weight=500)
        body += rect(cx + 12, cy + 80, cw, 11, SLATE_LIGHT, rx=3)
        body += text(cx + 16 + cw, cy + 90, f"{comp_ns:.2f} ns", 10, INK_SOFT, weight=600, family=MONO)

    mx, my, mw, mh = 16, 290, width - 32, 294
    body += rect(mx, my, mw, mh, CARD_BG, rx=10, stroke=BORDER)

    names = {
        "yolow64": "yolow64 v0.3",
        "turbo-base64": "Turbo-Base64 (default)",
        "base64-simd": "base64-simd",
        "base64-turbo": "base64-turbo",
        "encodify": "encodify",
        "simdutf": "simdutf",
        "base64-simd-engine": "base64 (SIMD engine)",
        "base64-scalar": "base64 (scalar)",
    }
    ordered = [
        "yolow64",
        "turbo-base64",
        "base64-simd",
        "base64-turbo",
        "encodify",
        "simdutf",
        "base64-simd-engine",
        "base64-scalar",
    ]

    left_col = mx + 186
    col_w = (mw - 198) / len(sizes)
    top_y = my + 38
    row_h = 30

    for c_idx, sz in enumerate(sizes):
        cx = left_col + c_idx * col_w
        body += text(cx + col_w / 2, top_y - 12, f"{sz} B", 12, INK, anchor="middle", weight=700, family=MONO)

    col_mins = {sz: min(data[lib][sz] for lib in ordered) for sz in sizes}
    col_maxs = {sz: max(data[lib][sz] for lib in ordered) for sz in sizes}

    for r_idx, lib in enumerate(ordered):
        ry = top_y + r_idx * row_h
        is_hi = lib == "yolow64"
        if is_hi:
            body += rect(mx + 8, ry, mw - 16, row_h - 2, TEAL_BG, rx=6, stroke=TEAL, sw=1.2)
        elif r_idx % 2 == 1:
            body += rect(mx + 8, ry, mw - 16, row_h - 2, "#ffffff", rx=4)

        body += text(
            left_col - 14,
            ry + 19,
            names[lib],
            12,
            TEAL_DARK if is_hi else INK_SOFT,
            anchor="end",
            weight=700 if is_hi else 500,
        )

        for c_idx, sz in enumerate(sizes):
            val = data[lib][sz]
            cx = left_col + c_idx * col_w + 4
            cw = col_w - 8
            is_best = abs(val - col_mins[sz]) < 1e-6

            body += rect(cx, ry + 3, cw, row_h - 8, "#ffffff" if is_hi else PANEL_BG, rx=4, stroke=TEAL if is_best else None, sw=1.0)
            ratio = min(1.0, val / col_maxs[sz])
            bar_fill = "#99f6e4" if is_best else ("#bfdbfe" if lib == "turbo-base64" else "#e2e8f0")
            body += rect(cx + 1, ry + 4, max(4.0, (cw - 2) * ratio), row_h - 10, bar_fill, rx=3)
            body += text(
                cx + cw / 2,
                ry + 19,
                f"{val:.2f}",
                11,
                TEAL_DARK if is_best else INK,
                anchor="middle",
                weight=700 if is_best else 500,
                family=MONO,
            )

    body += text(
        24,
        height - 14,
        "Note: yolow64 validates 100% of blocks and padding bits. Turbo-Base64 short rows use its default build. Source: benchmarks/results/8way_short_all.csv.",
        11,
        MUTED,
    )
    (OUT / "short-all.svg").write_text(svg_wrap(width, height, body, "Short decode latency across all 9 sizes"))


def decode_all_chart() -> None:
    width, height = 1060, 540
    rows = read_csv("8way_decode_all.csv")
    fullcheck = {
        int(row["size"]): float(row["gbps"])
        for row in read_csv("turbo_check_verify.csv")
        if row["mode"] == "direct" and row["build"] == "fullcheck"
    }
    sizes = [8192, 16384, 65536, 262144, 1048576, 4194304]
    col_labels = ["8 KiB", "16 KiB", "64 KiB", "256 KiB", "1 MiB", "4 MiB"]

    data: dict[str, dict[int, float]] = {}
    for row in rows:
        data.setdefault(row["lib"], {})[int(row["size"])] = float(row["gbps"])
    for sz in sizes:
        data.setdefault("turbo-base64", {})[sz] = fullcheck[sz]

    body = header_band(
        width,
        "Bulk Decode Throughput — 8 KiB to 4 MiB",
        "GB/s (higher is better) · Six-process medians · yolow64 v0.3 leads all 6 sizes under 100% validation",
        [
            ("8 KiB: 20.41 GB/s (#1)", "#0f766e", "#ccfbf1"),
            ("64 KiB: 20.36 GB/s (#1)", "#1d4ed8", "#dbeafe"),
            ("1 MiB: 20.40 GB/s (#1)", "#b45309", "#fef3c7"),
        ],
    )

    lx, ly, lw, lh = 16, 104, 440, 396
    body += rect(lx, ly, lw, lh, CARD_BG, rx=10, stroke=BORDER)
    body += text(lx + 20, ly + 28, "Throughput Trajectory (8 KiB – 4 MiB)", 15, INK, weight=700)
    body += text(lx + 20, ly + 46, "Sustained >20.3 GB/s across L1/L2/L3 cache sizes", 11, MUTED)

    px0, py0 = lx + 46, ly + 68
    pw, ph = lw - 68, 268
    max_g = 22.0

    for g in [0, 5, 10, 15, 20]:
        gy = py0 + ph - (g / max_g) * ph
        body += line(px0, gy, px0 + pw, gy, GRID, 1, "3,3" if g > 0 else None)
        body += text(px0 - 8, gy + 4, f"{g}", 10, MUTED, anchor="end", family=MONO)

    xs = [px0 + i * (pw / (len(sizes) - 1)) for i in range(len(sizes))]
    for i, lbl in enumerate(col_labels):
        body += line(xs[i], py0, xs[i], py0 + ph, GRID, 0.7)
        body += text(xs[i], py0 + ph + 18, lbl, 10, INK_SOFT, anchor="middle", weight=600, family=MONO)

    line_series = [
        ("base64-scalar", "base64 (scalar)", "#94a3b8", 1.8),
        ("base64-simd", "base64-simd", "#64748b", 1.8),
        ("base64-simd-engine", "base64 (SIMD)", "#8b5cf6", 2.0),
        ("simdutf", "simdutf", "#f59e0b", 2.0),
        ("base64-turbo", "base64-turbo", "#ec4899", 2.0),
        ("encodify", "encodify", "#3b82f6", 2.2),
        ("turbo-base64", "Turbo (FULLCHECK)", "#1d4ed8", 2.5),
        ("yolow64", "yolow64 v0.3", "#0f766e", 3.5),
    ]

    for lib_key, _, col, sw in line_series:
        pts = [(xs[i], py0 + ph - (data[lib_key][sz] / max_g) * ph) for i, sz in enumerate(sizes)]
        path_d = " ".join(f"{'M' if i == 0 else 'L'}{x:.1f},{y:.1f}" for i, (x, y) in enumerate(pts))
        body += f'<path d="{path_d}" fill="none" stroke="{col}" stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round"/>'
        for x, y in pts:
            body += circle(x, y, 4.0 if lib_key == "yolow64" else 2.6, col, "#ffffff", 1.0)

    leg_libs = [
        ("yolow64 v0.3", "#0f766e"),
        ("Turbo (FULLCHECK)", "#1d4ed8"),
        ("encodify", "#3b82f6"),
        ("simdutf", "#f59e0b"),
        ("base64 (SIMD)", "#8b5cf6"),
    ]
    lx_c = lx + 18
    for lbl, col in leg_libs:
        body += circle(lx_c + 4, ly + lh - 18, 4, col)
        body += text(lx_c + 12, ly + lh - 14, lbl, 10, INK_SOFT, weight=600 if "yolow64" in lbl else 500)
        lx_c += len(lbl) * 5.8 + 20

    rx, ry, rw, rh = 472, 104, 572, 396
    body += rect(rx, ry, rw, rh, CARD_BG, rx=10, stroke=BORDER)
    body += text(rx + 20, ry + 28, "All 8 Libraries — Recorded Medians (GB/s)", 15, INK, weight=700)
    body += text(rx + 20, ry + 46, "Fused decode_hash4_tail44 (8 KiB) & decode_hash4_tail88 (16 KiB–4 MiB)", 11, MUTED)

    ordered = [
        "yolow64",
        "turbo-base64",
        "encodify",
        "base64-turbo",
        "simdutf",
        "base64-simd-engine",
        "base64-simd",
        "base64-scalar",
    ]
    names = {
        "yolow64": "yolow64 v0.3",
        "turbo-base64": "Turbo (FULLCHECK)",
        "encodify": "encodify",
        "base64-turbo": "base64-turbo",
        "simdutf": "simdutf",
        "base64-simd-engine": "base64 (SIMD)",
        "base64-simd": "base64-simd",
        "base64-scalar": "base64 (scalar)",
    }

    left_col = rx + 150
    col_w = (rw - 162) / len(sizes)
    top_y = ry + 78
    row_h = 37

    for c_idx, lbl in enumerate(col_labels):
        cx = left_col + c_idx * col_w
        body += text(cx + col_w / 2, top_y - 12, lbl, 11, INK, anchor="middle", weight=700, family=MONO)

    col_maxs = {sz: max(data[lib][sz] for lib in ordered) for sz in sizes}

    for r_idx, lib in enumerate(ordered):
        y = top_y + r_idx * row_h
        is_hi = lib == "yolow64"
        if is_hi:
            body += rect(rx + 8, y, rw - 16, row_h - 3, TEAL_BG, rx=6, stroke=TEAL, sw=1.2)
        elif r_idx % 2 == 1:
            body += rect(rx + 8, y, rw - 16, row_h - 3, "#ffffff", rx=4)

        body += text(
            left_col - 10,
            y + 22,
            names[lib],
            11,
            TEAL_DARK if is_hi else INK_SOFT,
            anchor="end",
            weight=700 if is_hi else 500,
        )

        for c_idx, sz in enumerate(sizes):
            val = data[lib][sz]
            cx = left_col + c_idx * col_w + 3
            cw = col_w - 6
            is_best = abs(val - col_maxs[sz]) < 1e-6
            body += rect(cx, y + 4, cw, row_h - 11, "#ffffff" if is_hi else PANEL_BG, rx=4, stroke=TEAL if is_best else None, sw=1.0)
            ratio = val / 21.0
            fill = "#99f6e4" if is_best else ("#dbeafe" if lib == "turbo-base64" else "#e2e8f0")
            body += rect(cx + 1, y + 5, max(3.0, (cw - 2) * ratio), row_h - 13, fill, rx=3)
            body += text(
                cx + cw / 2,
                y + 22,
                f"{val:.2f}",
                11,
                TEAL_DARK if is_best else INK,
                anchor="middle",
                weight=700 if is_best else 500,
                family=MONO,
            )

    body += text(
        24,
        height - 14,
        "Turbo (FULLCHECK) = direct-kernel FULLCHECK=1 build (100% block validation). Default 25%-check build is documented in research/turbo-check.md.",
        11,
        MUTED,
    )
    (OUT / "decode-all.svg").write_text(svg_wrap(width, height, body, "Bulk decode throughput across all 8 libraries"))


def encode_all_chart() -> None:
    width, height = 1060, 672
    rows = [
        row
        for row in read_csv("encode_all_combined.csv")
        if row["mode"] == "large" and row["kind"] in ("slice", "alloc")
    ]
    data: dict[str, dict[int, float]] = {}
    kinds: dict[str, str] = {}
    for row in rows:
        lib = row["lib"]
        data.setdefault(lib, {})[int(row["size"])] = float(row["gbps"])
        kinds[lib] = row["kind"]

    ordered = sorted(data, key=lambda lib: data[lib].get(1048576, 0), reverse=True)

    body = header_band(
        width,
        "22-Library Encode Leaderboard (1 MiB & 64 KiB)",
        "GB/s (higher is better) · Standard-alphabet campaign across Rust crates and C/C++ bindings",
        [
            ("#1 Rust Crate at 1 MiB (21.75 GB/s)", "#0f766e", "#ccfbf1"),
            ("22.5 GB/s post-tuning", "#1d4ed8", "#dbeafe"),
        ],
    )

    px, py, pw, ph = 16, 104, width - 32, 528
    body += rect(px, py, pw, ph, CARD_BG, rx=10, stroke=BORDER)

    body += rect(px + pw - 260, py + 14, 14, 10, "url(#grad-teal)", rx=2)
    body += text(px + pw - 240, py + 23, "1 MiB (solid)", 11, INK_SOFT, weight=600)
    body += rect(px + pw - 140, py + 14, 14, 10, "#93c5fd", rx=2)
    body += text(px + pw - 120, py + 23, "64 KiB (marker)", 11, INK_SOFT, weight=600)
    body += text(px + 20, py + 24, "Library (sorted by 1 MiB throughput)", 13, INK, weight=700)

    bar_x = px + 230
    max_w = pw - 370
    max_g = 25.0
    start_y = py + 42
    row_h = 22.0

    for g in [0, 5, 10, 15, 20, 25]:
        gx = bar_x + (g / max_g) * max_w
        body += line(gx, start_y - 6, gx, start_y + len(ordered) * row_h, GRID, 1, "3,3")
        body += text(gx, start_y + len(ordered) * row_h + 15, f"{g} GB/s", 10, MUTED, anchor="middle", family=MONO)

    for idx, lib in enumerate(ordered):
        y = start_y + idx * row_h
        v_1m = data[lib].get(1048576, 0.0)
        v_64k = data[lib].get(65536, 0.0)
        is_hi = lib == "yolow64"
        is_alloc = kinds.get(lib) == "alloc"
        label = f"#{idx + 1}  {lib}" + (" (alloc)" if is_alloc else "")

        if is_hi:
            body += rect(px + 10, y - 1, pw - 20, row_h - 1, TEAL_BG, rx=4, stroke=TEAL, sw=1.2)

        body += text(
            bar_x - 12,
            y + 15,
            label,
            11,
            TEAL_DARK if is_hi else INK_SOFT,
            anchor="end",
            weight=700 if is_hi else 500,
            family=MONO,
        )

        w64 = (v_64k / max_g) * max_w
        body += rect(bar_x, y + 3, w64, 14, "#bfdbfe" if is_hi else "#e2e8f0", rx=3)

        w1m = (v_1m / max_g) * max_w
        fill = "url(#grad-teal)" if is_hi else ("url(#grad-blue)" if v_1m >= 15.0 else "url(#grad-slate)")
        body += rect(bar_x, y + 5, w1m, 10, fill, rx=3)

        val_str = f"{v_1m:5.2f} GB/s  (64K: {v_64k:5.2f})"
        body += text(
            bar_x + max(w1m, w64) + 8,
            y + 15,
            val_str,
            10,
            TEAL_DARK if is_hi else INK_SOFT,
            weight=700 if is_hi else 500,
            family=MONO,
        )

    body += text(
        24,
        height - 14,
        "Note: URL-only base64-url (2.52 GB/s at 1 MiB) is in encode_all_combined.csv. Post-campaign 8/16/32B & NT tuning raised yolow64 to 22.5 GB/s.",
        11,
        MUTED,
    )
    (OUT / "encode-all.svg").write_text(svg_wrap(width, height, body, "22-library encode throughput leaderboard"))


def decode_pipeline() -> None:
    width, height = 1060, 560
    body = header_band(
        width,
        "v0.3.0 Decode Pipeline & Exact-Length SIMD Kernels",
        "Unified clean-room AVX2 hash network · Speculative validation · Dedicated short & fused tail kernels",
        [
            ("YMM 256-bit", "#0f766e", "#ccfbf1"),
            ("XMM 128-bit", "#1d4ed8", "#dbeafe"),
            ("BMI2 PEXT", "#b45309", "#fef3c7"),
        ],
    )

    # Section 1: Bulk 4-Block Assembly Hash Loop + Fused Tail Kernels
    s1_y = 104
    body += rect(16, s1_y, width - 32, 168, CARD_BG, rx=10, stroke=BORDER)
    body += badge(32, s1_y + 14, "TIER 1 · BULK & LARGE PAYLOADS (≥ 128 chars)", TEAL_LIGHT, TEAL_DARK, w=320, h=22)
    body += text(364, s1_y + 29, "Zero constant-memory loads inside hot loop; all 7 LUT/pack vectors stay in YMM registers", 12, MUTED)

    stages = [
        ("1. 4×32B Load", "128 Base64 chars", "4× vmovdqu (YMM)", "Prefetch +1024B"),
        ("2. Hash Translate", "vpsrld + vpavgb", "delta_asso / values", "vpaddb 6-bit indices"),
        ("3. Speculative Check", "check_asso / values", "vpaddsb < 0 → vpor", "Single ymm10 accum"),
        ("4. Pack 96 Bytes", "vpmaddubsw → wd", "vpshufb 12B lanes", "8× 16B stores/iter"),
        ("5. Fused Tail", "tail44 (8K, 128B)", "tail88 (16K–4M)", "Single loop exit check"),
    ]
    box_w = 182
    for i, (t1, l1, l2, l3) in enumerate(stages):
        bx = 32 + i * 202
        by = s1_y + 48
        body += rect(bx, by, box_w, 104, "#ffffff", rx=8, stroke=TEAL if i < 4 else BLUE, sw=1.5)
        body += rect(bx, by, box_w, 26, TEAL_BG if i < 4 else BLUE_BG, rx=8)
        body += text(bx + box_w / 2, by + 18, t1, 12, TEAL_DARK if i < 4 else BLUE_DARK, anchor="middle", weight=700)
        body += text(bx + box_w / 2, by + 46, l1, 11, INK, anchor="middle", weight=600, family=MONO)
        body += text(bx + box_w / 2, by + 68, l2, 11, INK_SOFT, anchor="middle", family=MONO)
        body += text(bx + box_w / 2, by + 90, l3, 11, MUTED, anchor="middle")
        if i < 4:
            body += arrow(bx + box_w + 2, by + 52, bx + 200, by + 52, TEAL_DARK, 2.0, "arrow-teal")

    # Section 2: v0.3.0 Dedicated Padded Short Kernels
    s2_y = 286
    body += rect(16, s2_y, width - 32, 152, CARD_BG, rx=10, stroke=BORDER)
    body += badge(32, s2_y + 14, "TIER 2 · v0.3.0 DEDICATED PADDED SHORT KERNELS", BLUE_LIGHT, BLUE_DARK, w=340, h=22)
    body += text(384, s2_y + 29, "Eliminates multi-step helper calls and scalar tail bottlenecks for 8 B, 16 B, 32 B, and 64 B", 12, MUTED)

    short_kernels = [
        ("decode12 · 8 B", "XMM 128b", "12 chars: u64+u16+u8+'A'", "1× XMM hash/validate/pack", "_mm_extract_epi8::<8> → 5.47 ns"),
        ("decode24 · 16 B", "YMM 256b", "24 chars: 0..16 & 8..24 ('AA')", "Single 256-bit AVX2 pass", "_mm_extract_epi8::<4> → 5.69 ns"),
        ("decode44 · 32 B", "YMM + XMM", "44 chars: YMM 0..32 + XMM 28..44", "Shared LUT regs + 1× vptest", "Contiguous 24B + 8B → 6.89 ns"),
        ("decode88 · 64 B", "2×YMM+PEXT", "88 chars: 2×YMM + XMM 64..80", "BMI2 pext6_be on 80..86", "Single fused fn → 10.96 ns"),
    ]
    sk_w = 236
    for i, (title_k, reg_badge, line1, line2, line3) in enumerate(short_kernels):
        kx = 32 + i * 250
        ky = s2_y + 46
        body += rect(kx, ky, sk_w, 94, "#ffffff", rx=8, stroke=BLUE, sw=1.4)
        body += text(kx + 12, ky + 20, title_k, 12, BLUE_DARK, weight=700)
        bw = len(reg_badge) * 5.6 + 14
        body += badge(kx + sk_w - bw - 8, ky + 6, reg_badge, AMBER_LIGHT, AMBER_DARK, w=bw, h=18, size=9, family=MONO)
        body += text(kx + 12, ky + 44, line1, 10, INK, family=MONO)
        body += text(kx + 12, ky + 63, line2, 10, INK_SOFT, family=MONO)
        body += text(kx + 12, ky + 82, line3, 10, TEAL_DARK, weight=700)

    # Section 3: Unpadded Exact-Length Fast Paths & Validation Contract
    s3_y = 450
    body += rect(16, s3_y, width - 32, 72, CARD_BG, rx=10, stroke=BORDER)
    body += badge(32, s3_y + 14, "TIER 3 · UNPADDED FAST PATHS", AMBER_LIGHT, AMBER_DARK, w=230, h=22)
    body += text(
        276,
        s3_y + 29,
        "16c (12B: 5.04 ns) · 32c (24B: 7.87 ns) · 64c (48B: 9.69 ns) · 128c (96B: 13.02 ns)",
        11,
        INK,
        weight=600,
        family=MONO,
    )
    body += text(
        32,
        s3_y + 54,
        "Strict Validation Contract: Every path validates 100% of blocks, rejects non-zero trailing padding bits, and reports the exact InvalidByte offset.",
        12,
        INK_SOFT,
    )

    body += text(24, height - 12, "yolow64 v0.3.0 · Clean-room SIMD architecture (src/avx2/decode.rs)", 11, MUTED)
    (OUT / "decode-pipeline.svg").write_text(svg_wrap(width, height, body, "yolow64 v0.3.0 decode pipeline diagram"))


def cpu_dispatch() -> None:
    width, height = 1060, 460
    body = header_band(
        width,
        "Runtime CPU Feature Dispatch & Fallback Contract",
        "AVX2 and BMI2 are detected independently via OnceLock · Zero illegal-instruction risk on AVX2-only CPUs",
        [
            ("Independent Feature Gates", "#0f766e", "#ccfbf1"),
            ("Zero Runtime Dependencies", "#1d4ed8", "#dbeafe"),
        ],
    )

    body += rect(16, 104, width - 32, 316, CARD_BG, rx=10, stroke=BORDER)

    body += rect(36, 188, 176, 92, "#ffffff", rx=8, stroke=INK_SOFT, sw=1.5)
    body += rect(36, 188, 176, 26, PANEL_BG, rx=8)
    body += text(124, 206, "Codec::decode", 13, INK, anchor="middle", weight=700, family=MONO)
    body += text(124, 232, "Inline length + mode", 11, INK_SOFT, anchor="middle")
    body += text(124, 250, "Exact-length table", 11, INK_SOFT, anchor="middle")
    body += text(124, 268, "or general dispatch", 11, MUTED, anchor="middle")

    body += arrow(212, 234, 268, 234, TEAL_DARK, 2.0, "arrow-teal")

    body += rect(270, 188, 164, 92, "#ffffff", rx=8, stroke=TEAL, sw=2.0)
    body += badge(292, 198, "OnceLock Gate 1", TEAL_LIGHT, TEAL_DARK, w=120, h=20)
    body += text(352, 240, "AVX2 Available?", 14, TEAL_DARK, anchor="middle", weight=700)
    body += text(352, 262, "cpu::avx2_available()", 10, MUTED, anchor="middle", family=MONO)

    body += arrow(434, 214, 514, 168, TEAL_DARK, 2.0, "arrow-teal")
    body += badge(452, 172, "YES", GREEN_LIGHT, GREEN_DARK, w=42, h=18, size=10)

    body += rect(516, 124, 172, 92, "#ffffff", rx=8, stroke=BLUE, sw=2.0)
    body += badge(542, 134, "OnceLock Gate 2", BLUE_LIGHT, BLUE_DARK, w=120, h=20)
    body += text(602, 176, "BMI2 Available?", 14, BLUE_DARK, anchor="middle", weight=700)
    body += text(602, 198, "cpu::bmi2_available()", 10, MUTED, anchor="middle", family=MONO)

    body += arrow(688, 160, 748, 160, TEAL_DARK, 2.0, "arrow-teal")
    body += badge(698, 136, "YES", GREEN_LIGHT, GREEN_DARK, w=42, h=18, size=10)

    body += rect(750, 118, 274, 108, TEAL_BG, rx=8, stroke=TEAL, sw=1.8)
    body += text(887, 142, "AVX2 + BMI2 Fast Paths", 14, TEAL_DARK, anchor="middle", weight=700)
    body += text(887, 164, "• decode12 / decode24 / decode44", 11, INK, anchor="middle", family=MONO)
    body += text(887, 184, "• decode88 & tail88 (BMI2 pext6_be)", 11, INK, anchor="middle", family=MONO)
    body += text(887, 204, "• decode_hash4 ASM bulk + tail44", 11, INK, anchor="middle", family=MONO)

    body += arrow(602, 216, 748, 286, BLUE_DARK, 2.0, "arrow-blue")
    body += badge(650, 256, "NO", AMBER_LIGHT, AMBER_DARK, w=38, h=18, size=10)

    body += rect(750, 244, 274, 96, BLUE_BG, rx=8, stroke=BLUE, sw=1.5)
    body += text(887, 268, "Pure AVX2 Paths (No PEXT)", 13, BLUE_DARK, anchor="middle", weight=700)
    body += text(887, 290, "• decode12 / 24 / 44 (pure XMM/YMM)", 11, INK, anchor="middle", family=MONO)
    body += text(887, 308, "• decode_hash4 ASM bulk + tail44", 11, INK, anchor="middle", family=MONO)
    body += text(887, 326, "• Never executes PEXT instruction", 11, AMBER_DARK, anchor="middle", weight=600)

    body += arrow(352, 280, 352, 324, MUTED, 2.0)
    body += badge(362, 292, "NO", AMBER_LIGHT, AMBER_DARK, w=38, h=18, size=10)

    body += rect(236, 326, 460, 76, "#ffffff", rx=8, stroke=BORDER_STRONG, sw=1.5)
    body += text(466, 350, "AArch64 NEON / Portable Scalar Fallback", 13, INK, anchor="middle", weight=700)
    body += text(466, 370, "NEON vector loop on aarch64 · 4-char scalar table loop elsewhere", 11, INK_SOFT, anchor="middle")
    body += text(466, 388, "Identical byte output, padding bit checks, and InvalidByte offsets", 11, MUTED, anchor="middle")

    body += text(
        24,
        height - 14,
        "Verified by thread-local force_no_bmi2 / force_scalar tests and cross-architecture CI checks (src/cpu.rs).",
        11,
        MUTED,
    )
    (OUT / "cpu-dispatch.svg").write_text(svg_wrap(width, height, body, "CPU dispatch and fallback contract"))


def encode_streaming() -> None:
    width, height = 1060, 460
    body = header_band(
        width,
        "Cache-Aware Encode & Non-Temporal Streaming",
        "Cached stores for hot reused buffers · Non-temporal stores to avoid Read-For-Ownership (RFO) above LLC threshold",
        [
            ("3/7 LLC Auto-Switch", "#0f766e", "#ccfbf1"),
            ("1.39–1.68× Cold Speedup", "#1d4ed8", "#dbeafe"),
            ("258 vs 599 MiB RSS", "#b45309", "#fef3c7"),
        ],
    )

    body += rect(16, 104, width - 32, 316, CARD_BG, rx=10, stroke=BORDER)

    apis = [
        ("Codec::encode", "Default slice / Vec API", "Hot or reused buffers", TEAL),
        ("Codec::encode_streaming", "One-shot cold output API", "Socket / file / arena buffers", BLUE),
        ("Codec::encode_to_writer", "Bounded-memory I/O API", "Fixed 64 KiB chunk buffer", AMBER),
    ]
    for i, (fn_name, desc1, desc2, col) in enumerate(apis):
        ay = 124 + i * 94
        body += rect(36, ay, 248, 78, "#ffffff", rx=8, stroke=col, sw=1.6)
        body += text(52, ay + 24, fn_name, 13, col, weight=700, family=MONO)
        body += text(52, ay + 46, desc1, 11, INK, weight=600)
        body += text(52, ay + 64, desc2, 11, MUTED)

    body += rect(336, 124, 316, 172, "#ffffff", rx=8, stroke=TEAL, sw=1.6)
    body += rect(336, 124, 316, 28, TEAL_BG, rx=8)
    body += text(494, 143, "LLC Working-Set Policy (src/cpu.rs)", 13, TEAL_DARK, anchor="middle", weight=700)
    body += text(494, 172, "working_set = input_len + encoded_len", 11, INK, anchor="middle", family=MONO)
    body += text(494, 196, "• encode: NT stores when working_set > 3/7 LLC", 11, INK_SOFT, anchor="middle")
    body += text(494, 218, "• encode_streaming: NT stores from 4 KiB+", 11, INK_SOFT, anchor="middle")
    body += text(494, 240, "• 3-cache-line prefetch per 192 B AVX2 iter", 11, INK_SOFT, anchor="middle")
    body += text(494, 262, "• Short 8/16/32B: single 24B load + pad overwrite", 11, TEAL_DARK, anchor="middle", weight=600)

    body += arrow(284, 163, 334, 163, TEAL_DARK, 2.0, "arrow-teal")
    body += arrow(284, 257, 334, 220, BLUE_DARK, 2.0, "arrow-blue")

    body += rect(336, 312, 316, 90, "#ffffff", rx=8, stroke=AMBER, sw=1.5)
    body += text(494, 336, "Memory & Parallel Crossover", 13, AMBER_DARK, anchor="middle", weight=700)
    body += text(494, 358, "• encode_to_writer: 256 MiB input uses 258 MiB RSS", 11, INK_SOFT, anchor="middle")
    body += text(494, 378, "• encode_parallel: 4 workers in 4–6.5 MiB window;", 11, INK_SOFT, anchor="middle")
    body += text(494, 394, "  auto-selects 1-thread NT above LLC bandwidth limit", 11, MUTED, anchor="middle")
    body += arrow(284, 351, 334, 351, MUTED, 2.0)

    body += rect(704, 124, 320, 128, TEAL_BG, rx=8, stroke=TEAL, sw=1.6)
    body += text(864, 150, "Cached AVX2 Stores (_mm256_storeu)", 13, TEAL_DARK, anchor="middle", weight=700)
    body += text(864, 174, "Keeps output warm in L1/L2/L3 cache", 11, INK, anchor="middle")
    body += text(864, 196, "22.5 GB/s at 64 KiB · 22.3 GB/s at 1 MiB", 12, TEAL_DARK, anchor="middle", weight=700, family=MONO)
    body += text(864, 218, "Optimal when caller immediately reads output", 11, MUTED, anchor="middle")

    body += rect(704, 274, 320, 128, BLUE_BG, rx=8, stroke=BLUE, sw=1.6)
    body += text(864, 300, "Non-Temporal Stores (_mm_stream_si128)", 13, BLUE_DARK, anchor="middle", weight=700)
    body += text(864, 324, "Bypasses cache · Eliminates output RFO traffic", 11, INK, anchor="middle")
    body += text(864, 346, "16 MiB: 7.36 → 13.0 GB/s (1.77× speedup)", 12, BLUE_DARK, anchor="middle", weight=700, family=MONO)
    body += text(864, 368, "Cold 4K–4M: 1.39–1.68× faster than cached encode", 11, MUTED, anchor="middle")

    body += arrow(652, 172, 702, 172, TEAL_DARK, 2.0, "arrow-teal")
    body += arrow(652, 232, 702, 320, BLUE_DARK, 2.0, "arrow-blue")

    body += text(24, height - 14, "yolow64 v0.3.0 · Adaptive LLC detection via CPUID leaf 4 (x86_64) & sysfs fallback", 11, MUTED)
    (OUT / "encode-streaming.svg").write_text(svg_wrap(width, height, body, "Encode write policy and streaming diagram"))


def main() -> None:
    hero_benchmark_chart()
    short_all_chart()
    decode_all_chart()
    encode_all_chart()
    decode_pipeline()
    cpu_dispatch()
    encode_streaming()
    print("wrote:", ", ".join(sorted(path.name for path in OUT.glob("*.svg"))))


if __name__ == "__main__":
    main()
