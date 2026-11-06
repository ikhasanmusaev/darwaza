//! Рантайм проброса портов (#130).
//!
//! Поддерживаются два вида, которым достаточно `direct-tcpip` со стороны клиента:
//!
//! - **local** — слушаем порт у себя, каждое соединение уводим на `remote_host:remote_port`
//!   глазами сервера (аналог `ssh -L`);
//! - **dynamic** — слушаем порт у себя как SOCKS5-прокси, адрес назначения
//!   приходит в самом протоколе (аналог `ssh -D`).
//!
//! - **remote** (`ssh -R`) — просим сервер слушать порт у себя; входящие соединения
//!   он открывает к нам сам, отдельными каналами. Поэтому здесь только реестр
//!   «какой удалённый порт куда отдавать локально», а сам приём каналов живёт в
//!   `client::Handler::server_channel_open_forwarded_tcpip` (`commands/ssh.rs`).
//!
//! Каналы запрашиваются у задачи сессии через `SessionCmd::OpenDirectTcpip` — тем же
//! способом, что и SFTP: `russh::client::Handle` не `Clone` и живёт внутри той задачи.

use crate::ssh::session::SessionCmd;
use crate::ssh::SshManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};
use tracing::{info, warn};
use uuid::Uuid;

/// Куда отдавать соединение, пришедшее с сервера: локальный адрес и порт.
type LocalTarget = (String, u16);
/// Ключ реестра remote-пробросов: сессия и порт, который слушает сервер.
type RemoteKey = (Uuid, u32);

/// Активный проброс: слушающий сокет + способ его остановить.
struct ActiveForward {
    session_id: Uuid,
    /// Отправка сюда останавливает accept-цикл. У remote-проброса локального
    /// цикла нет, и отправлять некому — там канал сразу закрыт.
    shutdown: oneshot::Sender<()>,
    /// Фактический адрес прослушки (порт может выделить ОС или сервер).
    bound: String,
    /// Для remote — что именно слушает сервер: нужно, чтобы снять прослушку.
    remote_binding: Option<(String, u32)>,
}

#[derive(Clone, Default)]
pub struct ForwardManager {
    active: Arc<Mutex<HashMap<i64, ActiveForward>>>,
    /// Куда отдавать соединения, пришедшие с сервера по remote-пробросу:
    /// (сессия, порт на сервере) → локальный адрес.
    ///
    /// Живёт отдельно от `active`, потому что заполняется до того, как сервер
    /// начнёт слать каналы, и читается из обработчика, а не из accept-цикла.
    remote_targets: Arc<Mutex<HashMap<RemoteKey, LocalTarget>>>,
}

/// Что именно пробрасывать. Отдельной структурой, чтобы `start` не превращался
/// в функцию с девятью позиционными аргументами, где легко перепутать порты.
pub struct ForwardSpec {
    pub forward_id: i64,
    /// `local` или `dynamic`.
    pub kind: String,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: Option<String>,
    pub remote_port: Option<u16>,
}

/// Снимок активного проброса для UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardSnapshot {
    pub forward_id: i64,
    pub session_id: Uuid,
    pub bound: String,
}

