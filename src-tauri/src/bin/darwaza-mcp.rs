//! Посредник между агентом и приложением Darwaza (#202).
//!
//! Агент запускает эту программу сам и разговаривает с ней по стандартному
//! вводу-выводу протоколом MCP. Сама она никуда не подключается: передаёт
//! запросы работающему приложению через локальный сокет.
//!
//! Почему так, а не «агент ходит в SSH напрямую»: секреты остаются в
//! приложении, а подтверждения показываются в окне, где человек уже ввёл
//! мастер-пароль. Посредник не видит ни ключей, ни адресов — он только
//! пересылает.
//!
//! Настройка у агента выглядит так и не содержит ни одного секрета:
//! ```json
//! { "mcpServers": { "darwaza": { "command": "darwaza-mcp" } } }
//! ```

use serde_json::{json, Value};
use std::io::{BufRead, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
// Проверка владельца канала и прав — общий с приложением файл: две копии
// расходятся молча. Библиотеку приложения целиком посредник по-прежнему не тянет.
#[cfg(windows)]
#[path = "../winsec.rs"]
mod winsec;

/// Как назвался агент при `initialize`. Приложение показывает это рядом с
/// планом; проверить имя нечем, так что это подпись, а не удостоверение.
static CLIENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Идентификатор этой сессии: посредник живёт ровно столько, сколько сессия
/// агента, поэтому достаточно номера процесса и времени запуска.
fn session_id() -> &'static str {
    static SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SESSION.get_or_init(|| {
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{:x}-{:x}", std::process::id(), started & 0xffff)
    })
}

/// Версия протокола MCP, на которую мы отвечаем.
const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(response) = handle_line(&line) else {
            // Уведомления ответа не требуют: молчим, а не шлём пустое.
            continue;
        };
        if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
            break;
        }
    }
}

/// Разобрать сообщение MCP и подготовить ответ.
fn handle_line(line: &str) -> Option<String> {
    let request: Value = serde_json::from_str(line).ok()?;
    let method = request.get("method")?.as_str()?;
    // Уведомления приходят без идентификатора и ответа не ждут.
    let id = request.get("id").cloned()?;

    let result = match method {
        "initialize" => {
            if let Some(name) = request
                .pointer("/params/clientInfo/name")
                .and_then(Value::as_str)
            {
                let _ = CLIENT.set(name.to_string());
            }
            Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "darwaza", "version": env!("CARGO_PKG_VERSION") },
            }))
        }
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => call_tool(request.get("params")),
        other => Err(format!("неизвестный метод '{other}'")),
    };

    let body = match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(message) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32000, "message": message },
        }),
    };
    Some(body.to_string())
}

