import type { GroupMeta } from '@queries/soup/grouped/types';
import type { SoupApiItem } from '@service-storage/generated/schemas';
import type { InfiniteData } from '@tanstack/solid-query';
import { describe, expect, it } from 'vitest';
import {
  buildRestoredGroupQueryData,
  type GroupedSoupQueriesSnapshot,
  type GroupQueryPage,
} from './grouped-query-snapshot';

const makeItem = (id: string) => ({ id }) as unknown as SoupApiItem;

const makeGroup = (
  key: string,
  itemIds: string[],
  nextCursor: string | null = null
): GroupMeta => ({
  key,
  label: key,
  displayOrder: null,
  totalCount: itemIds.length,
  itemIds,
  nextCursor,
});

const makePage = (
  groupKey: string,
  itemIds: string[],
  nextCursor: string | null = null
): GroupQueryPage => ({
  items: Object.fromEntries(itemIds.map((id) => [id, makeItem(id)])),
  group: makeGroup(groupKey, itemIds, nextCursor),
});

const makeData = (
  pages: GroupQueryPage[],
  pageParams: (string | null)[]
): InfiniteData<GroupQueryPage, string | null> => ({
  pages,
  pageParams,
});

describe('buildRestoredGroupQueryData', () => {
  it('uses only the fresh initial page without a snapshot', () => {
    const initialPage = makePage('status:open', ['fresh-1'], 'next');

    expect(
      buildRestoredGroupQueryData({
        initialPage,
        groupBy: 'date',
        groupKey: 'status:open',
        scopeKey: 'scope-a',
      })
    ).toEqual(makeData([initialPage], [null]));
  });

  it('ignores snapshots from a different groupBy', () => {
    const initialPage = makePage('status:open', ['fresh-1'], 'next');
    const oldSecondPage = makePage('status:open', ['old-2']);

    const snapshot: GroupedSoupQueriesSnapshot = {
      groupBy: 'entity_type',
      scopeKey: 'scope-a',
      groups: {
        'status:open': makeData([initialPage, oldSecondPage], [null, 'next']),
      },
    };

    expect(
      buildRestoredGroupQueryData({
        initialPage,
        groupBy: 'date',
        groupKey: 'status:open',
        scopeKey: 'scope-a',
        snapshot,
      })
    ).toEqual(makeData([initialPage], [null]));
  });

  it('ignores snapshots from a different query scope', () => {
    const initialPage = makePage('status:open', ['fresh-1'], 'next');
    const oldSecondPage = makePage('status:open', ['old-2']);

    const snapshot: GroupedSoupQueriesSnapshot = {
      groupBy: 'date',
      scopeKey: 'scope-a',
      groups: {
        'status:open': makeData([initialPage, oldSecondPage], [null, 'next']),
      },
    };

    expect(
      buildRestoredGroupQueryData({
        initialPage,
        groupBy: 'date',
        groupKey: 'status:open',
        scopeKey: 'scope-b',
        snapshot,
      })
    ).toEqual(makeData([initialPage], [null]));
  });

  it('keeps the fresh initial page and restores previously loaded extra pages', () => {
    const freshInitialPage = makePage('status:open', ['fresh-1'], 'next');
    const staleInitialPage = makePage('status:open', ['stale-1'], 'next');
    const oldSecondPage = makePage('status:open', ['old-2'], 'next-2');
    const oldThirdPage = makePage('status:open', ['old-3']);

    const snapshot: GroupedSoupQueriesSnapshot = {
      groupBy: 'date',
      scopeKey: 'scope-a',
      groups: {
        'status:open': makeData(
          [staleInitialPage, oldSecondPage, oldThirdPage],
          [null, 'next', 'next-2']
        ),
      },
    };

    expect(
      buildRestoredGroupQueryData({
        initialPage: freshInitialPage,
        groupBy: 'date',
        groupKey: 'status:open',
        scopeKey: 'scope-a',
        snapshot,
      })
    ).toEqual(
      makeData(
        [freshInitialPage, oldSecondPage, oldThirdPage],
        [null, 'next', 'next-2']
      )
    );
  });

  it('does not restore snapshots that have only the initial page', () => {
    const freshInitialPage = makePage('status:open', ['fresh-1'], 'next');
    const staleInitialPage = makePage('status:open', ['stale-1'], 'next');

    const snapshot: GroupedSoupQueriesSnapshot = {
      groupBy: 'date',
      scopeKey: 'scope-a',
      groups: {
        'status:open': makeData([staleInitialPage], [null]),
      },
    };

    expect(
      buildRestoredGroupQueryData({
        initialPage: freshInitialPage,
        groupBy: 'date',
        groupKey: 'status:open',
        scopeKey: 'scope-a',
        snapshot,
      })
    ).toEqual(makeData([freshInitialPage], [null]));
  });
});
