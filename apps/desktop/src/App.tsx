import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  Archive,
  ArchiveRestore,
  ArrowDown,
  ArrowDownToLine,
  ArrowLeftRight,
  ArrowUp,
  Check,
  ChevronDown,
  ChevronUp,
  CloudDownload,
  Coins,
  Copy,
  Download,
  Eraser,
  Eye,
  EyeOff,
  Gauge,
  Moon,
  RefreshCw,
  RotateCw,
  Settings,
  ShieldCheck,
  Sun,
  Trash2,
  Upload,
  UserPlus,
  X,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  formatAgo,
  formatCreditExpires,
  formatGrantedAt,
  formatQuotaLeft,
  formatResetDisplay,
  formatTimeRemaining,
  hasShortWindow,
  resetTsOf,
  shortResetTsOf,
  soonestExpiringCredit,
  usedOf,
  weeklyOf,
  type AccountUsage,
} from "./lib/format";
import { AddAccountDialog } from "./components/AddAccountDialog";
import { QuotaMeter } from "./components/QuotaMeter";
import { SettingsDialog } from "./components/SettingsDialog";
import { api, type CodexStatus } from "./lib/tauri";
import { cn } from "./lib/utils";
import { useAppDispatch, useAppSelector } from "./store/hooks";
import {
  appendLines,
  markUrlOpened,
  resetLines,
  setDone,
  setError as setLoginError,
  setOpen as setLoginOpen,
  setUrl,
} from "./store/loginSlice";
import {
  clearLogs,
  refreshLogs,
  setOpen as setLogsOpen,
  toggleLogs,
} from "./store/logsSlice";
import {
  cycleSort,
  doExport,
  doImport,
  fetchAllAccounts,
  loadSnapshot,
  manualFetch,
  openResets,
  pollOnce,
  runConfirmAction,
  saveAutoFetchValue,
  setLoadError,
  toggleArchivedVisibility,
} from "./store/snapshotSlice";
import {
  checkUpdates,
  installUpdate,
  selectAnyBusy,
  selectBusy,
  setBusy,
  setConfirm,
  setResetsView,
  setSettingsOpen,
  setStatus,
  setSwitchInfo,
} from "./store/uiSlice";

function useTheme() {
  const [dark, setDark] = useState(() => {
    try {
      const saved = localStorage.getItem("cm-theme");
      if (saved) return saved === "dark";
    } catch {
      // Use the system theme when webview storage is unavailable.
    }
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  });
  useEffect(() => {
    const root = document.documentElement;
    root.classList.add("disable-transitions");
    root.classList.toggle("dark", dark);
    try {
      localStorage.setItem("cm-theme", dark ? "dark" : "light");
    } catch {
      // Keep theme switching available for this session without storage.
    }
    const frame = window.requestAnimationFrame(() => {
      void root.offsetHeight;
      root.classList.remove("disable-transitions");
    });
    return () => window.cancelAnimationFrame(frame);
  }, [dark]);
  return { dark, toggle: () => setDark((d) => !d) };
}

function Spin() {
  return (
    <span className="relative inline-flex size-4 shrink-0" aria-hidden="true">
      <span className="absolute inset-0 rounded-full border-2 border-current/25" />
      <span className="absolute inset-0 animate-spin rounded-full border-2 border-transparent border-t-current" />
    </span>
  );
}

function IconBtn({
  title,
  onClick,
  busyKey,
  variant = "ghost",
  pressed,
  children,
}: {
  title: string;
  onClick: () => void;
  busyKey?: string;
  variant?: "ghost" | "outline" | "default" | "destructive" | "secondary";
  pressed?: boolean;
  children: React.ReactNode;
}) {
  const busy = useAppSelector((s) => (busyKey ? !!s.ui.busy[busyKey] : false));
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant={pressed ? "secondary" : variant}
            size="icon-sm"
            onClick={onClick}
            disabled={busy}
            aria-label={busy ? `${title} (working…)` : title}
            aria-pressed={pressed}
          >
            {busy ? <Spin /> : children}
          </Button>
        }
      />
      <TooltipContent>{busy ? `${title} (working…)` : title}</TooltipContent>
    </Tooltip>
  );
}

function StatusDot({ active, pulse = false }: { active: boolean; pulse?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "size-1.5 shrink-0 rounded-full",
        active ? "bg-emerald-500" : "bg-muted-foreground/40",
        pulse && active && "animate-pulse",
      )}
    />
  );
}

