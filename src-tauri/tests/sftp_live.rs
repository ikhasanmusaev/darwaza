//! Живая проверка SSH/SFTP против настоящего sshd.
//!
//! До этого теста весь SFTP и сниппеты проверялись только на моках IPC: Playwright
//! убеждался, что UI зовёт нужные команды с нужными аргументами, но что `russh-sftp`
//! реально договорится с сервером — не проверял никто.
//!
//! Тест повторяет ту же последовательность, что и продакшн-код в
//! `commands/sftp.rs`: канал → подсистема `sftp` → `SftpSession` → операции.
//!
//! Помечен `#[ignore]`, потому что требует поднятого sshd. Запуск:
//! ```bash
//! # см. docs/05-manual-testing/ssh-live-test.md, раздел "SFTP и сниппеты"
//! DARWAZA_LIVE_KEY=/path/to/client_ed25519 cargo test --test sftp_live -- --ignored --nocapture
//! ```

use russh::client::{self, Handle};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct AcceptAnyKey;

impl client::Handler for AcceptAnyKey {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TOFU проверяется отдельно (#046); здесь интересует транспорт.
        Ok(true)
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

async fn connect() -> Handle<AcceptAnyKey> {
    let port: u16 = env_or("DARWAZA_LIVE_PORT", "2222").parse().expect("port");
    let user = env_or("DARWAZA_LIVE_USER", &whoami());
    let key_path = std::env::var("DARWAZA_LIVE_KEY")
        .expect("DARWAZA_LIVE_KEY не задан — см. docs/05-manual-testing/ssh-live-test.md");

    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });

    let mut session = client::connect(config, ("127.0.0.1", port), AcceptAnyKey)
        .await
        .expect("TCP+handshake");

    let key = load_secret_key(&key_path, None).expect("load key");
    let rsa_hash = session.best_supported_rsa_hash().await.unwrap().flatten();
    let res = session
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash))
        .await
        .expect("auth");
    assert!(res.success(), "publickey auth отклонена сервером");
    session
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".to_string())
}

/// Открыть SFTP ровно так же, как это делает `commands/ssh.rs::open_sftp_channel`.
async fn open_sftp(session: &Handle<AcceptAnyKey>) -> SftpSession {
    let channel = session.channel_open_session().await.expect("channel");
    channel
        .request_subsystem(true, "sftp")
        .await
        .expect("subsystem sftp");
    SftpSession::new(channel.into_stream())
        .await
        .expect("sftp handshake")
}

#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn sftp_full_cycle_against_real_sshd() {
    let session = connect().await;
    let sftp = open_sftp(&session).await;

    // Домашняя директория — точка входа файлового браузера.
    let home = sftp.canonicalize(".").await.expect("canonicalize");
    assert!(
        home.starts_with('/'),
        "ожидали абсолютный путь, получили {home}"
    );

    // Имя с пробелом и кириллицей: на моках такое не всплывало,
    // а в реальной эксплуатации встречается постоянно.
    let dir = format!("{home}/darwaza live тест");
    let _ = sftp.remove_dir(dir.clone()).await; // на случай мусора от прошлого прогона
    sftp.create_dir(dir.clone()).await.expect("create_dir");

    // Файл заведомо больше одного чанка передачи (32 КиБ),
    // чтобы проверить именно потоковую запись, а не единичный write.
    let payload: Vec<u8> = (0..(200 * 1024)).map(|i| (i % 251) as u8).collect();
    let remote_file = format!("{dir}/файл с пробелом.bin");

    {
        let mut f = sftp
            .open_with_flags(
                remote_file.clone(),
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            )
            .await
            .expect("open for write");
        for chunk in payload.chunks(32 * 1024) {
            f.write_all(chunk).await.expect("write chunk");
        }
        f.flush().await.expect("flush");
        f.shutdown().await.expect("close handle");
    }

    // Листинг должен видеть файл с правильным типом и размером.
    let entries = sftp.read_dir(dir.clone()).await.expect("read_dir");
    let found = entries
        .into_iter()
        .find(|e| e.file_name() == "файл с пробелом.bin")
        .expect("файл не найден в листинге");
    assert!(found.metadata().file_type().is_file());
    assert_eq!(
        found.metadata().size,
        Some(payload.len() as u64),
        "размер в листинге разошёлся с записанным"
    );

    // Чтение обратно — байт в байт.
    {
        let mut f = sftp.open(remote_file.clone()).await.expect("open for read");
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 32 * 1024];
        loop {
            let n = f.read(&mut chunk).await.expect("read chunk");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(buf.len(), payload.len(), "длина скачанного не совпала");
        assert_eq!(buf, payload, "содержимое скачанного не совпало");
    }

    // Переименование и удаление.
    let renamed = format!("{dir}/renamed.bin");
    sftp.rename(remote_file.clone(), renamed.clone())
        .await
        .expect("rename");
    assert!(sftp.try_exists(renamed.clone()).await.expect("try_exists"));
    assert!(!sftp.try_exists(remote_file).await.expect("try_exists"));

    sftp.remove_file(renamed).await.expect("remove_file");
    sftp.remove_dir(dir.clone()).await.expect("remove_dir");
    assert!(!sftp.try_exists(dir).await.expect("try_exists"));

    sftp.close().await.expect("sftp close");
    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
}

