import { test, expect } from "@playwright/test";

// The Start screen must render in a plain browser even without the Tauri
// backend (templates load only in the app, but the chrome is intact).
test("start screen renders the brand and the new-podcast prompt", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByText("SundayStudio").first()).toBeVisible();
  await expect(
    page.getByRole("heading", { name: /Start a new podcast/ }),
  ).toBeVisible();
});
