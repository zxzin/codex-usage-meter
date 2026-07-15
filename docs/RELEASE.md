# Release Guide

Token Meter ships through GitHub Releases with separate macOS and Windows assets.

## Release Assets

- macOS: `.dmg` is the primary download.
- Windows: `.exe` NSIS installer is the primary download; `.msi` is the alternate installer.

The current workflow does not sign or notarize builds. Users may see macOS Gatekeeper or Windows SmartScreen warnings.

## Create A Release

1. Update the version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json`.
2. Commit the version change.
3. Push a version tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The release workflow builds both platforms and uploads the assets to the GitHub Release.

## Manual Release

Open GitHub Actions, run `Release`, and provide a tag such as `v0.1.0`.

## Local Build Checks

Run these on the matching operating system:

```sh
npm run tauri:build:mac
npm run tauri:build:windows
```

Use `npm run build` for a fast frontend-only check on any platform.

## Mac App Store

The Mac App Store build uses the same React meter, quota logic, bee assets, and motion model as the GitHub release. Its macOS wrapper adds App Sandbox folder access and a public-API transparent renderer; the GitHub-only private WebKit feature must remain scoped to the `direct-download` Cargo feature.

Parity is enforced in code rather than by manually matching two independent animations:

- `src/bee-motion-contract.json` is the shared source of truth for speed mapping, acceleration/deceleration, orbit radius, facing angle, wing speed, trail opacity, and idle placements.
- The React/SVG layer renders the hive and both live quota rows in every build.
- The App Store renderer snapshots only that low-frequency static/data layer. It composites the original `bee-body-wingless-ui.png` and `bee-wing-ui.png` assets in native layers so flight, wing motion, and the two softened trail layers remain continuous when WebKit refreshes quota data.
- A 30-second 60 fps capture must have no multi-frame freeze. Compare its adjacent duplicate count and longest duplicate run with the current GitHub build before accepting a renderer change.

Build and launch the local sandbox candidate:

```sh
npm run tauri:build:appstore
open "/tmp/token-meter-appstore-target/release/bundle/macos/Token Meter.app"
```

On first launch, select the `.codex` folder that contains `auth.json`. The app stores a read-only security-scoped bookmark and restores it on later launches.

For a signed submission package, provide the local signing configuration without committing credentials:

```sh
APPLE_TEAM_ID="..." \
APPLE_SIGNING_IDENTITY="Apple Distribution: ..." \
APPLE_INSTALLER_IDENTITY="3rd Party Mac Developer Installer: ..." \
TOKEN_METER_PROVISIONING_PROFILE="/absolute/path/to/profile.provisionprofile" \
npm run tauri:build:appstore
```

The signed package is written to `.release/appstore/Token Meter_<version>.pkg`. Verify the exact artifact before upload:

```sh
codesign --verify --deep --strict "/tmp/token-meter-appstore-target/release/bundle/macos/Token Meter.app"
pkgutil --check-signature ".release/appstore/Token Meter_<version>.pkg"
```

Do not upload an ad-hoc candidate. Create or select the matching App Store Connect app record and provisioning profile, upload the signed package, then test the processed build before submitting it for App Review.
