//! Живая проверка интерактивной аутентификации (#153).
//!
//! Локальный `sshd` без root не умеет keyboard-interactive: он идёт через PAM,
//! а тому нужны привилегии. Поэтому поднимаем **свой** SSH-сервер на russh —
//! так проверяется именно протокольный цикл «вопрос сервера → ответ клиента»,
//! который и есть рискованная часть.

use russh::keys::PrivateKey;
use russh::server::{Auth, Config, Handler as ServerHandler, Msg, Server, Session};
use russh::{client, Channel, MethodKind, MethodSet};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

/// Код, который «выдаёт приложение-аутентификатор».
const EXPECTED_CODE: &str = "424242";

#[derive(Clone)]
struct TestServer;

impl Server for TestServer {
    type Handler = TestSession;
    fn new_client(&mut self, _peer: Option<std::net::SocketAddr>) -> TestSession {
        TestSession { asked: false }
    }
}

struct TestSession {
    /// Первый заход — задаём вопрос, второй — проверяем ответ.
    asked: bool,
}

impl ServerHandler for TestSession {
    type Error = russh::Error;

    async fn auth_keyboard_interactive(
        &mut self,
        _user: &str,
        _submethods: &str,
        response: Option<russh::server::Response<'_>>,
    ) -> Result<Auth, Self::Error> {
        if !self.asked {
            self.asked = true;
            return Ok(Auth::Partial {
                name: Cow::Borrowed("2FA"),
                instructions: Cow::Borrowed("Введите код из приложения"),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Verification code: "), false)]),
            });
        }

        let ok = response
            .map(|mut r| {
                r.next()
                    .map(|bytes| bytes == EXPECTED_CODE.as_bytes())
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        if ok {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            })
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

struct AcceptAnyKey;

impl client::Handler for AcceptAnyKey {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Поднять сервер на свободном порту и вернуть его адрес.
async fn spawn_server() -> std::net::SocketAddr {
    // rand_core берём через реэкспорт ssh_key: у него та же версия, что
    // ожидает russh — отдельная зависимость дала бы конфликт типов.
    let key = PrivateKey::random(
        &mut russh::keys::ssh_key::rand_core::OsRng,
        russh::keys::Algorithm::Ed25519,
    )
    .expect("host key");
    // Сервер предлагает только интерактивный метод — иначе клиент мог бы
    // проскочить на другой и тест проверял бы не то.
    let methods = MethodSet::from(&[MethodKind::KeyboardInteractive][..]);
    let config = Arc::new(Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        methods,
        keys: vec![key],
        ..Default::default()
    });

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    tokio::spawn(async move {
        let mut server = TestServer;
        let _ = server.run_on_socket(config, &listener).await;
    });

    addr
}

#[tokio::test]
async fn keyboard_interactive_accepts_correct_code() {
    let addr = spawn_server().await;
    let config = Arc::new(client::Config::default());
    let mut session = client::connect(config, addr, AcceptAnyKey)
        .await
        .expect("connect");

    let first = session
        .authenticate_keyboard_interactive_start("tester".to_string(), None)
        .await
        .expect("start");

    // Сервер обязан прислать вопрос, а не сразу успех.
    let prompts = match first {
        client::KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => prompts,
        other => panic!("ожидали вопрос от сервера, получили {other:?}"),
    };
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].prompt.contains("code"));
    assert!(!prompts[0].echo, "код не должен отображаться при вводе");

    let result = session
        .authenticate_keyboard_interactive_respond(vec![EXPECTED_CODE.to_string()])
        .await
        .expect("respond");
    assert!(
        matches!(result, client::KeyboardInteractiveAuthResponse::Success),
        "верный код должен приниматься, получено {result:?}"
    );
}

#[tokio::test]
async fn keyboard_interactive_rejects_wrong_code() {
    let addr = spawn_server().await;
    let config = Arc::new(client::Config::default());
    let mut session = client::connect(config, addr, AcceptAnyKey)
        .await
        .expect("connect");

    session
        .authenticate_keyboard_interactive_start("tester".to_string(), None)
        .await
        .expect("start");

    let result = session
        .authenticate_keyboard_interactive_respond(vec!["000000".to_string()])
        .await
        .expect("respond");
    assert!(
        matches!(
            result,
            client::KeyboardInteractiveAuthResponse::Failure { .. }
        ),
        "неверный код должен отклоняться, получено {result:?}"
    );
}
