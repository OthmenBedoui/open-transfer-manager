# Browser extensions

The extension sends downloads and video pages to the app. It talks to the app through
**native messaging**: the browser starts a small program (`otm-native-host`) that forwards
requests to the running app over `127.0.0.1` with a secret token. No open port is reachable
from web pages or from other computers.

```
 extension ──native messaging (stdin/stdout)──▶ otm-native-host ──HTTP + token (127.0.0.1)──▶ app
                                                  starts the app if needed
```

## Supported browsers

| Browser | Build | Bridge location |
|---|---|---|
| Chrome, Chromium, Brave, Edge, Vivaldi, Opera, Arc | `chromium` | `NativeMessagingHosts/` in the browser's config folder |
| Firefox, LibreWolf, Waterfox (incl. Snap & Flatpak paths) | `firefox` | `~/.mozilla/native-messaging-hosts/` (Linux), `~/Library/Application Support/Mozilla/NativeMessagingHosts/` (macOS) |
| Safari | planned | needs a containing macOS app (see below) |

`otm register-browsers` writes the bridge file for every browser it finds. Run it again after installing a new browser.
`otm doctor` shows which browsers are connected.

## What the extension does

* **Capture downloads** (switch in the popup): file types from a list (or all), optional minimum size, excluded sites.
  If the app cannot take a download, the browser downloads it itself – nothing is lost.
* **Right-click menus**: *Download with Open Transfer Manager* (links, images, videos), *Download video / audio from this page*.
* **Popup**: on/off switch, “Download video from this page” with quality, live list with pause/resume/open.
* Cookies and referrer are passed along so downloads behind a login keep working.

## Build

```bash
python scripts/build_extension.py
# dist/extension/chromium/  and  dist/extension/firefox/  (+ zip files for the stores)
```

### Extension IDs

The Chromium build has a fixed ID (`iaaaiogfbkpiddhonhfnekdffilohjjc`) thanks to the public `key` in
`extension/manifest.chromium.json`. Store versions get their own IDs: add them to
`CHROMIUM_EXTENSION_IDS` in `src/otm/browsers.py` when the extension is published.
The Firefox ID is `otm@opentransfermanager.org`.

## Publishing checklist

* **Chrome Web Store** – upload the chromium zip (remove `key` for the store build if asked), justify permissions:
  `downloads` (capture), `nativeMessaging` (talk to the app), `cookies` + host access (protected downloads),
  `contextMenus`, `storage`, `activeTab`.
* **Microsoft Edge Add-ons**, **Opera add-ons** – same zip.
* **Firefox Add-ons (AMO)** – upload the firefox zip; source code link: this repository.

## Safari (planned)

Safari Web Extensions must ship inside a macOS app and use the app as the native bridge:

1. `xcrun safari-web-extension-converter dist/extension/chromium --app-name "Open Transfer Manager" --bundle-identifier org.opentransfermanager.safari`
2. In the generated Xcode project, make `SafariWebExtensionHandler.swift` forward messages to the local API
   (read `runtime.json` from the app data folder, send the request with the `X-OTM-Token` header).
3. Sign with an Apple Developer ID and notarize.

Contributions welcome – see the roadmap.

## Troubleshooting

| Message | Fix |
|---|---|
| *Open Transfer Manager is not installed, or the bridge is not registered* | `otm register-browsers`, then restart the browser |
| *The bridge does not allow this extension ID* | Update the app; a new extension ID must be listed in `browsers.py` |
| *App not running* | It starts automatically when you send a download; or open it from the menu |
| Snap Firefox cannot reach the bridge | Recent Snap Firefox supports native messaging through the desktop portal; update Firefox |
