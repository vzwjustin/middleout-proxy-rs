# middleout-proxy-rs

A local, token-optimizing reverse proxy for **Anthropic Claude** (and a transparent passthrough for **OpenAI/Codex** Responses traffic), written in Rust.

It sits between your client (Claude Code, the Codex CLI, the SDKs, anything that speaks the Anthropic Messages API) and the upstream provider, and shrinks the **input** (and optionally output) payload with a stack of pluggable compression engines — while carefully **preserving Anthropic's native prompt cache** so you don't trade a server-side cache win for a smaller body. A built-in dashboard shows live compression savings, cost-by-model, latency, and recent traffic.

> Auth model: **subscription OAuth Bearer passthrough only.** The proxy forwards your existing `Authorization: Bearer …` header verbatim and **rejects `x-api-key` headers** — it does not inject or store credentials.

---

## Features

- **Cache-aware compression** — compresses volatile content while leaving everything at or before an Anthropic `cache_control` breakpoint untouched, so the upstream prompt cache stays valid. Can auto-insert a cache wall at the system/tools boundary.
- **Pluggable compression engines**, each independently toggleable with a level (lite/standard/aggressive/…):
  - `caveman` — aggressive whitespace/filler reduction
  - `rtk` — token-killer transforms
  - `json_aware` — structure-preserving JSON compaction (json-safe)
  - `lsh` / `jl_dedupe` — near-duplicate detection/removal
  - `lingua` (optional) — LLMLingua-2 lossy tail compression
  - `adaptive` — adapts behavior to payload
- **Input and output compression**, independently controlled.
- **Codex / OpenAI Responses passthrough** at `/v1/responses` → forwarded to the ChatGPT Codex backend; usage/cost is parsed from the response stream.
- **Cost tracking** by model and provider, with a **budget guard** and per-client **rate limiting** (token bucket, keyed on a hash of the auth header — raw tokens never leave the limiter).
- **Live dashboard** at `/dashboard` — savings chart, engine attribution, cost bars, error rate, p50/p95, recent requests (click a row for per-engine detail).
- **Runtime settings** — flip engines on/off and change levels live via the dashboard; persisted across restarts.

> Note: the optional **L1 (exact-match)** and **L2 (semantic)** response caches are intentionally **disabled** in the dashboard. They're a poor fit for agentic coding traffic (L1 almost never hits; L2 needs an external embedder and risks near-match false hits) — Anthropic's native prompt cache, which this proxy preserves, is the real caching win.

---

## Quick start

```bash
# Build (release recommended — far lower CPU than debug)
cargo build --release

# Run — binds 127.0.0.1:8787 by default
./target/release/middleout-proxy-rs
```

Then point your client at the proxy, e.g.:

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:8787
```

Open the dashboard at <http://127.0.0.1:8787/dashboard>.

> If you run a client in **strict subscription mode**, unset any conflicting API-key env var first, e.g.
> `env -u ANTHROPIC_AUTH_TOKEN ./target/release/middleout-proxy-rs`.

---

## Configuration

Settings resolve in order: **environment variable → TOML file → built-in default.** Point at a TOML file with `MIDDLEOUT_CONFIG=/path/to/config.toml`.

| Setting | Env var | Default |
|---|---|---|
| Bind host | `MIDDLEOUT_HOST` | `127.0.0.1` |
| Bind port | `MIDDLEOUT_PORT` | `8787` |
| Anthropic upstream | `PROXY_UPSTREAM_BASE_URL` | `https://api.anthropic.com` |
| Codex/OpenAI upstream | `MIDDLEOUT_OPENAI_UPSTREAM_URL` | `https://chatgpt.com/backend-api/codex` |
| Input compression | `MIDDLEOUT_INPUT_COMPRESSION` | on |
| Output compression | `MIDDLEOUT_OUTPUT_COMPRESSION` | off |
| Preserve Anthropic cache | `MIDDLEOUT_PRESERVE_ANTHROPIC_CACHE` | on |
| Auto-insert cache wall | `BRAIN_AUTO_INSERT_WALL` | on |
| Caveman / RTK / JSON-aware / LSH | `MIDDLEOUT_CAVEMAN`, `MIDDLEOUT_RTK`, `MIDDLEOUT_JSON_AWARE`, `MIDDLEOUT_LSH` (+ `_LEVEL`) | varies |
| Max text chars / head fraction | `MIDDLEOUT_MAX_TEXT_CHARS`, `MIDDLEOUT_HEAD_FRACTION` | — |
| Audit log dir | `MIDDLEOUT_AUDIT_DIR` | `.middleout-logs` |

(See `src/config.rs` for the full list, including JL-dedupe, rate-limit, and policy knobs.) Most engine flags and levels can also be changed **live** from the dashboard without a restart.

---

## Endpoints

| Path | Purpose |
|---|---|
| `POST /v1/messages` (and any other path) | Anthropic proxy (catch-all fallback) |
| `POST /v1/responses` | Codex / OpenAI Responses passthrough |
| `GET /dashboard` | Live web dashboard |
| `GET /healthz` | Health + effective settings snapshot |
| `GET /stats`, `/stats/timeseries`, `/stats/recent` | Compression/latency/error stats |
| `GET/POST /settings` | Read / update runtime settings |
| `GET /cost`, `POST /cost/reset` | Cost-by-model tracking |
| `GET /budget`, `POST /budget/reset` | Usage budget guard |
| `GET /rate-limit`, `/policies`, `/providers` | Limiter, routing policies, adapters |
| `POST /preview` | Dry-run compression on a payload |
| `GET /metrics` | Metrics |
| `GET /cache/stats`, `POST /cache/purge` | Cache stats / purge |

---

## Development

```bash
cargo build          # debug
cargo test           # run the test suite
cargo build --release
```

The compression engines live in `src/compression/engines/`, the HTTP layer in `src/server/`, cost/usage in `src/cost.rs`, and config in `src/config.rs`.
