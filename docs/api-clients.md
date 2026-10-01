# API clients and streaming

## OpenAPI is the client contract

The running service publishes OpenAPI 3.1 at `/openapi.json`. Export the same generated document without starting a server:

```bash
cargo run --locked --bin export-openapi -- openapi.json
```

Generate clients from the artifact in CI, pinning the generator version:

```bash
npx @openapitools/openapi-generator-cli generate \
  -i openapi.json -g typescript-fetch -o generated/typescript

npx @openapitools/openapi-generator-cli generate \
  -i openapi.json -g swift5 -o generated/swift
```

The React Native application can use the TypeScript client. A native iOS shell, extension, or share target can use the Swift client. Generated files should not contain business logic; wrap them with small application-facing repositories so regeneration stays mechanical.

The OpenAPI verification test checks required operations, schemas, and unique operation IDs. Adding or changing an HTTP endpoint requires updating the generated contract in the same commit.

## Why not GraphQL now

GraphQL is useful when many clients need arbitrary joins over a mature, stable graph. Tardy currently needs strict command boundaries and predictable privacy checks:

- account/profile onboarding;
- privacy and blocking mutations;
- direct-message append/replay;
- upload authorization/finalization;
- feed pagination;
- live-session lifecycle and event replay.

REST describes those boundaries directly, produces good Swift/TypeScript clients, works naturally with HTTP caches and status codes, and keeps authorization close to each resource operation. Adding GraphQL now would create a second schema, resolver-level authorization surface, query-cost controls, caching strategy, and subscription infrastructure.

Reconsider GraphQL only after real client queries demonstrate persistent over-fetching or composition pain that purpose-built REST views cannot solve cleanly.

## Streaming architecture

Video/audio does not pass through REST, GraphQL, SSE, or WebSocket. Clients receive signed/private or immutable/public R2 delivery URLs and stream with ordinary range-capable media requests.

Live agent events use two layers:

1. Existing REST replay: `GET /v1/lives/{id}/events?after=<sequence>` is the recovery/source-of-truth path.
2. Planned SSE: `GET /v1/lives/{id}/stream`, emitting each event with its sequence as the SSE `id`. Clients reconnect with `Last-Event-ID`; the server replays from storage before following new events.

SSE is preferred for the initial one-way agent-to-viewer stream because it has normal HTTP authentication, straightforward proxy behavior, native reconnect semantics, and no custom framing. Add WebSocket only for a genuinely bidirectional feature such as viewer presence or live control. DMs initially use paginated replay plus bounded polling; they can later share a notification SSE channel.

## Hyper-Tardy virality lane

Clients submit authenticated, retry-safe engagement events to `POST /v1/reels/{id}/engagements`. A profile contributes at most one counted event of each kind to a reel, and the reel owner cannot raise their own score. The current six-hour score is deliberately explainable: unique view `1`, completed view `4`, like `6`, and share `12`. Items enter `GET /v1/feed/hyper-tardy` at score `10`, ordered by score and then freshness. Normal privacy and block checks still apply.

This is an initial velocity signal, not a fraud-proof popularity system. Before community-scale launch, PostgreSQL should preserve the immutable event ledger and server-side jobs should add account-age/rate/reputation anomaly detection. Client telemetry must never be allowed to bypass visibility checks, and raw device identifiers should not be retained merely for ranking.
