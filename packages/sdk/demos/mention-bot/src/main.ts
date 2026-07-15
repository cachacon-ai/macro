import { appendFile } from 'node:fs/promises';
import { Macro, msg } from '@macro/sdk';

// Either point at the local proxy (from the host, e.g. http://localhost:31009)
// or, when this bot itself runs as a container on the backend's docker
// network, give the internal service hostnames directly (no proxy prefix).
const hosts =
  process.env.AUTH_HOST_URL || process.env.STORAGE_HOST_URL
    ? { auth: process.env.AUTH_HOST_URL, storage: process.env.STORAGE_HOST_URL }
    : process.env.LOCAL_PROXY_URL
      ? {
          auth: `${process.env.LOCAL_PROXY_URL}/auth`,
          storage: `${process.env.LOCAL_PROXY_URL}/dss`,
        }
      : undefined;

// Bind the server before registering the webhook — Macro's validation
// challenge needs a live endpoint to hit, and self-registration below would
// otherwise race the listener coming up.
let onWebhookRequest: (req: Request) => Promise<Response>;
Bun.serve({
  port: 3000,
  fetch: (req) => onWebhookRequest(req),
});
console.log('listening on http://localhost:3000');

// The signing secret is minted once, at creation, and never returned again —
// so on first run we register the webhook, wait for it to validate, and
// persist the secret to .env; every later start reuses it.
async function resolveWebhookSecret(): Promise<string> {
  const existing = process.env.MACRO_WEBHOOK_SECRET;
  if (existing) return existing;

  const url = process.env.WEBHOOK_URL;
  if (!url) {
    throw new Error(
      'No MACRO_WEBHOOK_SECRET set, and no WEBHOOK_URL to self-register with. ' +
        "Set WEBHOOK_URL to this server's public /webhook URL (e.g. an ngrok " +
        'tunnel to http://localhost:3000/webhook).',
    );
  }

  const setup = new Macro({ env: 'dev', hosts });
  const webhook = await setup.webhooks.create({
    url,
    name: 'mention-bot',
    filters: [{ events: ['channel.message_posted'] }],
  });
  const secret = webhook.signingSecret;
  if (!secret) throw new Error('webhook created without a signing secret');

  // The validation challenge can race this process's own listener coming up
  // (or a slow container network), so retry briefly instead of giving up
  // after one attempt.
  let validated = false;
  for (let attempt = 1; attempt <= 5 && !validated; attempt++) {
    await new Promise((r) => setTimeout(r, attempt * 500));
    const result = await webhook.validate();
    validated = result.is_valid;
    if (!validated) {
      console.log(
        `webhook validation attempt ${attempt} failed: ${result.message}`,
      );
    }
  }
  if (!validated) {
    throw new Error(
      `webhook ${webhook.id} was created but never validated — check that ` +
        `${url} is reachable from the backend`,
    );
  }

  await appendFile(
    '.env',
    `\nMACRO_WEBHOOK_SECRET=${secret}\nWEBHOOK_ID=${webhook.id}\n`,
  );
  console.log(`webhook ${webhook.id} registered and validated`);
  return secret;
}

const webhookSecret = await resolveWebhookSecret();
const macro = new Macro({ env: 'dev', hosts, webhookSecret });
const me = await macro.users.me();
console.log(`replying as ${(await me.name()) ?? me.id}`);

onWebhookRequest = async (req) => {
  try {
    return await macro.events.webhook()(req);
  } catch (err) {
    console.error('rejected webhook delivery:', err);
    return new Response('invalid', { status: 401 });
  }
};

macro.events.on('channel.message_posted', async ({ metadata, message }) => {
  if (metadata.sender === me.id) return; // don't reply to ourselves

  const mentioned = message.mentions.some(
    (m) => m.entity_type === 'user' && m.entity_id === me.id,
  );
  if (!mentioned) return;

  const sender = macro.users.byId(metadata.sender);
  await message.reply(msg`Hi there, ${sender}!`);
});
