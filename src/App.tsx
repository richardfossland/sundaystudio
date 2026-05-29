import { useState } from "react";

import { HomePage } from "@/features/home/HomePage";
import { DesignPage } from "@/features/design/DesignPage";

export type Route = "home" | "design";

/**
 * Phase 0.1/0.3 shell. Two routes for now — the home smoke screen and the
 * living design system. The real navigation (record, edit, jingle, export,
 * settings) arrives with those features in later phases.
 */
function App() {
  const [route, setRoute] = useState<Route>("home");

  return (
    <div className="min-h-screen bg-[var(--color-bg)] text-[var(--color-fg)]">
      {route === "design" ? (
        <DesignPage onBack={() => setRoute("home")} />
      ) : (
        <HomePage onOpenDesign={() => setRoute("design")} />
      )}
    </div>
  );
}

export default App;
