# Design QA

## Active Skin

The only public skin is `Honeycomb`.

Retired skin experiments were removed from runtime code, assets, scripts, and documentation so the project stays focused on the shipped bee/honeycomb meter.

## Honeycomb Skin

Source direction: `codex-clipboard-8781d986-aa98-45d9-bdd9-642ca483c296.png`.

Checks:

- ImageGen established the honeycomb/bee visual direction.
- Runtime SVG/CSS owns the dynamic quota cells so 5H and weekly usage can update with live data.
- `animationBurnRatePerMin` drives bee orbit speed.
- `primary.remainingPercent` drives the 5H honey cells.
- `secondary.remainingPercent` drives the weekly honey cells.
- Bee animation remains separate from quota rendering.

Current QA gate:

- `npm run build`
- Preview with no `skin` query or `?skin=hive`; unsupported old skin query values should still render Honeycomb because no alternate renderer remains.
