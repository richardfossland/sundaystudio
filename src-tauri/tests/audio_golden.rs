//! Audio golden harness.
//!
//! DSP and synthesis output must stay byte-for-byte stable across refactors —
//! a "transparent" EQ that silently shifts by 0.1 dB, or a tone generator that
//! drifts, is a regression we want CI to catch. This harness fingerprints
//! generated audio and compares it to a committed golden.
//!
//! Phase 0.1/0.2 seeds it with the deterministic test tone. Phase 4 (effects)
//! adds one golden per effect: render a reference input through the effect and
//! pin the output. For float effect output, switch `fingerprint` for a
//! tolerance-based comparator (the structure here stays the same).
//!
//! ## Blessing goldens
//! Set `SUNDAY_BLESS_GOLDENS=1` to (re)write the golden files instead of
//! asserting — do this intentionally when an output change is expected, and
//! review the diff before committing.
//!
//! ```sh
//! SUNDAY_BLESS_GOLDENS=1 cargo test --test audio_golden
//! ```

use std::fs;
use std::path::PathBuf;

use sundaystudio_lib::audio::tone;

/// FNV-1a 64-bit — a tiny, dependency-free, deterministic fingerprint. Good
/// enough to detect any change in exact PCM bytes; not a security hash.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

/// Assert that `actual` matches the committed golden named `name`, or write the
/// golden when blessing. Returns nothing; panics with a helpful message on
/// mismatch so the test name + diff point straight at the regression.
fn assert_golden_fingerprint(name: &str, actual: u64) {
    let path = goldens_dir().join(format!("{name}.fnv"));
    let blessing = std::env::var_os("SUNDAY_BLESS_GOLDENS").is_some();

    if blessing || !path.exists() {
        fs::create_dir_all(goldens_dir()).expect("create goldens dir");
        fs::write(&path, format!("{actual:016x}\n")).expect("write golden");
        if blessing {
            eprintln!("blessed golden {name} = {actual:016x}");
        } else {
            // First-ever run for this golden: record it and pass, so a fresh
            // checkout that is missing the file self-heals rather than failing
            // spuriously. Commit the generated file to make it a real guard.
            eprintln!("seeded missing golden {name} = {actual:016x}");
        }
        return;
    }

    let expected = fs::read_to_string(&path)
        .expect("read golden")
        .trim()
        .to_string();
    let actual_hex = format!("{actual:016x}");
    assert_eq!(
        actual_hex, expected,
        "golden mismatch for '{name}': output changed (expected {expected}, got {actual_hex}). \
         If this change is intentional, re-bless with SUNDAY_BLESS_GOLDENS=1 and review the diff."
    );
}

#[test]
fn tone_440hz_48k_1s_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");

    tone::write_test_tone(&path, 48_000, 440.0, 1000).expect("tone writes");
    let bytes = fs::read(&path).expect("read tone wav");

    // Sanity: 48000 mono i16 samples + 44-byte canonical header.
    assert_eq!(bytes.len(), 48_000 * 2 + 44);

    assert_golden_fingerprint("tone_440hz_48k_1s", fnv1a_64(&bytes));
}
