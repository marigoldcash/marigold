#!/usr/bin/env python3
"""Keep every copy of the litepaper's text in step with the markdown master.

    marigold-litepaper.md                 the English text master — edit this
    translations/marigold-litepaper.*.md  the same document, one file per language
    translations/chrome.json              the words that belong to the design
    litepaper-web.html                    the web page (published to marigoldcash.github.io)
    litepaper-brochure.html               the print source (built into the PDF)

Copies of the same words in two repositories and eight languages is how a
document ends up disagreeing with itself. This tool makes the markdown
authoritative.

WHAT IS REWRITTEN, AND WHAT IS ONLY CHECKED
-------------------------------------------
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
reported for a person to place properly. The brochure and the PDF are English.

That split is deliberate. Automate what is mechanical; report what is
editorial. The failure this prevents — the website and the PDF quietly saying
different things — is caught either way.

HOW THE OTHER LANGUAGES ARE BUILT
---------------------------------
There is exactly one piece of designed markup, the English one, and every
translation is stamped out of it: the English <article> is cloned and its
prose replaced block for block with the translation's. A translation must
therefore have the *same shape* as the master — same number of headings,
paragraphs and list items, in the same order. That is checked here, and a
language that fails is reported and left out of the page rather than written
in broken. It also means a design change is made once, in English, and every
language gets it on the next run.

The words that live in the design rather than in the master — headline,
strapline, buttons, footer, the lowercase kickers above each heading — come
from translations/chrome.json, including the English ones.

USAGE
-----
    ./sync-litepaper.py --check    report drift, exit 1 if any. Run before publishing.
    ./sync-litepaper.py            rewrite the web page; check the brochure.

After changing the text, run without --check, then rebuild the PDF with
./build-litepaper-pdf.sh, then copy litepaper-web.html to the pages repo as
litepaper/index.html (and the PDF beside it).
"""

from __future__ import annotations

import html
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MASTER = HERE / "marigold-litepaper.md"
WEB = HERE / "litepaper-web.html"
PRINT = HERE / "litepaper-brochure.html"
TRANSLATIONS = HERE / "translations"
CHROME = TRANSLATIONS / "chrome.json"

PROSE_TAGS = ("p", "li", "h2", "h3", "td")

# Blocks the print design deliberately leaves out, reviewed and accepted rather
# than silently tolerated. The closing section is set on the back cover, which
# styles it as a statement and does not repeat the heading above it.
PRINT_OMITS = {"What Marigold Is — and Is Not"}

# Typography is not content. The HTML has been through a typesetter's hands —
# curly apostrophes, non-breaking spaces — and the markdown has not. These are
# folded for COMPARISON so an apostrophe is never reported as drift; the
# rewriter puts the typography back so a fix never downgrades the page.
TYPOGRAPHY = {"’": "'", "‘": "'", "“": '"', "”": '"', " ": " ", "‑": "-"}

# A section of the page that carries one language, and the marker pair the
# generated copies of it are written between.
REGIONS = ("head", "main", "foot")


def normalise(s: str) -> str:
    for fancy, plain in TYPOGRAPHY.items():
        s = s.replace(fancy, plain)
    s = re.sub(r"^[■•·]\s*", "", s.strip())
    s = re.sub(r"\s+", " ", s)
    # Tags are stripped to a space so "<b>...it.</b><span>There" does not
    # become "it.There"; that leaves a space before punctuation where a tag
    # closed mid-sentence ("QR code ."), which is removed here.
    return re.sub(r"\s+([.,;:!?)’])", r"\1", s).strip()


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


def html_elements(text: str, base: int = 0):
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
        out.append((base + start, base + end, text[start:end]))
    return out


def heading_key(kind: str, key: str) -> str:
    """Both designs promote a heading's "Prefix: " to a kicker of their own.

    Chinese writes that colon full-width and without the space, so splitting on
    ": " alone would leave "问题：" sitting in the heading under a kicker that
    already says it."""
    if kind != "h2":
        return key
    for sep in (": ", "：", "：" + " "):
        if sep in key:
            rest = key.split(sep, 1)[1].strip()
            # Spanish, Russian and the rest write sentence case after a colon,
            # so lifting the prefix out leaves a headline starting lowercase.
            # A no-op for English, which is already title case, and for Chinese.
            return rest[:1].upper() + rest[1:]
    return key


