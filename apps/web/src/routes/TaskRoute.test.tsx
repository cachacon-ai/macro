/**
 * @vitest-environment jsdom
 */

import { err, ok } from 'neverthrow';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const DOCUMENT_ID = '019f148f-2a38-7f56-8944-ee118e48d27c';

const mocks = vi.hoisted(() => ({
  authenticated: true as boolean | undefined,
  getDocumentByTeamSlug: vi.fn(),
  navigate: vi.fn(),
  taskIdOrSlug: 'ENG-42',
}));

vi.mock('@components/app/split-layout/SplitLayoutRoute', () => ({
  SplitLayoutRouteContent: (props: { pairs: string[] }) => (
    <div>{props.pairs.join(':')}</div>
  ),
}));

vi.mock('@core/auth', () => ({
  useIsAuthenticated: () => () => mocks.authenticated,
}));

vi.mock('@core/component/LoadingBlock', () => ({
  LoadingBlock: () => <div>Loading task…</div>,
}));

vi.mock('@service-storage/client', () => ({
  storageServiceClient: {
    getDocumentByTeamSlug: mocks.getDocumentByTeamSlug,
  },
}));

vi.mock('@solidjs/router', () => ({
  useNavigate: () => mocks.navigate,
  useParams: () => ({
    get taskIdOrSlug() {
      return mocks.taskIdOrSlug;
    },
  }),
}));

vi.mock('@ui', () => ({
  Button: (props: { children: JSX.Element; onClick?: () => void }) => (
    <button onClick={props.onClick}>{props.children}</button>
  ),
}));

import { TaskRoute } from './TaskRoute';

let dispose: (() => void) | undefined;

function renderRoute(): HTMLElement {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const disposeRender = render(() => <TaskRoute />, container);
  dispose = () => {
    disposeRender();
    container.remove();
  };
  return container;
}

beforeEach(() => {
  mocks.authenticated = true;
  mocks.taskIdOrSlug = 'ENG-42';
  mocks.navigate.mockReset();
  mocks.getDocumentByTeamSlug.mockReset();
  mocks.getDocumentByTeamSlug.mockResolvedValue(
    ok({ documentMetadata: { documentId: DOCUMENT_ID } })
  );
  sessionStorage.clear();
  window.history.replaceState({}, '', '/app/task/ENG-42');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
});

describe('TaskRoute', () => {
  it('preserves the existing UUID task route without resolving a slug', () => {
    mocks.taskIdOrSlug = DOCUMENT_ID;

    const container = renderRoute();

    expect(container.textContent).toContain(`task:${DOCUMENT_ID}`);
    expect(mocks.getDocumentByTeamSlug).not.toHaveBeenCalled();
  });

  it('resolves a team task slug and replaces it with the canonical task route', async () => {
    renderRoute();

    await vi.waitFor(() => {
      expect(mocks.getDocumentByTeamSlug).toHaveBeenCalledWith({
        slug: 'ENG-42',
      });
      expect(mocks.navigate).toHaveBeenCalledWith(`/task/${DOCUMENT_ID}`, {
        replace: true,
      });
    });
  });

  it('preserves the slug route while sending a signed-out user to login', async () => {
    mocks.authenticated = false;

    renderRoute();

    await vi.waitFor(() => {
      expect(mocks.navigate).toHaveBeenCalledWith('/login', { replace: true });
    });
    expect(sessionStorage.getItem('redirectUrl')).toBe(window.location.href);
    expect(mocks.getDocumentByTeamSlug).not.toHaveBeenCalled();
  });

  it('shows a not-found state for an unknown task reference', async () => {
    mocks.getDocumentByTeamSlug.mockResolvedValue(
      err([{ code: 'NOT_FOUND', message: 'Resource not found' }])
    );

    const container = renderRoute();

    await vi.waitFor(() => {
      expect(container.textContent).toContain('Task not found');
    });
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
});
