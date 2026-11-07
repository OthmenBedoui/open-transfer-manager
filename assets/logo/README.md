# Logo

* `logo-source.webp` – original artwork (white background)
* `logo-1024.png` – transparent master, generated

All icons are generated from the source with:

```bash
pip install pillow
python scripts/make_icons.py
```

This writes the app icons (`src/otm/assets/otm*.png`), the extension icons (`extension/src/icons/`) and the
GitHub social preview (`docs/social-preview.png`).

Brand colours: pink `#ff0a8c` → violet `#7c3aed` → cyan `#00e5ff`. The UI accent is the violet
(`src/otm/ui/theme.py`, `extension/src/theme.css`).
