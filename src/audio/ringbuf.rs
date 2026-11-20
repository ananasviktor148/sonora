use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Safe, lock-free Single-Producer Single-Consumer (SPSC) audio sample ring buffer.
/// Stores `f32` samples as `AtomicU32` bits with Acquire/Release synchronization,
/// eliminating all mutex locking and allocations in the real-time audio callback.
pub struct AudioRingBuffer {
    buf: Box<[AtomicU32]>,
    mask: usize,
    write_pos: AtomicUsize,
    read_pos: AtomicUsize,
}

impl AudioRingBuffer {
    /// Create a ring buffer with capacity rounded up to the next power of two.
    pub fn new(min_capacity_samples: usize) -> Self {
        let cap = min_capacity_samples.max(1024).next_power_of_two();
        let mut v = Vec::with_capacity(cap);
        for _ in 0..cap {
            v.push(AtomicU32::new(0));
        }
        Self {
            buf: v.into_boxed_slice(),
            mask: cap - 1,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn available_read(&self) -> usize {
        let w = self.write_pos.load(Ordering::Acquire);
        let r = self.read_pos.load(Ordering::Relaxed);
        w.wrapping_sub(r)
    }

    pub fn available_write(&self) -> usize {
        self.capacity().saturating_sub(self.available_read())
    }

    /// Push interleaved samples from the producer thread. Returns number of samples written.
    pub fn push(&self, samples: &[f32]) -> usize {
        let w = self.write_pos.load(Ordering::Relaxed);
        let r = self.read_pos.load(Ordering::Acquire);
        let avail = self.capacity().saturating_sub(w.wrapping_sub(r));
        let count = samples.len().min(avail);

        for (i, &s) in samples[..count].iter().enumerate() {
            let idx = (w.wrapping_add(i)) & self.mask;
            self.buf[idx].store(s.to_bits(), Ordering::Relaxed);
        }

        self.write_pos
            .store(w.wrapping_add(count), Ordering::Release);
        count
    }

    /// Pop interleaved samples from the consumer (audio callback) thread.
    /// Returns the number of samples actually read.
    pub fn pop(&self, out: &mut [f32]) -> usize {
        let r = self.read_pos.load(Ordering::Relaxed);
        let w = self.write_pos.load(Ordering::Acquire);
        let avail = w.wrapping_sub(r);
        let count = out.len().min(avail);

        for (i, dst) in out[..count].iter_mut().enumerate() {
            let idx = (r.wrapping_add(i)) & self.mask;
            *dst = f32::from_bits(self.buf[idx].load(Ordering::Relaxed));
        }

        self.read_pos
            .store(r.wrapping_add(count), Ordering::Release);
        count
    }

    /// Discard all buffered samples immediately (safe when producer is paused/flushing).
    pub fn clear(&self) {
        let w = self.write_pos.load(Ordering::Acquire);
        self.read_pos.store(w, Ordering::Release);
    }
}

pub const MAX_METER_CHANNELS: usize = 8;
pub const VISUALIZER_FFT_SIZE: usize = 2048;

/// Lock-free tap feeding recent audio samples and per-channel peak/RMS meters to the UI.
pub struct VisualizerTap {
    mono_ring: Box<[AtomicU32]>,
    mono_pos: AtomicUsize,
    channel_peaks: [AtomicU32; MAX_METER_CHANNELS],
    channel_rms: [AtomicU32; MAX_METER_CHANNELS],
    active_channels: AtomicUsize,
}

impl VisualizerTap {
    pub fn new() -> Self {
        let mut ring = Vec::with_capacity(VISUALIZER_FFT_SIZE);
        for _ in 0..VISUALIZER_FFT_SIZE {
            ring.push(AtomicU32::new(0));
        }
        Self {
            mono_ring: ring.into_boxed_slice(),
            mono_pos: AtomicUsize::new(0),
            channel_peaks: std::array::from_fn(|_| AtomicU32::new(0)),
            channel_rms: std::array::from_fn(|_| AtomicU32::new(0)),
            active_channels: AtomicUsize::new(2),
        }
    }

    /// Push a chunk of planar multi-channel audio samples (called on DSP thread).
    pub fn push_planar(&self, channels: &[Vec<f32>]) {
        let ch_count = channels.len();
        if ch_count == 0 {
            return;
        }
        let frames = channels[0].len();
        if frames == 0 {
            return;
        }

        self.active_channels
            .store(ch_count.min(MAX_METER_CHANNELS), Ordering::Relaxed);

        // Update per-channel peak and RMS
        for ch in 0..MAX_METER_CHANNELS {
            if let Some(samples) = channels.get(ch) {
                let mut peak = 0.0f32;
                let mut sum_sq = 0.0f64;
                for &s in samples {
                    let a = s.abs();
                    if a > peak {
                        peak = a;
                    }
                    sum_sq += (s as f64) * (s as f64);
                }
                let rms = (sum_sq / frames as f64).sqrt() as f32;
                self.channel_peaks[ch].store(peak.to_bits(), Ordering::Relaxed);
                self.channel_rms[ch].store(rms.to_bits(), Ordering::Relaxed);
            } else {
                self.channel_peaks[ch].store(0, Ordering::Relaxed);
                self.channel_rms[ch].store(0, Ordering::Relaxed);
            }
        }

        // Push mono mix into FFT ring
        let inv_ch = 1.0 / (ch_count as f32);
        let mut pos = self.mono_pos.load(Ordering::Relaxed);
        let mask = VISUALIZER_FFT_SIZE - 1;
        for f in 0..frames {
            let mut sum = 0.0f32;
            for ch in channels {
                if f < ch.len() {
                    sum += ch[f];
                }
            }
            self.mono_ring[pos & mask].store((sum * inv_ch).to_bits(), Ordering::Relaxed);
            pos = pos.wrapping_add(1);
        }
        self.mono_pos.store(pos, Ordering::Release);
    }

    /// Reset meters to zero (on pause/stop).
    pub fn reset_meters(&self) {
        for ch in 0..MAX_METER_CHANNELS {
            self.channel_peaks[ch].store(0, Ordering::Relaxed);
            self.channel_rms[ch].store(0, Ordering::Relaxed);
        }
    }

    /// Snapshot the most recent `VISUALIZER_FFT_SIZE` mono samples in chronological order.
    pub fn read_mono_window(&self, out: &mut [f32; VISUALIZER_FFT_SIZE]) {
        let pos = self.mono_pos.load(Ordering::Acquire);
        let mask = VISUALIZER_FFT_SIZE - 1;
        let start = pos.wrapping_sub(VISUALIZER_FFT_SIZE);
        for (i, dst) in out.iter_mut().enumerate() {
            let idx = start.wrapping_add(i) & mask;
            *dst = f32::from_bits(self.mono_ring[idx].load(Ordering::Relaxed));
        }
    }

    /// Read current per-channel (peak, rms) linear values and active channel count.
    pub fn read_meters(&self) -> (usize, [(f32, f32); MAX_METER_CHANNELS]) {
        let active = self.active_channels.load(Ordering::Relaxed);
        let meters = std::array::from_fn(|ch| {
            let peak = f32::from_bits(self.channel_peaks[ch].load(Ordering::Relaxed));
            let rms = f32::from_bits(self.channel_rms[ch].load(Ordering::Relaxed));
            (peak, rms)
        });
        (active, meters)
    }
}

impl Default for VisualizerTap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_fifo_and_wraparound() {
        let rb = AudioRingBuffer::new(1024);
        let input: Vec<f32> = (0..700).map(|i| i as f32 * 0.001).collect();
        assert_eq!(rb.push(&input), 700);
        assert_eq!(rb.available_read(), 700);

        let mut out = vec![0.0f32; 500];
        assert_eq!(rb.pop(&mut out), 500);
        assert_eq!(out[0], 0.0);
        assert!((out[499] - 0.499).abs() < 1e-6);

        // Push again to wrap around 1024 boundary
        assert_eq!(rb.push(&input), 700);
        let mut out2 = vec![0.0f32; 900];
        assert_eq!(rb.pop(&mut out2), 900);
        assert!((out2[0] - 0.500).abs() < 1e-6);
        assert!((out2[200] - 0.0).abs() < 1e-6);
        assert!((out2[899] - 0.699).abs() < 1e-6);
    }
}
