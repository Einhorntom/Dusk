"""Sets (or checks) Dusk's version everywhere it is recorded.

The version lives in four files: the workspace Cargo.toml (every crate,
the Settings "About" line, `dusk version`), the C# Directory.Build.props,
the PowerToys Run plugin.json and the Command Palette AppxManifest.xml
(as x.y.z.0).

Usage:
  python scripts/set_version.py 1.2.3           set it everywhere
  python scripts/set_version.py --check 1.2.3   fail unless every file has it
                                                (the release workflow checks
                                                the tag this way)
"""

import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))

# (file, pattern with one group for the version, how the version is written)
PLACES = [
    ('Cargo.toml', r'(?ms)^\[workspace\.package\].*?^version = "([^"]+)"', '{v}'),
    ('integrations/Directory.Build.props', r'<Version>([^<]+)</Version>', '{v}'),
    ('integrations/PowerToysRun/plugin.json', r'"Version": "([^"]+)"', '{v}'),
    ('integrations/CommandPalette/AppxManifest.xml',
     r'<Identity Name="Dusk\.CommandPalette"[^>]*? Version="([^"]+)"', '{v}.0'),
]


def main(arguments):
    check = arguments[:1] == ['--check']
    if check:
        arguments = arguments[1:]
    if len(arguments) != 1 or not re.fullmatch(r'\d+\.\d+\.\d+', arguments[0]):
        sys.exit(__doc__)
    version = arguments[0]
    problems = []
    for name, pattern, form in PLACES:
        path = os.path.join(ROOT, name)
        text = open(path, encoding='utf-8').read()
        match = re.search(pattern, text)
        if not match:
            problems.append(f'{name}: version not found')
            continue
        wanted = form.format(v=version)
        if check:
            if match.group(1) != wanted:
                problems.append(f'{name}: {match.group(1)}, expected {wanted}')
            continue
        start, end = match.span(1)
        text = text[:start] + wanted + text[end:]
        open(path, 'w', encoding='utf-8', newline='').write(text)
        print(f'{name}: {wanted}')
    if problems:
        sys.exit('version mismatch:\n  ' + '\n  '.join(problems))
    if check:
        print(f'every file has version {version}')


if __name__ == '__main__':
    main(sys.argv[1:])
