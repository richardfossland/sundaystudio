// Integration smoke — the audio primitives render and reflect their state.
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

import { RecordButton } from "@/components/audio/RecordButton";
import { Timecode } from "@/components/audio/Timecode";
import { LevelMeter } from "@/components/audio/LevelMeter";

describe("RecordButton", () => {
  it("labels itself by state and fires onClick", () => {
    const onClick = vi.fn();
    render(<RecordButton state="recording" onClick={onClick} />);
    const btn = screen.getByRole("button", { name: /stop recording/i });
    fireEvent.click(btn);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("reads as a record control when idle", () => {
    render(<RecordButton state="idle" />);
    expect(
      screen.getByRole("button", { name: /^record$/i }),
    ).toBeInTheDocument();
  });
});

describe("Timecode", () => {
  it("formats the position", () => {
    render(<Timecode ms={3_661_007} />);
    expect(screen.getByText("01:01:01.007")).toBeInTheDocument();
  });

  it("commits a parsed edit and rejects garbage", () => {
    const onCommit = vi.fn();
    render(<Timecode ms={0} onCommit={onCommit} />);
    fireEvent.click(screen.getByText("00:00:00.000"));
    const input = screen.getByRole("textbox");

    fireEvent.change(input, { target: { value: "1:30" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith(90_000);

    // Re-open and submit garbage — onCommit must not fire again.
    fireEvent.click(screen.getByText("00:00:00.000"));
    const again = screen.getByRole("textbox");
    fireEvent.change(again, { target: { value: "nope" } });
    fireEvent.keyDown(again, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledOnce();
  });
});

describe("LevelMeter", () => {
  it("exposes an accessible meter with the current dB", () => {
    render(<LevelMeter db={-12} />);
    const meter = screen.getByRole("meter");
    expect(meter).toHaveAttribute("aria-valuenow", "-12");
    expect(meter).toHaveAttribute("aria-valuemax", "0");
  });
});
