# agent-bot

Demo: an LLM-powered bot. When someone @-mentions the bot's user in a channel,
it feeds the last 10 channel messages plus the tagging message to Claude (via
the Vercel AI SDK) and replies in the thread. The model has one tool,
`spawn_agent`, which it's told to use for code-related asks — currently a noop
that just logs.

Posts as whatever user `MACRO_API_KEY` belongs to — there's no separate bot
identity here, just a regular Macro account acting as the bot.

## Setup

```bash
bun install
```

Set env vars (or add to `.env`):

```
MACRO_API_KEY=your_macro_api_key
ANTHROPIC_API_KEY=your_anthropic_api_key
```

## Run

Expose this server publicly first (e.g. `ngrok http 3000`), then point
`WEBHOOK_URL` at the tunnel and start:

```bash
WEBHOOK_URL=https://your-tunnel.ngrok.app/webhook bun start
```

On first run, with no `MACRO_WEBHOOK_SECRET` set, the bot registers the webhook
itself using just `MACRO_API_KEY`. The signing secret is minted by Macro and
returned only once, at creation — so the bot appends it to `.env` as
`MACRO_WEBHOOK_SECRET` and reuses it on every later start (you can then drop
`WEBHOOK_URL`, since re-registration is skipped once the secret exists).

Then @-mention the bot's user in any channel it participates in. Code-related
asks ("can you fix the flaky test?") make the model call `spawn_agent` — watch
the server log for the noop.
