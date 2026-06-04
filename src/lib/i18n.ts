/**
 * i18n — SundayStudio.
 *
 * A tiny, dependency-light translation layer matching the pattern used across
 * the Sunday suite (SundayStage, SundayRec). A per-language catalog, a `t()`
 * that falls back to English for any missing key, and a Zustand locale store.
 *
 * English + Norwegian are complete at launch. Swedish, Danish, German, French
 * and Polish follow the same keys and fall back to English for missing entries
 * — full translation is tracked in docs/LAUNCH_READINESS.md.
 */

import { create } from "zustand";

export type Lang = "no" | "en" | "sv" | "da" | "de" | "fr" | "pl";

export const LANGS: Lang[] = ["no", "en", "sv", "da", "de", "fr", "pl"];

type Catalog = Record<string, string>;

// ── English ──────────────────────────────────────────────────────────────────

const en: Catalog = {
  // ── App shell ────────────────────────────────────────────────────────────
  appName: "SundayStudio",
  appTagline: "Podcast & Jingle Production",

  // ── Navigation / routes ──────────────────────────────────────────────────
  navRecord: "Record",
  navEdit: "Edit",
  navJingle: "Jingle",
  navSettings: "Settings",
  navDiagnostics: "Diagnostics",
  navBack: "Back",
  navHome: "Home",

  // ── Common actions ───────────────────────────────────────────────────────
  actionCancel: "Cancel",
  actionClose: "Close",
  actionSave: "Save",
  actionSaving: "Saving…",
  actionSaved: "Saved",
  actionDelete: "Delete",
  actionEdit: "Edit",
  actionAdd: "Add",
  actionDone: "Done",
  actionNew: "New",
  actionBack: "Back",
  actionOpen: "Open",
  actionRemove: "Remove",
  actionExport: "Export",
  actionExporting: "Exporting…",
  loadingShort: "Loading…",

  // ── Home / diagnostics page ───────────────────────────────────────────────
  homeTitle: "Diagnostics",
  homeProjectsTitle: "Projects",
  homeProjectsDesc: "Create a new project or reopen a registered one.",
  homeProjectNamePlaceholder: "Project name…",
  homeNewProject: "New",
  homeProjectListEmpty: "No projects yet — create one above.",
  homeProjectListLoading: "Loading…",
  homeProjectListError:
    "Project list loads when running in the SundayStudio app.",
  homeProjectOpenLabel: "Open",
  homeProjectRemoveLabel: "Remove {name} from registry",
  homeBackendTitle: "Rust backend",
  homeBackendError: "IPC unavailable (running outside Tauri?)",
  homeAudioDevicesTitle: "Audio devices",
  homeAudioDevicesScanning: "Scanning audio hardware…",
  homeAudioDevicesError:
    "Could not enumerate devices (running outside Tauri or no audio backend).",
  homeInputs: "Inputs",
  homeOutputs: "Outputs",
  homeNoneFound: "None found",
  homeDefaultBadge: "default",
  homeTestToneTitle: "Record test tone",
  homeTestToneDesc:
    "Writes a 1-second 440 Hz sine wave to a WAV on disk — proof the recording-to-file path works.",
  homeWriting: "Writing…",
  homeRecordTestTone: "Record test tone",
  homeWrote: "Wrote",
  homeAnalyzeLoudness: "Analyze loudness",
  homeMeasuring: "Measuring…",
  homeLoudnessTargetsTitle: "Loudness targets",
  homeLoudnessTargetsDesc:
    "Where each platform normalises your show. Master to the target and it plays back at the level you intended.",
  homeLoudnessTargetsError:
    "Targets load when running in the SundayStudio app.",
  homeMasterPresetsTitle: "Mastering presets",
  homeMasterPresetsDesc:
    "One-click master chains (EQ · 3-band compressor · brick-wall limiter) paired with a loudness target.",
  homeMasterPresetsError: "Presets load when running in the SundayStudio app.",
  homeExportPresetsTitle: "Export presets",
  homeExportPresetsDesc:
    "Platform-ready bounce settings. A finished episode renders to a mastered, loudness-normalised file in the project’s exports folder.",
  homeExportPresetsError: "Presets load when running in the SundayStudio app.",
  homeVoicePresetsTitle: "Voice presets",
  homeVoicePresetsDesc:
    "Bundled processing chains (gate · EQ · de-esser · compressor · saturator). They attach to mixer tracks in a later phase.",
  homeVoicePresetsError: "Presets load when running in the SundayStudio app.",
  metricIntegrated: "Integrated",
  metricShortTerm: "Short-term",
  metricMomentary: "Momentary",
  metricRange: "Range",
  metricTruePeak: "True peak",
  metricSamplePeak: "Sample peak",

  // ── Record page ───────────────────────────────────────────────────────────
  recordNoProject: "No project open.",
  recordNoTracks: "No tracks yet.",
  recordAddTrack: "Add track",
  recordTransportNotice:
    "Transport is a preview — live capture wires through the recorder engine on real audio hardware. Track settings and chapters below are saved to the project.",
  recordExported: "Exported",
  recordChapters: "Chapters",
  recordAddChapter: "Add chapter",
  recordWriterFailed:
    "Recording stopped: disk write error — file may be corrupt.",
  recordDroppedBadge: "{count} samples dropped",
  recordBackupProject: "Back up project",
  recordBackupDone: "Project backed up",
  recordBackupFailed: "Backup failed",
  recordActionFailed: "Could not save the change",

  // ── Settings page ─────────────────────────────────────────────────────────
  settingsTitle: "Audio device",
  settingsSubtitle: "Settings · Audio",
  settingsInputDevice: "Input device",
  settingsInputHint: "Microphone or USB interface",
  settingsOutputDevice: "Output device",
  settingsOutputHint: "Headphones or speakers",
  settingsSampleRate: "Sample rate",
  settingsBufferSize: "Buffer size",
  settingsBufferHint: "Lower = less latency",
  settingsLatencyTitle: "Estimated round-trip latency",
  settingsLatencyDesc:
    "Input + output buffers + driver. Live monitor figure refines in Phase 1.3.",
  settingsSystemDefault: "System default",
  settingsNotConnected: "(not connected)",
  settingsSaveChanges: "Save changes",
  settingsUnavailable: "Audio settings unavailable (running outside Tauri?).",
  settingsBackHome: "← Home",

  // ── Jingle feature ────────────────────────────────────────────────────────
  jingleTitle: "Jingle Studio",
  jingleSubtitle: "Generate a jingle in under 60 seconds",
  jingleFormTitle: "New jingle",
  jingleFormDesc:
    "Fill out the spec below. Once submitted, the AI generates stems and assembles them with voiceover.",

  jingleFieldTitle: "Title",
  jingleFieldTitlePlaceholder: "Sunday Morning Opener",
  jingleFieldDuration: "Duration",
  jingleFieldMood: "Mood",
  jingleFieldTempo: "Tempo (BPM)",
  jingleFieldInstruments: "Instruments",
  jingleFieldInstrumentsHint:
    "Comma-separated stems, e.g. piano, strings, drums",
  jingleFieldInstrumentsPlaceholder: "piano, strings, drums",
  jingleFieldVoiceover: "Voiceover text",
  jingleFieldVoiceoverPlaceholder: "Welcome to Sunday Morning…",
  jingleFieldVoiceoverHint: "Optional — leave blank for music only",

  jingleMoodEnergetic: "Energetic",
  jingleMoodCalm: "Calm",
  jingleMoodWorshipful: "Worshipful",
  jingleMoodProfessional: "Professional",

  jingleDuration20: "20 seconds",
  jingleDuration30: "30 seconds",
  jingleDuration60: "60 seconds",

  jingleValidateBpm: "BPM must be between 60 and 200",
  jingleValidateBpmInteger: "BPM must be a whole number",
  jingleValidateTitle: "Title is required",
  jingleValidateInstruments: "At least one instrument is required",
  jingleValidateTooManyInstruments: "Maximum 8 instrument stems allowed",
  jingleValidateDuration: "Duration must be 20, 30 or 60 seconds",
  jingleValidateMood:
    "Mood must be energetic, calm, worshipful or professional",

  jingleGenerateButton: "Generate jingle (Pro)",
  jinglePreviewPlan: "Preview render plan",
  jinglePlanTitle: "Render plan",
  jinglePlanDescription: "Description",
  jinglePlanOutput: "Output",
  jinglePlanStems: "Stems",
  jinglePlanFfmpegArgs: "ffmpeg args",
  jingleProNotice:
    "Jingle AI generation requires Sunday Cast Pro. The render plan is free to preview.",
  jingleGenerating: "Generating jingle…",
  jingleGenerateError: "Could not generate the jingle",
  jingleResultTitle: "Generated jingle",
  jingleResultModel: "Generated by",
  jingleResultDuration: "Duration",
  jingleResultAudio: "Audio",

  // ── Jingle page / gallery ─────────────────────────────────────────────────
  jinglePageGalleryTitle: "Your jingles",
  jinglePageGalleryEmpty:
    "No jingles yet. Fill out the spec above and generate your first one.",
  jinglePageCount: "{n} generated",
  jinglePagePlay: "Play",
  jinglePagePause: "Pause",
  jinglePageRegenerate: "Regenerate",
  jinglePageRegenerating: "Regenerating…",
  jinglePageRename: "Rename",
  jinglePageRenamePlaceholder: "Jingle name…",
  jinglePageDelete: "Delete",
  jinglePagePreviewUnavailable:
    "Preview plays once the generated audio downloads in the app.",
};

