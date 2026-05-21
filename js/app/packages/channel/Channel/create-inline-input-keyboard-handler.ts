import { isNativeMobilePlatform } from '@core/mobile/isNativeMobilePlatform';
import { isTouchDevice } from '@core/mobile/isTouchDevice';
import {
  virtualKeyboardHeight,
  virtualKeyboardVisible,
} from '@core/mobile/virtualKeyboard';
import { isPlatform } from '@core/util/platform';
import {
  type Accessor,
  createEffect,
  on,
  onCleanup,
  type Setter,
} from 'solid-js';
import { scrollElementAboveKeyboard } from '../scroll-utils';

const INPUT_CONTAINER_SELECTOR = '[data-inline-input-container-id]';

export function createInlineInputKeyboardHandler(
  containerEl: Accessor<HTMLElement | undefined>,
  setIsChannelInputHidden: Setter<boolean>
) {
  let activeInputContainer: HTMLElement | undefined;
  let removalObserver: MutationObserver | undefined;

  const stopWatchingForRemoval = () => {
    removalObserver?.disconnect();
    removalObserver = undefined;
  };

  const reset = () => {
    setIsChannelInputHidden(false);
    activeInputContainer = undefined;
    stopWatchingForRemoval();
  };

  // The active input container can be unmounted (e.g. after a reply is sent
  // and the thread reply UI closes) without firing a focusout that bubbles to
  // our listener, and without the virtual keyboard changing visibility. Watch
  // the message list for DOM mutations so we can reset in that case.
  const watchForContainerRemoval = () => {
    stopWatchingForRemoval();
    const root = containerEl();
    if (!root || !activeInputContainer) return;
    removalObserver = new MutationObserver(() => {
      if (activeInputContainer && !activeInputContainer.isConnected) {
        reset();
      }
    });
    removalObserver.observe(root, { childList: true, subtree: true });
  };

  const keyboardWillShowHandler = (event: Event) => {
    const height =
      (event as CustomEvent<{ height: number }>).detail?.height ?? 0;
    if (activeInputContainer) {
      scrollElementAboveKeyboard(activeInputContainer, height);
    }
  };

  const handleFocusIn = (e: FocusEvent) => {
    const inputContainer = (e.target as HTMLElement).closest<HTMLElement>(
      INPUT_CONTAINER_SELECTOR
    );
    if (!inputContainer) return;
    activeInputContainer = inputContainer;
    watchForContainerRemoval();

    // HACK: on mobile safari, we need to ensure that the input container is scrolled into view BEFORE we hide the input, and then perform the subsequent scroll. Some sort of weird Safari focus behavior going on.
    if (!isPlatform('ios')) {
      activeInputContainer.scrollIntoView({ block: 'end' });
    }

    setIsChannelInputHidden(true);

    if (isPlatform('ios')) {
      const currentKeyboardHeight = virtualKeyboardHeight();

      if (currentKeyboardHeight > 0) {
        scrollElementAboveKeyboard(activeInputContainer, currentKeyboardHeight);
      } else {
        window.addEventListener('keyboardWillShow', keyboardWillShowHandler, {
          once: true,
        });
      }
    } else {
      // HACK: on mobile safari need to jettison this scroll out past the layout changes caused by the virtual keyboard appearing
      setTimeout(() => {
        if (!activeInputContainer) return;
        activeInputContainer.scrollIntoView({ block: 'end' });
      }, 500);
    }
  };

  const handleFocusOut = (e: FocusEvent) => {
    if (!activeInputContainer) return;
    const nextInputContainer = (e.relatedTarget as HTMLElement | null)?.closest(
      INPUT_CONTAINER_SELECTOR
    );
    if (!nextInputContainer) {
      reset();
    }
  };

  // Attach focus in handler
  createEffect(
    on(containerEl, () => {
      if (!isTouchDevice()) return;
      const el = containerEl();
      if (!el) return;
      el.addEventListener('focusin', handleFocusIn);
      el.addEventListener('focusout', handleFocusOut);

      onCleanup(() => {
        el.removeEventListener('focusin', handleFocusIn);
        el.removeEventListener('focusout', handleFocusOut);
        stopWatchingForRemoval();
      });
    })
  );

  createEffect(
    on(virtualKeyboardVisible, () => {
      if (!isTouchDevice()) return;
      if (!virtualKeyboardVisible()) {
        reset();
        return;
      }
      // Mobile web only: scroll active input into view when keyboard appears.
      if (isNativeMobilePlatform()) return;
      setTimeout(() => {
        if (!activeInputContainer) return;
        activeInputContainer.scrollIntoView({
          block: 'center',
          behavior: 'smooth',
        });
      }, 0);
    }),
    { defer: true }
  );
}
