# Release Guide

Token Meter ships through GitHub Releases with separate macOS and Windows assets.

## Release Assets

- macOS: `.dmg` is the primary download.
- Windows: `.exe` NSIS installer is the primary download; `.msi` is the alternate installer.

The current workflow does not sign or notarize builds. Users may see macOS Gatekeeper or Windows SmartScreen warnings.

## Create A Release

1. Update the version in `package.json`, `package-lock.json`, and `src-tauri/tauri.conf.json`.
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
