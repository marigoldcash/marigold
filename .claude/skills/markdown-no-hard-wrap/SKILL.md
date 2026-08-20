---
name: markdown-no-hard-wrap
description: Markdown line-wrapping convention for this repo — one line per paragraph and per list item, no hard-wrapping at a fixed column. Consult before writing or editing ANY .md file in this repository (docs/x-fork/, FORK-PLAN.md, CONTRIBUTING.md, README.md), even for a small edit, since a hard-wrapped paragraph reflows the whole thing on the next edit and creates a noisy diff.
---

# No hard-wrapped Markdown

When writing or editing prose in this repository's Markdown files, put each paragraph
on **one line**, however long, and let the reader's editor/viewer soft-wrap it. Do not
insert a line break every ~80 characters. The same applies inside list items: a bullet's
text is one line (however long), not wrapped across several indented continuation
lines.

## Why

Hard-wrapping and git diffs actively fight each other: edit one word in the middle of a
hard-wrapped paragraph and every following line in that paragraph reflows, so a
one-word change shows up as a multi-line diff. One line per paragraph means an edit to
a sentence touches exactly the one line it's in — nothing else. This matters more here
than in most repos: FORK-PLAN.md and docs/x-fork/ are living documents edited
continuously across many sessions, and this project already cares a lot about clean,
single-concern diffs (see the git history's commit discipline). Soft-wrap in the
reader's own editor/viewer costs nothing; hard-wrap in the source costs a clean diff
every time.

## What stays untouched

Only prose paragraphs and list-item text get unwrapped. Everything else keeps its
existing line structure exactly as CommonMark/GFM requires it:

- **Tables** — one row per line, always (this repo's tables already are).
- **Fenced code blocks** (` ``` `) — verbatim, byte-for-byte, including any wrapping
  inside them.
- **Headers, horizontal rules, blockquote markers** — their own line.
- **Nested lists** — each item's marker line holds that item's own text unwrapped;
  nesting/indentation structure is preserved, only the hard-wrap within an item's text
  is removed.

## Files this applies to

`FORK-PLAN.md`, everything under `docs/x-fork/`, `CONTRIBUTING.md`, and `README.md`.
**Not** the inherited-upstream docs (`docs/crescendo-guide.md`, `docs/archival.md`,
`docs/override-params.md`, `docs/testnet10-transition.md`, `docs/testnet12.md`,
`docs/toccata-guide.md`, `bridge/docs/README.md`) or anything under `target/` —
those are upstream-owned and this project's convention doesn't apply to them.

## Writing new content

Just write it unwrapped — type (or generate) each paragraph and each list item as one
line. No tooling needed for a normal edit; this is a style choice you apply directly
while writing, the same way you'd choose not to add a trailing space.

## Bulk-reformatting an existing hard-wrapped file

If you ever need to convert a whole file (or a large chunk) that's still hard-wrapped,
don't hand-join lines with a naive script — Markdown's list/table/code-fence structure
is too easy to mis-join by regex, and a plain `prettier --prose-wrap never` pass will
*also* silently restyle things nobody asked for: it normalizes `*italic*` to `_italic_`
and reflows table-cell padding even when the table was already one line per row. The
reliable recipe (used to convert this repo's docs on 2026-08-19) is:

1. Mask, in order: table blocks (whole contiguous `|`-line runs, replaced with a
   placeholder), fenced code blocks, inline code spans, then single-asterisk emphasis
   spans (`*word*`, including ones with a nested `**bold**` inside) — each replaced
   with a placeholder or sentinel so prettier's parser never sees the real content.
   Do the emphasis-masking pass over the *whole* document text, not line-by-line —
   an emphasis span can straddle a hard-wrapped line break, and a per-line regex will
   miss half of it.
2. Run `npx prettier --parser markdown --prose-wrap never --embedded-language-formatting off`
   on the masked text.
3. Unmask everything back in, in reverse order.
4. Verify before trusting the result: strip all whitespace from both the original and
   the output (`re.sub(r'\s+', ' ', text)`) and diff — they must be identical. If they
   aren't, look for a literal `+` (or another bullet character) sitting at the start of
   a wrapped line in the original; CommonMark parses that as a genuine list-item marker
   regardless of authorial intent, which prettier will then "fix" by picking its
   canonical bullet character — this is a pre-existing latent bug in the source file,
   not a tooling bug, and the fix is a one-line content edit (reword, or join that one
   spot by hand) before re-running the tool.
