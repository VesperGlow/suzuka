#!/usr/bin/env python3
"""Check that templates, translations, and published source assets are used."""

from pathlib import Path
import re
import sys
import tomllib


def check(root):
    errors = []
    config = tomllib.loads((root / 'site.toml').read_text())
    templates = {
        path.relative_to(root / 'ssg/templates').as_posix(): path.read_text()
        for path in (root / 'ssg/templates').rglob('*.html')
    }
    rust = '\n'.join(path.read_text() for path in (root / 'ssg/src').glob('*.rs'))
    source = '\n'.join(templates.values()) + '\n' + rust
    used_keys = set(re.findall(r'\bt\("([^"]+)"', source))
    used_keys.update(re.findall(r'\.translate\([^\n]*?"([^"]+)"', rust))
    for lang in config['languages']:
        table = tomllib.loads((root / 'i18n' / f'{lang}.toml').read_text())
        for key in sorted(set(table) - used_keys):
            errors.append(f'Unused translation: {lang}/{key}')
        for key in sorted(used_keys - set(table)):
            errors.append(f'Missing translation: {lang}/{key}')
    reached = set()
    pending = set(re.findall(r'"([^"\n]+\.html)"', rust)) & templates.keys()
    while pending:
        name = pending.pop()
        if name in reached:
            continue
        reached.add(name)
        refs = re.findall(r'{%\s*(?:include|extends)\s+"([^"]+)"', templates[name])
        for ref in refs:
            if ref not in templates:
                errors.append(f'Missing template in {name}: {ref}')
            else:
                pending.add(ref)
    for name in sorted(templates.keys() - reached):
        errors.append(f'Unreachable template: {name}')
    for path in (root / 'assets').rglob('*'):
        if not path.is_file():
            continue
        rel = path.relative_to(root / 'assets').as_posix()
        if rel == 'js/theme.js':  # Explicitly read by assets.rs for inline publication.
            continue
        if f'asset("{rel}")' not in source:
            errors.append(f'Unreferenced asset: {rel}')
    references = source + (root / 'site.toml').read_text()
    for path in (root / 'static').rglob('*'):
        if path.is_file() and path.name != '.gitkeep' and path.relative_to(root / 'static').as_posix() not in references:
            errors.append(f'Unreferenced static resource: {path.relative_to(root)}')
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        return 1
    print(f'Source OK: {len(reached)} templates; translations and published assets are referenced')
    return 0


if __name__ == '__main__':
    sys.exit(check(Path(sys.argv[1] if len(sys.argv) > 1 else '.')))
