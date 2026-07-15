import { Macro } from '../src/macro';

// A newly created webhook has is_valid=false and is silently excluded from
// delivery matching until this challenge succeeds — the endpoint must already
// be up and reachable at its registered endpoint_url before running this.
const webhookId = process.argv[2];
const localProxyUrl = process.argv[3];
if (!webhookId)
  throw new Error(
    'usage: bun scripts/validate-webhook.ts <webhookId> [localProxyUrl]',
  );

const hosts = localProxyUrl
  ? { auth: `${localProxyUrl}/auth`, storage: `${localProxyUrl}/dss` }
  : undefined;

const macro = new Macro({ env: 'dev', hosts });
const result = await macro.webhooks.byId(webhookId).validate();
console.log(result);
