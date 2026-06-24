import { createInfiniteQueries } from '@app/component/next-soup/soup-view/create-infinite-queries';
import { throwOnErr } from '@core/util/result';
import type { EntityData } from '@entity';
import { useQueryClient } from '@queries/client';
import {
  parseGroupMeta,
  serializeGroupByField,
} from '@queries/soup/grouped/api';
import type { GroupByField, GroupMeta } from '@queries/soup/grouped/types';
import type {
  SoupApiItemFilter,
  SoupAstBody,
  SoupAstItemsGroupedPage,
  SoupParams,
} from '@queries/soup/items';
import { soupKeys } from '@queries/soup/keys';
import {
  isDisplayableSoupItem,
  isInstructionsMdDoc,
  mapApiSoupItemToEntity,
} from '@queries/soup/transform-utils';
import { useInstructionsMdIdQuery } from '@queries/storage/instructions-md';
import { storageServiceClient } from '@service-storage/client';
import type { SoupApiItem } from '@service-storage/generated/schemas';
import type { InfiniteData } from '@tanstack/solid-query';
import {
  type Accessor,
  batch,
  createEffect,
  createMemo,
  createSignal,
  on,
  untrack,
} from 'solid-js';
import {
  buildRestoredGroupQueryData,
  type GroupedSoupQueriesSnapshot,
  type GroupQueryPage,
} from './grouped-query-snapshot';

type InitialGroupPage = {
  items: SoupAstItemsGroupedPage['items'];
  groups: GroupMeta[];
};

export type GroupQueryData = {
  entities: EntityData[];
};

type CreateGroupedSoupQueriesArgs = {
  initialPage: Accessor<InitialGroupPage | undefined>;
  groupByField: Accessor<GroupByField | undefined>;
  soupParams: Accessor<
    Omit<SoupParams, 'sort_method'> & {
      sort_method: Exclude<SoupParams['sort_method'], 'frecency'>;
    }
  >;
  soupBody: Accessor<SoupAstBody>;
  initialSnapshot?: Accessor<GroupedSoupQueriesSnapshot | undefined>;
  queryOptions: Accessor<{
    enabled?: boolean;
    meta?: {
      groupBy?: GroupByField;
      groupKey?: string;
      itemFilter?: SoupApiItemFilter;
    };
  }>;
};

