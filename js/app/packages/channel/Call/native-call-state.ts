import { createSignal } from 'solid-js';

export type NativeCallConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'disconnecting';

// Phase 1: only mirror the call identity + connection state from native into
// JS. Mute / device / participant state are *not* tracked here.
// TODO(call-phase-2): introduce dedicated watch_audio_state /
// watch_participants channels.
// Adding fields here that the Swift side does not actively maintain causes
// the CallContext effect to clobber JS-driven state on every snapshot tick.
export type NativeCallSnapshot = {
  channelId: string;
  callId: string;
  connectionState: NativeCallConnectionState;
};

// Module-level signal for the most recent native (Swift-side) call state.
// `useCallKitSetup` writes here from the connection-state plugin events, and
// `CallContext` reads from here to mirror the native call into JS-visible state
// without minting a duplicate LiveKit token. Non-iOS platforms never write to
// this signal so it stays null.
export const [nativeCallSnapshot, setNativeCallSnapshot] =
  createSignal<NativeCallSnapshot | null>(null);
