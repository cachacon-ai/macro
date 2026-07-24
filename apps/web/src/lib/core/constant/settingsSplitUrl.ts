import { DEFAULT_ROUTE } from '@app/constants/defaultRoute';
import {
  PREVIEW_QUERY_PARAM,
  remapPreviewQueryForRemovedSplit,
} from '@components/app/split-layout/previewPersistence';

/**
 * Split a base-relative URL into pathname, search (`?…` or empty), and hash
 * (`#…` or empty). The base only anchors parsing; inputs come from the
 * router's `location`, which is already URL-canonical.
 */
const splitUrlParts = (url: string) => {
  const { pathname, search, hash } = new URL(url, 'http://localhost');
  return { pathname, search, hash };
};

/**
 * Drop a settings split from a base-relative split-layout URL, if present.
 * Handles both the URL encoding (`settings/<tab>`) and the legacy internal
 * form (`component/settings`). Only type positions (even indices) are
 * inspected so a block id that happens to be "settings" isn't mistaken for
 * one.
 *
 * The query string and hash are preserved. Removing a split shifts the URL
 * pair indices after it, so Preview Pairs declared in the `preview` query
 * param are remapped to keep pointing at the same splits (a pair that
 * referenced the settings split itself is dropped). When settings was the
 * only split there is no layout left to return to, so the default route is
 * returned bare.
 */
export const stripSettingsSplitFromUrl = (url: string): string => {
  const { pathname, search, hash } = splitUrlParts(url);
  const segments = pathname.split('/').filter(Boolean);

  let removedSplitIndex: number | undefined;
  for (let i = 0; i + 1 < segments.length; i += 2) {
    const type = segments[i];
    if (
      type === 'settings' ||
      (type === 'component' && segments[i + 1] === 'settings')
    ) {
      removedSplitIndex = i / 2;
      segments.splice(i, 2);
      break;
    }
  }

  if (segments.length === 0) return DEFAULT_ROUTE;

  let nextSearch = search;
  if (removedSplitIndex !== undefined) {
    const params = new URLSearchParams(search);
    const remappedPreview = remapPreviewQueryForRemovedSplit(
      params.get(PREVIEW_QUERY_PARAM) ?? undefined,
      removedSplitIndex
    );
    if (remappedPreview === undefined) {
      params.delete(PREVIEW_QUERY_PARAM);
    } else {
      params.set(PREVIEW_QUERY_PARAM, remappedPreview);
    }
    const serialized = params.toString();
    nextSearch = serialized ? `?${serialized}` : '';
  }

  return `/${segments.join('/')}${nextSearch}${hash}`;
};

/**
 * Append a docked settings split (`settings/<slug>`) to a base-relative
 * split-layout URL, keeping its query string and hash. Appending at the end
 * leaves existing URL pair indices untouched, so Preview Pairs in the
 * `preview` query param stay valid as-is.
 */
export const appendSettingsSplitToUrl = (
  url: string,
  settingsTabSlug: string
): string => {
  const { pathname, search, hash } = splitUrlParts(url);
  const base = pathname.replace(/\/$/, '');
  return `${base}/settings/${settingsTabSlug}${search}${hash}`;
};
