use std::collections::BTreeSet;
use std::ffi::{CStr, CString, c_int, c_void};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;
use std::slice;
use std::time::Duration;

use crate::core::{ColorPrimaries, TrackInfo, TrackKind, TransferFunction, VideoParams};
use crate::source::{ByteRange, MediaSource};
use kuroko_ffmpeg_sys as sys;
use libc::{EAGAIN, EINVAL, EIO, ESPIPE, SEEK_CUR, SEEK_END, SEEK_SET};
use thiserror::Error;

const AVERROR_EOF: i32 = -541_478_725;

#[derive(Debug, Error)]
pub enum FfmpegError {
    #[error("path contains interior nul byte")]
    InteriorNul,
    #[error("ffmpeg error in {operation}: {message} ({code})")]
    Api {
        operation: &'static str,
        code: i32,
        message: String,
    },
    #[error("ffmpeg returned a null pointer from {0}")]
    NullPointer(&'static str),
    #[error("unknown stream index: {0}")]
    UnknownStream(i32),
    #[error("packet stream {packet_stream} does not match decoder stream {decoder_stream}")]
    StreamMismatch {
        decoder_stream: i32,
        packet_stream: i32,
    },
    #[error("expected audio frame")]
    ExpectedAudioFrame,
    #[error("source error: {0}")]
    Source(String),
}

pub type Result<T> = std::result::Result<T, FfmpegError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeBase {
    pub num: i32,
    pub den: i32,
}

impl TimeBase {
    pub fn seconds_from_timestamp(self, timestamp: i64) -> f64 {
        timestamp as f64 * self.num as f64 / self.den as f64
    }

