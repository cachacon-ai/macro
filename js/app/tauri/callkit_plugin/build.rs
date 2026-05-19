fn main() {
    tauri_plugin::Builder::new(&["get_voip_token", "end_active_call", "get_pending_answered_call", "watch_call_answered", "watch_call_ended", "get_active_call_state"])
        .ios_path("ios")
        .try_build()
        .unwrap();
}
