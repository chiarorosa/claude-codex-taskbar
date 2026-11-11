"""Tests for the website build: python -m unittest discover -s site"""

import json
import re
import tempfile
import unittest
from html.parser import HTMLParser
from pathlib import Path

import build


class InlineMarkdown(unittest.TestCase):
    def test_code_bold_links(self):
        self.assertEqual(
            build.inline_md("**Bold** `a<b>` [x](https://e.com/?a=1&b=2)"),
            '<strong>Bold</strong> <code>a&lt;b&gt;</code> <a href="https://e.com/?a=1&amp;b=2">x</a>')

    def test_issue_and_user_links(self):
        out = build.inline_md("(#86, thanks @FramY2)")
        self.assertIn(f'<a href="{build.REPO_URL}/issues/86">#86</a>', out)
        self.assertIn('<a href="https://github.com/FramY2">@FramY2</a>', out)

    def test_code_spans_are_left_alone(self):
        self.assertEqual(build.inline_md("`#3 @x **y**`"), "<code>#3 @x **y**</code>")

    def test_email_like_and_anchor_text_not_linked(self):
        self.assertNotIn("<a", build.inline_md("user:password@proxy and page#12"))


class Changelog(unittest.TestCase):
    MD = """# Changelog

Intro text.

## [Unreleased]

- hidden

## [0.4.2] - 2026-11-10

### Changed
- **Faster.** Less CPU
  while idle (#86).
  - nested point
- second

[0.4.2]: https://example.com
"""

    def test_structure(self):
        out = build.changelog_html(self.MD)
        self.assertNotIn("Intro", out)
        self.assertNotIn("hidden", out)
        self.assertNotIn("example.com", out)
        self.assertIn('<h2 id="v0.4.2">0.4.2 <time datetime="2026-11-10">10 Nov 2026</time></h2>', out)
        self.assertIn("<h3>Changed</h3>", out)
        self.assertIn("<li><strong>Faster.</strong> Less CPU while idle", out)
        self.assertIn("<ul><li>nested point</li></ul></li><li>second</li></ul>", out)

    def test_real_changelog_is_balanced(self):
        out = build.changelog_html((build.REPO_DIR / "CHANGELOG.md").read_text(encoding="utf-8"))
        for tag in ("ul", "li", "p", "h2", "h3"):
            self.assertEqual(out.count(f"<{tag}>") + out.count(f"<{tag} "), out.count(f"</{tag}>"), tag)


class Templating(unittest.TestCase):
    def test_fill(self):
        self.assertEqual(build.fill("a {{x}} b", {"x": 1}), "a 1 b")

    def test_unknown_placeholder_fails(self):
        with self.assertRaises(KeyError):
            build.fill("{{nope}}", {})

    def test_parse_page(self):
        meta, body = build.parse_page('<!--\n{"path": "x/"}\n-->\n<p>hi</p>')
        self.assertEqual(meta, {"path": "x/"})
        self.assertEqual(body, "<p>hi</p>\n")


class StructuredData(unittest.TestCase):
    BODY = '<details class="faq"><summary>Is it free?</summary><p>Yes, <b>MIT</b>.</p></details>'

    def test_home_graph(self):
        meta = {"path": "", "kind": "home", "h1": "CCT", "description": "d"}
        data = json.loads(build.jsonld(meta, self.BODY, {"version": "v0.4.2", "exe_bytes": 607000}))
        types = [n["@type"] for n in data["@graph"]]
        self.assertEqual(types, ["WebSite", "SoftwareApplication", "SoftwareSourceCode", "FAQPage"])
        app = data["@graph"][1]
        self.assertEqual(app["softwareVersion"], "0.4.2")
        self.assertEqual(app["offers"]["price"], "0")
        self.assertEqual(data["@graph"][3]["mainEntity"][0]["acceptedAnswer"]["text"], "Yes, MIT .")

    def test_article_graph(self):
        meta = {"path": "a/", "kind": "article", "h1": "H", "description": "d",
                "published": "2026-11-10", "modified": "2026-11-11"}
        graph = json.loads(build.jsonld(meta, "", {}))["@graph"]
        self.assertEqual([n["@type"] for n in graph], ["WebSite", "SoftwareApplication", "TechArticle", "BreadcrumbList"])
        self.assertEqual(graph[2]["dateModified"], "2026-11-11")

    def test_script_breakout_is_escaped(self):
        meta = {"path": "", "kind": "home", "h1": "x", "description": "d"}
        self.assertNotIn("</script", build.jsonld(meta, '<details class="faq"><summary>q</summary>&lt;/script&gt;</details>', {}))


class Sitemap(unittest.TestCase):
    def test_alternates_and_lastmod(self):
        xml = build.sitemap_xml([{"path": "", "modified": "2026-11-10", "alternates": {"en": "", "pt-BR": "pt-br/"}}])
        self.assertIn(f"<loc>{build.SITE_URL}/</loc>", xml)
        self.assertIn('hreflang="pt-BR"', xml)
        self.assertIn('hreflang="x-default"', xml)
        self.assertIn("<lastmod>2026-11-10</lastmod>", xml)
        self.assertNotIn("priority", xml)
        self.assertNotIn("changefreq", xml)

    def test_lastmod_comes_before_extensions(self):
        # sitemaps.org XSD: loc, lastmod, then elements from other namespaces.
        xml = build.sitemap_xml([{"path": "", "modified": "2026-11-10", "alternates": {"en": "", "pt-BR": "pt-br/"}}])
        self.assertLess(xml.index("<loc>"), xml.index("<lastmod>"))
        self.assertLess(xml.index("<lastmod>"), xml.index("<xhtml:link"))


