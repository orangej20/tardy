# tardy reel rubric, v1

Bump the version when any gate, category, weight, anchor, or threshold changes. Scores are only
comparable within one version.

## Gates (pass/fail; any fail means `blocked`)

| Gate | Pass when | How |
|---|---|---|
| `technical` | Every `check.sh` line is PASS | `check.sh` output |
| `truth` | Every claim, number, name, and UI state in the reel is in `facts.md` with a source, and every source checks out | Transcribe the on-screen text and narration, match each line to `facts.md`, spot-check three sources in git |
| `originals` | No real person's likeness, voice imitation, or name as a character; no third-party game footage, characters, logos, or music | Stills + listen through |
| `privacy` | No secrets, emails, private repo names, internal URLs | Stills + transcript |

## Scored categories (0–4 each)

Score from the evidence set (`grade/stills/`, `grade/contact-sheet.png`), one full watch with
sound, and one full watch muted. Every score cites a timestamp or still as evidence.

| Category | Weight | 4 | 2 | 0 |
|---|---|---|---|---|
| `hook` | 15 | `hook-0.5s` alone says something specific to this event and makes you want the next second | Specific by 2s, generic before | Logo, fade-in, or generic text for 2s+ |
| `message` | 15 | A cold viewer can state what changed and why it matters after one watch | They get the topic, not the point | Unclear what the reel is about |
| `proof` | 15 | Real app capture or real diff carries a central beat | Real UI appears briefly or only as a still | Only redrawn or abstract visuals |
| `type_fit` | 15 | Meets every "Grade on" line in the post-type skill and feels like that channel | Meets most; format is recognizable | Could be any type |
| `brand` | 10 | Theme palette used with intent (yellow for the hero action, red only for urgency), SF Rounded wordmark, BRAND.md voice | Palette right, voice generic or accents overused | Off-brand look or voice |
| `pacing` | 10 | Every cut motivated, text holds ~0.3s/word, no dead beat, ends before you want it to | One slow stretch or one flash-read line | Drags, or text unreadable at speed |
| `sound` | 10 | Mix clean, voice intelligible, cuts and hits on the beat, and the reel still works muted | Fine with sound, loses meaning muted | Clipping, desync, or meaningless without sound |
| `craft` | 10 | Nothing in the chrome safe zones, no typos, overflow, aliasing, or half-animated poster | One minor flaw | Multiple visible flaws |

`total` = Σ(score / 4 × weight), out of 100.

## Limits pushed (not scored, recorded)

List each move from `brag-plan.md` `## Limits pushed` and mark it `landed`, `weak`, or `missing`.
Fewer than two `landed` is a fix item.

## Verdict

- `blocked`: any gate fails.
- `ship`: all gates pass, `total` ≥ 75, no category below 2.
- `revise`: everything else.

## Calibration

Before grading, read the most recent `ship` scorecard of the same type (if any) and its stills.
Use it as the anchor for what a 3 looks like. Do not raise a score because the reel improved on
its own last revision; grade the reel, not the delta.
