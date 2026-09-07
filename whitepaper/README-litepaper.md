# The litepaper, and how to change it

Three copies of the same words, in two repositories. `marigold-litepaper.md`
is the master; everything else follows from it.

| file | what it is | how it stays in step |
|---|---|---|
| `marigold-litepaper.md` | the text master | you edit this |
| `litepaper-web.html` | the web page | **rewritten** by `sync-litepaper.py` |
| `litepaper-brochure.html` | print source for the PDF | **checked** by `sync-litepaper.py` |
| `marigold-litepaper.pdf` | the download | built by `build-litepaper-pdf.sh` |
| `marigoldcash.github.io:litepaper/` | what the public sees | copied, see below |

## Changing the text

```sh
$EDITOR marigold-litepaper.md      # 1. the words, only here
./sync-litepaper.py                # 2. web page follows; brochure reports
./build-litepaper-pdf.sh           # 3. rebuild the PDF
./sync-litepaper.py --check        # 4. must be clean before publishing
```

Then publish, from a checkout of `marigoldcash/marigoldcash.github.io`:

```sh
cp litepaper-web.html      <pages>/litepaper/index.html
cp marigold-litepaper.pdf  <pages>/litepaper/marigold-litepaper.pdf
```

## Why the brochure is checked and not rewritten

The web page renders the markdown almost one for one, so it can be rewritten
safely. The brochure does not: it restructures the same words into a two-column
economics table, numbered "props", a latency comparison drawn as chips, and
pull-quote plates with hand-placed line breaks and gold emphasis. Rewriting it
positionally would fight the design and could silently damage a published
document, so `sync-litepaper.py` reports what it is missing and a person places
it.

Both paths catch the failure that matters — the website and the PDF quietly
saying different things.

## What the report means

- **MISSING** — the master says something this file does not. Real drift; fix it.
- **EDITORIAL** — text in the file that is not in the master: the cover
  strapline, the date line, the web page's subtitle. Expected, and listed so a
  new one is never mistaken for content.

`PRINT_OMITS` in the script records blocks the print design deliberately drops
— currently just the closing section's heading, which the back cover styles as
a statement without repeating it. Reviewed exceptions, not silent tolerance.