def heading_matches(md_key: str, html_key: str) -> bool:
    """A heading the design split differently still matches: "Economics at a
    Glance" becomes kicker "economics" over the title "At a Glance"."""
    if html_key == md_key:
        return True
    # Case-insensitively, because a design that lifts "Economics" out as the
    # kicker recapitalises what is left: "Economics at a Glance" -> "At a Glance".
    return len(html_key) > 6 and md_key.lower().endswith(html_key.lower())


# --------------------------------------------------------------------------
# the page's regions
# --------------------------------------------------------------------------

def markers(text: str, region: str) -> tuple[int, int]:
    """Where a region's generated translations begin and end."""
    open_, close = f"<!-- translations:{region} -->", f"<!-- /translations:{region} -->"
    a, b = text.find(open_), text.find(close)
    if a < 0 or b < 0:
        sys.exit(f"{WEB.name}: the {region} region's translation markers are missing")
    return a + len(open_), b


def english_span(text: str, region: str) -> tuple[int, int]:
    """Where the hand-designed English copy of a region lives."""
    opener = {
        "head": '<div class="head-text" data-lang="en"',
        "main": "<article data-lang=\"en\"",
        "foot": '<div class="foot-text" data-lang="en"',
    }[region]
    start = text.find(opener)
    if start < 0:
        sys.exit(f"{WEB.name}: the English {region} section is missing")
    return start, markers(text, region)[0] - len(f"<!-- translations:{region} -->")


def html_lang(lang: str) -> str:
    return "zh-Hans" if lang == "zh" else lang


# --------------------------------------------------------------------------
# English
# --------------------------------------------------------------------------

def sync_web(md, chrome, write: bool) -> int:
    """Rewrite the English prose from the master, markup untouched, and the
    chrome from chrome.json. Only the English sections are touched here; the
    translations are stamped out of the result afterwards."""
    text = WEB.read_text()
    keys = [normalise(heading_key(k, md_to_text(src))) for k, src in md]
    used, edits, orphans = set(), [], []

    body_start, body_end = english_span(text, "main")
    region = text[body_start:body_end]

    cursor = 0
    for start, end, inner in html_elements(region, base=body_start):
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

    # The English chrome is data like every other language's, so that editing
    # the headline in chrome.json changes the page in all eight.
    text, chrome_edits = apply_chrome(text, chrome)

    if (edits or chrome_edits) and write:
        WEB.write_text(text)

    missing = [keys[i] for i in range(len(md)) if i not in used]
    report(WEB.name + " (english)", len(used), len(edits) + chrome_edits, missing, orphans, write)
    return len(edits) + len(missing) + chrome_edits


def apply_chrome(text: str, chrome) -> tuple[str, int]:
    """Headline, strapline, buttons, footer, title and the data the page's own
    script reads — all from chrome.json, English included."""
    en = chrome["en"]
    before = text

    head_start, head_end = english_span(text, "head")
    text = text[:head_start] + head_html("en", en) + text[head_end:]

    foot_start, foot_end = english_span(text, "foot")
    text = text[:foot_start] + foot_html("en", en) + text[foot_end:]

    text = re.sub(r"<title>.*?</title>", "<title>" + html.escape(en["title"], quote=False) + "</title>", text, count=1, flags=re.S)

    body_start, body_end = english_span(text, "main")
    region = apply_kickers(text[body_start:body_end], en, en)
    region = re.sub(
        r'(<a class="btn gold" href="marigold-litepaper\.pdf" download>).*?(</a>)',
        lambda m: m.group(1) + html.escape(en["pdf_end"], quote=False) + m.group(2),
        region, flags=re.S,
    )
    text = text[:body_start] + region + text[body_end:]

    data = {k: {"title": v["title"], "theme_label": v["theme_label"], "lang_label": v["lang_label"]}
            for k, v in chrome.items() if not k.startswith("_")}
    text = re.sub(
        r'(<script id="chrome-data" type="application/json">).*?(</script>)',
        lambda m: m.group(1) + json.dumps(data, ensure_ascii=False) + m.group(2),
        text, count=1, flags=re.S,
    )

    text = re.sub(r'<span class="sr" id="themeLabel">.*?</span>',
                  '<span class="sr" id="themeLabel">' + html.escape(en["theme_label"], quote=False) + "</span>", text, count=1, flags=re.S)
    text = re.sub(r'<span class="sr" id="langLabel">.*?</span>',
                  '<span class="sr" id="langLabel">' + html.escape(en["lang_label"], quote=False) + "</span>", text, count=1, flags=re.S)

    options = "\n".join(
        f'      <option value="{code}">{html.escape(c["name"], quote=False)}</option>'
        for code, c in chrome.items() if not code.startswith("_")
    )
    text = re.sub(r'(<select id="langSel" aria-labelledby="langLabel">).*?(\n    </select>)',
                  lambda m: m.group(1) + "\n" + options + m.group(2), text, count=1, flags=re.S)

    return text, 0 if text == before else 1


