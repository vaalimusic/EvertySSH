use async_trait::async_trait;
use russh::client::{self, Handler};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;

use crate::app::AppEvent;
use crate::config::{AuthConfig, HostConfig};

pub fn get_known_hosts_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        home.join(".ssh").join("known_hosts")
    } else {
        PathBuf::from("known_hosts")
    }
}

pub fn remove_known_host_line(path: &Path, line_to_remove: usize) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(path)?;
    let mut new_lines = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if idx + 1 != line_to_remove {
            new_lines.push(line);
        }
    }
    let mut output = new_lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    std::fs::write(path, output)?;
    Ok(())
}

fn format_ssh_error(addr: &str, raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("serverkeycheckfailed") || lower.contains("check_server_key") {
        format!("Соединение прервано: ключ хоста {addr} не подтвержден пользователем")
    } else if raw.contains("10061") || lower.contains("connection refused") {
        format!("В соединении отказано ({addr}): порт закрыт или служба SSH не запущена")
    } else if raw.contains("10060") || lower.contains("timed out") || lower.contains("timeout") {
        format!("Превышено время ожидания ответа от {addr}: сервер недоступен или заблокирован брандмауэром")
    } else if raw.contains("10065") || lower.contains("no route") {
        format!("Хост {addr} недостижим: нет сетевого маршрута к хосту")
    } else if lower.contains("lookup") || lower.contains("dns") || lower.contains("getaddrinfo") {
        format!("Не удалось разрешить имя хоста {addr}: проверьте правильность доменного адреса")
    } else {
        format!("Не удалось подключиться к {addr}: {raw}")
    }
}

pub struct SshClientHandler {
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub event_tx: Sender<AppEvent>,
    pub waiting_for_key: tokio::sync::watch::Sender<bool>,
    pub known_hosts_path: PathBuf,
}

#[async_trait]
impl Handler for SshClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let path = self.known_hosts_path.clone();
        let snapshot = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let old_line = match russh_keys::check_known_hosts_path(
            &self.host,
            self.port,
            server_public_key,
            &path,
        ) {
            Ok(true) => return Ok(true),
            Ok(false) => None,
            Err(russh_keys::Error::KeyChanged { line }) => Some(line),
            Err(e) => {
                return Err(std::io::Error::other(format!("Cannot verify known_hosts: {e}")).into())
            }
        };
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let _pause = PromptWait::new(self.waiting_for_key.clone());
        if self
            .event_tx
            .send(AppEvent::HostKeyPrompt {
                session_id: self.session_id.clone(),
                host: self.host.clone(),
                port: self.port,
                key_type: server_public_key.name().to_string(),
                fingerprint: format!("SHA256:{}", server_public_key.fingerprint()),
                is_changed: old_line.is_some(),
                old_line,
                response_tx,
            })
            .await
            .is_err()
        {
            return Ok(false);
        }
        if response_rx.await != Ok(true) {
            return Ok(false);
        }
        persist_known_host(
            &path,
            &self.host,
            self.port,
            server_public_key,
            &snapshot,
            old_line,
        )
        .await
        .map_err(|e| russh::Error::from(std::io::Error::other(e)))?;
        Ok(true)
    }
}

struct PromptWait(tokio::sync::watch::Sender<bool>);
impl PromptWait {
    fn new(sender: tokio::sync::watch::Sender<bool>) -> Self {
        sender.send_replace(true);
        Self(sender)
    }
}
impl Drop for PromptWait {
    fn drop(&mut self) {
        self.0.send_replace(false);
    }
}

/// The timeout counts network work, excluding queued and visible user prompts.
async fn network_timeout<F: std::future::Future>(
    future: F,
    mut paused: tokio::sync::watch::Receiver<bool>,
    budget: Duration,
) -> Result<F::Output, String> {
    tokio::pin!(future);
    let mut remaining = budget;
    loop {
        let is_paused = *paused.borrow_and_update();
        let started = tokio::time::Instant::now();
        tokio::select! {
            biased;
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep(remaining), if !is_paused => return Err("SSH network connection timed out".into()),
            changed = paused.changed() => {
                if changed.is_err() { return Err("SSH connection cancelled".into()); }
            }
        }
        if !is_paused {
            remaining = remaining.saturating_sub(started.elapsed());
        }
    }
}

