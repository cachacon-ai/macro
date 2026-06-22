import { queryReadyGate } from '@queries/gate';
import { useCurrentTeamQuery } from '@queries/team/teams';
import { type Accessor, createMemo } from 'solid-js';
import { createAssertedContextProvider } from './createContext';

export const MACRO_TEAM_SLUG = 'MACRO';

type TeamContextValue = {
  isMacroTeam: Accessor<boolean>;
};

export const [TeamContextProvider, useTeamContext] =
  createAssertedContextProvider('TeamContext', (): TeamContextValue => {
    const currentTeam = useCurrentTeamQuery();
    const isMacroTeam = createMemo(() => {
      if (!queryReadyGate(currentTeam)) return false;
      return currentTeam.data.team?.slug === MACRO_TEAM_SLUG;
    });

    return {
      isMacroTeam,
    };
  });

export function useIsMacroTeam() {
  return useTeamContext().isMacroTeam;
}
