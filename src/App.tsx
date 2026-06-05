import { useState } from "react";
import {
  Home,
  Settings,
  Palette,
  Activity,
  Music,
  Mic,
  Scissors,
  FolderX,
} from "lucide-react";

import { StartPage } from "@/features/start/StartPage";
import { RecordingPage } from "@/features/record/RecordingPage";
import { EditPage } from "@/features/edit/EditPage";
import { DesignPage } from "@/features/design/DesignPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { DiagnosticsPage } from "@/features/diagnostics/DiagnosticsPage";
import { JinglePage } from "@/features/jingle/JinglePage";
import {
  CommandPalette,
  type PaletteCommand,
} from "@/components/CommandPalette";
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

  const hasProject = Boolean(snapshot);
  const goto = (next: Route) => setRoute(next);
  const commands: PaletteCommand[] = [
    {
      id: "home",
      label: hasProject ? "Back to project" : "Home / Start",
      group: "Go to",
      icon: Home,
      run: () => goto("main"),
    },
    {
      id: "settings",
      label: "Settings",
      group: "Go to",
      icon: Settings,
      run: () => goto("settings"),
    },
    {
      id: "design",
      label: "Design / Style guide",
      group: "Go to",
      icon: Palette,
      run: () => goto("design"),
    },
    {
      id: "diagnostics",
      label: "Diagnostics",
      group: "Go to",
      icon: Activity,
      run: () => goto("diagnostics"),
    },
    {
      id: "jingle",
      label: "Jingle studio",
      group: "Go to",
      icon: Music,
      run: () => goto("jingle"),
    },
    {
      id: "view-record",
      label: "Recording workspace",
      group: "Project",
      icon: Mic,
      disabled: !hasProject,
      run: () => {
        setProjectView("record");
        goto("main");
      },
    },
    {
      id: "view-edit",
      label: "Editing workspace",
      group: "Project",
      icon: Scissors,
      disabled: !hasProject,
      run: () => {
        setProjectView("edit");
        goto("main");
      },
    },
    {
      id: "close-project",
      label: "Close project",
      group: "Project",
      icon: FolderX,
      disabled: !hasProject,
      run: closeProject,
    },
  ];

  return (
    <div className="min-h-screen bg-[var(--color-bg)] text-[var(--color-fg)]">
      {content}
      <CommandPalette commands={commands} />
    </div>
  );
}

export default App;
