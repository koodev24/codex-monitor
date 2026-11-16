import { quotaLeft } from "./quota";

export interface RateLimitWindow {
  used_percent?: number;
  limit_window_seconds?: number;
  reset_after_seconds?: number;
  reset_at?: number;
}

export interface ResetCredit {
  reset_type?: string;
  status?: string;
  granted_at?: string;
  expires_at?: string;
}

export interface ResetCreditsPayload {
  available_count?: number;
  total_earned_count?: number;
  credits?: ResetCredit[];
}

export interface AccountUsage {
  reset_ts?: number;
  used_percent?: number;
  primary_window?: RateLimitWindow;
  secondary_window?: RateLimitWindow;
  short_window?: RateLimitWindow;
  weekly_window?: RateLimitWindow;
  last_fetched?: number;
  archived?: boolean;
  resets?: ResetCreditsPayload;
}

export type UsageMap = Record<string, AccountUsage>;

/**
 * Displayed quota rule (pinned by carry-over #27):
 * remaining = round(100 - weekly.used_percent). Integer percent, so the
 * number on screen can never drift from Codex by float formatting.
 * Raw API values are kept in the store for debugging mismatches.
 */
export function formatQuotaLeft(usedPercent?: number): string {
  const left = quotaLeft(usedPercent);
  return left === null ? "-" : `${left}%`;
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

function stamp(ts: number): string {
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function parts(diffSecs: number): string {
  const days = Math.floor(diffSecs / 86400);
  const hours = Math.floor((diffSecs % 86400) / 3600);
  const minutes = Math.floor((diffSecs % 3600) / 60);
  const out: string[] = [];
  if (days > 0) out.push(`${days}d`);
  if (hours > 0) out.push(`${hours}h`);
  out.push(`${minutes}m`);
  return out.join(" ");
}

export function formatResetDisplay(resetTs: number | undefined, nowTs: number): string {
  if (!resetTs) return "-";
  const countdown = nowTs >= resetTs ? "0m" : parts(Math.floor(resetTs - nowTs));
  return `${stamp(resetTs)} (${countdown})`;
}

/**
 * Human countdown, clamped at zero: `3d 9h 7m`, `4h 59m`, `27m`.
 * No zero padding: `03:09:07` reads like a clock time, not a duration.
 */
export function formatResetCountdown(
  resetTs: number | undefined,
  nowTs: number,
): string {
  if (!resetTs) return "-";
  return parts(Math.max(0, Math.floor(resetTs - nowTs)));
}

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/**
 * Human reset stamp. Same calendar day reads as a clock time (`6:26 PM`),
 * otherwise a short date (`18 Nov`), with the year only when it differs.
 * Locale-independent so the menu bar never reflows between systems.
 */
export function formatResetStamp(
  resetTs: number | undefined,
  nowTs: number,
): string {
  if (!resetTs) return "-";
  const d = new Date(resetTs * 1000);
  const now = new Date(nowTs * 1000);
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) {
    const hours = d.getHours();
    const h12 = hours % 12 === 0 ? 12 : hours % 12;
    return `${h12}:${pad(d.getMinutes())} ${hours < 12 ? "AM" : "PM"}`;
  }
  const date = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return d.getFullYear() === now.getFullYear()
    ? date
    : `${date} ${d.getFullYear()}`;
}

export function countdownText(secs: number): string {
  if (secs <= 0) return "expired";
  return parts(secs);
}

export function formatAgo(ts: number | undefined, nowTs: number): string {
  if (!ts) return "Never";
  const diff = Math.max(0, Math.floor(nowTs - ts));
  if (diff < 60) return "Just now";
  const minutes = Math.floor(diff / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function parseIso(value?: string | null): number | undefined {
  if (!value) return undefined;
  const ms = Date.parse(value);
  if (Number.isNaN(ms)) return undefined;
  return ms / 1000;
}

export function formatGrantedAt(grantedAt?: string): string {
  const ts = parseIso(grantedAt);
  return ts === undefined ? "-" : stamp(ts);
}

export function formatCreditExpires(expiresAt: string | undefined, nowTs: number): string {
  const ts = parseIso(expiresAt);
  if (ts === undefined) return "-";
  return `${stamp(ts)} (${countdownText(Math.floor(ts - nowTs))})`;
}

export function formatTimeRemaining(expiresAt: string | undefined, nowTs: number): string {
  const ts = parseIso(expiresAt);
  if (ts === undefined) return "-";
  return countdownText(Math.floor(ts - nowTs));
}

export function soonestExpiringCredit(
  payload?: ResetCreditsPayload | null,
): ResetCredit | undefined {
  const credits = payload?.credits;
  if (!Array.isArray(credits)) return undefined;
  const available = credits.filter((c) => c && c.status === "available");
  if (available.length === 0) return undefined;
  const parsed = available
    .map((c) => ({ c, ts: parseIso(c.expires_at) }))
    .filter((x) => x.ts !== undefined) as { c: ResetCredit; ts: number }[];
  if (parsed.length === 0) return available[0];
  parsed.sort((a, b) => a.ts - b.ts);
  return parsed[0].c;
}

export function weeklyOf(a: AccountUsage): RateLimitWindow | undefined {
  return a.weekly_window ?? a.secondary_window ?? a.primary_window;
}

export function shortOf(a: AccountUsage): RateLimitWindow | undefined {
  return a.short_window;
}

export function hasShortWindow(a: AccountUsage): boolean {
  const w = a.short_window;
  if (!w) return false;
  const hasUsed =
    typeof w.used_percent === "number" && Number.isFinite(w.used_percent);
  return hasUsed || typeof w.reset_at === "number";
}

export function shortResetTsOf(a: AccountUsage): number | undefined {
  return a.short_window?.reset_at;
}

export function resetTsOf(a: AccountUsage): number {
  return weeklyOf(a)?.reset_at ?? a.reset_ts ?? 0;
}

export function usedOf(a: AccountUsage): number | undefined {
  return weeklyOf(a)?.used_percent ?? a.used_percent;
}