def apply_kickers(region: str, chrome_lang, english) -> str:
    """The lowercase label above each heading. Keyed by the English heading it
    belongs to, so a reworded heading surfaces as a missing kicker rather than
    keeping one that no longer fits."""
    kickers = chrome_lang.get("kickers", {})
    english_headings = list(english.get("kickers", {}).keys())

    def one(m):
        head = normalise(html_to_text(m.group(2)))
        for en_heading in english_headings:
            if normalise(heading_key("h2", en_heading)) == head or normalise(en_heading) == head:
                label = kickers.get(en_heading)
                if label:
                    return f'{m.group(1)}<span class="kicker">{html.escape(label, quote=False)}</span>{m.group(2)}{m.group(3)}'
        return m.group(0)

    region = re.sub(r'(<h2[^>]*>)\s*<span class="kicker">.*?</span>(.*?)(</h2>)', one, region, flags=re.S)
    return re.sub(r'(<div class="rule">\s*<div class="kicker">).*?(</div>)',
                  lambda m: m.group(1) + html.escape(chrome_lang["rule_kicker"], quote=False) + m.group(2),
                  region, flags=re.S)


def head_html(lang: str, c) -> str:
    return (
        f'<div class="head-text" data-lang="{lang}" lang="{html_lang(lang)}">\n'
        f'    <h1>{html.escape(c["h1"], quote=False)}</h1>\n'
        f'    <p class="tag">{html.escape(c["tag"], quote=False)}</p>\n'
        f'    <div class="actions">\n'
        f'      <a class="btn gold" href="marigold-litepaper.pdf" download>{html.escape(c["pdf"], quote=False)}</a>\n'
        f'      <a class="btn line" href="https://marigold.cash/">marigold.cash</a>\n'
        f'    </div>\n'
        f'  </div>\n'
    )


def foot_html(lang: str, c) -> str:
    return (
        f'<div class="foot-text" data-lang="{lang}" lang="{html_lang(lang)}">\n'
        f'    <a href="https://marigold.cash/">marigold.cash</a> &middot; note@marigold.cash<br>\n'
        f'    {html.escape(c["footer1"], quote=False)}<br>\n'
        f'    {html.escape(c["footer2"], quote=False)}\n'
        f'  </div>\n'
    )


# --------------------------------------------------------------------------
# every other language
# --------------------------------------------------------------------------

def translate_region(region: str, lang: str, blocks, english_blocks) -> str:
    """Stamp one language out of the English markup: same design, other words."""
    out = region
    elements = html_elements(region)
    texts = [normalise(html_to_text(i)) for _, _, i in elements]

    # The English article carries exactly the master's blocks in order, so the
    # nth prose element is the nth block. Elements the master does not own (a
    # pull-quote, a chip) do not exist in this page, but guard anyway.
    keys = [normalise(heading_key(k, md_to_text(s))) for k, s in english_blocks]
    pairs, cursor = [], 0
    for (start, end, inner), key in zip(elements, texts):
        if not key:
            continue
        hit = next((i for i, k in enumerate(keys) if k == key and i >= cursor), None)
        if hit is None:
            continue
        cursor = hit + 1
        kind, src = blocks[hit]
        rendered = md_to_html(heading_key(kind, src), bold="b" if "<b>" in inner else "strong")
        pairs.append((start, end, rendered))

    for start, end, rendered in sorted(pairs, key=lambda e: -e[0]):
        out = out[:start] + rendered + out[end:]
    return out


