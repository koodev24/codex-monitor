import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import ReactDOM from "react-dom/client";
import {
  AlertTriangle,
  Check,
  ChevronLeft,
  Gauge,
  RefreshCw,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { QuotaMeter } from "@/components/QuotaMeter";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { I18nProvider, useI18n, useT } from "@/i18n";
import { api, type CodexStatus, type Snapshot, type SwitchResult } from "@/lib/tauri";
import {
  hasShortWindow,
  resetTsOf,
  shortResetTsOf,
  weeklyOf,
  type AccountUsage,
} from "@/lib/format";
import { cn } from "@/lib/utils";
import "./index.css";

document.documentElement.dataset.window = "tray";

function useTrayTheme() {
  useEffect(() => {
    const apply = () => {
      let dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
      try {
        const saved = localStorage.getItem("cm-theme");
        if (saved) dark = saved === "dark";
      } catch {
        // Use the system theme.
      }
      document.documentElement.classList.toggle("dark", dark);
    };
    apply();
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", apply);
    window.addEventListener("storage", apply);
    return () => {
      media.removeEventListener("change", apply);
      window.removeEventListener("storage", apply);
    };
  }, []);
}

function Spin() {
  return (
    <span className="relative inline-flex size-3.5 shrink-0" aria-hidden="true">
      <span className="absolute inset-0 rounded-full border-2 border-current/25" />
      <span className="absolute inset-0 animate-spin rounded-full border-2 border-transparent border-t-current" />
    </span>
  );
}

function StatusDot({ active }: { active: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "size-1.5 shrink-0 rounded-full",
        active ? "bg-emerald-500" : "bg-muted-foreground/40",
      )}
    />
  );
}

interface QuotaLine {
  label: string;
  used?: number | null;
  resetTs?: number;
}

/** One window per line, 5h first: `5h·bar·42%·Reset in 3d 9h 7m·18 Nov`. */
function quotaLinesOf(account: AccountUsage | undefined): QuotaLine[] {
  if (!account) return [];
  const lines: QuotaLine[] = [];
  if (hasShortWindow(account)) {
    lines.push({
      label: "5h",
      used: account.short_window?.used_percent,
      resetTs: shortResetTsOf(account),
    });
  }
  const weekly = weeklyOf(account);
  if (weekly) {
    lines.push({
      label: "7d",
      used: weekly.used_percent,
      resetTs: resetTsOf(account) || undefined,
    });
  }
  return lines;
}

function QuotaLines({ account, now }: { account: AccountUsage | undefined; now: number }) {
  const { t } = useI18n();
  const lines = quotaLinesOf(account);
  if (lines.length === 0) {
    return (
      <span className="block truncate text-[9px] tabular-nums text-muted-foreground">
        {t("tray.notFetched")}
      </span>
    );
  }
  return (
    <span className="flex flex-col gap-1">
      {lines.map((line) => (
        <QuotaMeter key={line.label} compact {...line} now={now} />
      ))}
    </span>
  );
}

