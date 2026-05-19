import AVFAudio
import Foundation
import LiveKit

/// Owns the native LiveKit Room and its audio-session integration.
///
/// All mutable state is main-queue only. RoomDelegate callbacks arrive from a
/// LiveKit-internal queue and hop back to main before touching state.
final class NativeLiveKitCallSession: NSObject, RoomDelegate, @unchecked Sendable {
    private let onSnapshotChanged: (ActiveCallSnapshot?) -> Void
    private let requestSystemEndCall: (UUID) -> Void

    // MARK: - State (main-queue only)

    private var room: Room?
    private var connectTask: Task<Void, Never>?
    private var activeCallUUID: UUID?
    private var activeCall: ActiveCallSnapshot?

    init(
        onSnapshotChanged: @escaping (ActiveCallSnapshot?) -> Void,
        requestSystemEndCall: @escaping (UUID) -> Void
    ) {
        self.onSnapshotChanged = onSnapshotChanged
        self.requestSystemEndCall = requestSystemEndCall
    }

    func prepareForCallKitAudio() {
        // CallKit owns AVAudioSession activation/deactivation. LiveKit should
        // configure tracks, but it must not activate the session independently.
        AudioManager.shared.audioSession.isAutomaticConfigurationEnabled = false
        try? AudioManager.shared.setEngineAvailability(.none)
    }

    func configureAudioSessionCategory() {
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(
                .playAndRecord,
                mode: .voiceChat,
                options: [.allowBluetoothHFP, .allowBluetoothA2DP, .duckOthers]
            )
        } catch {
            print("[CallKit] Failed to set audio session category: \(error)")
        }
    }

    func activateAudioEngine() {
        try? AudioManager.shared.setEngineAvailability(.default)
    }

    func deactivateAudioEngine() {
        try? AudioManager.shared.setEngineAvailability(.none)
    }

    func currentSnapshot() -> ActiveCallSnapshot? {
        activeCall
    }

    func connect(uuid: UUID, channelId: String, serverUrl: String, token: String) {
        // Must be called on main.
        let newRoom = Room(delegate: self)

        activeCallUUID = uuid
        activeCall = ActiveCallSnapshot(
            channelId: channelId,
            callId: uuid.uuidString,
            connectionState: "connecting",
            isAudioMuted: false
        )
        emitSnapshot()

        // Replace any prior Room atomically on the main queue. If a previous
        // disconnect task is still queued, do not overwrite the reference and
        // leave that Room without an owner.
        connectTask?.cancel()
        let oldRoom = room
        if let oldRoom {
            Task { await oldRoom.disconnect() }
        }
        room = newRoom

        connectTask = Task { [weak self, weak newRoom] in
            guard let newRoom else { return }
            do {
                try await newRoom.connect(url: serverUrl, token: token)
                try await newRoom.localParticipant.setMicrophone(enabled: true)
            } catch is CancellationError {
                return
            } catch {
                print("[CallKit] Failed to connect LiveKit room: \(error)")
                DispatchQueue.main.async { [weak self, weak newRoom] in
                    guard let self, self.activeCallUUID == uuid, self.room === newRoom else { return }
                    self.requestSystemEndCall(uuid)
                }
            }
        }
    }

    func disconnect() async {
        let toDisconnect: Room? = await MainActor.run {
            self.connectTask?.cancel()
            self.connectTask = nil
            let r = self.room
            self.room = nil
            self.activeCallUUID = nil
            self.activeCall = nil
            self.emitSnapshot()
            return r
        }

        if let toDisconnect {
            await toDisconnect.disconnect()
        }
    }

    // MARK: - RoomDelegate

    func room(
        _ room: Room,
        didUpdateConnectionState connectionState: ConnectionState,
        from oldConnectionState: ConnectionState
    ) {
        let stateString = describe(connectionState)
        DispatchQueue.main.async { [weak self, weak room] in
            guard let self, let room, self.room === room else { return }

            if connectionState == .disconnected {
                self.activeCallUUID = nil
                self.activeCall = nil
                self.emitSnapshot()
                return
            }

            if var snapshot = self.activeCall {
                snapshot.connectionState = stateString
                self.activeCall = snapshot
                self.emitSnapshot()
            }
        }
    }

    private func describe(_ state: ConnectionState) -> String {
        switch state {
        case .disconnected: return "disconnected"
        case .connecting: return "connecting"
        case .reconnecting: return "reconnecting"
        case .connected: return "connected"
        case .disconnecting: return "disconnecting"
        @unknown default: return "disconnected"
        }
    }

    private func emitSnapshot() {
        // Must be called on main.
        onSnapshotChanged(activeCall)
    }
}
