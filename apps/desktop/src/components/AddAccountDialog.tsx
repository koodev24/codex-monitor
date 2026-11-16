import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Check, FileJson, LogIn, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { api } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import {
  cancelLogin,
  openLoginUrlOrDefault,
  resetLines,
  setError as setLoginError,
  setOpen,
  startLogin,
} from "@/store/loginSlice";
import { refreshLogs } from "@/store/logsSlice";
import { loadSnapshot } from "@/store/snapshotSlice";
import { setStatus } from "@/store/uiSlice";

type Tab = "login" | "import";

function Spinner() {
  return (
    <span className="relative inline-flex size-4 shrink-0" aria-hidden="true">
      <span className="absolute inset-0 rounded-full border-2 border-current/25" />
      <span className="absolute inset-0 animate-spin rounded-full border-2 border-transparent border-t-current" />
    </span>
  );
}

export function AddAccountDialog() {
  const dispatch = useAppDispatch();
  const isOpen = useAppSelector((s) => s.login.open);
  const starting = useAppSelector((s) => s.login.starting);
  const url = useAppSelector((s) => s.login.url);
  const loginError = useAppSelector((s) => s.login.error);

  const [tab, setTab] = useState<Tab>("login");
  const [path, setPath] = useState<string | null>(null);
  const [pasted, setPasted] = useState("");
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<number | null>(null);

  const waiting = !!url && !loginError;
  const busy = starting || importing;

  // Every open starts clean: no stale error from a previous attempt, no file
  // left selected, and the login tab in front.
  useEffect(() => {
    if (!isOpen) return;
    setTab("login");
    setPath(null);
    setPasted("");
    setImportError(null);
    setCopied(false);
    dispatch(setLoginError(null));
  }, [isOpen, dispatch]);

  useEffect(
    () => () => {
      if (copiedTimer.current) window.clearTimeout(copiedTimer.current);
    },
    [],
  );

  function close() {
    // An import writes the session before it returns; dismissing halfway
    // would hide the error and leave the user unsure whether it landed.
    if (importing) return;
    if (starting || waiting) void dispatch(cancelLogin());
    dispatch(setOpen(false));
  }

  function selectTab(next: Tab) {
    if (next === tab) return;
    if (waiting) void dispatch(cancelLogin());
    setCopied(false);
    setTab(next);
    setImportError(null);
  }

  function start() {
    if (busy) return;
    dispatch(resetLines());
    void dispatch(startLogin());
  }

  async function copyUrl() {
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      if (copiedTimer.current) window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(false), 1800);
    } catch {
      dispatch(setLoginError("Clipboard unavailable — select the link and copy it manually."));
    }
  }

  async function pickFile() {
    setImportError(null);
    const selected = await open({
      multiple: false,
      title: "Select a Codex auth.json file",
      filters: [{ name: "Codex auth file", extensions: ["json"] }],
    });
    if (typeof selected === "string") setPath(selected);
  }

  async function runImport() {
    const text = pasted.trim();
    if ((!path && !text) || importing) return;
    setImporting(true);
    setImportError(null);
    try {
      const result = path
        ? await api.importAuthJson(path)
        : await api.importAuthJsonText(text);
      dispatch(setStatus(result.message));
      // Seed the new row with real quota data; a failed fetch must not undo
      // the import since the session is already saved.
      try {
        await api.fetchBackup(result.email);
      } catch {
        // The row stays visible and can be fetched again with the refresh button.
      }
      await dispatch(loadSnapshot());
      void dispatch(refreshLogs());
      setPath(null);
      setPasted("");
      dispatch(setOpen(false));
    } catch (error) {
      setImportError(String(error));
    } finally {
      setImporting(false);
    }
  }

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="max-h-[calc(100dvh-2rem)] max-w-[30rem] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add account</DialogTitle>
          <DialogDescription>
            Sign in with ChatGPT or import a saved Codex session. Adding never
            switches your active account or restarts Codex.
          </DialogDescription>
        </DialogHeader>

        <div
          role="tablist"
          aria-label="How to add an account"
          className="grid grid-cols-2 gap-1 rounded-xl bg-muted p-1"
        >
          {(
            [
              ["login", "ChatGPT login", LogIn],
              ["import", "Import file", FileJson],
            ] as const
          ).map(([value, label, Icon]) => (
            <button
              key={value}
              role="tab"
              type="button"
              aria-selected={tab === value}
              onClick={() => selectTab(value)}
              className={cn(
                "flex h-8 items-center justify-center gap-1.5 rounded-lg text-xs font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
                tab === value
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              <Icon className="size-3.5" aria-hidden />
              {label}
            </button>
          ))}
        </div>

        {tab === "login" ? (
          <div role="tabpanel" className="min-h-40 min-w-0">
            {waiting ? (
              <div className="flex w-full min-w-0 flex-col items-center gap-3 py-3 text-center">
                <span className="grid size-10 place-items-center rounded-full bg-primary/10 text-primary">
                  <Spinner />
                </span>
                <div>
                  <p className="text-sm font-medium">Waiting for sign-in…</p>
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    Finish the ChatGPT login in your browser. This window updates
                    by itself.
                  </p>
                </div>
                <div className="flex w-full min-w-0 items-center gap-1 overflow-hidden rounded-lg border bg-muted/40 p-1 pl-2.5">
                  <Tooltip>
                    <TooltipTrigger
                      render={
                        <span
                          className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground"
                          tabIndex={0}
                        >
                          {url}
                        </span>
                      }
                    />
                    <TooltipContent className="max-w-96 [overflow-wrap:anywhere]">{url}</TooltipContent>
                  </Tooltip>
                  <Button size="xs" variant="ghost" onClick={() => void copyUrl()}>
                    {copied ? <Check /> : null}
                    {copied ? "Copied" : "Copy"}
                  </Button>
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() => void dispatch(openLoginUrlOrDefault(url))}
                  >
                    <ExternalLink />
                    Open
                  </Button>
                </div>
                <p className="text-[11px] leading-relaxed text-muted-foreground">
                  “Open” uses a private browser window when one is available, so
                  you can pick a different ChatGPT account.
                </p>
              </div>
            ) : loginError ? (
              <div className="max-h-32 overflow-y-auto rounded-xl border border-destructive/30 bg-destructive/8 p-3 text-xs leading-relaxed break-words whitespace-pre-wrap text-destructive [overflow-wrap:anywhere]">
                {loginError}
              </div>
            ) : (
              <div className="flex flex-col items-center gap-2 px-2 py-5 text-center">
                <span className="grid size-10 place-items-center rounded-full bg-muted text-muted-foreground">
                  <LogIn className="size-4.5" aria-hidden />
                </span>
                <p className="text-sm font-medium">Sign in with ChatGPT</p>
                <p className="max-w-xs text-xs leading-relaxed text-muted-foreground">
                  Starts the official Codex browser login and saves the session.
                  Your active account stays active.
                </p>
              </div>
            )}
          </div>
        ) : (
          <div role="tabpanel" className="min-h-40 min-w-0">
            <div className="flex flex-col gap-1.5">
              <span className="text-xs font-medium">auth.json file</span>
              <button
                type="button"
                onClick={() => void pickFile()}
                className="group flex h-11 w-full min-w-0 items-center gap-2 overflow-hidden rounded-lg border border-dashed bg-muted/20 px-3 text-left transition-colors hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                <FileJson className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                {path ? (
                  <Tooltip>
                    <TooltipTrigger
                      render={
                        <span className="min-w-0 flex-1 truncate font-mono text-xs" tabIndex={0}>
                          {path}
                        </span>
                      }
                    />
                    <TooltipContent className="max-w-96 [overflow-wrap:anywhere]">{path}</TooltipContent>
                  </Tooltip>
                ) : (
                  <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                    Choose a Codex auth.json file…
                  </span>
                )}
                <span className="shrink-0 text-[11px] font-medium text-muted-foreground group-hover:text-foreground">
                  Browse
                </span>
              </button>
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                Exported from another machine or a backup? The session is filed
                under the account inside the file. Only ChatGPT sign-ins can be
                imported.
              </p>
            </div>
            <div className="mt-4 flex flex-col gap-1.5">
              <label htmlFor="pasted-auth" className="text-xs font-medium">
                or paste auth.json
              </label>
              <textarea
                id="pasted-auth"
                value={pasted}
                onChange={(event) => {
                  setPasted(event.target.value);
                  setImportError(null);
                }}
                spellCheck={false}
                rows={4}
                placeholder={'{\n  "tokens": { "access_token": "…", "refresh_token": "…" }\n}'}
                className="min-h-20 w-full resize-y rounded-lg border bg-background px-2.5 py-2 font-mono text-[11px] leading-relaxed outline-none placeholder:text-muted-foreground/60 focus-visible:ring-2 focus-visible:ring-ring/50"
              />
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                {path
                  ? "The selected file is imported instead of the pasted text."
                  : "Contents stay on this Mac and are never written to the activity log."}
              </p>
            </div>
            {importError && (
              <div className="mt-3 max-h-32 overflow-y-auto rounded-xl border border-destructive/30 bg-destructive/8 p-3 text-xs leading-relaxed break-words whitespace-pre-wrap text-destructive [overflow-wrap:anywhere]">
                {importError}
              </div>
            )}
          </div>
        )}

        <DialogFooter className="flex-wrap gap-2">
          <Button variant="outline" onClick={close}>
            {waiting ? "Cancel sign-in" : "Cancel"}
          </Button>
          {tab === "login" ? (
            <Button onClick={start} disabled={busy || waiting}>
              {starting ? <Spinner /> : <LogIn />}
              {starting ? "Starting…" : loginError ? "Try again" : "Start sign-in"}
            </Button>
          ) : (
            <Button
              onClick={() => void runImport()}
              disabled={(!path && !pasted.trim()) || busy}
            >
              {importing ? <Spinner /> : <FileJson />}
              {importing
                ? "Importing…"
                : path
                  ? "Import file"
                  : pasted.trim()
                    ? "Import pasted"
                    : "Import account"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
