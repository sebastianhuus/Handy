//! Chunks a long 16kHz mono WAV file into overlapping windows and
//! transcribes each one with Parakeet, emitting a JSON file in the schema
//! `eda/chunk_merge.py` expects (see ../SPEC.md).
//!
//! This mirrors exactly how `managers/transcription.rs` invokes Parakeet in
//! the main Handy app (same `ParakeetModel::load` / `ParakeetParams` /
//! `transcribe_with` calls, same `TimestampGranularity::Segment`) so results
//! here are representative of what the shipped app would actually produce --
//! this harness just adds the chunking loop around it.
//!
//! Usage:
//!   chunk_harness --model-dir <path to a downloaded parakeet model dir> \
//!       --input meeting.wav --window-s 30 --overlap-s 6 --out run.json
//!
//! `--input` must be 16kHz mono PCM WAV (the sample rate Handy itself
//! records and feeds to Parakeet at). Convert with, e.g.:
//!   ffmpeg -i input.m4a -ac 1 -ar 16000 meeting.wav

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Serialize;
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams, TimestampGranularity};
use transcribe_rs::onnx::Quantization;
use transcribe_rs::SpeechModel;

#[derive(Parser)]
#[command(about = "Chunk a long WAV file and transcribe each chunk with Parakeet")]
struct Args {
    /// Directory containing the downloaded Parakeet ONNX model files.
    #[arg(long)]
    model_dir: PathBuf,

    /// 16kHz mono PCM WAV file to transcribe.
    #[arg(long)]
    input: PathBuf,

    /// Window length in seconds -- how much audio each chunk hands to the
    /// model in one call.
    #[arg(long, default_value_t = 30.0)]
    window_s: f32,

    /// Overlap in seconds between consecutive windows -- must be smaller
    /// than --window-s. hop = window_s - overlap_s.
    #[arg(long, default_value_t = 6.0)]
    overlap_s: f32,

    /// Where to write the run JSON (chunk_merge.py's expected schema).
    #[arg(long)]
    out: PathBuf,
}

#[derive(Serialize)]
struct SegmentOut {
    start: f32,
    end: f32,
    text: String,
}

#[derive(Serialize)]
struct ChunkOut {
    index: usize,
    start: f32,
    end: f32,
    /// Wall-clock seconds the Parakeet call took for this chunk -- a cheap
    /// throughput signal for whether larger windows are worth their latency.
    transcribe_seconds: f64,
    segments: Vec<SegmentOut>,
}

#[derive(Serialize)]
struct RunOut {
    window_s: f32,
    overlap_s: f32,
    input: String,
    chunks: Vec<ChunkOut>,
}

const SAMPLE_RATE: u32 = 16_000;

fn main() -> Result<()> {
    let args = Args::parse();

    if args.overlap_s >= args.window_s {
        bail!(
            "--overlap-s ({}) must be smaller than --window-s ({})",
            args.overlap_s,
            args.window_s
        );
    }

    let samples = read_wav_16k_mono(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let total_samples = samples.len();
    let total_s = total_samples as f32 / SAMPLE_RATE as f32;
    eprintln!(
        "Loaded {} samples ({:.1}s) from {}",
        total_samples,
        total_s,
        args.input.display()
    );

    eprintln!("Loading Parakeet model from {}...", args.model_dir.display());
    let mut engine = ParakeetModel::load(&args.model_dir, &Quantization::Int8)
        .context("loading Parakeet model")?;

    let window_samples = (args.window_s * SAMPLE_RATE as f32) as usize;
    let overlap_samples = (args.overlap_s * SAMPLE_RATE as f32) as usize;
    let hop_samples = window_samples - overlap_samples;

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    loop {
        let end = (start + window_samples).min(total_samples);
        let chunk_audio = &samples[start..end];

        let params = ParakeetParams {
            timestamp_granularity: Some(TimestampGranularity::Segment),
            ..Default::default()
        };

        let chunk_start_s = start as f32 / SAMPLE_RATE as f32;
        let chunk_end_s = end as f32 / SAMPLE_RATE as f32;
        eprintln!(
            "Chunk {index}: [{chunk_start_s:.1}s, {chunk_end_s:.1}s] ({} samples)...",
            chunk_audio.len()
        );

        let call_start = Instant::now();
        let result = engine
            .transcribe_with(chunk_audio, &params)
            .with_context(|| format!("transcribing chunk {index}"))?;
        let transcribe_seconds = call_start.elapsed().as_secs_f64();

        // Offset segment timestamps by the chunk's start so every timestamp
        // in the output JSON is relative to the original recording, not the
        // chunk -- chunk_merge.py relies on this to line up overlaps.
        let segments = result
            .segments
            .unwrap_or_default()
            .into_iter()
            .map(|s| SegmentOut {
                start: s.start + chunk_start_s,
                end: s.end + chunk_start_s,
                text: s.text,
            })
            .collect();

        chunks.push(ChunkOut {
            index,
            start: chunk_start_s,
            end: chunk_end_s,
            transcribe_seconds,
            segments,
        });

        if end >= total_samples {
            break;
        }
        start += hop_samples;
        index += 1;
    }

    let run = RunOut {
        window_s: args.window_s,
        overlap_s: args.overlap_s,
        input: args.input.display().to_string(),
        chunks,
    };
    let json = serde_json::to_string_pretty(&run)?;
    std::fs::write(&args.out, json).with_context(|| format!("writing {}", args.out.display()))?;
    eprintln!("Wrote {}", args.out.display());

    Ok(())
}

/// Reads a WAV file and returns f32 samples in [-1, 1]. Errors (rather than
/// silently resampling/downmixing) if the file isn't 16kHz mono -- a silent
/// implicit conversion here would quietly skew every timestamp downstream.
fn read_wav_16k_mono(path: &PathBuf) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE {
        bail!(
            "expected {SAMPLE_RATE}Hz, got {}Hz -- resample first, e.g.: \
             ffmpeg -i input.wav -ac 1 -ar {SAMPLE_RATE} out.wav",
            spec.sample_rate
        );
    }
    if spec.channels != 1 {
        bail!(
            "expected mono, got {} channels -- downmix first, e.g.: \
             ffmpeg -i input.wav -ac 1 -ar {SAMPLE_RATE} out.wav",
            spec.channels
        );
    }

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / (1i32 << (spec.bits_per_sample - 1)) as f32))
            .collect::<std::result::Result<_, _>>()?,
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()?,
    };
    Ok(samples)
}
