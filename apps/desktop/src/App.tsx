import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { sendNotification } from "@tauri-apps/plugin-notification";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  formatCreditExpires,
  formatGrantedAt,
  formatQuotaLeft,
  formatResetDisplay,
  formatTimeRemaining,
  resetTsOf,
  soonestExpiringCredit,
  usedOf,
  weeklyOf,
  type AccountUsage,
  type ResetCreditsPayload,
} from "./lib/format";
import { api, type AuthOutcome, type Snapshot } from "./lib/tauri";

type SortKey = "email" | "quota" | "reset";
type ConfirmAction =
  | { kind: "remove"; email: string }
  | { kind: "archive"; email: string; archived: boolean }
  | { kind: "switch"; email: string }
  | null;

const MISSING_TOKEN_RETRIES = 6;

function useTheme() {
  const [dark, setDark] = useState(() => {
    const saved = localStorage.getItem("cm-theme");
    if (saved) return saved === "dark";
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  });
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    localStorage.setItem("cm-theme", dark ? "dark" : "light");
  }, [dark]);
  return { dark, toggle: () => setDark((d) => !d) };
}

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [status, setStatus] = useState("Starting…");
  const [busy, setBusy] = useState(false);
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortAsc, setSortAsc] = useState(true);
  const [confirm, setConfirm] = useState<ConfirmAction>(null);
  const [resetsEmail, setResetsEmail] = useState<string | null>(null);
  const [resets, setResets] = useState<ResetCreditsPayload | null>(null);
  const [logsOpen, setLogsOpen] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [loginOpen, setLoginOpen] = useState(false);
  const [loginLines, setLoginLines] = useState<string[]>([]);
  const [loginDone, setLoginDone] = useState<string | null>(null);
  const [updateReady, setUpdateReady] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [switchInfo, setSwitchInfo] = useState<string | null>(null);
  const { dark, toggle } = useTheme();
  const missingRetries = useRef(0);
  const urlOpened = useRef(false);
  const sortTimer = useRef<number | undefined>(undefined);

  const refresh = useCallback(async () => {
    try {
      const s = await api.snapshot();
      setSnap(s);
      setSortKey(
        s.sort_column === "weekly_quota" || s.sort_column === "quota"
          ? "quota"
          : s.sort_column === "weekly_reset"
            ? "reset"
            : s.sort_column === "email"
              ? "email"
              : null,
      );
      setSortAsc(s.sort_asc);
      setLogsOpen(s.logs_expanded);
    } catch (e) {
      setStatus(`Failed to load state: ${e}`);
    }
  }, []);

  const handleOutcome = useCallback(
    async (o: AuthOutcome) => {
      if (o.kind === "NoChange") return;
      if (o.kind === "MissingToken") {
        if (missingRetries.current < MISSING_TOKEN_RETRIES) {
          missingRetries.current += 1;
          setStatus("Auth file changing, retrying read…");
          setTimeout(async () => {
            try {
              await handleOutcome(await api.processAuthFile());
            } catch {
              /* next poll */
            }
          }, 350);
        } else {
          missingRetries.current = 0;
          await refresh();
        }
        return;
      }
      missingRetries.current = 0;
      if (o.kind === "AuthRefreshed") {
        setStatus(o.message);
        void sendNotification({ title: "Codex Account Monitor", body: o.message });
      } else if (o.kind === "Fetched" || o.kind === "LoggedOut") {
        setStatus(o.message);
      } else if (o.kind === "ParseError" || o.kind === "NoFile") {
        setStatus(o.kind === "NoFile" ? "Auth file removed. Signed out." : o.message);
      }
      await refresh();
    },
    [refresh],
  );

  const poll = useCallback(async () => {
    try {
      await handleOutcome(await api.processAuthFile());
    } catch {
      /* transient */
    }
    try {
      const r = await api.checkAutoFetch();
      if (r) {
        setStatus(r.message);
        await refresh();
      }
    } catch (e) {
      setStatus(`Auto-fetch failed: ${e}`);
    }
  }, [handleOutcome, refresh]);

  useEffect(() => {
    refresh().then(() => poll());
    const id = window.setInterval(poll, 5000);
    const unlisteners: Array<() => void> = [];
    listen("auth-file-changed", () => void poll()).then((u) => unlisteners.push(u));
    listen<string>("codex-login-output", (e) =>
      setLoginLines((lines) => [...lines.slice(-200), e.payload]),
    ).then((u) => unlisteners.push(u));
    listen<string>("codex-login-url", (e) => {
      if (!urlOpened.current) {
        urlOpened.current = true;
        void openUrl(e.payload).catch(() => undefined);
      }
    }).then((u) => unlisteners.push(u));
    listen<{ ok: boolean; message: string }>("codex-login-done", (e) => {
      setLoginDone(e.payload.message);
      setStatus(e.payload.message);
      void refresh();
    }).then((u) => unlisteners.push(u));
    return () => {
      window.clearInterval(id);
      unlisteners.forEach((u) => u());
    };
  }, [poll, refresh]);

  const checkUpdates = useCallback(async (manual: boolean) => {
    if (manual) setCheckingUpdate(true);
    try {
      const update = await check();
      if (update) {
        setUpdateReady(`Update ${update.version} ready to install.`);
        setStatus(`Update ${update.version} is available.`);
        if (manual) {
          await update.downloadAndInstall((ev) => {
            if (ev.event === "Finished") setStatus("Update installed, restarting…");
          });
          await relaunch();
        }
      } else {
        setUpdateReady(null);
        if (manual) setStatus("You're already on the latest version.");
      }
    } catch (e) {
      if (manual) setStatus(`Update check failed: ${e}`);
    } finally {
      if (manual) setCheckingUpdate(false);
    }
  }, []);

  useEffect(() => {
    const t = window.setTimeout(() => void checkUpdates(false), 1500);
    const id = window.setInterval(() => void checkUpdates(false), 6 * 3600 * 1000);
    return () => {
      window.clearTimeout(t);
      window.clearInterval(id);
    };
  }, [checkUpdates]);

  const installUpdate = useCallback(async () => {
    setCheckingUpdate(true);
    try {
      const update = await check();
      if (!update) {
        setStatus("You're already on the latest version.");
        return;
      }
      await update.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setStatus(`Update failed: ${e}`);
    } finally {
      setCheckingUpdate(false);
    }
  }, []);

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

  const onHeader = (key: SortKey) => {
    let nextKey: SortKey | null = key;
    let nextAsc = true;
    if (sortKey === key) {
      if (sortAsc) nextAsc = false;
      else nextKey = null;
    }
    setSortKey(nextKey);
    setSortAsc(nextAsc);
    window.clearTimeout(sortTimer.current);
    sortTimer.current = window.setTimeout(() => {
      const col = nextKey === "quota" ? "weekly_quota" : nextKey === "reset" ? "weekly_reset" : nextKey;
      void api.saveSort(col, nextAsc).catch(() => undefined);
    }, 500);
  };

  const arrow = (key: SortKey) => (sortKey === key ? (sortAsc ? " ▲" : " ▼") : " ↕");

  async function doFetch(email?: string) {
    setBusy(true);
    try {
      const r = email ? await api.fetchBackup(email) : await api.manualFetch();
      setStatus(r.message);
    } catch (e) {
      setStatus(`Fetch failed: ${e}`);
    } finally {
      setBusy(false);
      await refresh();
    }
  }

  async function doConfirm() {
    if (!confirm) return;
    setBusy(true);
    try {
      if (confirm.kind === "remove") {
        setStatus(await api.removeAccount(confirm.email));
      } else if (confirm.kind === "archive") {
        setStatus(await api.setArchived(confirm.email, !confirm.archived));
      } else {
        const msg = await api.switchAccount(confirm.email);
        setStatus(msg);
        setSwitchInfo(confirm.email);
      }
    } catch (e) {
      setStatus(`Action failed: ${e}`);
    } finally {
      setConfirm(null);
      setBusy(false);
      await refresh();
    }
  }

  async function openResets(email: string) {
    try {
      setResets(await api.resets(email));
      setResetsEmail(email);
    } catch (e) {
      setStatus(`Could not load reset credits: ${e}`);
    }
  }

  async function doExport() {
    const path = await save({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path) return;
    try {
      setStatus(await api.exportData(path));
    } catch (e) {
      setStatus(`Export failed: ${e}`);
    }
  }

  async function doImport() {
    const path = await open({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path || Array.isArray(path)) return;
    try {
      setStatus(await api.importData(path));
      await refresh();
    } catch (e) {
      setStatus(`Import failed: ${e}`);
    }
  }

  async function toggleLogs() {
    const next = !logsOpen;
    setLogsOpen(next);
    await api.saveLogsExpanded(next).catch(() => undefined);
    if (next) {
      try {
        setLogs(await api.logs());
      } catch (e) {
        setStatus(`Could not load logs: ${e}`);
      }
    }
  }

  function startLogin() {
    setLoginLines([]);
    setLoginDone(null);
    urlOpened.current = false;
    setLoginOpen(true);
    api.loginStart().catch((e) => setLoginLines([`Failed to start login: ${e}`]));
  }

  const now = Date.now() / 1000;
  const soonest = resetsEmail && snap ? soonestExpiringCredit(snap.accounts[resetsEmail]?.resets) : undefined;

  return (
    <div className="flex h-screen flex-col bg-slate-100 text-slate-900 dark:bg-slate-950 dark:text-slate-100">
      <div className="m-2 flex-1 overflow-hidden rounded-2xl border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900">
        <div className="grid grid-cols-[5fr_2fr_5fr_3fr] items-center gap-2 border-b border-slate-200 bg-slate-50 px-3 py-2 text-xs font-bold dark:border-slate-800 dark:bg-slate-800/60">
          <button className="text-left hover:text-blue-600" onClick={() => onHeader("email")} title="Sort by email">
            Account Email{arrow("email")}
          </button>
          <button className="text-left hover:text-blue-600" onClick={() => onHeader("quota")} title="Sort by quota">
            Quota{arrow("quota")}
          </button>
          <button className="text-left hover:text-blue-600" onClick={() => onHeader("reset")} title="Sort by reset">
            Reset{arrow("reset")}
          </button>
          <span className="text-right">Action</span>
        </div>
        <div className="h-full overflow-y-auto pb-10">
          {rows.length === 0 && (
            <div className="flex h-48 flex-col items-center justify-center gap-3 text-slate-500">
              <p>{snap && !snap.auth_file_exists ? "Not signed in — the Codex auth file is missing." : "No accounts yet. Fetch quota or add an account."}</p>
              <button onClick={startLogin} className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-bold text-white hover:bg-blue-500" title="Add account via Codex login">
                Add account
              </button>
            </div>
          )}
          {rows.map(([email, a]) => {
            const isCurrent = email === snap?.current_email;
            const weekly = weeklyOf(a);
            return (
              <div
                key={email}
                className={`grid grid-cols-[5fr_2fr_5fr_3fr] items-center gap-2 border-b border-slate-100 px-3 py-2 text-sm odd:bg-slate-50/60 dark:border-slate-800 dark:odd:bg-slate-800/30 ${isCurrent ? "bg-emerald-50 dark:bg-emerald-950/40" : ""}`}
              >
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate font-medium" title={email}>{email}</span>
                  {isCurrent && (
                    <span className="shrink-0 rounded-full border border-emerald-500 px-1.5 text-[10px] font-bold text-emerald-700 dark:text-emerald-300" title="Active account">
                      current
                    </span>
                  )}
                  {a.archived && <span className="shrink-0 text-[10px] text-slate-400" title="Archived">arch</span>}
                </div>
                <div title={weekly ? `Used ${weekly.used_percent ?? "?"}%` : "Fetch quota first"}>
                  {formatQuotaLeft(usedOf(a))}
                </div>
                <div className="truncate" title={formatResetDisplay(resetTsOf(a), now)}>
                  {formatResetDisplay(resetTsOf(a), now)}
                </div>
                <div className="flex justify-end gap-1">
                  <button onClick={() => void doFetch(isCurrent ? undefined : email)} disabled={busy} className="rounded-md bg-blue-600 px-2 py-1 text-xs font-bold text-white hover:bg-blue-500 disabled:opacity-50" title={isCurrent ? "Fetch quota" : "Fetch this backup account"}>
                    Fetch
                  </button>
                  <button onClick={() => void openResets(email)} className="rounded-md bg-slate-200 px-2 py-1 text-xs dark:bg-slate-700" title="Reset credits">
                    Credits
                  </button>
                  {!isCurrent && (
                    <button onClick={() => setConfirm({ kind: "switch", email })} disabled={busy} className="rounded-md bg-slate-200 px-2 py-1 text-xs dark:bg-slate-700" title="Switch to this account">
                      Switch
                    </button>
                  )}
                  <button onClick={() => setConfirm({ kind: "archive", email, archived: !!a.archived })} className="rounded-md bg-slate-200 px-2 py-1 text-xs dark:bg-slate-700" title={a.archived ? "Unarchive" : "Archive"}>
                    {a.archived ? "Unarch" : "Arch"}
                  </button>
                  <button onClick={() => setConfirm({ kind: "remove", email })} className="rounded-md bg-red-100 px-2 py-1 text-xs text-red-700 dark:bg-red-950 dark:text-red-300" title="Remove account">
                    Del
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      <div className="mx-2 mb-2 rounded-2xl border border-slate-200 bg-white p-2 dark:border-slate-800 dark:bg-slate-900">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="min-w-0 flex-1 truncate px-2 text-xs text-slate-500" title={status}>{status}</span>
          <button onClick={() => { void navigator.clipboard.writeText(status); }} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500" title="Copy status">⧉</button>
          <button onClick={() => void toggleLogs()} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500" title={logsOpen ? "Hide logs" : "Show logs"}>Logs</button>
          {logsOpen && (
            <button onClick={() => { void api.clearLogs().then(() => setLogs([])); }} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500" title="Clear logs">Clear</button>
          )}
          <button onClick={() => void doExport()} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500" title="Export data">Export</button>
          <button onClick={() => void doImport()} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500" title="Import data">Import</button>
          <button onClick={startLogin} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500" title="Add account">+Login</button>
          <label className="flex items-center gap-1 text-xs text-slate-500" title="Auto-fetch interval for the active account">
            Auto
            <select
              value={snap?.auto_fetch ?? "None"}
              onChange={(e) => { void api.saveAutoFetch(e.target.value).then((m) => { setStatus(m); void refresh(); }).catch((err) => setStatus(`Auto-fetch failed: ${err}`)); }}
              className="rounded-lg bg-blue-600 px-1.5 py-1.5 text-xs font-bold text-white"
            >
              {(snap?.auto_fetch_options ?? ["None"]).map((o) => (
                <option key={o} value={o}>{o}</option>
              ))}
            </select>
          </label>
          <button onClick={() => void doFetch()} disabled={busy} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500 disabled:opacity-50" title="Fetch quota now">Fetch</button>
          <button
            onClick={() => {
              const next = !(snap?.show_archived ?? false);
              void api.saveShowArchived(next).then(() => void refresh());
            }}
            className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500"
            title={snap?.show_archived ? "Hide archived accounts" : "Show archived accounts"}
          >
            Arch
          </button>
          <button onClick={() => void checkUpdates(true)} disabled={checkingUpdate} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500 disabled:opacity-50" title="Check for updates">
            {checkingUpdate ? "…" : "Check"}
          </button>
          {updateReady && (
            <button onClick={() => void installUpdate()} className="rounded-lg bg-emerald-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-emerald-500" title={updateReady}>
              Update
            </button>
          )}
          <button onClick={toggle} className="rounded-lg bg-blue-600 px-2.5 py-1.5 text-xs font-bold text-white hover:bg-blue-500" title="Toggle theme">
            {dark ? "☀" : "☾"}
          </button>
        </div>
        {logsOpen && (
          <pre className="mt-2 max-h-32 overflow-y-auto rounded-lg bg-slate-50 p-2 text-[11px] text-slate-500 dark:bg-slate-800/60">
            {logs.length === 0 ? "(no log entries)" : logs.join("\n")}
          </pre>
        )}
      </div>

      {confirm && (
        <Modal title="Confirm" onClose={() => setConfirm(null)}>
          <p className="text-sm">
            {confirm.kind === "remove" && `Remove ${confirm.email}? Quota history is deleted.`}
            {confirm.kind === "archive" && `${confirm.archived ? "Unarchive" : "Archive"} ${confirm.email}?`}
            {confirm.kind === "switch" && `Switch the active Codex account to ${confirm.email}?`}
          </p>
          <div className="mt-4 flex justify-end gap-2">
            <button onClick={() => setConfirm(null)} className="rounded-lg bg-slate-200 px-3 py-1.5 text-sm dark:bg-slate-700">Cancel</button>
            <button onClick={() => void doConfirm()} disabled={busy} className="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-bold text-white disabled:opacity-50">Confirm</button>
          </div>
        </Modal>
      )}

      {resetsEmail && (
        <Modal title={`Reset Credits — ${resetsEmail}`} onClose={() => { setResetsEmail(null); setResets(null); }}>
          {!resets?.credits?.length ? (
            <p className="text-sm text-slate-500">No reset credits found. Fetch quota first.</p>
          ) : (
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="text-xs text-slate-500">
                  <th className="py-1">Expires</th>
                  <th>Remaining</th>
                  <th>Granted</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {resets.credits.map((c, i) => (
                  <tr key={i} className="border-t border-slate-100 dark:border-slate-800">
                    <td className="py-1">{formatCreditExpires(c.expires_at, now)}</td>
                    <td>{formatTimeRemaining(c.expires_at, now)}</td>
                    <td>{formatGrantedAt(c.granted_at)}</td>
                    <td>{c.status ?? "unknown"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {soonest && (
            <p className="mt-2 text-xs text-slate-500" title="Soonest expiring available credit">
              Soonest expiry in {formatTimeRemaining(soonest.expires_at, now)}.
            </p>
          )}
        </Modal>
      )}

      {loginOpen && (
        <Modal title="Add account — Codex login" onClose={() => { if (!loginDone) void api.loginCancel(); setLoginOpen(false); }}>
          <pre className="max-h-56 min-h-24 overflow-y-auto rounded-lg bg-slate-950 p-2 text-xs text-slate-200">
            {loginLines.length === 0 ? "Starting login…" : loginLines.join("\n")}
          </pre>
          {loginDone ? (
            <div className="mt-3 flex justify-end">
              <button onClick={() => setLoginOpen(false)} className="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-bold text-white">Close</button>
            </div>
          ) : (
            <div className="mt-3 flex justify-end">
              <button onClick={() => { void api.loginCancel(); }} className="rounded-lg bg-slate-200 px-3 py-1.5 text-sm dark:bg-slate-700">Cancel</button>
            </div>
          )}
        </Modal>
      )}

      {switchInfo && (
        <Modal title="Switched account" onClose={() => setSwitchInfo(null)}>
          <p className="text-sm">Now using {switchInfo}. Restart the Codex app so it picks up the new auth?</p>
          <div className="mt-4 flex justify-end gap-2">
            <button onClick={() => setSwitchInfo(null)} className="rounded-lg bg-slate-200 px-3 py-1.5 text-sm dark:bg-slate-700">Later</button>
            <button
              onClick={() => { void api.restartCodex().then((m) => setStatus(m)).catch((e) => setStatus(`Restart failed: ${e}`)); setSwitchInfo(null); }}
              className="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-bold text-white"
            >
              Restart Codex
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}

function Modal({ title, children, onClose }: { title: string; children: React.ReactNode; onClose: () => void }) {
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div
        className="w-full max-w-xl rounded-2xl border border-slate-200 bg-white p-4 shadow-xl dark:border-slate-700 dark:bg-slate-900"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-2 text-base font-bold">{title}</h2>
        {children}
      </div>
    </div>
  );
}