class _Checker(HTMLParser):
    VOID = {"meta", "link", "img", "br", "hr", "input", "source", "wbr"}

    def __init__(self):
        super().__init__()
        self.stack, self.errors, self.ids, self.links, self.imgs, self.h1 = [], [], set(), [], [], 0

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if "id" in a:
            if a["id"] in self.ids:
                self.errors.append(f"duplicate id {a['id']}")
            self.ids.add(a["id"])
        if tag == "a" and a.get("href"):
            self.links.append(a["href"])
        if tag == "img":
            self.imgs.append(a)
        if tag == "h1":
            self.h1 += 1
        if tag not in self.VOID:
            self.stack.append(tag)

    def handle_endtag(self, tag):
        if tag in self.VOID:
            return
        if not self.stack or self.stack[-1] != tag:
            self.errors.append(f"unexpected </{tag}> (open: {self.stack[-3:]})")
            if tag in self.stack:
                while self.stack and self.stack.pop() != tag:
                    pass
            return
        self.stack.pop()


class FullBuild(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.out = Path(cls.tmp.name) / "site"
        cls.pages = build.build(cls.out, offline=True)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_every_page_is_well_formed(self):
        files = list(self.out.rglob("*.html"))
        self.assertGreaterEqual(len(files), 8)
        for f in files:
            c = _Checker()
            c.feed(f.read_text(encoding="utf-8"))
            self.assertEqual(c.errors, [], f)
            self.assertEqual(c.stack, [], f)
            self.assertEqual(c.h1, 1, f"{f}: exactly one <h1>")
            for img in c.imgs:
                self.assertIn("alt", img, f)
                self.assertIn("width", img, f)
                self.assertIn("height", img, f)
                self.assertFalse(img.get("loading") == "lazy" and img.get("fetchpriority") == "high", f)

    def test_internal_links_resolve(self):
        for f in self.out.rglob("*.html"):
            c = _Checker()
            c.feed(f.read_text(encoding="utf-8"))
            for href in c.links:
                if not href.startswith(build.ROOT):
                    continue
                rel = href[len(build.ROOT):].split("#")[0]
                target = self.out / rel
                if rel == "" or rel.endswith("/"):
                    target = target / "index.html"
                self.assertTrue(target.exists(), f"{f}: broken link {href}")

    def test_titles_and_descriptions(self):
        for p in self.pages:
            self.assertLessEqual(len(p["title"]), 70, p["title"])
            self.assertTrue(70 <= len(p["description"]) <= 170, (p["path"], len(p["description"])))

    def test_head_tags(self):
        html_text = (self.out / "index.html").read_text(encoding="utf-8")
        for needle in ('<link rel="canonical" href="https://chiarorosa.github.io/claude-codex-taskbar/">',
                       'hreflang="pt-BR"', 'property="og:image"', 'name="color-scheme" content="light dark"',
                       'application/ld+json'):
            self.assertIn(needle, html_text)
        ld = re.search(r'<script type="application/ld\+json">(.*?)</script>', html_text, re.S).group(1)
        json.loads(ld)

    def test_translations_point_at_each_other(self):
        by_path = {p["path"]: p for p in self.pages}
        for p in self.pages:
            for lang, path in p.get("alternates", {}).items():
                self.assertIn(path, by_path, f"{p['path']}: alternate {path} is not a page")
                self.assertEqual(by_path[path].get("lang", "en"), lang, path)
                self.assertEqual(by_path[path].get("alternates"), p["alternates"],
                                 f"{p['path']} and {path} must list the same alternates")

    def test_language_switch_opens_the_same_page(self):
        guide = (self.out / "pt-br" / "trocar-contas-claude-code" / "index.html").read_text(encoding="utf-8")
        self.assertIn(f'<a class="lang" href="{build.ROOT}switch-claude-code-accounts/"', guide)
        changelog = (self.out / "changelog" / "index.html").read_text(encoding="utf-8")
        self.assertIn(f'<a class="lang" href="{build.ROOT}pt-br/"', changelog)

    def test_site_pages_are_prefetched_on_hover(self):
        html_text = (self.out / "index.html").read_text(encoding="utf-8")
        rules = re.search(r'<script type="speculationrules">(.*?)</script>', html_text, re.S)
        self.assertIsNotNone(rules)
        rule = json.loads(rules.group(1))["prefetch"][0]
        self.assertEqual(rule["where"]["href_matches"], build.ROOT + "*")
        self.assertEqual(rule["eagerness"], "moderate")

    def test_support_files(self):
        for name in ("sitemap.xml", "llms.txt", ".nojekyll", "favicon.svg", "img/screenshot.png"):
            self.assertTrue((self.out / name).exists(), name)
        self.assertNotIn("404", (self.out / "sitemap.xml").read_text(encoding="utf-8"))


class IndexNow(unittest.TestCase):
    def test_one_valid_key_file_ships_with_the_site(self):
        import indexnow
        key = indexnow.find_key(build.HERE / "static")
        self.assertRegex(key, r"^[0-9a-f]{32}$")

    def test_payload_keeps_only_site_urls(self):
        import indexnow
        xml = ("<urlset><url><loc>https://chiarorosa.github.io/claude-codex-taskbar/</loc></url>"
               "<url><loc>https://example.com/x</loc></url></urlset>")
        body = indexnow.payload("a" * 32, indexnow.sitemap_urls(xml))
        self.assertEqual(body["host"], "chiarorosa.github.io")
        self.assertEqual(body["keyLocation"], f"{build.SITE_URL}/{'a' * 32}.txt")
        self.assertEqual(body["urlList"], [f"{build.SITE_URL}/"])

    def test_empty_sitemap_is_an_error(self):
        import indexnow
        with self.assertRaises(ValueError):
            indexnow.payload("a" * 32, [])


if __name__ == "__main__":
    unittest.main()