export function createGroupedSoupQueries(args: CreateGroupedSoupQueriesArgs) {
  const instructionsIdQuery = useInstructionsMdIdQuery();
  const queryClient = useQueryClient();

  const mapItemToEntity = (item: SoupApiItem) => {
    if (!isDisplayableSoupItem(item)) return;
    if (isInstructionsMdDoc(item, instructionsIdQuery)) return;

    return mapApiSoupItemToEntity(item);
  };

  const mapPageToEntities = (page: GroupQueryPage): EntityData[] => {
    const entities: EntityData[] = [];

    for (const id of page.group.itemIds) {
      const item = page.items[id];
      if (!item) continue;

      const entity = mapItemToEntity(item);
      if (entity) entities.push(entity);
    }

    return entities;
  };

  const combineGroupPages = (
    pages: GroupQueryPage[]
  ): GroupQueryData | undefined => {
    if (pages.length === 0) return;

    return {
      entities: pages.flatMap(mapPageToEntities),
    };
  };

  const makeInitialPage = (
    initialGroupedPage: InitialGroupPage,
    group: GroupMeta
  ): GroupQueryPage => {
    const groupItems: InitialGroupPage['items'] = {};
    for (const id of group.itemIds) {
      const item = initialGroupedPage.items[id];
      if (item) groupItems[id] = item;
    }
    return { items: groupItems, group };
  };

  const groupBySnapshotKey = (field: GroupByField) =>
    JSON.stringify(serializeGroupByField(field));

  const snapshotScopeKey = (field: GroupByField) =>
    JSON.stringify(
      soupKeys.astItems({
        params: args.soupParams(),
        body: args.soupBody(),
        groupBy: field,
      }).queryKey
    );

  const snapshotKeys = () => {
    const field = args.groupByField();
    if (!field) return;
    return {
      groupBy: groupBySnapshotKey(field),
      scopeKey: snapshotScopeKey(field),
    };
  };

  const configs = createMemo(() => {
    const field = args.groupByField();
    const initialGroupedPage = args.initialPage();

    if (!field || !initialGroupedPage) {
      return [];
    }

    const options = args.queryOptions();

    return initialGroupedPage.groups.map((group) => {
      const initialPage = makeInitialPage(initialGroupedPage, group);

      return {
        key: group.key,
        queryKey: soupKeys.groupedGroup({
          params: args.soupParams(),
          body: args.soupBody(),
          groupBy: field,
          groupKey: group.key,
        }).queryKey,
        queryFn: async (ctx: { pageParam: string | null }) => {
          if (ctx.pageParam == null) {
            return initialPage;
          }

          const response = await throwOnErr(async () =>
            storageServiceClient.getGroupedSoupAstGroupPage({
              params: {
                cursor: ctx.pageParam ?? undefined,
                group_by: serializeGroupByField(field),
                group_key: group.key,
                limit: args.soupParams().limit,
                sort_method: args.soupParams().sort_method ?? undefined,
              },
              body: args.soupBody(),
            })
          );

          return {
            items: response.items,
            group: parseGroupMeta(response.group),
          };
        },
        getNextPageParam: (lastPage: GroupQueryPage): string | null => {
          return lastPage.group.nextCursor;
        },
        select: combineGroupPages,
        initialData: {
          pages: [initialPage],
          pageParams: [null],
        },
        enabled: false,
        meta: {
          ...options.meta,
          groupBy: field,
          groupKey: group.key,
          normalize: false,
        },
        staleTime: Infinity,
      };
    });
  });

  const queries = createInfiniteQueries<
    GroupQueryPage,
    GroupQueryData | undefined
  >(configs);

  const [groupDataVersion, setGroupDataVersion] = createSignal(0);

  const list = createMemo(() =>
    queries.list().map((query) => ({
      ...query,
      data: () => {
        groupDataVersion();
        return untrack(query.data);
      },
      fetchNextPage: async () => {
        const result = await query.fetchNextPage();
        setGroupDataVersion((version) => version + 1);
        return result;
      },
    }))
  );

  const map = createMemo(() => {
    const next = new Map<string, ReturnType<typeof list>[number]>();
    for (const query of list()) {
      next.set(query.key, query);
    }
    return next;
  });

  const getSnapshot = (): GroupedSoupQueriesSnapshot | undefined => {
    const keys = snapshotKeys();
    if (!keys) return undefined;

    const groups: GroupedSoupQueriesSnapshot['groups'] = {};

    for (const config of configs()) {
      const data = queryClient.getQueryData<
        InfiniteData<GroupQueryPage, string | null>
      >(config.queryKey);
      if (!data || data.pages.length <= 1) continue;
      groups[config.key] = data;
    }

    return Object.keys(groups).length > 0 ? { ...keys, groups } : undefined;
  };

  createEffect(
    on(
      () => args.initialPage(),
      (initialGroupedPage) => {
        const field = args.groupByField();
        if (!field || !initialGroupedPage) return;
        const groupBy = groupBySnapshotKey(field);
        const scopeKey = snapshotScopeKey(field);
        const snapshot = args.initialSnapshot?.();

        batch(() => {
          for (const group of initialGroupedPage.groups) {
            const initialPage = makeInitialPage(initialGroupedPage, group);
            const queryKey = soupKeys.groupedGroup({
              params: args.soupParams(),
              body: args.soupBody(),
              groupBy: field,
              groupKey: group.key,
            }).queryKey;

            queryClient.setQueryData<
              InfiniteData<GroupQueryPage, string | null>
            >(
              queryKey,
              buildRestoredGroupQueryData({
                initialPage,
                groupBy,
                groupKey: group.key,
                scopeKey,
                snapshot,
              })
            );
          }

          setGroupDataVersion((version) => version + 1);
        });
      },
      { defer: true }
    )
  );

  return {
    ...queries,
    list,
    map,
    getSnapshot,
  };
}
