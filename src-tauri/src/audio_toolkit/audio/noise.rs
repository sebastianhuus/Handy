use std::time::Duration;

use nnnoiseless::DenoiseState;

use super::biquad::Biquad;
use super::FrameResampler;

const OUTPUT_RATE: usize = 48_000;
const FRAME_SIZE: usize = DenoiseState::FRAME_SIZE; // 480 samples = 10 ms at 48 kHz

/// Applies RNNoise-based noise suppression to a mono audio stream.
///
/// Input can be at any sample rate; the suppressor resamples to 48 kHz
/// internally before denoising, then emits denoised 48 kHz samples.
/// Feed the output into a FrameResampler configured for 48 kHz input.
///
/// Before each frame reaches RNNoise, a pre-EQ chain removes low-frequency
/// content that impairs speech recognition:
///   80 Hz HPF → plosive shelf (105 Hz) → room cut (165 Hz) → mud cut (285 Hz) → box cut (720 Hz)
/// Coefficients ported from BroadcastVoiceDSP (Swift reference implementation).
pub struct NoiseSuppressor {
    resampler: FrameResampler,
    state: Box<DenoiseState<'static>>,
    output: [f32; FRAME_SIZE],
    first_frame: bool,
    hpf: Biquad,
    plosive_shelf: Biquad,
    eq: [Biquad; 3],
    frame_buf: [f32; FRAME_SIZE],
}

impl NoiseSuppressor {
    pub fn new(input_rate: usize) -> Self {
        let sr = OUTPUT_RATE as f32;
        Self {
            resampler: FrameResampler::new(input_rate, OUTPUT_RATE, Duration::from_millis(10)),
            state: DenoiseState::new(),
            output: [0.0; FRAME_SIZE],
            first_frame: true,
            hpf: Biquad::new_highpass(80.0, sr, 0.707),
            plosive_shelf: Biquad::new_low_shelf(105.0, sr, -4.0, 0.75),
            eq: [
                Biquad::new_peaking(165.0, sr, 0.82, -4.0), // room cut
                Biquad::new_peaking(285.0, sr, 1.00, -3.0), // mud cut
                Biquad::new_peaking(720.0, sr, 1.05, -3.0), // box cut
            ],
            frame_buf: [0.0; FRAME_SIZE],
        }
    }

    pub fn push(&mut self, samples: &[f32], emit: &mut impl FnMut(&[f32])) {
        let state = &mut self.state;
        let output = &mut self.output;
        let first_frame = &mut self.first_frame;
        let hpf = &mut self.hpf;
        let plosive_shelf = &mut self.plosive_shelf;
        let eq = &mut self.eq;
        let frame_buf = &mut self.frame_buf;
        self.resampler.push(samples, |frame: &[f32]| {
            let len = frame.len().min(FRAME_SIZE);
            frame_buf[..len].copy_from_slice(&frame[..len]);
            hpf.process_buffer(&mut frame_buf[..len]);
            plosive_shelf.process_buffer(&mut frame_buf[..len]);
            for bq in eq.iter_mut() {
                bq.process_buffer(&mut frame_buf[..len]);
            }
            denoise_frame(frame_buf, state, output, first_frame, emit);
        });
    }

    pub fn finish(&mut self, emit: &mut impl FnMut(&[f32])) {
        let state = &mut self.state;
        let output = &mut self.output;
        let first_frame = &mut self.first_frame;
        let hpf = &mut self.hpf;
        let plosive_shelf = &mut self.plosive_shelf;
        let eq = &mut self.eq;
        let frame_buf = &mut self.frame_buf;
        self.resampler.finish(|frame: &[f32]| {
            let len = frame.len().min(FRAME_SIZE);
            frame_buf[..len].copy_from_slice(&frame[..len]);
            hpf.process_buffer(&mut frame_buf[..len]);
            plosive_shelf.process_buffer(&mut frame_buf[..len]);
            for bq in eq.iter_mut() {
                bq.process_buffer(&mut frame_buf[..len]);
            }
            denoise_frame(frame_buf, state, output, first_frame, emit);
        });
    }

    pub const fn output_rate() -> usize {
        OUTPUT_RATE
    }
}

