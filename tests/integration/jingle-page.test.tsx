// Integration — the Jingle page wires the generation form to a gallery of
// generated jingles, with preview / regenerate / rename / delete per card.
// The `ai_jingle_generate` IPC is mocked so the test runs offline (no Pro key,
// no network, no Suno), and HTMLMediaElement.play is stubbed for jsdom.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  within,
  cleanup,
} from "@testing-library/react";

import type { JingleResult } from "@/lib/bindings";

const { generateJingleMock } = vi.hoisted(() => ({
  generateJingleMock: vi.fn(),
}));

vi.mock("@/lib/ipc", () => ({
  ipc: { ai: { generateJingle: generateJingleMock } },
}));

import { JinglePage } from "@/features/jingle/JinglePage";
import { useI18n } from "@/lib/i18n";

function result(overrides: Partial<JingleResult> = {}): JingleResult {
  return {
    audio_url: "https://example.test/jingle.wav",
    model: "suno-v3",
    duration_sec: 30,
    title: "Sunday Morning Opener",
    image_url: null,
    ...overrides,
  };
}

/** Fill the form's required fields and click "Generate". */
function fillAndGenerate(title = "Sunday Morning Opener") {
  fireEvent.change(screen.getByPlaceholderText("Sunday Morning Opener"), {
    target: { value: title },
  });
  fireEvent.click(screen.getByRole("button", { name: /generate jingle/i }));
}

describe("JinglePage", () => {
  beforeEach(() => {
    generateJingleMock.mockReset();
    // English labels so the test assertions match the catalog strings.
    useI18n.getState().setLang("en");
    // jsdom has no media engine — make play/pause resolvable no-ops.
    vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  });

  afterEach(() => cleanup());

  it("shows the empty gallery before anything is generated", () => {
    render(<JinglePage />);
    expect(screen.getByText(/no jingles yet/i)).toBeInTheDocument();
  });

  it("submitting the form triggers ai_jingle_generate and renders a card", async () => {
    generateJingleMock.mockResolvedValue(result());
    render(<JinglePage />);

    fillAndGenerate();

    // The mocked generate was called with the spec the form assembled.
    await screen.findByRole("heading", { name: "Sunday Morning Opener" });
    expect(generateJingleMock).toHaveBeenCalledOnce();
    const spec = generateJingleMock.mock.calls[0][0];
    expect(spec.title).toBe("Sunday Morning Opener");
    expect(spec.duration_sec).toBe(30);

    // The gallery now counts one generated jingle and the metadata shows.
    expect(screen.getByText("1 generated")).toBeInTheDocument();
    expect(screen.getByText("suno-v3")).toBeInTheDocument();
  });

  it("play streams the generated audio via the audio element", async () => {
    generateJingleMock.mockResolvedValue(result());
    render(<JinglePage />);
    fillAndGenerate();
    await screen.findByRole("heading", { name: "Sunday Morning Opener" });

    const card = screen.getByRole("listitem");
    const audio = within(card).getByTestId("jingle-audio") as HTMLAudioElement;
    expect(audio.getAttribute("src")).toBe("https://example.test/jingle.wav");

    fireEvent.click(within(card).getByRole("button", { name: /^play$/i }));
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledOnce();
  });

  it("regenerate calls generateJingle again with the original spec", async () => {
    generateJingleMock.mockResolvedValueOnce(result());
    render(<JinglePage />);
    fillAndGenerate();
    await screen.findByRole("heading", { name: "Sunday Morning Opener" });

    generateJingleMock.mockResolvedValueOnce(
      result({ model: "suno-v4", title: "Sunday Morning Opener" }),
    );
    const card = screen.getByRole("listitem");
    fireEvent.click(within(card).getByRole("button", { name: /regenerate/i }));

    await screen.findByText("suno-v4");
    expect(generateJingleMock).toHaveBeenCalledTimes(2);
    // Same spec object that produced the original card.
    expect(generateJingleMock.mock.calls[1][0].title).toBe(
      "Sunday Morning Opener",
    );
    // Still a single card — regenerate replaces in place, not appends.
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  it("delete removes the jingle from the gallery", async () => {
    generateJingleMock.mockResolvedValue(result());
    render(<JinglePage />);
    fillAndGenerate();
    await screen.findByRole("heading", { name: "Sunday Morning Opener" });

    const card = screen.getByRole("listitem");
    fireEvent.click(within(card).getByRole("button", { name: /^delete /i }));

    expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
    expect(screen.getByText(/no jingles yet/i)).toBeInTheDocument();
  });

  it("rename updates the card title", async () => {
    generateJingleMock.mockResolvedValue(result());
    render(<JinglePage />);
    fillAndGenerate();
    await screen.findByRole("heading", { name: "Sunday Morning Opener" });

    const card = screen.getByRole("listitem");
    fireEvent.click(within(card).getByRole("button", { name: /^rename$/i }));
    const input = within(card).getByLabelText(/rename/i);
    fireEvent.change(input, { target: { value: "Easter Special" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(
      screen.getByRole("heading", { name: "Easter Special" }),
    ).toBeInTheDocument();
  });
});
