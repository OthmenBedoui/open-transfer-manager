# Contributing

Thanks for helping! Small, focused pull requests are easiest to review.

## Set up

```bash
git clone https://github.com/OthmenBedoui/open-transfer-manager
cd open-transfer-manager
python3 -m venv .venv && . .venv/bin/activate
pip install -e ".[dev]"
otm                      # run the app from your checkout
```

## Checks

```bash
ruff check src tests scripts
python -m unittest discover -s tests -v
python scripts/build_extension.py
```

The UI test opens real windows; on a headless Linux machine use `xvfb-run python -m unittest discover -s tests`.

## Guidelines

* Keep it light: the app depends only on the Python standard library, Tk and yt-dlp. Discuss new dependencies in an issue first.
* Python 3.9 compatible code.
* UI colours come from `ui/theme.py` – never hard-code colours in widgets.
* Anything that changes the download list from another thread must go through `Manager.call()`.
* Write user-facing text in plain English.

## Reporting bugs

Please include the output of `otm doctor` and the steps to reproduce.
