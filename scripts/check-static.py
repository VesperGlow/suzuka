#!/usr/bin/env python3
"""Check static output, local links, site identity, SEO, feeds, and sitemaps."""

import json
from datetime import datetime, timezone
from html.parser import HTMLParser
from pathlib import Path
import re
import sys
import tomllib
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET

VOID_TAGS = {'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link',
             'meta', 'param', 'source', 'track', 'wbr'}


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []
        self.forms = 0
        self.meta = {}
        self.canonical = ''
        self.title = ''
        self.brand = ''
        self.footer = ''
        self.jsonld = []
        self.stack = []
        self.script = None

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'form' and attrs.get('method', 'get').lower() != 'get':
            self.forms += 1
        for key in ('src', 'href', 'poster'):
            if attrs.get(key):
                self.links.append(attrs[key])
        if attrs.get('srcset'):
            self.links.extend(item.strip().split()[0] for item in attrs['srcset'].split(','))
        if tag == 'meta':
            self.meta[attrs.get('property') or attrs.get('name') or attrs.get('itemprop')] = attrs.get('content', '')
        if tag == 'link' and attrs.get('rel') == 'canonical':
            self.canonical = attrs.get('href', '')
        if tag == 'script' and attrs.get('type') == 'application/ld+json':
            self.script = ''
        if tag not in VOID_TAGS:
            self.stack.append((tag, set(attrs.get('class', '').split())))

    def handle_endtag(self, tag):
        if tag == 'script' and self.script is not None:
            self.jsonld.append(json.loads(self.script))
            self.script = None
        for index in range(len(self.stack) - 1, -1, -1):
            if self.stack[index][0] == tag:
                del self.stack[index:]
                break

    def handle_data(self, data):
        if self.script is not None:
            self.script += data
        if any(tag == 'title' for tag, _ in self.stack):
            self.title += data
        if any('site-brand' in classes for _, classes in self.stack):
            self.brand += data
        if any('site-footer' in classes for _, classes in self.stack):
            self.footer += data


def check(root):
    source = Path(__file__).resolve().parents[1]
    config = tomllib.loads((source / 'site.toml').read_text())
    title = config['title']
    origin = config['base_url'].rstrip('/')
    host = urlsplit(origin).netloc
    errors = set()
    pages = sorted(root.rglob('*.html'))
    if not pages:
        errors.add('No generated HTML pages')

    def expect(condition, message):
        if not condition:
            errors.add(message)

    def check_link(href, path, require_site=False):
        url = urlsplit(href)
        rel = path.relative_to(root)
        if require_site:
            expect(href.startswith(origin + '/'), f'Wrong deployment URL in {rel}: {href}')
        if (url.scheme or url.netloc) and url.netloc != host:
            return
        if not url.path:
            return
        base = root if url.path.startswith('/') else path.parent
        target = base / unquote(url.path).lstrip('/')
        expect(target.is_file() or (target / 'index.html').is_file(),
               f'Broken local reference in {rel}: {href}')

    for path in root.rglob('*'):
        if not path.is_file():
            continue
        rel = path.relative_to(root)
        expect('guestbook' not in str(rel) and path.name != 'csp-hashes.txt',
               f'Obsolete runtime artifact: {rel}')
        if path.suffix not in {'.html', '.js', '.json', '.css', '.xml'}:
            continue
        text = path.read_text(encoding='utf-8')
        expect('/api/' not in text, f'Runtime API dependency: {rel}')
        if path.suffix == '.html':
            page = Page()
            page.feed(text)
            expect(not page.forms, f'Form requiring a write endpoint: {rel}')
            expect(page.title.endswith(title), f'Incorrect browser title: {rel}')
            check_link(page.canonical, path, require_site=True)
            for href in page.links:
                check_link(href, path)
            if page.brand:  # Alias stubs intentionally have no page chrome.
                expect(' '.join(page.brand.split()) == title, f'Incorrect header brand: {rel}')
                expect(' '.join(page.footer.split()) == f'© {datetime.now(timezone.utc).year} {title}',
                       f'Incorrect footer identity: {rel}')
                expect(page.meta.get('author') == title, f'Incorrect author metadata: {rel}')
                expect(page.meta.get('og:site_name') == title, f'Incorrect Open Graph identity: {rel}')
                for key in ('og:image', 'twitter:image', 'image'):
                    if page.meta.get(key):
                        check_link(page.meta[key], path, require_site=True)
            for data in page.jsonld:
                if data.get('@type') != 'BlogPosting':
                    continue
                expect(data['author']['name'] == title, f'Incorrect JSON-LD author: {rel}')
                expect(data['publisher']['name'] == title, f'Incorrect JSON-LD publisher: {rel}')
                if data['author'].get('url'):
                    check_link(data['author']['url'], path, require_site=True)
                check_link(data['mainEntityOfPage']['@id'], path, require_site=True)
                for image in data.get('image', []):
                    check_link(image, path, require_site=True)
        if path.suffix == '.css':
            for href in re.findall(r'url\([\'"]?([^\)\'\"]+)', text):
                check_link(href, path)
        if path.name in {'index.json', 'feed.json'} and 'pagefind' not in rel.parts:
            data = json.loads(text)
            if path.name == 'feed.json':
                expect(data['title'] == title, f'Incorrect JSON Feed title: {rel}')
                expect(data['authors'] == [{'name': title}], f'Incorrect JSON Feed author: {rel}')
                check_link(data['home_page_url'], path, require_site=True)
                check_link(data['feed_url'], path, require_site=True)
            for item in data['items']:
                check_link(item['url'], path, require_site=path.name == 'feed.json')
                if item.get('image'):
                    check_link(item['image'], path, require_site=True)
        if path.suffix == '.xml':
            document = ET.fromstring(text)
            channel = document.find('channel')
            if channel is not None:
                expect(channel.findtext('title', '').endswith(title), f'Incorrect RSS identity: {rel}')
                expect(channel.findtext('copyright', '').endswith(title), f'Incorrect RSS copyright: {rel}')
                check_link(channel.findtext('link', ''), path, require_site=True)
                for item in channel.findall('item'):
                    check_link(item.findtext('link', ''), path, require_site=True)
            else:
                for element in document.iter():
                    if element.tag.endswith('}loc') and element.text:
                        check_link(element.text, path, require_site=True)
                    if element.tag.endswith('}link') and element.get('href'):
                        check_link(element.get('href'), path, require_site=True)
    for error in sorted(errors):
        print(error, file=sys.stderr)
    if errors:
        return 1
    print(f'Static output OK: {len(pages)} HTML pages; links, identity, SEO, feeds and sitemaps checked')
    return 0


if __name__ == '__main__':
    sys.exit(check(Path(sys.argv[1] if len(sys.argv) > 1 else 'public')))
