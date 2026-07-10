import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSentReplyReveal } from '../create-sent-reply-reveal';
import type { ThreadListNavigation } from '../ThreadList';

const THREAD_ID = 'thread-1';
const REPLY_ID = 'reply-1';

function addMessageElement(
  messageId: string,
  parent: HTMLElement = document.body
) {
  const element = document.createElement('div');
  element.setAttribute('data-message-id', messageId);
  element.scrollIntoView = vi.fn();
  parent.appendChild(element);
  return element;
}

function makeNavigation(): ThreadListNavigation {
  return {
    scrollToId: vi.fn(() => true),
  } as unknown as ThreadListNavigation;
}

function makeReveal(options?: {
  navigation?: ThreadListNavigation;
  obstructionHeight?: () => number;
}) {
  let reveal!: ReturnType<typeof createSentReplyReveal>;
  const dispose = createRoot((dispose) => {
    reveal = createSentReplyReveal({
      navigation: () => options?.navigation,
      obstructionHeight: options?.obstructionHeight ?? (() => 0),
    });
    return dispose;
  });
  return { reveal, dispose };
}

describe('createSentReplyReveal', () => {
  beforeEach(() => {
    vi.useFakeTimers({
      toFake: ['requestAnimationFrame', 'cancelAnimationFrame'],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = '';
  });

  it('scrolls the reply into view once its row renders', () => {
    const navigation = makeNavigation();
    const { reveal, dispose } = makeReveal({ navigation });
    addMessageElement(THREAD_ID);

    reveal(THREAD_ID, REPLY_ID);
    // The optimistic reply has not rendered yet; the thread is mounted, so
    // the reveal must wait rather than jump the viewport to the thread.
    vi.advanceTimersToNextFrame();
    expect(navigation.scrollToId).not.toHaveBeenCalled();

    const reply = addMessageElement(REPLY_ID);
    vi.advanceTimersToNextFrame();
    expect(reply.scrollIntoView).toHaveBeenCalledWith({ block: 'nearest' });

    // The reveal is one-shot: no re-scroll on later frames.
    vi.advanceTimersToNextFrame();
    expect(reply.scrollIntoView).toHaveBeenCalledTimes(1);

    dispose();
  });

  it('jumps to the thread when its row is not in the DOM', () => {
    const navigation = makeNavigation();
    const { reveal, dispose } = makeReveal({ navigation });

    reveal(THREAD_ID, REPLY_ID);
    vi.advanceTimersToNextFrame();
    expect(navigation.scrollToId).toHaveBeenCalledExactlyOnceWith(THREAD_ID, {
      align: 'end',
    });

    // Only one jump, even while the row is still mounting.
    vi.advanceTimersToNextFrame();
    expect(navigation.scrollToId).toHaveBeenCalledTimes(1);

    const reply = addMessageElement(REPLY_ID);
    vi.advanceTimersToNextFrame();
    expect(reply.scrollIntoView).toHaveBeenCalledWith({ block: 'nearest' });

    dispose();
  });

  it('scrolls the revealed reply above the bottom obstruction', () => {
    const container = document.createElement('div');
    container.setAttribute('data-channel-scroll', '');
    document.body.appendChild(container);
    const reply = addMessageElement(REPLY_ID, container);

    const bottom = window.innerHeight;
    vi.spyOn(container, 'getBoundingClientRect').mockReturnValue({
      bottom,
    } as DOMRect);
    // The reply sits 8px above the screen bottom — behind a 100px obstruction.
    vi.spyOn(reply, 'getBoundingClientRect').mockReturnValue({
      bottom: bottom - 8,
    } as DOMRect);

    const { reveal, dispose } = makeReveal({ obstructionHeight: () => 100 });
    reveal(THREAD_ID, REPLY_ID);
    vi.advanceTimersToNextFrame();

    // elBottom (bottom - 8) - visibleBottom (bottom - 100) + 8px offset.
    expect(container.scrollTop).toBe(100);

    dispose();
  });

  it('gives up quietly when the reply never renders', () => {
    const navigation = makeNavigation();
    const { reveal, dispose } = makeReveal({ navigation });

    reveal(THREAD_ID, REPLY_ID);
    for (let i = 0; i < 30; i++) vi.advanceTimersToNextFrame();

    // The frame budget is exhausted: a row rendering later is left alone.
    const reply = addMessageElement(REPLY_ID);
    vi.advanceTimersToNextFrame();
    expect(reply.scrollIntoView).not.toHaveBeenCalled();

    dispose();
  });

  it('lets a newer send take over the reveal', () => {
    const { reveal, dispose } = makeReveal({ navigation: makeNavigation() });

    reveal(THREAD_ID, 'reply-a');
    reveal(THREAD_ID, 'reply-b');

    const replyA = addMessageElement('reply-a');
    const replyB = addMessageElement('reply-b');
    vi.advanceTimersToNextFrame();
    vi.advanceTimersToNextFrame();

    expect(replyA.scrollIntoView).not.toHaveBeenCalled();
    expect(replyB.scrollIntoView).toHaveBeenCalledTimes(1);

    dispose();
  });

  it('cancels a pending reveal on dispose', () => {
    const { reveal, dispose } = makeReveal({ navigation: makeNavigation() });
    const reply = addMessageElement(REPLY_ID);

    reveal(THREAD_ID, REPLY_ID);
    dispose();
    vi.advanceTimersToNextFrame();

    expect(reply.scrollIntoView).not.toHaveBeenCalled();
  });
});