// ── Norwegian ──────────────────────────────────────────────────────────────

const no: Catalog = {
  // ── App shell ────────────────────────────────────────────────────────────
  appName: "SundayStudio",
  appTagline: "Podkast- og jingle-produksjon",

  // ── Navigation / routes ──────────────────────────────────────────────────
  navRecord: "Opptak",
  navEdit: "Rediger",
  navJingle: "Jingle",
  navSettings: "Innstillinger",
  navDiagnostics: "Diagnostikk",
  navBack: "Tilbake",
  navHome: "Hjem",

  // ── Common actions ───────────────────────────────────────────────────────
  actionCancel: "Avbryt",
  actionClose: "Lukk",
  actionSave: "Lagre",
  actionSaving: "Lagrer…",
  actionSaved: "Lagret",
  actionDelete: "Slett",
  actionEdit: "Rediger",
  actionAdd: "Legg til",
  actionDone: "Ferdig",
  actionNew: "Ny",
  actionBack: "Tilbake",
  actionOpen: "Åpne",
  actionRemove: "Fjern",
  actionExport: "Eksporter",
  actionExporting: "Eksporterer…",
  loadingShort: "Laster…",

  // ── Home / diagnostics page ───────────────────────────────────────────────
  homeTitle: "Diagnostikk",
  homeProjectsTitle: "Prosjekter",
  homeProjectsDesc: "Lag et nytt prosjekt eller gjenåpne et registrert.",
  homeProjectNamePlaceholder: "Prosjektnavn…",
  homeNewProject: "Ny",
  homeProjectListEmpty: "Ingen prosjekter ennå — lag ett over.",
  homeProjectListLoading: "Laster…",
  homeProjectListError:
    "Prosjektlisten laster når du kjører SundayStudio-appen.",
  homeProjectOpenLabel: "Åpne",
  homeProjectRemoveLabel: "Fjern {name} fra registeret",
  homeBackendTitle: "Rust-backend",
  homeBackendError: "IPC utilgjengelig (kjøres utenfor Tauri?)",
  homeAudioDevicesTitle: "Lydenheter",
  homeAudioDevicesScanning: "Skanner lydmaskinvare…",
  homeAudioDevicesError:
    "Kunne ikke liste opp enheter (kjøres utenfor Tauri eller ingen lyd-backend).",
  homeInputs: "Inndata",
  homeOutputs: "Utdata",
  homeNoneFound: "Ingen funnet",
  homeDefaultBadge: "standard",
  homeTestToneTitle: "Ta opp testtone",
  homeTestToneDesc:
    "Skriver en 1-sekunds 440 Hz sinusbølge til en WAV på disk — bevis på at opptak-til-fil-banen fungerer.",
  homeWriting: "Skriver…",
  homeRecordTestTone: "Ta opp testtone",
  homeWrote: "Skrev",
  homeAnalyzeLoudness: "Analyser loudness",
  homeMeasuring: "Måler…",
  homeLoudnessTargetsTitle: "Loudness-mål",
  homeLoudnessTargetsDesc:
    "Hvor hver plattform normaliserer showet ditt. Master til målet, så spilles det av på det nivået du ønsket.",
  homeLoudnessTargetsError: "Mål laster når du kjører SundayStudio-appen.",
  homeMasterPresetsTitle: "Masteringforskyv",
  homeMasterPresetsDesc:
    "Ét-klikks masterkjeder (EQ · 3-bands kompressor · murstein-limiter) koblet til et loudness-mål.",
  homeMasterPresetsError: "Forskyv laster når du kjører SundayStudio-appen.",
  homeExportPresetsTitle: "Eksportforskyv",
  homeExportPresetsDesc:
    "Plattformklare eksportinnstillinger. En ferdig episode renderes til en mastret, loudness-normalisert fil i prosjektets exports-mappe.",
  homeExportPresetsError: "Forskyv laster når du kjører SundayStudio-appen.",
  homeVoicePresetsTitle: "Stemme-forskyv",
  homeVoicePresetsDesc:
    "Innebygde prosesseringskjeder (port · EQ · de-esser · kompressor · saturator). Kobles til mikserspor i en senere fase.",
  homeVoicePresetsError: "Forskyv laster når du kjører SundayStudio-appen.",
  metricIntegrated: "Integrert",
  metricShortTerm: "Kortvarig",
  metricMomentary: "Momentan",
  metricRange: "Område",
  metricTruePeak: "Sann topp",
  metricSamplePeak: "Samplet topp",

  // ── Record page ───────────────────────────────────────────────────────────
  recordNoProject: "Ingen åpent prosjekt.",
  recordNoTracks: "Ingen spor ennå.",
  recordAddTrack: "Legg til spor",
  recordTransportNotice:
    "Transporten er en forhåndsvisning — levende opptak kobles til opptaksmotoren på ekte lydmaskinvare. Sporinnstillinger og kapitler under lagres i prosjektet.",
  recordExported: "Eksportert",
  recordChapters: "Kapitler",
  recordAddChapter: "Legg til kapittel",
  recordWriterFailed:
    "Opptak stoppet: feil ved skriving til disk — filen kan være ødelagt.",
  recordDroppedBadge: "{count} sampler tapt",
  recordBackupProject: "Sikkerhetskopier prosjekt",
  recordBackupDone: "Prosjektet er sikkerhetskopiert",
  recordBackupFailed: "Sikkerhetskopiering mislyktes",
  recordActionFailed: "Kunne ikke lagre endringen",

  // ── Settings page ─────────────────────────────────────────────────────────
  settingsTitle: "Lydenhet",
  settingsSubtitle: "Innstillinger · Lyd",
  settingsInputDevice: "Inndataenhet",
  settingsInputHint: "Mikrofon eller USB-grensesnitt",
  settingsOutputDevice: "Utdataenhet",
  settingsOutputHint: "Hodetelefoner eller høyttalere",
  settingsSampleRate: "Samplingsrate",
  settingsBufferSize: "Bufferstørrelse",
  settingsBufferHint: "Lavere = mindre forsinkelse",
  settingsLatencyTitle: "Estimert rundturforsinkelse",
  settingsLatencyDesc:
    "Inn- og utdatabuffere + driver. Levende monitorfigur finjusteres i fase 1.3.",
  settingsSystemDefault: "Systemstandard",
  settingsNotConnected: "(ikke tilkoblet)",
  settingsSaveChanges: "Lagre endringer",
  settingsUnavailable:
    "Lydinnstillinger utilgjengelig (kjøres utenfor Tauri?).",
  settingsBackHome: "← Hjem",

  // ── Jingle feature ────────────────────────────────────────────────────────
  jingleTitle: "Jingle-studio",
  jingleSubtitle: "Generer en jingle på under 60 sekunder",
  jingleFormTitle: "Ny jingle",
  jingleFormDesc:
    "Fyll ut spesifikasjonen under. Når du sender inn, genererer AI-en stems og setter dem sammen med voiceover.",

  jingleFieldTitle: "Tittel",
  jingleFieldTitlePlaceholder: "Søndagsmorgensåpner",
  jingleFieldDuration: "Varighet",
  jingleFieldMood: "Stemning",
  jingleFieldTempo: "Tempo (BPM)",
  jingleFieldInstruments: "Instrumenter",
  jingleFieldInstrumentsHint:
    "Kommaseparerte stems, f.eks. piano, strenger, trommer",
  jingleFieldInstrumentsPlaceholder: "piano, strenger, trommer",
  jingleFieldVoiceover: "Voiceover-tekst",
  jingleFieldVoiceoverPlaceholder: "Velkommen til søndagsmorgen…",
  jingleFieldVoiceoverHint: "Valgfritt — la stå tomt for kun musikk",

  jingleMoodEnergetic: "Energisk",
  jingleMoodCalm: "Rolig",
  jingleMoodWorshipful: "Tilbedende",
  jingleMoodProfessional: "Profesjonell",

  jingleDuration20: "20 sekunder",
  jingleDuration30: "30 sekunder",
  jingleDuration60: "60 sekunder",

  jingleValidateBpm: "BPM må være mellom 60 og 200",
  jingleValidateBpmInteger: "BPM må være et helt tall",
  jingleValidateTitle: "Tittel er påkrevd",
  jingleValidateInstruments: "Minst ett instrument er påkrevd",
  jingleValidateTooManyInstruments: "Maks 8 instrument-stems er tillatt",
  jingleValidateDuration: "Varighet må være 20, 30 eller 60 sekunder",
  jingleValidateMood:
    "Stemning må være energisk, rolig, tilbedende eller profesjonell",

  jingleGenerateButton: "Generer jingle (Pro)",
  jinglePreviewPlan: "Forhåndsvis renderplan",
  jinglePlanTitle: "Renderplan",
  jinglePlanDescription: "Beskrivelse",
  jinglePlanOutput: "Utgang",
  jinglePlanStems: "Stems",
  jinglePlanFfmpegArgs: "ffmpeg-argumenter",
  jingleProNotice:
    "AI-basert jingelgenerering krever Sunday Cast Pro. Renderplanen er gratis å forhåndsvise.",
  jingleGenerating: "Genererer jingle…",
  jingleGenerateError: "Kunne ikke generere jingelen",
  jingleResultTitle: "Generert jingle",
  jingleResultModel: "Generert av",
  jingleResultDuration: "Varighet",
  jingleResultAudio: "Lyd",

  // ── Jingle page / gallery ─────────────────────────────────────────────────
  jinglePageGalleryTitle: "Dine jingler",
  jinglePageGalleryEmpty:
    "Ingen jingler ennå. Fyll ut spesifikasjonen over og generer din første.",
  jinglePageCount: "{n} generert",
  jinglePagePlay: "Spill",
  jinglePagePause: "Pause",
  jinglePageRegenerate: "Generer på nytt",
  jinglePageRegenerating: "Genererer på nytt…",
  jinglePageRename: "Gi nytt navn",
  jinglePageRenamePlaceholder: "Jingle-navn…",
  jinglePageDelete: "Slett",
  jinglePagePreviewUnavailable:
    "Forhåndsvisning spilles av når den genererte lyden lastes ned i appen.",
};

