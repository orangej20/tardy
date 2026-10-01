# Saved posts, search, and Explore

## Current slice

Saved posts are private, account-scoped records. Saving is idempotent and retains the original `saved_at_ms`; unsaving is also idempotent. Listing re-runs the normal visibility and blocking policy, so a later block or privacy change removes an inaccessible reel from results without exposing that it remains in the account's underlying save ledger.

The API surface is:

- `PUT /v1/saved-posts/{reel_id}`
- `DELETE /v1/saved-posts/{reel_id}`
- `GET /v1/saved-posts`
- `POST /v1/ai-consents/search`
- `DELETE /v1/ai-consents/search`
- `POST /v1/search`
- `POST /v1/explore`

Search and Explore use a provider-owned cross-encoder reranker through a narrow Rust `Reranker` interface. Configure Voyage with `VOYAGE_API_KEY`; `VOYAGE_RERANK_MODEL` and `VOYAGE_RERANK_URL` are optional. The default model is `rerank-2.5-lite`. Calls have a five-second timeout and fail visibly when unavailable.

Search requires active, versioned consent for the configured provider. Only explicitly public feed candidates are serialized for the provider. Private/unlisted content, saved-post metadata, account identifiers, and profile identifiers do not leave Tardy in this slice.

Candidate enumeration is intentionally bounded to the newest 100 public items while the social store is in memory. This is a bootstrap path, not the final hybrid index.

## PG17 hybrid retrieval

During the PostgreSQL 17 migration, replace candidate enumeration behind the search boundary with:

1. ParadeDB `pg_search` BM25 candidates for exact handles, repositories, model names, and source terms.
2. `pgvector` HNSW candidates for semantic similarity, with iterative scans enabled for filtered queries.
3. Reciprocal Rank Fusion over stable `(source, rank, post_id)` inputs.
4. The existing reranker over the top 30–100 fused candidates.
5. Deterministic freshness, visibility, block, moderation, and diversity rules after relevance scoring.

Required durable relations are `saved_posts`, `search_documents`, `search_embeddings`, `embedding_jobs`, and the existing `external_ai_consents` ledger. Embedding rows must record provider, model, dimensions, source-content digest, and creation time so model migrations are explicit and resumable. Search documents should reference canonical posts rather than duplicate authorization state.

Do not dual-write search state from request handlers. Persist canonical post changes and use a transactional outbox worker to refresh derived text/vector indexes. Deletion and visibility changes must invalidate searchability before or atomically with public exposure.

## RAG boundary

Search returns source posts and relevance scores; it does not currently generate prose. A later answer endpoint may synthesize over the retrieved subset, but it must preserve source links, distinguish source text from model output, use its own explicit consent purpose, and remain optional. Retrieval evaluation comes before generated answers.

## Quality evaluation

Maintain a versioned judgment set containing queries, eligible post IDs, relevance grades, and privacy-negative cases. Compare lexical-only, vector-only, fused, and reranked results using Recall@50, nDCG@10, zero-result rate, latency, and privacy violations. Do not promote a model or fusion change solely from anecdotal examples.
