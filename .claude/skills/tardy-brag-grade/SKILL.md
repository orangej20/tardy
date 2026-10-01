---
name: tardy-brag-grade
description: Grade a finished tardy reel (or any /brag video) against the tardy rubric and write a scorecard with a ship / revise / blocked verdict. Use after /tardy-brag or any post-type skill renders, when asked to grade, score, review, or evaluate a brag video, or to compare reels over time.
---

# tardy-brag-grade

Grade one run directory (`content/<date>-<type>-<slug>/`, or a legacy `brag-output*/`).
Grade in a fresh context: if you built this reel in this session, hand the grade to a subagent or
a new session instead.

## 1. Gates

```bash
.claude/skills/tardy-brag-grade/check.sh <run> <min-s> <max-s>
```

The window comes from the post-type skill (`tardy-<type>/SKILL.md`, "Duration"). The script checks
format, duration, dead air, the poster, `npx hyperframes check`, and palette against
`mobile/src/theme/index.ts`, and writes the evidence set to `<run>/grade/`.

Then do the human-judgment gates from `rubric.md`: `truth` against `<run>/facts.md` (a run with no
`facts.md` fails `truth`), `originals`, `privacy`.

## 2. Score

Read `rubric.md` (this directory) and the post-type skill's "Grade on" list. Look at every still in
`grade/stills/` and the contact sheet, watch once with sound and once muted (play `brag.mp4` with
`open` or read frames with ffmpeg), then score each category with evidence.

## 3. Scorecard

Write `<run>/scorecard.json`:

```json
{
  "rubric": "v1",
  "run": "content/2026-10-01-podcast-audio-row",
  "type": "podcast",
  "video_sha256": "<from check.sh>",
  "duration_s": 38.2,
  "graded_at": "2026-10-01T18:00:00Z",
  "grader": "<model or person>, fresh context",
  "gates": {
    "technical": { "pass": true, "evidence": "check.sh: all PASS" },
    "truth": { "pass": true, "evidence": "11/11 lines sourced; spot-checked PR #10, 2be7ec5, playback.ts:42" },
    "originals": { "pass": true, "evidence": "..." },
    "privacy": { "pass": true, "evidence": "..." }
  },
  "scores": {
    "hook": { "score": 3, "evidence": "hook-0.5s: ..." },
    "message": { "score": 3, "evidence": "..." },
    "proof": { "score": 4, "evidence": "..." },
    "type_fit": { "score": 3, "evidence": "..." },
    "brand": { "score": 3, "evidence": "..." },
    "pacing": { "score": 2, "evidence": "..." },
    "sound": { "score": 3, "evidence": "..." },
    "craft": { "score": 3, "evidence": "..." }
  },
  "limits_pushed": [{ "move": "two-voice mix", "result": "landed" }],
  "total": 76.25,
  "verdict": "ship",
  "fixes": ["<most valuable fix, with timestamp>", "<second>", "<third>"]
}
```

`fixes` is always the three highest-value changes, ranked, each concrete enough to apply without
re-watching (timestamp, what's wrong, what to do), even on `ship`.

## 4. Report

One line: verdict, total, the weakest category, and fix #1. Then the path to `scorecard.json`.

Compare reels over time:

```bash
jq -s 'map({run, type, rubric, total, verdict})' content/*/scorecard.json
```
