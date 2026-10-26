use super::*;

fn tab(id: &str) -> SessionTab {
    SessionTab {
        id: id.into(),
        title: id.into(),
        host_address: "localhost:22".into(),
        username: "test".into(),
        terminal: Terminal::new(80, 24, 100),
        handle: None,
        status: "test".into(),
        host_id: None,
        host_config: None,
    }
}

#[test]
fn raw_password_input_is_never_command_history() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    app.tabs.push(tab("one"));
    app.db
        .record_session_start("one", None, "localhost:22", "test")
        .unwrap();
    app.hosts_doc.settings.record_command_history = true;
    app.handle_key_input("sudo apt update\r");
    app.handle_key_input("my sudo password\r");
    assert!(app.db.list_recent_commands(10).unwrap().is_empty());
    app.event_tx
        .try_send(AppEvent::SshOutput {
            session_id: "one".into(),
            data: b"\x1b]633;E;sudo apt update\x07\x1b]633;C\x07".to_vec(),
        })
        .unwrap();
    app.poll_events();
    assert_eq!(
        app.db.list_recent_commands(10).unwrap()[0].command,
        "sudo apt update"
    );
    assert_eq!(app.command_search_results[0].command, "sudo apt update");
    app.hosts_doc.settings.record_command_history = false;
    app.event_tx
        .try_send(AppEvent::SshOutput {
            session_id: "one".into(),
            data: b"\x1b]633;E;private command\x07\x1b]633;C\x07".to_vec(),
        })
        .unwrap();
    app.poll_events();
    assert_eq!(app.db.list_recent_commands(10).unwrap().len(), 1);
}

#[test]
fn host_key_prompts_are_fifo_and_closed_tabs_are_rejected() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    app.tabs = vec![tab("one"), tab("two"), tab("three")];
    let mut replies = Vec::new();
    for id in ["one", "two", "three"] {
        let (response_tx, rx) = tokio::sync::oneshot::channel();
        replies.push(rx);
        app.event_tx
            .try_send(AppEvent::HostKeyPrompt {
                session_id: id.into(),
                host: "localhost".into(),
                port: 22,
                key_type: "ssh-ed25519".into(),
                fingerprint: id.into(),
                is_changed: false,
                old_line: None,
                response_tx,
            })
            .unwrap();
    }
    app.poll_events();
    assert_eq!(app.host_key_modal.as_ref().unwrap().session_id, "one");
    assert_eq!(app.pending_host_keys.len(), 2);
    app.accept_host_key();
    assert_eq!(replies[0].try_recv(), Ok(true));
    assert_eq!(app.host_key_modal.as_ref().unwrap().session_id, "two");
    app.active_tab = 1;
    app.close_active_tab();
    assert!(matches!(
        replies[1].try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    assert_eq!(app.host_key_modal.as_ref().unwrap().session_id, "three");
    app.reject_host_key();
    assert_eq!(replies[2].try_recv(), Ok(false));
}

#[test]
fn output_queue_is_bounded_and_one_poll_leaves_time_for_ui() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    for _ in 0..128 {
        app.event_tx
            .try_send(AppEvent::SshOutput {
                session_id: "gone".into(),
                data: vec![0; 16 * 1024],
            })
            .unwrap();
    }
    assert!(matches!(
        app.event_tx.try_send(AppEvent::SshOutput {
            session_id: "gone".into(),
            data: Vec::new()
        }),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_))
    ));
    app.poll_events();
    assert!(app.event_rx.len() >= 64);
}

#[test]
fn paste_preview_is_bound_to_original_session_and_adds_no_enter() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    let (handle, mut input) = SshSessionHandle::fixture(app.hosts_doc.hosts[0].clone());
    let mut original = tab("one");
    original.handle = Some(handle);
    original.terminal.write_bytes(b"\x1b[?2004h");
    app.tabs = vec![original, tab("two")];
    app.paste_to_active_terminal("echo one\necho two");
    assert!(input.try_recv().is_err());
    assert!(app.confirm_run_modal.as_ref().unwrap().paste);
    app.active_tab = 1;
    app.execute_confirm_run_command();
    assert_eq!(
        input.try_recv().unwrap(),
        b"\x1b[200~echo one\necho two\x1b[201~"
    );
    app.active_tab = 0;
    app.paste_to_active_terminal("echo one\n");
    app.close_active_tab();
    app.execute_confirm_run_command();
    assert!(input.try_recv().is_err());
}