/// Описания инструментов для агента.
fn tool_definitions() -> Value {
    json!([
        {
            "name": "darwaza_hosts_list",
            "description": "Список серверов, к которым владелец разрешил доступ. \
                Возвращает только имена и группы: адреса, логины и ключи не раскрываются.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "darwaza_run",
            "description": "Выполнить читающую команду на сервере и вернуть вывод с кодом \
                возврата. Команды, которые что-либо меняют, отклоняются: изменения требуют \
                подтверждения человеком.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string", "description": "Имя сервера из darwaza_hosts_list" },
                    "command": { "type": "string", "description": "Одна команда без конвейеров и сцепления" }
                },
                "required": ["host", "command"]
            }
        },
        {
            "name": "darwaza_read_file",
            "description": "Прочитать файл на сервере.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["host", "path"]
            }
        },
        {
            "name": "darwaza_plan",
            "description": "Предложить изменения на сервере. Владелец увидит замысел и все \
                шаги одним списком и подтвердит их целиком. После подтверждения шаги \
                выполняются по порядку и останавливаются на первой же ошибке. Вызов ждёт \
                решения человека до пяти минут, а затем — выполнения шагов: на шаг \
                отводится до 15 минут, так что сборка или миграция успевают дойти до \
                конца. Долгий ответ здесь означает, что работа идёт, а не что всё \
                зависло. Смотрите на поле status: \"completed\" — выполнены все шаги; \
                \"stopped\" — план остановлен (шаг упал, не вышел по времени или запрещён), \
                в completed — что успело выполниться, в notRun — что НЕ выполнялось. \
                Остановленный план нельзя считать выполненным.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string", "description": "Имя сервера из darwaza_hosts_list" },
                    "intent": {
                        "type": "string",
                        "description": "Зачем это делается, одной фразой на языке владельца. \
                            Человек подтверждает замысел, а не набор строк."
                    },
                    "steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Команды по порядку, по одной в строке списка"
                    }
                },
                "required": ["host", "intent", "steps"]
            }
        },
        {
            "name": "darwaza_write_file",
            "description": "Записать файл на сервере целиком. Присылайте ПОЛНОЕ новое \
                содержимое, а не команду правки: владелец увидит построчную разницу и \
                подтвердит её. Перед записью сохраняется копия, а для известных \
                конфигураций выполняется проверка (nginx -t, sshd -t) с откатом при \
                неудаче.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "path": { "type": "string" },
                    "content": { "type": "string", "description": "Полное новое содержимое файла" },
                    "intent": { "type": "string", "description": "Зачем правится файл" }
                },
                "required": ["host", "path", "content"]
            }
        },
        {
            "name": "darwaza_patch_file",
            "description": "Правка файла заменами вместо полного содержимого: для каждой \
                замены — что найти дословно (find) и чем заменить (replace, пустая строка \
                удаляет). Так правят большие конфиги, не пересылая их целиком. Искомый \
                фрагмент обязан встречаться в файле РОВНО ОДИН раз: два совпадения — отказ, \
                добавьте окружающие строки. Владелец видит ту же построчную разницу и \
                подтверждает её; копия, проверка конфигурации и откат — как у \
                darwaza_write_file. Файл должен существовать: создать новый — через \
                darwaza_write_file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "path": { "type": "string" },
                    "edits": {
                        "type": "array",
                        "description": "Замены по порядку",
                        "items": {
                            "type": "object",
                            "properties": {
                                "find": { "type": "string" },
                                "replace": { "type": "string" }
                            },
                            "required": ["find", "replace"]
                        }
                    },
                    "intent": { "type": "string", "description": "Зачем правится файл" }
                },
                "required": ["host", "path", "edits"]
            }
        },
        {
            "name": "darwaza_service",
            "description": "Действие со службой systemd: start, stop, restart, reload, \
                try-reload-or-restart. Требует подтверждения. Перед действием выполняется \
                проверка конфигурации, если она для этой службы известна (nginx -t, \
                sshd -t, apachectl configtest) — не прошла, служба не трогается вовсе. \
                После действия проверяется, что служба в нужном состоянии, и при неудаче \
                возвращаются последние строки её журнала. enable, disable и mask сюда не \
                входят: они меняют поведение при загрузке, для них используйте darwaza_plan.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "unit": { "type": "string", "description": "Имя службы, например nginx" },
                    "action": {
                        "type": "string",
                        "enum": ["start", "stop", "restart", "reload", "try-reload-or-restart"]
                    },
                    "sudo": {
                        "type": "boolean",
                        "description": "Выполнять через sudo. Обычно нужно: общая служба без root не перезапускается"
                    },
                    "intent": { "type": "string", "description": "Зачем это делается" }
                },
                "required": ["host", "unit", "action"]
            }
        },
        {
            "name": "darwaza_open_terminal",
            "description": "Открыть владельцу вкладку терминала на сервере с НАБРАННОЙ, но \
                НЕ выполненной командой: Enter нажимает он сам. Для того, что агенту \
                запрещено навсегда — разметка диска, смена ключа сервера, — вместо \
                «сделайте сами» подаёт готовое. Команда не должна содержать перевод строки. \
                Подтверждения не требует: выполнять нечего.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "command": { "type": "string", "description": "Команда одной строкой, без перевода строки" },
                    "reason": { "type": "string", "description": "Зачем это владельцу — попадёт в журнал" }
                },
                "required": ["host"]
            }
        },
        {
            "name": "darwaza_host_notes",
            "description": "Заметки о сервере: что за приложение, где лежит, как выкатывается. \
                Без notes — прочитать, с notes — записать (заменяет прежние целиком, пустая \
                строка стирает). Подтверждения не требует: заметка не трогает сервер и видна \
                владельцу в карточке хоста. Пишите то, что сэкономит время следующей сессии, \
                а не журнал работ: не длиннее 4000 знаков.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "notes": { "type": "string", "description": "Новый текст заметки целиком" }
                },
                "required": ["host"]
            }
        },
        {
            "name": "darwaza_copy",
            "description": "Скопировать файл с одного сервера на другой, минуя диск владельца: \
                содержимое идёт потоком через приложение. Требует подтверждения. Оба хоста \
                должны быть открыты агенту по отдельности. Прежний файл на сервере назначения \
                сохраняется как .darwaza-bak и возвращается на место, если передача \
                оборвалась. Каталоги не копируются — только файл.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fromHost": { "type": "string" },
                    "fromPath": { "type": "string" },
                    "toHost": { "type": "string" },
                    "toPath": { "type": "string" },
                    "intent": { "type": "string", "description": "Зачем копируется" }
                },
                "required": ["fromHost", "fromPath", "toHost", "toPath"]
            }
        },
        {
            "name": "darwaza_tunnel",
            "description": "Временный проброс порта до внутренней службы сервера: PostgreSQL, \
                Redis, внутренний HTTP. Требует подтверждения. Слушает только 127.0.0.1 и \
                ЗАКРЫВАЕТСЯ САМ по истечении срока — по умолчанию через 5 минут, не больше \
                часа. В ответе придёт локальный адрес, к которому можно обращаться. SOCKS и \
                проброс со стороны сервера недоступны: это не доступ к службе, а выход в сеть.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "remotePort": {
                        "type": "integer",
                        "description": "Порт службы на сервере, 1–65535"
                    },
                    "remoteHost": {
                        "type": "string",
                        "description": "Адрес со стороны сервера (по умолчанию 127.0.0.1)"
                    },
                    "seconds": {
                        "type": "integer",
                        "description": "Насколько открыть, 1–3600 (по умолчанию 300)"
                    },
                    "intent": { "type": "string", "description": "Зачем нужен доступ" }
                },
                "required": ["host", "remotePort"]
            }
        },
        {
            "name": "darwaza_snippets_list",
            "description": "Сниппеты владельца, открытые агенту: имя, команда, описание. Без \
                подтверждения. Открыты не все — у каждого сниппета свой флаг, и выключен он \
                по умолчанию. Это готовые способы делать что-то на серверах владельца, уже \
                им проверенные: если нужное есть здесь, берите его, а не придумывайте свой.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "darwaza_run_snippet",
            "description": "Выполнить сниппет владельца на хосте. Требует подтверждения — \
                тем же планом, что и обычные изменения. То, что команду написал владелец, \
                подтверждения не отменяет: он писал её для себя, а не для запуска без \
                спроса. Имя — как в darwaza_snippets_list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "snippet": { "type": "string", "description": "Имя сниппета" }
                },
                "required": ["host", "snippet"]
            }
        },
        {
            "name": "darwaza_follow",
            "description": "Смотреть на вывод команды заданное число секунд и вернуть \
                собранное: journalctl -f во время перезапуска, tail -f по логу приложения. \
                Без подтверждения, проверка команды та же, что у darwaza_run. По умолчанию \
                15 секунд, не больше 120. Команда не завершается сама — её останавливает \
                время, поэтому exitCode придёт пустым, а stoppedByTime будет true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "command": { "type": "string" },
                    "seconds": {
                        "type": "integer",
                        "description": "Сколько секунд смотреть, 1–120 (по умолчанию 15)"
                    }
                },
                "required": ["host", "command"]
            }
        },
        {
            "name": "darwaza_summary",
            "description": "Сводка о сервере одним вызовом: система и ядро, время работы, диск, \
                память, слушающие порты, упавшие службы. Без подтверждения. С этого стоит \
                начинать знакомство с незнакомым сервером — иначе те же сведения берутся семью \
                вызовами darwaza_run. Имён процессов у портов не будет: для них нужны права. \
                Часть может не получиться (например, нет ss) — остальные всё равно придут.",
            "inputSchema": {
                "type": "object",
                "properties": { "host": { "type": "string" } },
                "required": ["host"]
            }
        },
        {
            "name": "darwaza_journal",
            "description": "Что уже делалось на этом сервере через Darwaza: последние записи \
                журнала — действие, что именно, чем кончилось. Отказы показаны наравне с \
                удачным: они избавляют от второй попытки в том же тупике. Без подтверждения. \
                Стоит смотреть в начале работы с сервером — вместе с darwaza_host_notes это \
                заменяет разведку заново. По умолчанию 50 записей, не больше 200.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "limit": {
                        "type": "integer",
                        "description": "Сколько последних записей, 1–200 (по умолчанию 50)"
                    }
                },
                "required": ["host"]
            }
        },
        {
            "name": "darwaza_list_dir",
            "description": "Содержимое каталога на сервере: имя, тип, размер, время изменения, \
                права. Без подтверждения. Больше 1000 записей не отдаётся — тогда \
                truncated=true и total показывает, сколько их на самом деле.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "path": { "type": "string", "description": "Абсолютный путь каталога на сервере" }
                },
                "required": ["host", "path"]
            }
        },
        {
            "name": "darwaza_stat",
            "description": "Сведения о пути на сервере: есть ли он, файл это или каталог, \
                размер, время изменения, права, владелец. Без подтверждения. \
                Отсутствие пути — ответ exists=false, а не ошибка.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["host", "path"]
            }
        },
        {
            "name": "darwaza_download",
            "description": "Скачать файл с сервера на компьютер владельца. Требует \
                подтверждения: владелец увидит, откуда, куда и сколько, и заменится ли \
                существующий файл. Локальный путь — абсолютный или с ~, без '..'. \
                Файл пишется целиком или не пишется вовсе: оборванная передача \
                существующий файл не портит. Каталоги не создаются.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "remotePath": { "type": "string", "description": "Путь файла на сервере" },
                    "localPath": { "type": "string", "description": "Куда положить на компьютере" },
                    "intent": { "type": "string", "description": "Зачем скачивается файл" }
                },
                "required": ["host", "remotePath", "localPath"]
            }
        },
        {
            "name": "darwaza_upload",
            "description": "Загрузить файл с компьютера владельца на сервер. Требует \
                подтверждения. Если на сервере файл уже есть, он сохраняется копией \
                <путь>.darwaza-bak, а при обрыве загрузки возвращается на место. \
                Локальный путь — абсолютный или с ~, без '..'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "localPath": { "type": "string", "description": "Файл на компьютере" },
                    "remotePath": { "type": "string", "description": "Куда положить на сервере" },
                    "intent": { "type": "string", "description": "Зачем загружается файл" }
                },
                "required": ["host", "localPath", "remotePath"]
            }
        },
        {
            "name": "darwaza_mkdir",
            "description": "Создать каталог на сервере. Требует подтверждения. Родительский \
                каталог должен существовать.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "path": { "type": "string" },
                    "intent": { "type": "string" }
                },
                "required": ["host", "path"]
            }
        },
        {
            "name": "darwaza_rename",
            "description": "Переименовать или переместить файл либо каталог на сервере. \
                Требует подтверждения. Существующий файл по новому пути не заменяется.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "intent": { "type": "string" }
                },
                "required": ["host", "from", "to"]
            }
        },
        {
            "name": "darwaza_delete",
            "description": "Удалить файл или ПУСТОЙ каталог на сервере. Требует \
                подтверждения. Рекурсивного удаления нет: для дерева каталогов \
                используйте darwaza_plan с командой rm, чтобы владелец видел её целиком.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "path": { "type": "string" },
                    "intent": { "type": "string" }
                },
                "required": ["host", "path"]
            }
        }
    ])
}