/// Картинка из буфера на сервере: те же флаги и права, что в
/// `commands/sftp.rs::create_private`. Снимок экрана в `/tmp` не должен читаться
/// другими пользователями, а подложенный заранее файл нельзя перезаписать.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn clipboard_image_file_is_private_and_exclusive() {
    use russh_sftp::protocol::FileAttributes;

    let session = connect().await;
    let sftp = open_sftp(&session).await;
    let path = format!("/tmp/darwaza-live-paste-{}.png", std::process::id());
    let attributes = || FileAttributes {
        permissions: Some(0o600),
        ..FileAttributes::default()
    };
    let flags = OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE;

    let mut file = sftp
        .open_with_flags_and_attributes(path.clone(), flags, attributes())
        .await
        .expect("create");
    file.write_all(b"png").await.expect("write");
    file.shutdown().await.expect("close");

    let meta = sftp.metadata(path.clone()).await.expect("metadata");
    assert_eq!(meta.permissions.expect("permissions") & 0o777, 0o600);
    assert_eq!(meta.size, Some(3));
    assert!(
        sftp.open_with_flags_and_attributes(path.clone(), flags, attributes())
            .await
            .is_err(),
        "существующий файл перезаписываться не должен"
    );

    sftp.remove_file(path).await.expect("cleanup");
}

/// SFTP-канал не должен мешать терминалу: оба живут на одной SSH-сессии.
/// Заодно проверяется путь сниппета — это тот же `ssh_send` с `\n` на конце.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn shell_and_sftp_share_one_session() {
    let session = connect().await;

    let mut shell = session.channel_open_session().await.expect("shell channel");
    shell
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    shell.request_shell(false).await.expect("shell");

    // SFTP поверх той же сессии, пока шелл уже открыт.
    let sftp = open_sftp(&session).await;
    let home = sftp.canonicalize(".").await.expect("canonicalize");
    assert!(home.starts_with('/'));

    // Отправка сниппета = ssh_send с переводом строки.
    shell
        .data(&b"echo DARWAZA_SNIPPET_OK\n"[..])
        .await
        .expect("send snippet");

    let mut output = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), shell.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                output.extend_from_slice(&data);
                if String::from_utf8_lossy(&output).contains("DARWAZA_SNIPPET_OK") {
                    break;
                }
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("DARWAZA_SNIPPET_OK"),
        "вывод сниппета не пришёл; получено: {text:?}"
    );

    // Проверяем, что SFTP жив после работы шелла — каналы не мешают друг другу.
    let entries = sftp.read_dir(home).await.expect("read_dir после шелла");
    assert!(entries.into_iter().count() > 0);

    sftp.close().await.expect("sftp close");
    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
}

/// ProxyJump (#151): подключение к цели через промежуточный хост.
///
/// Тот же sshd выступает и jump-хостом, и целью — для проверки механики этого
/// достаточно: важно, что второй SSH-хендшейк идёт **поверх direct-tcpip канала**
/// первого, а не по прямому TCP.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn proxy_jump_through_intermediate_host() {
    let port: u16 = env_or("DARWAZA_LIVE_PORT", "2222").parse().expect("port");

    // Хоп 1 — обычное TCP-подключение к jump-хосту.
    let jump = connect().await;

    // Хоп 2 — туннель до цели и полноценный SSH поверх него.
    let channel = jump
        .channel_open_direct_tcpip("127.0.0.1", port as u32, "127.0.0.1", 0)
        .await
        .expect("direct-tcpip до цели");

    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        ..Default::default()
    });
    let mut target = client::connect_stream(config, channel.into_stream(), AcceptAnyKey)
        .await
        .expect("handshake через туннель");

    let key_path = std::env::var("DARWAZA_LIVE_KEY").expect("DARWAZA_LIVE_KEY");
    let key = load_secret_key(&key_path, None).expect("load key");
    let rsa_hash = target.best_supported_rsa_hash().await.unwrap().flatten();
    let res = target
        .authenticate_publickey(
            env_or("DARWAZA_LIVE_USER", &whoami()),
            PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
        )
        .await
        .expect("auth на цели");
    assert!(res.success(), "auth на целевом хосте отклонена");

    // Через туннель должен работать обычный шелл.
    let mut shell = target.channel_open_session().await.expect("shell channel");
    shell
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    shell.request_shell(false).await.expect("shell");
    shell
        .data(&b"echo DARWAZA_JUMP_OK\n"[..])
        .await
        .expect("send");

    let mut output = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), shell.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                output.extend_from_slice(&data);
                if String::from_utf8_lossy(&output).contains("DARWAZA_JUMP_OK") {
                    break;
                }
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("DARWAZA_JUMP_OK"),
        "команда через jump-хост не отработала; получено: {text:?}"
    );

    // SFTP тоже должен работать через туннель — панель файлов на jump-хостах
    // не должна отличаться от прямого подключения.
    let sftp = open_sftp(&target).await;
    let home = sftp
        .canonicalize(".")
        .await
        .expect("canonicalize через jump");
    assert!(home.starts_with('/'));
    sftp.close().await.expect("sftp close");

    target
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect target");
    jump.disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect jump");
}

