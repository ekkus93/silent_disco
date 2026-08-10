import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

// Block 37.1 "add frontend build flag derived from backend capability,
// not only JavaScript environment": `available` is set only from the
// backend's own `get_lab_mode_available` command result -- never derived
// from `import.meta.env`/`process.env` or any other JS-only signal, so a
// dev-mode frontend build alone can never show Lab UI a production Rust
// binary did not actually compile in.
export interface LabState {
  // `null` until the backend has answered -- distinct from `false`, so
  // the UI never briefly claims Lab Mode is unavailable before it has
  // actually asked.
  available: boolean | null;
}

const initialState: LabState = {
  available: null,
};

export const labSlice = createSlice({
  name: "lab",
  initialState,
  reducers: {
    labModeAvailabilityReceived(state, action: PayloadAction<boolean>) {
      state.available = action.payload;
    },
  },
});

export const labActions = labSlice.actions;
export default labSlice.reducer;
