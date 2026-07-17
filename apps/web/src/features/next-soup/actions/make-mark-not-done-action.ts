import {
  applyEntitiesNotDoneOptimistic,
  executeMarkEntitiesUndone,
  resolveMarkEntitiesDoneVariables,
} from '@app/features/next-soup/utils';
import { toast } from '@core/component/Toast/Toast';
import type { EntityData } from '@entity';
import type { NotificationSource } from '@notifications';
import { invalidateAllSoup } from '@queries/soup/cache';
import type { SoupState } from '../create-soup-state';

type MakeMarkNotDoneOptions = {
  notificationSource: () => NotificationSource;
};

/**
 * Reverses a mark-done: unarchives email threads and restores their
 * notifications. Only done emails qualify — other entity types' done state
 * lives on their notifications, and their rows never render as done in the
 * mark-done-capable views.
 */
export const makeMarkNotDoneAction = (options: MakeMarkNotDoneOptions) => {
  const canExecute = (entity: EntityData): boolean =>
    entity.type === 'email' && entity.done === true;

  const execute = async (entities: EntityData[]) => {
    const targets = entities.filter(canExecute);
    if (targets.length === 0) return;

    const { emailIds, notificationIds } = resolveMarkEntitiesDoneVariables({
      entities: targets,
      notificationSource: options.notificationSource(),
    });

    const optimistic = applyEntitiesNotDoneOptimistic({
      emailIds,
      notificationIds,
    });

    try {
      await executeMarkEntitiesUndone({ emailIds, notificationIds });
      // Done-filtered views (inbox, mail Important/Noise) pick the restored
      // entities back up on refetch.
      invalidateAllSoup();
    } catch {
      optimistic.rollback();
      toast.failure('Failed to mark as not done');
    }
  };

  /** Signature parity with makeMarkDoneAction's executeWithSoup — no
   *  navigation or collapse: the rows stay in place. */
  const executeWithSoup = async (entities: EntityData[], _soup: SoupState) => {
    await execute(entities);
  };

  return { canExecute, execute, executeWithSoup };
};
