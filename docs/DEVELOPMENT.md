# Development Guide

This project is a tiny desktop token-usage pet. The product should stay small, visual, and playful. It is not a full analytics dashboard.

## Product Scope

The main window has only one user action:

- Right-click -> `Reload`

Everything else should be automatic:

- Usage data refreshes every second.
- The window stays tiny and transparent.
- The pet can be dragged from the widget surface.
- Provider mode defaults to `auto`.

Do not add visible toolbars, landing screens, onboarding panels, large settings pages, or skin selectors unless the product scope changes.

## Architecture

The app has three layers:

- `src-tauri/src/lib.rs`: provider collection, quota parsing, token-rate calculation, native window positioning, and the native reload menu.
- `src/App.tsx`: Honeycomb rendering, polling, drag handling, and reload wiring.
- `scripts/`: install and launch helpers for GitHub/Agent-driven setup.
- `.github/workflows/release.yml`: macOS and Windows release builds for GitHub Releases.

Provider behavior:

- Codex speed comes from local `~/.codex/sessions` `token_count` events.
- Codex quota comes from `https://chatgpt.com/backend-api/wham/usage`.
- Claude Code quota and usage come from the local `statusLine` bridge.
- `USAGE_METER_PROVIDER=codex|claude` can force a provider.
- `USAGE_METER_PROVIDER=auto` picks the provider with the freshest local activity.
- `CODEX_HOME` can point the app to a non-default Codex data directory, which is important when Windows Codex data lives outside the native Windows home directory.

Do not invent quota percentages. If a provider does not expose a real quota window, show no quota instead of fabricating one.

## Active Skin

Only `Honeycomb` is public and only its runtime assets should be kept in the project.

The Honeycomb skin binds to this data contract:

- `snapshot.animationBurnRatePerMin`: drives bee motion intensity.
- `snapshot.primary.remainingPercent`: 5H quota.
- `snapshot.secondary.remainingPercent`: weekly quota.
- `snapshot.state`: semantic state for heat/limit styling.

The skin must not own provider-specific logic. Provider differences belong in the backend snapshot.

## ImageGen Workflow

Use ImageGen to decide and preserve visual direction before implementing a future skin. For character skins, the selected ImageGen art should remain the runtime visual source unless a later vector pass can prove equal likeness at `88x88` and `76x76`.

1. Generate 4-6 rough concept options in one style family.
2. Label options clearly, for example `A`, `B`, `C`, `D`.
3. Ask the product owner to choose one direction before coding.
4. After selection, generate a cleaner final reference for the chosen direction.
5. Ask for rig-friendly output: large readable shapes, no baked quota values, no labels, and separable moving parts such as body, head, ears, paws, legs, wings, wheel, or prop.
6. Cut or export the final art into a small number of transparent PNG layers.
7. Separate ImageGen-owned character/prop art from dynamic UI areas.
8. Implement dynamic areas in frontend SVG/CSS, not as frozen pixels.

Important lesson from the Honeycomb skin:

- ImageGen can produce a good visual target, but quota cells must be rebuilt or overlaid so they can change with real data.
- Do not paste a full static image if it contains dynamic quota states.
- If a quota cell can turn blue/green/yellow/red, the cell shape and mask must be recreated in code or exported as separate empty/fill layers.
- Animation should be simple and legible at 76-88px.

## Living Skin Production Playbook

Use this workflow only when the product scope explicitly adds another animated skin. The goal is to get ImageGen-level visual taste without asking ImageGen to solve animation consistency.

Core principle:

- ImageGen owns character identity and static art direction: silhouette, face, species readability, color, mood, material, and final visual target.
- SVG/CSS owns structured data: 5H quota, weekly quota, speed rings, masks, progress cells, needles, and other values that must update.
- Rigged ImageGen layers own character motion: body, head, ears, wings, paws, legs, props, wheels, or decks should be separate transparent layers when possible.

Avoid these failure modes:

- Do not ask ImageGen to create many unrelated animation frames for a living character. Individual frames may look good, but the character will drift between frames.
- Approved pixel-sprite exception: when the style itself is pixel art, generate or sketch a full 4-frame contact sheet first, select the direction, then normalize the chosen character frames into one fixed-grid sprite before writing runtime code.
- Do not hand-draw the whole character in SVG unless the style is intentionally simple. Motion will be stable, but the character will often look worse than the ImageGen concept.
- For approved simple line/toy styles, an SVG part rig is acceptable when the body silhouette, face, ears, and color blocks are traced from the selected ImageGen direction first, and only legs/paws/ears/tail use small runtime transforms or frame states.
- Do not auto-vectorize or trace the full character unless visual QA proves the result still matches the selected reference. Tool output is not accepted by itself.
- Do not keep iterating on a technically clean animation when the subject no longer reads correctly. Go back to the ImageGen layers first.

Preferred pipeline:

1. Generate 4-6 ImageGen concepts for the skin.
2. Select one direction before implementation.
3. Mark every visual element as one of:
   - static decoration
   - dynamic quota UI
   - speed-driven motion
   - rigged character part
4. Ask ImageGen for a cleaner final art pass with simple, separable shapes and no baked dynamic values.
5. Export or cut the chosen art into a small number of transparent runtime layers.
6. Place those ImageGen layers in the frontend as the main visual source.
7. Rebuild quota and speed indicators in SVG/CSS so they can bind to real snapshot data.
8. Animate ImageGen layers with bucketed CSS transforms instead of swapping many full frames.
9. Add small SVG/CSS motion overlays only when they do not fight the reference art.
10. Verify against the ImageGen reference and at `88x88` and `76x76`, both idle and active.

