# CVEC app icon

`master.png` is the source artwork for CVEC's app icons and favicons (white Chihuahua motif on `#3030D8` ultramarine).

## Exporting icons

From the repository root:

```sh
npx tauri icon assets/branding/chihuahua-ultramarine/master.png --output /tmp/cvec-icon-export
```

Copy `icon.icns`, `icon.ico`, `icon.png`, `32x32.png`, `128x128.png`, and `128x128@2x.png` into `src-tauri/icons/`. CVEC draws its tray status dots directly in code.

Web assets in `public/` use the same exports (`128x128.png` → `app-icon.png`, `32x32.png` → `favicon-32.png`, `64x64.png` → `favicon-64.png`). Regenerate the Apple touch icon with:

```sh
sips -z 180 180 assets/branding/chihuahua-ultramarine/master.png --out public/apple-touch-icon.png
```

Rebuild the app with `npm run tauri -- build --debug --bundles app` after replacing icons.
