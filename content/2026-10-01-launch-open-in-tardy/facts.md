# Facts: "Open in Tardy" launch reel (keynote cut)

Event: ajmwagar/tardy PR #16, "iOS share extension: \"Open in Tardy\" sends links to your agents".
Merged 2026-10-01 (merge commit `73bdafb`). Every claim the reel makes, one per line, with its source.

| Claim on screen | Source |
|---|---|
| "Share any link to your agents." | PR #16 body: "share any link from Safari, YouTube or another app straight to your agents" |
| The extension is named "Open in Tardy" | `mobile/app.json` `iosShareExtensionName` (`7526b47`) |
| App icon shown is Tardy's real icon | `mobile/assets/images/icon.png` |
| "Tap Share. Tardy's in the row." | `composition/assets/reveal.mp4` (real take, standalone simulator build: Safari share row shows Tardy); PR #16 body |
| The shared link here is PR #16 itself (github.com card "ajmwagar tardy pull 16") | `reveal.mp4` (repo is public) |
| "Your agents, first." / "Your agents first." | PR #16 body; `reveal.mp4` ("Your agents" row above "Recent") |
| Agent avatars on the bento card are opus.backend and sonnet.ui | cropped from `reveal.mp4` |
| "One tap to send. A work chat starts." | PR #16 body: "one tap to send. Sending to an agent starts a work chat" |
| "✓ Sent to opus.backend" | `reveal.mp4` (opus.backend selected, Send tapped, sheet closes); PR #16 body ("pick opus.backend → … Send → sent") |
| "0 lines of Swift." / "expo-share-intent builds the extension." | `7526b47` message: "expo-share-intent … generates the extension target … No first-party Swift" |
| "Any link: Safari, YouTube, any app that shares one." | PR #16 body; `mobile/app.json` `iosActivationRules` (web URLs, web pages, text with a link) |
| "401 tests passing." | PR #16 body ("401 tests pass") |
| "+203 −75", "10 files" | `gh pr view 16 --json additions,deletions,files` |
| "Merged October 1 · iOS" | PR #16 `mergedAt` 2026-10-01T08:32:13Z; iOS share extension |
| "Real followers, real friends. Stay Tardy." | `README.md:22` sign-off (also `docs/brand/BRAND.md`) |

Not claimed: users, downloads, TestFlight/App Store availability (PR #16 says the next production
build creates provisioning; nothing is shipped to a store), Android behavior.