def build_translations(md, chrome, write: bool) -> int:
    text = WEB.read_text()
    langs = [c for c in chrome if not c.startswith("_") and c != "en"]

    body_start, body_end = english_span(text, "main")
    english_article = text[body_start:body_end].rstrip() + "\n"

    built, problems = [], 0
    heads, mains, foots = [], [], []

    for lang in langs:
        path = TRANSLATIONS / f"marigold-litepaper.{lang}.md"
        if not path.exists():
            print(f"  MISSING   no translation file for {lang}: {path.relative_to(HERE)}")
            problems += 1
            continue
        blocks = md_blocks(path.read_text())
        if len(blocks) != len(md) or [k for k, _ in blocks] != [k for k, _ in md]:
            print(f"  SHAPE     {lang}: {len(blocks)} blocks against the master's {len(md)}"
                  f"{' (kinds differ too)' if [k for k, _ in blocks] != [k for k, _ in md][:len(blocks)] else ''}"
                  " — left out of the page until it matches")
            problems += 1
            continue

        c = chrome[lang]
        # Kickers first, while the headings under them are still the English
        # ones this language's kickers are keyed by; the prose swap that
        # follows leaves the kicker spans alone.
        article = apply_kickers(english_article, c, chrome["en"])
        article = translate_region(article, lang, blocks, md)
        article = article.replace('<article data-lang="en">', f'<article data-lang="{lang}" lang="{html_lang(lang)}">', 1)
        # Ids are per language or the document has eight elements called
        # "problem"; a deep link keeps working, in the language it was made in.
        article = re.sub(r'(<h2 id=")([a-z]+)(")', rf'\g<1>\g<2>-{lang}\g<3>', article)
        article = re.sub(
            r'(<a class="btn gold" href="marigold-litepaper\.pdf" download>).*?(</a>)',
            lambda m: m.group(1) + html.escape(c["pdf_end"], quote=False) + m.group(2),
            article, flags=re.S,
        )

        heads.append(head_html(lang, c))
        mains.append(article)
        foots.append(foot_html(lang, c))
        built.append(lang)

    for region, parts in zip(REGIONS, (heads, mains, foots)):
        start, end = markers(text, region)
        joined = ("\n" + "".join(parts)) if parts else "\n"
        text = text[:start] + joined + text[end:]

    if write:
        WEB.write_text(text)

    print(f"\n{WEB.name} (translations): {len(built)} language(s) stamped from the English markup"
          + (f" — {', '.join(built)}" if built else ""))
    return problems


# --------------------------------------------------------------------------

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
    chrome = json.loads(CHROME.read_text())
    langs = [c for c in chrome if not c.startswith("_")]
    print(f"{MASTER.name}: {len(md)} prose block(s) — the master")
    print(f"{CHROME.parent.name}/{CHROME.name}: {len(langs)} language(s) — {', '.join(langs)}")

    missing_kickers = [
        (lang, head) for lang in langs for head in chrome["en"]["kickers"]
        if head not in chrome[lang].get("kickers", {})
    ]
    for lang, head in missing_kickers:
        print(f"  MISSING   {lang} has no kicker for: {head[:70]}")

    drift = sync_web(md, chrome, write=not check)
    drift += build_translations(md, chrome, write=not check)
    drift += check_print(md) + len(missing_kickers)

    if drift and check:
        print(f"\nDrift: {drift}. Run ./sync-litepaper.py, then rebuild the PDF.")
        return 1
    print("\nEvery copy carries the master's text." if not drift else "\nWeb page updated; see the report above for what a person still has to place.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