// ── Swedish (partial — falls back to English) ─────────────────────────────

const sv: Catalog = {
  appName: "SundayStudio",
  appTagline: "Podcast- och jingleproduktion",
  navRecord: "Spela in",
  navEdit: "Redigera",
  navJingle: "Jingle",
  navSettings: "Inställningar",
  navDiagnostics: "Diagnostik",
  navBack: "Tillbaka",
  navHome: "Hem",
  actionCancel: "Avbryt",
  actionClose: "Stäng",
  actionSave: "Spara",
  actionSaving: "Sparar…",
  actionSaved: "Sparad",
  actionDelete: "Ta bort",
  actionEdit: "Redigera",
  actionAdd: "Lägg till",
  actionDone: "Klar",
  actionNew: "Ny",
  actionBack: "Tillbaka",
  actionOpen: "Öppna",
  actionRemove: "Ta bort",
  actionExport: "Exportera",
  actionExporting: "Exporterar…",
  loadingShort: "Laddar…",
  recordWriterFailed:
    "Inspelningen stoppades: diskskrivfel — filen kan vara skadad.",
  recordDroppedBadge: "{count} sampel tappade",
  recordBackupProject: "Säkerhetskopiera projekt",
  recordBackupDone: "Projektet säkerhetskopierat",
  recordBackupFailed: "Säkerhetskopieringen misslyckades",
  recordActionFailed: "Det gick inte att spara ändringen",
};

