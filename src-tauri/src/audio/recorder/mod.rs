//! The multi-track recording engine (Phase 1.2).
//!
//! Architecture (see docs/ARCHITECTURE.md for the diagrams):
//! ```text
//!   cpal callback ──push──▶ per-channel rtrb rings ──drain──▶ writer thread ──▶ WAVs
//!        │                                                         │
//!        └── atomic peak meters ◀── UI polls ──────────────────────┘
//! ```
//!
//! Split by testability:
//! - `writer` — multi-track WAV writing with crash-safe incremental flush (tested)
//! - `meters` — block peak + atomic peak-hold meters (tested)
//! - `command` — lock-free UI→audio command queue (tested)
//! - `session` — rings + writer thread + controller; the FULL pipeline, driven in tests by synthetic frames, no device (tested)
//! - `stream` — the cpal input stream; the only hardware-dependent piece, wired to `session` but NOT verified without a real device
//!
//! The live start/stop Tauri commands (owning the `Stream` on a dedicated
//! thread, holding the `RecordController` in app state) land with the recording
//! UI in Phase 2.2, where they can be exercised against real interfaces.

pub mod command;
pub mod meters;
pub mod session;
pub mod stream;
pub mod writer;

pub use command::{command_channel, CommandRx, CommandTx, RecorderCommand};
pub use meters::PeakMeters;
pub use session::{start_session, CaptureSink, RecordConfig, RecordController};
pub use stream::{build_capture_stream, find_input_device};
pub use writer::{MultiTrackWriter, TrackSpec};
