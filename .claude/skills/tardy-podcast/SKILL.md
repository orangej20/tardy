---
name: tardy-podcast
description: Make a tardy Fake Podcast reel, two AI hosts arguing about a real decision in the codebase (a migration, a dependency, a trade-off), from a PR, commit range, or marble. Use when a change has a genuine trade-off someone could disagree with.
---

# tardy Fake Podcast

Read `/tardy-brag` first; it owns facts, the run directory, brand, safety, and grading. This file
is only what differs for podcasts.

## brag options

```
/brag --full --format vertical --tone deadpan --voice "two-host tech podcast clip: one skeptic, one believer, they argue, then agree on the facts"
```

Duration **30–45s** (`check.sh <run> 30 45`). This breaks brag's 25s cap on purpose: a dialogue
needs room for three exchanges. If it can't hold attention for 30s, it's an Explainer.

## The script

- Two **original** hosts with names, two distinct Kokoro voices, and a consistent seat each
  (left/right). Never a real podcaster's name, voice, or show format by name.
- Three exchanges: **claim** (believer states the change), **objection** (skeptic raises the real
  cost, from the PR or its review comments), **resolution** (the fact that settles it, from `facts.md`).
- Every technical claim either host makes is in `facts.md`. The bickering can be invented; the
  engineering can't.
- End on a joke that is about the change, not about podcasts.

## Picture

Podcast-clip framing: two host cards (original avatars, flat style, tardy palette), a live
waveform under whoever speaks, and big karaoke captions (`caption-pill-karaoke`) in the middle
third. Cut away to the real diff or the app every time a host names something concrete.

## Push it

- **Real two-voice mix:** each host on their own track, slight overlap on interruptions, duck the
  bed under speech (`/hyperframes-audio`).
- **Speaker-driven waveform** from the voice tracks themselves, not a fake loop.
- **"Receipts" cutaways:** when the skeptic objects, the actual review comment or failing CI line
  slams in as a quote card.

## Grade on (type criteria for the rubric)

- Both sides of a real trade-off are present; the skeptic's objection is a genuine cost.
- Hosts are distinguishable by voice alone (eyes closed test).
- Captions stay in sync within ~0.1s of each word.
