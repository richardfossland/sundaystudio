// Unit — the shared StatusBanner the feature pages use to surface invoke
// errors and status. Verifies the kind → ARIA role mapping (errors are
// assertive `alert`s; success/info are polite `status`es) and that the message
// and test id render. Renders headlessly via jsdom.
import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";

import { StatusBanner } from "@/components/ui/StatusBanner";

afterEach(() => cleanup());

describe("StatusBanner", () => {
  it("renders the message", () => {
    render(<StatusBanner kind="danger" message="disk full" />);
    expect(screen.getByText("disk full")).toBeInTheDocument();
  });

  it("uses role=alert for danger and critical", () => {
    render(<StatusBanner kind="danger" message="x" testId="d" />);
    render(<StatusBanner kind="critical" message="y" testId="c" />);
    expect(screen.getByTestId("d")).toHaveAttribute("role", "alert");
    expect(screen.getByTestId("c")).toHaveAttribute("role", "alert");
  });

  it("uses role=status for success and info", () => {
    render(<StatusBanner kind="success" message="x" testId="s" />);
    render(<StatusBanner kind="info" message="y" testId="i" />);
    expect(screen.getByTestId("s")).toHaveAttribute("role", "status");
    expect(screen.getByTestId("i")).toHaveAttribute("role", "status");
  });

  it("exposes the test id for targeting", () => {
    render(<StatusBanner kind="info" message="hi" testId="my-banner" />);
    expect(screen.getByTestId("my-banner")).toBeInTheDocument();
  });
});
