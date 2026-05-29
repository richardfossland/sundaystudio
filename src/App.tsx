import { HomePage } from "@/features/home/HomePage";

/**
 * Phase 0.1 shell. A single screen for now — the eventual navigation (record,
 * edit, jingle, export, settings) arrives with those features in later phases.
 */
function App() {
  return (
    <div className="min-h-screen bg-[var(--color-bg)] text-[var(--color-fg)]">
      <HomePage />
    </div>
  );
}

export default App;