/// ssh-agent (#150): аутентификация без чтения приватного ключа приложением.
///
/// Требует запущенного агента с добавленным тестовым ключом — тест сам его
/// не поднимает, поэтому при отсутствии `SSH_AUTH_SOCK` осмысленно пропускается.
#[cfg(unix)] // свой ssh-agent через unix-сокет; на Windows агент — именованный канал
#[tokio::test]
#[ignore = "требует sshd на 2222 и ssh-agent с тестовым ключом"]
async fn agent_auth_against_real_sshd() {
    if std::env::var("SSH_AUTH_SOCK").is_err() {
        eprintln!("SSH_AUTH_SOCK не задан — тест пропущен");
        return;
    }

    let port: u16 = env_or("DARWAZA_LIVE_PORT", "2222").parse().expect("port");
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        ..Default::default()
    });
    let mut session = client::connect(config, ("127.0.0.1", port), AcceptAnyKey)
        .await
        .expect("connect");

    let mut agent = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .expect("подключение к ssh-agent");
    let identities = agent.request_identities().await.expect("список ключей");
    assert!(
        !identities.is_empty(),
        "в агенте нет ключей — добавьте тестовый через ssh-add"
    );

    let rsa_hash = session.best_supported_rsa_hash().await.unwrap().flatten();
    let user = env_or("DARWAZA_LIVE_USER", &whoami());

    let mut accepted = false;
    for key in identities {
        let res = session
            .authenticate_publickey_with(user.clone(), key, rsa_hash, &mut agent)
            .await
            .expect("agent auth");
        if res.success() {
            accepted = true;
            break;
        }
    }
    assert!(accepted, "ни один ключ из агента не принят сервером");

    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
}

/// Local-проброс (#130, аналог `ssh -L`): соединение с локальным портом должно
/// доехать до цели глазами сервера.
///
/// Цель — сам же sshd на 2222: если через туннель приедет его SSH-баннер,
/// значит `direct-tcpip` донёс байты в обе стороны.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn local_forward_carries_traffic() {
    use tokio::net::TcpListener;

    let port: u16 = env_or("DARWAZA_LIVE_PORT", "2222").parse().expect("port");
    let session = connect().await;

    // Слушаем произвольный свободный порт (0 — выдаст ОС).
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let local_addr = listener.local_addr().expect("local_addr");

    let session = Arc::new(session);
    let accept_session = session.clone();
    let pump = tokio::spawn(async move {
        let (mut socket, peer) = listener.accept().await.expect("accept");
        let channel = accept_session
            .channel_open_direct_tcpip(
                "127.0.0.1",
                port as u32,
                peer.ip().to_string(),
                peer.port() as u32,
            )
            .await
            .expect("direct-tcpip");
        let mut stream = channel.into_stream();
        let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
    });

    // Клиент подключается к локальному порту и должен увидеть баннер sshd.
    let mut client = tokio::net::TcpStream::connect(local_addr)
        .await
        .expect("connect к локальному порту проброса");
    let mut banner = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut banner))
        .await
        .expect("таймаут ожидания данных через проброс")
        .expect("чтение из проброса");
    assert_eq!(
        &banner, b"SSH-",
        "через проброс пришло не то, что отдаёт sshd: {banner:?}"
    );

    drop(client);
    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
}