function SwitchPlan({
  running,
  processCount,
  stage,
  busy,
}: {
  running: boolean;
  processCount: number;
  stage: string | null;
  busy: boolean;
}) {
  const steps = running
    ? [
        {
          key: "closing",
          title: "Close Codex",
          detail: `${processCount} running process${processCount === 1 ? " quits" : "es quit"} cleanly, helpers included.`,
        },
        {
          key: "switching",
          title: "Write the saved session",
          detail: "The chosen account becomes the active Codex sign-in.",
        },
        {
          key: "reopening",
          title: "Reopen Codex",
          detail: "The same app bundle starts again on the new account.",
        },
      ]
    : [
        {
          key: "switching",
          title: "Write the saved session",
          detail: "Codex is closed, so the switch applies now and on the next launch.",
        },
      ];
  const done = stage === "done" || stage === "reopened";
  const current = done ? steps.length : steps.findIndex((step) => step.key === stage);
  return (
    <ol className="flex flex-col gap-2.5">
      {steps.map((step, index) => {
        const complete = done || (current >= 0 && index < current);
        const active = busy && !done && (current === index || (current < 0 && index === 0));
        return (
          <li key={step.key} className="flex items-start gap-2.5">
            <span
              aria-hidden="true"
              className={cn(
                "mt-0.5 grid size-5 shrink-0 place-items-center rounded-full border text-[10px] font-semibold tabular-nums transition-colors",
                complete
                  ? "border-primary/30 bg-primary/15 text-primary"
                  : active
                    ? "border-primary bg-primary text-primary-foreground"
                    : "border-border bg-muted text-muted-foreground",
              )}
            >
              {complete ? <Check className="size-3" /> : active ? <Spin /> : index + 1}
            </span>
            <span className="min-w-0 flex-1 leading-snug">
              <span
                className={cn(
                  "block text-xs font-medium",
                  complete || active ? "text-foreground" : "text-muted-foreground",
                )}
              >
                {step.title}
              </span>
              <span className="block text-[11px] leading-relaxed text-muted-foreground">
                {step.detail}
              </span>
            </span>
          </li>
        );
      })}
    </ol>
  );
}

function Stat({
  label,
  value,
  title,
}: {
  label: string;
  value: string;
  title?: string;
}) {
  const body = (
    <div
      className="min-w-0 rounded-lg border bg-muted/30 px-3 py-2"
      tabIndex={title ? 0 : undefined}
    >
      <p className="text-[10px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
        {label}
      </p>
      <p className="mt-0.5 truncate text-xs font-medium tabular-nums">{value}</p>
    </div>
  );
  if (!title) return body;
  return (
    <Tooltip>
      <TooltipTrigger render={body} />
      <TooltipContent>{title}</TooltipContent>
    </Tooltip>
  );
}

function SortButton({
  label,
  active,
  asc,
  onClick,
}: {
  label: string;
  active: boolean;
  asc: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex h-6 items-center gap-1 rounded-md px-2 text-[11px] font-medium transition-colors",
        active
          ? "bg-card text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
      aria-pressed={active}
    >
      {label}
      {active &&
        (asc ? (
          <ArrowUp className="size-3" aria-hidden />
        ) : (
          <ArrowDown className="size-3" aria-hidden />
        ))}
    </button>
  );
}

function AccountRow({
  email,
  account,
  current,
  now,
  failed,
  onCopy,
  onFetch,
  onResets,
  onSwitch,
  onArchive,
  onRemove,
}: {
  email: string;
  account: AccountUsage;
  current: boolean;
  now: number;
  failed?: string;
  onCopy: () => void;
  onFetch: () => void;
  onResets: () => void;
  onSwitch: () => void;
  onArchive: () => void;
  onRemove: () => void;
}) {
  const weekly = weeklyOf(account);
  const weeklyUsed = usedOf(account);
  const weeklyLeft = formatQuotaLeft(weeklyUsed);
  const hasShort = hasShortWindow(account);
  const shortLeft = formatQuotaLeft(account.short_window?.used_percent);
  const shortReset = formatResetDisplay(shortResetTsOf(account), now);
  const lastFetch =
    account.last_fetched === undefined
      ? "Never"
      : new Date(account.last_fetched * 1000).toLocaleString([], {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        });
  const lastFetchAgo =
    account.last_fetched === undefined ? "Never" : formatAgo(account.last_fetched, now);

  return (
    <div
      className={cn(
        "group flex flex-col gap-3 rounded-xl border bg-card px-3.5 py-3 transition-colors lg:flex-row lg:items-center lg:gap-4",
        current
          ? "border-primary/35 ring-1 ring-primary/10"
          : "hover:border-border hover:bg-muted/20",
      )}
    >
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5">
          <Tooltip>
            <TooltipTrigger
              render={
                <p className="truncate text-sm font-medium" tabIndex={0}>
                  {email}
                </p>
              }
            />
            <TooltipContent>{email}</TooltipContent>
          </Tooltip>
          <IconBtn title="Copy email" onClick={onCopy}>
            <Copy />
          </IconBtn>
          {current && (
            <Badge variant="outline" className="border-primary/30 bg-primary/5 text-primary">
              Active
            </Badge>
          )}
          {account.archived && <Badge variant="secondary">Archived</Badge>}
          {failed && (
            <Tooltip>
              <TooltipTrigger
                render={
                  <Badge variant="destructive" tabIndex={0}>
                    Re-auth
                  </Badge>
                }
              />
              <TooltipContent className="max-w-72 whitespace-normal">{failed}</TooltipContent>
            </Tooltip>
          )}
        </div>
        <div className="mt-1.5 flex max-w-md flex-col gap-1">
          {hasShort && (
            <QuotaMeter
              label="5h"
              used={account.short_window?.used_percent}
              resetTs={shortResetTsOf(account)}
              now={now}
            />
          )}
          <QuotaMeter
            label="7d"
            used={weeklyUsed}
            resetTs={resetTsOf(account)}
            now={now}
          />
        </div>
      </div>

      <div className="flex items-center gap-3">
        <div className="grid min-w-0 flex-1 grid-cols-3 gap-2 lg:w-72 lg:flex-none">
          <Stat
            label="Weekly"
            value={weeklyLeft}
            title={weekly ? `Used ${weekly.used_percent ?? "?"}%` : "Fetch quota first"}
          />
          <Stat
            label="5-hour"
            value={hasShort ? shortLeft : "—"}
            title={hasShort ? shortReset : "No short window reported"}
          />
          <Stat label="Fetched" value={lastFetchAgo} title={lastFetch} />
        </div>

        <div className="flex shrink-0 items-center justify-end gap-0.5">
          {(current || !failed) && (
            <IconBtn
              title={current ? "Fetch quota" : "Fetch this backup account"}
              busyKey={current ? "fetch:all" : `fetch:${email}`}
              onClick={onFetch}
            >
              <RefreshCw />
            </IconBtn>
          )}
          <IconBtn title="Reset credits" busyKey={`resets:${email}`} onClick={onResets}>
            <Coins />
          </IconBtn>
          {!current && (
            <IconBtn title="Switch to this account" onClick={onSwitch}>
              <ArrowLeftRight />
            </IconBtn>
          )}
          <IconBtn
            title={account.archived ? "Unarchive account" : "Archive account"}
            onClick={onArchive}
          >
            {account.archived ? <ArchiveRestore /> : <Archive />}
          </IconBtn>
          <IconBtn title="Remove account" onClick={onRemove}>
            <Trash2 />
          </IconBtn>
        </div>
      </div>
    </div>
  );
}

