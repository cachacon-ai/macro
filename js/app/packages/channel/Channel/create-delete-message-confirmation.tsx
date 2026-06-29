import { Button, Dialog, Panel } from '@ui';
import { createSignal, type JSX } from 'solid-js';
import type { DeleteMessageInput } from './create-channel-message-actions';

export type DeleteMessageConfirmation = {
  /** Opens the confirmation dialog for the given delete request. */
  requestDelete: (input: DeleteMessageInput) => void;
  /** Renders the confirmation dialog; mount once per channel surface. */
  ConfirmationDialog: () => JSX.Element;
};

/**
 * Wraps a `deleteMessage` mutation with a confirmation step. Deleting a
 * channel message is destructive, so every entry point (action menu, mobile
 * drawer, hotkeys) routes through `requestDelete`, which opens a dialog and
 * only fires the underlying delete once the user confirms.
 */
export function createDeleteMessageConfirmation(
  deleteMessage: (input: DeleteMessageInput) => void
): DeleteMessageConfirmation {
  const [pending, setPending] = createSignal<DeleteMessageInput | undefined>();

  const requestDelete = (input: DeleteMessageInput) => setPending(input);

  const close = () => setPending(undefined);

  const confirm = () => {
    const input = pending();
    if (input) deleteMessage(input);
    close();
  };

  const ConfirmationDialog = () => (
    <Dialog
      open={!!pending()}
      onOpenChange={(open) => {
        if (!open) close();
      }}
      position="center"
      class="w-120"
    >
      <Panel depth={2} class="rounded-xl">
        <Panel.Header class="px-6">
          <Dialog.Title class="text-ink text-sm font-semibold">
            Delete message
          </Dialog.Title>
        </Panel.Header>
        <Panel.Body class="p-6 font-sans flex flex-col gap-3">
          <Dialog.Description class="text-ink-muted text-sm/tight font-normal">
            This message will be permanently deleted. This action cannot be
            undone.
          </Dialog.Description>
          <div class="pt-3 justify-end items-center gap-3 inline-flex">
            <Button variant="base" depth={3} onClick={close}>
              Cancel
            </Button>
            <Button
              variant="danger"
              depth={3}
              ref={(el: HTMLButtonElement) => {
                requestAnimationFrame(() =>
                  requestAnimationFrame(() => el.focus())
                );
              }}
              onClick={confirm}
            >
              Delete
            </Button>
          </div>
        </Panel.Body>
      </Panel>
    </Dialog>
  );

  return { requestDelete, ConfirmationDialog };
}
