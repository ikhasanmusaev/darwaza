/**
 * Подстановка переменных в команду сниппета (#105).
 *
 * Поддерживаются плейсхолдеры `{{host}}`, `{{user}}`, `{{port}}`, `{{name}}` —
 * значения берутся из хоста активной сессии. Пробелы внутри скобок допустимы:
 * `{{ host }}` === `{{host}}`.
 *
 * Неизвестный плейсхолдер оставляется в тексте **как есть**, а не заменяется на
 * пустую строку: молча съеденная переменная превратила бы `rm -rf {{dir}}/tmp`
 * в `rm -rf /tmp` — команду с совершенно другим смыслом. Видимый `{{dir}}`
 * гарантированно упадёт в шелле, и пользователь заметит ошибку.
 */
export interface SnippetContext {
  host: string;
  user: string;
  port: number;
  /** Отображаемое имя хоста в приложении (не hostname). */
  name: string;
}

const PLACEHOLDER = /\{\{\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*\}\}/g;

export function expandSnippet(
  command: string,
  ctx: SnippetContext | null,
): string {
  if (!ctx) return command;
  return command.replace(PLACEHOLDER, (whole, key: string) => {
    switch (key) {
      case "host":
        return ctx.host;
      case "user":
        return ctx.user;
      case "port":
        return String(ctx.port);
      case "name":
        return ctx.name;
      default:
        return whole;
    }
  });
}

/** Список переменных для подсказки в UI. */
export const SNIPPET_VARS = ["host", "user", "port", "name"] as const;