Tool guidance:

- ImageGen is the default tool for character shape and visual taste.
- Local image processing is for cleanup, cropping, alpha extraction, and layer preparation.
- SVG/CSS is the default tool for dynamic quota UI, speed indicators, masks, strokes, labels, and simple mechanical motion.
- Figma/vector tools can help plan or refine a rig, but a Figma/SVG rebuild must be checked against the ImageGen reference before replacing the runtime PNG layers.
- Pixel sprites are a deliberate style choice, not the default fallback. Use them only if the sprite look is approved, every frame is reviewed, anchors are normalized, and the result remains readable at micro size.

Animation rules:

- Speed may control the main semantic motion: orbit speed, wheel rotation, headbang rate, disc rotation, growth rate, or glow intensity.
- Small repeated parts such as wings or tiny legs should use stable animation loops. Do not update their CSS `animation-duration` every polling tick unless the value is bucketed or debounced; WebView may restart the animation and make it look stuck.
- Keep subpart loops continuous when possible. Use `steps()` only when the desired look is explicitly frame-by-frame pixel animation.
- Idle must be visually calm and intentional: stop, perch, sleep, or low bob. Avoid random wobble.
- Active motion must visibly scale with `snapshot.animationBurnRatePerMin`.

Data binding rules:

- `snapshot.animationBurnRatePerMin` drives motion intensity.
- `snapshot.primary.remainingPercent` drives the 5H indicator.
- `snapshot.secondary.remainingPercent` drives the weekly indicator.
- `snapshot.state` may change color emphasis, but must not invent fake provider data.
- Dynamic values should never be baked into ImageGen pixels.

Acceptance gates for a new living skin:

- One screenshot or contact sheet proves the ImageGen direction was chosen before code.
- Runtime character art visibly matches the selected ImageGen reference; side-by-side mismatch is a blocking visual issue.
- Static art and dynamic UI are separated.
- 5H and weekly quota can show full, partial, and low states.
- Motion changes with token speed but does not jitter or restart every polling tick.
- Idle state looks intentional.
- The skin remains readable at `88x88` and `76x76`.
- The implementation does not add provider-specific logic inside the skin.

## Visual Rules

The active meter should feel like a compact desktop toy, not a SaaS widget.

Use:

- ImageGen-led toy-like character shapes.
- Runtime alpha layers where character or prop likeness matters.
- Strong silhouettes.
- Clear 5H and weekly quota encoding.
- Motion that visibly changes with token speed.
- Transparent outer background.
- Very small, readable labels only when needed.

Avoid:

- Large cards or panels.
- Full dashboards.
- Text-heavy explanations.
- Decorative effects that hide quota state.
- Static quota artwork that cannot update.
- Hand-drawn character replacements that no longer match the selected reference.
- Motion that looks like random shaking instead of intentional behavior.

## Right-Click Menu Rules

The native right-click menu is the only settings surface.

Menu contents:

- `Reload`: fetch a fresh snapshot immediately.

Do not add size controls, provider controls, debug controls, docs links, or skin selectors to the primary menu. Those can live in README or environment variables if needed.

Reason: the app window is too small for an in-window menu, and a native menu avoids clipping.

## QA Checklist

Run these before considering a skin or data change done:

- `npm run build`
- `cargo test` in `src-tauri`
- `npm run tauri:build:mac` on macOS
- `npm run tauri:build:windows` on Windows
- Confirm the app bundle has `Contents/Resources/icon.icns`.
- Confirm the Windows release includes `.exe` or `.msi` installer assets.
- Confirm right-click opens the native menu.
- Confirm `Reload` refreshes data without resizing or moving the pet.
- Confirm the default window stays transparent outside the visual pet.
- Confirm the widget still works at `88x88` and `76x76`.
- Confirm idle motion stops or becomes visibly calm.
- Confirm high speed makes motion visibly faster.
- Confirm 5H and weekly quota bars/cells are clear at small size.
- Confirm low quota changes color or warning emphasis.
- Confirm Codex provider still reads live session speed.
- Confirm Claude bridge mock input writes `~/.token-meter/claude-status.json`.

For visual QA, capture at least:

- idle state
- active/fast state
- full quota
- low 5H quota
- low weekly quota
- right-click menu

## Claude Bridge QA

Use a temporary state directory when testing the installer:

```sh
tmpdir=$(mktemp -d)
CLAUDE_CONFIG_DIR="$tmpdir/.claude" TOKEN_METER_HOME="$tmpdir/state" npm run install:claude
```

Use mock statusLine input to test state writing:

```sh
printf '%s' '{"session_id":"sample","context_window":{"current_usage":{"input_tokens":100,"output_tokens":25}},"rate_limits":{"five_hour":{"used_percentage":20},"seven_day":{"used_percentage":13}}}' \
  | TOKEN_METER_HOME="$(mktemp -d)" node scripts/claude-statusline.mjs
```

Expected visible output:

```text
Token Meter | 5H 80% | WK 87%
```

## GitHub Release Criteria

Do not publish as a GitHub-ready project until these are true:

- Honeycomb is the only exposed skin and the README does not mention retired skins.
- README includes install, Codex setup, Claude setup, provider forcing, and build commands.
- `docs/DEVELOPMENT.md` is current.
- `design-qa.md` records final visual QA for the public skin.
- The icon is wired into the macOS bundle.
- `npm run build`, `cargo test`, and debug bundle build pass.
- The app can be installed by an Agent using only README commands.

## Skin Backlog

No backlog skin is active. Before implementing a future skin, run the ImageGen workflow and choose the direction visually first.
