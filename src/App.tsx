import { useState } from "react";

import { HomePage } from "@/features/home/HomePage";
import { DesignPage } from "@/features/design/DesignPage";
import { SettingsPage } from "@/features/settings/SettingsPage";

export type Route = "home" | "design" | "settings";

/**
 * Phase 0/1 shell. Home smoke screen, the living design system, and the audio
 * settings. The full navigation (record, edit, jingle, export) arrives with
 * those features in later phases.
 */
function App() {
  const [route, setRoute] = useState<Route>("home");
  const back = () => setRoute("home");

  return (
    <div className="min-h-screen bg-[var(--color-bg)] text-[var(--color-fg)]">
      {route === "design" ? (
        <DesignPage onBack={back} />
      ) : route === "settings" ? (
        <SettingsPage onBack={back} />
      ) : (
        <HomePage
          onOpenDesign={() => setRoute("design")}
          onOpenSettings={() => setRoute("settings")}
        />
      )}
    </div>
  );
}

export default App;
