import { anthropic } from '@ai-sdk/anthropic';
import { generateText, stepCountIs, tool } from 'ai';
import { z } from 'zod';

const spawnAgent = tool({
  description:
    'Spawn a coding agent to complete a task. Use this if you are asked to ' +
    'complete code-related tasks (writing code, fixing bugs, refactoring, etc.).',
  inputSchema: z.object({
    task: z.string().describe('What the agent should do, in full detail.'),
  }),
  execute: async ({ task }) => {
    console.log(`[spawn agent] noop — would spawn an agent for: ${task}`);
    return 'Agent spawned. It will report back in this thread when done.';
  },
});

export async function generateReply(opts: {
  botName: string;
  context: string;
  sender: string;
  content: string;
}): Promise<string> {
  const { text } = await generateText({
    model: anthropic('claude-opus-4-8'),
    system:
      `You are ${opts.botName}, a helpful assistant in a team chat channel. ` +
      'Reply to the message that tagged you, using the recent channel ' +
      'history for context. Be concise — this is chat, not email. ' +
      'If asked to complete code-related tasks, use the spawn_agent tool ' +
      'and tell the user the agent is on it.',
    prompt:
      `Recent channel history (oldest first):\n${opts.context}\n\n` +
      `Message tagging you, from ${opts.sender}:\n${opts.content}`,
    tools: { spawn_agent: spawnAgent },
    stopWhen: stepCountIs(3),
  });
  return text;
}