export default function App() {
  const dispatch = useAppDispatch();
  const snap = useAppSelector((s) => s.snapshot.snap);
  const initialized = useAppSelector((s) => s.snapshot.initialized);
  const loadError = useAppSelector((s) => s.snapshot.loadError);
  const sortKey = useAppSelector((s) => s.snapshot.sortKey);
  const sortAsc = useAppSelector((s) => s.snapshot.sortAsc);
  const status = useAppSelector((s) => s.ui.status);
  const anyBusy = useAppSelector(selectAnyBusy);
  const confirm = useAppSelector((s) => s.ui.confirm);
  const resetsEmail = useAppSelector((s) => s.ui.resetsEmail);
  const resets = useAppSelector((s) => s.ui.resets);
  const switchInfo = useAppSelector((s) => s.ui.switchInfo);
  const updateReady = useAppSelector((s) => s.ui.updateReady);
  const updateProgress = useAppSelector((s) => s.ui.updateProgress);
  const logs = useAppSelector((s) => s.logs.entries);
  const logsOpen = useAppSelector((s) => s.logs.open);
  const fetchFailed = useAppSelector((s) => s.snapshot.fetchFailed);
  const loginStarting = useAppSelector((s) => s.login.starting);
  const confirmBusy = useAppSelector(selectBusy("confirm"));
  const fetchAllBusy = useAppSelector(selectBusy("fetch:all"));
  const restartBusy = useAppSelector(selectBusy("restart-codex"));
  const autoCheckUpdates = snap?.auto_check_updates ?? null;
  const { dark, toggle } = useTheme();
  const [codexStatus, setCodexStatus] = useState<CodexStatus | null>(null);
  const [confirmStatus, setConfirmStatus] = useState<CodexStatus | null>(null);
  const [switchStage, setSwitchStage] = useState<string | null>(null);
  const [alwaysScrollToBottom, setAlwaysScrollToBottom] = useState(() => {
    try {
      return localStorage.getItem("cm-logs-auto-scroll") === "true";
    } catch {
      return false;
    }
  });
  const logsScrollHost = useRef<HTMLDivElement>(null);
  const logsSynced = useRef(false);

  useEffect(() => {
    if (!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
      dispatch(
        setLoadError(
          "Not running inside Tauri (no backend bridge). Run `npm run tauri dev`, not `npm run dev`.",
        ),
      );
      dispatch(setStatus("Not running inside Tauri."));
      return;
    }
    void dispatch(loadSnapshot()).then(() => {
      void dispatch(pollOnce());
      void dispatch(refreshLogs());
    });
    const id = window.setInterval(() => {
      void dispatch(pollOnce());
      void dispatch(refreshLogs());
    }, 5000);
    // StrictMode mounts, unmounts, and remounts this effect while the
    // listen() promises are still pending; without the cancelled flag the
    // first set of listeners never unsubscribes and every event fires twice.
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      void p.then((u) => {
        if (cancelled) u();
        else unlisteners.push(u);
      });
    };
    track(
      listen("auth-file-changed", () => {
        void dispatch(pollOnce());
        void dispatch(refreshLogs());
      }),
    );
    track(
      listen<string>("codex-login-output", (e) => {
        dispatch(appendLines([e.payload]));
      }),
    );
    track(
      listen<string>("codex-login-url", (e) => {
        dispatch((_, getState) => {
          if (!getState().login.urlOpened) {
            dispatch(markUrlOpened());
            dispatch(setUrl(e.payload));
            dispatch(setLoginError(null));
            dispatch(setStatus("Login page ready — copy the URL or open it below."));
          }
        });
      }),
    );
    track(
      listen<{ ok: boolean; message: string }>("codex-login-done", (e) => {
        dispatch(setDone(e.payload.message));
        dispatch(setStatus(e.payload.message));
        dispatch(setLoginError(e.payload.ok ? null : e.payload.message));
        if (e.payload.ok) dispatch(setLoginOpen(false));
        void dispatch(loadSnapshot());
        void dispatch(refreshLogs());
      }),
    );
    track(
      listen<string>("switch-stage", (e) => {
        setSwitchStage(e.payload);
      }),
    );
    return () => {
      cancelled = true;
      window.clearInterval(id);
      unlisteners.forEach((u) => u());
    };
  }, [dispatch]);

  useEffect(() => {
    if (confirm?.kind !== "switch") setSwitchStage(null);
  }, [confirm]);

  useEffect(() => {
    if (!snap || logsSynced.current) return;
    logsSynced.current = true;
    dispatch(setLogsOpen(snap.logs_expanded));
    if (snap.logs_expanded) void dispatch(refreshLogs());
  }, [snap, dispatch]);

  const loggedVersion = useRef<string | null>(null);
  useEffect(() => {
    const v = snap?.app_version;
    if (v && loggedVersion.current !== v) {
      loggedVersion.current = v;
      void api
        .logMessage(`CodexMonitor v${v} started.`)
        .then(() => {
          void dispatch(refreshLogs());
        })
        .catch(() => undefined);
    }
  }, [snap?.app_version, dispatch]);

  const lastLoggedStatus = useRef<string | null>(null);
  useEffect(() => {
    if (!status || lastLoggedStatus.current === status) return;
    lastLoggedStatus.current = status;
    void api
      .logMessage(status)
      .then(() => {
        void dispatch(refreshLogs());
      })
      .catch(() => undefined);
  }, [status, dispatch]);

  useEffect(() => {
    let active = true;
    const load = () => {
      void api
        .codexStatus()
        .then((next) => {
          if (active) setCodexStatus(next);
        })
        .catch(() => undefined);
    };
    load();
    const id = window.setInterval(load, 15000);
    return () => {
      active = false;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    if (confirm?.kind !== "switch") {
      setConfirmStatus(null);
      return;
    }
    let active = true;
    api
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
  }, [confirm]);

  useEffect(() => {
    if (confirm !== null) return;
    void api
      .codexStatus()
      .then(setCodexStatus)
      .catch(() => undefined);
  }, [confirm]);

  useEffect(() => {
    try {
      localStorage.setItem("cm-logs-auto-scroll", String(alwaysScrollToBottom));
    } catch {
      // Keep the toggle usable for this session when storage is unavailable.
    }
    if (!alwaysScrollToBottom || !logsOpen) return;
    const viewport = logsScrollHost.current?.querySelector<HTMLElement>(
      '[data-slot="scroll-area-viewport"]',
    );
    if (viewport) viewport.scrollTop = viewport.scrollHeight;
  }, [alwaysScrollToBottom, logs, logsOpen]);

  useEffect(() => {
    if (autoCheckUpdates !== true) return;
    const t = window.setTimeout(() => void dispatch(checkUpdates(false)), 1500);
    const id = window.setInterval(() => void dispatch(checkUpdates(false)), 6 * 3600 * 1000);
    return () => {
      window.clearTimeout(t);
      window.clearInterval(id);
    };
  }, [dispatch, autoCheckUpdates]);

  const rows = useMemo(() => {
    if (!snap) return [];
    const entries = Object.entries(snap.accounts);
    const visible = snap.show_archived ? entries : entries.filter(([, a]) => !a.archived);
    const keyOf = (email: string, a: AccountUsage): string | number => {
      if (sortKey === "email") return email.toLowerCase();
      if (sortKey === "quota") return usedOf(a) ?? -1;
      return resetTsOf(a);
    };
    return visible.sort(([ea, aa], [eb, ab]) => {
      if (ea === snap.current_email) return -1;
      if (eb === snap.current_email) return 1;
      if ((aa.archived ?? false) !== (ab.archived ?? false)) {
        return (aa.archived ?? false) ? 1 : -1;
      }
      if (!sortKey) return resetTsOf(aa) - resetTsOf(ab);
      const ka = keyOf(ea, aa);
      const kb = keyOf(eb, ab);
      const cmp = ka < kb ? -1 : ka > kb ? 1 : 0;
      return sortAsc ? cmp : -cmp;
    });
  }, [snap, sortKey, sortAsc]);

  async function pickExportFile() {
    const path = await save({
      defaultPath: "codex-monitor-backup.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (path) void dispatch(doExport(path));
  }

  async function pickImportFile() {
    const path = await open({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path || Array.isArray(path)) return;
    void dispatch(doImport(path));
  }

  async function copyAccountEmail(email: string) {
    try {
      await navigator.clipboard.writeText(email);
      dispatch(setStatus(`Copied ${email}.`));
    } catch {
      dispatch(setStatus("Could not copy the email."));
    }
  }

  function startLoginUi() {
    // Open the dialog first: the user picks ChatGPT login (and starts it) or
    // imports a saved session, so no browser popup appears before they choose.
    dispatch(resetLines());
    dispatch(setLoginOpen(true));
  }

  function restartCodexNow() {
    dispatch(setBusy({ key: "restart-codex", on: true }));
    dispatch(setStatus("Restarting Codex…"));
    void api
      .restartCodex()
      .then((message) => {
        dispatch(setStatus(message));
        void dispatch(refreshLogs());
      })
      .catch((e) => {
        dispatch(setStatus(`Restart failed: ${e}`));
        void dispatch(refreshLogs());
      })
      .finally(() => {
        dispatch(setBusy({ key: "restart-codex", on: false }));
        void api
          .codexStatus()
          .then(setCodexStatus)
          .catch(() => undefined);
      });
  }

  const now = Date.now() / 1000;
  const soonest =
    resetsEmail && snap ? soonestExpiringCredit(snap.accounts[resetsEmail]?.resets) : undefined;
  const activeEmail = snap?.current_email ?? null;
  const active = activeEmail && snap ? snap.accounts[activeEmail] : undefined;
  const activeUsed = active ? usedOf(active) : undefined;
  const activeLastFetch =
    active?.last_fetched === undefined
      ? "Never"
      : new Date(active.last_fetched * 1000).toLocaleString([], {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        });
  const activeLastFetchAgo =
    active?.last_fetched === undefined ? "Never" : formatAgo(active.last_fetched, now);
  const codexLabel = !codexStatus
    ? "Checking Codex…"
    : !codexStatus.supported
      ? "Lifecycle unavailable"
      : !codexStatus.installed
        ? "Codex not installed"
        : codexStatus.running
          ? `Codex running${codexStatus.process_count > 1 ? ` · ${codexStatus.process_count} processes` : ""}`
          : "Codex closed";
  const switchStatusText = !confirmStatus
    ? "Checking whether Codex is running…"
    : !confirmStatus.supported
      ? "Codex lifecycle control is unavailable here. Only the active auth file will change."
      : !confirmStatus.installed
        ? "Codex desktop was not found. Only the active auth file will change."
        : confirmStatus.running
          ? `Codex is running (${confirmStatus.process_count} process${
              confirmStatus.process_count === 1 ? "" : "es"
            }). It will close, switch, then reopen automatically.`
          : "Codex isn't running. The switch applies now and the next launch uses this account.";

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col bg-background text-foreground">
        <header className="relative z-20 flex h-14 shrink-0 items-center gap-3 border-b bg-background/80 px-4 backdrop-blur-xl">
          <span className="grid size-8 shrink-0 place-items-center rounded-xl bg-primary/10 text-primary">
            <Gauge className="size-4.5" aria-hidden />
          </span>
          <div className="min-w-0">
            <p className="truncate text-xs font-medium">
              {activeEmail ?? "No active session"}
            </p>
            <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <StatusDot active={!!codexStatus?.running} />
              <span className="truncate">{codexLabel}</span>
            </p>
          </div>
          <div className="ml-auto flex items-center gap-1.5">
            <IconBtn
              title="Fetch all accounts"
              busyKey="fetch:all-accounts"
              onClick={() => void dispatch(fetchAllAccounts())}
            >
              <RefreshCw />
            </IconBtn>
            <Button
              size="sm"
              variant="outline"
              onClick={startLoginUi}
              disabled={loginStarting}
            >
              {loginStarting ? <Spin /> : <UserPlus />}
              Add account
            </Button>
            <IconBtn title="Toggle theme" onClick={toggle}>
              {dark ? <Sun /> : <Moon />}
            </IconBtn>
            <IconBtn title="Settings" onClick={() => dispatch(setSettingsOpen(true))}>
              <Settings />
            </IconBtn>
          </div>
        </header>

        <main className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto flex w-full max-w-5xl flex-col gap-4 px-4 py-5 sm:px-6">
            {switchInfo?.warning && (
              <div className="flex flex-col gap-2 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 text-amber-900 sm:flex-row sm:items-center dark:text-amber-200">
                <AlertTriangle className="size-4 shrink-0" aria-hidden />
                <p className="min-w-0 flex-1 text-xs leading-relaxed">{switchInfo.warning}</p>
                {codexStatus?.supported && (
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={restartCodexNow}
                    disabled={restartBusy}
                    className="shrink-0 border-amber-500/40 bg-transparent"
                  >
                    {restartBusy ? <Spin /> : <RotateCw />}
                    Retry restart
                  </Button>
                )}
                <IconBtn title="Dismiss" onClick={() => dispatch(setSwitchInfo(null))}>
                  <X />
                </IconBtn>
              </div>
            )}

            {!initialized ? (
              loadError ? (
                <div className="flex flex-col items-center gap-3 rounded-2xl border bg-card px-6 py-14 text-center">
                  <p className="text-sm font-semibold">Could not start the app</p>
                  <p className="max-w-md text-xs text-muted-foreground">{loadError}</p>
                  <Button
                    onClick={() => {
                      dispatch(setLoadError(null));
                      void dispatch(loadSnapshot()).then(() => void dispatch(pollOnce()));
                    }}
                  >
                    <RefreshCw />
                    Retry
                  </Button>
                </div>
              ) : (
                <div className="flex items-center justify-center gap-2 rounded-2xl border bg-card px-6 py-20 text-sm text-muted-foreground">
                  <Spin /> Loading accounts…
                </div>
              )
            ) : (
              <>
                <section className="rounded-2xl border bg-card p-4 shadow-sm sm:p-5">
                  <div className="flex flex-col gap-4 lg:flex-row lg:items-center">
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <p className="text-[10px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                          Active account
                        </p>
                        {active && (
                          <Badge
                            variant="outline"
                            className="border-primary/30 bg-primary/5 text-primary"
                          >
                            Current session
                          </Badge>
                        )}
                        {snap?.auto_fetch && snap.auto_fetch !== "None" && (
                          <Badge variant="secondary">Auto · {snap.auto_fetch}</Badge>
                        )}
                        {activeEmail && fetchFailed[activeEmail] && (
                          <Tooltip>
                            <TooltipTrigger
                              render={
                                <Badge variant="destructive" tabIndex={0}>
                                  Re-auth needed
                                </Badge>
                              }
                            />
                            <TooltipContent className="max-w-72 whitespace-normal">
                              {fetchFailed[activeEmail]}
                            </TooltipContent>
                          </Tooltip>
                        )}
                      </div>
                      <div className="mt-1 flex min-w-0 items-center gap-1">
                        {activeEmail ? (
                          <Tooltip>
                            <TooltipTrigger
                              render={
                                <h1
                                  className="truncate text-xl font-semibold tracking-[-0.02em] sm:text-2xl"
                                  tabIndex={0}
                                >
                                  {activeEmail}
                                </h1>
                              }
                            />
                            <TooltipContent>{activeEmail}</TooltipContent>
                          </Tooltip>
                        ) : (
                          <h1 className="truncate text-xl font-semibold tracking-[-0.02em] sm:text-2xl">
                            No active session
                          </h1>
                        )}
                        {activeEmail && (
                          <IconBtn
                            title="Copy active email"
                            onClick={() => void copyAccountEmail(activeEmail)}
                          >
                            <Copy />
                          </IconBtn>
                        )}
                      </div>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {active
                          ? active.last_fetched === undefined
                            ? "No quota fetched yet."
                            : `Last fetch ${activeLastFetchAgo} · ${activeLastFetch}`
                          : snap?.auth_file_exists
                            ? "Fetch quota for the active Codex session."
                            : "Sign in to Codex to create an account."}
                      </p>
                      <div className="mt-3 flex flex-wrap items-center gap-1.5">
                        {activeEmail ? (
                          <>
                            <Button
                              size="sm"
                              onClick={() => void dispatch(manualFetch())}
                              disabled={fetchAllBusy}
                            >
                              <RefreshCw />
                              Refresh quota
                            </Button>
                            <Button
                              size="sm"
                              variant="outline"
                              onClick={() => void dispatch(openResets(activeEmail))}
                            >
                              <Coins />
                              Reset credits
                            </Button>
                          </>
                        ) : (
                          <Button size="sm" onClick={startLoginUi} disabled={loginStarting}>
                            <UserPlus />
                            Add account
                          </Button>
                        )}
                        {codexStatus?.supported && (
                          <IconBtn
                            title="Restart Codex"
                            busyKey="restart-codex"
                            onClick={restartCodexNow}
                          >
                            <RotateCw />
                          </IconBtn>
                        )}
                      </div>
                    </div>

                    <div className="flex w-full max-w-[29.625rem] shrink-0 flex-col gap-2 rounded-xl border bg-muted/20 p-3 lg:w-[29.625rem]">
                      <p className="text-[10px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
                        Quota remaining
                      </p>
                      {active ? (
                        <>
                          {hasShortWindow(active) && (
                            <QuotaMeter
                              label="5h"
                              used={active.short_window?.used_percent}
                              resetTs={shortResetTsOf(active)}
                              now={now}
                            />
                          )}
                          <QuotaMeter
                            label="7d"
                            used={activeUsed}
                            resetTs={resetTsOf(active)}
                            now={now}
                          />
                        </>
                      ) : (
                        <p className="py-2 text-xs text-muted-foreground">No quota data yet.</p>
                      )}
                    </div>
                  </div>
                </section>

                <section className="flex flex-col gap-2.5">
                  <div className="flex flex-wrap items-center gap-2">
                    <h2 className="text-sm font-semibold tracking-[-0.01em]">
                      Accounts
                      <span className="ml-1.5 text-xs font-normal text-muted-foreground">
                        {rows.length}
                      </span>
                    </h2>
                    <div className="ml-auto flex flex-wrap items-center gap-1">
                      <div className="inline-flex items-center gap-0.5 rounded-lg bg-muted p-0.5">
                        <SortButton
                          label="Reset"
                          active={sortKey === "reset"}
                          asc={sortAsc}
                          onClick={() => void dispatch(cycleSort("reset"))}
                        />
                        <SortButton
                          label="Email"
                          active={sortKey === "email"}
                          asc={sortAsc}
                          onClick={() => void dispatch(cycleSort("email"))}
                        />
                        <SortButton
                          label="Quota"
                          active={sortKey === "quota"}
                          asc={sortAsc}
                          onClick={() => void dispatch(cycleSort("quota"))}
                        />
                      </div>
                      <IconBtn
                        title={
                          snap?.show_archived
                            ? "Hide archived accounts"
                            : "Show archived accounts"
                        }
                        pressed={!!snap?.show_archived}
                        onClick={() => void dispatch(toggleArchivedVisibility())}
                      >
                        {snap?.show_archived ? <EyeOff /> : <Eye />}
                      </IconBtn>
                      <IconBtn
                        title="Fetch all accounts"
                        busyKey="fetch:all-accounts"
                        onClick={() => void dispatch(fetchAllAccounts())}
                      >
                        <RefreshCw />
                      </IconBtn>
                    </div>
                  </div>

                  {rows.length === 0 ? (
                    <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed bg-card px-6 py-12 text-center text-muted-foreground">
                      <p className="text-sm">
                        {snap && !snap.auth_file_exists
                          ? "Not signed in — the Codex auth file is missing."
                          : "No accounts yet. Add an account to start monitoring quotas."}
                      </p>
                      <Button onClick={startLoginUi} disabled={loginStarting}>
                        {loginStarting ? <Spin /> : <UserPlus />}
                        Add account
                      </Button>
                    </div>
                  ) : (
                    <div className="flex flex-col gap-2">
                      {rows.map(([email, account]) => (
                        <AccountRow
                          key={email}
                          email={email}
                          account={account}
                          current={email === snap?.current_email}
                          now={now}
                          failed={fetchFailed[email]}
                          onCopy={() => void copyAccountEmail(email)}
                          onFetch={() =>
                            void dispatch(
                              manualFetch(email === snap?.current_email ? undefined : email),
                            )
                          }
                          onResets={() => void dispatch(openResets(email))}
                          onSwitch={() => dispatch(setConfirm({ kind: "switch", email }))}
                          onArchive={() =>
                            dispatch(
                              setConfirm({
                                kind: "archive",
                                email,
                                archived: !!account.archived,
                              }),
                            )
                          }
                          onRemove={() => dispatch(setConfirm({ kind: "remove", email }))}
                        />
                      ))}
                    </div>
                  )}
                </section>
              </>
            )}
          </div>
        </main>

        <footer className="shrink-0 border-t bg-card/70 backdrop-blur-xl">
          <div className="flex h-10 items-center gap-2 px-3 sm:px-4">
            <button
              type="button"
              className="flex min-w-0 flex-1 items-center gap-2 text-left"
              onClick={() => void dispatch(toggleLogs())}
              aria-expanded={logsOpen}
            >
              <StatusDot active={anyBusy} pulse />
              <span className="min-w-0 truncate text-xs text-muted-foreground">
                {anyBusy ? "Working… " : ""}
                {updateProgress !== null ? `Downloading update ${updateProgress}%… ` : ""}
                {status}
              </span>
              <span className="ml-auto shrink-0 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
                Activity
              </span>
              {logsOpen ? (
                <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
              ) : (
                <ChevronUp className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
              )}
            </button>
            {snap?.app_version && (
              <span className="shrink-0 text-[10px] text-muted-foreground/70">
                v{snap.app_version}
              </span>
            )}
          </div>
          {logsOpen && (
            <div className="border-t px-3 pt-2 pb-3 sm:px-4">
              <div className="flex flex-wrap items-center gap-1 pb-2">
                <IconBtn
                  title="Copy status"
                  onClick={() => void navigator.clipboard.writeText(status)}
                >
                  <Copy />
                </IconBtn>
                <IconBtn
                  title="Clear logs"
                  busyKey="logs:clear"
                  onClick={() => void dispatch(clearLogs())}
                >
                  <Eraser />
                </IconBtn>
                <IconBtn
                  title={
                    alwaysScrollToBottom
                      ? "Disable always scroll to bottom"
                      : "Always scroll to bottom"
                  }
                  pressed={alwaysScrollToBottom}
                  onClick={() => setAlwaysScrollToBottom((enabled) => !enabled)}
                >
                  <ArrowDownToLine />
                </IconBtn>
                <IconBtn
                  title="Export data"
                  busyKey="data:export"
                  onClick={() => void pickExportFile()}
                >
                  <Download />
                </IconBtn>
                <IconBtn
                  title="Import data"
                  busyKey="data:import"
                  onClick={() => void pickImportFile()}
                >
                  <Upload />
                </IconBtn>
                <Separator orientation="vertical" className="mx-1 h-5" />
                <IconBtn
                  title={`Check for updates (v${snap?.app_version ?? "?"})`}
                  busyKey="update:check"
                  onClick={() => void dispatch(checkUpdates(true))}
                >
                  <CloudDownload />
                </IconBtn>
                {updateReady && (
                  <IconBtn
                    title={
                      updateProgress !== null ? `Installing… ${updateProgress}%` : updateReady
                    }
                    busyKey="update:install"
                    onClick={() => void dispatch(installUpdate())}
                  >
                    <ArrowDownToLine />
                  </IconBtn>
                )}
                {codexStatus?.supported && (
                  <IconBtn
                    title="Restart Codex"
                    busyKey="restart-codex"
                    onClick={restartCodexNow}
                  >
                    <RotateCw />
                  </IconBtn>
                )}
                <span className="ml-auto flex shrink-0 items-center gap-1.5">
                  <Label htmlFor="auto-fetch" className="text-[11px] text-muted-foreground">
                    Auto fetch
                  </Label>
                  <Select
                    value={snap?.auto_fetch ?? "None"}
                    onValueChange={(v) => void dispatch(saveAutoFetchValue(v ?? "None"))}
                  >
                    <SelectTrigger id="auto-fetch" className="h-7 w-24 text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {(snap?.auto_fetch_options ?? ["None"]).map((o) => (
                        <SelectItem key={o} value={o}>
                          {o}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </span>
              </div>
              <div ref={logsScrollHost}>
                <ScrollArea className="h-48 rounded-lg border bg-muted/30">
                  <pre className="p-2.5 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-muted-foreground select-text">
                    {logs.length === 0 ? "(no log entries yet)" : logs.join("\n")}
                  </pre>
                </ScrollArea>
              </div>
            </div>
          )}
        </footer>

        <Dialog open={confirm !== null} onOpenChange={(o) => !o && dispatch(setConfirm(null))}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <DialogTitle>
                {confirm?.kind === "switch"
                  ? "Switch Codex account"
                  : confirm?.kind === "remove"
                    ? "Remove account"
                    : "Archive account"}
              </DialogTitle>
              <DialogDescription>
                {confirm?.kind === "remove" &&
                  `Remove ${confirm.email}? Quota history for this account is deleted.`}
                {confirm?.kind === "archive" &&
                  `${confirm.archived ? "Unarchive" : "Archive"} ${confirm.email}?`}
                {confirm?.kind === "switch" && (
                  <>
                    Make <span className="font-medium text-foreground">{confirm.email}</span> the
                    active Codex sign-in.
                  </>
                )}
              </DialogDescription>
            </DialogHeader>
            {confirm?.kind === "switch" && (
              <>
                {(!confirmStatus?.supported || !confirmStatus?.installed) && (
                  <p className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-900 dark:text-amber-200">
                    <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden />
                    {switchStatusText}
                  </p>
                )}
                <div className="rounded-xl border bg-muted/30 p-3">
                  <SwitchPlan
                    running={!!confirmStatus?.supported && !!confirmStatus?.installed && !!confirmStatus.running}
                    processCount={confirmStatus?.process_count ?? 0}
                    stage={switchStage}
                    busy={confirmBusy}
                  />
                </div>
                <p className="flex items-start gap-1.5 text-[11px] leading-relaxed text-muted-foreground">
                  <ShieldCheck className="mt-0.5 size-3.5 shrink-0 text-primary" aria-hidden />
                  Only the active Codex session file is rewritten. Other saved accounts stay
                  untouched.
                </p>
              </>
            )}
            <DialogFooter>
              <Button variant="outline" onClick={() => dispatch(setConfirm(null))}>
                Cancel
              </Button>
              <Button
                variant={confirm?.kind === "remove" ? "destructive" : "default"}
                onClick={() => void dispatch(runConfirmAction())}
                disabled={confirmBusy}
              >
                {confirmBusy ? (
                  <>
                    <Spin />
                    {confirm?.kind === "switch"
                      ? switchStage === "closing"
                        ? "Closing Codex…"
                        : switchStage === "switching"
                          ? "Switching…"
                          : switchStage === "reopening"
                            ? "Reopening Codex…"
                            : "Switching…"
                      : "Working…"}
                  </>
                ) : confirm?.kind === "switch" ? (
                  confirmStatus?.running ? (
                    "Close, switch & reopen"
                  ) : (
                    "Switch account"
                  )
                ) : (
                  "Confirm"
                )}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Dialog
          open={resetsEmail !== null}
          onOpenChange={(o) => !o && dispatch(setResetsView({ email: null, data: null }))}
        >
          <DialogContent className="max-w-xl">
            <DialogHeader>
              <DialogTitle>Reset Credits — {resetsEmail}</DialogTitle>
            </DialogHeader>
            {!resets?.credits?.length ? (
              <p className="text-sm text-muted-foreground">
                No reset credits found. Fetch quota first.
              </p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Expires</TableHead>
                    <TableHead>Remaining</TableHead>
                    <TableHead>Granted</TableHead>
                    <TableHead>Status</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {resets.credits.map((c, i) => (
                    <TableRow key={i}>
                      <TableCell>{formatCreditExpires(c.expires_at, now)}</TableCell>
                      <TableCell>{formatTimeRemaining(c.expires_at, now)}</TableCell>
                      <TableCell>{formatGrantedAt(c.granted_at)}</TableCell>
                      <TableCell>{c.status ?? "unknown"}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
            {soonest && (
              <p className="text-xs text-muted-foreground">
                Soonest expiry in {formatTimeRemaining(soonest.expires_at, now)}.
              </p>
            )}
          </DialogContent>
        </Dialog>

        <AddAccountDialog />
        <SettingsDialog />
      </div>
    </TooltipProvider>
  );
}
