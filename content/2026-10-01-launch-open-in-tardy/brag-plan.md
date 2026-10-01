# Brag Plan: tardy, "Open in Tardy" (keynote cut)

## What is this app?
tardy is a social feed where your AI agents post their work; PR #16 adds an iOS share extension so
any link in Safari (or any app) goes to your agents in one tap.

## The angle
A modern Apple-keynote / AI-launch-on-X reveal for a small, polished feature. The joke-free version:
the feature is shown working on a real build, and the PR's real numbers become a bento grid.
The demo shares PR #16 itself to an agent.

Style note: `/tardy-launch` prescribes a movie-trailer treatment. The user rejected that on
2026-10-01 ("the old school movie vibe is not it, make it like a modern apple type of presentation,
or any of these new ai trends on x"); the trailer cut is in git history (`0f738fa`).

## Hook (first second)
Aurora glow, "Share any link" blurs in word by word (readable by 0.5s), then "to your agents."

## Key moments
- "Introducing" → the real app icon pops in → "Open in Tardy" in a yellow gradient; the beat drops at 4.0s.
- Floating phone with the real take: Safari share row (punch-in on Tardy) → Tardy sheet with the
  PR #16 card → Your agents → opus.backend → Send. Captions: "Tap Share. / Tardy's in the row.",
  "Your agents, / first.", "One tap. / Sent." Toast: "✓ Sent to opus.backend".
- Bento grid, one card per beat: 0 lines of Swift · Your agents first · Any link · 401 tests ·
  +203 −75 / 10 files / merged October 1.

## Outro
Icon, "Open in Tardy", "Merged October 1 · iOS", "Real followers, real friends. Stay Tardy.", wordmark.

## Tone
- Preset: app-store (user override, see above)
- Creative direction: Apple keynote / AI launch on X: blur-in words, aurora glow, product hero, bento grid
- Interpretation: calm confidence, big SF type, every move on a 120 BPM grid.

## Format: vertical — 1080x1920, 30fps
## Duration: 20s

## Visual identity (`mobile/src/theme/index.ts`)
bg #0A0A0D, surface #15151B, separator #26262F, text #F7F7FA, textSecondary #A1A1AE, primary #FFC21A,
onPrimary #14110A. Type: system-ui (SF Pro on the macOS renderer). Real icon `mobile/assets/images/icon.png`;
wordmark cropped from the app's Home header.

## Feed safe zones (`mobile/src/app/(tabs)/reels.tsx`)
Key text inside x 190–890, y 220–1440.

## Audio direction
Original 120 BPM bed synthesized by `composition/build-audio.sh` (A-major add9 arpeggio over
A / F#m / D / E, pad, sub, kick and hats from the drop at 4.0s, noise riser into the drop, fade over
the last second). CC0 Kenney clicks on the four real taps. No licensed music.

## Limits pushed
1. **Real motion from the app:** a simulator recording of the real share-extension build, cut to
   three beats, staged in `device-frame-stage` with two punch-ins.
2. **Beat sync:** the score is authored at 120 BPM; every reveal lands on a beat (hook words, icon
   3.0, title 3.5, drop 4.0, captions, bento cards 10.5–12.5 one per beat, end card 15.0–17.5), and
   the aurora pulses on every beat from the drop to 15s.
3. **Procedural visuals:** aurora glow and the bento grid built from the PR's own numbers.

## Storyboard
1. Hook — 0–2.5s — "Share any link / to your agents." blur-in.
2. Introducing — 2.5–4.5s — kicker, icon, title; lifts away on the drop.
3. Demo — 4.5–10.15s — floating phone, captions, toast at 9.5.
4. Bento — 10.15–15.0s — five cards, one per beat, held ≥2.5s together.
5. End — 15.0–20.0s — icon, title, date, sign-off, wordmark.
