#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayHdrCapabilities {
    pub supports_edr: bool,
    pub maximum_potential_edr_headroom: f64,
}

impl Default for DisplayHdrCapabilities {
    fn default() -> Self {
        Self {
            supports_edr: false,
            maximum_potential_edr_headroom: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppleDecodeBackend {
    VideoToolbox,
    Software,
}

#[cfg(target_os = "macos")]
pub mod coreaudio {
    use std::sync::{Arc, Mutex};

    use coreaudio::audio_unit::audio_format::LinearPcmFlags;
    use coreaudio::audio_unit::render_callback::{self, data};
    use coreaudio::audio_unit::{AudioUnit, Element, IOType, SampleFormat, Scope, StreamFormat};
    use thiserror::Error;

    use crate::audio::{
        AudioClockSnapshot, AudioOutputBackend, AudioOutputState, AudioPushResult, AudioReadResult,
        AudioRingBuffer, AudioRingBufferConfig, AudioRingBufferStats,
    };
    use crate::ffmpeg::{PcmAudioFrame, PcmFormat, PcmSampleFormat};

    #[derive(Debug, Error)]
    pub enum CoreAudioOutputError {
        #[error("audio error: {0}")]
        Audio(#[from] crate::audio::AudioError),
        #[error("CoreAudio error: {0:?}")]
        CoreAudio(#[from] coreaudio::Error),
        #[error("CoreAudio output buffer is not configured")]
        NotConfigured,
        #[error("CoreAudio output lock was poisoned")]
        LockPoisoned,
    }

    pub type Result<T> = std::result::Result<T, CoreAudioOutputError>;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CoreAudioOutputConfig {
        pub ring_buffer: AudioRingBufferConfig,
    }

    impl Default for CoreAudioOutputConfig {
        fn default() -> Self {
            Self {
                ring_buffer: AudioRingBufferConfig {
                    capacity_frames: 96_000,
                    drop_oldest_on_overflow: true,
                },
            }
        }
    }

    pub struct CoreAudioOutput {
        state: AudioOutputState,
        audio_unit: Option<AudioUnit>,
        buffer: Arc<Mutex<AudioRingBuffer>>,
    }

    impl CoreAudioOutput {
        pub fn new(config: CoreAudioOutputConfig) -> Self {
            Self {
                state: AudioOutputState::Stopped,
                audio_unit: None,
                buffer: Arc::new(Mutex::new(AudioRingBuffer::new(config.ring_buffer))),
            }
        }

        pub fn configure(&mut self, format: PcmFormat) -> Result<()> {
            configure_buffer(&self.buffer, format)?;
            let mut audio_unit = AudioUnit::new_uninitialized(IOType::DefaultOutput)?;
            audio_unit.set_stream_format(
                coreaudio_stream_format(format),
                Scope::Input,
                Element::Output,
            )?;
            install_render_callback(&mut audio_unit, Arc::clone(&self.buffer))?;
            audio_unit.initialize()?;
            self.audio_unit = Some(audio_unit);
            Ok(())
        }

        pub fn start(&mut self) -> Result<()> {
            let Some(audio_unit) = &mut self.audio_unit else {
                return Err(CoreAudioOutputError::NotConfigured);
            };
            audio_unit.start()?;
            self.state = AudioOutputState::Playing;
            Ok(())
        }

        pub fn pause(&mut self) -> Result<()> {
            if let Some(audio_unit) = &mut self.audio_unit {
                audio_unit.stop()?;
            }
            self.state = AudioOutputState::Paused;
            Ok(())
        }

        pub fn stop(&mut self) -> Result<()> {
            if let Some(audio_unit) = &mut self.audio_unit {
                audio_unit.stop()?;
            }
            clear_buffer(&self.buffer)?;
            self.state = AudioOutputState::Stopped;
            Ok(())
        }

        pub fn push(&mut self, frame: PcmAudioFrame) -> Result<AudioPushResult> {
            let mut buffer = self
                .buffer
                .lock()
                .map_err(|_| CoreAudioOutputError::LockPoisoned)?;
            Ok(buffer.push_frame(frame)?)
        }

        pub fn read_for_test(&mut self, output: &mut [f32]) -> Result<AudioReadResult> {
            let mut buffer = self
                .buffer
                .lock()
                .map_err(|_| CoreAudioOutputError::LockPoisoned)?;
            Ok(buffer.read_interleaved(output)?)
        }

        pub fn state(&self) -> AudioOutputState {
            self.state
        }

        pub fn stats(&self) -> Result<AudioRingBufferStats> {
            let buffer = self
                .buffer
                .lock()
                .map_err(|_| CoreAudioOutputError::LockPoisoned)?;
            Ok(buffer.stats())
        }

        pub fn clock_snapshot(&self) -> Result<AudioClockSnapshot> {
            let buffer = self
                .buffer
                .lock()
                .map_err(|_| CoreAudioOutputError::LockPoisoned)?;
            Ok(buffer.clock_snapshot())
        }
    }

    impl Default for CoreAudioOutput {
        fn default() -> Self {
            Self::new(CoreAudioOutputConfig::default())
        }
    }

    impl AudioOutputBackend for CoreAudioOutput {
        fn configure(&mut self, format: PcmFormat) -> crate::audio::Result<()> {
            CoreAudioOutput::configure(self, format)
                .map_err(|error| crate::audio::AudioError::Backend(error.to_string()))
        }

        fn start(&mut self) -> crate::audio::Result<()> {
            CoreAudioOutput::start(self)
                .map_err(|error| crate::audio::AudioError::Backend(error.to_string()))
        }

        fn pause(&mut self) -> crate::audio::Result<()> {
            CoreAudioOutput::pause(self)
                .map_err(|error| crate::audio::AudioError::Backend(error.to_string()))
        }

        fn stop(&mut self) -> crate::audio::Result<()> {
            CoreAudioOutput::stop(self)
                .map_err(|error| crate::audio::AudioError::Backend(error.to_string()))
        }

        fn push(&mut self, frame: PcmAudioFrame) -> crate::audio::Result<AudioPushResult> {
            CoreAudioOutput::push(self, frame)
                .map_err(|error| crate::audio::AudioError::Backend(error.to_string()))
        }

        fn state(&self) -> AudioOutputState {
            self.state
        }

        fn stats(&self) -> AudioRingBufferStats {
            self.stats().unwrap_or_default()
        }

        fn clock_snapshot(&self) -> Option<AudioClockSnapshot> {
            self.clock_snapshot().ok()
        }
    }

    fn coreaudio_stream_format(format: PcmFormat) -> StreamFormat {
        match format.sample_format {
            PcmSampleFormat::F32Interleaved => StreamFormat {
                sample_rate: format.sample_rate as f64,
                sample_format: SampleFormat::F32,
                flags: LinearPcmFlags::IS_FLOAT | LinearPcmFlags::IS_PACKED,
                channels: format.channels,
            },
        }
    }

    fn install_render_callback(
        audio_unit: &mut AudioUnit,
        buffer: Arc<Mutex<AudioRingBuffer>>,
    ) -> Result<()> {
        type Args = render_callback::Args<data::Interleaved<f32>>;
        audio_unit.set_render_callback(move |mut args: Args| {
            let read_result = buffer
                .lock()
                .map_err(|_| ())
                .and_then(|mut buffer| buffer.read_interleaved(args.data.buffer).map_err(|_| ()))?;
            if read_result.underflow_frames > 0 {
                args.flags
                    .insert(render_callback::ActionFlags::OUTPUT_IS_SILENCE);
            }
            Ok(())
        })?;
        Ok(())
    }

    fn configure_buffer(buffer: &Arc<Mutex<AudioRingBuffer>>, format: PcmFormat) -> Result<()> {
        let mut buffer = buffer
            .lock()
            .map_err(|_| CoreAudioOutputError::LockPoisoned)?;
        Ok(buffer.configure(format)?)
    }

    fn clear_buffer(buffer: &Arc<Mutex<AudioRingBuffer>>) -> Result<()> {
        let mut buffer = buffer
            .lock()
            .map_err(|_| CoreAudioOutputError::LockPoisoned)?;
        buffer.clear();
        Ok(())
    }
}
