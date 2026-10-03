import { createAsyncThunk, createSlice } from "@reduxjs/toolkit";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../lib/tauri";
import { setBusy, setStatus } from "./uiSlice";
import type { RootState } from "./store";

interface LoginState {
  open: boolean;
  lines: string[];
  done: string | null;
  starting: boolean;
  urlOpened: boolean;
  url: string | null;
}

const initialState: LoginState = {
  open: false,
  lines: [],
  done: null,
  starting: false,
  urlOpened: false,
  url: null,
};

export const openLoginUrlOrDefault = createAsyncThunk<void, string, { state: RootState }>(
  "login/openUrl",
  async (url, { dispatch }) => {
    try {
      dispatch(setStatus(await api.openLoginUrl(url)));
    } catch {
      dispatch(setStatus("Opened login page in the default browser."));
      void openUrl(url).catch(() => undefined);
    }
  },
);

export const copyLoginUrl = createAsyncThunk<void, string, { state: RootState }>(
  "login/copyUrl",
  async (url, { dispatch }) => {
    try {
      await navigator.clipboard.writeText(url);
      dispatch(setStatus("Login URL copied — paste it into a private window."));
    } catch {
      dispatch(setStatus("Could not copy the URL. Select it from the log above."));
    }
  },
);

export const startLogin = createAsyncThunk<void, void, { state: RootState }>(
  "login/start",
  async (_, { getState, dispatch }) => {
    if (getState().login.starting) return;
    dispatch(setStarting(true));
    dispatch(resetLines());
    dispatch(setOpen(true));
    try {
      const msg = await api.loginStart();
      dispatch(setStatus(msg));
    } catch (e) {
      dispatch(appendLines([`Failed to start login: ${e}`]));
    } finally {
      dispatch(setStarting(false));
    }
  },
);

export const cancelLogin = createAsyncThunk<void, void, { state: RootState }>(
  "login/cancel",
  async (_, { dispatch }) => {
    dispatch(setBusy({ key: "login:cancel", on: true }));
    try {
      await api.loginCancel();
    } catch (e) {
      dispatch(setStatus(`Cancel failed: ${e}`));
    } finally {
      dispatch(setBusy({ key: "login:cancel", on: false }));
    }
  },
);

const loginSlice = createSlice({
  name: "login",
  initialState,
  reducers: {
    setOpen(state, action: { payload: boolean }) {
      state.open = action.payload;
    },
    setStarting(state, action: { payload: boolean }) {
      state.starting = action.payload;
    },
    resetLines(state) {
      state.lines = [];
      state.done = null;
      state.urlOpened = false;
      state.url = null;
    },
    setUrl(state, action: { payload: string }) {
      state.url = action.payload;
    },
    appendLines(state, action: { payload: string[] }) {
      for (const line of action.payload) {
        if (line !== state.lines[state.lines.length - 1]) {
          state.lines.push(line);
        }
      }
      state.lines = state.lines.slice(-200);
    },
    markUrlOpened(state) {
      state.urlOpened = true;
    },
    setDone(state, action: { payload: string }) {
      state.done = action.payload;
    },
  },
});

export const { setOpen, setStarting, resetLines, appendLines, markUrlOpened, setDone, setUrl } =
  loginSlice.actions;
export default loginSlice.reducer;
