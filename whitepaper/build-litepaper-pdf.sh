#!/usr/bin/env sh
# Typeset the litepaper brochure PDF — headless Chromium in docker (same
# zero-install philosophy as build-pdf.sh), in three steps:
#
#   1. litepaper-brochure.html -> content PDF (cream flowed pages with @page
#      margins; full-bleed dark cover/back via a named @page with margin 0).
#   2. brochure-frame.html -> a one-page cream "letterhead" (full-bleed sheet
#      + the marigold·litepaper strip in the top margin).
#   3. Stamp the frame UNDER every content page (pypdf). Chromium cannot paint
#      @page margin areas from inside a document — fixed elements are clipped
#      to the content box — but it leaves those areas transparent in the PDF,
#      so an underlay shows through exactly there. The opaque dark pages
#      (cover/back) cover the frame entirely on their own.
#
# The HTML's TEXT comes from marigold-litepaper.md, which is the content
# master. Do not edit the words in the HTML: edit the markdown, then run
# ./sync-litepaper.py, which rewrites the web page's prose and reports anything
# in the brochure that a person needs to place by hand. ./sync-litepaper.py
# --check exits non-zero if any copy has drifted; run it before publishing.
set -eu
cd "$(dirname "$0")"

python3 -c "import pypdf" 2>/dev/null || python3 -m pip install --user --quiet --break-system-packages pypdf

CHROME="docker run --rm -v $PWD:/work zenika/alpine-chrome \
  --no-sandbox --headless --disable-gpu --no-pdf-header-footer --virtual-time-budget=15000"

$CHROME --print-to-pdf=/work/.content.pdf file:///work/litepaper-brochure.html >/dev/null 2>&1
$CHROME --print-to-pdf=/work/.frame.pdf   file:///work/brochure-frame.html    >/dev/null 2>&1
$CHROME --print-to-pdf=/work/.back.pdf    file:///work/brochure-back.html     >/dev/null 2>&1

python3 - << 'EOF'
from pypdf import PdfReader, PdfWriter
content = PdfReader('.content.pdf')
writer = PdfWriter()
for page in content.pages:
    base = PdfReader('.frame.pdf').pages[0]   # fresh frame per page
    base.merge_page(page)                      # content over the frame
    writer.add_page(base)
writer.add_page(PdfReader('.back.pdf').pages[0])  # full-bleed back cover, appended
with open('marigold-litepaper.pdf', 'wb') as f:
    writer.write(f)
EOF
rm -f .content.pdf .frame.pdf .back.pdf

echo "wrote whitepaper/marigold-litepaper.pdf"
