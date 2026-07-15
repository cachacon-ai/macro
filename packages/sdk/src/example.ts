import { Hono } from 'hono';
import { Macro } from './macro';
import { msg } from './mentions';

const macro = new Macro({
  token: 'mk_live_...',
  env: 'dev',
  webhookSecret: 'whsec_...',
});

const HOUR_MS = 60 * 60 * 1000;
const reminders = new Map<string, ReturnType<typeof setTimeout>>();

const CHANNEL_ID = 'ch_abc';
const channel = macro.channels.byId(CHANNEL_ID);

macro.events?.on('channel.message_posted', async ({ metadata, message }) => {
  if (metadata.channel_id !== CHANNEL_ID) return;

  const match = (await message.content()).match(/^(\d+)h\s+(.+)$/);
  if (!match) return;

  const [, hours, note] = match;
  const author = await message.author();

  const timer = setTimeout(async () => {
    await channel.send(msg`⏰ ${author} reminder: ${note}`);
    reminders.delete(message.id);
  }, Number(hours) * HOUR_MS);

  reminders.set(message.id, timer);
  await message.react('alarm_clock');
});

macro.events?.on('channel.message_deleted', async ({ metadata, message }) => {
  if (metadata.channel_id !== CHANNEL_ID) return;

  const timer = reminders.get(message.id);
  if (!timer) return;
  clearTimeout(timer);
  reminders.delete(message.id);
});

const app = new Hono();
app.post('/webhook', (c) => macro.events?.webhook()(c.req.raw));

export default app;
