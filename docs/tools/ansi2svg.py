#!/usr/bin/env python3
"""Render a `tmux capture-pane -e -p` dump (24-bit SGR colours) as an SVG
"terminal window". Box-drawing, block and meter characters are drawn as
shapes rather than font glyphs so the result stays crisp and gap-free.

    ansi2svg.py capture.ansi out.svg --title "root@host: xtop" [--bg '#101014']
"""
import argparse
import html
import re

CW, CH = 8.6, 18.0          # cell size (px) for 14.3px DejaVu Sans Mono
FONT = 14.3
PAD, BAR = 18, 34           # window padding, title bar height

SGR = re.compile(r"\x1b\[([0-9;]*)m")


def parse(text, default_fg, default_bg):
    """-> list of rows, each a list of (char, fg, bg, bold)."""
    rows = []
    fg, bg, bold = default_fg, default_bg, False
    for line in text.rstrip("\n").split("\n"):
        row, pos = [], 0
        for m in SGR.finditer(line + "\x1b[m"):
            for ch in line[pos:m.start()]:
                row.append((ch, fg, bg, bold))
            pos = m.end()
            codes = [int(c) if c else 0 for c in m.group(1).split(";")] if m.group(1) else [0]
            i = 0
            while i < len(codes):
                c = codes[i]
                if c == 0:
                    fg, bg, bold = default_fg, default_bg, False
                elif c == 1:
                    bold = True
                elif c == 22:
                    bold = False
                elif c == 39:
                    fg = default_fg
                elif c == 49:
                    bg = default_bg
                elif c in (38, 48) and i + 4 < len(codes) and codes[i + 1] == 2:
                    col = "#%02x%02x%02x" % tuple(codes[i + 2:i + 5])
                    if c == 38:
                        fg = col
                    else:
                        bg = col
                    i += 4
                i += 1
        rows.append(row)
    return rows


def box_path(ch, x, y):
    """SVG path data for a box-drawing char in the cell at (x, y), or None."""
    cx, cy = x + CW / 2, y + CH / 2
    r = CW / 2
    return {
        "─": f"M{x},{cy}H{x + CW}",
        "│": f"M{cx},{y}V{y + CH}",
        "┃": f"M{cx},{y}V{y + CH}",
        "╭": f"M{x + CW},{cy}H{cx + r}Q{cx},{cy} {cx},{cy + r}V{y + CH}",
        "╮": f"M{x},{cy}H{cx - r}Q{cx},{cy} {cx},{cy + r}V{y + CH}",
        "╰": f"M{x + CW},{cy}H{cx + r}Q{cx},{cy} {cx},{cy - r}V{y}",
        "╯": f"M{x},{cy}H{cx - r}Q{cx},{cy} {cx},{cy - r}V{y}",
    }.get(ch)


