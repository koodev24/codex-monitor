import { describe, expect, it } from "vitest";
import { QUOTA_TONE, quotaLeft, quotaTone } from "./quota";

describe("quotaLeft", () => {
  it("turns used percent into remaining percent, rounded and clamped", () => {
    expect(quotaLeft(0)).toBe(100);
    expect(quotaLeft(42)).toBe(58);
    expect(quotaLeft(100)).toBe(0);
    expect(quotaLeft(110)).toBe(0);
    expect(quotaLeft(-5)).toBe(100);
    expect(quotaLeft(33.4)).toBe(67);
  });

  it("returns null when there is no usable reading", () => {
    expect(quotaLeft(undefined)).toBeNull();
    expect(quotaLeft(null)).toBeNull();
    expect(quotaLeft(Number.NaN)).toBeNull();
    expect(quotaLeft(Number.POSITIVE_INFINITY)).toBeNull();
  });
});

describe("quotaTone", () => {
  it("reserves green for more than half a window left", () => {
    expect(quotaTone(51)).toBe(QUOTA_TONE.high);
    expect(quotaTone(100)).toBe(QUOTA_TONE.high);
  });

  it("uses amber from 21 to 50 percent left", () => {
    expect(quotaTone(50)).toBe(QUOTA_TONE.medium);
    expect(quotaTone(21)).toBe(QUOTA_TONE.medium);
  });

  it("uses red at 20 percent or less", () => {
    expect(quotaTone(20)).toBe(QUOTA_TONE.low);
    expect(quotaTone(5)).toBe(QUOTA_TONE.low);
    expect(quotaTone(0)).toBe(QUOTA_TONE.low);
  });

  it("uses a muted tone without a reading", () => {
    expect(quotaTone(null)).toBe(QUOTA_TONE.unknown);
  });
});
