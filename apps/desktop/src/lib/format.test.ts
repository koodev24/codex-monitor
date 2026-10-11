import { describe, expect, it } from "vitest";
import {
  countdownText,
  formatCreditExpires,
  formatGrantedAt,
  formatQuotaLeft,
  formatResetDisplay,
  formatTimeRemaining,
  hasShortWindow,
  parseIso,
  resetTsOf,
  shortResetTsOf,
  soonestExpiringCredit,
  usedOf,
  weeklyOf,
  type AccountUsage,
} from "./format";

const weekly = (used_percent: number, reset_at = 1700000000) => ({
  used_percent,
  reset_at,
});

describe("formatQuotaLeft", () => {
  it("shows remaining as a rounded integer percent", () => {
    expect(formatQuotaLeft(25)).toBe("75%");
    expect(formatQuotaLeft(99.6)).toBe("0%");
    expect(formatQuotaLeft(0)).toBe("100%");
  });

  it("renders a dash when the value is missing", () => {
    expect(formatQuotaLeft(undefined)).toBe("-");
    expect(formatQuotaLeft(Number.NaN)).toBe("-");
  });
});

describe("weekly window selection", () => {
  it("prefers weekly over secondary over primary", () => {
    const a: AccountUsage = {
      primary_window: weekly(10),
      secondary_window: weekly(20),
      weekly_window: weekly(30),
    };
    expect(weeklyOf(a)?.used_percent).toBe(30);
  });

  it("falls back through secondary to primary", () => {
    expect(weeklyOf({ secondary_window: weekly(20) })?.used_percent).toBe(20);
    expect(weeklyOf({ primary_window: weekly(10) })?.used_percent).toBe(10);
    expect(weeklyOf({})).toBeUndefined();
  });

  it("derives used percent and reset timestamp from the weekly window", () => {
    const a: AccountUsage = { weekly_window: weekly(35, 1700000100), used_percent: 99 };
    expect(usedOf(a)).toBe(35);
    expect(resetTsOf(a)).toBe(1700000100);
  });

  it("falls back to legacy top-level fields", () => {
    expect(usedOf({ used_percent: 42 })).toBe(42);
    expect(resetTsOf({ reset_ts: 123 })).toBe(123);
    expect(resetTsOf({})).toBe(0);
  });
});

describe("short (5h) window", () => {
  it("detects a short window by used percent or reset time", () => {
    expect(hasShortWindow({ short_window: weekly(50) })).toBe(true);
    expect(hasShortWindow({ short_window: { reset_at: 1700000000 } })).toBe(true);
    expect(hasShortWindow({})).toBe(false);
    expect(hasShortWindow({ short_window: {} })).toBe(false);
    expect(hasShortWindow({ short_window: { used_percent: Number.NaN } })).toBe(false);
  });

  it("reads the short reset timestamp", () => {
    expect(shortResetTsOf({ short_window: weekly(10, 1700000200) })).toBe(1700000200);
    expect(shortResetTsOf({})).toBeUndefined();
  });
});

describe("reset display", () => {
  it("stamps the reset time with a countdown", () => {
    const text = formatResetDisplay(1700000000, 1700000000 - 90 * 60);
    expect(text).toContain("1h 30m");
  });

  it("clamps past resets to zero", () => {
    expect(formatResetDisplay(100, 200)).toContain("0m");
  });

  it("renders a dash without a timestamp", () => {
    expect(formatResetDisplay(undefined, 0)).toBe("-");
    expect(formatResetDisplay(0, 0)).toBe("-");
  });

  it("counts down credit expiry in parts", () => {
    expect(countdownText(0)).toBe("expired");
    expect(countdownText(-5)).toBe("expired");
    expect(countdownText(90000)).toContain("1d");
  });
});

describe("iso helpers", () => {
  it("parses iso timestamps to seconds", () => {
    expect(parseIso("2026-04-24T15:17:00.949Z")).toBeCloseTo(1777043820.949, 2);
    expect(parseIso(undefined)).toBeUndefined();
    expect(parseIso("not-a-date")).toBeUndefined();
  });

  it("formats grant and expiry displays", () => {
    expect(formatGrantedAt(undefined)).toBe("-");
    expect(formatGrantedAt("not-a-date")).toBe("-");
    expect(formatGrantedAt("2026-04-24T15:17:00.949Z")).toContain("2026-04-24");
    expect(formatCreditExpires(undefined, 0)).toBe("-");
    expect(formatTimeRemaining(undefined, 0)).toBe("-");
  });
});

describe("soonestExpiringCredit", () => {
  it("picks the soonest available credit", () => {
    const payload = {
      credits: [
        { status: "available", expires_at: "2026-05-02T00:00:00Z" },
        { status: "available", expires_at: "2026-05-01T00:00:00Z" },
        { status: "used", expires_at: "2026-04-01T00:00:00Z" },
      ],
    };
    expect(soonestExpiringCredit(payload)?.expires_at).toBe("2026-05-01T00:00:00Z");
  });

  it("returns nothing without available credits", () => {
    expect(soonestExpiringCredit({ credits: [] })).toBeUndefined();
    expect(soonestExpiringCredit(null)).toBeUndefined();
  });
});
