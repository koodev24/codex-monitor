import { createAsyncThunk, createSlice } from "@reduxjs/toolkit";
import { api } from "../lib/tauri";
import { setBusy, setStatus } from "./uiSlice";
import type { RootState } from "./store";

interface LoginState {
  open: boolean;
  lines: string[];
  done: string | null;
  starting: boolean;
  urlOpened: boolean;
}

const initialState: LoginState = {
  open: false,
  lines: [],
  done: null,
  starting: false,
  urlOpened: false,
};

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
    },
    appendLines(state, action: { payload: string[] }) {
      state.lines = [...state.lines, ...action.payload].slice(-200);
    },
    markUrlOpened(state) {
      state.urlOpened = true;
    },
    setDone(state, action: { payload: string }) {
      state.done = action.payload;
    },
  },
});

export const { setOpen, setStarting, resetLines, appendLines, markUrlOpened, setDone } =
  loginSlice.actions;
export default loginSlice.reducer;