def render(rows, title, win_bg, out):
    cols = max(len(r) for r in rows)
    w = cols * CW + 2 * PAD
    h = len(rows) * CH + 2 * PAD + BAR
    o = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w + 60:.0f}" height="{h + 60:.0f}" '
        f'viewBox="-30 -24 {w + 60:.0f} {h + 60:.0f}">',
        '<defs><filter id="sh" x="-10%" y="-10%" width="120%" height="130%">'
        '<feDropShadow dx="0" dy="10" stdDeviation="12" flood-opacity="0.45"/></filter></defs>',
        f'<rect width="{w:.1f}" height="{h:.1f}" rx="11" fill="{win_bg}" filter="url(#sh)"/>',
        f'<rect width="{w:.1f}" height="{BAR}" rx="11" fill="#2a2a31"/>',
        f'<rect y="{BAR - 11}" width="{w:.1f}" height="11" fill="#2a2a31"/>',
    ]
    for i, c in enumerate(["#ff5f57", "#febc2e", "#28c840"]):
        o.append(f'<circle cx="{22 + i * 20}" cy="{BAR / 2}" r="6" fill="{c}"/>')
    o.append(
        f'<text x="{w / 2:.1f}" y="{BAR / 2 + 5}" text-anchor="middle" fill="#a0a0aa" '
        f'font-family="DejaVu Sans" font-size="13">{html.escape(title)}</text>'
    )
    o.append(f'<g transform="translate({PAD},{BAR + PAD / 2})" font-family="DejaVu Sans Mono" '
             f'font-size="{FONT}">')

    for ry, row in enumerate(rows):
        y = ry * CH
        # Backgrounds, merged into runs.
        x0 = 0
        while x0 < len(row):
            bg = row[x0][2]
            x1 = x0
            while x1 < len(row) and row[x1][2] == bg:
                x1 += 1
            if bg != win_bg:
                o.append(f'<rect x="{x0 * CW:.1f}" y="{y:.1f}" width="{(x1 - x0) * CW + 0.3:.1f}" '
                         f'height="{CH + 0.3:.1f}" fill="{bg}"/>')
            x0 = x1
        # Shapes and text.
        run, run_x, run_style = "", 0, None

        def flush():
            nonlocal run
            if run.strip():
                fg, bold = run_style
                wt = ' font-weight="bold"' if bold else ""
                o.append(f'<text x="{run_x * CW:.1f}" y="{y + CH * 0.76:.1f}" fill="{fg}"{wt} '
                         f'textLength="{len(run) * CW:.1f}" lengthAdjust="spacingAndGlyphs" '
                         f'xml:space="preserve">{html.escape(run)}</text>')
            run = ""

        for rx, (ch, fg, bg, bold) in enumerate(row):
            x = rx * CW
            shape = None
            if ch in "▀█":
                hh = CH / 2 if ch == "▀" else CH
                shape = f'<rect x="{x:.1f}" y="{y:.1f}" width="{CW + 0.3:.1f}" height="{hh + 0.3:.1f}" fill="{fg}"/>'
            elif ch == "■":
                s = CW * 0.72
                shape = (f'<rect x="{x + (CW - s) / 2:.1f}" y="{y + (CH - s) / 2:.1f}" width="{s:.1f}" '
                         f'height="{s:.1f}" rx="1" fill="{fg}"/>')
            elif ch in "▁▂▃▄▅▆▇":
                hh = CH * ("▁▂▃▄▅▆▇".index(ch) + 1) / 8
                shape = f'<rect x="{x:.1f}" y="{y + CH - hh:.1f}" width="{CW + 0.3:.1f}" height="{hh:.1f}" fill="{fg}"/>'
            elif 0x2801 <= ord(ch) <= 0x28FF:
                # Braille: draw the dots as blocks on a 2x4 grid.
                bits, dw, dh = ord(ch) - 0x2800, CW / 2, CH / 4
                dots = [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (0, 3, 0x40),
                        (1, 0, 0x08), (1, 1, 0x10), (1, 2, 0x20), (1, 3, 0x80)]
                shape = "".join(
                    f'<rect x="{x + dx * dw + 0.6:.1f}" y="{y + dy * dh + 0.6:.1f}" '
                    f'width="{dw - 1.2:.1f}" height="{dh - 1.2:.1f}" rx="0.8" fill="{fg}"/>'
                    for dx, dy, bit in dots if bits & bit)
            elif box_path(ch, x, y):
                sw = 2.2 if ch == "┃" else 1.2
                shape = (f'<path d="{box_path(ch, x, y)}" stroke="{fg}" stroke-width="{sw}" '
                         f'fill="none"/>')
            if shape or ch == " " or (run and run_style != (fg, bold)):
                flush()
            if shape:
                o.append(shape)
                continue
            if ch == " ":
                continue
            if not run:
                run_x, run_style = rx, (fg, bold)
            run += ch
        flush()
    o.append("</g></svg>")
    with open(out, "w") as f:
        f.write("\n".join(o))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("capture")
    ap.add_argument("out")
    ap.add_argument("--title", default="xentop-ng")
    ap.add_argument("--bg", default="#101014", help="terminal background (theme bg)")
    ap.add_argument("--fg", default="#cccccc")
    a = ap.parse_args()
    with open(a.capture, encoding="utf-8") as f:
        rows = parse(f.read(), a.fg, a.bg)
    render(rows, a.title, a.bg, a.out)


if __name__ == "__main__":
    main()
