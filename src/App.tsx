import { useState } from "react";

import { StartPage } from "@/features/start/StartPage";
import { RecordingPage } from "@/features/record/RecordingPage";
import { DesignPage } from "@/features/design/DesignPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { HomePage } from "@/features/home/HomePage";
import { useSession } from "@/lib/session";

/** Overlay routes reachable from the main flow. "main" defers to the session:
 *  the recording page when a project is open, the Start gallery otherwise. */
type Route = "main" | "design" | "settings" | "diagnostics";

function App() {
  const [route, setRoute] = useState<Route>("main");
  const snapshot = useSession((s) => s.snapshot);
  const closeProject = useSession((s) => s.close);
  const back = () => setRoute("main");

  let content;
  if (route === "design") {
    content = <DesignPage onBack={back} />;
  } else if (route === "settings") {
    content = <SettingsPage onBack={back} />;
  } else if (route === "diagnostics") {
    content = <HomePage onBack={back} />;
  } else if (snapshot) {
    content = <RecordingPage onBack={closeProject} />;
  } else {
    content = (
      <StartPage
        onOpenSettings={() => setRoute("settings")}
        onOpenDesign={() => setRoute("design")}
        onOpenDiagnostics={() => setRoute("diagnostics")}
      />
    );
  }

  return (
    <div className="min-h-screen bg-[var(--color-bg)] text-[var(--color-fg)]">
      {content}
    </div>
  );
}

export default App;
