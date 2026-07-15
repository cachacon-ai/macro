import { Macro } from '../src/macro';

const query = process.argv[2];
if (!query)
  throw new Error('usage: bun scripts/search-channel.ts <query> [searchOn]');
const searchOn = process.argv[3] as
  | 'name'
  | 'content'
  | 'name_content'
  | undefined;
const macro = new Macro({ env: 'dev' });

let count = 0;
for await (const channel of macro.channels.search(query, { searchOn })) {
  const [name, type] = await Promise.all([channel.name(), channel.type()]);
  console.log({ id: channel.id, name, type, url: channel.webUrl() });
  if (++count >= 5) break;
}
if (count === 0) console.log(`no channels matched '${query}'`);
