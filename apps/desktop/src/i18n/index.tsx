import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  type ReactNode,
} from "react";
import {
  resolveLocale,
  translate,
  type Locale,
  type MessageKey,
} from "./messages";

export type Translate = (
  key: MessageKey,
  params?: Record<string, string | number>,
) => string;

export interface I18nValue {
  locale: Locale;
  t: Translate;
}

const fallback: I18nValue = {
  locale: "en",
  t: (key, params) => translate("en", key, params),
};

const I18nContext = createContext<I18nValue>(fallback);

export function I18nProvider({
  preference,
  children,
}: {
  preference: string | null | undefined;
  children: ReactNode;
}) {
  const locale = useMemo(() => resolveLocale(preference), [preference]);
  const t = useCallback<Translate>(
    (key, params) => translate(locale, key, params),
    [locale],
  );
  const value = useMemo(() => ({ locale, t }), [locale, t]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  return useContext(I18nContext);
}

export function useT(preference: string | null | undefined): Translate {
  const locale = useMemo(() => resolveLocale(preference), [preference]);
  return useCallback<Translate>(
    (key, params) => translate(locale, key, params),
    [locale],
  );
}

export * from "./messages";