/// Метод приложения, которым выполняется инструмент.
///
/// Отдельной функцией ради проверки: описание инструмента для агента и его
/// проводка в приложение живут в двух местах этого файла, и забыть второе —
/// значит выдать агенту инструмент, который на каждый вызов отвечает
/// «неизвестный инструмент».
fn method_for(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "darwaza_hosts_list" => "hosts.list",
        "darwaza_run" => "hosts.run",
        "darwaza_read_file" => "files.read",
        "darwaza_plan" => "hosts.plan",
        "darwaza_write_file" => "files.write",
        "darwaza_patch_file" => "files.patch",
        "darwaza_service" => "hosts.service",
        "darwaza_open_terminal" => "hosts.open_terminal",
        "darwaza_host_notes" => "hosts.notes",
        "darwaza_journal" => "hosts.journal",
        "darwaza_summary" => "hosts.summary",
        "darwaza_follow" => "hosts.follow",
        "darwaza_copy" => "files.copy",
        "darwaza_tunnel" => "hosts.tunnel",
        "darwaza_snippets_list" => "snippets.list",
        "darwaza_run_snippet" => "snippets.run",
        "darwaza_list_dir" => "files.list",
        "darwaza_stat" => "files.stat",
        "darwaza_download" => "files.download",
        "darwaza_upload" => "files.upload",
        "darwaza_mkdir" => "files.mkdir",
        "darwaza_rename" => "files.rename",
        "darwaza_delete" => "files.delete",
        _ => return None,
    })
}

