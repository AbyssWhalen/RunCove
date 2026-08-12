import { type ReactNode, useCallback, useEffect, useMemo, useState } from "react";

import {
  I18nContext,
  type I18nContextValue,
  initialPreference,
  LANGUAGE_STORAGE_KEY,
  type LanguagePreference,
  renderMessage,
  resolveLanguage,
} from "./context";
import type { MessageKey, MessageParams } from "./messages";

export function I18nProvider({ children }: { children: ReactNode }) {
  const [preference, setPreferenceState] = useState<LanguagePreference>(initialPreference);
  const language = resolveLanguage(preference);
  const locale = language === "zh-CN" ? "zh-CN" : "en-US";

  const setPreference = useCallback((next: LanguagePreference) => {
    setPreferenceState(next);
    try {
      window.localStorage.setItem(LANGUAGE_STORAGE_KEY, next);
    } catch {
      // The in-memory preference still works when WebView storage is unavailable.
    }
  }, []);

  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);

  const t = useCallback(
    (key: MessageKey, params?: MessageParams) => renderMessage(language, key, params),
    [language],
  );
  const formatDateTime = useCallback(
    (value: number | Date, options: Intl.DateTimeFormatOptions) =>
      new Intl.DateTimeFormat(locale, options).format(new Date(value)),
    [locale],
  );
  const formatTime = useCallback(
    (value: number | Date) =>
      new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(value)),
    [locale],
  );

  const context = useMemo<I18nContextValue>(() => ({
    preference,
    language,
    locale,
    setPreference,
    t,
    formatDateTime,
    formatTime,
  }), [formatDateTime, formatTime, language, locale, preference, setPreference, t]);

  return <I18nContext.Provider value={context}>{children}</I18nContext.Provider>;
}