function TrayApp() {
  useTrayTheme();
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const t = useT(snap?.locale);
  const [codex, setCodex] = useState<CodexStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [confirmStatus, setConfirmStatus] = useState<CodexStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [stage, setStage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<SwitchResult | null>(null);
  const [nowSecs, setNowSecs] = useState(() => Date.now() / 1000);

  useEffect(() => {
    const timer = window.setInterval(() => setNowSecs(Date.now() / 1000), 1000);
    return () => window.clearInterval(timer);
  }, []);

  async function load() {
    try {
      const next = await api.snapshot();
      setSnap(next);
      setError(null);
    } catch (e) {
      setError(`${e}`);
    } finally {
      setLoading(false);
    }
    void api
      .codexStatus()
      .then(setCodex)
      .catch(() => undefined);
  }

  // The refresh button used to only re-read local state, which looked like a
  // no-op next to stale percentages. It now pulls fresh usage from the API for
  // every visible account and reports which ones failed.
  async function fetchQuota() {
    if (refreshing || busy || pending) return;
    setRefreshing(true);
    setNotice(null);
    setActionError(null);
    let failures: string[] = [];
    try {
      const current = await api.snapshot();
      const emails = Object.entries(current.accounts)
        .filter(([, account]) => !account.archived)
        .map(([email]) => email);
      const results = await Promise.allSettled(
        emails.map((email) =>
          email === current.current_email
            ? api.manualFetch()
            : api.fetchBackup(email),
        ),
      );
      failures = results.flatMap((result, index) => {
        if (result.status !== "rejected") return [];
        const raw = `${result.reason}`;
        const prefix = `NO_BACKUP ${emails[index]}: `;
        return `${emails[index]}: ${raw.startsWith(prefix) ? raw.slice(prefix.length) : raw}`;
      });
    } catch (e) {
      failures = [`${e}`];
    } finally {
      await load();
      setRefreshing(false);
    }
    if (failures.length > 0) {
      setError(t("tray.refreshFailed", { errors: failures.join("; ") }));
    }
  }

  useEffect(() => {
    void load();
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      void p.then((u) => {
        if (cancelled) u();
        else unlisteners.push(u);
      });
    };
    track(listen("tray-refresh", () => void load()));
    track(listen("auth-file-changed", () => void load()));
    track(listen("switch-completed", () => void load()));
    track(listen<string>("switch-stage", (event) => setStage(event.payload)));
    const onFocus = () => void load();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") void api.trayHide();
    };
    window.addEventListener("focus", onFocus);
    window.addEventListener("keydown", onKey);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("keydown", onKey);
      unlisteners.forEach((u) => u());
    };
  }, []);

  useEffect(() => {
    if (!pending) {
      setConfirmStatus(null);
      return;
    }
    let active = true;
    void api
      .codexStatus()
      .then((next) => {
        if (active) setConfirmStatus(next);
      })
      .catch(() => {
        if (active) setConfirmStatus(null);
      });
    return () => {
      active = false;
    };
  }, [pending]);

  const now = nowSecs;
  const activeEmail = snap?.current_email ?? null;
  const activeUsage = activeEmail && snap ? snap.accounts[activeEmail] : undefined;

  const accounts = useMemo(() => {
    if (!snap) return [];
    return Object.entries(snap.accounts)
      .filter(([, account]) => !account.archived)
      .sort(([ea, a], [eb, b]) => {
        if (ea === snap.current_email) return -1;
        if (eb === snap.current_email) return 1;
        return resetTsOf(a) - resetTsOf(b);
      });
  }, [snap]);

  const codexLabel = !codex
    ? t("codex.checking")
    : !codex.supported
      ? t("codex.lifecycleUnavailable")
      : !codex.installed
        ? t("codex.notInstalled")
        : codex.running
          ? codex.process_count > 1
            ? t("codex.runningWithCount", { count: codex.process_count })
            : t("codex.running")
          : t("codex.closed");
  const confirmText = !confirmStatus
    ? t("switch.checking")
    : !confirmStatus.supported
      ? t("switch.lifecycleUnavailable")
      : !confirmStatus.installed
        ? t("switch.notFound")
        : confirmStatus.running
          ? confirmStatus.process_count === 1
            ? t("switch.runningOne")
            : t("switch.runningMany", { count: confirmStatus.process_count })
          : t("switch.notRunning");

  function beginSwitch(email: string) {
    setPending(email);
    setActionError(null);
    setNotice(null);
    setConfirmStatus(null);
  }

  async function doSwitch() {
    if (!pending || busy) return;
    setBusy(true);
    setStage(null);
    setActionError(null);
    try {
      const result = await api.switchAccount(pending);
      setNotice(result);
      setPending(null);
      await load();
    } catch (e) {
      setActionError(`${e}`);
    } finally {
      setBusy(false);
      setStage(null);
    }
  }

  return (
    <I18nProvider preference={snap?.locale}>
      <TooltipProvider delay={150}>
      <div className="flex h-screen w-screen flex-col overflow-hidden rounded-2xl border border-black/10 bg-popover/95 text-popover-foreground shadow-2xl ring-1 ring-black/5 backdrop-blur-2xl dark:border-white/10 dark:ring-white/5">
        <header className="flex shrink-0 items-center gap-2 border-b px-3.5 py-2.5">
          <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-primary/10 text-primary">
            <Gauge className="size-4" aria-hidden />
          </span>
          <div className="min-w-0 flex-1 leading-tight">
            <p className="text-xs font-semibold tracking-[-0.01em]">Codex Monitor</p>
            <p className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
              <StatusDot active={!!codex?.running} />
              <span className="truncate">{codexLabel}</span>
            </p>
          </div>
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  size="icon-xs"
                  variant="ghost"
                  onClick={() => void fetchQuota()}
                  disabled={refreshing || busy || pending !== null}
                  aria-label={t("tray.fetchQuotaAria")}
                >
                  <RefreshCw className={cn(refreshing && "animate-spin")} />
                </Button>
              }
            />
            <TooltipContent
              side="bottom"
              sideOffset={8}
              align="end"
              className="max-w-[16rem] whitespace-normal"
            >
              <span className="flex flex-col gap-0.5">
                <span className="font-medium">{t("tray.fetchQuota")}</span>
                <span className="opacity-70">{t("tray.fetchQuotaDetail")}</span>
              </span>
            </TooltipContent>
          </Tooltip>
        </header>

      {pending ? (
        <div className="flex min-h-0 flex-1 flex-col gap-3 p-3.5">
          <button
            type="button"
            className="flex w-fit items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
            onClick={() => {
              if (!busy) {
                setPending(null);
                setActionError(null);
              }
            }}
            disabled={busy}
          >
            <ChevronLeft className="size-3.5" aria-hidden />
            {t("action.back")}
          </button>
          <div className="rounded-xl border bg-card p-3">
            <p className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
              {t("tray.switchTo")}
            </p>
            <Tooltip>
              <TooltipTrigger
                render={
                  <p className="mt-0.5 truncate text-sm font-semibold" tabIndex={0}>
                    {pending}
                  </p>
                }
              />
              <TooltipContent>{pending}</TooltipContent>
            </Tooltip>
            <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">
              {confirmText}
            </p>
          </div>
          {actionError && (
            <p className="rounded-lg border border-destructive/25 bg-destructive/10 px-2.5 py-2 text-[11px] leading-relaxed break-words text-destructive [overflow-wrap:anywhere]">
              {actionError}
            </p>
          )}
          <div className="mt-auto grid grid-cols-2 gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setPending(null);
                setActionError(null);
              }}
              disabled={busy}
            >
              {t("action.cancel")}
            </Button>
            <Button size="sm" onClick={() => void doSwitch()} disabled={busy}>
              {busy ? (
                <>
                  <Spin />{" "}
                  {stage === "closing"
                    ? t("switch.closing")
                    : stage === "reopening"
                      ? t("switch.reopening")
                      : t("switch.switching")}
                </>
              ) : confirmStatus?.running ? (
                t("tray.switchAndReopen")
              ) : (
                t("tray.switchAccount")
              )}
            </Button>
          </div>
        </div>
      ) : (
        <>
          <div className="shrink-0 p-3.5 pb-2">
            <div className="rounded-xl border bg-card p-3">
              <p className="text-[9px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                {t("app.activeAccount")}
              </p>
              {activeEmail ? (
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <p className="truncate text-xs font-medium" tabIndex={0}>
                        {activeEmail}
                      </p>
                    }
                  />
                  <TooltipContent>{activeEmail}</TooltipContent>
                </Tooltip>
              ) : (
                <p className="truncate text-xs font-medium">{t("tray.noActiveSession")}</p>
              )}
              <div className="mt-1.5">
                {activeUsage ? (
                  <QuotaLines account={activeUsage} now={now} />
                ) : (
                  <span className="block text-[9px] text-muted-foreground">
                    {t("tray.fetchQuotaToSee")}
                  </span>
                )}
              </div>
            </div>
          </div>

          {notice && (
            <div
              className={cn(
                "mx-3.5 mb-2 flex items-start gap-2 rounded-lg border px-2.5 py-2 text-[10px] leading-relaxed",
                notice.warning
                  ? "border-amber-500/30 bg-amber-500/10 text-amber-900 dark:text-amber-200"
                  : "border-emerald-500/25 bg-emerald-500/10 text-emerald-800 dark:text-emerald-200",
              )}
            >
              {notice.warning ? (
                <AlertTriangle className="mt-0.5 size-3 shrink-0" aria-hidden />
              ) : (
                <Check className="mt-0.5 size-3 shrink-0" aria-hidden />
              )}
              <p className="min-w-0 flex-1 break-words [overflow-wrap:anywhere]">
                {notice.warning ?? notice.message}
              </p>
              <button
                type="button"
                className="shrink-0 opacity-70 hover:opacity-100"
                onClick={() => setNotice(null)}
                aria-label={t("action.dismiss")}
              >
                <X className="size-3" />
              </button>
            </div>
          )}

          {error && (
            <div className="mx-3.5 mb-2 max-h-20 overflow-y-auto rounded-lg border border-destructive/25 bg-destructive/10 px-2.5 py-2 text-[10px] leading-relaxed break-words text-destructive [overflow-wrap:anywhere]">
              {error}
            </div>
          )}

          <div className="flex shrink-0 items-center justify-between px-4 pb-1">
            <p className="text-[9px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
              {t("tray.accounts")}
            </p>
            <span className="text-[9px] tabular-nums text-muted-foreground">
              {accounts.length}
            </span>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-1">
            {loading ? (
              <div className="flex items-center justify-center gap-2 py-8 text-[11px] text-muted-foreground">
                <Spin /> {t("common.loading")}
              </div>
            ) : accounts.length === 0 ? (
              <p className="px-2 py-6 text-center text-[11px] text-muted-foreground">
                {t("tray.noAccounts")}
              </p>
            ) : (
              <div className="flex flex-col gap-0.5">
                {accounts.map(([email, account]) => {
                  const current = email === snap?.current_email;
                  return (
                    <button
                      key={email}
                      type="button"
                      onClick={() => {
                        if (!current && !refreshing && !busy) beginSwitch(email);
                      }}
                      disabled={current || refreshing || busy}
                      className={cn(
                        "flex w-full items-start rounded-lg px-2 py-1.5 text-left transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
                        current
                          ? "bg-primary/10"
                          : "hover:bg-muted disabled:opacity-100",
                      )}
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[11px] font-medium">{email}</span>
                        <span className="mt-1 block">
                          <QuotaLines account={account} now={now} />
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          <footer className="flex shrink-0 items-center gap-1.5 border-t px-3 py-2">
            <Button
              size="sm"
              className="flex-1"
              onClick={() => void api.trayShowMain()}
            >
              {t("tray.openDashboard")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void api.quitApp()}
            >
              {t("tray.quit")}
            </Button>
          </footer>
        </>
      )}
      </div>
      </TooltipProvider>
    </I18nProvider>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<TrayApp />);
