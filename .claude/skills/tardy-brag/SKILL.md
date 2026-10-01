---
name: tardy-brag
description: Make a tardy reel (a vertical status-update video about real work) with /brag rendered through HyperFrames, then grade it. Use for any tardy content video, "brag about this PR/feature/deploy", or when a post-type skill (tardy-explainer, tardy-podcast, tardy-launch, tardy-ugc, tardy-brainrot) sends you here for the shared rules. Picks the post type when none is named.
---

# tardy-brag

The shared pipeline for every tardy reel. Post-type skills hold only what differs per format;
everything below applies to all of them. A reel is not done until `/tardy-brag-grade` says `ship`.

## 0. Pick the post type

If the user named a type, load that skill and come back here for the pipeline.
Otherwise pick from the event, in this order:

| Event | Type |
|---|---|
| A launch, a release, a project going public | `/tardy-launch` |
| A change whose *why* matters (architecture, a fix with a cause, a migration) | `/tardy-explainer` |
| A decision with a real trade-off, or anything two people could argue about | `/tardy-podcast` |
| A small, delightful win; a before/after a user would feel | `/tardy-ugc` |
| A busy sprint, many small updates, a recap | `/tardy-brainrot` |

State the pick and the reason in one line.

## 1. Facts first: `facts.md`

Source of truth is git and the tracker, never memory. Gather the event:

```bash
git log --oneline <range>
GH_TOKEN=$(gh auth token -u orangej20) gh pr view <n> -R ajmwagar/tardy --json title,body,files,mergedAt
mb show <id>    # when a marble is involved
```

Write `<run>/facts.md`: every claim the reel will make, one per line, each with its source
(`PR #10`, `2be7ec5`, `mobile/src/stories/playback.ts:42`). The grader's truth gate checks the
finished reel against this file. A claim with no source does not go in the reel.

- tardy is pre-alpha. No users, metrics, testimonials, logos of customers, or roadmap items
  presented as shipped. The README roadmap says what is real.
- Never print secrets, private repo names, internal URLs, or anyone's email.

## 2. Run directory

`<run>` = `content/<YYYY-MM-DD>-<type>-<slug>/` at the repo root, e.g.
`content/2026-10-01-podcast-audio-row/`. It holds `facts.md`, brag's `brag-plan.md`,
`composition/` (the HyperFrames project), `brag.mp4`, `brag.jpg`, `share-copy.txt`, and later
`grade/` and `scorecard.json`.

## 3. Run /brag, full HyperFrames path

Invoke `/brag` with the output directory set to `<run>` and these options:

```
/brag --full --format vertical --tone <type's tone> [--voice] <type's creative direction>
```

- **`--full` is mandatory.** On Opus 5.5 `/brag` silently switches to `/brag-slim`, which skips
  HyperFrames. A run without `<run>/composition/` fails the grade.
- **`--voice`** when the type skill says so (Kokoro via HyperFrames).
- Pass the type skill's beat structure, duration window, and "push it" moves as the creative
  direction. They override brag's 15–25s window and its default hook→reveal→highlights→outro
  shape only where the type skill says so.

## 4. tardy laws (every type)

**Brand comes from the app.** Colors and type come from `mobile/src/theme/index.ts`; read it, never
copy hexes from memory or old runs. Near-black `bg`, Tardy yellow `primary` for the hero action,
alarm red only for urgency (blocked, breaking, the logo dot). Status colors only for work status.
Wordmark: lowercase `tardy` in SF Rounded Black with the red dot. Voice and sign-off: `docs/brand/BRAND.md`.

**Show the real app.** At least one beat shows the actual tardy app, captured, not redrawn:

```bash
xcrun simctl io booted screenshot <run>/composition/assets/screen-<n>.png
xcrun simctl io booted recordVideo --codec h264 <run>/composition/assets/clip-<n>.mov   # Ctrl-C to stop
```

Stage it with `device-frame-stage` (`npx hyperframes add device-frame-stage`). For a code change,
real diff lines from the PR count as "the thing" too.

**Native to the feed.** 1080×1920, 30fps. The app lays its own chrome over every reel: the top bar,
the caption block and tab bar at the bottom, and the action rail on the right (`styles.rail`,
`styles.info` in `mobile/src/app/(tabs)/reels.tsx`; the info block stops 70pt from the right edge,
about 190px at 1080 wide). Read that file for the current layout, then keep key text out of those
zones; as of this writing roughly the top 220px, the bottom 480px, and the right 190px. Anything a
viewer must read must survive autoplay on mute: on-screen text or captions carry the story, sound
makes it better.

**Hook in 1 second, not 2.** Frame 0.5s must already say something specific to this event.

**Originals only.** No real person's voice, face, or name as a character ("Morgan Freeman
narrates" means a generic trailer-voice archetype, never an imitation). No copyrighted game
footage, characters, or music. Build stand-ins in the composition; it's HTML, so you can.

## 5. Push the limits

brag's defaults make a good launch video. tardy reels compete with TikTok, so every reel takes at
least **two** of these, named in `brag-plan.md` under `## Limits pushed`:

- **Search before building.** `npx hyperframes catalog --query "<the beat in plain words>"`, then
  `npx hyperframes add <name>`. Known-good: `caption-pill-karaoke`, `beat-pulse-background`,
  `device-frame-stage`, `comparison-split`, `grain-overlay`, `glitch`, `cinematic-zoom`,
  `wireframe-portal-title`. If nothing fits, report the gap
  (`npx hyperframes feedback --search-miss ...`) and hand-build it.
- **Real motion from the app,** not screenshots: a simulator recording cut on the beat.
- **Multiple voices** (Kokoro has several): hosts, a narrator plus a "user", a cold-open voice.
- **Procedural visuals** built in the composition: generated gameplay, data-driven charts from
  `git log --numstat`, the actual diff typing itself out.
- **Beat sync:** `npx hyperframes beats <composition> --json`, cut and pulse on the grid.
- **A loop:** the last frame matches the first so the reel replays seamlessly in the feed.

## 6. Grade, then deliver

1. Run `/tardy-brag-grade <run>` in a fresh context (a subagent or a new session), so the builder
   never grades its own work.
2. `revise`: apply its top three fixes, re-render, re-grade. Two revise rounds max, then bring the
   scorecard to the user.
3. `ship`: commit the run directory (not `composition/node_modules`, renders, or `grade/stills`;
   `.gitignore` covers them) with the marble id in the message. Publishing anywhere is the user's call.

Tell the user: the video path, the verdict and total, and the share copy.
