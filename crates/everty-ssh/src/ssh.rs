use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use crossbeam_channel::Sender;
use russh::client::{self, Handler};
use tokio::sync::mpsc;

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
}

#[async_trait]
impl Handler for SshClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let kh_path = get_known_hosts_path();

        let check_res = russh_keys::check_known_hosts_path(&self.host, self.port, server_public_key, &kh_path);

        match check_res {
            Ok(true) => {
                // Key is already known and strictly matches!
                Ok(true)
            }
            Ok(false) => {
                // Host is not in known_hosts (first connection)
                let key_type = server_public_key.name().to_string();
                let fingerprint = format!("SHA256:{}", server_public_key.fingerprint());
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();

                let _ = self.event_tx.send(AppEvent::HostKeyPrompt {
                    session_id: self.session_id.clone(),
                    host: self.host.clone(),
                    port: self.port,
                    key_type,
                    fingerprint,
                    is_changed: false,
                    old_line: None,
                    response_tx,
                });

                match response_rx.await {
                    Ok(true) => {
                        let _ = russh_keys::learn_known_hosts_path(&self.host, self.port, server_public_key, &kh_path);
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            Err(russh_keys::Error::KeyChanged { line }) => {
                // Host key changed! MITM warning
                let key_type = server_public_key.name().to_string();
                let fingerprint = format!("SHA256:{}", server_public_key.fingerprint());
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();

                let _ = self.event_tx.send(AppEvent::HostKeyPrompt {
                    session_id: self.session_id.clone(),
                    host: self.host.clone(),
                    port: self.port,
                    key_type,
                    fingerprint,
                    is_changed: true,
                    old_line: Some(line),
                    response_tx,
                });

                match response_rx.await {
                    Ok(true) => {
                        let _ = remove_known_host_line(&kh_path, line);
                        let _ = russh_keys::learn_known_hosts_path(&self.host, self.port, server_public_key, &kh_path);
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            Err(_) => {
                // Other format or missing path: prompt user
                let key_type = server_public_key.name().to_string();
                let fingerprint = format!("SHA256:{}", server_public_key.fingerprint());
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();

                let _ = self.event_tx.send(AppEvent::HostKeyPrompt {
                    session_id: self.session_id.clone(),
                    host: self.host.clone(),
                    port: self.port,
                    key_type,
                    fingerprint,
                    is_changed: false,
                    old_line: None,
                    response_tx,
                });

                match response_rx.await {
                    Ok(true) => {
                        let _ = russh_keys::learn_known_hosts_path(&self.host, self.port, server_public_key, &kh_path);
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
        }
    }
}

pub struct SshSessionHandle {
    pub session_id: String,
    pub host: HostConfig,
    pub input_tx: mpsc::Sender<Vec<u8>>,
    pub resize_tx: mpsc::Sender<(u32, u32)>,
    pub is_connected: Arc<parking_lot::RwLock<bool>>,
}

impl SshSessionHandle {
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
    output_tx: mpsc::Sender<Vec<u8>>,
    event_tx: Sender<AppEvent>,
) -> Result<SshSessionHandle, String> {
    let config = Arc::new(client::Config::default());
    let addr = format!("{}:{}", host.hostname, host.port);

    // 1. TCP Connection + Server Key Verification with 30s timeout
    let handler = SshClientHandler {
        session_id: session_id.clone(),
        host: host.hostname.clone(),
        port: host.port,
        event_tx,
    };
    let connect_fut = client::connect(config, &addr, handler);
    let mut session = match tokio::time::timeout(Duration::from_secs(30), connect_fut).await {
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
                let key = russh_keys::load_secret_key(path, passphrase.as_deref())
                    .map_err(|e| format!("Не удалось прочитать приватный ключ {}: {e}", path.display()))?;

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
        Err(_) => return Err(format!("Таймаут запуска оболочки на сервере {addr} (превышено 5 сек)")),
    };

    let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(128);
    let (resize_tx, mut resize_rx) = mpsc::channel::<(u32, u32)>(16);
    let is_connected = Arc::new(parking_lot::RwLock::new(true));
    let is_connected_clone = is_connected.clone();

    // Spawn async background runner for this session
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(data) = input_rx.recv() => {
                    if channel.data(&data[..]).await.is_err() {
                        break;
                    }
                }
                Some((cols, rows)) = resize_rx.recv() => {
                    let _ = channel.window_change(cols, rows, 0, 0).await;
                }
                msg = channel.wait() => {
                    match msg {
                        Some(russh::ChannelMsg::Data { ref data }) => {
                            if output_tx.send(data.to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Some(russh::ChannelMsg::ExtendedData { ref data, .. }) => {
                            if output_tx.send(data.to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Some(russh::ChannelMsg::Close) | None => {
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        *is_connected_clone.write() = false;
        let _ = channel.close().await;
    });

    Ok(SshSessionHandle {
        session_id,
        host,
        input_tx,
        resize_tx,
        is_connected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_ssh_nonexistent_host() {
        let host = HostConfig {
            id: "test1".into(),
            name: "test".into(),
            hostname: "127.0.0.1".into(),
            port: 59999, // Unused local port, immediately refused
            username: "root".into(),
            auth: AuthConfig::Password { password: Some("123".into()) },
            group: None,
            tags: Vec::new(),
            badge_color: None,
            default_command: None,
            env: std::collections::HashMap::new(),
        };
        let (tx, _rx) = mpsc::channel(10);
        let (event_tx, _event_rx) = crossbeam_channel::unbounded();
        let res = connect_ssh(host, "sess1".into(), 80, 24, tx, event_tx).await;
        assert!(res.is_err());
        println!("Result: {:?}", res.err());
    }

    #[tokio::test]
    async fn test_agent_connect_on_windows() {
        let res = russh_keys::agent::client::AgentClient::connect_env().await;
        println!("Agent connect result on windows: {:?}", res.is_ok());
    }

    #[test]
    fn test_remove_known_host_line() {
        let tmp_dir = std::env::temp_dir();
        let test_file = tmp_dir.join("test_known_hosts_removal.txt");
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
