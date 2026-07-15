import type { Macro } from '@macro/sdk';

const CONTEXT_MESSAGES = 10;

// sender id -> display name, so context lines don't refetch the same author
const names = new Map<string, string>();

export async function authorName(message: {
  author(): Promise<{ id: string; name(): Promise<string | undefined> }>;
}): Promise<string> {
  const author = await message.author();
  const cached = names.get(author.id);
  if (cached) return cached;
  const name = (await author.name()) ?? author.id;
  names.set(author.id, name);
  return name;
}

/** The channel's last messages as "Name: content" lines, oldest first. */
export async function channelContext(
  macro: Macro,
  channelId: string,
  excludeMessageId: string,
): Promise<string> {
  const channel = macro.channels.byId(channelId);
  const lines: string[] = [];
  for await (const message of channel.messages({
    pageSize: CONTEXT_MESSAGES,
  })) {
    if (message.id === excludeMessageId) continue;
    lines.push(`${await authorName(message)}: ${await message.content()}`);
    if (lines.length >= CONTEXT_MESSAGES) break;
  }
  return lines.reverse().join('\n');
}
