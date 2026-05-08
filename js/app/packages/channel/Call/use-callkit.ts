import { isPlatform, isTauri } from '@core/util/platform';
import { notificationServiceClient } from '@service-notification/client';
import type { DeviceType } from '@service-notification/generated/schemas/deviceType';
import { whenSplitManagerReady } from '@app/signal/splitLayout';
import { Channel, addPluginListener, invoke } from '@tauri-apps/api/core';
import { onCleanup, onMount } from 'solid-js';
import { joinChannelCall } from './join-channel-call';

// The 'iosvoip' variant exists in the backend but the generated schema has not been
// regenerated to include it yet. Cast until a regeneration picks it up.
const DEVICE_TYPE_IOS_VOIP = 'iosvoip' as DeviceType;

type VoipTokenPayload = { token: string };
type CallAnsweredPayload = { channelId: string };
type CallEndedPayload = { callId: string };
type CallEndedHandler = (payload: CallEndedPayload) => void | Promise<void>;

const callEndedHandlers: CallEndedHandler[] = [];

/**
 * Registers channel-scoped cleanup for native CallKit end events.
 *
 * `useCallKitSetup` owns the single global Tauri listener, while `useCall()`
 * registers the active channel's leave handler here. If multiple channel UI
 * surfaces are mounted, the most recently registered active handler wins; when
 * it unmounts, the previous active handler becomes available again.
 */
export function registerCallKitCallEndedHandler(
  handler: CallEndedHandler
): () => void {
  callEndedHandlers.push(handler);
  return () => {
    const index = callEndedHandlers.lastIndexOf(handler);
    if (index !== -1) callEndedHandlers.splice(index, 1);
  };
}

function getActiveCallEndedHandler(): CallEndedHandler | undefined {
  return callEndedHandlers[callEndedHandlers.length - 1];
}

// globalSplitManager is set deep in the router tree and may not be ready
// when a call-answered event arrives on fresh app launch.
async function joinChannelCallWhenReady(channelId: string): Promise<void> {
  await whenSplitManagerReady();
  return joinChannelCall(channelId);
}

/**
 * Sets up CallKit / PushKit integration for iOS.
 *
 * - Registers VoIP tokens with the backend as they arrive from PushKit.
 * - When the user answers via the native incoming-call sheet, navigates to the
 *   channel and joins the call via the existing deep-link flow.
 *
 * Must be mounted once at app startup on iOS (no-op on all other platforms).
 */
export function useCallKitSetup() {
  onMount(() => {
    if (!isTauri() || !isPlatform('ios')) return;

    let cleaned = false;
    onCleanup(() => { cleaned = true; });

    addPluginListener<VoipTokenPayload>(
      'call-kit',
      'voip-token-updated',
      async ({ token }) => {
        await notificationServiceClient
          .registerDevice({ token, deviceType: DEVICE_TYPE_IOS_VOIP })
          .catch((err) => console.error('[callkit] failed to register VoIP token', err));
      }
    ).then((l) => { if (cleaned) l.unregister().catch(() => {}); });

    // Drain any VoIP token that arrived from PushKit before the listener above
    // was registered (common on first launch).
    invoke<{ token: string | null }>('plugin:call-kit|get_voip_token')
      .then(({ token }) => {
        if (!token) return;
        return notificationServiceClient
          .registerDevice({ token, deviceType: DEVICE_TYPE_IOS_VOIP })
          .catch((err) =>
            console.error('[callkit] failed to register cached VoIP token', err)
          );
      })
      .catch((err) => console.error('[callkit] get_voip_token failed', err));

    const callAnsweredChannel = new Channel<CallAnsweredPayload>();
    callAnsweredChannel.onmessage = ({ channelId }) => {
      joinChannelCallWhenReady(channelId).catch(() => {});
    };
    invoke<void>('plugin:call-kit|watch_call_answered', {
      channel: callAnsweredChannel,
    })
      .then(() => {
        if (cleaned) return;
        // Initial drain for the fresh-launch case (answer arrived before command completed).
        return invoke<{ channelId: string | null }>(
          'plugin:call-kit|get_pending_answered_call'
        ).then(({ channelId }) => {
          if (channelId) joinChannelCallWhenReady(channelId).catch(() => {});
        });
      })
      .catch(() => {});

    const callEndedChannel = new Channel<CallEndedPayload>();
    callEndedChannel.onmessage = (payload) => {
      const handler = getActiveCallEndedHandler();
      if (!handler) return;
      Promise.resolve(handler(payload)).catch((err) =>
        console.error('[callkit] failed to handle ended call', err)
      );
    };
    invoke<void>('plugin:call-kit|watch_call_ended', { channel: callEndedChannel }).catch(() => {});
  });
}

/**
 * Tells CallKit to end the active call session.
 *
 * Call this from `leaveCall()` so the native system call UI is dismissed when
 * the user leaves from within the app rather than from the CallKit sheet.
 */
export async function endCallKitCall(): Promise<void> {
  if (!isTauri() || !isPlatform('ios')) return;
  await invoke('plugin:call-kit|end_active_call').catch((err) =>
    console.error('[callkit] failed to end active call', err)
  );
}
