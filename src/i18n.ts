import { createI18n } from "vue-i18n";
import uz from "./locales/uz";
import ru from "./locales/ru";
import en from "./locales/en";
import kk from "./locales/kk";
import ky from "./locales/ky";
import tr from "./locales/tr";

/**
 * Коды языков — ISO 639-1. Для киргизского это `ky`: `kg` в TASKS.md
 * это код страны (Кыргызстан), а не языка.
 */
export type Locale = "uz" | "ru" | "en" | "kk" | "ky" | "tr";

const stored = localStorage.getItem("locale") as Locale | null;
const defaultLocale: Locale = stored ?? "uz";

export const i18n = createI18n({
  legacy: false,
  locale: defaultLocale,
  fallbackLocale: "en",
  messages: { uz, ru, en, kk, ky, tr },
});

export function setLocale(locale: Locale) {
  i18n.global.locale.value = locale;
  localStorage.setItem("locale", locale);
}