// ── Danish (partial) ──────────────────────────────────────────────────────

const da: Catalog = {
  appName: "SundayStudio",
  appTagline: "Podcast- og jingleproduktion",
  navRecord: "Optag",
  navEdit: "Rediger",
  navJingle: "Jingle",
  navSettings: "Indstillinger",
  navDiagnostics: "Diagnostik",
  navBack: "Tilbage",
  navHome: "Hjem",
  actionCancel: "Annuller",
  actionClose: "Luk",
  actionSave: "Gem",
  actionSaving: "Gemmer…",
  actionSaved: "Gemt",
  actionDelete: "Slet",
  actionEdit: "Rediger",
  actionAdd: "Tilføj",
  actionDone: "Færdig",
  actionNew: "Ny",
  actionBack: "Tilbage",
  actionOpen: "Åbn",
  actionRemove: "Fjern",
  actionExport: "Eksporter",
  actionExporting: "Eksporterer…",
  loadingShort: "Indlæser…",
  recordWriterFailed:
    "Optagelsen stoppet: diskskrivefejl — filen kan være beskadiget.",
  recordDroppedBadge: "{count} samples tabt",
  recordBackupProject: "Sikkerhedskopiér projekt",
  recordBackupDone: "Projektet er sikkerhedskopieret",
  recordBackupFailed: "Sikkerhedskopiering mislykkedes",
  recordActionFailed: "Kunne ikke gemme ændringen",
};

