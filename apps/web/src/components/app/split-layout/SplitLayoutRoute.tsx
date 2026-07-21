import { setGlobalSplitManager } from '@app/signal/splitLayout';
import type { WithRequired } from '@core/util/withRequired';
import type { RouteDefinition, RouteSectionProps } from '@solidjs/router';
import { SplitLayoutContainer } from './SplitLayout';

type LayoutPath = {
  params: {
    splits: string | undefined;
  };
};

function LayoutRoute(props: RouteSectionProps & LayoutPath) {
  return (
    <SplitLayoutRouteContent pairs={props.params.splits?.split('/') ?? []} />
  );
}

/** Renders the split layout for an explicit set of URL segment pairs. */
export function SplitLayoutRouteContent(props: { pairs: string[] }) {
  return (
    <SplitLayoutContainer
      pairs={props.pairs}
      setManager={setGlobalSplitManager}
    />
  );
}

export const LAYOUT_ROUTE: WithRequired<RouteDefinition, 'component'> = {
  path: '/*splits',
  component: LayoutRoute,
};
