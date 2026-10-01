---
name: tardy-explainer
description: Make a tardy Explainer reel, a crisp AI-voiced breakdown of what changed and why (JARVIS energy), from a PR, commit range, or marble. Use when the "why" of a change is the story: architecture, a root-caused fix, a migration, a new subsystem.
---

# tardy Explainer

Read `/tardy-brag` first; it owns facts, the run directory, brand, safety, and grading. This file
is only what differs for Explainers.

## brag options

```
/brag --full --format vertical --tone polished --voice "JARVIS-style HUD briefing: calm, precise, a little smug"
```

Duration **20–30s** (`check.sh <run> 20 30`). Longer than brag's cap because the voice carries a
cause and an effect; cut anything that isn't one of them.

## Beats

1. **0–2s, the problem as a symptom.** One line the viewer recognizes: the bug, the slow thing, the
   missing feature. Show it (a failing test line, a stalled spinner from the simulator).
2. **The cause.** One diagram, built from the real code: modules as nodes, the bad edge in alarm red.
3. **The change.** Real diff lines from the PR, typed or highlighted, 3–6 lines max.
4. **The result.** The app doing the thing now, captured. Status pill `Shipped`.
5. **Sign-off.** PR number and the tardy wordmark.

## Push it

- A HUD layer that tracks what the voice says: rings, callouts, and labels that lock onto the diff
  line or node being named, timed to the narration words (`caption-pill-karaoke` timing data).
- The diagram is generated from the actual import graph or file list in the PR, not drawn by hand.
- One voice, but the HUD "talks back" with on-screen readouts (file counts, lines changed, test
  count) computed from `git diff --stat` and listed in `facts.md`.

## Grade on (type criteria for the rubric)

- **Cause → change → effect** is explicit; a cold viewer can say why the change was needed.
- Every diagram node and diff line exists in the PR.
- The voice never reads on-screen text verbatim for more than three words.
