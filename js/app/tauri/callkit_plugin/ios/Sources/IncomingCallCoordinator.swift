import AVFAudio
import CallKit
import Foundation
import PushKit

/// Owns the iOS system-call surface: PushKit registration, incoming VoIP push
/// handling, CallKit reporting, and CallKit answer/end actions.
///
/// All mutable state is main-queue only. CXProvider and PKPushRegistry are
/// configured with `queue: .main`, so their delegate callbacks arrive there.
final class IncomingCallCoordinator: NSObject, CXProviderDelegate, PKPushRegistryDelegate, @unchecked Sendable {
    private let mediaSession: NativeLiveKitCallSession
    private let onVoipTokenUpdated: (String) -> Void
    private let onCallAnswered: (String) -> Void
    private let onCallEnded: (String) -> Void

    private var provider: CXProvider!
    private let callController = CXCallController()
    private var registry: PKPushRegistry!

    // MARK: - State (main-queue only)

    // Keyed by call UUID. Holds the channelId so it is available when
    // CXAnswerCallAction fires.
    private var pendingCalls: [UUID: String] = [:]

    // Keyed by call UUID. Holds LiveKit credentials from the VoIP push payload
    // so a lock-screen answer can connect native audio without a webview trip.
    private var pendingCallTokens: [UUID: PendingCallToken] = [:]

    // The UUID of the most recently reported incoming call, used by
    // endActiveCall.
    private var activeCallUUID: UUID?

    // Last VoIP token received from PushKit; may arrive before JS is ready.
    private var cachedVoipToken: String?

    // channelId from the most recent answered call; may arrive before JS is
    // ready.
    private var pendingAnsweredChannelId: String?

    init(
        mediaSession: NativeLiveKitCallSession,
        onVoipTokenUpdated: @escaping (String) -> Void,
        onCallAnswered: @escaping (String) -> Void,
        onCallEnded: @escaping (String) -> Void
    ) {
        self.mediaSession = mediaSession
        self.onVoipTokenUpdated = onVoipTokenUpdated
        self.onCallAnswered = onCallAnswered
        self.onCallEnded = onCallEnded
    }

    func load() {
        let config = CXProviderConfiguration()
        config.supportsVideo = false
        config.maximumCallsPerCallGroup = 1
        config.supportedHandleTypes = [.generic]

        provider = CXProvider(configuration: config)
        provider.setDelegate(self, queue: .main)

        registry = PKPushRegistry(queue: .main)
        registry.delegate = self
        registry.desiredPushTypes = [.voIP]
    }

    func getVoipToken() -> String? {
        cachedVoipToken
    }

    func drainPendingAnsweredChannelId() -> String? {
        let channelId = pendingAnsweredChannelId
        pendingAnsweredChannelId = nil
        return channelId
    }

    func endActiveCall(completion: @escaping () -> Void) {
        guard let uuid = activeCallUUID else {
            completion()
            return
        }

        requestEndCall(uuid: uuid) { [weak self] error in
            guard let self else {
                completion()
                return
            }
            self.onMain {
                if error != nil {
                    self.clearCallState(uuid: uuid)
                    Task { [weak self] in
                        await self?.mediaSession.disconnect()
                    }
                }
                completion()
            }
        }
    }

    func requestEndCall(uuid: UUID) {
        requestEndCall(uuid: uuid) { [weak self] error in
            guard let self, error != nil else { return }
            self.onMain {
                self.clearCallState(uuid: uuid)
                Task { [weak self] in
                    await self?.mediaSession.disconnect()
                }
            }
        }
    }

    private func requestEndCall(uuid: UUID, completion: @escaping (Error?) -> Void) {
        let transaction = CXTransaction(action: CXEndCallAction(call: uuid))
        callController.request(transaction) { error in
            if let error {
                print("[CallKit] CXEndCallAction request failed: \(error)")
            }
            completion(error)
        }
    }

    // MARK: - PKPushRegistryDelegate

    func pushRegistry(
        _ registry: PKPushRegistry,
        didUpdate pushCredentials: PKPushCredentials,
        for type: PKPushType
    ) {
        guard type == .voIP else { return }
        let token = pushCredentials.token.map { String(format: "%02.2hhx", $0) }.joined()
        cachedVoipToken = token
        onVoipTokenUpdated(token)
    }

