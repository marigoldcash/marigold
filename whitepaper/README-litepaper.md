# The litepaper, and how to change it

One document, eight languages, three copies, two repositories. `marigold-litepaper.md` is the master; everything else follows from it.

| file | what it is | how it stays in step |
|---|---|---|
| `marigold-litepaper.md` | the English text master | you edit this |
| `translations/marigold-litepaper.<lang>.md` | the same document in another language | a translator edits this |
| `translations/chrome.json` | the words that belong to the design, not the text: headline, strapline, buttons, footer, the lowercase kickers — in every language, English included | you edit this |
| `litepaper-web.html` | the web page, all eight languages in one file | **rewritten** by `sync-litepaper.py` |
| `litepaper-brochure.html` | print source for the PDF (English) | **checked** by `sync-litepaper.py` |
| `marigold-litepaper.pdf` | the download (English) | built by `build-litepaper-pdf.sh` |
| `marigoldcash.github.io:litepaper/` | what the public sees | copied, see below |

## Changing the text

```sh
$EDITOR marigold-litepaper.md      # 1. the words, only here
./sync-litepaper.py                # 2. web page follows; brochure reports
./build-litepaper-pdf.sh           # 3. rebuild the PDF
./sync-litepaper.py --check        # 4. must be clean before publishing
```

A change to the English master leaves the seven translations saying the old thing. The sync cannot detect that — it only checks *shape* — so a text change means either retranslating the affected blocks or accepting that the other languages lag, deliberately and briefly.

Then publish, from a checkout of `marigoldcash/marigoldcash.github.io`:

```sh
cp litepaper-web.html      <pages>/litepaper/index.html
cp marigold-litepaper.pdf  <pages>/litepaper/marigold-litepaper.pdf
```

## Day, night, and the language picker

Both controls live in the page's top row and need nothing from you. The theme is a set of CSS variables with three states — the reader's choice, stored; failing that, the operating system's; failing that, day — and the toggle writes `data-theme` on `<html>`. The language works the same way with `data-doclang`, which is why switching is instant and cannot flash the wrong language: every language is already in the file, and CSS shows one. First visit picks up `?lang=es`, then a stored choice, then the browser's own language, then English.

With JavaScript off the page reads in English and cannot switch. That is the whole degradation.

## Adding a language

1. Translate `marigold-litepaper.md` into `translations/marigold-litepaper.<lang>.md`, keeping **the same shape**: the same number of headings, paragraphs and list items, in the same order, with the same `**bold**` spans. The generator maps block *n* of the translation onto block *n* of the English markup, so shape is the contract. `./sync-litepaper.py` reports a translation whose shape has drifted and leaves that language out of the page rather than writing it in broken.
2. Add a block for it in `translations/chrome.json` — copy the English one and translate every value. Kickers are keyed by the **English** heading they sit above, so a reworded heading shows up as a missing kicker instead of silently keeping one that no longer fits.
3. Add the language to the `LANGS` list in the page's head script and to the language rules in its stylesheet, both of which name the codes explicitly.
4. Run `./sync-litepaper.py`. The new language is stamped out of the English markup and appears in the picker.

The PDF stays English, which is why every other language's download button says so.

## Why the brochure is checked and not rewritten

The web page renders the markdown almost one for one, so it can be rewritten safely. The brochure does not: it restructures the same words into a two-column economics table, numbered "props", a latency comparison drawn as chips, and pull-quote plates with hand-placed line breaks and gold emphasis. Rewriting it positionally would fight the design and could silently damage a published document, so `sync-litepaper.py` reports what it is missing and a person places it.

Both paths catch the failure that matters — the website and the PDF quietly saying different things.

## What the report means

- **MISSING** — the master says something this file does not. Real drift; fix it.
- **SHAPE** — a translation no longer has the master's block structure. It is left out of the page until it does.
- **EDITORIAL** — text in the file that is not in the master: the cover strapline, the date line, a pull-quote that rearranges a sentence. Expected, and listed so a new one is never mistaken for content.

`PRINT_OMITS` in the script records blocks the print design deliberately drops — currently just the closing section's heading, which the back cover styles as a statement without repeating it. Reviewed exceptions, not silent tolerance.
