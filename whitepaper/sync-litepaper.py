#!/usr/bin/env python3
"""Keep every copy of the litepaper's text in step with the markdown master.

    marigold-litepaper.md      the text master — edit this, always
    litepaper-web.html         the web page (published to marigoldcash.github.io)
    litepaper-brochure.html    the print source (built into the PDF)

Three copies of the same words in two repositories is how a document ends up
disagreeing with itself. This tool makes the markdown authoritative.

WHY THE TWO HTML FILES ARE TREATED DIFFERENTLY
----------------------------------------------
The web page renders the markdown almost one for one: heading, paragraph,
list, in the same order. So it is *rewritten* from the markdown — change a
sentence in the master, run this, and the page follows.

The print brochure does not. It is a designed artefact that restructures the
same words: the economics list becomes a two-column table, the numbered points
become numbered "props", the latency comparison becomes chips, and sentences
are lifted out as pull-quote plates with hand-placed line breaks and gold
emphasis. Rewriting it positionally would fight the design and could silently
wreck a published document, so it is *checked* instead: every sentence of the
markdown must appear in it somewhere, and anything that has drifted is
reported for a person to place properly.

That split is deliberate. Automate what is mechanical; report what is
editorial. The failure this prevents — the website and the PDF quietly saying
different things — is caught either way.

USAGE
-----
    ./sync-litepaper.py --check    report drift, exit 1 if any. Run before publishing.
    ./sync-litepaper.py            rewrite the web page from the markdown; check the brochure.

After changing the text, run without --check, then rebuild the PDF with
./build-litepaper-pdf.sh, then copy litepaper-web.html to the pages repo as
litepaper/index.html (and the PDF beside it).
"""

from __future__ import annotations

import html
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MASTER = HERE / "marigold-litepaper.md"
WEB = HERE / "litepaper-web.html"
PRINT = HERE / "litepaper-brochure.html"

PROSE_TAGS = ("p", "li", "h2", "h3", "td")

# Blocks the print design deliberately leaves out, reviewed and accepted rather
# than silently tolerated. The closing section is set on the back cover, which
# styles it as a statement and does not repeat the heading above it.
PRINT_OMITS = {"What Marigold Is — and Is Not"}

# Typography is not content. The HTML has been through a typesetter's hands —
# curly apostrophes, non-breaking spaces — and the markdown has not. These are
# folded for COMPARISON so an apostrophe is never reported as drift; the
# rewriter puts the typography back so a fix never downgrades the page.
TYPOGRAPHY = {"’": "'", "‘": "'", "“": '"', "”": '"', " ": " ", "‑": "-"}


def normalise(s: str) -> str:
    for fancy, plain in TYPOGRAPHY.items():
        s = s.replace(fancy, plain)
    s = re.sub(r"^[■•·]\s*", "", s.strip())
    s = re.sub(r"\s+", " ", s)
    # Tags are stripped to a space so "<b>...it.</b><span>There" does not
    # become "it.There"; that leaves a space before punctuation where a tag
    # closed mid-sentence ("QR code ."), which is removed here.
    return re.sub(r"\s+([.,;:!?)\u2019])", r"\1", s).strip()


def typeset(s: str) -> str:
    """The typography the HTML uses, so a rewrite never flattens it."""
    return re.sub(r"(?<=\w)'(?=\w)", "’", s)


def md_to_text(s: str) -> str:
    s = re.sub(r"\*\*(.+?)\*\*", r"\1", s)
    s = re.sub(r"\*(.+?)\*", r"\1", s)
    return re.sub(r"\[(.+?)\]\(.+?\)", r"\1", s)


def html_to_text(s: str) -> str:
    # <br> becomes a space (it separates words); every other tag becomes
    # nothing, or "<strong>QR code</strong>." would read as "QR code ."
    return html.unescape(re.sub(r"<[^>]+>", " ", re.sub(r"<br\s*/?>", " ", s)))


def md_to_html(s: str, bold: str = "strong") -> str:
    s = html.escape(typeset(s), quote=False)
    s = re.sub(r"\*\*(.+?)\*\*", rf"<{bold}>\1</{bold}>", s)
    return re.sub(r"\*(.+?)\*", r"<em>\1</em>", s)


def md_blocks(text: str) -> list[tuple[str, str]]:
    """The master's prose, in order, as (kind, markdown source)."""
    blocks: list[tuple[str, str]] = []
    for raw in re.split(r"\n\s*\n", text):
        block = raw.strip()
        if not block or block == "---" or block.startswith("# "):
            continue
        if block.startswith("## "):
            blocks.append(("h2", block[3:].strip()))
        elif block.startswith("### "):
            blocks.append(("h3", block[4:].strip()))
        elif re.match(r"^([-*]|\d+\.)\s", block):
            for line in block.splitlines():
                item = re.sub(r"^([-*]|\d+\.)\s+", "", line.strip())
                if item:
                    blocks.append(("li", item))
        else:
            blocks.append(("p", " ".join(l.strip() for l in block.splitlines())))
    return blocks


