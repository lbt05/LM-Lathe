//! Audio manager: stores generated audio, inspects it, converts it and plays
//! it back locally.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::error::{TtsError, TtsResult};
use crate::paths::AppPaths;
use crate::types::{AudioFormat, AudioOutput};

/// Metadata of an audio file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioInfo {
    /// Container of the file.
    pub format: AudioFormat,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u16,
    /// Bits per sample.
    pub bits_per_sample: u16,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Size of the file in bytes.
    pub size_bytes: u64,
}

/// Manages the audio files produced by the TTS subsystem.
#[derive(Debug, Clone)]
pub struct AudioManager {
    generated_root: PathBuf,
}

impl AudioManager {
    /// Creates a manager rooted at the given paths.
    pub fn new(paths: &AppPaths) -> Self {
        Self {
            generated_root: paths.generated_audio_root(),
        }
    }

    /// Creates a manager with an explicit output directory.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            generated_root: root.into(),
        }
    }

    /// Directory generated audio is written to.
    pub fn generated_root(&self) -> &Path {
        &self.generated_root
    }

    /// Stores an audio payload on disk.
    ///
    /// The file name is derived from `stem` plus a timestamp so repeated
    /// synthesis never overwrites a previous result.
    pub fn save_generated(&self, output: &AudioOutput, stem: &str) -> TtsResult<PathBuf> {
        std::fs::create_dir_all(&self.generated_root)
            .map_err(|e| TtsError::io(&self.generated_root, e))?;
        let safe_stem = sanitize(stem);
        let name = format!(
            "{}-{}.{}",
            safe_stem,
            chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"),
            output.format.extension()
        );
        let path = self.generated_root.join(name);
        std::fs::write(&path, &output.data).map_err(|e| TtsError::io(&path, e))?;
        Ok(path)
    }

    /// Inspects an audio file.
    pub fn inspect_file(&self, path: &Path) -> TtsResult<AudioInfo> {
        let bytes = std::fs::read(path).map_err(|e| TtsError::io(path, e))?;
        let format = detect_format(path, &bytes);
        let mut info = inspect_bytes(&bytes, format)
            .ok_or_else(|| TtsError::InvalidRequest(format!("unsupported audio file: {path:?}")))?;
        info.size_bytes = bytes.len() as u64;
        Ok(info)
    }

    /// Lists the generated audio files, newest first.
    pub fn generated_files(&self) -> TtsResult<Vec<PathBuf>> {
        if !self.generated_root.exists() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.generated_root)
            .map_err(|e| TtsError::io(&self.generated_root, e))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        files.reverse();
        Ok(files)
    }

    /// Converts an audio file into another format.
    ///
    /// WAV conversions (sample rate, channel count, bit depth) are done
    /// natively. Compressed formats are delegated to `ffmpeg` when it is
    /// installed.
    pub fn convert(
        &self,
        source: &Path,
        target: AudioFormat,
        sample_rate: Option<u32>,
        channels: Option<u16>,
    ) -> TtsResult<PathBuf> {
        if !source.exists() {
            return Err(TtsError::InvalidRequest(format!(
                "audio file does not exist: {source:?}"
            )));
        }
        let out_dir = self.generated_root.clone();
        std::fs::create_dir_all(&out_dir).map_err(|e| TtsError::io(&out_dir, e))?;
        let stem = source
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "audio".to_string());
        let target_path = out_dir.join(format!("{}-converted.{}", stem, target.extension()));

        match target {
            AudioFormat::Wav => {
                let samples = read_wav_samples(source)?;
                let samples = match channels {
                    Some(target_channels) if target_channels != samples.channels => {
                        change_channel_count(&samples, target_channels)
                    }
                    _ => samples,
                };
                let samples = match sample_rate {
                    Some(rate) if rate != samples.sample_rate => resample(&samples, rate),
                    _ => samples,
                };
                write_wav(&target_path, &samples)?;
            }
            AudioFormat::Pcm => {
                let samples = read_wav_samples(source)?;
                let samples = match channels {
                    Some(target_channels) if target_channels != samples.channels => {
                        change_channel_count(&samples, target_channels)
                    }
                    _ => samples,
                };
                let samples = match sample_rate {
                    Some(rate) if rate != samples.sample_rate => resample(&samples, rate),
                    _ => samples,
                };
                let pcm = encode_pcm16(&samples);
                std::fs::write(&target_path, pcm).map_err(|e| TtsError::io(&target_path, e))?;
            }
            other => {
                if !ffmpeg_available() {
                    return Err(TtsError::UnsupportedFormat(other));
                }
                let mut cmd = std::process::Command::new("ffmpeg");
                cmd.arg("-y")
                    .arg("-i")
                    .arg(source)
                    .arg("-f")
                    .arg(ffmpeg_format(other))
                    .arg(&target_path);
                if let Some(rate) = sample_rate {
                    cmd.arg("-ar").arg(rate.to_string());
                }
                if let Some(ch) = channels {
                    cmd.arg("-ac").arg(ch.to_string());
                }
                let status = cmd
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map_err(|e| TtsError::Internal(format!("failed to run ffmpeg: {e}")))?;
                if !status.success() {
                    return Err(TtsError::UnsupportedFormat(other));
                }
            }
        }
        Ok(target_path)
    }

    /// Plays an audio file with the platform player.
    pub fn play(&self, path: &Path) -> TtsResult<()> {
        if !path.exists() {
            return Err(TtsError::InvalidRequest(format!(
                "audio file does not exist: {path:?}"
            )));
        }
        let player = platform_player().ok_or_else(|| {
            TtsError::Internal("no audio player available for this platform".to_string())
        })?;
        let mut cmd = std::process::Command::new(player);
        cmd.arg(path);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        cmd.spawn()
            .map_err(|e| TtsError::Internal(format!("failed to start {player}: {e}")))?;
        Ok(())
    }
}

