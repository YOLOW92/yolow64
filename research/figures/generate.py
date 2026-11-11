#!/usr/bin/env python3
"""Generate the public 0.2.0 performance charts and principle diagrams.

The SVGs are checked into docs/figures/ so they render on GitHub and crates.io.
Run from anywhere:

    python3 research/figures/generate.py
"""

from __future__ import annotations

import csv
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "figures"
OUT.mkdir(parents=True, exist_ok=True)

INK = "#0f172a"
MUTED = "#64748b"
GRID = "#e2e8f0"
TEAL = "#0f766e"
BLUE = "#2563eb"
SLATE = "#94a3b8"
AMBER = "#b45309"
GREEN = "#15803d"
PANEL = "#f8fafc"
FONT = "Inter, ui-sans-serif, system-ui, -apple-system, sans-serif"
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"


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
    size: int = 14,
    fill: str = INK,
    anchor: str = "start",
    weight: int = 400,
    family: str = FONT,
    opacity: float = 1.0,
) -> str:
    return (
        f'<text x="{x:.1f}" y="{y:.1f}" font-family="{family}" '
        f'font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
        f'font-weight="{weight}" opacity="{opacity}">{esc(value)}</text>'
    )


def rect(
    x: float,
    y: float,
    w: float,
    h: float,
    fill: str,
    rx: float = 6,
    stroke: str | None = None,
    sw: float = 1.5,
) -> str:
    border = f' stroke="{stroke}" stroke-width="{sw}"' if stroke else ""
    return f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" fill="{fill}"{border}/>'


def line(x1: float, y1: float, x2: float, y2: float, color: str = GRID, width: float = 1) -> str:
    return f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="{color}" stroke-width="{width}"/>'


def arrow(x1: float, y1: float, x2: float, y2: float, color: str = MUTED) -> str:
    return (
        f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
        f'stroke="{color}" stroke-width="2" marker-end="url(#arrow)"/>'
    )


def svg(width: int, height: int, body: str, label: str) -> str:
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="{esc(label)}">
<defs>
  <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
    <path d="M0,0 L10,5 L0,10 z" fill="{MUTED}"/>
  </marker>
