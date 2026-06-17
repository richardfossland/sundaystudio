/**
 * The open-project session (UI state). The backend owns the authoritative
 * open project; this store mirrors the latest snapshot so the shell can decide
 * what to show (Start gallery vs the recording page) and screens can read the
 * project without re-fetching.
 *
 * It also holds the editor-curated show-notes chapters (AI-suggested or hand-
 * added) so the export screen can embed them as ffmpeg chapter metadata, plus
 * any context/glossary carried in from a SundayRec deep-link handoff so the
 * show-notes prompt can spell names right.
 */
import { create } from "zustand";

import type { ProjectSnapshot, ShowNotesChapter } from "./bindings";

interface SessionStore {
  snapshot: ProjectSnapshot | null;
  setSnapshot: (snapshot: ProjectSnapshot) => void;
  /** Chapters the user curated in the show-notes panel, embedded on export. */
  chapters: ShowNotesChapter[];
  setChapters: (chapters: ShowNotesChapter[]) => void;
  /** Context primer from a deep-link handoff (e.g. "Sermon, speaker: Ola"). */
  handoffContext: string | null;
  /** Glossary terms (speaker names, jargon) from a deep-link handoff. */
  handoffGlossary: string[];
  setHandoff: (context: string | null, glossary: string[]) => void;
  close: () => void;
}

export const useSession = create<SessionStore>((set) => ({
  snapshot: null,
  setSnapshot: (snapshot) => set({ snapshot }),
  chapters: [],
  setChapters: (chapters) => set({ chapters }),
  handoffContext: null,
  handoffGlossary: [],
  setHandoff: (handoffContext, handoffGlossary) =>
    set({ handoffContext, handoffGlossary }),
  // Closing a project clears its derived editor state too, so chapters/handoff
  // never leak from one project into the next.
  close: () =>
    set({
      snapshot: null,
      chapters: [],
      handoffContext: null,
      handoffGlossary: [],
    }),
}));
