import { SplitLayoutRouteContent } from '@components/app/split-layout/SplitLayoutRoute';
import { useIsAuthenticated } from '@core/auth';
import { LoadingBlock } from '@core/component/LoadingBlock';
import { storageServiceClient } from '@service-storage/client';
import { useNavigate, useParams } from '@solidjs/router';
import { Button } from '@ui';
import {
  createEffect,
  createResource,
  Match,
  on,
  Show,
  Switch,
} from 'solid-js';
import { validate as isUuid } from 'uuid';

type TaskRouteParams = {
  taskIdOrSlug: string;
};

/** Resolves team task slugs while preserving the existing UUID task route. */
export function TaskRoute() {
  const params = useParams<TaskRouteParams>();
  const navigate = useNavigate();
  const isAuthenticated = useIsAuthenticated();
  const taskReference = () => params.taskIdOrSlug;
  const isDocumentId = () => isUuid(taskReference());
  const slugToResolve = () =>
    isAuthenticated() === true && !isDocumentId() ? taskReference() : undefined;

  const [task, { refetch }] = createResource(slugToResolve, async (slug) => {
    const result = await storageServiceClient.getDocumentByTeamSlug({ slug });
    if (result.isErr()) {
      return { ok: false as const, errors: result.error };
    }
    return { ok: true as const, data: result.value };
  });
  const documentId = () => {
    const result = task();
    return result?.ok ? result.data.documentMetadata.documentId : undefined;
  };
  const resolutionFailed = () => task()?.ok === false || task.error;
  const taskNotFound = () => {
    const result = task();
    return (
      result?.ok === false &&
      result.errors.some((error) => error.code === 'NOT_FOUND')
    );
  };

  // A GitHub autolink may be opened before the user has signed in. Preserve
  // the full URL so BasePathComponent can restore it after authentication.
  createEffect(
    on(isAuthenticated, (authenticated) => {
      if (authenticated !== false || isDocumentId()) return;
      sessionStorage.setItem('redirectUrl', window.location.href);
      navigate('/login', { replace: true });
    })
  );

  createEffect(
    on(documentId, (id) => {
      if (id) navigate(`/task/${id}`, { replace: true });
    })
  );

  return (
    <Switch fallback={<LoadingBlock />}>
      <Match when={isDocumentId()}>
        <SplitLayoutRouteContent pairs={['task', taskReference()]} />
      </Match>
      <Match when={isAuthenticated() !== true}>
        <LoadingBlock />
      </Match>
      <Match when={task.loading}>
        <LoadingBlock />
      </Match>
      <Match when={resolutionFailed()}>
        <TaskRouteError
          notFound={taskNotFound()}
          onRetry={() => void refetch()}
        />
      </Match>
    </Switch>
  );
}

function TaskRouteError(props: { notFound: boolean; onRetry: () => void }) {
  const navigate = useNavigate();

  return (
    <div class="size-full flex items-center justify-center p-8">
      <div class="flex max-w-sm flex-col items-center gap-4 text-center">
        <h1 class="text-lg font-medium text-ink">
          {props.notFound ? 'Task not found' : 'Unable to open task'}
        </h1>
        <p class="text-sm text-ink-muted">
          {props.notFound
            ? 'This task reference does not exist or is not available to your team.'
            : 'Something went wrong while resolving this task reference.'}
        </p>
        <div class="flex items-center gap-2">
          <Button variant="base" size="sm" onClick={() => navigate('/tasks')}>
            Go to tasks
          </Button>
          <Show when={!props.notFound}>
            <Button variant="base" size="sm" onClick={props.onRetry}>
              Try again
            </Button>
          </Show>
        </div>
      </div>
    </div>
  );
}
