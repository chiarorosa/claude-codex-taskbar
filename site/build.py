#!/usr/bin/env python3
"""Build the CCT website (GitHub Pages) into an output folder.

Standard library only, so the build needs nothing but Python 3.10+.
Pages live in site/pages/ as HTML fragments that start with a JSON metadata
comment. The build wraps them in site/layout.html, fills in live facts
(latest release, star count, open help-wanted issues, changelog) and writes
sitemap.xml, llms.txt and the images.

Network is optional. Without it (or without GITHUB_TOKEN) the build falls back
to local git data and leaves the live numbers out, so it always succeeds.

    python site/build.py --out _site
"""

from __future__ import annotations

import argparse
import datetime as dt
import html
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

REPO = "chiarorosa/claude-codex-taskbar"
REPO_URL = f"https://github.com/{REPO}"
SITE_URL = "https://chiarorosa.github.io/claude-codex-taskbar"
ROOT = "/claude-codex-taskbar/"
HERE = Path(__file__).resolve().parent
REPO_DIR = HERE.parent
AUTHOR = {"@type": "Person", "name": "Pablo Chiaro Rosa", "url": "https://github.com/chiarorosa"}
IMAGES = ["hero-accounts.svg", "screenshot.png", "screenshot-settings.png",
          "screenshot-pc-usage.png", "social-preview.png"]

NAV = {
    "en": [("home", "", "Home"), ("claude", "claude-code-usage-windows/", "Claude usage"),
           ("codex", "codex-usage-limits-windows/", "Codex usage"),
           ("accounts", "switch-claude-code-accounts/", "Switch accounts"),
           ("changelog", "changelog/", "Changelog"), ("contribute", "contribute/", "Contribute")],
    "pt-BR": [("home", "pt-br/", "Início"), ("claude", "claude-code-usage-windows/", "Uso do Claude (en)"),
              ("accounts", "switch-claude-code-accounts/", "Trocar contas (en)"),
              ("changelog", "changelog/", "Novidades (en)"), ("contribute", "contribute/", "Contribuir (en)")],
}
UI = {
    "en": {"skip": "Skip to content", "star": "Star on GitHub", "nav": "Main",
           "footer": "CCT is not affiliated with, endorsed by, or sponsored by Anthropic or OpenAI. "
                     "“Claude”, “Claude Code” and “Codex” are trademarks of their owners.",
           "made": "Free and open source (MIT) by", "source": "Source", "lang_name": "Português", "lang_path": "pt-br/"},
    "pt-BR": {"skip": "Pular para o conteúdo", "star": "Estrela no GitHub", "nav": "Principal",
              "footer": "O CCT não é afiliado, endossado nem patrocinado pela Anthropic ou pela OpenAI. "
                        "“Claude”, “Claude Code” e “Codex” são marcas de seus donos.",
              "made": "Gratuito e de código aberto (MIT), por", "source": "Código", "lang_name": "English", "lang_path": ""},
}

META_RE = re.compile(r"\A\s*<!--(.*?)-->", re.S)
VAR_RE = re.compile(r"\{\{(\w+)\}\}")
FAQ_RE = re.compile(r'<details class="faq">\s*<summary>(.*?)</summary>(.*?)</details>', re.S)
TAG_RE = re.compile(r"<[^>]+>")


# ---------------------------------------------------------------- pure helpers

def parse_page(text: str) -> tuple[dict, str]:
    """Split a page into its JSON metadata comment and its HTML body."""
    m = META_RE.match(text)
    if not m:
        raise ValueError("page has no metadata comment")
    return json.loads(m.group(1)), text[m.end():].strip() + "\n"


def fill(template: str, ctx: dict) -> str:
    """Replace {{name}} placeholders; an unknown name is an error, not a blank."""
    def sub(m: re.Match) -> str:
        key = m.group(1)
        if key not in ctx:
            raise KeyError(f"unknown placeholder {{{{{key}}}}}")
        return str(ctx[key])
    return VAR_RE.sub(sub, template)


