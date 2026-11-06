import { i18n } from "../i18n";

/**
 * Текст ошибки для человека (#177).
 *
 * Backend возвращает **код** (`errors.saveFailed`), а не текст: языков шесть,
 * и текст, написанный в Rust, был бы русским во всех шести. Перевод лежит в
 * локалях рядом со всеми остальными надписями.
 *
 * Если код незнакомый — значит, откуда-то ещё прилетела сырая техническая
 * строка. Показывать её человеку нельзя: он всё равно ничего с ней не сделает,
 * а «14 values for 15 columns» выглядит как поломка всего приложения. Такую
 * строку кладём в консоль разработчику и показываем общую фразу.
 */
export function errorText(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  const t = i18n.global.t;

  if (raw.startsWith("errors.")) {
    const text = t(raw);
    // vue-i18n возвращает сам ключ, когда перевода нет — значит код новый,
    // а строку в локали добавить забыли.
    if (text !== raw) return text;
    console.error("[errors] нет перевода для кода", raw);
    return t("errors.unexpected");
  }

  console.error("[errors] нераспознанная ошибка от backend:", raw);
  return t("errors.unexpected");
}
