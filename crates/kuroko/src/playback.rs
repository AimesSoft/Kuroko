use std::collections::VecDeque;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::core::{MediaRequest, TrackInfo, TrackKind, VideoParams};
use crate::ffmpeg::{
    self, AudioResampler, Decoder, DecoderBackend, DecoderConfig, DecoderOutputFrame, Demuxer,
    Frame, PcmAudioFrame, PcmFormat, StreamSelection,
};
use crate::source::{self, source_from_uri_with_hint};

#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("ffmpeg error: {0}")]
    Ffmpeg(#[from] ffmpeg::FfmpegError),
    #[error("source error: {0}")]
    Source(#[from] source::SourceError),
    #[error("no video track found")]
    NoVideoTrack,
    #[error("selected decoder output is not a video frame")]
    UnexpectedDecoderOutput,
}

pub type Result<T> = std::result::Result<T, PlaybackError>;

const VIDEO_FRAME_QUEUE_LIMIT: usize = 8;
const AUDIO_FRAME_QUEUE_LIMIT: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoDecodePreference {
    Software,
    VideoToolbox,
}

impl VideoDecodePreference {
    fn decoder_config(self) -> DecoderConfig {
        match self {
            Self::Software => DecoderConfig::software(),
            Self::VideoToolbox => DecoderConfig::videotoolbox(),
        }
    }
}

#[cfg(target_os = "macos")]
impl Default for VideoDecodePreference {
    fn default() -> Self {
        Self::VideoToolbox
    }
}

#[cfg(not(target_os = "macos"))]
impl Default for VideoDecodePreference {
    fn default() -> Self {
        Self::Software
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackSessionConfig {
    pub video_decode: VideoDecodePreference,
    pub audio_output: PcmFormat,
}

impl Default for PlaybackSessionConfig {
    fn default() -> Self {
        Self {
            video_decode: VideoDecodePreference::default(),
            audio_output: PcmFormat::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenedMediaInfo {
    pub uri: String,
    pub duration: Option<Duration>,
    pub tracks: Vec<TrackInfo>,
    pub video_params: Option<VideoParams>,
    pub selected_video_track: Option<i64>,
    pub selected_audio_track: Option<i64>,
    pub video_decode_backend: Option<DecoderBackend>,
    pub audio_output: Option<PcmFormat>,
}

pub struct PlaybackSession {
    demuxer: Demuxer,
    video_decoder: Option<Decoder>,
    audio_decoder: Option<Decoder>,
    audio_resampler: Option<AudioResampler>,
    audio_output: PcmFormat,
    info: OpenedMediaInfo,
    video_frames: VecDeque<Frame>,
    audio_frames: VecDeque<PcmAudioFrame>,
    eof: bool,
}

unsafe impl Send for PlaybackSession {}

impl PlaybackSession {
    pub fn open(request: &MediaRequest, config: PlaybackSessionConfig) -> Result<Self> {
        let source = source_from_uri_with_hint(&request.uri, request.source_hint)?;
        let mut demuxer = Demuxer::open_source(source)?;
        let selected_video_track = demuxer
            .probe()
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Video)
            .map(|track| track.id as i32);
        let selected_audio_track = demuxer
            .probe()
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Audio)
            .map(|track| track.id as i32);

        let mut video_decoder = None;
        let mut selected_streams = Vec::new();
        if let Some(stream_index) = selected_video_track {
            selected_streams.push(stream_index);
            let parameters = demuxer.codec_parameters(stream_index)?;
            video_decoder = Some(Decoder::open_with_config(
                parameters,
                config.video_decode.decoder_config(),
            )?);
        }
        let mut audio_decoder = None;
        if let Some(stream_index) = selected_audio_track {
            selected_streams.push(stream_index);
            let parameters = demuxer.codec_parameters(stream_index)?;
            audio_decoder = Some(Decoder::open(parameters)?);
        }
        if !selected_streams.is_empty() {
            demuxer.set_stream_selection(StreamSelection::only(selected_streams))?;
        }

        let probe = demuxer.probe().clone();
        let video_params = selected_video_track.and_then(|stream_index| {
            probe
                .video
                .iter()
                .find(|video| video.track_id == stream_index as i64)
                .map(|video| video.params.clone())
        });
        let info = OpenedMediaInfo {
            uri: probe.uri,
            duration: probe.duration,
            tracks: probe.tracks,
            video_params,
            selected_video_track: selected_video_track.map(i64::from),
            selected_audio_track: selected_audio_track.map(i64::from),
            video_decode_backend: video_decoder.as_ref().map(Decoder::backend),
            audio_output: audio_decoder.as_ref().map(|_| config.audio_output),
        };

        Ok(Self {
            demuxer,
            video_decoder,
            audio_decoder,
            audio_resampler: None,
            audio_output: config.audio_output,
            info,
            video_frames: VecDeque::new(),
            audio_frames: VecDeque::new(),
            eof: false,
        })
    }

    pub fn info(&self) -> &OpenedMediaInfo {
        &self.info
    }

    pub fn next_video_frame(&mut self) -> Result<Option<Frame>> {
        if self.video_decoder.is_none() {
            return Ok(None);
        }
        while self.video_frames.is_empty() && !self.eof {
            self.pump_once()?;
        }
        Ok(self.video_frames.pop_front())
    }

    pub fn next_audio_frame(&mut self) -> Result<Option<PcmAudioFrame>> {
        if self.audio_decoder.is_none() {
            return Ok(None);
        }
        while self.audio_frames.is_empty() && !self.eof {
            if self.video_decoder.is_some() && self.video_frames.len() >= VIDEO_FRAME_QUEUE_LIMIT {
                return Ok(None);
            }
            self.pump_once()?;
        }
        Ok(self.audio_frames.pop_front())
    }

    pub fn seek(&mut self, position: Duration) -> Result<()> {
        self.demuxer.seek(position)?;
        if let Some(decoder) = &mut self.video_decoder {
            decoder.flush();
        }
        if let Some(decoder) = &mut self.audio_decoder {
            decoder.flush();
        }
        self.audio_resampler = None;
        self.video_frames.clear();
        self.audio_frames.clear();
        self.eof = false;
        Ok(())
    }

    fn pump_once(&mut self) -> Result<()> {
        match self.demuxer.read_packet()? {
            Some(packet) => self.route_packet(packet),
            None => self.finish_decoders(),
        }
    }

    fn route_packet(&mut self, packet: ffmpeg::Packet) -> Result<()> {
        if self
            .video_decoder
            .as_ref()
            .is_some_and(|decoder| packet.stream_index() == decoder.stream_index())
        {
            let decoder = self.video_decoder.as_mut().expect("video decoder exists");
            decoder.send_packet(&packet)?;
            drain_video_frames(decoder, &mut self.video_frames)?;
            trim_video_queue(&mut self.video_frames);
            return Ok(());
        }

        if self
            .audio_decoder
            .as_ref()
            .is_some_and(|decoder| packet.stream_index() == decoder.stream_index())
        {
            {
                let decoder = self.audio_decoder.as_mut().expect("audio decoder exists");
                decoder.send_packet(&packet)?;
            }
            self.drain_audio_frames()?;
            trim_audio_queue(&mut self.audio_frames);
        }
        Ok(())
    }

    fn finish_decoders(&mut self) -> Result<()> {
        if self.eof {
            return Ok(());
        }
        if let Some(decoder) = &mut self.video_decoder {
            decoder.send_eof()?;
            drain_video_frames(decoder, &mut self.video_frames)?;
            trim_video_queue(&mut self.video_frames);
        }
        if self.audio_decoder.is_some() {
            {
                let decoder = self.audio_decoder.as_mut().expect("audio decoder exists");
                decoder.send_eof()?;
            }
            self.drain_audio_frames()?;
            trim_audio_queue(&mut self.audio_frames);
        }
        self.eof = true;
        Ok(())
    }

    fn drain_audio_frames(&mut self) -> Result<()> {
        loop {
            let output = {
                let decoder = self.audio_decoder.as_mut().expect("audio decoder exists");
                decoder.receive_frame()?
            };
            match output {
                DecoderOutputFrame::Frame(frame) => {
                    let pcm = self.convert_audio_frame(frame)?;
                    self.audio_frames.push_back(pcm);
                }
                DecoderOutputFrame::NeedMoreInput | DecoderOutputFrame::EndOfStream => {
                    return Ok(());
                }
            }
        }
    }

    fn convert_audio_frame(&mut self, frame: Frame) -> Result<PcmAudioFrame> {
        if self.audio_resampler.is_none() {
            self.audio_resampler = Some(AudioResampler::new_from_frame(&frame, self.audio_output)?);
        }
        self.audio_resampler
            .as_mut()
            .expect("audio resampler exists")
            .convert(&frame)
            .map_err(PlaybackError::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackRunState {
    Paused,
    Playing,
    Stopped,
    Ended,
}

pub struct TimedVideoFrame {
    pub frame: Frame,
    pub pts: Option<Duration>,
    pub media_time: Duration,
    pub late_by: Option<Duration>,
}

pub struct TimedAudioFrame {
    pub frame: PcmAudioFrame,
    pub pts: Option<Duration>,
    pub media_time: Duration,
    pub late_by: Option<Duration>,
}

pub struct VideoPlaybackEngine {
    session: PlaybackSession,
    state: PlaybackRunState,
    clock_base: Duration,
    started_at: Option<Instant>,
    pending_frame: Option<Frame>,
    pending_audio: Option<PcmAudioFrame>,
    last_presented_pts: Option<Duration>,
    frame_lead_time: Duration,
    audio_lead_time: Duration,
    eof: bool,
    waiting_for_first_frame: bool,
}

unsafe impl Send for VideoPlaybackEngine {}

impl VideoPlaybackEngine {
    pub fn open(request: &MediaRequest, config: PlaybackSessionConfig) -> Result<Self> {
        Ok(Self::from_session(PlaybackSession::open(request, config)?))
    }

    pub fn from_session(session: PlaybackSession) -> Self {
        Self {
            session,
            state: PlaybackRunState::Paused,
            clock_base: Duration::ZERO,
            started_at: None,
            pending_frame: None,
            pending_audio: None,
            last_presented_pts: None,
            frame_lead_time: Duration::from_millis(4),
            audio_lead_time: Duration::from_millis(12),
            eof: false,
            waiting_for_first_frame: false,
        }
    }

    pub fn info(&self) -> &OpenedMediaInfo {
        self.session.info()
    }

    pub fn state(&self) -> PlaybackRunState {
        self.state
    }

    pub fn play(&mut self) {
        if matches!(
            self.state,
            PlaybackRunState::Playing | PlaybackRunState::Ended
        ) {
            return;
        }
        self.started_at = Some(Instant::now());
        self.state = PlaybackRunState::Playing;
        self.waiting_for_first_frame = self.last_presented_pts.is_none();
    }

    pub fn pause(&mut self) {
        if self.state != PlaybackRunState::Playing {
            return;
        }
        self.clock_base = self.media_time();
        self.started_at = None;
        self.state = PlaybackRunState::Paused;
    }

    pub fn stop(&mut self) {
        self.clock_base = Duration::ZERO;
        self.started_at = None;
        self.pending_frame = None;
        self.pending_audio = None;
        self.last_presented_pts = None;
        self.state = PlaybackRunState::Stopped;
        self.eof = false;
        self.waiting_for_first_frame = false;
    }

    pub fn seek(&mut self, position: Duration) -> Result<()> {
        self.session.seek(position)?;
        self.clock_base = position;
        self.started_at = (self.state == PlaybackRunState::Playing).then(Instant::now);
        self.pending_frame = None;
        self.pending_audio = None;
        self.last_presented_pts = None;
        self.eof = false;
        self.waiting_for_first_frame = self.state == PlaybackRunState::Playing;
        Ok(())
    }

    pub fn media_time(&self) -> Duration {
        match (self.state, self.started_at) {
            (PlaybackRunState::Playing, Some(started_at)) => self.clock_base + started_at.elapsed(),
            _ => self.clock_base,
        }
    }

    pub fn next_audio_frame(&mut self) -> Result<Option<PcmAudioFrame>> {
        self.session.next_audio_frame()
    }

    pub fn tick_audio(&mut self) -> Result<Option<TimedAudioFrame>> {
        if self.state != PlaybackRunState::Playing {
            return Ok(None);
        }
        self.ensure_pending_audio()?;
        let Some(frame) = self.pending_audio.as_ref() else {
            return Ok(None);
        };

        let pts = frame.pts;
        let media_time = self.media_time();
        if pts.is_some_and(|pts| pts > media_time + self.audio_lead_time) {
            return Ok(None);
        }

        let frame = self.pending_audio.take().expect("pending audio exists");
        let late_by = pts.and_then(|pts| media_time.checked_sub(pts));
        Ok(Some(TimedAudioFrame {
            frame,
            pts,
            media_time,
            late_by,
        }))
    }

    pub fn tick(&mut self) -> Result<Option<TimedVideoFrame>> {
        if self.state != PlaybackRunState::Playing {
            return Ok(None);
        }
        self.ensure_pending_frame()?;
        let Some(frame) = self.pending_frame.as_ref() else {
            return Ok(None);
        };

        let pts = frame.pts().and_then(|pts| pts.as_duration());
        let should_present_first = self.last_presented_pts.is_none();
        if should_present_first && self.waiting_for_first_frame {
            self.clock_base = pts.unwrap_or(Duration::ZERO);
            self.started_at = Some(Instant::now());
            self.waiting_for_first_frame = false;
        }

        let media_time = self.media_time();
        let should_present_by_time = pts.is_none_or(|pts| pts <= media_time + self.frame_lead_time);
        if !should_present_first && !should_present_by_time {
            return Ok(None);
        }

        let frame = self.pending_frame.take().expect("pending frame exists");
        let late_by = pts.and_then(|pts| media_time.checked_sub(pts));
        self.last_presented_pts = pts;
        Ok(Some(TimedVideoFrame {
            frame,
            pts,
            media_time,
            late_by,
        }))
    }

    fn ensure_pending_frame(&mut self) -> Result<()> {
        if self.pending_frame.is_some() || self.eof {
            return Ok(());
        }
        self.pending_frame = self.session.next_video_frame()?;
        if self.pending_frame.is_none() {
            self.eof = true;
            self.state = PlaybackRunState::Ended;
            self.started_at = None;
            self.clock_base = self.info().duration.unwrap_or_else(|| self.media_time());
        }
        Ok(())
    }

    fn ensure_pending_audio(&mut self) -> Result<()> {
        if self.pending_audio.is_some() || self.eof {
            return Ok(());
        }
        self.pending_audio = self.session.next_audio_frame()?;
        Ok(())
    }
}

fn drain_video_frames(decoder: &mut Decoder, frames: &mut VecDeque<Frame>) -> Result<()> {
    loop {
        match decoder.receive_frame()? {
            DecoderOutputFrame::Frame(frame) => frames.push_back(frame),
            DecoderOutputFrame::NeedMoreInput | DecoderOutputFrame::EndOfStream => return Ok(()),
        }
    }
}

fn trim_video_queue(frames: &mut VecDeque<Frame>) {
    while frames.len() > VIDEO_FRAME_QUEUE_LIMIT {
        let _ = frames.pop_back();
    }
}

fn trim_audio_queue(frames: &mut VecDeque<PcmAudioFrame>) {
    while frames.len() > AUDIO_FRAME_QUEUE_LIMIT {
        let _ = frames.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opened_media_info_keeps_probe_summary() {
        let info = OpenedMediaInfo {
            uri: "file:///tmp/test.mp4".to_string(),
            duration: Some(Duration::from_secs(12)),
            tracks: vec![TrackInfo {
                id: 0,
                kind: TrackKind::Video,
                title: None,
                language: None,
                codec: Some("hevc".to_string()),
            }],
            video_params: Some(VideoParams {
                width: 3840,
                height: 2160,
                primaries: crate::core::ColorPrimaries::Bt2020,
                transfer: crate::core::TransferFunction::Pq,
            }),
            selected_video_track: Some(0),
            selected_audio_track: Some(1),
            video_decode_backend: Some(DecoderBackend::Software),
            audio_output: Some(PcmFormat::default()),
        };

        assert_eq!(info.duration, Some(Duration::from_secs(12)));
        assert_eq!(info.tracks.len(), 1);
        assert_eq!(
            info.video_params.as_ref().map(|params| params.width),
            Some(3840)
        );
        assert_eq!(info.selected_video_track, Some(0));
        assert_eq!(info.selected_audio_track, Some(1));
        assert_eq!(info.video_decode_backend, Some(DecoderBackend::Software));
        assert_eq!(info.audio_output, Some(PcmFormat::default()));
    }
}
