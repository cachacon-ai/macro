import type { GroupMeta } from '@queries/soup/grouped/types';
import type { SoupApiItem } from '@service-storage/generated/schemas';
import type { InfiniteData } from '@tanstack/solid-query';

export type GroupQueryPage = {
  items: Record<string, SoupApiItem>;
  group: GroupMeta;
};

export type GroupedSoupQueriesSnapshot = {
  groupBy: string;
  scopeKey: string;
  groups: Record<string, InfiniteData<GroupQueryPage, string | null>>;
};

export function buildRestoredGroupQueryData(input: {
  initialPage: GroupQueryPage;
  groupBy: string;
  groupKey: string;
  scopeKey: string;
  snapshot?: GroupedSoupQueriesSnapshot;
}): InfiniteData<GroupQueryPage, string | null> {
  const savedData =
    input.snapshot?.groupBy === input.groupBy &&
    input.snapshot.scopeKey === input.scopeKey
      ? input.snapshot.groups[input.groupKey]
      : undefined;

  if (savedData && savedData.pages.length > 1) {
    return {
      pages: [input.initialPage, ...savedData.pages.slice(1)],
      pageParams: [null, ...savedData.pageParams.slice(1)],
    };
  }

  return {
    pages: [input.initialPage],
    pageParams: [null],
  };
}
