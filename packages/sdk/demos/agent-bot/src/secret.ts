import { appendFile } from 'node:fs/promises';
import { Macro } from '@macro/sdk';

// The secret is minted once, when the webhook is created, and never returned
// again — so on first run we register the webhook, persist the secret to .env,
// and reuse it on every subsequent start (bun auto-loads .env).
export async function resolveWebhookSecret(): Promise<string> {
  const existing = process.env.MACRO_WEBHOOK_SECRET;
  if (existing) return existing;

  const url = process.env.WEBHOOK_URL;
  if (!url) {
    throw new Error(
      'No MACRO_WEBHOOK_SECRET set, and no WEBHOOK_URL to self-register with. ' +
        "Set WEBHOOK_URL to this server's public /webhook URL (e.g. an ngrok " +
        'tunnel to http://localhost:3000/webhook) so the bot can register itself.',
    );
  }

  const macro = new Macro({ env: 'dev' });
  const webhook = await macro.webhooks.create({
    url,
    name: 'agent-bot',
    filters: [{ events: ['channel.message_posted'] }],
  });

  const secret = webhook.signingSecret;
  if (!secret) throw new Error('webhook created without a signing secret');

  await appendFile('.env', `\nMACRO_WEBHOOK_SECRET=${secret}\n`);
  console.log(
    `Registered webhook ${webhook.id} at ${url}; saved MACRO_WEBHOOK_SECRET to .env`,
  );
  return secret;
}
