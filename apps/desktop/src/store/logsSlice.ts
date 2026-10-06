import { createAsyncThunk, createSlice } from "@reduxjs/toolkit";
import { api } from "../lib/tauri";
import { setBusy, setStatus } from "./uiSlice";
import type { RootState } from "./store";

interface LogsState {
  open: boolean;
  entries: string[];
  loading: boolean;
}

const initialState: LogsState = { open: true, entries: [], loading: false };

export const refreshLogs = createAsyncThunk<void, void, { state: RootState }>(
  "logs/refresh",
  async (_, { getState, dispatch }) => {
    if (!getState().logs.open) return;
    try {
      const entries = await api.logs();
      dispatch(setEntries(entries));
    } catch (e) {
      dispatch(setStatus(`Could not load logs: ${e}`));
    }
  },
);

export const toggleLogs = createAsyncThunk<void, void, { state: RootState }>(
  "logs/toggle",
  async (_, { getState, dispatch }) => {
    const next = !getState().logs.open;
    dispatch(setOpen(next));
    dispatch(setBusy({ key: "logs:toggle", on: true }));
    try {
      await api.saveLogsExpanded(next);
    } catch {
    }
    if (next) await dispatch(refreshLogs()).unwrap().catch(() => undefined);
    dispatch(setBusy({ key: "logs:toggle", on: false }));
  },
);

export const clearLogs = createAsyncThunk<void, void, { state: RootState }>(
  "logs/clear",
  async (_, { dispatch }) => {
    dispatch(setBusy({ key: "logs:clear", on: true }));
    try {
      await api.clearLogs();
      dispatch(setEntries([]));
    } catch (e) {
      dispatch(setStatus(`Clear logs failed: ${e}`));
    } finally {
      dispatch(setBusy({ key: "logs:clear", on: false }));
    }
  },
);

const logsSlice = createSlice({
  name: "logs",
  initialState,
  reducers: {
    setOpen(state, action: { payload: boolean }) {
      state.open = action.payload;
    },
    setEntries(state, action: { payload: string[] }) {
      state.entries = action.payload;
    },
  },
});

export const { setOpen, setEntries } = logsSlice.actions;
export default logsSlice.reducer;
