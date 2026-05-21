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
///   80 Hz HPF → room cut (165 Hz) → mud cut (285 Hz) → box cut (720 Hz)
/// Coefficients ported from BroadcastVoiceDSP (Swift reference implementation).
pub struct NoiseSuppressor {
    resampler: FrameResampler,
    state: Box<DenoiseState<'static>>,
    output: [f32; FRAME_SIZE],
    first_frame: bool,
    hpf: Biquad,
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
        let eq = &mut self.eq;
        let frame_buf = &mut self.frame_buf;
        self.resampler.push(samples, |frame: &[f32]| {
            let len = frame.len().min(FRAME_SIZE);
            frame_buf[..len].copy_from_slice(&frame[..len]);
            hpf.process_buffer(&mut frame_buf[..len]);
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
        let eq = &mut self.eq;
        let frame_buf = &mut self.frame_buf;
        self.resampler.finish(|frame: &[f32]| {
            let len = frame.len().min(FRAME_SIZE);
            frame_buf[..len].copy_from_slice(&frame[..len]);
            hpf.process_buffer(&mut frame_buf[..len]);
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