/// Remote-проброс (#130a, аналог `ssh -R`): сервер слушает порт и сам открывает
/// к нам каналы. Проверяем всю цепочку — запрос прослушки, приём входящего
/// канала, доставку байт на локальную цель.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn remote_forward_delivers_incoming_connections() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::net::TcpListener;

    /// Handler, который принимает входящие каналы проброса и тянет их
    /// на локальный echo-сервер — ровно то, что делает ClientHandler.
    struct ForwardHandler {
        local_port: Arc<AtomicU32>,
    }

    impl client::Handler for ForwardHandler {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _key: &russh::keys::ssh_key::PublicKey,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }

        async fn server_channel_open_forwarded_tcpip(
            &mut self,
            channel: russh::Channel<client::Msg>,
            _connected_address: &str,
            _connected_port: u32,
            _originator_address: &str,
            _originator_port: u32,
            _session: &mut client::Session,
        ) -> Result<(), Self::Error> {
            let port = self.local_port.load(Ordering::SeqCst) as u16;
            tokio::spawn(async move {
                let mut local = tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .expect("подключение к локальной цели");
                let mut stream = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut local, &mut stream).await;
            });
            Ok(())
        }
    }

    // Локальная цель: отдаёт фиксированную строку и закрывается.
    let target = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind target");
    let target_port = target.local_addr().expect("addr").port();
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = target.accept().await {
            let _ = socket.write_all(b"REMOTE_FORWARD_OK").await;
            let _ = socket.shutdown().await;
        }
    });

    let port: u16 = env_or("DARWAZA_LIVE_PORT", "2222").parse().expect("port");
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        ..Default::default()
    });
    let mut session = client::connect(
        config,
        ("127.0.0.1", port),
        ForwardHandler {
            local_port: Arc::new(AtomicU32::new(target_port as u32)),
        },
    )
    .await
    .expect("connect");

    let key_path = std::env::var("DARWAZA_LIVE_KEY").expect("DARWAZA_LIVE_KEY");
    let key = load_secret_key(&key_path, None).expect("load key");
    let rsa_hash = session.best_supported_rsa_hash().await.unwrap().flatten();
    let res = session
        .authenticate_publickey(
            env_or("DARWAZA_LIVE_USER", &whoami()),
            PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
        )
        .await
        .expect("auth");
    assert!(res.success(), "auth отклонена");

    // Просим сервер слушать произвольный свободный порт (0 — выберет сам).
    let bound_port = session
        .tcpip_forward("127.0.0.1", 0)
        .await
        .expect("сервер отказал в прослушке");
    assert!(bound_port > 0, "сервер не вернул номер порта");

    // Подключаемся к порту НА СЕРВЕРЕ — он откроет канал к нам,
    // handler дотянет его до локальной цели.
    let mut probe = tokio::net::TcpStream::connect(("127.0.0.1", bound_port as u16))
        .await
        .expect("подключение к проброшенному порту сервера");
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), probe.read_to_end(&mut buf))
        .await
        .expect("таймаут ожидания данных через remote-проброс")
        .expect("чтение");
    assert_eq!(
        String::from_utf8_lossy(&buf),
        "REMOTE_FORWARD_OK",
        "через remote-проброс пришло не то"
    );

    session
        .cancel_tcpip_forward("127.0.0.1", bound_port)
        .await
        .expect("снятие прослушки");
    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
}

/// Проброс агента (#150a, аналог `ForwardAgent yes`).
///
/// Проверяем всю цепочку: запрос `auth-agent-req@openssh.com`, приём обратного
/// канала от сервера и проксирование его на локальный сокет агента. Успех — если
/// `ssh-add -l` **на сервере** видит наш ключ, то есть агент реально доступен
/// удалённой стороне.
#[cfg(unix)] // свой ssh-agent через unix-сокет; на Windows агент — именованный канал
#[tokio::test]
#[ignore = "требует sshd на 2222 и ssh-agent с тестовым ключом"]
async fn agent_forwarding_exposes_agent_to_remote() {
    if std::env::var("SSH_AUTH_SOCK").is_err() {
        eprintln!("SSH_AUTH_SOCK не задан — тест пропущен");
        return;
    }

    struct AgentForwardHandler;

    impl client::Handler for AgentForwardHandler {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _key: &russh::keys::ssh_key::PublicKey,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }

        async fn server_channel_open_agent_forward(
            &mut self,
            channel: russh::Channel<client::Msg>,
            _session: &mut client::Session,
        ) -> Result<(), Self::Error> {
            let sock = std::env::var("SSH_AUTH_SOCK").expect("SSH_AUTH_SOCK");
            tokio::spawn(async move {
                if let Ok(mut agent) = tokio::net::UnixStream::connect(&sock).await {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut agent, &mut stream).await;
                }
            });
            Ok(())
        }
    }

    let port: u16 = env_or("DARWAZA_LIVE_PORT", "2222").parse().expect("port");
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        ..Default::default()
    });
    let mut session = client::connect(config, ("127.0.0.1", port), AgentForwardHandler)
        .await
        .expect("connect");

    let key_path = std::env::var("DARWAZA_LIVE_KEY").expect("DARWAZA_LIVE_KEY");
    let key = load_secret_key(&key_path, None).expect("load key");
    let rsa_hash = session.best_supported_rsa_hash().await.unwrap().flatten();
    let res = session
        .authenticate_publickey(
            env_or("DARWAZA_LIVE_USER", &whoami()),
            PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
        )
        .await
        .expect("auth");
    assert!(res.success(), "auth отклонена");

    let mut channel = session.channel_open_session().await.expect("channel");
    channel
        .agent_forward(false)
        .await
        .expect("запрос проброса агента");
    channel
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(false).await.expect("shell");

    // На сервере агент должен быть виден: ssh-add -l перечислит наши ключи.
    channel
        .data(&b"ssh-add -l && echo AGENT_FORWARD_OK\n"[..])
        .await
        .expect("send");

    let mut output = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                output.extend_from_slice(&data);
                if String::from_utf8_lossy(&output).contains("AGENT_FORWARD_OK") {
                    break;
                }
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }

    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("AGENT_FORWARD_OK"),
        "ssh-add -l на сервере не увидел проброшенный агент; вывод: {text:?}"
    );

    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
}

