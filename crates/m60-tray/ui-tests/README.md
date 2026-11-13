# Page tests

Behaviour tests for the two web pages in `../ui`: the volume popup (`index.html`) and the custom EQ
editor (`eq.html`). They open each page in headless Chromium with a small stand-in for the Tauri API,
so they need neither an app build nor the speaker.

They live outside `ui/` on purpose: Tauri bundles everything in `ui/` into the app.

```sh
npm install        # once
npm run setup      # once: downloads the Chromium build Playwright uses
npm test
```

Set `SCREENSHOTS_DIR` to save screenshots of each state, e.g. `SCREENSHOTS_DIR=shots npm test`
(PowerShell: `$env:SCREENSHOTS_DIR="shots"; npm test`).
