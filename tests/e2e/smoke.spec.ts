import { test, expect } from "@playwright/test";

// The app shell must render in a plain browser even without the Tauri backend
// (the device list and identity show their "IPC unavailable" states, but the
// chrome and the primary action are intact).
test("app shell renders the brand and the record action", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("SundayStudio").first()).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Record test tone/ }),
  ).toBeVisible();
});
