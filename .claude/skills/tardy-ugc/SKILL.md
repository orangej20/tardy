---
name: tardy-ugc
description: Make a tardy Fake UGC reel, a selfie-style "okay so I wasn't going to post this, but my agent just..." clip about a small, delightful win, from a PR, commit, or marble. Use for quality-of-life fixes and before/after moments a user would feel.
---

# tardy Fake UGC

Read `/tardy-brag` first; it owns facts, the run directory, brand, safety, and grading. This file
is only what differs for UGC.

## brag options

```
/brag --full --format vertical --tone chaotic --voice "phone-shot creator clip: breathless, conversational, talking to camera about their agent"
```

Duration **15–25s** (brag's window; `check.sh <run> 15 25`).

## The bit

- The "creator" is an **original** character, never a real influencer or a photoreal human face.
  Use an illustrated or abstract presenter (the agent's own avatar from its tardy profile is ideal),
  or no face at all: hands, a phone, a screen.
- Script shape: confession hook ("okay so…") → the before, mildly suffering → "and then my agent…"
  → the after, in the real app → one reaction line.
- Write it the way people talk: fragments, restarts, "like". Keep every fact in `facts.md`.

## Picture

Handheld feel: subtle camera shake and drift on the whole frame, phone-native captions
(`caption-pill-karaoke`) center screen, a fake "front camera" corner for the presenter, then a hard
cut to the real screen recording for the after.

## Push it

- **Before/after with `comparison-split`** on the real app: old behavior, wipe, new behavior.
- **Native-app parody chrome** made of tardy's own UI language (not another platform's logo or UI).
- Jump cuts on breaths, like a real creator edit, aligned to the voice track's pauses.

## Grade on (type criteria for the rubric)

- It sounds like a person, not a press release; no line could be ad copy.
- The before/after difference is visible on mute.
- Nothing imitates a real creator or another platform's branding.
