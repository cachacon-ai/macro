import { isTauri } from './platform';

let browserFetch: typeof fetch | undefined;

export function setPlatformBrowserFetch(fetchImpl: typeof fetch) {
  browserFetch = fetchImpl;
}

function getBrowserFetch(): typeof fetch {
  if (browserFetch) return browserFetch;
  if (typeof window !== 'undefined') return window.fetch.bind(window);
  if (globalThis.fetch) return globalThis.fetch.bind(globalThis);
  throw new Error('fetch is not available');
}

function inputUrl(input: RequestInfo | URL) {
  if (input instanceof Request) return input.url;
  if (input instanceof URL) return input.href;
  return input;
}

function isNetworkUrl(input: RequestInfo | URL) {
  try {
    const url = new URL(inputUrl(input), window.location.href);
    return url.protocol === 'http:' || url.protocol === 'https:';
  } catch {
    return false;
  }
}

async function acquireFetch(input: RequestInfo | URL) {
  // Local Tauri assets are served through the WebView protocol handler. The
  // Tauri HTTP plugin only supports network URLs and rejects tauri:// URLs,
  // including relative asset URLs resolved against the tauri:// app origin.
  if (isTauri() && isNetworkUrl(input)) {
    const { fetch } = await import('@tauri-apps/plugin-http');
    return fetch;
  }
  return getBrowserFetch();
}

// async function acquireWebsocket() {
//   if (isTauri()) {
//     const ws = await import("@tauri-apps/plugin-websocket");
//     return ws
//   }
//   return WebSocket
// }

// export const connectPlatformWebsocket: ReturnType<typeof WebSocket["conne"]

// Wrapper around fetch which forwards network requests through the Tauri HTTP
// client in Tauri environments. Local app assets stay on the WebView fetch.
export const platformFetch: typeof window.fetch = (url, opts) => {
  return acquireFetch(url).then((f) => f(url, opts));
};
