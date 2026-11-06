use crate::db::models::{Host, HostInput};
use crate::keyring_store::kinds;
use crate::master::MasterKeyring;
use crate::AppState;
use sqlx::SqlitePool;
use tauri::State;

const SELECT_FIELDS: &str = "id, group_id, name, hostname, port, username, identity_id, \
    auth_method, color, icon, jump_host_id, forward_agent, agent_access, agent_sudo_read, \
    agent_notes, keep_alive_interval, description, last_connected_at, created_at, updated_at";

/// SQL вставки хоста.
///
/// Вынесен из `hosts_create`, чтобы его мог выполнить тест: список колонок,
/// число плейсхолдеров и число `.bind()` обязаны совпадать, а компилятор за
/// этим не следит. Разъезд стоил сломанного добавления хоста — колонка
/// `agent_access` попала в список, а `?15` дописать забыли, и человек видел
/// «14 values for 15 columns».
fn insert_sql() -> String {
    format!(
        "INSERT INTO hosts (group_id, name, hostname, port, username, identity_id, \
            auth_method, color, icon, jump_host_id, forward_agent, agent_access, agent_sudo_read, \
            agent_notes, keep_alive_interval, description, sync_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
         RETURNING {SELECT_FIELDS}"
    )
}

/// Список команд с `sudo` в том виде, в каком он ложится в базу (#214).
///
/// Пустой список и «не задано» — одно и то же: строка из пробелов и переводов
/// строки не должна выглядеть как разрешение.
fn sudo_list(raw: Option<&str>) -> Option<String> {
    let cleaned: Vec<&str> = raw?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.join("\n"))
    }
}

/// Ошибка базы при чтении: занятая пара и мёртвая ссылка разбираются
/// отдельно, остальное — «не удалось прочитать» (#177).
fn load_err(e: sqlx::Error) -> String {
    crate::errors::db_load("hosts", e)
}

/// То же при записи.
fn save_err(e: sqlx::Error) -> String {
    crate::errors::db_save("hosts", e)
}

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("hosts", e)
}

fn validate_auth_method(value: &str) -> Result<(), String> {
    match value {
        "password" | "publickey" | "agent" => Ok(()),
        other => Err(crate::errors::opaque(
            "validate_auth_method",
            format!("недопустимый auth_method: {other}"),
        )),
    }
}

fn validate_port(port: i64) -> Result<(), String> {
    if !(1..=65535).contains(&port) {
        return Err(crate::errors::invalid(
            "validate_port",
            format!("порт {port} вне 1-65535"),
            crate::errors::codes::INVALID_PORT,
        ));
    }
    Ok(())
}

fn validate_keep_alive(value: i64) -> Result<(), String> {
    // 0 = disabled, верхняя граница 3600 (1 час) — за пределами теряет смысл
    if !(0..=3600).contains(&value) {
        return Err(crate::errors::invalid(
            "validate_keep_alive",
            format!("keep_alive {value} вне 0-3600"),
            crate::errors::codes::INVALID_KEEP_ALIVE,
        ));
    }
    Ok(())
}

/// Обрезать пробелы по краям у адреса, имени пользователя и названия.
///
/// Хост, сохранённый как « 1.1.1.1», превращал соединение в `root@ 1.1.1.1`,
/// и оно не работало, а причину в форме было не увидеть. Пустое после обрезки
/// и пробел внутри адреса или имени пользователя — ошибка проверки, а не
/// молчаливое «как есть».
fn normalize_host_input(data: &mut HostInput) -> Result<(), String> {
    use crate::errors::{codes, invalid};

    data.name = data.name.trim().to_string();
    data.hostname = data.hostname.trim().to_string();
    data.username = data.username.trim().to_string();

    if data.name.is_empty() {
        return Err(invalid("normalize_host_input", "название пусто", codes::NAME_REQUIRED));
    }
    if data.hostname.is_empty() || data.hostname.chars().any(char::is_whitespace) {
        return Err(invalid(
            "normalize_host_input",
            "адрес пуст или содержит пробел",
            codes::INVALID_HOSTNAME,
        ));
    }
    if data.username.is_empty() || data.username.chars().any(char::is_whitespace) {
        return Err(invalid(
            "normalize_host_input",
            "имя пользователя пусто или содержит пробел",
            codes::INVALID_USERNAME,
        ));
    }
    Ok(())
}

