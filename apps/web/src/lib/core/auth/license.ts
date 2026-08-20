import { createMemo } from 'solid-js';

// FORK: this is a single-user fork — there's no paid concept, so every user
// is treated as having paid access. The license-status context still exists
// for other consumers that want to read the underlying status, but every UI
// gate that asks "does this user have paid access?" should see `true`.
//
// If you ever want to reintroduce a paywall here, swap the body back to the
// `trialing | active` check against `useLicenseStatus()`.
export function useHasPaidAccess() {
  return createMemo((): boolean => true);
}