def html_elements(text: str):
    """Prose-bearing elements, in order, as (start, end, inner).

    A heading's editorial kicker sits inside the <h2> but is not the markdown's
    to own, so it is excluded from the tracked span.
    """
    out = []
    for m in re.finditer(rf"<({'|'.join(PROSE_TAGS)})(?:\s[^>]*)?>(.*?)</\1>", text, re.S):
        start, end = m.start(2), m.end(2)
        kicker = re.match(r'\s*<span class="kicker">.*?</span>', text[start:end], re.S)
        if kicker:
            start += kicker.end()
        out.append((start, end, text[start:end]))
    return out


def heading_key(kind: str, key: str) -> str:
    """Both designs promote a heading's "Prefix: " to a kicker of their own."""
    return key.split(": ", 1)[1] if kind == "h2" and ": " in key else key


def heading_matches(md_key: str, html_key: str) -> bool:
    """A heading the design split differently still matches: "Economics at a
    Glance" becomes kicker "economics" over the title "At a Glance"."""
    if html_key == md_key:
        return True
    # Case-insensitively, because a design that lifts "Economics" out as the
    # kicker recapitalises what is left: "Economics at a Glance" -> "At a Glance".
    return len(html_key) > 6 and md_key.lower().endswith(html_key.lower())


def sync_web(md, write: bool) -> int:
    """Rewrite the web page's prose from the master, markup untouched."""
    text = WEB.read_text()
    keys = [normalise(heading_key(k, md_to_text(src))) for k, src in md]
    used, edits, orphans = set(), [], []

    cursor = 0
    for start, end, inner in html_elements(text):
        key = normalise(html_to_text(inner))
        if not key:
            continue
        hit = next((i for i, k in enumerate(keys) if k == key and i >= cursor), None)
        if hit is None:
            hit = next((i for i, k in enumerate(keys) if k == key and i not in used), None)
        if hit is None:
            hit = next((i for i, k in enumerate(keys) if i not in used and heading_matches(k, key)), None)
        if hit is None:
            orphans.append(key)
            continue
        used.add(hit)
        cursor = hit + 1
        rendered = md_to_html(heading_key(md[hit][0], md[hit][1]), bold="b" if "<b>" in inner else "strong")
        if "<br>" in inner:  # a hand-placed break carries no words; keep it
            rendered = inner
        if rendered != inner:
            edits.append((start, end, rendered))

    for start, end, rendered in sorted(edits, key=lambda e: -e[0]):
        text = text[:start] + rendered + text[end:]
    if edits and write:
        WEB.write_text(text)

    missing = [keys[i] for i in range(len(md)) if i not in used]
    report(WEB.name, len(used), len(edits), missing, orphans, write)
    return len(edits) + len(missing)


def check_print(md) -> int:
    """The brochure restructures the text by design, so it is checked for
    containment rather than rewritten: every sentence of the master must be in
    it somewhere, however the designer chose to lay it out."""
    # The brochure's closing section is set on the back cover, which is its
    # own file stamped into the PDF — so the "print copy" is both files.
    back = HERE / "brochure-back.html"
    text = PRINT.read_text() + (back.read_text() if back.exists() else "")
    haystack = normalise(html_to_text(text))
    present = {normalise(html_to_text(inner)) for _, _, inner in html_elements(text) if inner.strip()}

    missing = []
    for kind, src in md:
        key = normalise(heading_key(kind, md_to_text(src)))
        if not key or key in PRINT_OMITS:
            continue
        # Trailing punctuation is the typesetter's: a display line that drops
        # its full stop is a design choice, not a change of words.
        if key in haystack or key.rstrip(".") in haystack:
            continue
        # A "**Key:** value" list item becomes two table cells in print.
        if ": " in key:
            k, v = key.split(": ", 1)
            if k in haystack and v in haystack:
                continue
        missing.append(key)

    orphans = [p for p in present if p and p not in normalise(md_to_text(MASTER.read_text()))]
    report(PRINT.name, len(md) - len(missing), 0, missing, orphans, write=False, checked_only=True)
    return len(missing)


def report(name, tracked, edits, missing, orphans, write, checked_only=False):
    verb = "rewritten" if write and not checked_only else "out of date"
    print(f"\n{name}: {tracked} block(s) carried" + (f", {edits} {verb}" if edits or not checked_only else ""))
    for key in missing:
        print(f"  MISSING   the master says this and the file does not: {key[:88]}")
    for key in orphans[:12]:
        print(f"  EDITORIAL not from the master (pull-quote, chip, cover): {key[:70]}")
    if len(orphans) > 12:
        print(f"            ... and {len(orphans) - 12} more")


def main() -> int:
    check = "--check" in sys.argv
    md = md_blocks(MASTER.read_text())
    print(f"{MASTER.name}: {len(md)} prose block(s) — the master")
    drift = sync_web(md, write=not check) + check_print(md)
    if drift and check:
        print(f"\nDrift: {drift}. Run ./sync-litepaper.py, then rebuild the PDF.")
        return 1
    print("\nEvery copy carries the master's text." if not drift else "\nWeb page updated; see MISSING above for the brochure.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
