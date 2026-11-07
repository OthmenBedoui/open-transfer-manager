# Security

Please report vulnerabilities privately through GitHub: **Security → Report a vulnerability** on this repository.
Do not open a public issue. We aim to answer within 7 days.

Design notes:
* The control API listens on `127.0.0.1` only, requires a random token stored in the user's application data directory (mode 600 on Unix; inherited user-directory ACLs on Windows),
  checks the `Host` header and never sends CORS headers.
* The browser host manifests allow the extension IDs listed in `src/browsers.rs`.
* Downloaded content is not executed automatically. Opening a completed file is an explicit user action. No telemetry is sent.
* Browser cookies/headers may be persisted with download records for resumption. Do not share the application data directory or diagnostics containing private URLs.
* Local installers are unsigned until a code-signing certificate is configured for release.