/// Запись сессии (#144): вывод настоящего сервера должен оказаться в `.cast`.
///
/// Проверяем именно сквозной путь — байты из SSH-канала проходят через рекордер
/// и читаются обратно как валидный asciinema-файл.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn recording_captures_real_terminal_output() {
    use darwaza_lib::recording::Recorder;

    let session = connect().await;
    let mut shell = session.channel_open_session().await.expect("shell channel");
    shell
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    shell.request_shell(false).await.expect("shell");

    let dir = std::env::temp_dir().join(format!("darwaza-live-rec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("live.cast");
    let mut recorder = Recorder::start(path.clone(), 80, 24, Some("live".into()))
        .await
        .expect("start recorder");

    shell
        .data(&b"echo DARWAZA_RECORDING_OK\n"[..])
        .await
        .expect("send");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut seen = false;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), shell.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                // Ровно то, что делает задача сессии: тот же чанк идёт и в UI, и в запись.
                recorder.write_output(&data).await.expect("write");
                if String::from_utf8_lossy(&data).contains("DARWAZA_RECORDING_OK") {
                    seen = true;
                    break;
                }
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    assert!(seen, "маркер не пришёл из шелла");
    recorder.finish().await.expect("finish");

    // Файл должен быть валидным asciinema: заголовок + события с нашим маркером.
    let content = std::fs::read_to_string(&path).expect("read cast");
    let mut lines = content.lines();
    let header: serde_json::Value =
        serde_json::from_str(lines.next().expect("заголовок")).expect("заголовок — JSON");
    assert_eq!(header["version"], 2);

    let payload: String = lines
        .map(|l| {
            let event: (f64, String, String) = serde_json::from_str(l).expect("событие — JSON");
            event.2
        })
        .collect();
    assert!(
        payload.contains("DARWAZA_RECORDING_OK"),
        "вывод сервера не попал в запись: {payload:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
}

/// Одиночное выполнение команды (#200): именно то, чем будет пользоваться агент.
///
/// Проверяется на настоящем sshd, потому что модульный тест здесь ничего не
/// доказывает: смысл в том, как протокол разделяет stdout, stderr и код
/// возврата, а это поведение сервера, а не нашего кода.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn exec_separates_streams_and_reports_exit_code() {
    let session = connect().await;

    // Успешная команда: вывод в stdout, код 0.
    let mut channel = session.channel_open_session().await.expect("канал");
    channel.exec(true, "echo DARWAZA_OUT").await.expect("exec");

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut code = None;
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
            // Только Close: код возврата приходит ПОСЛЕ Eof.
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    assert!(String::from_utf8_lossy(&stdout).contains("DARWAZA_OUT"));
    assert!(
        stderr.is_empty(),
        "чистая команда не должна писать в stderr"
    );
    assert_eq!(code, Some(0));

    // Неуспешная: вывод в stderr и НЕнулевой код. Агент обязан отличать
    // «выполнилось» от «выполнилось с ошибкой», иначе он примет провал за
    // успех и пойдёт дальше по плану.
    let mut channel = session.channel_open_session().await.expect("канал");
    channel
        .exec(true, "echo DARWAZA_ERR >&2; exit 3")
        .await
        .expect("exec");

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut code = None;
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
            // Только Close: код возврата приходит ПОСЛЕ Eof.
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    assert!(stdout.is_empty(), "вывод не должен был попасть в stdout");
    assert!(String::from_utf8_lossy(&stderr).contains("DARWAZA_ERR"));
    assert_eq!(code, Some(3), "код возврата обязан доехать точно");

    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("disconnect");
}