/// Decoded PCM samples.
#[derive(Debug, Clone)]
pub struct PcmSamples {
    /// Interleaved samples in the range `[-1.0, 1.0]`.
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u16,
}

/// Reads a WAV file and decodes it into floating point samples.
pub fn read_wav_samples(path: &Path) -> TtsResult<PcmSamples> {
    let mut reader = hound::WavReader::open(path).map_err(|e| hound_error(path, e))?;
    let spec = reader.spec();
    let channels = spec.channels;
    let sample_rate = spec.sample_rate;
    let scale = (1i32 << (spec.bits_per_sample.saturating_sub(1).min(31))) as f32;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| hound_error(path, e))?,
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .map(|sample| sample.map(|value| value as f32 / scale))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| hound_error(path, e))?,
    };
    Ok(PcmSamples {
        samples,
        sample_rate,
        channels,
    })
}

/// Maps a `hound` error onto a TTS error.
fn hound_error(path: &Path, error: hound::Error) -> TtsError {
    TtsError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()),
    }
}

/// Writes floating point samples into a 16 bit PCM WAV file.
pub fn write_wav(path: &Path, pcm: &PcmSamples) -> TtsResult<()> {
    let spec = hound::WavSpec {
        channels: pcm.channels,
        sample_rate: pcm.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| hound_error(path, e))?;
    for sample in &pcm.samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(value)
            .map_err(|e| hound_error(path, e))?;
    }
    writer.finalize().map_err(|e| hound_error(path, e))?;
    Ok(())
}

/// Encodes floating point samples as little endian PCM16.
pub fn encode_pcm16(pcm: &PcmSamples) -> Vec<u8> {
    let mut out = Vec::with_capacity(pcm.samples.len() * 2);
    for sample in &pcm.samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Encodes samples as a WAV file in memory.
///
/// The engines that take a reference clip want a whole WAV file — the
/// header and the samples — and not the raw samples: they check the file
/// before they read it.
pub fn encode_wav(pcm: &PcmSamples) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: pcm.channels,
        sample_rate: pcm.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("a WAV fits in memory");
        for sample in &pcm.samples {
            let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(value).expect("a WAV fits in memory");
        }
        writer.finalize().expect("a WAV fits in memory");
    }
    cursor.into_inner()
}

/// Inspects a WAV file in memory.
pub fn inspect_wav_bytes(bytes: &[u8]) -> Option<AudioInfo> {
    let cursor = std::io::Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor).ok()?;
    let spec = reader.spec();
    let total = reader.len() as u64;
    let bytes_per_frame = spec.channels as u64 * (spec.bits_per_sample as u64 / 8);
    let frames = total.checked_div(bytes_per_frame).unwrap_or(0);
    let duration_ms = if spec.sample_rate == 0 {
        0
    } else {
        frames * 1000 / spec.sample_rate as u64
    };
    Some(AudioInfo {
        format: AudioFormat::Wav,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bits_per_sample: spec.bits_per_sample,
        duration_ms,
        size_bytes: bytes.len() as u64,
    })
}

/// Inspects an audio file.
pub fn inspect_file(path: &Path) -> TtsResult<AudioInfo> {
    let bytes = std::fs::read(path).map_err(|e| TtsError::io(path, e))?;
    let format = detect_format(path, &bytes);
    let mut info = inspect_bytes(&bytes, format)
        .ok_or_else(|| TtsError::InvalidRequest(format!("unsupported audio file: {path:?}")))?;
    info.size_bytes = bytes.len() as u64;
    Ok(info)
}

/// Inspects an audio file.
pub fn inspect_bytes(bytes: &[u8], format: AudioFormat) -> Option<AudioInfo> {
    match format {
        AudioFormat::Wav => inspect_wav_bytes(bytes),
        AudioFormat::Pcm => None,
        _ => None,
    }
}