fn validate_host_input(data: &HostInput) -> Result<(), String> {
    validate_auth_method(&data.auth_method)?;
    validate_port(data.port)?;
    if let Some(ka) = data.keep_alive_interval {
        validate_keep_alive(ka)?;
    }
    Ok(())
}

/// Если пришёл inline password и нет identity_id — авто-создать identity + сохранить в keyring.
async fn resolve_inline_password(
    db: &SqlitePool,
    master: &MasterKeyring,
    data: &mut HostInput,
) -> Result<(), String> {
    let password = match data.password.take() {
        Some(p) if !p.is_empty() && data.identity_id.is_none() => p,
        _ => return Ok(()),
    };

    let label = format!("{}@{}", data.username, data.hostname);
    let sql = "INSERT INTO identities (label, kind, has_passphrase, sync_id) \
               VALUES (?1, 'password', false, ?2) RETURNING id";
    let (identity_id,): (i64,) = sqlx::query_as(sql)
        .bind(&label)
        .bind(crate::db::new_sync_id())
        .fetch_one(db)
        .await
        .map_err(map_err)?;

    crate::secrets::set(db, master, kinds::PASSWORD, identity_id, &password).await?;

    data.identity_id = Some(identity_id);
    Ok(())
}

#[tauri::command]
pub async fn hosts_list(state: State<'_, AppState>) -> Result<Vec<Host>, String> {
    let sql = format!("SELECT {SELECT_FIELDS} FROM hosts ORDER BY name");
    sqlx::query_as::<_, Host>(&sql)
        .fetch_all(&state.db)
        .await
        .map_err(load_err)
}

#[tauri::command]
pub async fn hosts_get(state: State<'_, AppState>, id: i64) -> Result<Host, String> {
    let sql = format!("SELECT {SELECT_FIELDS} FROM hosts WHERE id = ?1");
    sqlx::query_as::<_, Host>(&sql)
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(load_err)?
        .ok_or_else(|| crate::errors::not_found("hosts", "хост", id))
}

#[tauri::command]
pub async fn hosts_create(state: State<'_, AppState>, mut data: HostInput) -> Result<Host, String> {
    normalize_host_input(&mut data)?;
    validate_host_input(&data)?;
    resolve_inline_password(&state.db, &state.master, &mut data).await?;
    let keep_alive = data.keep_alive_interval.unwrap_or(30);

    let sql = insert_sql();

    sqlx::query_as::<_, Host>(&sql)
        .bind(data.group_id)
        .bind(&data.name)
        .bind(&data.hostname)
        .bind(data.port)
        .bind(&data.username)
        .bind(data.identity_id)
        .bind(&data.auth_method)
        .bind(&data.color)
        .bind(&data.icon)
        .bind(data.jump_host_id)
        .bind(data.forward_agent.unwrap_or(false))
        // Доступ агента не включается «по умолчанию из формы»: даже если поле
        // забудут прислать, хост останется закрытым.
        .bind(data.agent_access.unwrap_or(false))
        // Пустая строка — то же, что «не задано»: иначе список из одних
        // переводов строки выглядел бы как разрешение.
        .bind(sudo_list(data.agent_sudo_read.as_deref()))
        .bind(&data.agent_notes)
        .bind(keep_alive)
        .bind(&data.description)
        .bind(crate::db::new_sync_id())
        .fetch_one(&state.db)
        .await
        .map_err(save_err)
}

