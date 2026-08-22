use std::f32::consts::TAU;

/// Transposed direct-form II biquad filter.
///
/// Ported from BroadcastVoiceDSP/Sources/BroadcastVoiceDSP/DSP/Biquad.swift.
/// Zero allocations in the hot path; denormal-safe.
#[derive(Clone, Copy)]
pub struct Biquad {
    z1: f32,
    z2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    fn bypass() -> Self {
        Self {
            z1: 0.0,
            z2: 0.0,
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        if !a0.is_finite() || a0.abs() < 1e-12 {
            return Self::bypass();
        }
        let inv = 1.0 / a0;
        let c = Self {
            z1: 0.0,
            z2: 0.0,
            b0: b0 * inv,
            b1: b1 * inv,
            b2: b2 * inv,
            a1: a1 * inv,
            a2: a2 * inv,
        };
        if [c.b0, c.b1, c.b2, c.a1, c.a2].iter().all(|v| v.is_finite()) {
            c
        } else {
            Self::bypass()
        }
    }

    /// Second-order high-pass (Butterworth at Q=0.707).
    pub fn new_highpass(freq_hz: f32, sample_rate: f32, q: f32) -> Self {
        let f = freq_hz.clamp(10.0, sample_rate * 0.45);
        let q = q.max(0.05);
        let omega = TAU * f / sample_rate;
        let sn = omega.sin();
        let cs = omega.cos();
        let alpha = sn / (2.0 * q);
        Self::from_coeffs(
            (1.0 + cs) / 2.0,
            -(1.0 + cs),
            (1.0 + cs) / 2.0,
            1.0 + alpha,
            -2.0 * cs,
            1.0 - alpha,
        )
    }

    /// Low-shelf EQ. `slope` controls the transition steepness (1.0 = standard, 0.75 = gentle).
    /// Returns bypass when |gain_db| < 0.001.
    pub fn new_low_shelf(freq_hz: f32, sample_rate: f32, gain_db: f32, slope: f32) -> Self {
        if gain_db.abs() < 0.001 {
            return Self::bypass();
        }
        let f = freq_hz.clamp(10.0, sample_rate * 0.45);
        let s = slope.max(0.05);
        let omega = TAU * f / sample_rate;
        let sn = omega.sin();
        let cs = omega.cos();
        let a = 10.0_f32.powf(gain_db / 40.0);
        let sqrt_a = a.sqrt();
        let radicand = ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).max(1e-6);
        let alpha = sn / 2.0 * radicand.sqrt();
        Self::from_coeffs(
            a * ((a + 1.0) - (a - 1.0) * cs + 2.0 * sqrt_a * alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - 2.0 * sqrt_a * alpha),
            (a + 1.0) + (a - 1.0) * cs + 2.0 * sqrt_a * alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - 2.0 * sqrt_a * alpha,
        )
    }

    /// Peaking (bell) EQ. Returns bypass when |gain_db| < 0.001.
    pub fn new_peaking(freq_hz: f32, sample_rate: f32, q: f32, gain_db: f32) -> Self {
        if gain_db.abs() < 0.001 {
            return Self::bypass();
        }
        let f = freq_hz.clamp(10.0, sample_rate * 0.45);
        let q = q.max(0.05);
        let omega = TAU * f / sample_rate;
        let sn = omega.sin();
        let cs = omega.cos();
        let alpha = sn / (2.0 * q);
        let a = 10.0_f32.powf(gain_db / 40.0);
        Self::from_coeffs(
            1.0 + alpha * a,
            -2.0 * cs,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cs,
            1.0 - alpha / a,
        )
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let x = if x.is_finite() { x } else { 0.0 };
        let y = self.b0 * x + self.z1;
        let nz1 = self.b1 * x - self.a1 * y + self.z2;
        let nz2 = self.b2 * x - self.a2 * y;
        if y.is_finite() && nz1.is_finite() && nz2.is_finite() {
            self.z1 = if nz1.abs() < 1e-20 { 0.0 } else { nz1 };
            self.z2 = if nz2.abs() < 1e-20 { 0.0 } else { nz2 };
            y
        } else {
            self.z1 = 0.0;
            self.z2 = 0.0;
            0.0
        }
    }

    #[inline(always)]
    pub fn process_buffer(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            *s = self.process(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_wave(freq_hz: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq_hz * i as f32 / sample_rate).sin())
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// Deterministic xorshift PRNG so the "blows up" test has no extra
    /// dependency and is reproducible across runs.
    fn noise(len: usize, seed: u64) -> Vec<f32> {
        let mut state = seed | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state as i64 as f64 / (u64::MAX as f64 / 2.0)) as f32
            })
            .collect()
    }

    #[test]
    fn highpass_attenuates_low_frequencies_more_than_high() {
        let sr = 48_000.0;
        let mut hpf_low = Biquad::new_highpass(80.0, sr, 0.707);
        let mut hpf_high = Biquad::new_highpass(80.0, sr, 0.707);

        // 40 Hz sits a full octave below the 80 Hz corner; 2 kHz is deep in
        // the passband. Half a second gives the filter time to settle.
        let low = sine_wave(40.0, sr, 0.5);
        let high = sine_wave(2000.0, sr, 0.5);

        let mut low_out = low.clone();
        hpf_low.process_buffer(&mut low_out);
        let mut high_out = high.clone();
        hpf_high.process_buffer(&mut high_out);

        // Measure only the back half, past the filter's settling transient.
        let half = low.len() / 2;
        let low_ratio = rms(&low_out[half..]) / rms(&low[half..]);
        let high_ratio = rms(&high_out[half..]) / rms(&high[half..]);

        assert!(
            low_ratio < 0.5,
            "expected substantial attenuation below cutoff, got ratio {low_ratio}"
        );
        assert!(
            high_ratio > 0.9,
            "expected near-unity gain well above cutoff, got ratio {high_ratio}"
        );
        assert!(
            high_ratio > low_ratio * 2.0,
            "high-frequency gain ({high_ratio}) should clear low-frequency gain ({low_ratio}) by a wide margin"
        );
    }

    #[test]
    fn filter_chain_stays_finite_and_bounded_for_typical_inputs() {
        let sr = 48_000.0;
        let make_chain = || {
            [
                Biquad::new_highpass(80.0, sr, 0.707),
                Biquad::new_low_shelf(105.0, sr, -4.0, 0.75),
                Biquad::new_peaking(165.0, sr, 0.82, -4.0),
                Biquad::new_peaking(285.0, sr, 1.00, -3.0),
                Biquad::new_peaking(720.0, sr, 1.05, -3.0),
            ]
        };

        let silence = vec![0.0f32; 4800];
        // Full-scale square wave (rich in harmonics) via a clipped sine.
        let full_scale: Vec<f32> = sine_wave(440.0, sr, 0.1)
            .into_iter()
            .map(|s| s.signum())
            .collect();
        let mixed: Vec<f32> = sine_wave(300.0, sr, 0.5)
            .into_iter()
            .zip(noise(24_000, 42))
            .map(|(tone, n)| tone * 0.7 + n * 0.3)
            .collect();

        for mut signal in [silence, full_scale, mixed] {
            let mut chain = make_chain();
            for stage in chain.iter_mut() {
                stage.process_buffer(&mut signal);
            }
            for &s in &signal {
                assert!(s.is_finite(), "filter chain produced a non-finite sample");
                assert!(
                    s.abs() <= 4.0,
                    "filter chain output blew up to {s}, expected a bounded signal"
                );
            }
        }
    }
}