def inline_md(text: str) -> str:
    """Render the inline Markdown the changelog uses: code, bold, links, #issues, @users."""
    parts = re.split(r"(`[^`]+`)", text)
    out = []
    for part in parts:
        if part.startswith("`") and part.endswith("`") and len(part) > 1:
            out.append(f"<code>{html.escape(part[1:-1], quote=False)}</code>")
            continue
        s = html.escape(part, quote=False)
        s = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", s)
        s = re.sub(r"\[([^\]]+)\]\(([^)\s]+)\)", r'<a href="\2">\1</a>', s)
        s = re.sub(r"(?<![\w/&;])#(\d+)\b", rf'<a href="{REPO_URL}/issues/\1">#\1</a>', s)
        s = re.sub(r"(?<![\w/:.])@([A-Za-z0-9][A-Za-z0-9-]*)", r'<a href="https://github.com/\1">@\1</a>', s)
        out.append(s)
    return "".join(out)


def _render_list(items: list[tuple[int, str]]) -> str:
    out, depth = [], -1
    for level, text in items:
        level = min(level, depth + 1)
        if level > depth:
            out.append("<ul>")
            depth = level
        else:
            out.append("</li>")
            while depth > level:
                out.append("</ul></li>")
                depth -= 1
        out.append("<li>" + inline_md(text))
    out.append("</li>")
    while depth > 0:
        out.append("</ul></li>")
        depth -= 1
    out.append("</ul>")
    return "".join(out)


def human_date(iso: str, lang: str = "en") -> str:
    d = dt.date.fromisoformat(iso[:10])
    if lang == "pt-BR":
        months = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"]
        return f"{d.day} {months[d.month - 1]} {d.year}"
    return f"{d.day} {d.strftime('%b')} {d.year}"