/// Перевести вызов инструмента в запрос к приложению.
fn call_tool(params: Option<&Value>) -> Result<Value, String> {
    let params = params.ok_or("нет параметров вызова")?;
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("не указано имя инструмента")?;
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    let method = method_for(name).ok_or_else(|| format!("неизвестный инструмент '{name}'"))?;

    let answer = ask_app(method, arguments)?;

    // MCP ждёт содержимое списком блоков. Отдаём текстом: агенту всё равно
    // разбирать глазами, а JSON внутри текста читается им не хуже.
    let text = if let Some(error) = answer.get("error").and_then(|v| v.as_str()) {
        return Ok(json!({
            "content": [{ "type": "text", "text": error }],
            "isError": true,
        }));
    } else {
        serde_json::to_string_pretty(answer.get("result").unwrap_or(&Value::Null))
            .unwrap_or_else(|_| "нет ответа".into())
    };

    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

/// Сколько ждать ответа на обычный вызов.
///
/// Чуть больше, чем приложение ждёт решения человека (пять минут): обрезать
/// раньше — значит показать агенту отказ ровно тогда, когда владелец уже
/// тянется к кнопке «подтвердить».
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(330);

/// Сколько ждать то, что по своей природе идёт долго (#213).
///
/// Подтверждённый план выполняет шаги — `docker build`, миграцию, `apt
/// upgrade`; передача файла на сотни мегабайт тоже не укладывается в пять
/// минут. С прежним общим пределом агент получал «приложение не ответило
/// вовремя» посреди работы, которая шла как надо, и не мог знать, чем она
/// кончилась. Это хуже долгого ожидания.
///
/// Совсем без предела зависшее приложение держало бы агента вечно, поэтому
/// час — но час, а не пять минут.
const REPLY_TIMEOUT_LONG: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Сколько ждать ответа на этот метод.
fn reply_timeout(method: &str) -> std::time::Duration {
    match method {
        "hosts.plan" | "hosts.service" | "files.upload" | "files.download" | "snippets.run"
        | "hosts.tunnel" => REPLY_TIMEOUT_LONG,
        _ => REPLY_TIMEOUT,
    }
}

/// Спросить приложение через локальный сокет.
fn ask_app(method: &str, params: Value) -> Result<Value, String> {
    let token = std::fs::read_to_string(token_path()?)
        .map_err(|_| "приложение Darwaza не запущено или доступ агента не настроен".to_string())?
        .trim()
        .to_string();

    let request = json!({
        "token": token,
        "method": method,
        "params": params,
        "client": CLIENT.get(),
        "session": session_id(),
    });
    let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    line.push(b'\n');

    // Это самая частая причина отказа, и объяснить её надо человеческим
    // языком: агент покажет текст пользователю дословно.
    let not_running = || "приложение Darwaza не запущено: откройте его и повторите".to_string();
    let no_answer =
        || "приложение не ответило вовремя: подтвердите план в окне Darwaza".to_string();

    #[cfg(unix)]
    {
        let socket = socket_path()?;
        let mut stream = UnixStream::connect(&socket).map_err(|_| not_running())?;
        let _ = stream.set_read_timeout(Some(reply_timeout(method)));
        stream.write_all(&line).map_err(|e| e.to_string())?;
        stream.flush().map_err(|e| e.to_string())?;

        let mut reader = std::io::BufReader::new(stream);
        let mut answer = String::new();
        reader.read_line(&mut answer).map_err(|_| no_answer())?;
        serde_json::from_str(&answer).map_err(|e| format!("ответ не разбирается: {e}"))
    }

    #[cfg(windows)]
    {
        let pipe = pipe_name(&token);
        let file = open_pipe(&pipe).map_err(|_| not_running())?;
        // Имя канала видят все пользователи машины, и занять его раньше
        // приложения может любой. Токен отправляем, только убедившись, что канал
        // создан процессом ТЕКУЩЕГО пользователя. Не удалось проверить — тоже отказ.
        match winsec::server_is_current_user(&file) {
            Ok(true) => {}
            Ok(false) => {
                return Err("канал агента создан не вами: токен не отправлен. Закройте чужой процесс или перезапустите Darwaza".to_string())
            }
            Err(e) => return Err(format!("не удалось проверить владельца канала агента ({e}): токен не отправлен")),
        }

        // У синхронного чтения из канала нет тайм-аута, поэтому обмен идёт в
        // отдельном потоке, а здесь ждём ответ с пределом. Опоздавший поток
        // доживёт до закрытия канала приложением — это редкий случай.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut file = file;
            let result = file
                .write_all(&line)
                .and_then(|_| file.flush())
                .and_then(|_| {
                    let mut answer = String::new();
                    std::io::BufReader::new(file).read_line(&mut answer)?;
                    Ok(answer)
                });
            let _ = tx.send(result);
        });
        let answer = rx
            .recv_timeout(reply_timeout(method))
            .map_err(|_| no_answer())?
            .map_err(|_| no_answer())?;
        serde_json::from_str(&answer).map_err(|e| format!("ответ не разбирается: {e}"))
    }
}

