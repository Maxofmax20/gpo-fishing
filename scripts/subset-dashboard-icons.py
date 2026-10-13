"""Subset the dashboard icon font to static and dynamic icon names.
Requires fonttools[woff]. Supply the full Material Symbols font as --source
when adding icons; the original is also recoverable from repository history.
"""
import argparse
import re
from pathlib import Path
from fontTools import subset
from fontTools.ttLib import TTFont

parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--output', type=Path, default=Path('src-tauri/src/bot/web/assets/fonts/sym1.woff2'))
args = parser.parse_args()
font = TTFont(args.source)
reverse = {glyph: chr(code) for code, glyph in font.getBestCmap().items() if code < 128}
ligatures = {}
subtables = []
for lookup in font['GSUB'].table.LookupList.Lookup:
    for raw in lookup.SubTable:
        table = getattr(raw, 'ExtSubTable', raw)
        if not hasattr(table, 'ligatures'):
            continue
        subtables.append(table)
        for first, items in table.ligatures.items():
            for item in items:
                try:
                    name = ''.join(reverse[g] for g in [first] + item.Component)
                except KeyError:
                    continue
                ligatures[name] = item.LigGlyph
root = Path('src-tauri/src/bot/web')
used = set()
for path in list(root.glob('*.html')) + list((root / 'assets').glob('gpo-*.js')):
    text = path.read_text(encoding='utf-8')
    used.update(re.findall(r'<span[^>]*class="[^"]*material-symbols-outlined[^"]*"[^>]*>\s*([a-z0-9_]+)\s*</span>', text))
    used.update(name for name in re.findall(r"['\"]([a-z0-9_]+)['\"]", text) if name in ligatures)
missing = used - ligatures.keys()
if missing:
    raise SystemExit('Unsupported icons (use the full source font if adding icons): ' + ', '.join(sorted(missing)))
glyphs = {ligatures[name] for name in used}
for table in subtables:
    table.ligatures = {first: [item for item in items if item.LigGlyph in glyphs]
                      for first, items in table.ligatures.items()}
    table.ligatures = {first: items for first, items in table.ligatures.items() if items}
options = subset.Options()
options.flavor = 'woff2'
subsetter = subset.Subsetter(options=options)
subsetter.populate(glyphs=glyphs, text=''.join(sorted(used)))
subsetter.subset(font)
font.flavor = 'woff2'
font.save(args.output)
print(f'{len(used)} icons: {args.source.stat().st_size:,} -> {args.output.stat().st_size:,} bytes')