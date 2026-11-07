# Security

Please report vulnerabilities privately through GitHub: **Security → Report a vulnerability** on this repository.
Do not open a public issue. We aim to answer within 7 days.

Design notes:
* The control API listens on `127.0.0.1` only, requires a random token stored in a file readable only by you,
  checks the `Host` header and never sends CORS headers.
* Only the extension IDs listed in `src/otm/browsers.py` may start the native host.
* The app never runs downloaded files and sends no telemetry.
