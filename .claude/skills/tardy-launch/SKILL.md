---
name: tardy-launch
description: Make a tardy Product Launch reel, a movie-trailer-scale announcement ("This fall... one dev... one refactor...") for a release, a feature landing, or a project going public. Use for launches, big merges, and the tardy launch reel itself.
---

# tardy Product Launch

Read `/tardy-brag` first; it owns facts, the run directory, brand, safety, and grading. This file
is only what differs for launches.

## brag options

```
/brag --full --format vertical --tone cinematic [--voice] "blockbuster trailer for a pull request: dead serious about something small"
```

Duration **15–25s** (brag's window; `check.sh <run> 15 25`). `--voice` is optional; trailer cards
alone work, and the narrator, if used, is a generic deep trailer voice, never an imitation of a
real actor.

## Beats

1. **Cold open (0–2s).** Black, a low hit, one card: the stakes in five words or fewer.
2. **The build (3 cards).** "THIS FALL…" / "ONE DEV…" / "ONE <REAL THING FROM facts.md>…", each on a
   hit, letterboxed.
3. **The reveal.** The feature running in the real app, in a device frame, full brightness, the
   bed opens up.
4. **Title card.** The feature name as a movie title; the release date is the merge date.
5. **Tag.** "Real followers, real friends. Stay Tardy." and the wordmark.

## Push it

- **Letterbox that opens:** 2.39:1 bars during the build, bars slide away on the reveal.
- **Trailer sound design:** hits, a riser, a braam, and a silence beat right before the reveal
  (silence under 0.5s, or `check.sh` flags it).
- `cinematic-zoom` or `wireframe-portal-title` into the title card; `grain-overlay` on the build only.
- **Fake credits block** made of real data: the agents and humans on the PR, the line counts.

## Grade on (type criteria for the rubric)

- The contrast between trailer gravity and the size of the change is the joke, and it lands.
- The reveal shows the real feature, not a title card about it.
- The title card is readable for at least 1.2s.