/// Scales, denoises one 480-sample frame, then scales back.
/// Discards the very first output frame (documented fade-in artifact).
fn denoise_frame(
    frame: &[f32],
    state: &mut Box<DenoiseState<'static>>,
    output: &mut [f32; FRAME_SIZE],
    first_frame: &mut bool,
    emit: &mut impl FnMut(&[f32]),
) {
    // nnnoiseless expects i16-range floats, not [-1, 1].
    let mut scaled = [0.0f32; FRAME_SIZE];
    for (dst, &src) in scaled.iter_mut().zip(frame.iter()) {
        *dst = src * 32768.0;
    }

    state.process_frame(output, &scaled);

    if *first_frame {
        *first_frame = false;
        return;
    }

    let mut denoised = [0.0f32; FRAME_SIZE];
    for (dst, &src) in denoised.iter_mut().zip(output.iter()) {
        *dst = src / 32768.0;
    }
    emit(&denoised);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A few decaying harmonics on top of the fundamental so the test signal
    /// has some formant-like structure instead of a bare sine, which RNNoise
    /// (trained on real speech) is more likely to pass through unmolested.
    fn voice_like_tone(fundamental_hz: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        let harmonics = [(1.0, 1.0), (2.0, 0.5), (3.0, 0.25), (4.0, 0.125)];
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate;
                harmonics
                    .iter()
                    .map(|(mult, amp)| {
                        amp * (2.0 * std::f32::consts::PI * fundamental_hz * mult * t).sin()
                    })
                    .sum::<f32>()
                    * 0.2 // keep the composite comfortably under full scale
            })
            .collect()
    }

    /// Deterministic xorshift PRNG so the broadband-noise test has no extra
    /// dependency and is reproducible across runs.
    fn noise(len: usize, seed: u64, amplitude: f32) -> Vec<f32> {
        let mut state = seed | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                amplitude * (state as i64 as f64 / (u64::MAX as f64 / 2.0)) as f32
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// Runs the full input through the suppressor in one or more chunks and
    /// returns the concatenated denoised output.
    fn run_suppressor(suppressor: &mut NoiseSuppressor, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::new();
        suppressor.push(input, &mut |frame: &[f32]| out.extend_from_slice(frame));
        suppressor.finish(&mut |frame: &[f32]| out.extend_from_slice(frame));
        out
    }

    #[test]
    fn denoising_reduces_energy_of_a_noisy_signal() {
        let sr = OUTPUT_RATE as f32;
        let duration = 1.0;

        let tone = voice_like_tone(180.0, sr, duration);
        let noisy: Vec<f32> = tone
            .iter()
            .zip(noise(tone.len(), 7, 0.25))
            .map(|(&t, n)| t + n)
            .collect();

        let mut suppressor = NoiseSuppressor::new(OUTPUT_RATE);
        let denoised = run_suppressor(&mut suppressor, &noisy);

        assert!(!denoised.is_empty(), "suppressor produced no output frames");
        for &s in &denoised {
            assert!(
                s.is_finite(),
                "denoised output contains a non-finite sample"
            );
        }

        // Skip the first couple of frames: one is dropped internally as a
        // documented fade-in artifact, and the RNNoise gain needs a short
        // window to ramp up on noise it has just started observing.
        let skip = FRAME_SIZE * 4;
        let settled_denoised = &denoised[skip.min(denoised.len())..];
        let settled_noisy = &noisy[skip.min(noisy.len())..];

        let denoised_rms = rms(settled_denoised);
        let noisy_rms = rms(settled_noisy);

        assert!(
            denoised_rms < noisy_rms * 0.85,
            "expected denoising to measurably reduce energy: noisy_rms={noisy_rms}, denoised_rms={denoised_rms}"
        );
    }

    #[test]
    fn clean_tone_is_not_materially_distorted() {
        let sr = OUTPUT_RATE as f32;
        let tone = voice_like_tone(180.0, sr, 1.0);

        let mut suppressor = NoiseSuppressor::new(OUTPUT_RATE);
        let denoised = run_suppressor(&mut suppressor, &tone);

        assert!(!denoised.is_empty(), "suppressor produced no output frames");
        for &s in &denoised {
            assert!(
                s.is_finite(),
                "denoised output contains a non-finite sample"
            );
            assert!(s.abs() <= 2.0, "denoised sample {s} is out of a sane range");
        }

        let skip = FRAME_SIZE * 4;
        let settled_denoised = &denoised[skip.min(denoised.len())..];
        let settled_tone = &tone[skip.min(tone.len())..];

        let denoised_rms = rms(settled_denoised);
        let tone_rms = rms(settled_tone);

        // A clean, already-voice-like signal should survive roughly intact:
        // neither silenced nor amplified into distortion.
        assert!(
            denoised_rms > tone_rms * 0.3,
            "clean tone was suppressed too aggressively: tone_rms={tone_rms}, denoised_rms={denoised_rms}"
        );
        assert!(
            denoised_rms < tone_rms * 2.0,
            "clean tone was amplified unexpectedly: tone_rms={tone_rms}, denoised_rms={denoised_rms}"
        );
    }
}