/// Открыть канал приложения. Если все экземпляры заняты другими клиентами
/// (`ERROR_PIPE_BUSY`, 231), подождать и повторить — приложение поднимает
/// следующий экземпляр сразу, так что ждать приходится доли секунды.
#[cfg(windows)]
fn open_pipe(name: &str) -> std::io::Result<std::fs::File> {
    let mut attempts = 0;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(name) {
            Err(e) if e.raw_os_error() == Some(231) && attempts < 50 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

/// Имя канала на Windows. Повторяет `agent::pipe_name` приложения — по той же
/// причине, что и пути ниже; расхождение ловит тест с общим значением.
/// Имя выводится из токена: постоянное имя можно было бы занять раньше
/// приложения и получить токен от посредника.
#[cfg_attr(not(windows), allow(dead_code))]
fn pipe_name(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("darwaza-agent-pipe:{token}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!(r"\\.\pipe\darwaza-agent-{}", &hex[..32])
}

/// Пути вычисляются так же, как в приложении.
///
/// Повторены здесь, а не взяты из библиотеки, намеренно: посредник должен
/// оставаться маленькой самостоятельной программой, которую агент запускает
/// сотни раз. Тянуть в неё половину приложения ради двух путей — плохой
/// размен.
fn token_path() -> Result<std::path::PathBuf, String> {
    // На Windows токен лежит в `%LOCALAPPDATA%`, а не в Roaming — как в приложении.
    #[cfg(windows)]
    {
        let dirs = directories::ProjectDirs::from("dev", "darwaza", "darwaza")
            .ok_or("не удалось определить каталог приложения")?;
        Ok(dirs.data_local_dir().join("agent-token"))
    }
    #[cfg(not(windows))]
    {
        Ok(state_dir()?.join("agent-token"))
    }
}

#[cfg(unix)]
fn socket_path() -> Result<std::path::PathBuf, String> {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(std::path::PathBuf::from(runtime)
            .join("darwaza")
            .join("agent.sock"));
    }
    Ok(state_dir()?.join("agent.sock"))
}

#[cfg_attr(windows, allow(dead_code))]
fn state_dir() -> Result<std::path::PathBuf, String> {
    let dirs = directories::ProjectDirs::from("dev", "darwaza", "darwaza")
        .ok_or("не удалось определить каталог приложения")?;
    Ok(dirs
        .state_dir()
        .unwrap_or_else(|| dirs.data_dir())
        .to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Имя канала совпадает с тем, что считает приложение (`agent::pipe_name`).
    /// Те же входные данные и тот же ответ проверяются там — расхождение
    /// оставило бы Windows без связи.
    #[test]
    fn pipe_name_matches_the_app() {
        assert_eq!(
            pipe_name("test-token"),
            r"\\.\pipe\darwaza-agent-4b2a71fa45091137a84a869a7ed2f210"
        );
    }

    /// Каждый описанный агенту инструмент проведён в приложение.
    #[test]
    fn every_described_tool_is_routed() {
        let tools = tool_definitions();
        let tools = tools.as_array().expect("список инструментов");
        assert!(!tools.is_empty());
        for tool in tools {
            let name = tool["name"].as_str().expect("имя инструмента");
            assert!(
                method_for(name).is_some(),
                "инструмент {name} описан агенту, но не проведён в приложение"
            );
        }
    }

    /// То, что идёт долго, обязано и ждаться долго (#213).
    ///
    /// Иначе посредник обрывает ожидание посреди работы, которая идёт как
    /// надо, и агент не узнаёт, чем она кончилась. Список коротких и длинных
    /// живёт в двух местах — здесь и в приложении, — и разъехаться им нельзя.
    #[test]
    fn long_running_methods_are_waited_for_longer() {
        for method in [
            "hosts.plan",
            "hosts.service",
            "files.upload",
            "files.download",
        ] {
            assert_eq!(
                reply_timeout(method),
                REPLY_TIMEOUT_LONG,
                "{method} идёт долго: план выполняет шаги, передача гонит байты"
            );
        }
        for method in ["hosts.list", "hosts.run", "files.read", "files.stat"] {
            assert_eq!(reply_timeout(method), REPLY_TIMEOUT, "{method} — быстрый");
        }
        // Предел плана обязан покрывать ожидание решения человека плюс сами
        // шаги, иначе ожидание обрывается на первом же долгом шаге.
        assert!(REPLY_TIMEOUT_LONG > REPLY_TIMEOUT * 2);
    }

    /// Файловые инструменты, которые меняют что-либо, прямо говорят агенту о
    /// подтверждении: иначе он сочтёт долгое ожидание зависанием.
    #[test]
    fn changing_file_tools_say_they_need_confirmation() {
        let tools = tool_definitions();
        for name in [
            "darwaza_download",
            "darwaza_upload",
            "darwaza_mkdir",
            "darwaza_rename",
            "darwaza_delete",
        ] {
            let tool = tools
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("нет инструмента {name}"));
            assert!(
                tool["description"]
                    .as_str()
                    .unwrap()
                    .contains("подтверждения"),
                "{name}: в описании не сказано о подтверждении"
            );
        }
    }
}