/// Запись файла обязана создавать его и обрезать (#208).
///
/// Найдено на живом сервере: `darwaza_write_file` падал на шаге «сохранить
/// копию» с невнятным «No such file». Причина — `SftpSession::write` из
/// библиотеки открывает файл **только** флагом `WRITE`, без `CREATE` и без
/// `TRUNCATE`.
///
/// Проверяются обе беды, и вторая опаснее первой:
///
/// 1. **Создание.** Копии `*.darwaza-bak` ещё не существует — открыть её на
///    запись нельзя, и правка обрывалась до самой записи.
/// 2. **Обрезка.** Без `TRUNCATE` новый текст ложится поверх старого, не
///    укорачивая файл: запиши конфиг короче прежнего — и в хвосте останется
///    мусор от старого. Этого никто не видел только потому, что первая беда
///    срабатывала раньше.
///
/// Обычный тест такого не покажет: флаги открытия — это разговор с сервером,
/// и внутри процесса разницы между `write` и `create` не видно.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — см. docs/05-manual-testing/ssh-live-test.md"]
async fn writing_a_file_creates_it_and_truncates_it() {
    let session = connect().await;
    let sftp = open_sftp(&session).await;

    let path = format!("/tmp/darwaza-write-{}.conf", std::process::id());
    let _ = sftp.remove_file(path.clone()).await;

    // ── 1. Файла нет: запись обязана его создать ─────────────────────────
    let long = "строка один\nстрока два\nстрока три\nхвост подлиннее\n";
    put(&sftp, &path, long.as_bytes()).await;

    let read = String::from_utf8(sftp.read(path.clone()).await.expect("чтение")).expect("utf-8");
    assert_eq!(read, long, "запись обязана создавать отсутствующий файл");

    // ── 2. Пишем короче: хвоста старого остаться не должно ───────────────
    let short = "одна строка\n";
    put(&sftp, &path, short.as_bytes()).await;

    let read = String::from_utf8(sftp.read(path.clone()).await.expect("чтение")).expect("utf-8");
    assert_eq!(
        read, short,
        "файл обязан укорачиваться: в хвосте остался мусор от прежнего содержимого"
    );
    assert!(
        !read.contains("хвост подлиннее"),
        "обрезки не произошло, прежний текст виден: {read:?}"
    );

    sftp.remove_file(path).await.expect("уборка за собой");
    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("отключение");
}

/// Запись тем же способом, каким её делает приложение (`create`, не `write`).
async fn put(sftp: &SftpSession, path: &str, data: &[u8]) {
    let mut file = sftp.create(path.to_string()).await.expect("создание файла");
    file.write_all(data).await.expect("запись");
    file.shutdown().await.expect("закрытие");
}

/// Передача файлов для агента: загрузка с копией и скачивание без порчи (#212).
///
/// Проверяется то, что обычный тест внутри процесса не увидит: как ведёт себя
/// настоящий SFTP-сервер при замене файла, при переименовании поверх
/// существующей копии и при оборванной передаче.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — scripts/live-sshd.sh"]
async fn agent_transfers_keep_files_whole() {
    use darwaza_lib::agent::transfer::{download, upload, BACKUP_SUFFIX};

    let session = connect().await;
    let sftp = open_sftp(&session).await;

    let remote_dir = format!("/tmp/darwaza-agent-{}", std::process::id());
    let _ = sftp.create_dir(remote_dir.clone()).await;
    let local_dir =
        std::env::temp_dir().join(format!("darwaza-agent-local-{}", std::process::id()));
    std::fs::create_dir_all(&local_dir).expect("локальный каталог");

    let remote = format!("{remote_dir}/app.conf");
    let local = local_dir.join("app.conf");

    // ── 1. Нового файла нет: загрузка создаёт его, копии нет ─────────────
    std::fs::write(&local, "первая версия\n").expect("запись");
    let outcome = upload(&sftp, &local, &remote).await.expect("загрузка");
    assert_eq!(
        outcome.backup, None,
        "заменять было нечего — копии быть не должно"
    );
    assert_eq!(
        String::from_utf8(sftp.read(remote.clone()).await.expect("чтение")).unwrap(),
        "первая версия\n"
    );

    // ── 2. Файл есть: он уходит в копию, новый — на место ────────────────
    std::fs::write(&local, "вторая\n").expect("запись");
    let outcome = upload(&sftp, &local, &remote).await.expect("замена");
    let backup = format!("{remote}{BACKUP_SUFFIX}");
    assert_eq!(outcome.backup.as_deref(), Some(backup.as_str()));
    assert_eq!(
        String::from_utf8(sftp.read(remote.clone()).await.unwrap()).unwrap(),
        "вторая\n"
    );
    assert_eq!(
        String::from_utf8(sftp.read(backup.clone()).await.unwrap()).unwrap(),
        "первая версия\n"
    );

    // ── 3. Замена ещё раз: SFTP не переименовывает поверх существующего, ─
    //       старая копия обязана уступить место свежей
    std::fs::write(&local, "третья\n").expect("запись");
    upload(&sftp, &local, &remote).await.expect("вторая замена");
    assert_eq!(
        String::from_utf8(sftp.read(backup.clone()).await.unwrap()).unwrap(),
        "вторая\n",
        "в копии обязана лежать последняя заменённая версия"
    );

    // ── 4. На сервере каталог: отказ, ничего не тронуто ──────────────────
    let err = upload(&sftp, &local, &remote_dir)
        .await
        .expect_err("поверх каталога нельзя");
    assert!(err.contains("каталог"), "{err}");

    // ── 5. Скачивание: файл целиком, недокачанного хвоста не остаётся ────
    let downloaded = local_dir.join("downloaded.conf");
    let bytes = download(&sftp, &remote, &downloaded)
        .await
        .expect("скачивание");
    assert_eq!(bytes, "третья\n".len() as u64);
    assert_eq!(std::fs::read_to_string(&downloaded).unwrap(), "третья\n");
    assert!(
        !local_dir.join("downloaded.conf.darwaza-part").exists(),
        "после удачного скачивания временного файла быть не должно"
    );

    // ── 6. Скачивание оборвалось: существующий файл остаётся целым ───────
    //
    // Обрыв нужен именно ПОСЛЕ начала записи. Несуществующий файл здесь не
    // годится: открытие на сервере падает раньше, чем создаётся локальный, и
    // защита временным файлом не проверялась бы вовсе — первая версия этого
    // теста так и проходила с поломанной защитой. Каталог открывается, а
    // чтение из него падает — то, что нужно.
    std::fs::write(&downloaded, "моё, не трогать\n").expect("запись");
    download(&sftp, &remote_dir, &downloaded)
        .await
        .expect_err("из каталога не скачать");
    assert_eq!(
        std::fs::read_to_string(&downloaded).unwrap(),
        "моё, не трогать\n",
        "неудачное скачивание не должно было тронуть лежащий на месте файл"
    );
    assert!(!local_dir.join("downloaded.conf.darwaza-part").exists());

    // ── 7. Каталога назначения нет: отказ, каталоги сами не создаются ────
    let missing = local_dir.join("нет-каталога").join("x.conf");
    download(&sftp, &remote, &missing)
        .await
        .expect_err("каталога нет");
    assert!(!local_dir.join("нет-каталога").exists());

    // Уборка.
    for f in [remote.clone(), backup] {
        let _ = sftp.remove_file(f).await;
    }
    let _ = sftp.remove_dir(remote_dir).await;
    let _ = std::fs::remove_dir_all(&local_dir);
    session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
        .expect("отключение");
}

