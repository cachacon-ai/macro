import type { SplitHandle } from '@components/app/split-layout/layoutManager';
import { createRoot, createSignal } from 'solid-js';
import { describe, expect, it, vi } from 'vitest';
import type { SoupRow } from '../create-soup-state';
import {
  hasPreviewableSoupRows,
  useSoupPreviewAvailability,
} from './use-soup-preview-availability';

const row = (
  options: { grouped?: boolean; loadMore?: boolean } = {}
): SoupRow =>
  ({
    getIsGrouped: () => options.grouped ?? false,
    getIsLoadMore: () => options.loadMore ?? false,
  }) as SoupRow;

const flushEffects = () => Promise.resolve();

/**
 * A SplitHandle stub whose controller role follows engage/disengage calls,
 * mirroring how the layout manager reports a Preview Pair.
 */
const splitHandleStub = (options: { controller: boolean; room?: boolean }) => {
  let controller = options.controller;
  const engagePreview = vi.fn(() => {
    controller = true;
  });
  const disengagePreview = vi.fn(() => {
    controller = false;
  });
  return {
    engagePreview,
    disengagePreview,
    exitPreviewAsUser: () => {
      controller = false;
    },
    handle: {
      isControllerSplit: () => controller,
      isViewerSplit: () => false,
      canEngagePreview: () => options.room ?? true,
      engagePreview,
      disengagePreview,
    } as unknown as SplitHandle,
  };
};

describe('Soup preview availability', () => {
  it('requires an entity row rather than a group or load-more row', () => {
    expect(
      hasPreviewableSoupRows([row({ grouped: true }), row({ loadMore: true })])
    ).toBe(false);
    expect(hasPreviewableSoupRows([row({ grouped: true }), row()])).toBe(true);
  });

  it('disengages preview when the last entity row disappears', async () => {
    const stub = splitHandleStub({ controller: true });
    let setRows!: (rows: SoupRow[]) => void;
    let dispose!: () => void;

    createRoot((rootDispose) => {
      dispose = rootDispose;
      const [rows, updateRows] = createSignal([row()]);
      setRows = updateRows;
      useSoupPreviewAvailability({
        rows,
        isLoading: () => false,
        splitHandle: stub.handle,
      });
    });

    await flushEffects();
    expect(stub.disengagePreview).not.toHaveBeenCalled();

    setRows([]);
    await flushEffects();
    expect(stub.disengagePreview).toHaveBeenCalledOnce();
    dispose();
  });

  it('keeps preview open while an empty result is still loading', async () => {
    const stub = splitHandleStub({ controller: true });
    let setLoading!: (loading: boolean) => void;
    let dispose!: () => void;

    createRoot((rootDispose) => {
      dispose = rootDispose;
      const [isLoading, updateLoading] = createSignal(true);
      setLoading = updateLoading;
      useSoupPreviewAvailability({
        rows: () => [],
        isLoading,
        splitHandle: stub.handle,
      });
    });

    await flushEffects();
    expect(stub.disengagePreview).not.toHaveBeenCalled();

    setLoading(false);
    await flushEffects();
    expect(stub.disengagePreview).toHaveBeenCalledOnce();
    dispose();
  });

  it('re-engages preview when entities return after an empty state', async () => {
    const stub = splitHandleStub({ controller: true });
    const onPreviewRestored = vi.fn();
    let setRows!: (rows: SoupRow[]) => void;
    let dispose!: () => void;

    createRoot((rootDispose) => {
      dispose = rootDispose;
      const [rows, updateRows] = createSignal([row()]);
      setRows = updateRows;
      useSoupPreviewAvailability({
        rows,
        isLoading: () => false,
        splitHandle: stub.handle,
        onPreviewRestored,
      });
    });

    await flushEffects();
    setRows([]);
    await flushEffects();
    expect(stub.disengagePreview).toHaveBeenCalledOnce();
    expect(stub.engagePreview).not.toHaveBeenCalled();

    setRows([row()]);
    await flushEffects();
    expect(stub.engagePreview).toHaveBeenCalledOnce();
    expect(onPreviewRestored).toHaveBeenCalledOnce();
    dispose();
  });

  it('does not re-engage after the user exits preview mode', async () => {
    const stub = splitHandleStub({ controller: true });
    let setRows!: (rows: SoupRow[]) => void;
    let dispose!: () => void;

    createRoot((rootDispose) => {
      dispose = rootDispose;
      const [rows, updateRows] = createSignal([row()]);
      setRows = updateRows;
      useSoupPreviewAvailability({
        rows,
        isLoading: () => false,
        splitHandle: stub.handle,
      });
    });

    await flushEffects();
    stub.exitPreviewAsUser();

    setRows([]);
    await flushEffects();
    expect(stub.disengagePreview).not.toHaveBeenCalled();

    setRows([row()]);
    await flushEffects();
    expect(stub.engagePreview).not.toHaveBeenCalled();
    dispose();
  });

  it('does not engage when preview was never on', async () => {
    const stub = splitHandleStub({ controller: false });
    let setRows!: (rows: SoupRow[]) => void;
    let dispose!: () => void;

    createRoot((rootDispose) => {
      dispose = rootDispose;
      const [rows, updateRows] = createSignal<SoupRow[]>([]);
      setRows = updateRows;
      useSoupPreviewAvailability({
        rows,
        isLoading: () => false,
        splitHandle: stub.handle,
      });
    });

    await flushEffects();
    setRows([row()]);
    await flushEffects();
    expect(stub.engagePreview).not.toHaveBeenCalled();
    dispose();
  });

  it('consumes the suspension when there is no room to restore', async () => {
    const stub = splitHandleStub({ controller: true, room: false });
    const onPreviewRestored = vi.fn();
    let setRows!: (rows: SoupRow[]) => void;
    let dispose!: () => void;

    createRoot((rootDispose) => {
      dispose = rootDispose;
      const [rows, updateRows] = createSignal([row()]);
      setRows = updateRows;
      useSoupPreviewAvailability({
        rows,
        isLoading: () => false,
        splitHandle: stub.handle,
        onPreviewRestored,
      });
    });

    await flushEffects();
    setRows([]);
    await flushEffects();
    expect(stub.disengagePreview).toHaveBeenCalledOnce();

    setRows([row()]);
    await flushEffects();
    expect(stub.engagePreview).not.toHaveBeenCalled();
    expect(onPreviewRestored).not.toHaveBeenCalled();

    // The consumed suspension cannot re-engage on a later cycle either.
    setRows([]);
    await flushEffects();
    setRows([row()]);
    await flushEffects();
    expect(stub.engagePreview).not.toHaveBeenCalled();
    dispose();
  });
});
