//! Frame assembly with the Layer III bit reservoir.
//!
//! Each frame has a fixed-size slot for main data after its side
//! information. A frame's main data may start up to 511 bytes before its
//! own slot, in space earlier frames left unused (`main_data_begin`), so
//! main data runs as one contiguous stream across the slots. Frames are
//! held back until no later frame can still write into them.

use std::collections::VecDeque;

/// Largest `main_data_begin` the 9-bit field can express.
pub(crate) const MAX_MAIN_DATA_BEGIN: usize = (1 << 9) - 1;

struct PendingFrame {
    bytes: Vec<u8>,
    /// Offset of the main-data slot within `bytes`.
    slot_offset: usize,
    /// Position of the slot's first byte in the main-data stream.
    slot_start: usize,
}

impl PendingFrame {
    fn slot_end(&self) -> usize {
        self.slot_start + self.bytes.len() - self.slot_offset
    }
}

/// Places frames and their main data, and releases finished frames.
#[derive(Default)]
pub(crate) struct Reservoir {
    pending: VecDeque<PendingFrame>,
    /// Main-data stream position just past the last slot.
    end: usize,
    /// Main-data stream position just past the last byte written.
    written: usize,
}

impl Reservoir {
    /// Bytes of earlier slots the next frame's main data may start in.
    pub(crate) fn main_data_begin(&self) -> usize {
        (self.end - self.written).min(MAX_MAIN_DATA_BEGIN)
    }

    /// Adds a frame. `head` is the header, optional CRC and side information,
    /// whose `main_data_begin` must equal [`Self::main_data_begin`];
    /// `slot_len` is the frame's main-data space and `main_data` the bytes
    /// to place, which must fit in `main_data_begin + slot_len`.
    pub(crate) fn push(&mut self, head: &[u8], slot_len: usize, main_data: &[u8]) {
        let begin = self.main_data_begin();
        assert!(
            main_data.len() <= begin + slot_len,
            "main data overflows the reservoir"
        );

        let mut bytes = Vec::with_capacity(head.len() + slot_len);
        bytes.extend_from_slice(head);
        bytes.resize(head.len() + slot_len, 0);
        self.pending.push_back(PendingFrame {
            bytes,
            slot_offset: head.len(),
            slot_start: self.end,
        });
        self.end += slot_len;

        let start = self.end - slot_len - begin;
        let stop = start + main_data.len();
        for frame in &mut self.pending {
            let (lo, hi) = (start.max(frame.slot_start), stop.min(frame.slot_end()));
            if lo < hi {
                let dst = frame.slot_offset + lo - frame.slot_start;
                frame.bytes[dst..dst + hi - lo].copy_from_slice(&main_data[lo - start..hi - start]);
            }
        }
        self.written = stop;
    }

    /// Moves frames no later main data can reach into `out`.
    pub(crate) fn drain_finished(&mut self, out: &mut Vec<u8>) {
        let reachable_from = self.end - self.main_data_begin();
        while self
            .pending
            .front()
            .is_some_and(|f| f.slot_end() <= reachable_from)
        {
            out.extend_from_slice(&self.pending.pop_front().unwrap().bytes);
        }
    }

    /// Moves every remaining frame into `out`.
    pub(crate) fn drain_all(&mut self, out: &mut Vec<u8>) {
        for frame in self.pending.drain(..) {
            out.extend_from_slice(&frame.bytes);
        }
        self.written = self.end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_data_fills_its_own_slot_then_lends_the_rest() {
        let mut r = Reservoir::default();
        assert_eq!(r.main_data_begin(), 0);

        r.push(&[0xAA], 4, &[1, 2]);
        assert_eq!(r.main_data_begin(), 2);

        // The next frame starts in the 2 unused bytes, then its own slot.
        r.push(&[0xBB], 4, &[3, 4, 5]);
        assert_eq!(r.main_data_begin(), 3);

        let mut out = Vec::new();
        r.drain_all(&mut out);
        assert_eq!(out, [0xAA, 1, 2, 3, 4, 0xBB, 5, 0, 0, 0]);
    }

    #[test]
    fn short_main_data_leaves_the_whole_slot_free() {
        let mut r = Reservoir::default();
        r.push(&[0xAA], 4, &[]);
        r.push(&[0xBB], 4, &[7]);
        // Frame 2's data went into frame 1's slot; both slots' tails are free.
        assert_eq!(r.main_data_begin(), 7);
        let mut out = Vec::new();
        r.drain_all(&mut out);
        assert_eq!(out, [0xAA, 7, 0, 0, 0, 0xBB, 0, 0, 0, 0]);
    }

    #[test]
    fn frames_are_released_once_out_of_reach() {
        let mut r = Reservoir::default();
        let mut out = Vec::new();
        r.push(&[1], 300, &[9; 300]);
        r.drain_finished(&mut out);
        assert_eq!(out.len(), 301, "a full frame is final at once");

        r.push(&[2], 300, &[]);
        r.drain_finished(&mut out);
        assert_eq!(out.len(), 301, "an empty slot may still be borrowed");

        r.push(&[3], 300, &[]);
        r.push(&[4], 300, &[]);
        r.drain_finished(&mut out);
        // 511 bytes back from the end reaches into frame 3 only.
        assert_eq!(out.len(), 602);
        assert_eq!(r.main_data_begin(), MAX_MAIN_DATA_BEGIN);
    }

    #[test]
    #[should_panic(expected = "overflows")]
    fn overflowing_main_data_panics() {
        Reservoir::default().push(&[0], 2, &[1, 2, 3]);
    }
}