    func pushRegistry(
        _ registry: PKPushRegistry,
        didReceiveIncomingPushWith payload: PKPushPayload,
        for type: PKPushType,
        completion: @escaping () -> Void
    ) {
        guard type == .voIP else {
            completion()
            return
        }

        let dict = payload.dictionaryPayload
        let channelId = dict["channelId"] as? String ?? ""
        let callerName = dict["callerName"] as? String ?? "Incoming Call"
        let callIdString = dict["callId"] as? String ?? ""
        let livekitServerUrl = dict["livekitServerUrl"] as? String
        let livekitToken = dict["livekitToken"] as? String

        guard let uuid = UUID(uuidString: callIdString) else {
            // iOS terminates apps that skip reportNewIncomingCall inside this
            // delegate. Report a ghost call and immediately end it as failed so
            // the system requirement is satisfied while surfacing the server bug.
            print("[CallKit] Invalid callId '\(callIdString)' in VoIP payload: \(dict)")
            let fallbackUUID = UUID()
            provider.reportNewIncomingCall(with: fallbackUUID, update: CXCallUpdate()) { [weak self] _ in
                self?.provider.reportCall(with: fallbackUUID, endedAt: nil, reason: .failed)
                completion()
            }
            return
        }

        // Enforce the single-call invariant. Snapshot the keys before mutation
        // so we do not mutate the dictionary while iterating its live key view.
        for staleUUID in Array(pendingCalls.keys) where staleUUID != uuid {
            provider.reportCall(with: staleUUID, endedAt: nil, reason: .failed)
            pendingCalls.removeValue(forKey: staleUUID)
            pendingCallTokens.removeValue(forKey: staleUUID)
        }

        pendingCalls[uuid] = channelId
        if let serverUrl = livekitServerUrl, let token = livekitToken {
            pendingCallTokens[uuid] = PendingCallToken(serverUrl: serverUrl, token: token)
        } else {
            print("[CallKit] VoIP payload missing livekitServerUrl/livekitToken; lock-screen answer will not connect natively")
        }
        activeCallUUID = uuid

        let update = CXCallUpdate()
        update.remoteHandle = CXHandle(type: .generic, value: channelId)
        update.localizedCallerName = callerName
        update.hasVideo = false

        // iOS 13+: must call reportNewIncomingCall synchronously within this
        // delegate. If we do not, iOS will terminate the app.
        provider.reportNewIncomingCall(with: uuid, update: update) { [weak self] error in
            if error != nil {
                self?.pendingCalls.removeValue(forKey: uuid)
                self?.pendingCallTokens.removeValue(forKey: uuid)
                if self?.activeCallUUID == uuid { self?.activeCallUUID = nil }
            }
            completion()
        }
    }

    // MARK: - CXProviderDelegate

    func providerDidReset(_ provider: CXProvider) {
        pendingCalls.removeAll()
        pendingCallTokens.removeAll()
        activeCallUUID = nil
        pendingAnsweredChannelId = nil
        Task { [weak self] in
            await self?.mediaSession.disconnect()
        }
    }

    func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        guard let channelId = pendingCalls[action.callUUID] else {
            action.fail()
            return
        }

        mediaSession.configureAudioSessionCategory()

        pendingAnsweredChannelId = channelId
        onCallAnswered(channelId)

        let answeredUUID = action.callUUID
        if let pending = pendingCallTokens[answeredUUID] {
            mediaSession.connect(
                uuid: answeredUUID,
                channelId: channelId,
                serverUrl: pending.serverUrl,
                token: pending.token
            )
        } else {
            print("[CallKit] No cached LiveKit token for answered call \(answeredUUID.uuidString); JS-driven join required")
        }

        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        let callId = action.callUUID.uuidString
        onCallEnded(callId)

        Task { [weak self] in
            await self?.mediaSession.disconnect()
        }

        action.fulfill()
        clearCallState(uuid: action.callUUID)
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        mediaSession.activateAudioEngine()
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        mediaSession.deactivateAudioEngine()
    }

    // MARK: - Helpers

    private func clearCallState(uuid: UUID) {
        pendingCalls.removeValue(forKey: uuid)
        pendingCallTokens.removeValue(forKey: uuid)
        if activeCallUUID == uuid { activeCallUUID = nil }
        pendingAnsweredChannelId = nil
    }

    private func onMain(_ block: @escaping () -> Void) {
        if Thread.isMainThread {
            block()
        } else {
            DispatchQueue.main.async(execute: block)
        }
    }
}