impl ForwardManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_active(&self, forward_id: i64) -> bool {
        self.active.lock().await.contains_key(&forward_id)
    }

    pub async fn snapshot(&self) -> Vec<ForwardSnapshot> {
        self.active
            .lock()
            .await
            .iter()
            .map(|(id, f)| ForwardSnapshot {
                forward_id: *id,
                session_id: f.session_id,
                bound: f.bound.clone(),
            })
            .collect()
    }

    /// Запомнить, куда отдавать соединения с удалённого порта.
    pub async fn register_remote_target(
        &self,
        session_id: Uuid,
        remote_port: u32,
        local_host: String,
        local_port: u16,
    ) {
        self.remote_targets
            .lock()
            .await
            .insert((session_id, remote_port), (local_host, local_port));
    }

    pub async fn remote_target(&self, session_id: Uuid, remote_port: u32) -> Option<LocalTarget> {
        self.remote_targets
            .lock()
            .await
            .get(&(session_id, remote_port))
            .cloned()
    }

    pub async fn unregister_remote_target(&self, session_id: Uuid, remote_port: u32) {
        self.remote_targets
            .lock()
            .await
            .remove(&(session_id, remote_port));
    }

    /// Проброшенное сервером соединение: тянем его на локальный адрес.
    /// Вызывается из обработчика входящих каналов.
    pub async fn serve_remote_connection(
        &self,
        session_id: Uuid,
        connected_port: u32,
        channel: russh::Channel<russh::client::Msg>,
    ) -> Result<(), String> {
        let (host, port) = self
            .remote_target(session_id, connected_port)
            .await
            .ok_or_else(|| format!("нет правила для удалённого порта {connected_port}"))?;

        let mut local = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| format!("не удалось подключиться к {host}:{port}: {e}"))?;
        let mut stream = channel.into_stream();
        tokio::io::copy_bidirectional(&mut local, &mut stream)
            .await
            .map_err(err)?;
        Ok(())
    }

    /// Учесть запущенный remote-проброс: у него нет локального сокета, но
    /// остановка и снимок для UI должны работать так же, как у остальных.
    pub async fn track_remote(
        &self,
        forward_id: i64,
        session_id: Uuid,
        address: String,
        port: u32,
    ) {
        let (shutdown, _rx) = oneshot::channel();
        self.active.lock().await.insert(
            forward_id,
            ActiveForward {
                session_id,
                shutdown,
                bound: format!("{address}:{port}"),
                remote_binding: Some((address, port)),
            },
        );
    }

    /// Адрес, который сервер слушает для этого правила — нужен, чтобы снять прослушку.
    pub async fn remote_binding(&self, forward_id: i64) -> Option<(Uuid, String, u32)> {
        let guard = self.active.lock().await;
        let f = guard.get(&forward_id)?;
        let (address, port) = f.remote_binding.clone()?;
        Some((f.session_id, address, port))
    }

    /// Остановить проброс. Возвращает `false`, если он и не был запущен.
    pub async fn stop(&self, forward_id: i64) -> bool {
        match self.active.lock().await.remove(&forward_id) {
            Some(f) => {
                // Ошибку игнорируем: задача могла уже завершиться сама
                // (например, при разрыве сессии).
                let _ = f.shutdown.send(());
                true
            }
            None => false,
        }
    }

    /// Остановить всё, что относится к сессии. Вызывается при её закрытии:
    /// без сессии каналы открывать не через что.
    pub async fn stop_for_session(&self, session_id: Uuid) {
        let mut guard = self.active.lock().await;
        let ids: Vec<i64> = guard
            .iter()
            .filter(|(_, f)| f.session_id == session_id)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(f) = guard.remove(&id) {
                let _ = f.shutdown.send(());
            }
        }
        drop(guard);
        // Сервер больше не пришлёт каналы этой сессии — реестр можно чистить.
        self.remote_targets
            .lock()
            .await
            .retain(|(sid, _), _| *sid != session_id);
    }

    /// Запустить проброс. `kind` — `local` или `dynamic`.
    pub async fn start(
        &self,
        manager: SshManager,
        session_id: Uuid,
        spec: ForwardSpec,
    ) -> Result<String, String> {
        let forward_id = spec.forward_id;
        if self.is_active(forward_id).await {
            return Err(format!("проброс {forward_id} уже запущен"));
        }

        let listener = TcpListener::bind((spec.local_host.as_str(), spec.local_port))
            .await
            .map_err(|e| {
                format!(
                    "не удалось занять {}:{}: {e}",
                    spec.local_host, spec.local_port
                )
            })?;
        let bound = listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| format!("{}:{}", spec.local_host, spec.local_port));

        let target = match spec.kind.as_str() {
            "local" => {
                let host = spec
                    .remote_host
                    .ok_or_else(|| "для local-проброса нужен remote_host".to_string())?;
                let port = spec
                    .remote_port
                    .ok_or_else(|| "для local-проброса нужен remote_port".to_string())?;
                Some((host, port))
            }
            "dynamic" => None,
            other => return Err(format!("неподдерживаемый вид проброса: {other}")),
        };

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let active = self.active.clone();

        tokio::spawn(async move {
            info!(target: "forward", "проброс {forward_id} слушает {}", bound_label(&listener));
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let (socket, peer) = match accepted {
                            Ok(v) => v,
                            Err(e) => {
                                warn!(target: "forward", "проброс {forward_id}: accept failed: {e}");
                                break;
                            }
                        };
                        let manager = manager.clone();
                        let target = target.clone();
                        tokio::spawn(async move {
                            let result = match target {
                                Some((host, port)) => {
                                    handle_local(manager, session_id, socket, peer, host, port).await
                                }
                                None => handle_socks5(manager, session_id, socket, peer).await,
                            };
                            if let Err(e) = result {
                                warn!(target: "forward", "проброс {forward_id}: соединение оборвалось: {e}");
                            }
                        });
                    }
                }
            }
            active.lock().await.remove(&forward_id);
            info!(target: "forward", "проброс {forward_id} остановлен");
        });

        self.active.lock().await.insert(
            forward_id,
            ActiveForward {
                session_id,
                shutdown: shutdown_tx,
                bound: bound.clone(),
                remote_binding: None,
            },
        );
        Ok(bound)
    }
}

