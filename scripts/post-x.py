#!/usr/bin/env python3
"""Post to X as @marigoldcash through the "Marigold Cash" app (made 2026-10-10).

    scripts/post-x.py "The website has a new face: https://marigold.cash"
    scripts/post-x.py < post.txt                 # the text from stdin
    DRY_RUN=1 scripts/post-x.py "text"           # print, send nothing

The four credentials are read from a file and never passed on the command
line, printed, or kept in the repository: ~/.config/marigold/x-api.env
(mode 600, placed by the founder), or the path in MARIGOLD_X_ENV_FILE, with
one KEY=value per line:

    X_API_KEY=        the app's consumer key
    X_API_SECRET=     its secret
    X_ACCESS_TOKEN=   the access token for @marigoldcash (read and write)
    X_ACCESS_SECRET=  its secret

The request is OAuth 1.0a user context, signed here with the standard library
so nothing beyond python3 is needed. X counts a post's length its own way
(every link is 23 characters, some scripts count double); a plain post over
280 characters is refused before sending unless X_LONG=1 says the account's
longer limit applies. On success the post's link is printed.
"""
import base64
import hashlib
import hmac
import json
import os
import re
import secrets
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

ENV_FILE = os.environ.get("MARIGOLD_X_ENV_FILE", os.path.expanduser("~/.config/marigold/x-api.env"))
ENDPOINT = "https://api.x.com/2/tweets"
HANDLE = "marigoldcash"
NEEDED = ("X_API_KEY", "X_API_SECRET", "X_ACCESS_TOKEN", "X_ACCESS_SECRET")


def credentials() -> dict:
    try:
        with open(ENV_FILE) as f:
            lines = f.read().splitlines()
    except OSError:
        sys.exit(f"credentials file {ENV_FILE} is missing or unreadable")
    creds = {}
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, _, v = line.partition("=")
        creds[k.strip()] = v.strip().strip('"').strip("'")
    missing = [k for k in NEEDED if not creds.get(k)]
    if missing:
        sys.exit(f"{ENV_FILE} lacks {', '.join(missing)}")
    return creds


def pct(s: str) -> str:
    return urllib.parse.quote(s, safe="-._~")


def x_length(text: str) -> int:
    """X's count, near enough: a link is 23 characters whatever its length."""
    url = re.compile(r"https?://\S+")
    return len(url.sub("x" * 23, text))


def authorization(creds: dict, method: str, url: str) -> str:
    oauth = {
        "oauth_consumer_key": creds["X_API_KEY"],
        "oauth_nonce": secrets.token_hex(16),
        "oauth_signature_method": "HMAC-SHA1",
        "oauth_timestamp": str(int(time.time())),
        "oauth_token": creds["X_ACCESS_TOKEN"],
        "oauth_version": "1.0",
    }
    # a JSON body contributes no parameters to the signature; only the oauth_* ones do
    params = "&".join(f"{pct(k)}={pct(v)}" for k, v in sorted(oauth.items()))
    base = "&".join([method.upper(), pct(url), pct(params)])
    key = f"{pct(creds['X_API_SECRET'])}&{pct(creds['X_ACCESS_SECRET'])}".encode()
    oauth["oauth_signature"] = base64.b64encode(hmac.new(key, base.encode(), hashlib.sha1).digest()).decode()
    return "OAuth " + ", ".join(f'{pct(k)}="{pct(v)}"' for k, v in sorted(oauth.items()))


def main() -> int:
    text = " ".join(sys.argv[1:]) if len(sys.argv) > 1 else sys.stdin.read()
    text = text.rstrip("\n")
    if not text.strip():
        sys.exit("nothing to post")
    n = x_length(text)
    if n > 280 and not os.environ.get("X_LONG"):
        sys.exit(f"the post counts {n} characters for X; 280 is the limit (X_LONG=1 if the account allows more)")
    if os.environ.get("DRY_RUN"):
        print(f"would post as @{HANDLE} ({n} characters):\n{text}")
        return 0
    creds = credentials()
    body = json.dumps({"text": text}).encode()
    req = urllib.request.Request(ENDPOINT, data=body, method="POST")
    req.add_header("Authorization", authorization(creds, "POST", ENDPOINT))
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            reply = json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")
        try:
            d = json.loads(detail)
            detail = d.get("detail") or d.get("title") or detail
        except ValueError:
            pass
        sys.exit(f"X refused ({e.code}): {detail}")
    post_id = reply.get("data", {}).get("id")
    if not post_id:
        sys.exit(f"unexpected answer: {json.dumps(reply)[:300]}")
    print(f"posted: https://x.com/{HANDLE}/status/{post_id}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
