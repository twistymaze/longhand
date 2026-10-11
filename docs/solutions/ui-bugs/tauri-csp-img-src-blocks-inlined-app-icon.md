---
title: "The app icon did not render in the signed bundle because the CSP had no img-src for data: URIs"
date: 2026-09-25
category: ui-bugs
module: "App icon display (src/settings/icons.tsx AppIcon) and the CSP in src-tauri/tauri.conf.json"
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "The Home header icon did not render in the signed bundle; an empty square showed in its place"
  - "The icon rendered in the vite dev server and in Playwright screenshots but not in the built app"
root_cause: config_error
resolution_type: config_change
framework_version: "tauri 2.11"
related_components:
  - tooling
tags:
  - tauri
  - csp
  - img-src
  - data-uri
  - vite
  - app-icon
  - wkwebview
---

# The app icon did not render in the signed bundle because the CSP had no img-src for data: URIs

## Problem

The Home header app icon did not show in the signed Tauri bundle. Vite put the small PNG into the JavaScript bundle as a `data:` URI, and the app CSP did not allow `data:` images.

## Symptoms

- In the signed bundle, an empty square showed where the app icon must be. The user reported: "The icon doesn't render".
- In the browser previews (the Vite dev server with Playwright), the icon showed correctly.
- The source was correct. `src/settings/icons.tsx:5` imports the PNG (`import appIcon from "../assets/longhand-icon.png"`), and `src/settings/icons.tsx:28` renders it as `<img src={appIcon} ...>` in `AppIcon`.
- The file `src/assets/longhand-icon.png` is 128 x 128 pixels and 3914 bytes.

## What Didn't Work

- **Browser previews.** The development screenshots used the Vite dev server and Playwright with a mocked Tauri IPC. They showed the icon. Two differences hide the defect there:
  - The Vite dev server serves each imported asset as a file URL. It does not inline assets. Inlining occurs only in `vite build`.
  - The dev server sends no CSP. The CSP in `src-tauri/tauri.conf.json` applies only when Tauri serves the page.
- **`tauri dev`.** `src-tauri/tauri.conf.json:16` sets `"devCsp": null`. Thus `tauri dev` applies no CSP, and it also loads the page from the dev server (`devUrl`, `src-tauri/tauri.conf.json:8`). Both causes of the defect are absent in dev mode.
- Only the signed bundle (`bun tauri build --bundles app`) uses the production build and the production CSP together. That is the only place the defect showed.

## Solution

Add `img-src 'self' data:` to the app CSP in `src-tauri/tauri.conf.json:15`. The fix was merged in archived PR 4 (twistymaze/longhand).

Before:

```json
"csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; connect-src ipc: http://ipc.localhost",
"devCsp": null
```

After (`src-tauri/tauri.conf.json:15-16`):

```json
"csp": "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src ipc: http://ipc.localhost",
"devCsp": null
```

The next signed bundle showed the icon. Per this session's conclusion, the user's later review raised no icon issue.

## Why This Works

- **Vite inlines small assets.** Vite 6.4.3 (the version in `node_modules/vite/package.json:3`) sets `DEFAULT_ASSETS_INLINE_LIMIT = 4096` at `node_modules/vite/dist/node/constants.js:143`. The default config uses it at `node_modules/vite/dist/node/chunks/dep-Dm0c1Wj2.js:46036`. The inline check at `dep-Dm0c1Wj2.js:10611-10619` returns `content.length < limit`. The icon is 3914 bytes, which is less than 4096, so Vite encodes it as a `data:image/png;base64,...` string.
- `vite.config.ts` has no `build` section (`vite.config.ts:9-26`), so the default limit applies.
- A `bun run build` at the current tree confirms it. `dist/assets/index-*.js` contains `data:image/png;base64,iVBORw0KGgo...`, and it is the only file under `dist/assets/` that contains a `data:` URI.
- **The CSP falls back to `default-src`.** The old CSP had no `img-src` directive. For images, the browser then uses `default-src`, which was `'self'`. The `'self'` source does not match the `data:` scheme. WKWebView blocked the image and showed an empty box.
- The new `img-src 'self' data:` directive applies to images in place of `default-src`. It allows both files from the app origin and `data:` URIs.

## Prevention

- **Each fetch type that Vite can inline needs its own directive.** `font-src`, `media-src` and `img-src` all fall back to `default-src 'self'`, which blocks `data:`. When you import a new asset type from source, add the matching directive to `src-tauri/tauri.conf.json:15`:
  - Fonts (`.woff`, `.woff2`, `.ttf`) under 4096 bytes: add `font-src 'self' data:`.
  - Audio or video under 4096 bytes: add `media-src 'self' data:`.
  - A CSS `url()` that points to a small image is also inlined. `img-src` covers it.
- **Alternative: stop inlining.** Set `build: { assetsInlineLimit: 0 }` in `vite.config.ts`. The type definition at `node_modules/vite/dist/node/index.d.ts:2545-2553` says "Set to `0` to disable." Every asset is then a file under `'self'`, and the CSP needs no `data:` source. This session did not take this path, because the one-line CSP change was sufficient. Either path works. Do not use both without a reason.
- **Quick check after a build.** Run `bun run build`, then run:

  ```sh
  grep -lo 'data:[a-z]*/' dist/assets/*
  grep -o 'data:[a-z]*/[a-z0-9+.-]*' dist/assets/* | sort -u
  ```

  Each MIME type in the output (`image/...`, `font/...`, `audio/...`, `video/...`) needs a CSP directive that allows `data:`.
- **Test asset rendering in the signed bundle.** Build with `bun tauri build --bundles app` and open the app. Dev mode and browser previews do not apply the production CSP (`devCsp` is `null`) and do not inline assets. They cannot show this class of defect.
- **Optional: a dev CSP.** You can set `devCsp` to the same value as `csp`. That shows CSP blocks in `tauri dev`. It still does not show inlining defects, because the dev server does not inline.

## Related Issues

- `docs/solutions/ui-bugs/tao-traffic-light-y-not-top-inset.md`: another defect that showed only in the signed bundle, from the same review.