</defs>
<rect width="100%" height="100%" fill="#ffffff"/>
{body}
</svg>
'''


def chart_header(title: str, subtitle: str) -> str:
    return text(48, 48, title, 26, INK, weight=700) + text(48, 74, subtitle, 14, MUTED)


def footer(label: str) -> str:
    return text(48, 512, label, 12, MUTED, opacity=0.9)


def read_csv(name: str) -> list[dict[str, str]]:
    with (ROOT / "benchmarks" / "results" / name).open(newline="") as handle:
        return list(csv.DictReader(handle))


def lerp_color(t: float) -> str:
    # Light slate to deep teal. t=0 light, t=1 dark.
    t = max(0.0, min(1.0, t))
    a = (241, 245, 249)
    b = (15, 118, 110)
    r = round(a[0] + (b[0] - a[0]) * t)
    g = round(a[1] + (b[1] - a[1]) * t)
    bl = round(a[2] + (b[2] - a[2]) * t)
    return f"#{r:02x}{g:02x}{bl:02x}"


def heatmap(
    filename: str,
    title: str,
    subtitle: str,
    row_labels: list[str],
    columns: list[str],
    values: list[list[float | None]],
    lower_better: bool,
    highlight: str,
    footer: str,
    left: int,
    row_h: int,
    width: int = 1000,
) -> None:
    top = 126
    right = width - 50
    rows = len(row_labels)
    height = top + rows * row_h + 82
    body = text(48, 48, title, 26, INK, weight=700)
    body += text(48, 74, subtitle, 14, MUTED)
    # Compact color legend: dark end is always the better value.
    legend_x = right - 150
    left_label, right_label = ("better", "slower") if lower_better else ("slower", "better")
    body += text(legend_x - 8, 70, left_label, 11, MUTED, anchor="end")
    body += text(right, 70, right_label, 11, MUTED, anchor="end")
    for i in range(5):
        shade = 1.0 - i / 4 if lower_better else i / 4
        body += rect(legend_x + i * 22, 58, 18, 10, lerp_color(shade), rx=2)
    col_w = (right - left) / len(columns)
    for index, label in enumerate(columns):
        body += text(left + col_w * (index + 0.5), top - 14, label, 13, INK, anchor="middle", weight=600)
    # Per-column normalization so each size is compared within itself.
    mins: list[float] = []
    maxs: list[float] = []
    for column in range(len(columns)):
        present = [row[column] for row in values if row[column] is not None]
        mins.append(min(present) if present else 0.0)
        maxs.append(max(present) if present else 1.0)
    for r, label in enumerate(row_labels):
        y = top + r * row_h
        if label == highlight:
            body += rect(30, y + 1, width - 60, row_h - 2, "#f0fdfa", rx=5)
            body += rect(30, y + 4, 4, row_h - 8, TEAL, rx=2)
        body += text(left - 16, y + row_h / 2 + 5, label, 13, INK, anchor="end", weight=700 if label == highlight else 400)
        for c in range(len(columns)):
            value = values[r][c]
            if value is None:
                body += rect(left + col_w * c + 2, y + 3, col_w - 4, row_h - 8, "#f1f5f9", rx=4)
                body += text(left + col_w * (c + 0.5), y + row_h / 2 + 4, "—", 12, MUTED, anchor="middle")
                continue
            span = maxs[c] - mins[c]
            t = 0.5 if span == 0 else (value - mins[c]) / span
            if lower_better:
                t = 1.0 - t
            fill = lerp_color(t)
            body += rect(left + col_w * c + 2, y + 3, col_w - 4, row_h - 8, fill, rx=4)
            ink = "#ffffff" if t > 0.56 else INK
            body += text(
                left + col_w * (c + 0.5),
                y + row_h / 2 + 4,
                f"{value:.1f}" if row_h <= 30 else f"{value:.2f}",
                12,
                ink,
                anchor="middle",
                family=MONO,
                weight=600,
            )
    body += text(48, height - 16, footer, 12, MUTED, opacity=0.9)
    (OUT / filename).write_text(svg(width, height, body, title))


def decode_all_chart() -> None:
    rows = read_csv("8way_decode_all.csv")
    fullcheck = {
        int(row["size"]): float(row["gbps"])
        for row in read_csv("turbo_check_verify.csv")
        if row["mode"] == "direct" and row["build"] == "fullcheck"
    }
    sizes = [16384, 65536, 262144, 1048576, 4194304]
    columns = ["16 KiB", "64 KiB", "256 KiB", "1 MiB", "4 MiB"]
    data: dict[str, dict[int, float]] = {}
    for row in rows:
        data.setdefault(row["lib"], {})[int(row["size"])] = float(row["gbps"])
    names = {
        "base64-scalar": "base64 (scalar)",
        "base64-simd-engine": "base64 (SIMD engine)",
        "base64-simd": "base64-simd",
        "base64-turbo": "base64-turbo",
        "encodify": "encodify",
        "simdutf": "simdutf",
        "turbo-base64": "Turbo (FULLCHECK)",
        "yolow64": "yolow64",
    }
    display: dict[str, list[float | None]] = {}
    for lib in data:
        if lib == "turbo-base64":
            display[lib] = [fullcheck.get(size) for size in sizes]
        else:
            display[lib] = [data[lib].get(size) for size in sizes]
    ordered = sorted(display, key=lambda lib: display[lib][3] or 0, reverse=True)
    row_labels = [names[lib] for lib in ordered]
    values = [display[lib] for lib in ordered]
    # Place the footer inside the computed canvas: heatmap() appends it at y=0.
    # Use a dedicated footer line by patching the last text y after generation.
    # Simpler: title/subtitle carry the source note; keep a short footer.
    heatmap(
        "decode-all.svg",
        "Decode throughput — all compared libraries",
        "GB/s, higher is better. Records are six-process medians; Turbo uses FULLCHECK=1.",
        row_labels,
        columns,
        values,
        lower_better=False,
        highlight="yolow64",
        footer="Turbo row = direct-kernel FULLCHECK; other rows = recorded API run. 4 MiB is noisy. Source: 8way_decode_all.csv + turbo_check_verify.csv.",
        left=250,
        row_h=44,
    )


def short_all_chart() -> None:
    rows = read_csv("8way_short_all.csv")
    sizes = [12, 16, 24, 48, 96, 128]
    columns = [f"{size} B" for size in sizes]
    data: dict[str, dict[int, float]] = {}
    for row in rows:
        data.setdefault(row["lib"], {})[int(row["size"])] = float(row["median_ns"])
    names = {
        "base64-scalar": "base64 (scalar)",
        "base64-simd-engine": "base64 (SIMD engine)",
        "base64-simd": "base64-simd",
        "base64-turbo": "base64-turbo",
        "encodify": "encodify",
        "simdutf": "simdutf",
        "turbo-base64": "Turbo (default, 25%)",
        "yolow64": "yolow64",
    }
    ordered = sorted(data, key=lambda lib: data[lib].get(24, 0), reverse=False)
    values = [[data[lib].get(size) for size in sizes] for lib in ordered]
    heatmap(
        "short-all.svg",
        "Short decode latency — all compared libraries",
        "ns/call, lower is better. Turbo's short row is its default partial-check build.",
        [names[lib] for lib in ordered],
        columns,
        values,
        lower_better=True,
        highlight="yolow64",
        footer="yolow64 validates every block; under FULLCHECK Turbo is slower. Source: benchmarks/results/8way_short_all.csv.",
        left=250,
        row_h=44,
    )


def encode_all_chart() -> None:
    rows = [
        row
        for row in read_csv("encode_all_combined.csv")
        if row["mode"] == "large" and row["kind"] == "slice"
    ]
    sizes = [8192, 65536, 1048576, 4194304]
    columns = ["8 KiB", "64 KiB", "1 MiB", "4 MiB"]
    data: dict[str, dict[int, float]] = {}
    for row in rows:
        data.setdefault(row["lib"], {})[int(row["size"])] = float(row["gbps"])
    # Keep every project in the 22-way campaign visible; URL-only and
    # allocation-only entries have no comparable standard-slice cell.
    for lib in ["base64-url", "base64_light"]:
        data.setdefault(lib, {})
    ordered = sorted(data, key=lambda lib: data[lib].get(1048576, 0), reverse=True)
    labels = [lib.replace("fastbase64-", "fb64-") for lib in ordered]
    values = [[data[lib].get(size) for size in sizes] for lib in ordered]
    heatmap(
        "encode-all.svg",
        "Encode throughput — recorded 22-way comparison",
        "GB/s, higher is better. Standard-alphabet slice mode; URL/alloc-only libraries are omitted.",
        labels,
        columns,
        values,
        lower_better=False,
        highlight="yolow64",
        footer="Recorded 22-way campaign; external columns were not re-ranked. URL/alloc-only entries show —. Final 8-way numbers: benchmarks/README.md.",
        left=300,
        row_h=27,
    )


def box(x: float, y: float, w: float, h: float, title: str, lines: list[str], color: str) -> str:
    body = rect(x, y, w, h, PANEL, rx=10, stroke=color)
    body += text(x + w / 2, y + 28, title, 15, color, anchor="middle", weight=700)
    for index, value in enumerate(lines):
        body += text(x + w / 2, y + 50 + index * 17, value, 12, MUTED, anchor="middle")
    return body


def decode_pipeline() -> None:
    body = text(48, 48, "Decode pipeline", 26, INK, weight=700)
    body += text(48, 74, "Bulk AVX2 block path and the short exact-length / BMI2 pext paths.", 14, MUTED)
    body += text(48, 112, "bulk, one 32-character block", 13, TEAL, weight=700)
    body += box(48, 126, 168, 96, "32 base64 chars", ["256-bit load", "unaligned"], TEAL)
    body += arrow(216, 174, 246, 174)
    body += box(246, 126, 206, 96, "translate + validate", ["2× vpshufb hash", "vpaddsb OR accumulator"], TEAL)
    body += arrow(452, 174, 482, 174)
    body += box(482, 126, 206, 96, "pack 24 bytes", ["vpmaddubsw → vpmaddwd", "vpshufb + 2 stores"], TEAL)
    body += arrow(688, 174, 718, 174)
    body += box(718, 126, 214, 96, "24 output bytes", ["two 16-byte stores", "speculative error test"], TEAL)

    body += text(48, 286, "short exact-length paths", 13, BLUE, weight=700)
    body += box(48, 300, 276, 100, "no-pad 16 / 32 / 64 / 128", ["exact-length dispatch", "decode16 / block kernel"], BLUE)
    body += box(354, 300, 310, 100, "padded 24 / 44 / 172", ["128-bit head + BMI2 pext tail", "padding-bit checks"], BLUE)
    body += box(694, 300, 238, 100, "short-path contract", ["every block validated", "same bytes / error offsets"], MUTED)
    body += text(48, 448, "All paths validate every block and reject non-zero trailing bits. Feature fallback is shown in the CPU dispatch figure.", 13, MUTED)
    body += footer("yolow64 0.2.0 · AVX2 and BMI2 are detected independently")
    (OUT / "decode-pipeline.svg").write_text(svg(1000, 520, body, "Decode pipeline diagram"))


def cpu_dispatch() -> None:
    body = text(48, 48, "CPU dispatch and fallback", 26, INK, weight=700)
    body += text(48, 74, "AVX2 and BMI2 are independent feature bits. AVX2 does not imply PEXT.", 14, MUTED)
    body += box(48, 150, 200, 86, "decode call", ["length + padding", "strict / whitespace"], MUTED)
    body += arrow(248, 193, 300, 193)
    body += f'<polygon points="300,193 380,143 460,193 380,243" fill="{PANEL}" stroke="{TEAL}" stroke-width="2"/>'
    body += text(380, 188, "AVX2?", 16, TEAL, anchor="middle", weight=700)
    body += text(380, 207, "available?", 12, MUTED, anchor="middle")
    body += arrow(460, 193, 540, 193)
    body += text(500, 183, "yes", 12, GREEN, anchor="middle", weight=600)
    body += f'<polygon points="540,193 620,143 700,193 620,243" fill="{PANEL}" stroke="{TEAL}" stroke-width="2"/>'
    body += text(620, 188, "BMI2?", 16, TEAL, anchor="middle", weight=700)
    body += text(620, 207, "available?", 12, MUTED, anchor="middle")
    body += arrow(700, 193, 754, 193)
    body += text(727, 183, "yes", 12, GREEN, anchor="middle", weight=600)
    body += box(754, 145, 240, 96, "fast paths", ["BMI2 pext tails", "24 / 44 / 172", "AVX2 bulk otherwise"], GREEN)

    body += line(380, 243, 380, 320, MUTED, 2)
    body += arrow(380, 320, 380, 330)
    body += text(390, 285, "no", 12, AMBER, weight=600)
    body += box(270, 330, 220, 86, "scalar path", ["no AVX2", "same bytes / errors"], MUTED)

    body += line(620, 243, 620, 320, MUTED, 2)
    body += arrow(620, 320, 620, 330)
    body += text(630, 285, "no", 12, AMBER, weight=600)
    body += box(510, 330, 240, 86, "AVX2 fallback", ["AVX2 / scalar paths", "never execute PEXT"], AMBER)

    body += text(48, 452, "Fallback is exercised by a thread-local no-BMI2 test on a BMI2 CPU and by cross-builds; production checks are independent.", 13, MUTED)
    body += footer("yolow64 0.2.0 · no runtime third-party dependencies")
    (OUT / "cpu-dispatch.svg").write_text(svg(1040, 520, body, "CPU dispatch and fallback diagram"))


def encode_streaming() -> None:
    body = text(48, 48, "Encode write policy", 26, INK, weight=700)
    body += text(48, 74, "Cached stores for reused buffers; non-temporal stores for one-shot and large outputs.", 14, MUTED)
    body += box(48, 128, 250, 88, "working set", ["input + output", "against detected LLC"], BLUE)
    body += arrow(298, 172, 348, 172)
    body += f'<polygon points="456,112 558,172 456,232 354,172" fill="{PANEL}" stroke="{BLUE}" stroke-width="2"/>'
    body += text(456, 168, "below", 15, BLUE, anchor="middle", weight=700)
    body += text(456, 188, "3/7 LLC?", 15, BLUE, anchor="middle", weight=700)
    body += arrow(558, 172, 608, 172)
    body += text(580, 162, "yes", 12, GREEN, anchor="middle", weight=600)
    body += box(608, 128, 250, 88, "cached stores", ["default encode", "reused / reread buffers"], GREEN)
    body += line(456, 232, 456, 306, MUTED, 2)
    body += arrow(456, 306, 608, 306)
    body += text(466, 274, "no", 12, AMBER, weight=600)
    body += box(608, 272, 250, 88, "NT stores", ["single-thread streaming", "no read-for-ownership"], AMBER)
    body += text(48, 398, "encode_streaming: NT from 4 KiB when the output is 16-byte aligned; encode keeps the 3/7 LLC rule.", 13, MUTED)
    body += text(48, 422, "Unknown LLC or unaligned output falls back to cached stores.", 13, MUTED)
    body += footer("yolow64 0.2.0 · threshold scales with the detected last-level cache")
    (OUT / "encode-streaming.svg").write_text(svg(1000, 500, body, "Encode write-policy diagram"))


def main() -> None:
    decode_all_chart()
    short_all_chart()
    encode_all_chart()
    decode_pipeline()
    cpu_dispatch()
    encode_streaming()
    print("wrote:", ", ".join(sorted(path.name for path in OUT.glob("*.svg"))))


if __name__ == "__main__":
    main()
