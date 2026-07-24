# FORK: BYOK (Kimi Code + MiniMax) — setup & verification guide

This fork of [macro-inc/macro](https://github.com/macro-inc/macro) reroutes
Macro's entire AI layer to bring-your-own-key providers. Everything is
configured by environment variables — **no keys are stored in the repo** (the
fork is public; never commit secrets).

## What changed

| Area | File(s) | Change |
|---|---|---|
| Model router | `crates/agent/src/model/router.rs` | All providers optional; Anthropic arm is now a registry with `with_anthropic_provider(name, base_url, key)`. `KIMI_API_KEY`/`KIMI_BASE_URL` (default `https://api.kimi.com/coding`) and `MINIMAX_API_KEY`/`MINIMAX_BASE_URL` (default `https://api.minimax.io/v1`) registered from env. |
| Model tiers | `crates/agent/src/model/predefined_model.rs` | `Smart` → `kimi/k3`, `Fast` → `kimi/kimi-for-coding`, `Sonnet4_6` (memory judge, call summarizer) → `minimax/MiniMax-M2.7`. Moves memory builds, subagents, chat renaming, summaries with zero call-site edits. |
| Thinking config | `crates/agent/src/model/anthropic.rs` | `kimi-for-coding` gets `thinking: enabled` (endpoint requires it); `k3` sends nothing (thinks by default). |
| Chat defaults | `crates/chat/src/domain/models/model_access.rs` | Chat model list + paid/free defaults lead with Kimi/MiniMax. |
| Doc edit chains | `crates/documents/src/outbound/editing_worker_client.rs`, `apps/web/src/lib/service-clients/ai-editing-worker/client.ts` | Per-role fallback chains: Kimi primary → MiniMax fallback. |
| Frontend picker | `apps/web/src/lib/core/component/AI/constant/model.ts` (+ `assets/kimi.svg`, `assets/minimax.svg`) | Kimi K3 / K2.7 Code, MiniMax M3 / M2.7 HS selectable; Kimi K3 paid default, MiniMax M2.7 HS free default. |
| AI editing worker | `services/ai-editing-worker/src/{endpoints/edit.ts, run-edit.ts, env.ts}` | `kimi` provider via `createAnthropic({apiKey, baseURL})`; `minimax` via `createOpenAI(...).chat()`; all provider keys optional. |

Design note: Kimi Code rides the **Anthropic-compatible** surface
(`api.kimi.com/coding`). Its OpenAI-compatible surface is gated to
whitelisted coding agents; the Anthropic one accepts third-party clients
(same wiring as the switchboard relay).

## Environment variables

Set these wherever you run the backend (your `local.env`, shell, Pulumi, or
`wrangler secret` for the worker):

```bash
# Required for BYOK
KIMI_API_KEY=sk-kimi-...            # Kimi Code console key
MINIMAX_API_KEY=sk-...              # MiniMax international API key

# Optional overrides (defaults already point at the right endpoints)
# KIMI_BASE_URL=https://api.kimi.com/coding
# MINIMAX_BASE_URL=https://api.minimax.io/v1

# Optional but recommended
OPENAI_API_KEY=sk-...               # embeddings (crates/embedding) still use OpenAI
# ANTHROPIC_API_KEY=...             # only if you want the Claude variants + web-fetch server tool
# CEREBRAS_API_KEY=...              # only if you want the cerebras import path
```

Any provider without a key simply isn't registered; unroutable model ids fall
back to `Smart` (Kimi K3). If *no* provider keys exist, the router panics on
first use with a message listing the expected vars.

## Step 1 — compile & unit verification (no Docker needed)

This is the gate for the patch itself. Needs only a Rust toolchain (see
`rust-toolchain.toml`) and Bun:

```bash
git clone git@github.com:cachacon-ai/macro.git
cd macro

cargo check -p agent -p chat -p documents
cargo test  -p agent -p chat

cd services/ai-editing-worker
bun install
bun run type-check   # or: bunx tsc --noEmit
cd ../..

# frontend constants tests
cd apps/web && bun install && bunx vitest run src/lib/core/component/AI/constant/model.test.ts
```

The patch was written via the GitHub API without a local checkout, so treat
this step as the merge gate, not a formality.

## Step 2 — run the full stack locally

Requires: Docker Compose v2, Rust toolchain, `cargo-zigbuild` + Zig, Bun,
sqlx CLI (or the repo's Nix shell, which provides all of it). You don't have
Macro's Doppler org, so run with an env file:

```bash
just doctor-local                                # preflight
just run_local --no-doppler --env-file ./local.env
```

- First cold run migrates the DB, runs the FusionAuth kickstart, and creates
  the search indices; later runs restore the cached init snapshot.
- `local.env` must contain the normal service config **plus** the BYOK vars
  above. Generated per-instance files land in `infra/local/generated/`.
- Press `r` to rebuild changed Rust services, `q` to tear down cleanly.
- Headless alternative: `just stack up` (everything behind one origin, no
  terminal attached). See `docs/RUNNING_LOCALLY.md`.

For the Cloudflare editing worker: deploy your own copy
(`services/ai-editing-worker`) with `wrangler secret put KIMI_API_KEY` /
`MINIMAX_API_KEY`, and point the frontend at it via
`VITE_AI_EDITING_WORKER_URL`.

## Step 3 — smoke tests

1. **Chat**: open a channel/agent chat, select **Kimi K3** in the picker,
   send a message. Expect a normal streamed reply (thinking renders if the
   endpoint streams reasoning).
2. **Fast tier**: select **Kimi K2.7 Code**, ask for a small code change.
3. **Fallback**: temporarily unset `KIMI_API_KEY` and restart — requests
   should silently fall back (default model resolution), not 500.
4. **Doc edit**: open a markdown doc, run an AI edit; this exercises the
   worker's Kimi→MiniMax fallback chains per role.
5. **Background jobs**: rename a chat (Fast), then check nightly memory build
   logs (Smart) and a call summary (MiniMax M2.7) if you use calls.

## Known gaps (deliberate)

- **Embeddings** (`crates/embedding`) still call OpenAI — keep an
  `OPENAI_API_KEY` (cheap) or repoint its base URL at an OpenAI-compatible
  embeddings server (e.g. LM Studio).
- **Anthropic server tools** (web fetch / code execution in
  `crates/ai_tools/src/build_context.rs`) need an Anthropic key; without one
  those specific tools fail, everything else works.
- **Usage/cost dashboards** don't know BYOK pricing (tokens log fine; dollar
  figures won't).
- Kimi K3 context is set to 262K (`predefined_model.rs`); bump to 1M if your
  Kimi Code plan is Allegretto.

## Security

- Never commit API keys. This fork is **public** (forks of public repos
  always are); anything committed is published.
- If a key was ever pasted into a chat/issue/PR, rotate it in the provider
  console before relying on it.
