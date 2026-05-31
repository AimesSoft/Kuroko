use std::collections::VecDeque;
use std::time::Duration;

use thiserror::Error;

use crate::ffmpeg::{PcmAudioFrame, PcmFormat};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AudioError {
    #[error("audio format changed from {expected:?} to {actual:?}")]
    FormatChanged {
        expected: PcmFormat,
        actual: PcmFormat,
    },
    #[error("audio format is not configured")]
    FormatNotConfigured,
    #[error("audio channel count must be greater than zero")]
    InvalidChannelCount,
    #[error("audio backend error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, AudioError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOutputState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioRingBufferConfig {
    pub capacity_frames: usize,
    pub drop_oldest_on_overflow: bool,
}

impl Default for AudioRingBufferConfig {
    fn default() -> Self {
        Self {
            capacity_frames: 48_000,
            drop_oldest_on_overflow: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioRingBufferStats {
    pub queued_frames: usize,
    pub queued_samples: usize,
    pub written_frames: u64,
    pub read_frames: u64,
    pub dropped_frames: u64,
    pub underflow_frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioPushResult {
    pub accepted_frames: usize,
    pub dropped_frames: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioReadResult {
    pub frames: usize,
    pub samples: usize,
    pub underflow_frames: usize,
}

#[derive(Debug)]
pub struct AudioRingBuffer {
    config: AudioRingBufferConfig,
    format: Option<PcmFormat>,
    samples: VecDeque<f32>,
    stats: AudioRingBufferStats,
}

impl AudioRingBuffer {
    pub fn new(config: AudioRingBufferConfig) -> Self {
        Self {
            config,
            format: None,
            samples: VecDeque::new(),
            stats: AudioRingBufferStats::default(),
        }
    }

    pub fn with_format(config: AudioRingBufferConfig, format: PcmFormat) -> Result<Self> {
        let mut buffer = Self::new(config);
        buffer.configure(format)?;
        Ok(buffer)
    }

    pub fn configure(&mut self, format: PcmFormat) -> Result<()> {
        if format.channels == 0 {
            return Err(AudioError::InvalidChannelCount);
        }
        self.format = Some(format);
        self.clear();
        Ok(())
    }

    pub fn format(&self) -> Option<PcmFormat> {
        self.format
    }

    pub fn capacity_frames(&self) -> usize {
        self.config.capacity_frames
    }

    pub fn queued_frames(&self) -> usize {
        let Some(format) = self.format else {
            return 0;
        };
        self.samples.len() / format.channels as usize
    }

    pub fn queued_duration(&self) -> Option<Duration> {
        let format = self.format?;
        if format.sample_rate == 0 {
            return None;
        }
        Some(Duration::from_secs_f64(
            self.queued_frames() as f64 / format.sample_rate as f64,
        ))
    }

    pub fn stats(&self) -> AudioRingBufferStats {
        AudioRingBufferStats {
            queued_frames: self.queued_frames(),
            queued_samples: self.samples.len(),
            ..self.stats
        }
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.stats.queued_frames = 0;
        self.stats.queued_samples = 0;
    }

    pub fn push_frame(&mut self, frame: PcmAudioFrame) -> Result<AudioPushResult> {
        match self.format {
            Some(format) if format != frame.format => {
                return Err(AudioError::FormatChanged {
                    expected: format,
                    actual: frame.format,
                });
            }
            Some(_) => {}
            None => self.configure(frame.format)?,
        }

        let format = self.format.expect("audio format exists");
        let channels = format.channels as usize;
        let incoming_frames = frame.samples.len() / channels;
        let mut dropped_frames = 0usize;

        if self.config.drop_oldest_on_overflow {
            while self.queued_frames() + incoming_frames > self.config.capacity_frames {
                if !self.drop_oldest_frame(channels) {
                    break;
                }
                dropped_frames += 1;
            }
        }

        let skip_incoming_frames = if self.config.drop_oldest_on_overflow {
            incoming_frames.saturating_sub(self.config.capacity_frames)
        } else {
            0
        };
        let skipped_incoming_samples = skip_incoming_frames * channels;
        let free_frames = self
            .config
            .capacity_frames
            .saturating_sub(self.queued_frames());
        let accepted_frames = incoming_frames
            .saturating_sub(skip_incoming_frames)
            .min(free_frames);
        let accepted_samples = accepted_frames * channels;
        self.samples.extend(
            frame
                .samples
                .into_iter()
                .skip(skipped_incoming_samples)
                .take(accepted_samples),
        );
        self.stats.written_frames += accepted_frames as u64;
        self.stats.dropped_frames +=
            (dropped_frames + incoming_frames.saturating_sub(accepted_frames)) as u64;

        Ok(AudioPushResult {
            accepted_frames,
            dropped_frames: dropped_frames + incoming_frames.saturating_sub(accepted_frames),
        })
    }

    pub fn read_interleaved(&mut self, output: &mut [f32]) -> Result<AudioReadResult> {
        let format = self.format.ok_or(AudioError::FormatNotConfigured)?;
        let channels = format.channels as usize;
        if channels == 0 {
            return Err(AudioError::InvalidChannelCount);
        }

        let requested_frames = output.len() / channels;
        let requested_samples = requested_frames * channels;
        let mut read_samples = 0usize;
        for sample in output.iter_mut().take(requested_samples) {
            if let Some(value) = self.samples.pop_front() {
                *sample = value;
                read_samples += 1;
            } else {
                *sample = 0.0;
            }
        }
        for sample in output.iter_mut().skip(requested_samples) {
            *sample = 0.0;
        }

        let read_frames = read_samples / channels;
        let underflow_frames = requested_frames.saturating_sub(read_frames);
        self.stats.read_frames += read_frames as u64;
        self.stats.underflow_frames += underflow_frames as u64;

        Ok(AudioReadResult {
            frames: read_frames,
            samples: read_samples,
            underflow_frames,
        })
    }

    fn drop_oldest_frame(&mut self, channels: usize) -> bool {
        if self.samples.len() < channels {
            self.samples.clear();
            return false;
        }
        for _ in 0..channels {
            let _ = self.samples.pop_front();
        }
        true
    }
}

pub trait AudioOutputBackend {
    fn configure(&mut self, format: PcmFormat) -> Result<()>;
    fn start(&mut self) -> Result<()>;
    fn pause(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn push(&mut self, frame: PcmAudioFrame) -> Result<AudioPushResult>;
    fn state(&self) -> AudioOutputState;
    fn stats(&self) -> AudioRingBufferStats;
}

#[derive(Debug)]
pub struct BufferedAudioOutput {
    state: AudioOutputState,
    buffer: AudioRingBuffer,
}

impl BufferedAudioOutput {
    pub fn new(config: AudioRingBufferConfig) -> Self {
        Self {
            state: AudioOutputState::Stopped,
            buffer: AudioRingBuffer::new(config),
        }
    }

    pub fn buffer(&self) -> &AudioRingBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut AudioRingBuffer {
        &mut self.buffer
    }

    pub fn read_interleaved(&mut self, output: &mut [f32]) -> Result<AudioReadResult> {
        self.buffer.read_interleaved(output)
    }
}

impl AudioOutputBackend for BufferedAudioOutput {
    fn configure(&mut self, format: PcmFormat) -> Result<()> {
        self.buffer.configure(format)
    }

    fn start(&mut self) -> Result<()> {
        self.state = AudioOutputState::Playing;
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        self.state = AudioOutputState::Paused;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.state = AudioOutputState::Stopped;
        self.buffer.clear();
        Ok(())
    }

    fn push(&mut self, frame: PcmAudioFrame) -> Result<AudioPushResult> {
        self.buffer.push_frame(frame)
    }

    fn state(&self) -> AudioOutputState {
        self.state
    }

    fn stats(&self) -> AudioRingBufferStats {
        self.buffer.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::PcmSampleFormat;

    fn stereo_format() -> PcmFormat {
        PcmFormat {
            sample_rate: 48_000,
            channels: 2,
            sample_format: PcmSampleFormat::F32Interleaved,
        }
    }

    fn frame(samples: Vec<f32>) -> PcmAudioFrame {
        PcmAudioFrame {
            format: stereo_format(),
            pts: None,
            frames: samples.len() / 2,
            samples,
        }
    }

    #[test]
    fn ring_buffer_reads_interleaved_samples_and_zero_fills_underflow() {
        let mut buffer = AudioRingBuffer::with_format(
            AudioRingBufferConfig {
                capacity_frames: 8,
                drop_oldest_on_overflow: true,
            },
            stereo_format(),
        )
        .unwrap();
        buffer.push_frame(frame(vec![0.1, 0.2, 0.3, 0.4])).unwrap();

        let mut output = [1.0; 6];
        let result = buffer.read_interleaved(&mut output).unwrap();

        assert_eq!(result.frames, 2);
        assert_eq!(result.underflow_frames, 1);
        assert_eq!(output, [0.1, 0.2, 0.3, 0.4, 0.0, 0.0]);
    }

    #[test]
    fn ring_buffer_drops_oldest_frames_on_overflow() {
        let mut buffer = AudioRingBuffer::with_format(
            AudioRingBufferConfig {
                capacity_frames: 2,
                drop_oldest_on_overflow: true,
            },
            stereo_format(),
        )
        .unwrap();

        let result = buffer
            .push_frame(frame(vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]))
            .unwrap();
        let mut output = [0.0; 4];
        buffer.read_interleaved(&mut output).unwrap();

        assert_eq!(result.accepted_frames, 2);
        assert_eq!(result.dropped_frames, 1);
        assert_eq!(output, [0.3, 0.4, 0.5, 0.6]);
    }

    #[test]
    fn buffered_audio_output_tracks_state() {
        let mut output = BufferedAudioOutput::new(AudioRingBufferConfig::default());

        output.configure(stereo_format()).unwrap();
        output.start().unwrap();
        assert_eq!(output.state(), AudioOutputState::Playing);
        output.pause().unwrap();
        assert_eq!(output.state(), AudioOutputState::Paused);
        output.stop().unwrap();
        assert_eq!(output.state(), AudioOutputState::Stopped);
    }
}