/// SQL обновления хоста. Отдельной функцией, чтобы тест гонял тот же текст.
///
/// `icon`: не пришёл (`NULL`) — прежний остаётся, иначе клиент, не знающий о
/// поле, стёр бы его при каждом сохранении; пустая строка — убрать значок.
fn update_sql() -> String {
    format!(
        "UPDATE hosts SET \
            group_id = ?2, name = ?3, hostname = ?4, port = ?5, username = ?6, \
            identity_id = ?7, auth_method = ?8, color = ?9, \
            icon = CASE WHEN ?10 IS NULL THEN icon ELSE NULLIF(?10, '') END, \
            jump_host_id = ?11, forward_agent = COALESCE(?12, forward_agent), \
            agent_access = COALESCE(?15, agent_access), \
            agent_sudo_read = ?16, \
            agent_notes = COALESCE(?17, agent_notes), \
            keep_alive_interval = COALESCE(?13, keep_alive_interval), \
            description = ?14, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?1 \
         RETURNING {SELECT_FIELDS}"
    )
}

#[tauri::command]
pub async fn hosts_update(
    state: State<'_, AppState>,
    id: i64,
    mut data: HostInput,
) -> Result<Host, String> {
    normalize_host_input(&mut data)?;
    validate_host_input(&data)?;
    resolve_inline_password(&state.db, &state.master, &mut data).await?;

    let sql = update_sql();

    sqlx::query_as::<_, Host>(&sql)
        .bind(id)
        .bind(data.group_id)
        .bind(&data.name)
        .bind(&data.hostname)
        .bind(data.port)
        .bind(&data.username)
        .bind(data.identity_id)
        .bind(&data.auth_method)
        .bind(&data.color)
        .bind(&data.icon)
        .bind(data.jump_host_id)
        .bind(data.forward_agent)
        .bind(data.keep_alive_interval)
        .bind(&data.description)
        .bind(data.agent_access)
        .bind(sudo_list(data.agent_sudo_read.as_deref()))
        .bind(&data.agent_notes)
        .fetch_optional(&state.db)
        .await
        .map_err(save_err)?
        .ok_or_else(|| crate::errors::not_found("hosts", "хост", id))
}