async fn persist_known_host(
    path: &Path,
    host: &str,
    port: u16,
    key: &russh_keys::key::PublicKey,
    snapshot: &[u8],
    old_line: Option<usize>,
) -> Result<(), String> {
    static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = WRITE_LOCK.lock().await;
    let current = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let parent = path.parent().ok_or("Invalid known_hosts path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    std::fs::write(temp.path(), &current).map_err(|e| e.to_string())?;
    let remove_line = match russh_keys::check_known_hosts_path(host, port, key, temp.path()) {
        Ok(true) => return Ok(()),
        Ok(false) if old_line.is_none() => None,
        Err(russh_keys::Error::KeyChanged { line }) if old_line.is_some() => {
            let original = std::str::from_utf8(snapshot)
                .ok()
                .and_then(|text| text.lines().nth(old_line.unwrap() - 1));
            let latest = std::str::from_utf8(&current)
                .ok()
                .and_then(|text| text.lines().nth(line - 1));
            if original.is_none() || original != latest {
                return Err(
                    "Host entry changed during confirmation. Reconnect and verify again.".into(),
                );
            }
            Some(line)
        }
        _ => {
            return Err(
                "Host entry changed during confirmation. Reconnect and verify again.".into(),
            )
        }
    };
    if let Some(line) = remove_line {
        remove_known_host_line(temp.path(), line).map_err(|e| e.to_string())?;
    }
    russh_keys::learn_known_hosts_path(host, port, key, temp.path()).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(temp.path()).map_err(|e| e.to_string())?;
    let latest = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    if latest != current {
        return Err("known_hosts changed while saving. Reconnect and verify again.".into());
    }
    crate::config_store::atomic_write(path, &bytes)
}

pub struct SshSessionHandle {
    pub session_id: String,
    pub host: HostConfig,
    pub input_tx: mpsc::Sender<Vec<u8>>,
    pub resize_tx: mpsc::Sender<(u32, u32)>,
    pub is_connected: Arc<parking_lot::RwLock<bool>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for SshSessionHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl SshSessionHandle {
    #[cfg(test)]
    pub(crate) fn fixture(host: HostConfig) -> (Self, mpsc::Receiver<Vec<u8>>) {
        let (input_tx, input_rx) = mpsc::channel(128);
        let (resize_tx, _resize_rx) = mpsc::channel(16);
        (
            Self {
                session_id: host.id.clone(),
                host,
                input_tx,
                resize_tx,
                is_connected: Arc::new(parking_lot::RwLock::new(true)),
                shutdown_tx: None,
            },
            input_rx,
        )
    }

    pub async fn send_data(&self, data: Vec<u8>) -> Result<(), String> {
        self.input_tx
            .send(data)
            .await
            .map_err(|e| format!("Failed to send data to SSH channel: {e}"))
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), String> {
        self.resize_tx
            .send((cols, rows))
            .await
            .map_err(|e| format!("Failed to send resize to SSH channel: {e}"))
    }
}

pub async fn connect_ssh(
    host: HostConfig,
    session_id: String,
    initial_cols: u32,
    initial_rows: u32,
    event_tx: Sender<AppEvent>,
) -> Result<SshSessionHandle, String> {
    connect_ssh_with_path(
        host,
        session_id,
        initial_cols,
        initial_rows,
        event_tx,
        get_known_hosts_path(),
    )
    .await
}

async fn connect_ssh_with_path(
    host: HostConfig,
    session_id: String,
    initial_cols: u32,
    initial_rows: u32,
    event_tx: Sender<AppEvent>,
    known_hosts_path: PathBuf,
) -> Result<SshSessionHandle, String> {
    let config = Arc::new(client::Config::default());
    let addr = format!("{}:{}", host.hostname, host.port);

    // 1. TCP Connection + Server Key Verification with 30s timeout
    let (waiting_for_key, paused) = tokio::sync::watch::channel(false);
    let handler = SshClientHandler {
        session_id: session_id.clone(),
        host: host.hostname.clone(),
        port: host.port,
        event_tx: event_tx.clone(),
        waiting_for_key,
        known_hosts_path,
    };
    let connect_fut = client::connect(config, (host.hostname.as_str(), host.port), handler);
    let mut session = match network_timeout(connect_fut, paused, Duration::from_secs(30)).await {
        Ok(res) => res.map_err(|e| format_ssh_error(&addr, &e.to_string()))?,
        Err(_) => {
            return Err(format!(
                "Таймаут подключения к {addr} (превышено 30 сек). Сервер недоступен или блокирует порт {}.",
                host.port
            ))
        }
    };

    // 2. Authentication with 7s timeout
    let auth_fut = async {
        match &host.auth {
            AuthConfig::Password { password } => {
                if let Some(pwd) = password {
                    session
                        .authenticate_password(&host.username, pwd)
                        .await
                        .map_err(|e| format!("Ошибка проверки пароля: {e}"))
                } else {
                    Err("Требуется пароль, но он не указан в настройках сервера".into())
                }
            }
            AuthConfig::Key { path, passphrase } => {
                let path = path.clone();
                let passphrase = passphrase.clone();
                let key = tokio::task::spawn_blocking(move || {
                    russh_keys::load_secret_key(&path, passphrase.as_deref())
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("Cannot read private key: {e}"))?;

                session
                    .authenticate_publickey(&host.username, Arc::new(key))
                    .await
                    .map_err(|e| format!("Ошибка проверки SSH ключа: {e}"))
            }
            AuthConfig::Agent => {
                let mut agent = russh_keys::agent::client::AgentClient::connect_env()
                    .await
                    .map_err(|e| format!("Не удалось подключиться к SSH Agent: {e}"))?;

                let identities = agent
                    .request_identities()
                    .await
                    .map_err(|e| format!("Ошибка чтения ключей из SSH Agent: {e}"))?;

                let mut authed = false;
                for pubkey in identities {
                    let (ret_agent, res) = session
                        .authenticate_future(&host.username, pubkey, agent)
                        .await;
                    agent = ret_agent;
                    if let Ok(true) = res {
                        authed = true;
                        break;
                    }
                }
                Ok(authed)
            }
        }
    };

    let auth_success = match tokio::time::timeout(Duration::from_secs(7), auth_fut).await {
        Ok(res) => res?,
        Err(_) => {
            return Err(format!(
                "Таймаут аутентификации пользователя '{}' на сервере {addr} (превышено 7 сек)",
                host.username
            ))
        }
    };

    if !auth_success {
        return Err(format!(
            "Аутентификация отклонена сервером: неверный логин/пароль или ключ не принят (пользователь '{}' на {})",
            host.username, addr
        ));
    }

    // 3. Open session channel + PTY + Shell with 5s timeout
    let channel_fut = async {
        let channel = session
            .channel_open_session()
            .await
            .map_err(|e| format!("Не удалось открыть SSH канал сессии: {e}"))?;

        channel
            .request_pty(
                false,
                "xterm-256color",
                initial_cols,
                initial_rows,
                0,
                0,
                &[],
            )
            .await
            .map_err(|e| format!("Не удалось запросить PTY: {e}"))?;

        channel
            .request_shell(true)
            .await
            .map_err(|e| format!("Не удалось запустить shell оболочку: {e}"))?;

        Ok::<_, String>(channel)
    };

    let mut channel = match tokio::time::timeout(Duration::from_secs(5), channel_fut).await {
        Ok(res) => res?,
        Err(_) => {
            return Err(format!(
                "Таймаут запуска оболочки на сервере {addr} (превышено 5 сек)"
            ))
        }
    };

    let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(128);
    let (resize_tx, mut resize_rx) = mpsc::channel::<(u32, u32)>(16);
    let is_connected = Arc::new(parking_lot::RwLock::new(true));
    let is_connected_clone = is_connected.clone();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let runner_id = session_id.clone();
    // Keep the authenticated connection alive until its channel is closed.
    tokio::spawn(async move {
        let _session = session;
        let mut error = None;
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                input = input_rx.recv() => {
                    let Some(data) = input else { break; };
                    if let Err(e) = channel.data(&data[..]).await { error = Some(e.to_string()); break; }
                }
                Some((cols, rows)) = resize_rx.recv() => {
                    if let Err(e) = channel.window_change(cols, rows, 0, 0).await { error = Some(e.to_string()); break; }
                }
                msg = channel.wait() => {
                    match msg {
                        Some(russh::ChannelMsg::Data { ref data }) | Some(russh::ChannelMsg::ExtendedData { ref data, .. }) => {
                            for chunk in data.chunks(16 * 1024) {
                                let event = AppEvent::SshOutput { session_id: runner_id.clone(), data: chunk.to_vec() };
                                tokio::select! {
                                    _ = &mut shutdown_rx => { *is_connected_clone.write() = false; let _ = channel.close().await; return; }
                                    result = event_tx.send(event) => if result.is_err() { *is_connected_clone.write() = false; let _ = channel.close().await; return; }
                                }
                            }
                        }
                        Some(russh::ChannelMsg::Close) | None => break,
                        _ => {}
                    }
                }
            }
        }
        *is_connected_clone.write() = false;
        let _ = channel.close().await;
        let _ = event_tx
            .send(AppEvent::SshDisconnected {
                session_id: runner_id,
                error,
            })
            .await;
    });

    Ok(SshSessionHandle {
        session_id,
        host,
        input_tx,
        resize_tx,
        is_connected,
        shutdown_tx: Some(shutdown_tx),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoServer;

    #[async_trait]
    impl russh::server::Handler for EchoServer {
        type Error = russh::Error;

        async fn auth_password(
            &mut self,
            user: &str,
            password: &str,
        ) -> Result<russh::server::Auth, Self::Error> {
            assert_eq!((user, password), ("test", "synthetic password"));
            Ok(russh::server::Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            _: russh::Channel<russh::server::Msg>,
            _: &mut russh::server::Session,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }

        async fn shell_request(
            &mut self,
            channel: russh::ChannelId,
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel);
            Ok(())
        }

        async fn data(
            &mut self,
            channel: russh::ChannelId,
            data: &[u8],
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            session.data(channel, russh::CryptoVec::from_slice(data));
            Ok(())
        }
    }

    #[tokio::test]
    async fn local_ssh_connection_authenticates_streams_bounded_chunks_and_closes() {
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Arc::new(russh::server::Config {
            keys: vec![russh_keys::key::KeyPair::generate_ed25519().unwrap()],
            ..Default::default()
        });
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let session = russh::server::run_stream(config, stream, EchoServer)
                .await
                .unwrap();
            let _ = session.await;
        });
        let mut host = crate::config::sample_hosts_document().hosts.remove(0);
        host.hostname = "127.0.0.1".into();
        host.port = port;
        host.username = "test".into();
        host.auth = AuthConfig::Password {
            password: Some("synthetic password".into()),
        };
        let (events, mut receiver) = mpsc::channel(2);
        let path = temp.path().join("known_hosts");
        let connection = tokio::spawn(connect_ssh_with_path(
            host,
            "test-session".into(),
            80,
            24,
            events,
            path.clone(),
        ));
        let AppEvent::HostKeyPrompt { response_tx, .. } =
            tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("Missing key prompt");
        };
        response_tx.send(true).unwrap();
        let handle = tokio::time::timeout(Duration::from_secs(5), connection)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(path.exists());
        let payload = vec![b'x'; 48 * 1024];
        handle.send_data(payload.clone()).await.unwrap();
        let mut received = Vec::new();
        while received.len() < payload.len() {
            let event = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            let AppEvent::SshOutput { data, .. } = event else {
                panic!("Unexpected disconnect");
            };
            assert!(data.len() <= 16 * 1024);
            received.extend_from_slice(&data);
        }
        assert_eq!(received, payload);
        drop(handle);
        let event = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            AppEvent::SshDisconnected { error: None, .. }
        ));
        server.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn user_confirmation_time_is_excluded_from_network_timeout() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let future = async move {
            {
                let _pause = PromptWait::new(tx.clone());
                tokio::time::sleep(Duration::from_secs(120)).await;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            42
        };
        assert_eq!(
            network_timeout(future, rx, Duration::from_secs(2))
                .await
                .unwrap(),
            42
        );
    }

    #[tokio::test(start_paused = true)]
    async fn network_work_still_times_out() {
        let (_tx, rx) = tokio::sync::watch::channel(false);
        assert!(network_timeout(
            tokio::time::sleep(Duration::from_secs(3)),
            rx,
            Duration::from_secs(2)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn host_key_acceptance_is_persisted_and_rejection_does_not_write() {
        for accept in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("known_hosts");
            let key = russh_keys::key::KeyPair::generate_ed25519()
                .unwrap()
                .clone_public_key()
                .unwrap();
            let expected = key.clone();
            let (event_tx, mut events) = mpsc::channel(4);
            let (waiting_for_key, _paused) = tokio::sync::watch::channel(false);
            let mut handler = SshClientHandler {
                session_id: "test".into(),
                host: "localhost".into(),
                port: 2222,
                event_tx,
                waiting_for_key,
                known_hosts_path: path.clone(),
            };
            let task = tokio::spawn(async move { handler.check_server_key(&key).await });
            let AppEvent::HostKeyPrompt {
                response_tx,
                is_changed,
                ..
            } = events.recv().await.unwrap()
            else {
                panic!("Expected prompt");
            };
            assert!(!is_changed);
            response_tx.send(accept).unwrap();
            assert_eq!(task.await.unwrap().unwrap(), accept);
            assert_eq!(path.exists(), accept);
            if accept {
                assert!(
                    russh_keys::check_known_hosts_path("localhost", 2222, &expected, path).unwrap()
                );
            }
        }
    }

    #[tokio::test]
    async fn stale_host_key_confirmation_preserves_external_edits() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("known_hosts");
        let old = russh_keys::key::KeyPair::generate_ed25519()
            .unwrap()
            .clone_public_key()
            .unwrap();
        let new = russh_keys::key::KeyPair::generate_ed25519()
            .unwrap()
            .clone_public_key()
            .unwrap();
        russh_keys::learn_known_hosts_path("localhost", 22, &old, &path).unwrap();
        let snapshot = std::fs::read(&path).unwrap();
        let replacement = russh_keys::key::KeyPair::generate_ed25519()
            .unwrap()
            .clone_public_key()
            .unwrap();
        std::fs::write(&path, b"").unwrap();
        russh_keys::learn_known_hosts_path("localhost", 22, &replacement, &path).unwrap();
        let external = std::fs::read(&path).unwrap();
        assert!(
            persist_known_host(&path, "localhost", 22, &new, &snapshot, Some(2))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), external);
    }

    #[tokio::test]
    async fn parallel_confirmations_for_different_hosts_merge_known_hosts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("known_hosts");
        let a = russh_keys::key::KeyPair::generate_ed25519()
            .unwrap()
            .clone_public_key()
            .unwrap();
        let b = russh_keys::key::KeyPair::generate_ed25519()
            .unwrap()
            .clone_public_key()
            .unwrap();
        persist_known_host(&path, "one.example", 22, &a, b"", None)
            .await
            .unwrap();
        persist_known_host(&path, "two.example", 2222, &b, b"", None)
            .await
            .unwrap();
        assert!(russh_keys::check_known_hosts_path("one.example", 22, &a, &path).unwrap());
        assert!(russh_keys::check_known_hosts_path("two.example", 2222, &b, &path).unwrap());
    }

    #[tokio::test]
    async fn non_ssh_service_fails_without_modifying_known_hosts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("known_hosts");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let mut host = crate::config::sample_hosts_document().hosts.remove(0);
        host.port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
        });
        let (events, _receiver) = mpsc::channel(4);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            connect_ssh_with_path(host, "test".into(), 80, 24, events, path.clone()),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(!path.exists());
        server.await.unwrap();
    }

    #[test]
    fn test_remove_known_host_line() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let test_file = tmp_dir.path().join("known_hosts");
        let initial = "host1 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...\nhost2 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...\nhost3 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...\n";
        std::fs::write(&test_file, initial).unwrap();

        remove_known_host_line(&test_file, 2).unwrap();
        let updated = std::fs::read_to_string(&test_file).unwrap();
        assert!(!updated.contains("host2"));
        assert!(updated.contains("host1"));
        assert!(updated.contains("host3"));
        let _ = std::fs::remove_file(&test_file);
    }

    #[test]
    fn test_get_known_hosts_path() {
        let path = get_known_hosts_path();
        assert!(path.to_string_lossy().contains("known_hosts"));
    }
}
