import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

/// The shell's destinations. Exactly one is rendered at a time.
///
/// `host` is the working destination and resolves to either the setup or the
/// session screen from the authoritative host lifecycle -- the shell does not
/// model that split, because the core already owns it. The rest are peer
/// destinations reached from the header.
export type DesktopView = "host" | "diagnostics" | "storage" | "settings" | "lab";

export interface UiState {
  activeView: DesktopView;
}

const initialState: UiState = {
  activeView: "host",
};

export const uiSlice = createSlice({
  name: "ui",
  initialState,
  reducers: {
    activeViewChanged(state, action: PayloadAction<DesktopView>) {
      state.activeView = action.payload;
    },
  },
});

export const uiActions = uiSlice.actions;
export default uiSlice.reducer;
