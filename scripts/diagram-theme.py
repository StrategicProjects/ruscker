#!/usr/bin/env python3
"""Make the documentation diagrams follow a dark colour scheme.

The diagrams are hand-written SVG in the light palette. This script adds
one <style media="(prefers-color-scheme: dark)"> block to each of them,
mapping every light colour to its dark counterpart by attribute selector
— the drawing itself is not touched, so a diagram is still edited in one
palette. Shown as an image (README, the site without JavaScript) a
diagram follows the system's scheme; the site's theme switch flips the
same block (book/theme/ruscker.js).

    scripts/diagram-theme.py            # rewrite the block in every diagram, and the copies
    scripts/diagram-theme.py --check    # exit 1 if a file is out of date (CI)

Each diagram exists in up to three identical copies (book/src/images is
the one to edit; docs/images and, for the architecture, docs/). The
script refreshes the copies from the book's.
"""
import pathlib, re, shutil, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "book/src/images"
COPIES = {
    "architecture.svg": ["docs/images", "docs"],
    "crate-map.svg": ["docs/images"],
    "deployment.svg": ["docs/images"],
    "roadmap.svg": ["docs/images"],
    "auth-flow.svg": ["docs/images"],
    "auth-overview.svg": ["docs/images"],
}

# (selector, property, dark value). `i` makes the attribute match
# case-insensitive. White TEXT and the white brand mark are not listed:
# they sit on solid green and stay white.
RULES = [
    ('rect[fill="#FFFFFF" i], circle[fill="#FFFFFF" i]', "fill", "#132a22"),
    ('rect[fill="#F4F8F7" i], path[fill="#F4F8F7" i]', "fill", "#0f211b"),
    ('rect[fill="#E8F3EE" i], path[fill="#E8F3EE" i]', "fill", "#17352b"),
    ('rect[fill="#CDECE0" i], path[fill="#CDECE0" i]', "fill", "#1c4034"),
    ('[fill="#1A1A1A" i]', "fill", "#e7f3ee"),
    ('text[fill="#085041" i], tspan[fill="#085041" i]', "fill", "#a8e6cf"),
    ('text[fill="#0F6E56" i], tspan[fill="#0F6E56" i]', "fill", "#5dcaa5"),
    ('[fill="#5B6B66" i]', "fill", "#9bb8ac"),
    ('[fill="#90A39C" i], [fill="#B9CDC6" i]', "fill", "#7f9c90"),
    ('[stroke="#0F6E56" i]', "stroke", "#2fae87"),
    ('[stroke="#D7E5E0" i]', "stroke", "#2a4a3f"),
    ('[stroke="#B9CDC6" i]', "stroke", "#3b5a4f"),
    ('path[stroke="#FFFFFF" i][fill="#1D9E75" i]', "stroke", "#08130f"),
]
BLOCK = (
    '  <style media="(prefers-color-scheme: dark)" data-rk-dark="">\n'
    + "".join(f"    .rk-d {sel.replace(', ', ', .rk-d ')} {{ {prop}: {val}; }}\n" for sel, prop, val in RULES)
    + "  </style>\n"
)
OLD = re.compile(r'  <style media="\(prefers-color-scheme: dark\)" data-rk-dark="">.*?</style>\n', re.S)


def themed(svg: str) -> str:
    svg = OLD.sub("", svg)
    if 'class="rk-d"' not in svg:
        svg = svg.replace("<svg ", '<svg class="rk-d" ', 1)
    head, sep, rest = svg.partition("</title>\n")
    assert sep, "a diagram needs a <title>"
    return head + sep + BLOCK + rest


def main() -> int:
    check = "--check" in sys.argv
    stale = []
    for name, copies in COPIES.items():
        src = SRC / name
        want = themed(src.read_text())
        if src.read_text() != want:
            stale.append(str(src.relative_to(ROOT)))
            if not check:
                src.write_text(want)
        for d in copies:
            dst = ROOT / d / name
            if not dst.exists() or dst.read_text() != want:
                stale.append(str(dst.relative_to(ROOT)))
                if not check:
                    dst.write_text(want)
    if check and stale:
        print("out of date (run scripts/diagram-theme.py):", *stale, sep="\n  ")
        return 1
    print("diagrams up to date" if not stale else "updated:\n  " + "\n  ".join(stale))
    return 0


if __name__ == "__main__":
    sys.exit(main())
