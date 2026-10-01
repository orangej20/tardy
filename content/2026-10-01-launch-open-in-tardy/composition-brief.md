# Hyperframes Composition Brief: tardy, "Open in Tardy"

## Objective
A 22s vertical movie-trailer launch reel for PR #16 (iOS share extension), per `/tardy-launch`.

## Output
- Composition: `composition/` · Video: `brag.mp4` · 1080x1920, 30fps, 22.0s

## Source material
- PR #16 body and commits (`7526b47`, merge `73bdafb`); claims and sources in `facts.md`.
- Real app take: `composition/assets/reveal.mp4`, recorded from the standalone simulator build
  (`xcrun simctl io booted recordVideo`), Safari → Share → Tardy → Your agents → Send → ✓ Sent.
- Wordmark: `composition/assets/wordmark.png`, cropped from an app screenshot (Home header).
- Copy that appears verbatim: "Open in Tardy" (`iosShareExtensionName`), "Real followers, real
  friends. Stay Tardy." (`docs/brand/BRAND.md`).

## Creative direction
cinematic; "blockbuster trailer for a pull request: dead serious about something small". Storyboard,
beats, safe zones and limits pushed: `brag-plan.md`.

## Visual identity
`mobile/src/theme/index.ts`: bg #0A0A0D, surface #15151B, text #F7F7FA, textSecondary #A1A1AE,
textTertiary #80808C, primary #FFC21A. Trailer caps in Avenir Next Condensed (macOS system font,
loaded with `local()`, not shipped).

## Audio
Original score synthesized by `composition/build-audio.sh` (sub hits, riser, braam, drone), plus
CC0 Kenney click/bell SFX from brag's library. Silence beat 7.4–7.75s before the reveal.

## Implementation notes
- `device-frame-stage` (registry) holds the take; local edits: video in the slot, viewport fills the
  screen, duration 7.25s. `grain-overlay` (registry) inlined with seekable stepped jitter instead
  of an infinite CSS animation.
- Letterbox hand-built (no catalog match; gap reported).
