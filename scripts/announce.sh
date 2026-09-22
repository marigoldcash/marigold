#!/usr/bin/env bash
#
# Post to the Marigold Telegram channel through the release bot
# (t.me/marigoldcash_release_bot, made 2026-09-22).
#
#   scripts/announce.sh "v2.60.270 is out: ..."         # one message
#   scripts/announce.sh < notes.txt                      # the text from stdin
#   scripts/announce.sh --to @somechannel "text"         # another channel
#   DRY_RUN=1 scripts/announce.sh "text"                 # print, send nothing
#
# The bot token is read from a file and never passed on the command line or
# kept in the repository: ~/.config/marigold/announce-bot.token (mode 600), or
# the path in MARIGOLD_ANNOUNCE_TOKEN_FILE. The channel comes from --to, or
# MARIGOLD_ANNOUNCE_CHANNEL, or ~/.config/marigold/announce-channel. Messages
# are plain text with link previews on, so a release link shows its card.
# Telegram caps a message at 4096 characters; longer text is refused here
# rather than cut mid-sentence.
set -euo pipefail
TOKEN_FILE="${MARIGOLD_ANNOUNCE_TOKEN_FILE:-$HOME/.config/marigold/announce-bot.token}"
CHANNEL="${MARIGOLD_ANNOUNCE_CHANNEL:-}"
if [ "${1:-}" = "--to" ]; then CHANNEL="$2"; shift 2; fi
if [ -z "$CHANNEL" ] && [ -f "$HOME/.config/marigold/announce-channel" ]; then
  CHANNEL="$(head -1 "$HOME/.config/marigold/announce-channel")"
fi
[ -n "$CHANNEL" ] || { echo "no channel: give --to @handle, or write it to ~/.config/marigold/announce-channel" >&2; exit 2; }
if [ $# -gt 0 ]; then TEXT="$*"; else TEXT="$(cat)"; fi
TEXT="${TEXT%$'\n'}"
[ -n "$TEXT" ] || { echo "nothing to post" >&2; exit 2; }
[ ${#TEXT} -le 4096 ] || { echo "message is ${#TEXT} characters; Telegram takes 4096" >&2; exit 2; }
if [ -n "${DRY_RUN:-}" ]; then
  printf 'would post to %s:\n%s\n' "$CHANNEL" "$TEXT"
  exit 0
fi
[ -r "$TOKEN_FILE" ] || { echo "token file $TOKEN_FILE is missing or unreadable" >&2; exit 2; }
TOKEN="$(head -1 "$TOKEN_FILE" | tr -d '[:space:]')"
# The token goes into the URL path, as Telegram's API requires; curl is told
# to read the whole URL from a config on stdin so it never shows in `ps`.
RESPONSE="$(printf 'url = "https://api.telegram.org/bot%s/sendMessage"\n' "$TOKEN" | curl -sS --config - \
  --data-urlencode "chat_id=$CHANNEL" \
  --data-urlencode "text=$TEXT")"
unset TOKEN
if printf '%s' "$RESPONSE" | grep -q '"ok":true'; then
  echo "posted to $CHANNEL"
else
  # Telegram's error descriptions never contain the token.
  echo "Telegram refused: $(printf '%s' "$RESPONSE" | sed -n 's/.*"description":"\([^"]*\)".*/\1/p')" >&2
  exit 1
fi
