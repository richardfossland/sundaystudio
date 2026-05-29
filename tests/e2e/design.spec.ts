import { test, expect } from "@playwright/test";

// Visual smoke for the /design route. Navigates via the in-app button (the app
// is a single-window SPA), asserts the primitives are present, and captures a
// full-page screenshot artifact for eyeballing the look.
test("design system renders all primitives", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: /Design system/ }).click();

  await expect(
    page.getByRole("heading", { name: "RecordButton" }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "LevelMeter" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "TrackHeader" }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "JingleCard" })).toBeVisible();

  // Give the simulated meters/animations a beat to settle.
  await page.waitForTimeout(400);
  // Artifact for eyeballing — test-results/ is gitignored and cross-platform.
  await page.screenshot({
    path: "test-results/design.png",
    fullPage: true,
  });
});
