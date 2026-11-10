#!/usr/bin/env python3
"""Tell Bing (and the other IndexNow search engines) which site pages changed.

Run after a deploy. Reads the live sitemap, so only published URLs are sent, and
submits them with the public key in site/static/<key>.txt. Standard library only.

    python site/indexnow.py            # submit
    python site/indexnow.py --dry-run  # print the payload
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.request
from pathlib import Path
from urllib.parse import urlsplit

HERE = Path(__file__).resolve().parent
SITE_URL = "https://chiarorosa.github.io/claude-codex-taskbar"
ENDPOINT = "https://api.indexnow.org/indexnow"
KEY_RE = re.compile(r"^[0-9a-f]{32}$")


def find_key(static: Path) -> str:
    """The IndexNow key: the one <32 hex>.txt in site/static whose content is its own name."""
    keys = [p.stem for p in static.glob("*.txt") if KEY_RE.match(p.stem)
            and p.read_text(encoding="utf-8").strip() == p.stem]
    if len(keys) != 1:
        raise SystemExit(f"expected exactly one IndexNow key file in {static}, found {len(keys)}")
    return keys[0]


def sitemap_urls(xml: str) -> list[str]:
    return re.findall(r"<loc>([^<]+)</loc>", xml)


def payload(key: str, urls: list[str]) -> dict:
    """IndexNow body. The key file sits under the site path, so it covers every site URL."""
    prefix = SITE_URL + "/"
    urls = [u for u in urls if u.startswith(prefix)]
    if not urls:
        raise ValueError("no site URLs to submit")
    return {"host": urlsplit(SITE_URL).hostname, "key": key,
            "keyLocation": f"{SITE_URL}/{key}.txt", "urlList": urls}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()
    key = find_key(HERE / "static")
    with urllib.request.urlopen(f"{SITE_URL}/sitemap.xml", timeout=20) as r:
        body = payload(key, sitemap_urls(r.read().decode("utf-8")))
    if args.dry_run:
        print(json.dumps(body, indent=2))
        return 0
    req = urllib.request.Request(ENDPOINT, data=json.dumps(body).encode("utf-8"), method="POST",
                                 headers={"Content-Type": "application/json; charset=utf-8"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            print(f"IndexNow: HTTP {r.status}, {len(body['urlList'])} URLs submitted")
    except urllib.error.HTTPError as e:
        # 200/202 are success; anything else is reported but must not fail the deploy.
        print(f"::warning::IndexNow answered HTTP {e.code}: {e.reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
