/**
 * The open-project session (UI state). The backend owns the authoritative
 * open project; this store mirrors the latest snapshot so the shell can decide
 * what to show (Start gallery vs the recording page) and screens can read the
 * project without re-fetching.
 */
import { create } from "zustand";

import type { ProjectSnapshot } from "./bindings";

interface SessionStore {
  snapshot: ProjectSnapshot | null;
  setSnapshot: (snapshot: ProjectSnapshot) => void;
  close: () => void;
}

export const useSession = create<SessionStore>((set) => ({
  snapshot: null,
  setSnapshot: (snapshot) => set({ snapshot }),
  close: () => set({ snapshot: null }),
}));
