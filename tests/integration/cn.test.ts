// Integration smoke — the `lib` (utility) layer.
import { describe, it, expect } from "vitest";

import { cn } from "@/lib/cn";

describe("cn", () => {
  it("dedupes conflicting tailwind classes (last wins)", () => {
    expect(cn("p-2", "p-4")).toBe("p-4");
  });

  it("drops falsy values", () => {
    const hidden = false;
    expect(cn("text-sm", hidden && "hidden", "font-bold")).toBe(
      "text-sm font-bold",
    );
  });
});