// ── German (partial) ──────────────────────────────────────────────────────

const de: Catalog = {
  appName: "SundayStudio",
  appTagline: "Podcast- und Jingle-Produktion",
  navRecord: "Aufnahme",
  navEdit: "Bearbeiten",
  navJingle: "Jingle",
  navSettings: "Einstellungen",
  navDiagnostics: "Diagnose",
  navBack: "Zurück",
  navHome: "Start",
  actionCancel: "Abbrechen",
  actionClose: "Schließen",
  actionSave: "Speichern",
  actionSaving: "Speichert…",
  actionSaved: "Gespeichert",
  actionDelete: "Löschen",
  actionEdit: "Bearbeiten",
  actionAdd: "Hinzufügen",
  actionDone: "Fertig",
  actionNew: "Neu",
  actionBack: "Zurück",
  actionOpen: "Öffnen",
  actionRemove: "Entfernen",
  actionExport: "Exportieren",
  actionExporting: "Exportiert…",
  loadingShort: "Lädt…",
  recordWriterFailed:
    "Aufnahme gestoppt: Schreibfehler auf der Festplatte — Datei möglicherweise beschädigt.",
  recordDroppedBadge: "{count} Samples verloren",
  recordBackupProject: "Projekt sichern",
  recordBackupDone: "Projekt gesichert",
  recordBackupFailed: "Sicherung fehlgeschlagen",
  recordActionFailed: "Änderung konnte nicht gespeichert werden",
};