/// Detects the container of a file from its extension and magic bytes.
pub fn detect_format(path: &Path, bytes: &[u8]) -> AudioFormat {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return AudioFormat::Wav;
    }
    if bytes.len() >= 4 && &bytes[0..4] == b"fLaC" {
        return AudioFormat::Flac;
    }
    if bytes.len() >= 3 && &bytes[0..3] == b"ID3" {
        return AudioFormat::Mp3;
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(AudioFormat::parse)
    {
        Some(format) => format,
        None => AudioFormat::Wav,
    }
}

/// Resamples PCM data with linear interpolation.
pub fn resample(pcm: &PcmSamples, target_rate: u32) -> PcmSamples {
    if target_rate == 0 || pcm.sample_rate == target_rate || pcm.samples.is_empty() {
        return pcm.clone();
    }
    let frames = pcm.samples.len() / pcm.channels.max(1) as usize;
    if frames == 0 {
        return pcm.clone();
    }
    let ratio = target_rate as f64 / pcm.sample_rate as f64;
    let target_frames = ((frames as f64) * ratio).round().max(1.0) as usize;
    let channels = pcm.channels.max(1) as usize;
    let mut out = Vec::with_capacity(target_frames * channels);
    for frame in 0..target_frames {
        let src_pos = frame as f64 / ratio;
        let src_index = src_pos.floor() as usize;
        let frac = (src_pos - src_index as f64) as f32;
        let next_index = (src_index + 1).min(frames - 1);
        for channel in 0..channels {
            let a = pcm.samples[src_index * channels + channel];
            let b = pcm.samples[next_index * channels + channel];
            out.push(a + (b - a) * frac);
        }
    }
    PcmSamples {
        samples: out,
        sample_rate: target_rate,
        channels: pcm.channels,
    }
}

/// Converts between mono and stereo.
pub fn change_channel_count(pcm: &PcmSamples, target_channels: u16) -> PcmSamples {
    let source_channels = pcm.channels.max(1) as usize;
    let target_channels = target_channels.max(1) as usize;
    if source_channels == target_channels {
        return pcm.clone();
    }
    let frames = pcm.samples.len() / source_channels;
    let mut out = Vec::with_capacity(frames * target_channels);
    for frame in 0..frames {
        let base = frame * source_channels;
        if target_channels == 1 {
            let mut sum = 0.0;
            for channel in 0..source_channels {
                sum += pcm.samples[base + channel];
            }
            out.push(sum / source_channels as f32);
        } else {
            for channel in 0..target_channels {
                out.push(pcm.samples[base + (channel % source_channels)]);
            }
        }
    }
    PcmSamples {
        samples: out,
        sample_rate: pcm.sample_rate,
        channels: target_channels as u16,
    }
}

/// Converts a WAV file into a base64 encoded mono 44.1 kHz PCM16 payload.
///
/// The native MLX runtime of Fish Audio Drama 3 only accepts reference clips
/// in that shape.
pub fn wav_to_mono_44k_base64(path: &Path) -> TtsResult<String> {
    let pcm = read_wav_samples(path)?;
    let mono = change_channel_count(&pcm, 1);
    let resampled = resample(&mono, 44_100);
    // The native runtime wants a WAV file, and not the raw samples: it
    // reads the header before it reads the audio.
    let wav = encode_wav(&resampled);
    Ok(base64_encode(&wav))
}

/// Minimal base64 encoder (standard alphabet, padded).
pub fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// `true` when `ffmpeg` is installed.
pub fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The platform specific audio player.
pub fn platform_player() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("afplay")
    } else if cfg!(target_os = "linux") {
        Some("ffplay")
    } else {
        None
    }
}

fn ffmpeg_format(format: AudioFormat) -> &'static str {
    match format {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Flac => "flac",
        AudioFormat::Ogg => "ogg",
        AudioFormat::Opus => "opus",
        AudioFormat::Aac => "adts",
        AudioFormat::Wav => "wav",
        AudioFormat::Pcm => "s16le",
    }
}

/// Removes characters that are unsafe in file names.
fn sanitize(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "audio".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn resample_changes_length() {
        let pcm = PcmSamples {
            samples: vec![0.0; 1000],
            sample_rate: 10_000,
            channels: 1,
        };
        let out = resample(&pcm, 20_000);
        assert_eq!(out.sample_rate, 20_000);
        assert_eq!(out.samples.len(), 2000);
    }

    /// The reference clips are sent to the engine as a whole WAV file.
    ///
    /// The native runtime reads the header of the file before it reads the
    /// audio, so raw samples are not enough.
    #[test]
    fn a_reference_clip_is_a_wav_file() {
        let pcm = PcmSamples {
            samples: (0..100).map(|i| (i as f32 / 100.0) - 0.5).collect(),
            sample_rate: 44_100,
            channels: 1,
        };
        let wav = encode_wav(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        // The header and the samples are there.
        assert_eq!(wav.len(), 44 + 200);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let pcm = PcmSamples {
            samples: vec![1.0, 0.0, 1.0, 0.0],
            sample_rate: 16_000,
            channels: 2,
        };
        let mono = change_channel_count(&pcm, 1);
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.samples, vec![0.5, 0.5]);
    }
}