/// Копирование между серверами (#222): содержимое едет целиком, прежний файл
/// на приёмнике откладывается в копию.
///
/// Две SFTP-сессии, а не одна: копирование между серверами — это два разных
/// подключения, и общий на двоих объект вёл бы себя иначе. Локальный sshd
/// здесь играет обе стороны, и этого достаточно: различает их приложение, а
/// не сервер.
///
/// Обычный тест такого не покажет — поток идёт через настоящий SFTP, и
/// разница между «прочитали всё» и «прочитали первый кусок» видна только на
/// файле, который в один кусок не влезает.
#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — scripts/live-sshd.sh"]
async fn copying_between_servers_moves_the_whole_file() {
    use darwaza_lib::agent::transfer::{copy_between, BACKUP_SUFFIX};

    let from_session = connect().await;
    let from_sftp = open_sftp(&from_session).await;
    let to_session = connect().await;
    let to_sftp = open_sftp(&to_session).await;

    let dir = format!("/tmp/darwaza-copy-{}", std::process::id());
    let _ = from_sftp.create_dir(dir.clone()).await;
    let source = format!("{dir}/dump.sql");
    let target = format!("{dir}/dump-copy.sql");
    let backup = format!("{target}{BACKUP_SUFFIX}");

    // Больше одного куска передачи (32 КиБ): на файле в пару байт
    // «прочитали только первый кусок» выглядело бы как успех.
    let body: String = (0..5000)
        .map(|i| format!("INSERT INTO t VALUES ({i}, 'строка {i}');\n"))
        .collect();
    put(&from_sftp, &source, body.as_bytes()).await;
    put(&to_sftp, &target, "прежнее содержимое\n".as_bytes()).await;

    let outcome = copy_between(&from_sftp, &source, &to_sftp, &target)
        .await
        .expect("копирование");

    assert_eq!(outcome.bytes, body.len() as u64, "передано не всё");
    assert_eq!(outcome.backup.as_deref(), Some(backup.as_str()));

    let copied =
        String::from_utf8(to_sftp.read(target.clone()).await.expect("чтение")).expect("utf-8");
    assert_eq!(copied, body, "содержимое доехало не целиком");

    let kept = String::from_utf8(to_sftp.read(backup.clone()).await.expect("чтение копии"))
        .expect("utf-8");
    assert_eq!(
        kept, "прежнее содержимое\n",
        "прежний файл приёмника не сохранён"
    );

    // Отсутствующий источник — отказ, а не пустой файл на приёмнике.
    let missing = format!("{dir}/нет-такого.sql");
    let err = copy_between(&from_sftp, &missing, &to_sftp, &target)
        .await
        .expect_err("копирование несуществующего обязано отказать");
    assert!(err.contains("нет файла"), "{err}");
    let intact =
        String::from_utf8(to_sftp.read(target.clone()).await.expect("чтение")).expect("utf-8");
    assert_eq!(
        intact, body,
        "неудачное копирование испортило файл приёмника"
    );

    for path in [source, target, backup] {
        let _ = to_sftp.remove_file(path).await;
    }
    let _ = to_sftp.remove_dir(dir).await;
    for session in [from_session, to_session] {
        session
            .disconnect(Disconnect::ByApplication, "", "en")
            .await
            .expect("отключение");
    }
}

