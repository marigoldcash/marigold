#!/usr/bin/env sh
# Typeset the whitepaper PDF from the markdown master, via the pandoc/extra
# docker image — no local TeX/typst install needed (see LAUNCH-PLAN 3.1).
#
# The markdown file stays the single source of truth (it renders on GitHub and
# feeds the website); this script only adapts it for print: the byline line is
# lifted out of the body and passed to pandoc as title-block metadata (author +
# date under the title, above the TOC), and the H1 becomes the document title
# via --shift-heading-level-by=-1. Font note: the image's fontconfig doesn't
# index its texmf fonts, so Source Serif 4 is loaded by *filename* through
# kpathsea — and it covers every non-ASCII character the paper uses (verified:
# zero missing-character warnings; Latin Modern, the default, lacks ⁸ and ≈).
#
# The generated PDF is gitignored for now — regenerate at will; decide at
# publication time whether a versioned PDF gets committed or released as an
# artifact.
set -eu
cd "$(dirname "$0")"

SRC=marigold-whitepaper.md
BYLINE=$(grep -m1 '^\*\*The Marigold Project\*\*' "$SRC" | sed 's/\*\*//g')
AUTHOR=$(printf '%s\n' "$BYLINE" | awk -F' · ' '{print $1 " · " $2 " · " $3}')
DATE=$(printf '%s\n' "$BYLINE" | awk -F' · ' '{out=$4; for (i=5;i<=NF;i++) out=out " · " $i; print out}')

grep -v '^\*\*The Marigold Project\*\*' "$SRC" > .build-src.md
trap 'rm -f .build-src.md' EXIT

docker run --rm -v "$PWD:/data" pandoc/extra:latest \
  .build-src.md -o marigold-whitepaper.pdf \
  --pdf-engine=xelatex --toc --shift-heading-level-by=-1 \
  -M author="$AUTHOR" -M date="$DATE" \
  -V geometry:margin=2.5cm -V fontsize=11pt -V colorlinks=true \
  -V mainfont="SourceSerif4-Regular.otf" \
  -V mainfontoptions="BoldFont=SourceSerif4-Bold.otf" \
  -V mainfontoptions="ItalicFont=SourceSerif4-RegularIt.otf" \
  -V mainfontoptions="BoldItalicFont=SourceSerif4-BoldIt.otf"

echo "wrote whitepaper/marigold-whitepaper.pdf ($AUTHOR — $DATE)"
