/**
 * Editor UI state (Phase 3.1): zoom, playhead, selection, snapping. This is
 * view-only state — the authoritative regions/takes live in the backend and are
 * fetched through `ipc.edit`. Kept separate from the session store so opening the
 * editor doesn't disturb the open-project snapshot.
 */
import { create } from "zustand";

import { clampZoom, DEFAULT_PX_PER_SEC, ZOOM_FACTOR } from "@/lib/timeline";

interface EditorStore {
  /** Zoom, in pixels per second of audio. */
  pxPerSec: number;
  /** Playhead position in milliseconds. */
  playheadMs: number;
  /** The selected region, or null. */
  selectedRegionId: string | null;
  /** Whether drags snap to region edges / markers / playhead. */
  snapEnabled: boolean;

  setZoom: (pxPerSec: number) => void;
  zoomIn: () => void;
  zoomOut: () => void;
  setPlayhead: (ms: number) => void;
  select: (regionId: string | null) => void;
  toggleSnap: () => void;
}

export const useEditor = create<EditorStore>((set) => ({
  pxPerSec: DEFAULT_PX_PER_SEC,
  playheadMs: 0,
  selectedRegionId: null,
  snapEnabled: true,

  setZoom: (pxPerSec) => set({ pxPerSec: clampZoom(pxPerSec) }),
  zoomIn: () => set((s) => ({ pxPerSec: clampZoom(s.pxPerSec * ZOOM_FACTOR) })),
  zoomOut: () =>
    set((s) => ({ pxPerSec: clampZoom(s.pxPerSec / ZOOM_FACTOR) })),
  setPlayhead: (ms) => set({ playheadMs: Math.max(0, ms) }),
  select: (regionId) => set({ selectedRegionId: regionId }),
  toggleSnap: () => set((s) => ({ snapEnabled: !s.snapEnabled })),
}));
