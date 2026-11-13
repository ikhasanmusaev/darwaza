/**
 * Какая это система.
 *
 * Нужна там, где поведение задаёт не приложение, а привычки системы: Option
 * в терминале и сочетания копирования. Везде, где этой разницы нет, кода
 * с проверкой платформы быть не должно.
 *
 * Смотрим строку агента, а не `navigator.platform`: тот объявлен устаревшим.
 * Плагин ОС в зависимостях не нужен — одна проверка не стоит ещё одного
 * плагина в сборке каждой системы.
 */
export function isMacOS(): boolean {
  return /Mac OS X/.test(navigator.userAgent);
}

/**
 * Нажат ли главный модификатор сочетаний приложения: Cmd на macOS, Ctrl на
 * остальных. На Mac Ctrl остаётся оболочке: Ctrl+W стирает слово, Ctrl+K —
 * строку до конца, Ctrl+F — шаг вперёд, и отнимать их у терминала нельзя.
 */
export function isAppMod(e: KeyboardEvent): boolean {
  return isMacOS() ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
}

/**
 * Подпись сочетания для подсказок: `shortcutLabel("Shift", "D")` — «⇧⌘D» на
 * Mac и «Ctrl+Shift+D» на остальных. Порядок значков на Mac — как в меню
 * системы: ⌥, ⇧, затем ⌘.
 */
export function shortcutLabel(...keys: string[]): string {
  if (!isMacOS()) return ["Ctrl", ...keys].join("+");
  const symbols: Record<string, string> = { Alt: "⌥", Shift: "⇧" };
  const mods = keys.filter((k) => k in symbols).map((k) => symbols[k]);
  const rest = keys.filter((k) => !(k in symbols));
  return [...mods, "⌘", ...rest].join("");
}
