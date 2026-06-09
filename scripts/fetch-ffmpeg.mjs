#!/usr/bin/env node
// Copy a static ffmpeg into src-tauri/binaries/ with the Rust target-triple
// suffix Tauri's `externalBin` expects (e.g. ffmpeg-aarch64-apple-darwin). Run
// before `tauri build`, both locally (via beforeBuildCommand) and in CI (each
// platform's runner copies its own binary).
//
// SundayStudio only needs ffmpeg (master WAV → MP3/AAC/FLAC re-encode); it does
// not probe media, so ffprobe is intentionally not bundled. The binary comes
// from the `ffmpeg-static` npm package — a GPL/LGPL ffmpeg build. See
// docs/DISTRIBUTION.md for the licensing note before any public release.

import { execSync } from "node:child_process";
import { mkdirSync, copyFileSync, chmodSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const ffmpegSrc = require("ffmpeg-static");

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "binaries");

// Rust host triple — what `externalBin` matches against.
const host = execSync("rustc -vV", { encoding: "utf8" })
  .split("\n")
  .find((l) => l.startsWith("host:"))
  .slice("host:".length)
  .trim();
const ext = host.includes("windows") ? ".exe" : "";

mkdirSync(outDir, { recursive: true });

if (!ffmpegSrc || !existsSync(ffmpegSrc)) {
  console.error(
    `✗ ffmpeg: source binary missing (${ffmpegSrc}). Run \`npm install\` first.`,
  );
  process.exit(1);
}
const dest = join(outDir, `ffmpeg-${host}${ext}`);
copyFileSync(ffmpegSrc, dest);
chmodSync(dest, 0o755);
console.log(`✓ ffmpeg → src-tauri/binaries/ffmpeg-${host}${ext}`);
