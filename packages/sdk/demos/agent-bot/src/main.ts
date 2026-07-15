import { Macro } from '@macro/sdk';
import { Hono } from 'hono';
import { authorName, channelContext } from './context';
import { generateReply } from './reply';
import { resolveWebhookSecret } from './secret';

const webhookSecret = await resolveWebhookSecret();
const macro = new Macro({ env: 'dev', webhookSecret });

const me = await macro.users.me();
const myName = (await me.name()) ?? 'the assistant';
console.log(`Answering as ${myName}`);

macro.events.on('channel.message_posted', async ({ metadata, message }) => {
  if (metadata.sender === me.id) return; // don't reply to ourselves

  const mentioned = message.mentions.some(
    (m) => m.entity_type === 'user' && m.entity_id === me.id,
  );
  if (!mentioned) return;

  const [context, sender] = await Promise.all([
    channelContext(macro, metadata.channel_id, message.id),
    authorName(message),
  ]);

  const text = await generateReply({
    botName: myName,
    context,
    sender,
    content: metadata.content,
  });
  if (text) await message.reply(text);
});

const app = new Hono();
app.post('/webhook', (c) => macro.events.webhook()(c.req.raw));

export default { port: 3000, fetch: app.fetch };
console.log('Listening on http://localhost:3000 — POST /webhook');