    fn from_av(rational: sys::AVRational) -> Self {
        if rational.den == 0 {
            return Self { num: 0, den: 1 };
        }
        Self {
            num: rational.num,
            den: rational.den,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketTimestamp {
    pub raw: i64,
    pub time_base: TimeBase,
}

impl PacketTimestamp {
    pub fn seconds(self) -> f64 {
        self.time_base.seconds_from_timestamp(self.raw)
    }

    pub fn as_duration(self) -> Option<Duration> {
        let seconds = self.seconds();
        if seconds.is_sign_negative() || !seconds.is_finite() {
            return None;
        }
        Some(Duration::from_secs_f64(seconds))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketFlags {
    bits: i32,
}

impl PacketFlags {
    pub fn bits(self) -> i32 {
        self.bits
    }

    pub fn is_key(self) -> bool {
        self.bits & sys::AV_PKT_FLAG_KEY as i32 != 0
    }

    pub fn is_corrupt(self) -> bool {
        self.bits & sys::AV_PKT_FLAG_CORRUPT as i32 != 0
    }

    pub fn is_discard(self) -> bool {
        self.bits & sys::AV_PKT_FLAG_DISCARD as i32 != 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaProbe {
    pub uri: String,
    pub duration: Option<Duration>,
    pub tracks: Vec<TrackInfo>,
    pub video: Vec<VideoProbe>,
    pub audio: Vec<AudioProbe>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoProbe {
    pub track_id: i64,
    pub params: VideoParams,
    pub codec: Option<String>,
    pub pixel_format: Option<String>,
    pub profile: Option<String>,
    pub level: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioProbe {
    pub track_id: i64,
    pub codec: Option<String>,
    pub sample_rate: u32,
    pub channels: u32,
    pub sample_format: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmSampleFormat {
    F32Interleaved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u32,
    pub sample_format: PcmSampleFormat,
}

impl PcmFormat {
    pub fn f32_interleaved(sample_rate: u32, channels: u32) -> Self {
        Self {
            sample_rate,
            channels,
            sample_format: PcmSampleFormat::F32Interleaved,
        }
    }
}

impl Default for PcmFormat {
    fn default() -> Self {
        Self::f32_interleaved(48_000, 2)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PcmAudioFrame {
    pub format: PcmFormat,
    pub pts: Option<Duration>,
    pub frames: usize,
    pub samples: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamSelection {
    All,
    Only(BTreeSet<i32>),
}

impl StreamSelection {
    pub fn all() -> Self {
        Self::All
    }

    pub fn only<I>(stream_indices: I) -> Self
    where
        I: IntoIterator<Item = i32>,
    {
        Self::Only(stream_indices.into_iter().collect())
    }

    fn accepts(&self, stream_index: i32) -> bool {
        match self {
            Self::All => true,
            Self::Only(streams) => streams.contains(&stream_index),
        }
    }
}

pub struct Demuxer {
    context: FormatContext,
    probe: MediaProbe,
    stream_time_bases: Vec<Option<TimeBase>>,
    selection: StreamSelection,
}

impl Demuxer {
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_uri(&path.as_ref().to_string_lossy())
    }

    pub fn open_uri(uri: &str) -> Result<Self> {
        let mut context = open_format_context(uri)?;
        find_stream_info(&mut context)?;
        let (probe, stream_time_bases) = inspect_format_context(uri, context.as_ptr());
        Ok(Self {
            context,
            probe,
            stream_time_bases,
            selection: StreamSelection::all(),
        })
    }

    pub fn open_source(source: Box<dyn MediaSource>) -> Result<Self> {
        let uri = source.uri().to_string();
        let mut context = open_source_format_context(source)?;
        find_stream_info(&mut context)?;
        let (probe, stream_time_bases) = inspect_format_context(&uri, context.as_ptr());
        Ok(Self {
            context,
            probe,
            stream_time_bases,
            selection: StreamSelection::all(),
        })
    }

    pub fn probe(&self) -> &MediaProbe {
        &self.probe
    }

    pub fn selection(&self) -> &StreamSelection {
        &self.selection
    }

    pub fn set_stream_selection(&mut self, selection: StreamSelection) -> Result<()> {
        if let StreamSelection::Only(streams) = &selection {
            for stream_index in streams {
                if !self.has_stream(*stream_index) {
                    return Err(FfmpegError::UnknownStream(*stream_index));
                }
            }
        }
        self.selection = selection;
        Ok(())
    }

    pub fn stream_time_base(&self, stream_index: i32) -> Option<TimeBase> {
        if stream_index < 0 {
            return None;
        }
        self.stream_time_bases
            .get(stream_index as usize)
            .copied()
            .flatten()
    }

    pub fn codec_parameters(&self, stream_index: i32) -> Result<CodecParameters<'_>> {
        if stream_index < 0 {
            return Err(FfmpegError::UnknownStream(stream_index));
        }
        let raw = self.context.as_ptr();
        let stream_count = unsafe { (*raw).nb_streams as usize };
        let Some(stream_slot) = (stream_index as usize)
            .checked_sub(0)
            .filter(|index| *index < stream_count)
        else {
            return Err(FfmpegError::UnknownStream(stream_index));
        };
        let stream = unsafe { *(*raw).streams.add(stream_slot) };
        if stream.is_null() {
            return Err(FfmpegError::UnknownStream(stream_index));
        }
        let codecpar = unsafe { (*stream).codecpar };
        if codecpar.is_null() {
            return Err(FfmpegError::NullPointer("AVStream.codecpar"));
        }
        Ok(CodecParameters {
            ptr: codecpar,
            stream_index,
            time_base: TimeBase::from_av(unsafe { (*stream).time_base }),
            _owner: PhantomData,
        })
    }

    pub fn open_decoder(&self, stream_index: i32) -> Result<Decoder> {
        Decoder::open(self.codec_parameters(stream_index)?)
    }

    pub fn read_packet(&mut self) -> Result<Option<Packet>> {
        loop {
            let mut packet = Packet::alloc()?;
            let code =
                unsafe { sys::av_read_frame(self.context.as_mut_ptr(), packet.as_mut_ptr()) };
            if code == AVERROR_EOF {
                return Ok(None);
            }
            check(code, "av_read_frame")?;

            let stream_index = packet.stream_index();
            packet.time_base = self.stream_time_base(stream_index);
            if self.selection.accepts(stream_index) {
                return Ok(Some(packet));
            }
        }
    }

    pub fn seek(&mut self, position: Duration) -> Result<()> {
        let target = position.as_micros().min(i64::MAX as u128) as i64;
        check(
            unsafe {
                sys::av_seek_frame(
                    self.context.as_mut_ptr(),
                    -1,
                    target,
                    sys::AVSEEK_FLAG_BACKWARD as i32,
                )
            },
            "av_seek_frame",
        )?;
        check(
            unsafe { sys::avformat_flush(self.context.as_mut_ptr()) },
            "avformat_flush",
        )?;
        Ok(())
    }

    fn has_stream(&self, stream_index: i32) -> bool {
        self.stream_time_base(stream_index).is_some()
            || self
                .probe
                .tracks
                .iter()
                .any(|track| track.id == stream_index as i64)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CodecParameters<'a> {
    ptr: *const sys::AVCodecParameters,
    stream_index: i32,
    time_base: TimeBase,
    _owner: PhantomData<&'a Demuxer>,
}

impl CodecParameters<'_> {
    pub fn stream_index(self) -> i32 {
        self.stream_index
    }

    pub fn time_base(self) -> TimeBase {
        self.time_base
    }

    pub fn codec_name(self) -> Option<String> {
        unsafe { codec_name((*self.ptr).codec_id) }
    }

    pub fn kind(self) -> Option<TrackKind> {
        unsafe { track_kind((*self.ptr).codec_type) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderOutput {
    Frame,
    NeedMoreInput,
    EndOfStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderBackend {
    Software,
    VideoToolbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderConfig {
    pub backend: DecoderBackend,
}

impl DecoderConfig {
    pub fn software() -> Self {
        Self {
            backend: DecoderBackend::Software,
        }
    }

    pub fn videotoolbox() -> Self {
        Self {
            backend: DecoderBackend::VideoToolbox,
        }
    }
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self::software()
    }
}

struct HardwareDecoderState {
    device_ref: *mut sys::AVBufferRef,
    pixel_format: sys::AVPixelFormat,
}

impl Drop for HardwareDecoderState {
    fn drop(&mut self) {
        unsafe { sys::av_buffer_unref(&mut self.device_ref) };
    }
}

pub struct Decoder {
    context: *mut sys::AVCodecContext,
    stream_index: i32,
    time_base: TimeBase,
    backend: DecoderBackend,
    hw_state: Option<Box<HardwareDecoderState>>,
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn open(parameters: CodecParameters<'_>) -> Result<Self> {
        Self::open_with_config(parameters, DecoderConfig::default())
    }

    pub fn open_with_config(
        parameters: CodecParameters<'_>,
        config: DecoderConfig,
    ) -> Result<Self> {
        let codec_id = unsafe { (*parameters.ptr).codec_id };
        let codec = unsafe { sys::avcodec_find_decoder(codec_id) };
        if codec.is_null() {
            return Err(FfmpegError::NullPointer("avcodec_find_decoder"));
        }
        let context = unsafe { sys::avcodec_alloc_context3(codec) };
        if context.is_null() {
            return Err(FfmpegError::NullPointer("avcodec_alloc_context3"));
        }
        let decoder = Self {
            context,
            stream_index: parameters.stream_index,
            time_base: parameters.time_base,
            backend: config.backend,
            hw_state: None,
        };
        check(
            unsafe { sys::avcodec_parameters_to_context(decoder.context, parameters.ptr) },
            "avcodec_parameters_to_context",
        )?;
        let mut decoder = decoder;
        if config.backend == DecoderBackend::VideoToolbox {
            decoder.configure_videotoolbox(codec)?;
        }
        check(
            unsafe { sys::avcodec_open2(decoder.context, codec, ptr::null_mut()) },
            "avcodec_open2",
        )?;
        Ok(decoder)
    }

    pub fn stream_index(&self) -> i32 {
        self.stream_index
    }

    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }

    pub fn backend(&self) -> DecoderBackend {
        self.backend
    }

    pub fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if packet.stream_index() != self.stream_index {
            return Err(FfmpegError::StreamMismatch {
                decoder_stream: self.stream_index,
                packet_stream: packet.stream_index(),
            });
        }
        check(
            unsafe { sys::avcodec_send_packet(self.context, packet.as_ptr()) },
            "avcodec_send_packet",
        )
    }

    pub fn send_eof(&mut self) -> Result<()> {
        check(
            unsafe { sys::avcodec_send_packet(self.context, ptr::null()) },
            "avcodec_send_packet_eof",
        )
    }

    pub fn receive_frame(&mut self) -> Result<DecoderOutputFrame> {
        let frame = Frame::alloc(self.time_base)?;
        let code = unsafe { sys::avcodec_receive_frame(self.context, frame.ptr) };
        if code == av_error(EAGAIN) {
            return Ok(DecoderOutputFrame::NeedMoreInput);
        }
        if code == AVERROR_EOF {
            return Ok(DecoderOutputFrame::EndOfStream);
        }
        check(code, "avcodec_receive_frame")?;
        Ok(DecoderOutputFrame::Frame(frame))
    }

    pub fn flush(&mut self) {
        unsafe { sys::avcodec_flush_buffers(self.context) };
    }

    fn configure_videotoolbox(&mut self, codec: *const sys::AVCodec) -> Result<()> {
        let pixel_format =
            hardware_pixel_format(codec, sys::AVHWDeviceType_AV_HWDEVICE_TYPE_VIDEOTOOLBOX)
                .ok_or_else(|| FfmpegError::NullPointer("avcodec_get_hw_config(VideoToolbox)"))?;
        let mut device_ref = ptr::null_mut();
        check(
            unsafe {
                sys::av_hwdevice_ctx_create(
                    &mut device_ref,
                    sys::AVHWDeviceType_AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                    ptr::null(),
                    ptr::null_mut(),
                    0,
                )
            },
            "av_hwdevice_ctx_create(VideoToolbox)",
        )?;
        if device_ref.is_null() {
            return Err(FfmpegError::NullPointer(
                "av_hwdevice_ctx_create(VideoToolbox)",
            ));
        }

        let context_device_ref = unsafe { sys::av_buffer_ref(device_ref) };
        if context_device_ref.is_null() {
            unsafe { sys::av_buffer_unref(&mut device_ref) };
            return Err(FfmpegError::NullPointer("av_buffer_ref(hw_device_ctx)"));
        }

        let mut hw_state = Box::new(HardwareDecoderState {
            device_ref,
            pixel_format,
        });

        unsafe {
            (*self.context).hw_device_ctx = context_device_ref;
            (*self.context).opaque = (&mut *hw_state) as *mut HardwareDecoderState as *mut _;
            (*self.context).get_format = Some(select_hw_format);
        }
        self.hw_state = Some(hw_state);
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { sys::avcodec_free_context(&mut self.context) };
    }
}

pub enum DecoderOutputFrame {
    Frame(Frame),
    NeedMoreInput,
    EndOfStream,
}

pub struct Frame {
    ptr: *mut sys::AVFrame,
    time_base: TimeBase,
}

pub struct AudioResampler {
    context: *mut sys::SwrContext,
    output_format: PcmFormat,
    #[allow(dead_code)]
    output_layout: ChannelLayout,
}

unsafe impl Send for AudioResampler {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoToolboxPixelBuffer<'a> {
    raw: *mut c_void,
    width: u32,
    height: u32,
    _frame: PhantomData<&'a Frame>,
}

impl VideoToolboxPixelBuffer<'_> {
    pub fn raw(self) -> *mut c_void {
        self.raw
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }
}

unsafe impl Send for Frame {}

impl Frame {
    fn alloc(time_base: TimeBase) -> Result<Self> {
        let ptr = unsafe { sys::av_frame_alloc() };
        if ptr.is_null() {
            return Err(FfmpegError::NullPointer("av_frame_alloc"));
        }
        Ok(Self { ptr, time_base })
    }

    pub fn as_ptr(&self) -> *const sys::AVFrame {
        self.ptr
    }

    pub fn width(&self) -> u32 {
        unsafe { (*self.ptr).width.max(0) as u32 }
    }

    pub fn height(&self) -> u32 {
        unsafe { (*self.ptr).height.max(0) as u32 }
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe { (*self.ptr).sample_rate.max(0) as u32 }
    }

    pub fn channel_count(&self) -> u32 {
        unsafe { (*self.ptr).ch_layout.nb_channels.max(0) as u32 }
    }

    pub fn sample_count(&self) -> usize {
        unsafe { (*self.ptr).nb_samples.max(0) as usize }
    }

    pub fn sample_format(&self) -> Option<String> {
        unsafe { sample_format_name((*self.ptr).format) }
    }

    pub fn raw_sample_format(&self) -> sys::AVSampleFormat {
        unsafe { (*self.ptr).format }
    }

    pub fn is_audio(&self) -> bool {
        self.sample_rate() > 0 && self.sample_count() > 0 && self.channel_count() > 0
    }

    pub fn pixel_format(&self) -> Option<String> {
        unsafe { pixel_format_name((*self.ptr).format) }
    }

    pub fn raw_pixel_format(&self) -> i32 {
        unsafe { (*self.ptr).format }
    }

    pub fn is_videotoolbox(&self) -> bool {
        self.raw_pixel_format() == sys::AVPixelFormat_AV_PIX_FMT_VIDEOTOOLBOX
    }

    pub fn has_hw_frames_context(&self) -> bool {
        unsafe { !(*self.ptr).hw_frames_ctx.is_null() }
    }

    pub fn videotoolbox_pixel_buffer(&self) -> Option<VideoToolboxPixelBuffer<'_>> {
        if !self.is_videotoolbox() {
            return None;
        }
        let raw = unsafe { (*self.ptr).data[3] }.cast::<c_void>();
        if raw.is_null() {
            return None;
        }
        Some(VideoToolboxPixelBuffer {
            raw,
            width: self.width(),
            height: self.height(),
            _frame: PhantomData,
        })
    }

    pub fn raw_pts(&self) -> Option<i64> {
        timestamp_value(unsafe { (*self.ptr).pts })
    }

    pub fn pts(&self) -> Option<PacketTimestamp> {
        Some(PacketTimestamp {
            raw: self.raw_pts()?,
            time_base: self.time_base,
        })
    }

    pub fn color_primaries(&self) -> ColorPrimaries {
        unsafe { color_primaries((*self.ptr).color_primaries) }
    }

    pub fn transfer_function(&self) -> TransferFunction {
        unsafe { transfer_function((*self.ptr).color_trc) }
    }

    pub fn transfer_to_system_memory(&self) -> Result<Frame> {
        let frame = Frame::alloc(self.time_base)?;
        check(
            unsafe { sys::av_hwframe_transfer_data(frame.ptr, self.ptr, 0) },
            "av_hwframe_transfer_data",
        )?;
        unsafe {
            (*frame.ptr).pts = (*self.ptr).pts;
            (*frame.ptr).color_primaries = (*self.ptr).color_primaries;
            (*frame.ptr).color_trc = (*self.ptr).color_trc;
        }
        Ok(frame)
    }
}

impl AudioResampler {
    pub fn new_from_frame(frame: &Frame, output_format: PcmFormat) -> Result<Self> {
        if !frame.is_audio() {
            return Err(FfmpegError::ExpectedAudioFrame);
        }
        let output_layout = ChannelLayout::default_for_channels(output_format.channels)?;
        let mut context = ptr::null_mut();
        check(
            unsafe {
                sys::swr_alloc_set_opts2(
                    &mut context,
                    output_layout.as_ptr(),
                    sys::AVSampleFormat_AV_SAMPLE_FMT_FLT,
                    output_format.sample_rate as i32,
                    &(*frame.ptr).ch_layout,
                    frame.raw_sample_format(),
                    frame.sample_rate() as i32,
                    0,
                    ptr::null_mut(),
                )
            },
            "swr_alloc_set_opts2",
        )?;
        if context.is_null() {
            return Err(FfmpegError::NullPointer("swr_alloc_set_opts2"));
        }
        check(unsafe { sys::swr_init(context) }, "swr_init")?;
        Ok(Self {
            context,
            output_format,
            output_layout,
        })
    }

    pub fn output_format(&self) -> PcmFormat {
        self.output_format
    }

    pub fn convert(&mut self, frame: &Frame) -> Result<PcmAudioFrame> {
        if !frame.is_audio() {
            return Err(FfmpegError::ExpectedAudioFrame);
        }
        let input_samples = frame.sample_count().min(i32::MAX as usize) as i32;
        let delay = unsafe { sys::swr_get_delay(self.context, frame.sample_rate() as i64) }.max(0);
        let output_capacity = unsafe {
            sys::av_rescale_rnd(
                delay + input_samples as i64,
                self.output_format.sample_rate as i64,
                frame.sample_rate() as i64,
                sys::AVRounding_AV_ROUND_UP,
            )
        }
        .max(1)
        .min(i32::MAX as i64) as i32;
        let channels = self.output_format.channels.max(1) as usize;
        let mut samples = vec![0.0f32; output_capacity as usize * channels];
        let mut output_planes = [samples.as_mut_ptr().cast::<u8>()];
        let input = unsafe { (*frame.ptr).extended_data as *const *const u8 };
        if input.is_null() {
            return Err(FfmpegError::NullPointer("AVFrame.extended_data"));
        }
        let converted = unsafe {
            sys::swr_convert(
                self.context,
                output_planes.as_mut_ptr(),
                output_capacity,
                input,
                input_samples,
            )
        };
        check(converted, "swr_convert")?;
        let frames = converted.max(0) as usize;
        samples.truncate(frames * channels);
        Ok(PcmAudioFrame {
            format: self.output_format,
            pts: frame.pts().and_then(|pts| pts.as_duration()),
            frames,
            samples,
        })
    }
}

impl Drop for AudioResampler {
    fn drop(&mut self) {
        unsafe { sys::swr_free(&mut self.context) };
    }
}

struct ChannelLayout {
    raw: sys::AVChannelLayout,
}

impl ChannelLayout {
    fn default_for_channels(channels: u32) -> Result<Self> {
        let channels = channels.min(i32::MAX as u32) as i32;
        let mut raw = sys::AVChannelLayout::default();
        unsafe { sys::av_channel_layout_default(&mut raw, channels) };
        if unsafe { sys::av_channel_layout_check(&raw) } == 0 {
            unsafe { sys::av_channel_layout_uninit(&mut raw) };
            return Err(FfmpegError::Api {
                operation: "av_channel_layout_default",
                code: -1,
                message: "invalid channel layout".to_string(),
            });
        }
        Ok(Self { raw })
    }

    fn as_ptr(&self) -> *const sys::AVChannelLayout {
        &self.raw
    }
}

impl Drop for ChannelLayout {
    fn drop(&mut self) {
        unsafe { sys::av_channel_layout_uninit(&mut self.raw) };
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { sys::av_frame_free(&mut self.ptr) };
    }
}

pub struct Packet {
    ptr: *mut sys::AVPacket,
    time_base: Option<TimeBase>,
}

unsafe impl Send for Packet {}

impl Packet {
    fn alloc() -> Result<Self> {
        let ptr = unsafe { sys::av_packet_alloc() };
        if ptr.is_null() {
            return Err(FfmpegError::NullPointer("av_packet_alloc"));
        }
        Ok(Self {
            ptr,
            time_base: None,
        })
    }

    pub fn as_ptr(&self) -> *const sys::AVPacket {
        self.ptr
    }

    pub fn as_mut_ptr(&mut self) -> *mut sys::AVPacket {
        self.ptr
    }

    pub fn stream_index(&self) -> i32 {
        unsafe { (*self.ptr).stream_index }
    }

    pub fn size(&self) -> usize {
        unsafe { (*self.ptr).size.max(0) as usize }
    }

    pub fn data(&self) -> &[u8] {
        let size = self.size();
        let data = unsafe { (*self.ptr).data };
        if data.is_null() || size == 0 {
            return &[];
        }
        unsafe { slice::from_raw_parts(data, size) }
    }

    pub fn flags(&self) -> PacketFlags {
        PacketFlags {
            bits: unsafe { (*self.ptr).flags },
        }
    }

    pub fn is_key(&self) -> bool {
        self.flags().is_key()
    }

    pub fn pos(&self) -> Option<i64> {
        let pos = unsafe { (*self.ptr).pos };
        if pos < 0 { None } else { Some(pos) }
    }

    pub fn raw_pts(&self) -> Option<i64> {
        timestamp_value(unsafe { (*self.ptr).pts })
    }

    pub fn raw_dts(&self) -> Option<i64> {
        timestamp_value(unsafe { (*self.ptr).dts })
    }

    pub fn raw_duration(&self) -> Option<i64> {
        let duration = unsafe { (*self.ptr).duration };
        if duration <= 0 { None } else { Some(duration) }
    }

    pub fn time_base(&self) -> Option<TimeBase> {
        self.time_base
    }

    pub fn pts(&self) -> Option<PacketTimestamp> {
        Some(PacketTimestamp {
            raw: self.raw_pts()?,
            time_base: self.time_base?,
        })
    }

    pub fn dts(&self) -> Option<PacketTimestamp> {
        Some(PacketTimestamp {
            raw: self.raw_dts()?,
            time_base: self.time_base?,
        })
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        Some(self.time_base?.seconds_from_timestamp(self.raw_duration()?))
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { sys::av_packet_free(&mut self.ptr) };
    }
}

pub fn version() -> String {
    unsafe {
        let ptr = sys::av_version_info();
        if ptr.is_null() {
            return "unknown".to_string();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

pub fn probe_path(path: impl AsRef<Path>) -> Result<MediaProbe> {
    let uri = path.as_ref().to_string_lossy().into_owned();
    probe_uri(&uri)
}

pub fn probe_uri(uri: &str) -> Result<MediaProbe> {
    Ok(Demuxer::open_uri(uri)?.probe().clone())
}

struct FormatContext {
    ptr: *mut sys::AVFormatContext,
    avio: Option<Box<CustomAvio>>,
}

impl FormatContext {
    fn new(ptr: *mut sys::AVFormatContext) -> Result<Self> {
        if ptr.is_null() {
            return Err(FfmpegError::NullPointer("avformat_open_input"));
        }
        Ok(Self { ptr, avio: None })
    }

    fn new_with_custom_io(ptr: *mut sys::AVFormatContext, avio: Box<CustomAvio>) -> Result<Self> {
        if ptr.is_null() {
            return Err(FfmpegError::NullPointer("avformat_open_input"));
        }
        Ok(Self {
            ptr,
            avio: Some(avio),
        })
    }

    fn as_ptr(&self) -> *const sys::AVFormatContext {
        self.ptr
    }

    fn as_mut_ptr(&mut self) -> *mut sys::AVFormatContext {
        self.ptr
    }
}

impl Drop for FormatContext {
    fn drop(&mut self) {
        unsafe { sys::avformat_close_input(&mut self.ptr) };
        let _ = self.avio.take();
    }
}

struct CustomAvio {
    context: *mut sys::AVIOContext,
    source: Box<dyn MediaSource>,
    offset: u64,
}

impl CustomAvio {
    const BUFFER_SIZE: usize = 64 * 1024;

    fn new(source: Box<dyn MediaSource>) -> Result<Box<Self>> {
        let buffer = unsafe { sys::av_malloc(Self::BUFFER_SIZE) }.cast::<u8>();
        if buffer.is_null() {
            return Err(FfmpegError::NullPointer("av_malloc(avio buffer)"));
        }

        let mut avio = Box::new(Self {
            context: ptr::null_mut(),
            source,
            offset: 0,
        });
        let context = unsafe {
            sys::avio_alloc_context(
                buffer,
                Self::BUFFER_SIZE as c_int,
                0,
                (&mut *avio) as *mut CustomAvio as *mut c_void,
                Some(custom_avio_read_packet),
                None,
                Some(custom_avio_seek),
            )
        };
        if context.is_null() {
            unsafe { sys::av_free(buffer.cast::<c_void>()) };
            return Err(FfmpegError::NullPointer("avio_alloc_context"));
        }
        avio.context = context;
        Ok(avio)
    }

    fn context(&self) -> *mut sys::AVIOContext {
        self.context
    }

    fn read_packet(&mut self, buffer: *mut u8, buffer_size: c_int) -> c_int {
        if buffer.is_null() || buffer_size <= 0 {
            return av_error(EINVAL);
        }
        let length = buffer_size as u64;
        match self.source.read_range(ByteRange {
            start: self.offset,
            length: Some(length),
        }) {
            Ok(bytes) if bytes.is_empty() => AVERROR_EOF,
            Ok(bytes) => {
                let copy_len = bytes.len().min(buffer_size as usize);
                unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, copy_len) };
                self.offset = self.offset.saturating_add(copy_len as u64);
                copy_len as c_int
            }
            Err(_) => av_error(EIO),
        }
    }

    fn seek(&mut self, offset: i64, whence: c_int) -> i64 {
        if whence == sys::AVSEEK_SIZE as c_int {
            return match self.source.len() {
                Ok(Some(length)) => length.min(i64::MAX as u64) as i64,
                Ok(None) => av_error(ESPIPE) as i64,
                Err(_) => av_error(EIO) as i64,
            };
        }

        let target = match whence {
            SEEK_SET => offset,
            SEEK_CUR => self.offset as i64 + offset,
            SEEK_END => match self.source.len() {
                Ok(Some(length)) => length.min(i64::MAX as u64) as i64 + offset,
                Ok(None) => return av_error(ESPIPE) as i64,
                Err(_) => return av_error(EIO) as i64,
            },
            _ => return av_error(EINVAL) as i64,
        };
        if target < 0 {
            return av_error(EINVAL) as i64;
        }
        self.offset = target as u64;
        self.offset.min(i64::MAX as u64) as i64
    }
}

impl Drop for CustomAvio {
    fn drop(&mut self) {
        if !self.context.is_null() {
            let buffer = unsafe { (*self.context).buffer };
            let mut context = self.context;
            unsafe { sys::avio_context_free(&mut context) };
            if !buffer.is_null() {
                unsafe { sys::av_free(buffer.cast::<c_void>()) };
            }
            self.context = ptr::null_mut();
        }
    }
}

unsafe extern "C" fn custom_avio_read_packet(
    opaque: *mut c_void,
    buffer: *mut u8,
    buffer_size: c_int,
) -> c_int {
    if opaque.is_null() {
        return av_error(EINVAL);
    }
    unsafe { (&mut *(opaque.cast::<CustomAvio>())).read_packet(buffer, buffer_size) }
}

unsafe extern "C" fn custom_avio_seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    if opaque.is_null() {
        return av_error(EINVAL) as i64;
    }
    unsafe { (&mut *(opaque.cast::<CustomAvio>())).seek(offset, whence) }
}

fn open_format_context(uri: &str) -> Result<FormatContext> {
    let input = CString::new(uri).map_err(|_| FfmpegError::InteriorNul)?;
    let mut format_context = ptr::null_mut();
    check(
        unsafe {
            sys::avformat_open_input(
                &mut format_context,
                input.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            )
        },
        "avformat_open_input",
    )?;
    FormatContext::new(format_context)
}

fn open_source_format_context(source: Box<dyn MediaSource>) -> Result<FormatContext> {
    let uri = CString::new(source.uri()).map_err(|_| FfmpegError::InteriorNul)?;
    let avio = CustomAvio::new(source)?;
    let format_context = unsafe { sys::avformat_alloc_context() };
    if format_context.is_null() {
        return Err(FfmpegError::NullPointer("avformat_alloc_context"));
    }
    unsafe {
        (*format_context).pb = avio.context();
        (*format_context).flags |= sys::AVFMT_FLAG_CUSTOM_IO as c_int;
    }

    let mut opened_context = format_context;
    match check(
        unsafe {
            sys::avformat_open_input(
                &mut opened_context,
                uri.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            )
        },
        "avformat_open_input(custom_io)",
    ) {
        Ok(()) => FormatContext::new_with_custom_io(opened_context, avio),
        Err(error) => {
            if !opened_context.is_null() {
                unsafe { sys::avformat_close_input(&mut opened_context) };
            }
            Err(error)
        }
    }
}

fn find_stream_info(context: &mut FormatContext) -> Result<()> {
    check(
        unsafe { sys::avformat_find_stream_info(context.as_mut_ptr(), ptr::null_mut()) },
        "avformat_find_stream_info",
    )
}

fn inspect_format_context(
    uri: &str,
    raw: *const sys::AVFormatContext,
) -> (MediaProbe, Vec<Option<TimeBase>>) {
    let duration = unsafe { duration_from_av((*raw).duration) };
    let stream_count = unsafe { (*raw).nb_streams as usize };
    let mut tracks = Vec::with_capacity(stream_count);
    let mut video = Vec::new();
    let mut audio = Vec::new();
    let mut stream_time_bases = Vec::with_capacity(stream_count);

    for index in 0..stream_count {
        let stream = unsafe { *(*raw).streams.add(index) };
        if stream.is_null() {
            stream_time_bases.push(None);
            continue;
        }

        stream_time_bases.push(Some(TimeBase::from_av(unsafe { (*stream).time_base })));

        let codecpar = unsafe { (*stream).codecpar };
        if codecpar.is_null() {
            continue;
        }

        let Some(kind) = (unsafe { track_kind((*codecpar).codec_type) }) else {
            continue;
        };

        let codec = unsafe { codec_name((*codecpar).codec_id) };
        let track = TrackInfo {
            id: unsafe { (*stream).index as i64 },
            kind,
            title: metadata_value(unsafe { (*stream).metadata }, "title"),
            language: metadata_value(unsafe { (*stream).metadata }, "language"),
            codec: codec.clone(),
        };

        if kind == TrackKind::Video {
            video.push(unsafe { video_probe(&track, codecpar) });
        }
        if kind == TrackKind::Audio {
            audio.push(unsafe { audio_probe(&track, codecpar) });
        }

        tracks.push(track);
    }

    (
        MediaProbe {
            uri: uri.to_string(),
            duration,
            tracks,
            video,
            audio,
        },
        stream_time_bases,
    )
}

fn check(code: i32, operation: &'static str) -> Result<()> {
    if code >= 0 {
        Ok(())
    } else {
        Err(FfmpegError::Api {
            operation,
            code,
            message: error_string(code),
        })
    }
}

fn error_string(code: i32) -> String {
    let mut buffer = [0i8; 256];
    unsafe {
        if sys::av_strerror(code, buffer.as_mut_ptr(), buffer.len()) == 0 {
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        } else {
            "unknown FFmpeg error".to_string()
        }
    }
}

unsafe fn duration_from_av(duration: i64) -> Option<Duration> {
    if duration <= 0 || duration == i64::MIN {
        return None;
    }
    let micros = duration as u64;
    Some(Duration::from_micros(micros))
}

unsafe fn track_kind(kind: sys::AVMediaType) -> Option<TrackKind> {
    match kind {
        sys::AVMediaType_AVMEDIA_TYPE_VIDEO => Some(TrackKind::Video),
        sys::AVMediaType_AVMEDIA_TYPE_AUDIO => Some(TrackKind::Audio),
        sys::AVMediaType_AVMEDIA_TYPE_SUBTITLE => Some(TrackKind::Subtitle),
        _ => None,
    }
}

unsafe fn codec_name(codec_id: sys::AVCodecID) -> Option<String> {
    let descriptor = unsafe { sys::avcodec_descriptor_get(codec_id) };
    if descriptor.is_null() {
        return None;
    }
    let name = unsafe { (*descriptor).name };
    if name.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe fn video_probe(track: &TrackInfo, codecpar: *const sys::AVCodecParameters) -> VideoProbe {
    let width = unsafe { (*codecpar).width.max(0) as u32 };
    let height = unsafe { (*codecpar).height.max(0) as u32 };
    let pixel_format = unsafe { pixel_format_name((*codecpar).format) };
    let codec = track.codec.clone();
    let profile = codec
        .as_deref()
        .and_then(|codec_name| unsafe { profile_name(codecpar, codec_name) });
    VideoProbe {
        track_id: track.id,
        params: VideoParams {
            width,
            height,
            primaries: unsafe { color_primaries((*codecpar).color_primaries) },
            transfer: unsafe { transfer_function((*codecpar).color_trc) },
        },
        codec,
        pixel_format,
        profile,
        level: Some(unsafe { (*codecpar).level }).filter(|level| *level > 0),
    }
}

unsafe fn audio_probe(track: &TrackInfo, codecpar: *const sys::AVCodecParameters) -> AudioProbe {
    AudioProbe {
        track_id: track.id,
        codec: track.codec.clone(),
        sample_rate: unsafe { (*codecpar).sample_rate.max(0) as u32 },
        channels: unsafe { (*codecpar).ch_layout.nb_channels.max(0) as u32 },
        sample_format: unsafe { sample_format_name((*codecpar).format) },
    }
}

unsafe fn pixel_format_name(format: i32) -> Option<String> {
    if format < 0 {
        return None;
    }
    let name = unsafe { sys::av_get_pix_fmt_name(format) };
    if name.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe fn sample_format_name(format: i32) -> Option<String> {
    if format < 0 {
        return None;
    }
    let name = unsafe { sys::av_get_sample_fmt_name(format) };
    if name.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe fn profile_name(
    codecpar: *const sys::AVCodecParameters,
    codec_name: &str,
) -> Option<String> {
    let codec_id = unsafe { (*codecpar).codec_id };
    let profile = unsafe { (*codecpar).profile };
    if profile == sys::FF_PROFILE_UNKNOWN {
        return None;
    }
    let name = unsafe { sys::av_get_profile_name(sys::avcodec_find_decoder(codec_id), profile) };
    if name.is_null() {
        return Some(format!("{codec_name}:{profile}"));
    }
    Some(
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe fn color_primaries(value: sys::AVColorPrimaries) -> ColorPrimaries {
    match value {
        sys::AVColorPrimaries_AVCOL_PRI_BT709 => ColorPrimaries::Bt709,
        sys::AVColorPrimaries_AVCOL_PRI_SMPTE432 => ColorPrimaries::DisplayP3,
        sys::AVColorPrimaries_AVCOL_PRI_BT2020 => ColorPrimaries::Bt2020,
        _ => ColorPrimaries::Unknown,
    }
}

unsafe fn transfer_function(value: sys::AVColorTransferCharacteristic) -> TransferFunction {
    match value {
        sys::AVColorTransferCharacteristic_AVCOL_TRC_IEC61966_2_1 => TransferFunction::Srgb,
        sys::AVColorTransferCharacteristic_AVCOL_TRC_BT709 => TransferFunction::Bt1886,
        sys::AVColorTransferCharacteristic_AVCOL_TRC_SMPTE2084 => TransferFunction::Pq,
        sys::AVColorTransferCharacteristic_AVCOL_TRC_ARIB_STD_B67 => TransferFunction::Hlg,
        _ => TransferFunction::Unknown,
    }
}

fn metadata_value(metadata: *mut sys::AVDictionary, key: &str) -> Option<String> {
    let key = CString::new(key).ok()?;
    unsafe {
        let entry = sys::av_dict_get(metadata, key.as_ptr(), ptr::null(), 0);
        if entry.is_null() || (*entry).value.is_null() {
            return None;
        }
        Some(
            CStr::from_ptr((*entry).value)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn timestamp_value(value: i64) -> Option<i64> {
    if value == i64::MIN { None } else { Some(value) }
}

fn av_error(errno: i32) -> i32 {
    -errno
}

fn hardware_pixel_format(
    codec: *const sys::AVCodec,
    device_type: sys::AVHWDeviceType,
) -> Option<sys::AVPixelFormat> {
    let mut index = 0;
    loop {
        let config = unsafe { sys::avcodec_get_hw_config(codec, index) };
        if config.is_null() {
            return None;
        }
        let supports_device_ctx = unsafe {
            (*config).device_type == device_type
                && ((*config).methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32) != 0
        };
        if supports_device_ctx {
            return Some(unsafe { (*config).pix_fmt });
        }
        index += 1;
    }
}

unsafe extern "C" fn select_hw_format(
    context: *mut sys::AVCodecContext,
    formats: *const sys::AVPixelFormat,
) -> sys::AVPixelFormat {
    let state = unsafe { (*context).opaque as *const HardwareDecoderState };
    if !state.is_null() {
        let target = unsafe { (*state).pixel_format };
        let mut index = 0usize;
        loop {
            let format = unsafe { *formats.add(index) };
            if format == sys::AVPixelFormat_AV_PIX_FMT_NONE {
                break;
            }
            if format == target {
                return format;
            }
            index += 1;
        }
    }
    unsafe { sys::avcodec_default_get_format(context, formats) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_ffmpeg_reports_version() {
        assert!(!version().is_empty());
    }

    #[test]
    fn time_base_converts_packet_timestamps() {
        let timestamp = PacketTimestamp {
            raw: 24,
            time_base: TimeBase { num: 1, den: 24 },
        };
        assert_eq!(timestamp.seconds(), 1.0);
        assert_eq!(timestamp.as_duration(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn stream_selection_accepts_expected_streams() {
        let selection = StreamSelection::only([0, 2]);
        assert!(selection.accepts(0));
        assert!(!selection.accepts(1));
        assert!(selection.accepts(2));
    }
}
