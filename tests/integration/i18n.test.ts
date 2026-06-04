/**
 * i18n tests — catalog completeness, fallback behaviour, store API.
 */

import { describe, expect, it, beforeEach } from "vitest";

import { LANGS, t, useI18n, type Lang } from "@/lib/i18n";

// Reset store lang to "no" before each test so tests are isolated.
beforeEach(() => {
  useI18n.setState({ lang: "no" });
});

describe("i18n: store API", () => {
  it("defaults to Norwegian ('no')", () => {
    expect(useI18n.getState().lang).toBe("no");
  });

  it("setLang updates the lang in the store", () => {
    useI18n.getState().setLang("en");
    expect(useI18n.getState().lang).toBe("en");
    // Reset for subsequent tests
    useI18n.getState().setLang("no");
  });

  it("t() returns the Norwegian value for a known key", () => {
    useI18n.setState({ lang: "no" });
    const val = useI18n.getState().t("actionSave");
    expect(val).toBe("Lagre");
  });

  it("t() returns the English value when lang is 'en'", () => {
    useI18n.setState({ lang: "en" });
    const val = useI18n.getState().t("actionSave");
    expect(val).toBe("Save");
  });

  it("t() falls back to English for a key missing in the current locale", () => {
    useI18n.setState({ lang: "sv" });
    // sv catalog doesn't define settingsTitle — should fall back to English
    const val = useI18n.getState().t("settingsTitle");
    expect(val).toBe("Audio device");
  });

  it("t() returns the key itself when no locale has it", () => {
    useI18n.setState({ lang: "no" });
    const val = useI18n.getState().t("__no_such_key__");
    expect(val).toBe("__no_such_key__");
  });
});

describe("i18n: standalone t() helper", () => {
  it("reflects the current store language", () => {
    useI18n.setState({ lang: "no" });
    expect(t("actionCancel")).toBe("Avbryt");
  });

  it("switches when the store lang changes", () => {
    useI18n.setState({ lang: "en" });
    expect(t("actionCancel")).toBe("Cancel");
  });
});

describe("i18n: catalog completeness", () => {
  const EN_KEYS = [
    "appName",
    "appTagline",
    "navRecord",
    "navEdit",
    "navJingle",
    "navSettings",
    "actionSave",
    "actionCancel",
    "actionClose",
    "jingleTitle",
    "jingleFormTitle",
    "jingleFieldTitle",
    "jingleFieldDuration",
    "jingleFieldMood",
    "jingleFieldTempo",
    "jingleFieldInstruments",
    "jingleMoodEnergetic",
    "jingleMoodCalm",
    "jingleMoodWorshipful",
    "jingleMoodProfessional",
    "jingleDuration20",
    "jingleDuration30",
    "jingleDuration60",
    "settingsTitle",
    "settingsInputDevice",
    "settingsOutputDevice",
    "homeProjectsTitle",
    "homeTestToneTitle",
    "recordAddTrack",
    "recordWriterFailed",
    "recordDroppedBadge",
    "recordBackupProject",
    "recordBackupDone",
    "recordBackupFailed",
    "importLinkTitle",
    "importLinkDesc",
    "importLinkButton",
    "importLinkImporting",
    "importLinkDone",
    "importLinkError",
    "importLinkHint",
  ];

  it("English catalog defines all core keys", () => {
    useI18n.setState({ lang: "en" });
    for (const key of EN_KEYS) {
      const v = useI18n.getState().t(key);
      expect(
        v,
        `English key "${key}" must not fall back to the key itself`,
      ).not.toBe(key);
    }
  });

  it("Norwegian catalog defines all core keys (no fallback expected)", () => {
    useI18n.setState({ lang: "no" });
    const noFallbackKeys = [
      "appName",
      "navRecord",
      "navJingle",
      "navSettings",
      "actionSave",
      "actionCancel",
      "jingleTitle",
      "jingleMoodEnergetic",
      "jingleDuration30",
      "settingsTitle",
    ];
    for (const key of noFallbackKeys) {
      const v = useI18n.getState().t(key);
      expect(v, `Norwegian key "${key}" must not fall back`).not.toBe(key);
    }
  });

  it("partial locales don't crash — they return a string for any key", () => {
    const partialLangs: Lang[] = ["sv", "da", "de", "fr", "pl"];
    for (const lang of partialLangs) {
      useI18n.setState({ lang });
      for (const key of EN_KEYS) {
        const v = useI18n.getState().t(key);
        expect(typeof v, `${lang}/${key} must be a string`).toBe("string");
        expect(v.length, `${lang}/${key} must be non-empty`).toBeGreaterThan(0);
      }
    }
  });
});

describe("i18n: jingle-specific keys (Norwegian)", () => {
  beforeEach(() => useI18n.setState({ lang: "no" }));

  it("validates jingle title message is in Norwegian", () => {
    expect(t("jingleValidateTitle")).toContain("påkrevd");
  });

  it("jingle BPM range message is in Norwegian", () => {
    expect(t("jingleValidateBpm")).toContain("200");
  });

  it("jingle Pro notice is in Norwegian", () => {
    expect(t("jingleProNotice")).toContain("Pro");
  });
});

describe("i18n: import-from-link keys (all locales native)", () => {
  const IMPORT_KEYS = [
    "importLinkTitle",
    "importLinkDesc",
    "importLinkPlaceholder",
    "importLinkButton",
    "importLinkImporting",
    "importLinkDone",
    "importLinkError",
    "importLinkHint",
  ];

  it("every locale defines all import-link keys with a non-empty string", () => {
    for (const lang of LANGS) {
      useI18n.setState({ lang });
      for (const key of IMPORT_KEYS) {
        const v = useI18n.getState().t(key);
        expect(typeof v, `${lang}/${key} must be a string`).toBe("string");
        expect(v.length, `${lang}/${key} must be non-empty`).toBeGreaterThan(0);
      }
    }
  });

  it("importLinkDone carries the {name} placeholder for interpolation", () => {
    for (const lang of LANGS) {
      useI18n.setState({ lang });
      expect(
        useI18n.getState().t("importLinkDone"),
        `${lang}/importLinkDone must contain {name}`,
      ).toContain("{name}");
    }
  });
});

describe("i18n: LANGS constant", () => {
  it("contains exactly the 7 expected locales", () => {
    expect(LANGS).toEqual(
      expect.arrayContaining(["no", "en", "sv", "da", "de", "fr", "pl"]),
    );
    expect(LANGS).toHaveLength(7);
  });
});
