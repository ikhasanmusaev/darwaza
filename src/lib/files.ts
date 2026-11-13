/**
 * Общие мелочи файловых панелей: боковой в терминале и экрана «Файлы».
 */

/** Скрытый по обычаю unix: имя с точки. */
export function isHidden(name: string): boolean {
  return name.startsWith(".");
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Права в привычном виде `rwxr-xr-x`. Нет атрибута — пусто. */
export function permString(mode: number | null | undefined): string {
  if (mode === null || mode === undefined) return "";
  const bits = "rwxrwxrwx";
  let out = "";
  for (let i = 0; i < 9; i++) {
    out += mode & (1 << (8 - i)) ? bits[i] : "-";
  }
  return out;
}

/**
 * Дата изменения: в этом году — день и время, раньше — день и год. Так
 * пишет `ls -l`, и так видно и вчерашнюю правку, и старый файл.
 */
export function formatModified(seconds: number | null | undefined, locale: string): string {
  if (seconds === null || seconds === undefined) return "";
  const date = new Date(seconds * 1000);
  const thisYear = date.getFullYear() === new Date().getFullYear();
  const options: Intl.DateTimeFormatOptions = thisYear
    ? { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }
    : { day: "numeric", month: "short", year: "numeric" };
  try {
    return new Intl.DateTimeFormat(locale, options).format(date);
  } catch {
    return date.toISOString().slice(0, 10);
  }
}