#[test]
fn failed_save_does_not_claim_success_or_change_memory() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    app.open_settings_modal();
    app.settings_modal
        .as_mut()
        .unwrap()
        .default_user
        .set("changed-user");
    app.settings_modal
        .as_mut()
        .unwrap()
        .scrollback_lines
        .set("0");
    let original = app.hosts_doc.settings.default_user.clone();
    app.save_general_settings();
    assert_eq!(app.hosts_doc.settings.default_user, original);
    assert!(
        app.settings_modal
            .as_ref()
            .unwrap()
            .general_msg
            .as_ref()
            .unwrap()
            .1
    );
}

#[test]
fn importing_restores_settings_and_reports_only_new_hosts() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    let mut bundle = ExportBundle {
        version: 2,
        exported_at: "now".into(),
        hosts: app.hosts_doc.hosts.clone(),
        settings: app.hosts_doc.settings.clone(),
    };
    bundle.settings.default_user = "imported".into();
    bundle.settings.master_password_hash = Some("untrusted lock".into());
    assert_eq!(app.merge_backup(bundle).unwrap(), 0);
    assert_eq!(app.hosts_doc.settings.default_user, "imported");
    assert!(app.hosts_doc.settings.master_password_hash.is_none());
}

#[test]
fn new_sessions_respect_scrollback_and_locked_app_cannot_connect() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    app.is_app_locked = true;
    app.connect_to_host(app.hosts_doc.hosts[0].clone());
    assert!(app.tabs.is_empty());
    app.is_app_locked = false;
    app.hosts_doc.settings.scrollback_lines = 100;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut host = app.hosts_doc.hosts[0].clone();
    host.port = listener.local_addr().unwrap().port();
    app.connect_to_host(host);
    for _ in 0..300 {
        app.tabs[0].terminal.write_bytes(b"line\r\n");
    }
    assert_eq!(app.tabs[0].terminal.scrollback_len(), 100);
    app.shutdown_sessions();
}

#[test]
fn password_hashes_are_salted_and_legacy_unlock_is_upgraded() {
    let a = hash_password(" password ").unwrap();
    let b = hash_password(" password ").unwrap();
    assert_ne!(a, b);
    assert!(verify_password(" password ", &a));
    assert!(!verify_password("password", &a));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(b"evertyssh_salt_v1_oldpassword"));
    app.hosts_doc.settings.master_password_hash = Some(hash);
    app.is_app_locked = true;
    app.lock_password_input.set("oldpassword");
    assert!(app.try_unlock_app());
    assert!(app
        .hosts_doc
        .settings
        .master_password_hash
        .as_ref()
        .unwrap()
        .starts_with("$argon2id$"));
}

#[test]
fn render_settings_screens() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut app = AppState::new(rt.handle().clone()).unwrap();
    let mut ui = crate::ui::UiRenderer::new();
    app.open_settings_modal();
    for (name, section) in [
        ("backup", SettingsTab::Backup),
        ("general", SettingsTab::General),
    ] {
        app.settings_modal.as_mut().unwrap().active_tab = section;
        let pixels = ui
            .render_frame(
                &app,
                everty_ui::DpiScale::new(1.0).unwrap(),
                everty_ui::PhysicalSize::new(1120.0, 720.0),
            )
            .unwrap();
        assert_eq!(pixels.len(), 1120 * 720 * 4);
        if let Some(dir) = std::env::var_os("EVERTYSSH_TEST_RENDER_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(
                std::path::Path::new(&dir).join(format!("{name}.png")),
                pixels,
                1120,
                720,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
    }
}
