#!/usr/bin/env python3
"""Subset the bundled Tabler icon font to only the glyphs the templates use
(#911).

The full Tabler set is ~447 KB woff2 + ~209 KB CSS but Ruscker uses only a
few dozen `ti-*` icons. The full files are kept in-tree as the regeneration
source (`tabler-icons-full.{woff2,css}`); this script reads the `ti-*`
classes referenced under `templates/`, maps them to code points via the
full CSS, and writes the trimmed `tabler-icons.{woff2,min.css}` that get
`include_bytes!`'d.

Run from the repo root with fonttools + brotli available:

    python3 -m venv .venv && .venv/bin/pip install fonttools brotli
    .venv/bin/python scripts/subset-tabler-icons.py

Re-run whenever a new `ti-*` icon is used in a template. The
`tests/icon_subset.rs` gate fails if a used icon isn't in the shipped CSS.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "crates/ruscker-admin/assets/icons"
TEMPLATES = ROOT / "crates/ruscker-admin/templates"
FULL_CSS = ICONS / "tabler-icons-full.css"
FULL_WOFF2 = ICONS / "tabler-icons-full.woff2"
OUT_CSS = ICONS / "tabler-icons.min.css"
OUT_WOFF2 = ICONS / "tabler-icons.woff2"

# class name -> codepoint string, e.g. {"ti-search": "eb1c"}
rule_re = re.compile(r"\.(ti-[a-z0-9-]+):before\{content:\"\\([0-9a-fA-F]+)\"\}")
full_css = FULL_CSS.read_text()
all_icons = {m.group(1): m.group(2) for m in rule_re.finditer(full_css)}

# Base rule that sets the font-family on `.ti` — always keep it.
base = re.search(r"\.ti\{font-family[^}]*\}", full_css)
face = re.search(r"@font-face\{[^}]*\}", full_css)
if not base or not face:
    sys.exit("could not find .ti base rule or @font-face in the full CSS")

# Every `ti-*` token referenced in templates (HTML + inline JS) AND in the
# Rust source (some handlers emit icon classes from a match, e.g.
# routes/admin/audit.rs). A stray match inside a word ("multi-host" →
# "ti-host") is dropped below unless it's a real Tabler icon.
SRC = ROOT / "crates/ruscker-admin/src"
used = set()
for f in list(TEMPLATES.rglob("*.html")) + list(SRC.rglob("*.rs")):
    used.update(re.findall(r"ti-[a-z0-9-]+", f.read_text()))

# Keep only tokens that are real Tabler icons (a stray match like the
# `ti-host` inside "multi-host" simply isn't in the map and is dropped).
keep = sorted(i for i in used if i in all_icons)
codepoints = sorted({all_icons[i] for i in keep})
print(f"templates reference {len(used)} ti-* tokens; {len(keep)} are real icons "
      f"({len(codepoints)} unique glyphs)")

# Subset the woff2 to just those code points.
unicodes = ",".join(f"U+{c}" for c in codepoints)
subprocess.run(
    [sys.executable, "-m", "fontTools.subset", str(FULL_WOFF2),
     f"--unicodes={unicodes}", "--flavor=woff2",
     f"--output-file={OUT_WOFF2}", "--no-layout-closure"],
    check=True,
)

# Rebuild a trimmed CSS: license, @font-face, base rule, used icon rules.
license_hdr = full_css[: full_css.index("@font-face")].strip()
rules = "".join(f".{i}:before{{content:\"\\{all_icons[i]}\"}}" for i in keep)
OUT_CSS.write_text(f"{license_hdr}\n{face.group(0)}{base.group(0)}{rules}\n")

print(f"wrote {OUT_WOFF2.name} ({OUT_WOFF2.stat().st_size} bytes) "
      f"and {OUT_CSS.name} ({OUT_CSS.stat().st_size} bytes)")