// ── French (partial) ──────────────────────────────────────────────────────

const fr: Catalog = {
  appName: "SundayStudio",
  appTagline: "Production de podcasts et jingles",
  navRecord: "Enregistrer",
  navEdit: "Modifier",
  navJingle: "Jingle",
  navSettings: "Paramètres",
  navDiagnostics: "Diagnostics",
  navBack: "Retour",
  navHome: "Accueil",
  actionCancel: "Annuler",
  actionClose: "Fermer",
  actionSave: "Enregistrer",
  actionSaving: "Enregistrement…",
  actionSaved: "Enregistré",
  actionDelete: "Supprimer",
  actionEdit: "Modifier",
  actionAdd: "Ajouter",
  actionDone: "Terminé",
  actionNew: "Nouveau",
  actionBack: "Retour",
  actionOpen: "Ouvrir",
  actionRemove: "Supprimer",
  actionExport: "Exporter",
  actionExporting: "Exportation…",
  loadingShort: "Chargement…",
  recordWriterFailed:
    "Enregistrement arrêté : erreur d’écriture disque — le fichier peut être corrompu.",
  recordDroppedBadge: "{count} échantillons perdus",
  recordBackupProject: "Sauvegarder le projet",
  recordBackupDone: "Projet sauvegardé",
  recordBackupFailed: "Échec de la sauvegarde",
  recordActionFailed: "Impossible d’enregistrer la modification",
};