def changelog_html(md: str) -> str:
    """Render a Keep a Changelog file. Skips the intro and the Unreleased section."""
    out: list[str] = []
    items: list[tuple[int, str]] = []
    para: list[str] = []
    skip, started = False, False

    def flush() -> None:
        if items:
            out.append(_render_list(items))
            items.clear()
        if para:
            out.append("<p>" + inline_md(" ".join(para)) + "</p>")
            para.clear()

    for line in md.splitlines():
        if line.startswith("## "):
            flush()
            started = True
            title = line[3:].strip()
            m = re.match(r"\[?([^\]\s]+)\]?\s*-\s*(\d{4}-\d{2}-\d{2})", title)
            skip = title.lower().strip("[]") == "unreleased"
            if skip:
                continue
            if m:
                ver, date = m.groups()
                out.append(f'<h2 id="v{html.escape(ver)}">{html.escape(ver)} '
                           f'<time datetime="{date}">{human_date(date)}</time></h2>')
            else:
                out.append(f"<h2>{inline_md(title.strip('[]'))}</h2>")
            continue
        if not started or skip or re.match(r"^\[[^\]]+\]:\s", line):
            continue
        if line.startswith("### "):
            flush()
            out.append(f"<h3>{inline_md(line[4:].strip())}</h3>")
            continue
        m = re.match(r"^(\s*)[-*] (.*)", line)
        if m:
            if para:
                flush()
            items.append((len(m.group(1)) // 2, m.group(2).strip()))
        elif not line.strip():
            continue
        elif line.startswith(" ") and items:
            level, text = items[-1]
            items[-1] = (level, text + " " + line.strip())
        else:
            if items:
                flush()
            para.append(line.strip())
    flush()
    return "\n".join(out)


def faq_entries(body: str) -> list[tuple[str, str]]:
    """Question and answer text of every <details class="faq"> block."""
    def text(s: str) -> str:
        return html.unescape(re.sub(r"\s+", " ", TAG_RE.sub(" ", s))).strip()
    return [(text(q), text(a)) for q, a in FAQ_RE.findall(body)]


def page_url(path: str) -> str:
    return f"{SITE_URL}/{path}"


def jsonld(meta: dict, body: str, facts: dict) -> str:
    """Structured data for a page, as one JSON-LD @graph. Matches the visible content."""
    url = page_url(meta["path"])
    lang = meta.get("lang", "en")
    app = {
        "@type": "SoftwareApplication",
        "@id": f"{SITE_URL}/#app",
        "name": "CCT (claude-codex-taskbar)",
        "alternateName": ["CCT", "claude-codex-taskbar"],
        "description": "Windows 11 and Windows 10 taskbar widget that shows Claude Code and OpenAI Codex "
                       "5-hour and weekly usage limits, tells you which account is active, and switches "
                       "Claude Code accounts by hand. Single native Rust executable under 1 MB, no telemetry.",
        "url": f"{SITE_URL}/",
        "applicationCategory": "DeveloperApplication",
        "applicationSubCategory": "Usage monitor",
        "operatingSystem": "Windows 11, Windows 10",
        "processorRequirements": "x64",
        "isAccessibleForFree": True,
        "offers": {"@type": "Offer", "price": "0", "priceCurrency": "USD"},
        "license": "https://opensource.org/licenses/MIT",
        "downloadUrl": f"{REPO_URL}/releases/latest",
        "releaseNotes": f"{SITE_URL}/changelog/",
        "screenshot": f"{SITE_URL}/img/screenshot.png",
        "image": f"{SITE_URL}/img/social-preview.png",
        "inLanguage": ["en", "pt-BR"],
        "author": AUTHOR,
        "maintainer": AUTHOR,
        "sameAs": [REPO_URL],
    }
    if facts.get("version"):
        app["softwareVersion"] = facts["version"].lstrip("v")
    if facts.get("release_date"):
        app["dateModified"] = facts["release_date"]
    if facts.get("exe_bytes"):
        app["fileSize"] = f"{round(facts['exe_bytes'] / 1000)} KB"
    graph: list[dict] = [
        {"@type": "WebSite", "@id": f"{SITE_URL}/#website", "url": f"{SITE_URL}/",
         "name": "CCT", "inLanguage": ["en", "pt-BR"], "publisher": AUTHOR},
        app,
    ]
    kind = meta.get("kind", "page")
    if kind == "home":
        graph.append({"@type": "SoftwareSourceCode", "@id": f"{SITE_URL}/#source",
                      "name": "claude-codex-taskbar", "codeRepository": REPO_URL,
                      "programmingLanguage": "Rust", "license": "https://opensource.org/licenses/MIT",
                      "targetProduct": {"@id": f"{SITE_URL}/#app"}, "author": AUTHOR})
    else:
        page = {"@type": "TechArticle" if kind == "article" else "WebPage",
                "@id": f"{url}#page", "url": url, "inLanguage": lang,
                "isPartOf": {"@id": f"{SITE_URL}/#website"}, "about": {"@id": f"{SITE_URL}/#app"}}
        if kind == "article":
            page.update({"headline": meta["h1"], "description": meta["description"], "author": AUTHOR,
                         "datePublished": meta["published"], "dateModified": meta["modified"]})
        else:
            page["name"] = meta["h1"]
        graph.append(page)
        graph.append({"@type": "BreadcrumbList", "itemListElement": [
            {"@type": "ListItem", "position": 1, "name": "CCT", "item": f"{SITE_URL}/"},
            {"@type": "ListItem", "position": 2, "name": meta["h1"], "item": url}]})
    faqs = faq_entries(body)
    if faqs:
        graph.append({"@type": "FAQPage", "@id": f"{url}#faq", "mainEntity": [
            {"@type": "Question", "name": q, "acceptedAnswer": {"@type": "Answer", "text": a}}
            for q, a in faqs]})
    data = {"@context": "https://schema.org", "@graph": graph}
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).replace("</", "<\\/")


def sitemap_xml(pages: list[dict]) -> str:
    rows = ['<?xml version="1.0" encoding="UTF-8"?>',
            '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" '
            'xmlns:xhtml="http://www.w3.org/1999/xhtml">']
    for p in pages:
        rows.append("  <url>")
        rows.append(f"    <loc>{html.escape(page_url(p['path']))}</loc>")
        for lang, path in sorted(p.get("alternates", {}).items()):
            rows.append(f'    <xhtml:link rel="alternate" hreflang="{lang}" href="{html.escape(page_url(path))}"/>')
        if p.get("alternates"):
            rows.append(f'    <xhtml:link rel="alternate" hreflang="x-default" '
                        f'href="{html.escape(page_url(p["alternates"]["en"]))}"/>')
        rows.append(f"    <lastmod>{p['modified']}</lastmod>")
        rows.append("  </url>")
    rows.append("</urlset>")
    return "\n".join(rows) + "\n"


def nav_html(lang: str, current: str) -> str:
    links = []
    for key, path, label in NAV[lang]:
        cur = ' aria-current="page"' if key == current else ""
        links.append(f'<li><a href="{ROOT}{path}"{cur}>{label}</a></li>')
    return "\n        ".join(links)


def issues_html(issues: list[dict] | None) -> str:
    if not issues:
        return (f'<p>See the <a href="{REPO_URL}/issues?q=is%3Aopen+label%3A%22help+wanted%22">'
                f'open “help wanted” issues</a> on GitHub.</p>')
    rows = []
    for it in issues[:12]:
        labels = "".join(f'<span class="tag">{html.escape(lb)}</span>' for lb in it["labels"])
        rows.append(f'<li><a href="{html.escape(it["url"])}">#{it["number"]} {html.escape(it["title"])}</a> {labels}</li>')
    return '<ul class="issues">\n' + "\n".join(rows) + "\n</ul>"


def size_label(n: int | None) -> str:
    return f"{round(n / 1000)} KB .exe" if n else "under 1 MB"


# ---------------------------------------------------------------- data sources

def _api(path: str) -> object | None:
    token = os.environ.get("GITHUB_TOKEN")
    req = urllib.request.Request(f"https://api.github.com/{path}",
                                 headers={"Accept": "application/vnd.github+json",
                                          "User-Agent": "cct-site-build"})
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.load(r)
    except Exception as e:  # noqa: BLE001 - offline builds are fine
        print(f"note: GitHub API {path} unavailable ({e}); building without it", file=sys.stderr)
        return None


def _git(*args: str) -> str:
    try:
        return subprocess.run(["git", *args], cwd=REPO_DIR, capture_output=True, text=True,
                              check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return ""


def last_modified(path: Path) -> str:
    iso = _git("log", "-1", "--format=%cI", "--", str(path.relative_to(REPO_DIR)))
    return iso[:10] if iso else dt.date.today().isoformat()


def gather_facts(offline: bool) -> dict:
    facts: dict = {"version": "", "release_date": "", "exe_bytes": None, "stars": None, "issues": None}
    rel = None if offline else _api(f"repos/{REPO}/releases/latest")
    if isinstance(rel, dict) and rel.get("tag_name"):
        facts["version"] = rel["tag_name"]
        facts["release_date"] = (rel.get("published_at") or "")[:10]
        for a in rel.get("assets", []):
            if a.get("name") == "claude-codex-taskbar.exe":
                facts["exe_bytes"] = a.get("size")
    else:
        tag = _git("describe", "--tags", "--abbrev=0")
        if tag:
            facts["version"] = tag
            facts["release_date"] = _git("log", "-1", "--format=%cI", tag)[:10]
    if not offline:
        repo = _api(f"repos/{REPO}")
        if isinstance(repo, dict):
            facts["stars"] = repo.get("stargazers_count")
        found: dict[int, dict] = {}
        for label in ("good first issue", "help wanted"):
            data = _api(f"repos/{REPO}/issues?state=open&per_page=50&labels={label.replace(' ', '%20')}")
            for it in data if isinstance(data, list) else []:
                if "pull_request" in it:
                    continue
                found[it["number"]] = {"number": it["number"], "title": it["title"], "url": it["html_url"],
                                       "labels": [lb["name"] for lb in it.get("labels", [])
                                                  if lb["name"] in ("good first issue", "help wanted")]}
        if found:
            facts["issues"] = sorted(found.values(), key=lambda i: ("good first issue" not in i["labels"], -i["number"]))
    return facts


# ---------------------------------------------------------------- build

def build(out: Path, offline: bool = False) -> list[dict]:
    facts = gather_facts(offline)
    layout = (HERE / "layout.html").read_text(encoding="utf-8")
    css = (HERE / "style.css").read_text(encoding="utf-8")
    changelog = changelog_html((REPO_DIR / "CHANGELOG.md").read_text(encoding="utf-8"))
    today = dt.date.today().isoformat()
    verification = ""
    for name, env in (("google-site-verification", "GOOGLE_SITE_VERIFICATION"),
                      ("msvalidate.01", "BING_SITE_VERIFICATION")):
        if os.environ.get(env):
            verification += f'<meta name="{name}" content="{html.escape(os.environ[env])}">\n'

    if out.exists():
        shutil.rmtree(out)
    (out / "img").mkdir(parents=True)
    for name in IMAGES:
        shutil.copy2(REPO_DIR / "docs" / name, out / "img" / name)
    for f in (HERE / "static").iterdir():
        shutil.copy2(f, out / f.name)
    (out / ".nojekyll").write_text("", encoding="utf-8")

    built = []
    for src in sorted((HERE / "pages").rglob("*.html")):
        meta, body = parse_page(src.read_text(encoding="utf-8"))
        lang = meta.get("lang", "en")
        modified = last_modified(src)
        if meta.get("kind") == "home" or meta.get("live"):
            modified = max(modified, facts["release_date"] or modified)
        meta.setdefault("published", modified)
        meta["modified"] = modified
        stars = facts["stars"]
        ctx = {
            "root": ROOT, "repo_url": REPO_URL, "site_url": SITE_URL,
            "release_url": f"{REPO_URL}/releases/latest",
            "exe_url": f"{REPO_URL}/releases/latest/download/claude-codex-taskbar.exe",
            "version": facts["version"] or "latest",
            "release_date_human": human_date(facts["release_date"], lang) if facts["release_date"] else "",
            "exe_size": size_label(facts["exe_bytes"]),
            "star_count": f'<span class="count">{stars}</span>' if stars else "",
            "changelog": changelog, "issues": issues_html(facts["issues"]),
            "modified_human": human_date(modified, lang), "modified": modified,
            "year": today[:4],
        }
        body = fill(body, ctx)
        alternates = meta.get("alternates", {})
        alt_links = "".join(f'<link rel="alternate" hreflang="{k}" href="{page_url(v)}">\n'
                            for k, v in sorted(alternates.items()))
        if alternates:
            alt_links += f'<link rel="alternate" hreflang="x-default" href="{page_url(alternates["en"])}">\n'
        ui = UI[lang]
        page = fill(layout, {
            **ctx,
            "lang": lang, "title": html.escape(meta["title"]),
            "description": html.escape(meta["description"]),
            "canonical": page_url(meta["path"]), "alternates": alt_links,
            "og_type": "article" if meta.get("kind") == "article" else "website",
            "og_locale": "pt_BR" if lang == "pt-BR" else "en_US",
            "robots": '<meta name="robots" content="noindex">\n' if meta.get("noindex") else "",
            "verification": verification, "css": css, "jsonld": jsonld(meta, body, facts),
            "nav": nav_html(lang, meta.get("nav", "")), "body": body,
            "skip": ui["skip"], "star": ui["star"], "nav_label": ui["nav"], "footer": ui["footer"],
            "made": ui["made"], "source": ui["source"], "lang_name": ui["lang_name"],
            "lang_href": ROOT + ui["lang_path"], "lang_other": "en" if lang == "pt-BR" else "pt-BR",
        })
        dest = out / meta["path"] / "index.html" if not meta.get("file") else out / meta["file"]
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(page, encoding="utf-8")
        if not meta.get("noindex"):
            built.append({**meta, "body": body})

    (out / "sitemap.xml").write_text(sitemap_xml(built), encoding="utf-8")
    llms = (REPO_DIR / "llms.txt").read_text(encoding="utf-8").rstrip() + "\n\n## Website\n\n"
    llms += "".join(f"- [{p['h1']}]({page_url(p['path'])}): {p['description']}\n"
                    for p in built if p.get("lang", "en") == "en")
    (out / "llms.txt").write_text(llms, encoding="utf-8")
    return built


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", type=Path, default=REPO_DIR / "_site")
    ap.add_argument("--offline", action="store_true", help="skip the GitHub API")
    args = ap.parse_args()
    pages = build(args.out.resolve(), args.offline)
    print(f"built {len(pages)} indexable pages into {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
