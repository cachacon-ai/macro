import { type Accessor, onCleanup } from 'solid-js';
import {
  getMessageElement,
  scrollElementAboveObstruction,
} from '../scroll-utils';
import type { ThreadListNavigation } from './ThreadList';

/**
 * How many frames to wait for the sent reply's row to render before giving
 * up — covers the optimistic-insert flush, plus a virtua mount when the
 * thread had to be scrolled back into view first.
 */
const MAX_REVEAL_FRAMES = 24;

type CreateSentReplyRevealOptions = {
  navigation: Accessor<ThreadListNavigation | undefined>;
  /**
   * Height of whatever covers the bottom of the screen — the floating input
   * chrome, plus the virtual keyboard while it is up. Read at scroll time,
   * since the keyboard is often mid-dismissal right after a send.
   */
  obstructionHeight: Accessor<number>;
};

/**
 * Scrolls a just-sent thread reply into view (unified-input mode). The reply
 * renders optimistically at the end of its thread, which the viewport does
 * not follow: it can sit below the fold, behind the floating input and
 * keyboard, or in a thread that was scrolled away from (and virtualized out
 * of the DOM) while composing. The reveal waits for the reply's row to
 * render — jumping to the thread first when its row isn't mounted — then
 * scrolls the reply into view, clear of the bottom chrome. Already-visible
 * replies are left alone.
 */
export function createSentReplyReveal(options: CreateSentReplyRevealOptions) {
  let frame: number | undefined;

  const cancel = () => {
    if (frame === undefined) return;
    cancelAnimationFrame(frame);
    frame = undefined;
  };

  onCleanup(cancel);

  return (threadId: string, messageId: string) => {
    // A newer send owns the reveal.
    cancel();

    let didScrollToThread = false;
    let framesLeft = MAX_REVEAL_FRAMES;

    const tick = () => {
      frame = undefined;

      const element = getMessageElement(messageId);
      if (element) {
        element.scrollIntoView({ block: 'nearest' });
        scrollElementAboveObstruction(element, options.obstructionHeight());
        return;
      }

      // The reply isn't rendered yet. If its whole thread is out of the DOM,
      // jump there so the reply's row can mount; when the thread is mounted
      // the reply is at most an optimistic-insert flush away, so just wait.
      if (!didScrollToThread && !getMessageElement(threadId)) {
        const navigation = options.navigation();
        if (navigation) {
          didScrollToThread = true;
          navigation.scrollToId(threadId, { align: 'end' });
        }
      }

      framesLeft -= 1;
      if (framesLeft > 0) frame = requestAnimationFrame(tick);
    };

    frame = requestAnimationFrame(tick);
  };
}
