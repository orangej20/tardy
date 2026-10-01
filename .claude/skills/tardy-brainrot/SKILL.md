---
name: tardy-brainrot
description: Make a tardy Brainrot reel, a split-screen recap (gameplay on the bottom, two cartoon hosts summarizing your sprint on top) for a busy stretch of work, from a commit range, a week of PRs, or a set of marbles. Use for sprint recaps, "what happened this week", and many small updates at once.
---

# tardy Brainrot

Read `/tardy-brag` first; it owns facts, the run directory, brand, safety, and grading. This file
is only what differs for brainrot.

## brag options

```
/brag --full --format vertical --tone chaotic --voice "split-screen brainrot recap: two cartoon hosts rapid-fire summarizing a sprint over gameplay"
```

Duration **20–35s** (`check.sh <run> 20 35`). Past brag's cap because it's a list; each item gets
2–4s. 6–10 items from `facts.md`, nothing padded.

## Layout

- **Top half:** two **original** cartoon hosts (flat, tardy palette, made for this reel or reused
  from earlier tardy reels), karaoke captions between them. Never existing cartoon characters or
  their voices.
- **Bottom half:** gameplay, **generated in the composition**: an endless runner, a parkour block
  course, or a driving loop built in HTML/canvas/three.js. No captured footage from any real game.
  The world is themed on the repo: jump over commit hashes, dodge red CI blocks, collect PR coins.
- Divider in Tardy yellow.

## Push it

- **The gameplay is the data.** Each obstacle or pickup is a real item from `facts.md`: a merged PR
  is a coin, a revert is a crash, a blocked marble is a wall. The run *is* the sprint.
- **Counter HUD** in the gameplay corner: PRs merged, tests added, computed from git and listed in
  `facts.md`.
- Item transitions on the beat grid (`npx hyperframes beats`), hosts alternate per item.
- **Seamless loop:** the runner's last frame matches its first.

## Grade on (type criteria for the rubric)

- Every item is a real, sourced event; a viewer could reconstruct the sprint from the captions.
- Gameplay and hosts are original; nothing resembles a specific game or show.
- Readable at speed: each item's caption holds at least 0.3s per word.
