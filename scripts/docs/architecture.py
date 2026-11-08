#!/usr/bin/env python3
"""Writes the README's architecture diagram, docs/assets/architecture-{light,dark}.svg, from the layout below:
one source for both themes (the README picks one with <picture>). Standard library only.

    python3 scripts/docs/architecture.py [--out docs/assets]
"""
import argparse
import pathlib
from xml.sax.saxutils import escape

W, H = 1000, 812
FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif"

THEMES = {
    "light": {"text": "#1f2328", "muted": "#59636e", "group": "#f6f8fa", "group_line": "#d1d9e0", "box": "#ffffff",
              "box_line": "#afb8c1", "core": "#fbe3ee", "accent": "#d6156f", "line": "#8c959f", "on_accent": "#ffffff"},
    "dark": {"text": "#e6edf3", "muted": "#9198a1", "group": "#151b23", "group_line": "#3d444d", "box": "#0d1117",
             "box_line": "#656c76", "core": "#2a0f1c", "accent": "#ff3f98", "line": "#8b949e", "on_accent": "#0a0b0c"},
}


def text(x, y, s, size=12, weight="normal", fill="text", anchor="start"):
    return (f'<text x="{x}" y="{y}" font-size="{size}" font-weight="{weight}" fill="var(--{fill})" '
            f'text-anchor="{anchor}">{escape(s)}</text>')


def group(x, y, w, h, title, note=""):
    out = [f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="var(--group)" stroke="var(--group_line)"/>',
           f'<text x="{x + 20}" y="{y + 26}" font-size="14" font-weight="600" fill="var(--text)">{escape(title)}'
           + (f'<tspan font-size="12.5" font-weight="normal" fill="var(--muted)" dx="8">{escape(note)}</tspan>' if note else "")
           + "</text>"]
    return "\n".join(out)


def box(x, y, w, h, title, sub=(), core=False, title_size=14):
    """A component: a bold title and muted lines, centred."""
    fill, line = ("var(--core)", "var(--accent)") if core else ("var(--box)", "var(--box_line)")
    titles = [title] if isinstance(title, str) else list(title)
    lines = len(titles) + len(sub)
    lh_title, lh_sub = title_size + 4, 16
    total = len(titles) * lh_title + len(sub) * lh_sub
    cy = y + (h - total) / 2
    out = [f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="{fill}" stroke="{line}"'
           + (' stroke-width="1.5"' if core else "") + "/>"]
    cx = x + w / 2
    yy = cy + title_size
    for t in titles:
        out.append(text(cx, yy, t, size=title_size, weight="600", anchor="middle"))
        yy += lh_title
    yy += 12.5 - title_size + (lh_sub - 12.5) / 2 if sub else 0
    for s in sub:
        out.append(text(cx, yy, s, size=12.5, fill="muted", anchor="middle"))
        yy += lh_sub
    assert lines
    return "\n".join(out)


def path(points, accent=False, dashed=False, start=False, end=True):
    d = "M " + " L ".join(f"{px} {py}" for px, py in points)
    color = "accent" if accent else "line"
    attrs = [f'd="{d}"', 'fill="none"', f'stroke="var(--{color})"', f'stroke-width="{1.8 if accent else 1.3}"']
    if dashed:
        attrs.append('stroke-dasharray="5 4"')
    if end:
        attrs.append(f'marker-end="url(#head-{color})"')
    if start:
        attrs.append(f'marker-start="url(#head-{color})"')
    return f'<path {" ".join(attrs)}/>'


def step(x, y, n):
    """A numbered step: the accent disc, centred at (x, y)."""
    return (f'<circle cx="{x}" cy="{y}" r="9" fill="var(--accent)"/>'
            f'<text x="{x}" y="{y + 4}" font-size="11.5" font-weight="700" fill="var(--on_accent)" text-anchor="middle">{n}</text>')