#[tauri::command]
pub async fn hosts_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    sqlx::query("DELETE FROM hosts WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(save_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_db() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("подключение к памяти");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");
        pool
    }

    /// Вставка хоста обязана выполняться на настоящей базе.
    ///
    /// Проверка нужна потому, что SQL здесь — строка: ни список колонок, ни
    /// число `?N`, ни число `.bind()` компилятор не сверяет. Расхождение
    /// собирается зелёным и падает только у человека, нажавшего «Сохранить»:
    /// «14 values for 15 columns». Тест повторяет ровно ту же
    /// последовательность привязок, что и `hosts_create`.
    #[tokio::test]
    async fn insert_binds_every_column() {
        let pool = fresh_db().await;

        let host: Host = sqlx::query_as(&insert_sql())
            .bind(None::<i64>) // group_id
            .bind("тестовый")
            .bind("example.org")
            .bind(22)
            .bind("root")
            .bind(None::<i64>) // identity_id
            .bind("password")
            .bind(None::<String>) // color
            .bind(None::<String>) // icon
            .bind(None::<i64>) // jump_host_id
            .bind(false) // forward_agent
            .bind(false) // agent_access
            .bind(None::<String>) // agent_sudo_read
            .bind(None::<String>) // agent_notes
            .bind(30) // keep_alive_interval
            .bind(None::<String>) // description
            .bind(crate::db::new_sync_id())
            .fetch_one(&pool)
            .await
            .expect("вставка хоста тем же SQL, каким её делает hosts_create");

        assert_eq!(host.name, "тестовый");
        // Хост, созданный из формы, закрыт для агента, пока его не откроют.
        assert!(!host.agent_access);
    }

    async fn update_icon(pool: &SqlitePool, id: i64, icon: Option<&str>) -> Host {
        sqlx::query_as::<_, Host>(&update_sql())
            .bind(id)
            .bind(None::<i64>)
            .bind("тестовый")
            .bind("example.org")
            .bind(22)
            .bind("root")
            .bind(None::<i64>)
            .bind("password")
            .bind(None::<String>) // color
            .bind(icon)
            .bind(None::<i64>) // jump_host_id
            .bind(None::<bool>) // forward_agent
            .bind(None::<i64>) // keep_alive_interval
            .bind(None::<String>) // description
            .bind(None::<bool>) // agent_access
            .bind(None::<String>) // agent_sudo_read
            .bind(None::<String>) // agent_notes
            .fetch_one(pool)
            .await
            .expect("обновление")
    }

    fn input(name: &str, hostname: &str, username: &str) -> HostInput {
        serde_json::from_value(serde_json::json!({
            "name": name, "hostname": hostname, "port": 22,
            "username": username, "authMethod": "password",
        }))
        .expect("HostInput")
    }

    /// Пробелы по краям адреса, имени пользователя и названия убираются;
    /// пустое и пробел внутри — ошибка с кодом, а не «как есть».
    #[test]
    fn spaces_around_the_address_are_trimmed_and_bad_values_refused() {
        let mut ok = input("  prod  ", " 1.1.1.1 ", " root\t");
        normalize_host_input(&mut ok).expect("обрезка");
        assert_eq!((ok.name.as_str(), ok.hostname.as_str(), ok.username.as_str()), ("prod", "1.1.1.1", "root"));

        use crate::errors::codes;
        let mut a = input("n", "   ", "root");
        assert_eq!(normalize_host_input(&mut a).unwrap_err(), codes::INVALID_HOSTNAME);
        let mut b = input("n", "1.1. 1.1", "root");
        assert_eq!(normalize_host_input(&mut b).unwrap_err(), codes::INVALID_HOSTNAME);
        let mut c = input("n", "1.1.1.1", " ");
        assert_eq!(normalize_host_input(&mut c).unwrap_err(), codes::INVALID_USERNAME);
        let mut d = input("n", "1.1.1.1", "ro ot");
        assert_eq!(normalize_host_input(&mut d).unwrap_err(), codes::INVALID_USERNAME);
        let mut e = input("  ", "1.1.1.1", "root");
        assert_eq!(normalize_host_input(&mut e).unwrap_err(), codes::NAME_REQUIRED);
    }

    /// Уже сохранённая запись с пробелами чистится миграцией.
    #[tokio::test]
    async fn the_trim_migration_cleans_existing_rows() {
        let pool = fresh_db().await;
        sqlx::query(
            "INSERT INTO hosts (name, hostname, port, username, auth_method, sync_id) \
             VALUES (' old ', ' 1.1.1.1', 22, 'root ', 'password', 'sync-trim')",
        )
        .execute(&pool)
        .await
        .expect("вставка");
        sqlx::query(include_str!("../../migrations/20261104000001_trim_host_fields.sql"))
            .execute(&pool)
            .await
            .expect("миграция");
        let (n, h, u): (String, String, String) =
            sqlx::query_as("SELECT name, hostname, username FROM hosts")
                .fetch_one(&pool)
                .await
                .expect("чтение");
        assert_eq!((n.as_str(), h.as_str(), u.as_str()), ("old", "1.1.1.1", "root"));
    }

    /// Клиент, не передавший `icon`, не стирает значок; пустая строка стирает.
    #[tokio::test]
    async fn update_without_icon_keeps_it_and_empty_string_clears_it() {
        let pool = fresh_db().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO hosts (name, hostname, port, username, auth_method, icon, sync_id) \
             VALUES ('h', 'example.org', 22, 'root', 'password', 'server', ?1) RETURNING id",
        )
        .bind(crate::db::new_sync_id())
        .fetch_one(&pool)
        .await
        .expect("хост");

        assert_eq!(update_icon(&pool, id, None).await.icon.as_deref(), Some("server"));
        assert_eq!(update_icon(&pool, id, Some("db")).await.icon.as_deref(), Some("db"));
        assert_eq!(update_icon(&pool, id, Some("")).await.icon, None);
    }
}
