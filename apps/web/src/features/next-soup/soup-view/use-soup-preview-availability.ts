import type { SoupRow } from '@app/features/next-soup/create-soup-state';
import type { SplitHandle } from '@components/app/split-layout/layoutManager';
import { type Accessor, createEffect, createMemo, untrack } from 'solid-js';

export function hasPreviewableSoupRows(rows: readonly SoupRow[]): boolean {
  return rows.some((row) => !row.getIsGrouped() && !row.getIsLoadMore());
}

/**
 * Tracks whether a Soup view has an entity that can be previewed. A settled
 * result with no such entities suspends the view's Preview Pair rather than
 * ending it: preview mode disengages while the view is empty and re-engages
 * as soon as an entity returns. Only exits that happen outside this hook —
 * the user toggling preview off, closing the Viewer, layout constraints —
 * are permanent.
 */
export function useSoupPreviewAvailability(options: {
  rows: Accessor<readonly SoupRow[]>;
  isLoading: Accessor<boolean>;
  splitHandle: SplitHandle;
  /** Called after an empty-state suspension ends with preview re-engaged. */
  onPreviewRestored?: () => void;
}): Accessor<boolean> {
  const hasPreviewItems = createMemo(() =>
    hasPreviewableSoupRows(options.rows())
  );

  // Set only when this hook disengages preview itself, so an empty state is
  // a suspension while every other exit stays permanent.
  let suspended = false;

  createEffect(() => {
    if (options.isLoading()) return;

    if (!hasPreviewItems()) {
      if (options.splitHandle.isControllerSplit()) {
        suspended = true;
        options.splitHandle.disengagePreview();
      }
      return;
    }

    if (!suspended) return;
    // A failed restore (e.g. no room for the Viewer) consumes the
    // suspension instead of re-engaging on an unrelated later layout change.
    suspended = false;
    untrack(() => {
      if (
        options.splitHandle.isViewerSplit() ||
        options.splitHandle.isControllerSplit() ||
        !options.splitHandle.canEngagePreview()
      ) {
        return;
      }
      options.splitHandle.engagePreview();
      if (options.splitHandle.isControllerSplit()) {
        options.onPreviewRestored?.();
      }
    });
  });

  return hasPreviewItems;
}