def body():
    o = []
    # Who acts.
    o.append(box(40, 24, 240, 64, "Project", ["commits its rules before a launch"]))
    o.append(box(380, 24, 240, 64, "Hunter or AI agent", ["CLI or MCP"]))

    # The legend.
    o.append(path([(712, 42), (760, 42)], accent=True))
    o.append(text(772, 46, "steps 1 to 5 of How it works"))
    o.append(path([(712, 66), (760, 66)], dashed=True))
    o.append(text(772, 70, "a read or a check"))
    o.append(text(712, 94, "Keepers and relayers carry the quote, the", size=11.5, fill="muted"))
    o.append(text(712, 109, "payout call and the permits: anyone can run one.", size=11.5, fill="muted"))

    # NEAR testnet.
    o.append(group(20, 150, 690, 395, "NEAR testnet", "every contract locked"))
    o.append(box(290, 200, 240, 90, "near-antibody", ["programs, escrow,", "verdicts, payouts"], core=True, title_size=16))
    o.append(box(40, 200, 200, 52, "Ethereum light client", ["confirms the anchor block"], title_size=13.5))
    o.append(box(40, 264, 200, 52, "Intel TDX quote verifier", ["checks the run's quote: T1"], title_size=13.5))
    o.append(box(40, 360, 210, 56, "near-t2", ["verifies SP1 proofs: T2"]))
    o.append(box(300, 360, 200, 56, "Verdict guard", ["a NEAR app acting on verdicts"]))
    o.append(box(540, 360, 160, 56, "near-guardian", ["pause permits"]))
    o.append(box(540, 470, 160, 46, "MPC network", ["signs for other chains"], title_size=13.5))

    # OutLayer.
    o.append(group(740, 150, 240, 250, "OutLayer"))
    o.append(box(755, 192, 210, 188, ("Verifier in an", "Intel TDX enclave"),
                 ["opens the sealed proof,", "checks its state against", "the anchor block, re-runs", "the calls, and reports the",
                  "rule broken, the entry", "point and the impact"]))

    # The protocol's chains.
    o.append(group(20, 610, 960, 185, "The protocol's chains", "Ethereum testnets"))
    for gx, name in ((40, "Sepolia"), (510, "Base Sepolia")):
        o.append(f'<rect x="{gx}" y="650" width="450" height="128" rx="10" fill="none" stroke="var(--group_line)"/>')
        o.append(text(gx + 436, 672, name, size=13, weight="600", fill="muted", anchor="end"))
        o.append(box(gx + 20, 694, 180, 62, "AntibodyAdapter", ["accepts signed permits"]))
        o.append(box(gx + 260, 694, 170, 62, "Acme Lend", ["our test protocol"]))
        o.append(path([(gx + 200, 725), (gx + 260, 725)]))
        o.append(text(gx + 230, 717, "pause()", size=11.5, fill="muted", anchor="middle"))

    # Each step's disc sits on its arrow; its words sit beside it.
    # 1: the program.
    o.append(path([(160, 88), (160, 110), (320, 110), (320, 200)], accent=True))
    o.append(step(240, 110, 1))
    o.append(text(170, 128, "rules as code,"))
    o.append(text(170, 143, "payouts and escrow"))
    # 2: the sealed proof; 4: the payout.
    o.append(path([(410, 88), (410, 200)], accent=True))
    o.append(step(410, 118, 2))
    o.append(text(422, 112, "sealed proof"))
    o.append(text(422, 127, "and a bond:"))
    o.append(text(422, 142, "commit, reveal"))
    o.append(path([(520, 200), (520, 88)], accent=True))
    o.append(step(520, 118, 4))
    o.append(text(532, 112, "the bounty in wNEAR"))
    o.append(text(532, 127, "in seconds once attested,", size=11.5, fill="muted"))
    o.append(text(532, 141, "else after its window", size=11.5, fill="muted"))
    # 3: the run, its verdict and its quote.
    o.append(path([(530, 212), (755, 212)], accent=True))
    o.append(step(552, 212, 3))
    o.append(text(568, 205, "run request, pinned verifier"))
    o.append(path([(755, 238), (530, 238)]))
    o.append(text(642, 233, "the verdict", anchor="middle"))
    o.append(path([(755, 262), (530, 262)]))
    o.append(text(642, 257, "the run's TDX quote, relayed", anchor="middle"))
    # Reads and checks inside NEAR.
    o.append(path([(290, 226), (240, 226)], dashed=True))
    o.append(path([(290, 278), (240, 290)], dashed=True))
    o.append(path([(200, 360), (320, 290)], dashed=True))
    o.append(text(48, 341, "reads the run's record", size=11.5, fill="muted"))
    o.append(path([(400, 360), (400, 290)], dashed=True))
    o.append(text(392, 331, "reads verdicts", size=11.5, fill="muted", anchor="end"))
    # 5: the pause.
    o.append(path([(530, 282), (690, 282), (690, 360)], accent=True))
    o.append(step(690, 318, 5))
    o.append(text(676, 314, "a critical verdict", anchor="end"))
    o.append(text(676, 330, "in the pause policy", anchor="end"))
    o.append(path([(620, 416), (620, 470)], start=True))
    o.append(text(612, 447, "signs the permits", size=11.5, fill="muted", anchor="end"))
    o.append(path([(620, 516), (620, 580), (620, 694)], accent=True))
    o.append(path([(620, 580), (150, 580), (150, 694)], accent=True))
    o.append(text(385, 573, "the signed permits, relayed to every chain", anchor="middle"))
    return "\n".join(o)


def svg(theme):
    """The layout in one theme's colours, written as literal values (an SVG shown as an image may not resolve
    CSS variables in its attributes)."""
    t = THEMES[theme]
    s = _svg()
    for k, v in t.items():
        s = s.replace(f"var(--{k})", v)
    assert "var(--" not in s
    return s


def _svg():
    heads = "".join(
        f'<marker id="head-{c}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" '
        f'orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="var(--{c})"/></marker>'
        for c in ("accent", "line"))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
            f'font-family="{escape(FONT)}" role="img" '
            'aria-label="Antibody architecture: a project commits rules, payouts and an escrow to near-antibody on NEAR '
            "testnet; a hunter or AI agent commits and reveals a sealed proof with a bond; near-antibody asks OutLayer to "
            "run the pinned verifier in an Intel TDX enclave, which returns the verdict and its TDX quote, verified on NEAR; "
            "near-antibody pays the bounty in wNEAR; a critical verdict in the pause policy has near-guardian and the MPC "
            'network sign pause permits, relayed to the AntibodyAdapter that pauses Acme Lend on Sepolia and Base Sepolia.">'
            f"<title>How Antibody works: the contracts, the enclave and the chains</title><defs>{heads}</defs>\n"
            f"{body()}\n</svg>\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(pathlib.Path(__file__).resolve().parents[2] / "docs" / "assets"))
    a = ap.parse_args()
    out = pathlib.Path(a.out)
    for theme in THEMES:
        p = out / f"architecture-{theme}.svg"
        p.write_text(svg(theme), encoding="utf-8", newline="\n")
        print(p)


if __name__ == "__main__":
    main()