#[tokio::test]
#[ignore = "требует локальный sshd на 2222 — scripts/live-sshd.sh"]
async fn folders_travel_whole_in_every_direction() {
    use darwaza_lib::transfers::tree::{copy_tree, LocalTree, OnConflict, Outcome, RemoteTree};
    use std::sync::atomic::AtomicBool;

    let a_session = connect().await;
    let a = Arc::new(open_sftp(&a_session).await);
    let b_session = connect().await;
    let b = Arc::new(open_sftp(&b_session).await);

    let pid = std::process::id();
    let local = std::env::temp_dir().join(format!("darwaza-tree-live-{pid}"));
    let _ = std::fs::remove_dir_all(&local);
    std::fs::create_dir_all(local.join("proj/a/b")).unwrap();
    std::fs::create_dir_all(local.join("proj/empty")).unwrap();
    // Больше одного куска передачи (32 КиБ) и не-ASCII в имени.
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(local.join("proj/big.bin"), &big).unwrap();
    std::fs::write(local.join("proj/a/b/файл.txt"), "привет").unwrap();

    let remote_a = format!("/tmp/darwaza-tree-a-{pid}");
    let remote_b = format!("/tmp/darwaza-tree-b-{pid}");
    let _ = a.create_dir(remote_a.clone()).await;
    let _ = b.create_dir(remote_b.clone()).await;

    let no_cancel = AtomicBool::new(false);
    let mut quiet = |_p: darwaza_lib::transfers::tree::Progress| {};

    // Компьютер → сервер.
    let up = copy_tree(
        &LocalTree,
        &local.join("proj").to_string_lossy(),
        "proj",
        true,
        &RemoteTree(a.clone()),
        &remote_a,
        OnConflict::Rename,
        &no_cancel,
        &mut quiet,
    )
    .await
    .expect("загрузка папки");
    assert!(matches!(up, Outcome::Done { files: 2, .. }), "{up:?}");
    assert_eq!(a.read(format!("{remote_a}/proj/big.bin")).await.unwrap(), big);
    assert!(a.metadata(format!("{remote_a}/proj/empty")).await.unwrap().is_dir());

    // Сервер → сервер, без диска: те же файлы на другой сессии.
    let across = copy_tree(
        &RemoteTree(a.clone()),
        &format!("{remote_a}/proj"),
        "proj",
        true,
        &RemoteTree(b.clone()),
        &remote_b,
        OnConflict::Rename,
        &no_cancel,
        &mut quiet,
    )
    .await
    .expect("копирование между серверами");
    assert!(matches!(across, Outcome::Done { files: 2, .. }), "{across:?}");
    assert_eq!(b.read(format!("{remote_b}/proj/big.bin")).await.unwrap(), big);
    assert_eq!(
        b.read(format!("{remote_b}/proj/a/b/файл.txt")).await.unwrap(),
        "привет".as_bytes()
    );

    // Сервер → компьютер, повтор в ту же папку получает свободное имя.
    let down_dir = local.join("down");
    std::fs::create_dir_all(&down_dir).unwrap();
    for _ in 0..2 {
        let down = copy_tree(
            &RemoteTree(b.clone()),
            &format!("{remote_b}/proj"),
            "proj",
            true,
            &LocalTree,
            &down_dir.to_string_lossy(),
            OnConflict::Rename,
            &no_cancel,
            &mut quiet,
        )
        .await
        .expect("скачивание папки");
        assert!(matches!(down, Outcome::Done { files: 2, .. }), "{down:?}");
    }
    assert_eq!(std::fs::read(down_dir.join("proj/big.bin")).unwrap(), big);
    assert!(down_dir.join("proj (2)/a/b/файл.txt").exists(), "повтор затёр или потерялся");

    // Уборка.
    let _ = std::fs::remove_dir_all(&local);
    for (sftp, root) in [(&a, &remote_a), (&b, &remote_b)] {
        for f in ["proj/big.bin", "proj/a/b/файл.txt"] {
            let _ = sftp.remove_file(format!("{root}/{f}")).await;
        }
        for d in ["proj/a/b", "proj/a", "proj/empty", "proj"] {
            let _ = sftp.remove_dir(format!("{root}/{d}")).await;
        }
        let _ = sftp.remove_dir(root.clone()).await;
    }
    for s in [a_session, b_session] {
        s.disconnect(Disconnect::ByApplication, "", "en").await.expect("отключение");
    }
}
