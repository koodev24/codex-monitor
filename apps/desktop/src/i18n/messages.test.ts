import { describe, expect, it } from "vitest";
import {
  isLocale,
  LOCALES,
  resolveLocale,
  translate,
  translateFor,
} from "./messages";

describe("locale resolution", () => {
  it("uses an explicit shipped locale", () => {
    expect(resolveLocale("ja", "en-US")).toBe("ja");
    expect(resolveLocale("zh-TW", "en-US")).toBe("zh-TW");
  });

  it("follows the system language when the preference is system or unknown", () => {
    expect(resolveLocale("system", "zh-Hans-CN")).toBe("zh-CN");
    expect(resolveLocale("system", "zh-TW")).toBe("zh-TW");
    expect(resolveLocale("system", "zh-HK")).toBe("zh-TW");
    expect(resolveLocale("system", "ja-JP")).toBe("ja");
    expect(resolveLocale("system", "fr-FR")).toBe("en");
    expect(resolveLocale("klingon", "ja-JP")).toBe("ja");
  });

  it("normalizes underscore system tags", () => {
    expect(resolveLocale("system", "zh_TW")).toBe("zh-TW");
  });

  it("recognizes only shipped locales", () => {
    for (const locale of LOCALES) expect(isLocale(locale)).toBe(true);
    expect(isLocale("system")).toBe(false);
    expect(isLocale(undefined)).toBe(false);
  });
});

describe("translation lookup", () => {
  it("interpolates named parameters", () => {
    expect(translate("en", "status.fetchingAccount", { email: "a@b.c" })).toBe(
      "Fetching quota for a@b.c…",
    );
  });

  it("keeps unknown placeholders intact", () => {
    expect(translate("en", "status.fetchingAccount", {})).toBe(
      "Fetching quota for {email}…",
    );
  });

  it("uses the stored preference for non-React callers", () => {
    expect(translateFor("ja", "common.loading")).toBe("読み込み中…");
    expect(translateFor(null, "common.loading", undefined)).toBe("Loading…");
  });
});
