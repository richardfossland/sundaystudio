//! The UI → audio-thread command channel.
//!
//! Commands the UI sends into the real-time callback (start/stop/arm/mute) must
//! cross the thread boundary without locks. This is a thin, typed wrapper over
//! an `rtrb` SPSC ring: the UI side `send`s, the audio side `try_recv`s once per
//! callback. `try_recv` never blocks and never allocates.
//!
//! In Phase 1.2 the live recorder's callback drains this each block to apply
//! mute/monitor changes; session teardown uses a separate shutdown flag (see
//! `session.rs`). The primitive is kept standalone and tested here.

use rtrb::{Consumer, Producer, RingBuffer};

/// A control message for the audio thread. `Copy` so it never carries heap
/// state across the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderCommand {
    /// Stop capture and tear down the take.
    Stop,
    /// Toggle software monitoring of the input.
    SetMonitoring(bool),
    /// Mute/unmute a track index in the monitor mix (capture continues).
    SetMute { track: usize, muted: bool },
}

/// UI side of the command channel.
pub struct CommandTx(Producer<RecorderCommand>);

/// Audio side of the command channel.
pub struct CommandRx(Consumer<RecorderCommand>);

/// Create a command channel with room for `capacity` pending commands.
pub fn command_channel(capacity: usize) -> (CommandTx, CommandRx) {
    let (p, c) = RingBuffer::new(capacity.max(1));
    (CommandTx(p), CommandRx(c))
}

impl CommandTx {
    /// Enqueue a command. Returns false if the queue is full (the UI can retry;
    /// in practice the audio thread drains far faster than the UI produces).
    pub fn send(&mut self, cmd: RecorderCommand) -> bool {
        self.0.push(cmd).is_ok()
    }
}

impl CommandRx {
    /// Pop the next command if any. Non-blocking; safe on the audio thread.
    pub fn try_recv(&mut self) -> Option<RecorderCommand> {
        self.0.pop().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_arrive_in_order() {
        let (mut tx, mut rx) = command_channel(8);
        assert!(tx.send(RecorderCommand::SetMonitoring(true)));
        assert!(tx.send(RecorderCommand::SetMute {
            track: 2,
            muted: true
        }));
        assert!(tx.send(RecorderCommand::Stop));

        assert_eq!(rx.try_recv(), Some(RecorderCommand::SetMonitoring(true)));
        assert_eq!(
            rx.try_recv(),
            Some(RecorderCommand::SetMute {
                track: 2,
                muted: true
            })
        );
        assert_eq!(rx.try_recv(), Some(RecorderCommand::Stop));
        assert_eq!(rx.try_recv(), None);
    }

    #[test]
    fn full_queue_reports_failure_without_blocking() {
        let (mut tx, mut rx) = command_channel(2);
        assert!(tx.send(RecorderCommand::Stop));
        assert!(tx.send(RecorderCommand::Stop));
        assert!(!tx.send(RecorderCommand::Stop)); // full
        assert_eq!(rx.try_recv(), Some(RecorderCommand::Stop));
        assert!(tx.send(RecorderCommand::Stop)); // room again
    }
}
