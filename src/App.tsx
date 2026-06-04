import { useState } from "react";

import { StartPage } from "@/features/start/StartPage";
import { RecordingPage } from "@/features/record/RecordingPage";
import { EditPage } from "@/features/edit/EditPage";
import { DesignPage } from "@/features/design/DesignPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { DiagnosticsPage } from "@/features/diagnostics/DiagnosticsPage";
import { JinglePage } from "@/features/jingle/JinglePage";
import { useSession } from "@/lib/session";

/** Overlay routes reachable from the main flow. "main" defers to the session:
 *  the recording page when a project is open, the Start gallery otherwise. */
type Route = "main" | "design" | "settings" | "diagnostics" | "jingle";

/** When a project is open, which workspace is showing. */
type ProjectView = "record" | "edit";

function App() {
  const [route, setRoute] = useState<Route>("main");
  const [projectView, setProjectView] = useState<ProjectView>("record");
  const snapshot = useSession((s) => s.snapshot);
  const close = useSession((s) => s.close);
  const back = () => setRoute("main");
  const closeProject = () => {
    setProjectView("record");
    close();
  };

  let content;
  if (route === "design") {
    content = <DesignPage onBack={back} />;
  } else if (route === "settings") {
    content = <SettingsPage onBack={back} />;
  } else if (route === "diagnostics") {
    content = <DiagnosticsPage onBack={back} />;
  } else if (route === "jingle") {
    content = <JinglePage onBack={back} />;
  } else if (snapshot) {
    content =
      projectView === "edit" ? (
        <EditPage
          onBack={closeProject}
          onOpenRecord={() => setProjectView("record")}
        />
      ) : (
        <RecordingPage
          onBack={closeProject}
          onOpenEdit={() => setProjectView("edit")}
        />
      );
  } else {
    content = (
      <StartPage
        onOpenSettings={() => setRoute("settings")}
        onOpenDesign={() => setRoute("design")}
        onOpenDiagnostics={() => setRoute("diagnostics")}
        onOpenJingle={() => setRoute("jingle")}
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
