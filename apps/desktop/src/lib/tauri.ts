import { invoke } from "@tauri-apps/api/core";
import type { ResetCreditsPayload, UsageMap } from "./format";

export interface Snapshot {
  accounts: UsageMap;
  current_email: string | null;
  auto_fetch: string;
  auto_fetch_options: string[];
  sort_column: string | null;
  sort_asc: boolean;
  show_archived: boolean;
  logs_expanded: boolean;
  auth_file_exists: boolean;
  backup_emails: string[];
  app_version: string;
}

export type AuthOutcome =
  | { kind: "NoChange" }
  | { kind: "Fetched"; email: string; message: string }
  | { kind: "AuthRefreshed"; message: string }
  | { kind: "LoggedOut"; message: string }
  | { kind: "MissingToken" }
  | { kind: "ParseError"; message: string }
  | { kind: "NoFile" };

export interface FetchResult {
  email: string;
  message: string;
}

export interface LoginDone {
  ok: boolean;
  message: string;
}

export const api = {
  ping: () => invoke<string>("ping"),
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  manualFetch: () => invoke<FetchResult>("manual_fetch"),
  fetchBackup: (email: string) => invoke<FetchResult>("fetch_backup", { email }),
  checkAutoFetch: () => invoke<FetchResult | null>("check_auto_fetch"),
  processAuthFile: () => invoke<AuthOutcome>("process_auth_file"),
  switchAccount: (email: string) => invoke<string>("switch_account", { email }),
  removeAccount: (email: string) => invoke<string>("remove_account", { email }),
  setArchived: (email: string, archived: boolean) =>
    invoke<string>("set_archived", { email, archived }),
  saveAutoFetch: (value: string) => invoke<string>("save_auto_fetch", { value }),
  saveSort: (column: string | null, asc: boolean) =>
    invoke<void>("save_sort", { column, asc }),
  saveShowArchived: (show: boolean) => invoke<void>("save_show_archived", { show }),
  saveLogsExpanded: (expanded: boolean) => invoke<void>("save_logs_expanded", { expanded }),
  resets: (email: string) => invoke<ResetCreditsPayload | null>("get_resets", { email }),
  logs: () => invoke<string[]>("get_logs"),
  clearLogs: () => invoke<void>("clear_logs"),
  exportData: (path: string) => invoke<string>("export_data", { path }),
  importData: (path: string) => invoke<string>("import_data", { path }),
  logout: () => invoke<string>("logout"),
  loginStart: () => invoke<string>("login_start"),
  loginCancel: () => invoke<string>("login_cancel"),
  restartCodex: () => invoke<string>("restart_codex"),
};