// ── Polish (partial) ──────────────────────────────────────────────────────

const pl: Catalog = {
  appName: "SundayStudio",
  appTagline: "Produkcja podcastów i dżingli",
  navRecord: "Nagrywanie",
  navEdit: "Edycja",
  navJingle: "Dżingiel",
  navSettings: "Ustawienia",
  navDiagnostics: "Diagnostyka",
  navBack: "Wstecz",
  navHome: "Strona główna",
  actionCancel: "Anuluj",
  actionClose: "Zamknij",
  actionSave: "Zapisz",
  actionSaving: "Zapisywanie…",
  actionSaved: "Zapisano",
  actionDelete: "Usuń",
  actionEdit: "Edytuj",
  actionAdd: "Dodaj",
  actionDone: "Gotowe",
  actionNew: "Nowy",
  actionBack: "Wstecz",
  actionOpen: "Otwórz",
  actionRemove: "Usuń",
  actionExport: "Eksportuj",
  actionExporting: "Eksportowanie…",
  loadingShort: "Ładowanie…",
  recordWriterFailed:
    "Nagrywanie zatrzymane: błąd zapisu na dysk — plik może być uszkodzony.",
  recordDroppedBadge: "Utracono {count} próbek",
  recordBackupProject: "Utwórz kopię zapasową projektu",
  recordBackupDone: "Utworzono kopię zapasową projektu",
  recordBackupFailed: "Tworzenie kopii zapasowej nie powiodło się",
  recordActionFailed: "Nie udało się zapisać zmiany",
};

// ── Catalog map ───────────────────────────────────────────────────────────────

const CATALOGS: Record<Lang, Catalog> = { no, en, sv, da, de, fr, pl };

// ── Store ─────────────────────────────────────────────────────────────────────

interface I18nState {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (key: string) => string;
}

const LOCALE_KEY = "sundaystudio_lang";

function readPersistedLang(): Lang {
  try {
    const v = localStorage.getItem(LOCALE_KEY);
    if (v && (LANGS as string[]).includes(v)) return v as Lang;
  } catch {
    // localStorage may be unavailable in tests
  }
  return "no";
}

function resolve(lang: Lang, key: string): string {
  return CATALOGS[lang][key] ?? CATALOGS["en"][key] ?? key;
}

export const useI18n = create<I18nState>((set, get) => ({
  lang: readPersistedLang(),
  setLang: (lang) => {
    try {
      localStorage.setItem(LOCALE_KEY, lang);
    } catch {
      // ignore
    }
    set({ lang });
  },
  t: (key) => resolve(get().lang, key),
}));

/**
 * Convenience: access the translate function without subscribing to re-renders.
 * Useful in plain TS utilities; React components should call `useI18n`.
 */
export function t(key: string): string {
  return resolve(useI18n.getState().lang, key);
}