fn bound_label(listener: &TcpListener) -> String {
    listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "?".to_string())
}

/// Попросить у задачи сессии `direct-tcpip` канал.
async fn open_channel(
    manager: &SshManager,
    session_id: Uuid,
    host: String,
    port: u16,
    peer: std::net::SocketAddr,
) -> Result<russh::Channel<russh::client::Msg>, String> {
    let (tx, rx) = oneshot::channel();
    manager
        .send_cmd(
            session_id,
            SessionCmd::OpenDirectTcpip {
                host,
                port: port as u32,
                originator_host: peer.ip().to_string(),
                originator_port: peer.port() as u32,
                responder: tx,
            },
        )
        .await?;
    rx.await
        .map_err(|_| format!("сессия {session_id} закрылась при открытии канала"))?
}

/// `ssh -L`: всё, что пришло на локальный сокет, уходит на фиксированный адрес.
async fn handle_local(
    manager: SshManager,
    session_id: Uuid,
    mut socket: TcpStream,
    peer: std::net::SocketAddr,
    host: String,
    port: u16,
) -> Result<(), String> {
    let channel = open_channel(&manager, session_id, host, port, peer).await?;
    let mut stream = channel.into_stream();
    tokio::io::copy_bidirectional(&mut socket, &mut stream)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

/// `ssh -D`: минимальный SOCKS5 — ровно столько, сколько нужно браузеру или curl.
///
/// Поддерживается метод «без аутентификации» и команда CONNECT; BIND и
/// UDP ASSOCIATE не реализованы — они не нужны для проброса TCP и требуют
/// принципиально другой обвязки.
async fn handle_socks5(
    manager: SshManager,
    session_id: Uuid,
    mut socket: TcpStream,
    peer: std::net::SocketAddr,
) -> Result<(), String> {
    // --- приветствие: VER, NMETHODS, METHODS ---
    let mut head = [0u8; 2];
    socket.read_exact(&mut head).await.map_err(err)?;
    if head[0] != 0x05 {
        return Err(format!("ожидали SOCKS5, получили версию {}", head[0]));
    }
    let mut methods = vec![0u8; head[1] as usize];
    socket.read_exact(&mut methods).await.map_err(err)?;
    if !methods.contains(&0x00) {
        // 0xFF — «нет подходящих методов»
        socket.write_all(&[0x05, 0xFF]).await.map_err(err)?;
        return Err("клиент не умеет SOCKS5 без аутентификации".to_string());
    }
    socket.write_all(&[0x05, 0x00]).await.map_err(err)?;

    // --- запрос: VER, CMD, RSV, ATYP ---
    let mut req = [0u8; 4];
    socket.read_exact(&mut req).await.map_err(err)?;
    if req[1] != 0x01 {
        // 0x07 — команда не поддерживается
        socket
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .map_err(err)?;
        return Err(format!("поддерживается только CONNECT, пришло {}", req[1]));
    }

    let host = match req[3] {
        0x01 => {
            let mut ip = [0u8; 4];
            socket.read_exact(&mut ip).await.map_err(err)?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            socket.read_exact(&mut len).await.map_err(err)?;
            let mut name = vec![0u8; len[0] as usize];
            socket.read_exact(&mut name).await.map_err(err)?;
            String::from_utf8(name).map_err(|e| format!("битое имя хоста: {e}"))?
        }
        0x04 => {
            let mut ip = [0u8; 16];
            socket.read_exact(&mut ip).await.map_err(err)?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        other => return Err(format!("неизвестный тип адреса SOCKS5: {other}")),
    };
    let mut port_bytes = [0u8; 2];
    socket.read_exact(&mut port_bytes).await.map_err(err)?;
    let port = u16::from_be_bytes(port_bytes);

    // --- собственно туннель ---
    let channel = match open_channel(&manager, session_id, host.clone(), port, peer).await {
        Ok(c) => c,
        Err(e) => {
            // 0x05 — сбой на стороне хоста; отвечаем, чтобы клиент не висел.
            let _ = socket
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return Err(e);
        }
    };

    // Успех. BND.ADDR/BND.PORT клиенты в режиме CONNECT игнорируют — шлём нули.
    socket
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(err)?;

    let mut stream = channel.into_stream();
    tokio::io::copy_bidirectional(&mut socket, &mut stream)
        .await
        .map_err(err)?;
    Ok(())
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}
