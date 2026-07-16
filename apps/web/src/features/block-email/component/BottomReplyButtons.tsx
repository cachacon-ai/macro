import { FloatRegionOrInline } from '@components/app/mobile/float-regions/FloatRegion';
import { inboxIconProps } from '@core/component/inboxIcon';
import { UserIcon } from '@core/component/UserIcon';
import { useEmail } from '@core/context/user';
import { TOKENS } from '@core/hotkey/tokens';
import { getActiveCommandByToken, runCommand } from '@core/hotkey/utils';
import { isMobile } from '@core/mobile/isMobile';
import ArrowBendUpLeft from '@phosphor/arrow-bend-up-left.svg';
import ArrowBendUpRight from '@phosphor/arrow-bend-up-right.svg';
import CheckIcon from '@phosphor/check.svg';
import { useEmailLinksQuery } from '@queries/email/link';
import type { ApiMessage } from '@service-email/generated/schemas';
import { createCallback } from '@solid-primitives/rootless';
import { Button, cn } from '@ui';
import { type Component, Show } from 'solid-js';
import type { ReplyType } from '../util/replyType';
import { useEmailContext } from './EmailContext';
import { getEmailFormRegistry } from './EmailFormContext';
import { openEmailReplyComposerForMessage } from './emailReplyActions';

function ReplyActionButton(props: {
  icon: Component<{ class?: string }>;
  label?: string;
  ariaLabel?: string;
  onClick: () => void;
}) {
  return (
    <Button
      // Button wraps itself in Layer depth={0} by default; in the floating
      // accessory region, match the chrome's depth so the island surface
      // matches the dock buttons. (The region host's Layer can't help —
      // Button's own Layer would reset it.)
      depth={isMobile() ? 3 : undefined}
      variant="base"
      aria-label={props.ariaLabel ?? props.label}
      class={cn(
        // Island pills when floating in the mobile accessory region.
        'mobile:island mobile:h-8 mobile:rounded-full mobile:border-0'
      )}
      onClick={props.onClick}
    >
      <props.icon class="size-4 shrink-0" />
      <Show when={props.label}>
        <span>{props.label}</span>
      </Show>
    </Button>
  );
}

export function BottomReplyButtons(props: { lastMessage: ApiMessage }) {
  const ctx = useEmailContext();
  const formRegistry = getEmailFormRegistry();
  const currentUserEmail = useEmail();
  const linksQuery = useEmailLinksQuery();

  const open = (type: ReplyType) =>
    createCallback(() => {
      const messageId = props.lastMessage.db_id;
      if (!messageId) return;
      openEmailReplyComposerForMessage({
        ctx,
        formRegistry,
        message: props.lastMessage,
        replyType: type,
        isLastMessage: true,
      });
    });

  const currentUserIconProps = () => {
    const email = currentUserEmail();
    return email ? inboxIconProps(email) : { email: '' };
  };

  // Only surface Mark done for threads the user owns, mirroring the TopBar
  // action's gating.
  const isOwnThread = () => {
    const thread = ctx.thread();
    const links = linksQuery.data?.links;
    if (!thread || !links) return false;
    return links.some((link) => link.id === thread.link_id);
  };

  const markDone = () => {
    // Prefer the active Mark done command so it drives soup navigation and
    // notifications; fall back to archiving the thread directly.
    const command = getActiveCommandByToken(TOKENS.entity.action.markDone);
    if (command) {
      runCommand(command);
    } else {
      ctx.archiveThread();
    }
  };

  if (!isMobile()) {
    return (
      <div class="flex w-full items-center gap-2 pt-4">
        <button
          type="button"
          class="flex min-w-0 flex-1 items-center gap-2 rounded-md text-left text-sm text-ink-placeholder hover:text-ink-muted"
          onClick={open('reply-all')}
        >
          <UserIcon
            {...currentUserIconProps()}
            size="md"
            showTooltip={false}
            suppressClick
          />
          <span class="truncate">Reply...</span>
        </button>
        <Show when={isOwnThread()}>
          <button
            type="button"
            aria-label="Mark done"
            title="Mark done"
            class="flex size-8 shrink-0 items-center justify-center rounded-md text-ink-placeholder hover:bg-ink/10 hover:text-ink-muted"
            onClick={markDone}
          >
            <CheckIcon class="size-5" />
          </button>
        </Show>
      </div>
    );
  }

  return (
    <FloatRegionOrInline region="accessory">
      <div class="w-full p-2 pb-2 pt-4 mobile:px-(--mobile-chrome-gutter) mobile:py-0">
        <div class="flex flex-row items-center gap-2 mobile:pointer-events-auto">
          <ReplyActionButton
            icon={ArrowBendUpLeft}
            label="Reply"
            onClick={open('reply-all')}
          />
          <ReplyActionButton
            icon={ArrowBendUpRight}
            label="Forward"
            onClick={open('forward')}
          />
          <Show when={isOwnThread()}>
            <div class="ml-auto">
              <ReplyActionButton
                icon={CheckIcon}
                ariaLabel="Mark done"
                onClick={markDone}
              />
            </div>
          </Show>
        </div>
      </div>
    </FloatRegionOrInline>
  );
}
