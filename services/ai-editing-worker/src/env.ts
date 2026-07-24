import { envsafe, str } from 'envsafe';
import type { D1Database } from './traces-db';

export type Bindings = {
  ANTHROPIC_API_KEY: string | undefined;
  CEREBRAS_API_KEY: string | undefined;
  OPENAI_API_KEY: string | undefined;
  /** FORK: BYOK provider credentials. All optional — a role whose provider has
   * no key errors and the fallback chain advances. */
  KIMI_API_KEY: string | undefined;
  KIMI_BASE_URL: string | undefined;
  MINIMAX_API_KEY: string | undefined;
  MINIMAX_BASE_URL: string | undefined;
  SYNC_WS_BASE: string;
  /** D1 database storing edit-session traces. Absent in envs without the binding. */
  TRACES_DB: D1Database | undefined;
  /** Shared admin key gating the trace-read endpoint; validated via getEnv. */
  TRACE_ADMIN_KEY: string | undefined;
  /** Org internal service-to-service key; accepted on the trace endpoints via
   * the `x-internal-auth-key` header (used by the delete-document worker). */
  INTERNAL_API_KEY: string | undefined;
};

export type Env = ReturnType<typeof validateEnv>;

function validateEnv(rawEnv: Bindings) {
  const {
    ANTHROPIC_API_KEY,
    CEREBRAS_API_KEY,
    OPENAI_API_KEY,
    KIMI_API_KEY,
    KIMI_BASE_URL,
    MINIMAX_API_KEY,
    MINIMAX_BASE_URL,
    SYNC_WS_BASE,
    TRACE_ADMIN_KEY,
    INTERNAL_API_KEY,
  } = rawEnv;
  return envsafe(
    {
      // FORK: provider keys are all optional — the deployment may run on any
      // subset of providers. Empty means "not configured"; the endpoint layer
      // throws on use so fallback chains advance.
      ANTHROPIC_API_KEY: str({ default: '', allowEmpty: true }),
      CEREBRAS_API_KEY: str({ default: '', allowEmpty: true }),
      OPENAI_API_KEY: str({ default: '', allowEmpty: true }),
      KIMI_API_KEY: str({ default: '', allowEmpty: true }),
      KIMI_BASE_URL: str({ default: '', allowEmpty: true }),
      MINIMAX_API_KEY: str({ default: '', allowEmpty: true }),
      MINIMAX_BASE_URL: str({ default: '', allowEmpty: true }),
      SYNC_WS_BASE: str({ allowEmpty: false }),
      // Empty when unset; the trace-read endpoint stays closed until it's set.
      TRACE_ADMIN_KEY: str({ default: '', allowEmpty: true }),
      // Empty when unset; internal-key trace access stays closed until it's set.
      INTERNAL_API_KEY: str({ default: '', allowEmpty: true }),
    },
    {
      env: {
        ANTHROPIC_API_KEY,
        CEREBRAS_API_KEY,
        OPENAI_API_KEY,
        KIMI_API_KEY,
        KIMI_BASE_URL,
        MINIMAX_API_KEY,
        MINIMAX_BASE_URL,
        SYNC_WS_BASE,
        TRACE_ADMIN_KEY,
        INTERNAL_API_KEY,
      },
    }
  );
}

let cachedEnv: Env | undefined;
export function getEnv(rawEnv: Bindings): Env {
  if (cachedEnv === undefined) {
    cachedEnv = validateEnv(rawEnv);
  }

  return cachedEnv;
}
