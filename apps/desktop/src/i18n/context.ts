import { createContext, useContext } from "react";

import { enMessages, type MessageKey, type MessageParams, zhCnMessages } from "./messages";

export type LanguagePreference = "system" | "en" | "zh-CN";
export type ResolvedLanguage = "en" | "zh-CN";

export const LANGUAGE_STORAGE_KEY = "runcove.language";

function systemLanguage(): ResolvedLanguage {
  if (typeof navigator === "undefined") return "en";
  const preferredLanguage = navigator.language || navigator.languages[0] || "en";
  return preferredLanguage.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

export function isLanguagePreference(value: string | null): value is LanguagePreference {
  return value === "system" || value === "en" || value === "zh-CN";
}

export function initialPreference(): LanguagePreference {
  if (typeof window === "undefined") return "system";
  try {
    const stored = window.localStorage.getItem(LANGUAGE_STORAGE_KEY);
    return isLanguagePreference(stored) ? stored : "system";
  } catch {
    return "system";
  }
}

export function resolveLanguage(preference: LanguagePreference): ResolvedLanguage {
  return preference === "system" ? systemLanguage() : preference;
}

export function renderMessage(language: ResolvedLanguage, key: MessageKey, params: MessageParams = {}): string {
  const message = language === "zh-CN" ? zhCnMessages[key] : enMessages[key];
  return typeof message === "function" ? message(params) : message;
}

export interface I18nContextValue {
  preference: LanguagePreference;
  language: ResolvedLanguage;
  locale: "en-US" | "zh-CN";
  setPreference: (preference: LanguagePreference) => void;
  t: (key: MessageKey, params?: MessageParams) => string;
  formatDateTime: (value: number | Date, options: Intl.DateTimeFormatOptions) => string;
  formatTime: (value: number | Date) => string;
}

const fallbackContext: I18nContextValue = {
  preference: "en",
  language: "en",
  locale: "en-US",
  setPreference: () => undefined,
  t: (key, params) => renderMessage("en", key, params),
  formatDateTime: (value, options) => new Intl.DateTimeFormat("en-US", options).format(new Date(value)),
  formatTime: (value) => new Intl.DateTimeFormat("en-US", { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(value)),
};

export const I18nContext = createContext<I18nContextValue>(fallbackContext);

export function useI18n(): I18nContextValue {
  return useContext(I18nContext);
}
