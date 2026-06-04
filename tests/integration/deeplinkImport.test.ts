/**
 * Deep-link import seeding — the parse → project → take wiring.
 *
 * Exercises `seedProjectFromImportLink` against a mocked `ipc` so the full
 * sequence (parse the link, create the project, import the take) is verified
 * without Tauri, a DB, or audio hardware.
 */
import { describe, it, expect, vi } from "vitest";

import {
  projectNameFromPath,
  seedProjectFromImportLink,
  type ImportSeederIpc,
} from "@/lib/deeplinkImport";
import type { ImportRequest, ProjectMeta } from "@/lib/bindings";

function makeIpc(overrides?: {
  parseImport?: (url: string) => Promise<ImportRequest>;
}) {
  const parseImport = vi.fn(
    overrides?.parseImport ??
      (async (_url: string): Promise<ImportRequest> => ({
        path: "/Users/ola/sermon.wav",
        return_to: "sundayrec",
      })),
  );
  const projectNew = vi.fn(
    async (name: string): Promise<ProjectMeta> => ({
      id: "proj-1",
      name,
      path: `/data/projects/${name}.scast`,
      created_at: 0,
      updated_at: 0,
    }),
  );
  const importTakes = vi.fn(async (_paths: string[]) => ({}) as never);

  const client = {
    deeplink: { parseImport },
    project: { new: projectNew },
    edit: { importTakes },
  } as unknown as ImportSeederIpc;

  return { client, parseImport, projectNew, importTakes };
}

describe("projectNameFromPath", () => {
  it("uses the basename without extension", () => {
    expect(projectNameFromPath("/Users/ola/My Sermon.wav")).toBe("My Sermon");
    expect(projectNameFromPath("C:\\rec\\take.flac")).toBe("take");
  });

  it("falls back to a default for an empty/extensionless name", () => {
    expect(projectNameFromPath("")).toBe("Imported recording");
    expect(projectNameFromPath("/a/.wav")).toBe("Imported recording");
  });
});

describe("seedProjectFromImportLink", () => {
  it("parses the link, creates a project, and imports the take in order", async () => {
    const { client, parseImport, projectNew, importTakes } = makeIpc();

    const meta = await seedProjectFromImportLink(
      client,
      "sundaystudio://import?path=%2FUsers%2Fola%2Fsermon.wav&returnTo=sundayrec",
    );

    expect(parseImport).toHaveBeenCalledWith(
      "sundaystudio://import?path=%2FUsers%2Fola%2Fsermon.wav&returnTo=sundayrec",
    );
    // Project named from the parsed path's basename.
    expect(projectNew).toHaveBeenCalledWith("sermon");
    // The parsed file is laid onto the (now current) project as a take.
    expect(importTakes).toHaveBeenCalledWith(["/Users/ola/sermon.wav"]);
    expect(meta.name).toBe("sermon");

    // Ordering: parse before create before import.
    const parseOrder = parseImport.mock.invocationCallOrder[0];
    const newOrder = projectNew.mock.invocationCallOrder[0];
    const importOrder = importTakes.mock.invocationCallOrder[0];
    expect(parseOrder).toBeLessThan(newOrder);
    expect(newOrder).toBeLessThan(importOrder);
  });

  it("propagates a parse failure without creating a project", async () => {
    const { client, projectNew, importTakes } = makeIpc({
      parseImport: async () => {
        throw new Error("not a sundaystudio:// link");
      },
    });

    await expect(
      seedProjectFromImportLink(client, "https://example.com"),
    ).rejects.toThrow("not a sundaystudio:// link");

    expect(projectNew).not.toHaveBeenCalled();
    expect(importTakes).not.toHaveBeenCalled();
  });
});
