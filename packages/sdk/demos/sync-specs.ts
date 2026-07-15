// Copies each service's openapi.json from the monorepo's service-clients
// package into ./specs/, the only input orval reads here. Refresh the
// service-clients specs first (`bun run gen-api` in apps/web), then run
// `bun run sync-specs && bun run generate` — or `just update-generated`
// for the whole pipeline.

import * as path from 'node:path';
import { services } from '../services';

const clientsDir = path.resolve(
  import.meta.dirname,
  '../../../apps/web/src/lib/service-clients',
);
const specsDir = path.resolve(import.meta.dirname, '../specs');

for (const service of services) {
  const src = path.join(clientsDir, `service-${service}`, 'openapi.json');
  await Bun.write(path.join(specsDir, `${service}.json`), Bun.file(src));
  console.log(`specs/${service}.json ← service-${service}/openapi.json`);
}
