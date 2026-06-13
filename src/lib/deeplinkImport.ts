/**
 * Deep-link import seeding (Rec → Studio handoff).
 *
 * Turns a `sundaystudio://import?path=…` URL into a real, ready-to-edit project:
 *
 *   1. `deeplink_parse_import` validates the URL into an `ImportRequest`.
 *   2. `project_new` creates + opens a fresh project (named from the file).
 *   3. `take_import` lays the source file onto the new project's timeline.
 *
 * Steps 2–3 are the same flow the Start screen and recorder use, so the imported
 * take behaves exactly like any other. Kept dependency-free (no React, no Tauri
 * import) so it can be unit-tested against a mocked `ipc`.
 *
 * The OS scheme auto-registration (single-instance + open-url event) is deferred
 * — it needs the bundled app and a real OS event. The paste-a-link entry point
 * on the diagnostics screen makes this whole path reachable and testable today.
 */

import type { ipc as Ipc } from "./ipc";
import type { ImportRequest, ProjectMeta } from "./bindings";

/** The slice of the IPC surface the seeder needs (for easy mocking in tests). */
export type ImportSeederIpc = Pick<typeof Ipc, "deeplink" | "project" | "edit">;

/** The outcome of seeding: the created project plus the parsed handoff, so the
 *  caller can carry `context`/`glossary` into the show-notes panel. */
export interface SeededImport {
  meta: ProjectMeta;
  request: ImportRequest;
}

/** Derive a human project name from an imported file path (basename, no ext). */
export function projectNameFromPath(path: string): string {
  const base = path.split(/[/\\]/).pop() ?? path;
  const stem = base.replace(/\.[^.]+$/, "");
  return stem.trim() || "Imported recording";
}

/**
 * Parse `url` and seed a new project from it, importing the referenced file as
 * the project's first take. Returns the created project plus the parsed handoff
 * (so the caller can seed the show-notes panel's context/glossary).
 *
 * Throws (an `IPCError` from `deeplink_parse_import`) for a malformed link or a
 * missing path — the caller surfaces the message.
 */
export async function seedProjectFromImportLink(
  client: ImportSeederIpc,
  url: string,
): Promise<SeededImport> {
  const request = await client.deeplink.parseImport(url);
  // `project.new` creates AND makes the project current, so the subsequent
  // `take_import` lands on it without an explicit open.
  const meta = await client.project.new(projectNameFromPath(request.path));
  await client.edit.importTakes([request.path]);
  return { meta, request };
}
