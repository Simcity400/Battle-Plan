use crate::{AudioConfig, Finished, VideoConfig, VideoSink, color::rgba_to_nv12_bt709};
use std::{
    ffi::c_void,
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use windows::Win32::{
    Foundation::{HMODULE, RPC_E_CHANGED_MODE, S_FALSE, S_OK},
    Media::MediaFoundation::{
        IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter, MF_API_VERSION,
        MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION,
        MF_MT_AAC_PAYLOAD_TYPE, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
        MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
        MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
        MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
        MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES,
        MF_MT_YUV_MATRIX, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SDK_VERSION,
        MF_SINK_WRITER_DISABLE_THROTTLING, MF_TRANSCODE_CONTAINERTYPE, MFAudioFormat_AAC,
        MFAudioFormat_PCM, MFMediaType_Audio, MFMediaType_Video, MFNominalRange_16_235,
        MFSTARTUP_LITE, MFTranscodeContainerType_MPEG4, MFVideoFormat_H264, MFVideoFormat_NV12,
        MFVideoInterlace_Progressive, MFVideoPrimaries_BT709, MFVideoTransFunc_709,
        MFVideoTransferMatrix_BT709,
    },
    System::{
        Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        LibraryLoader::{GetProcAddress, LoadLibraryW},
    },
};
use windows_core::{HRESULT, Interface, PCSTR, PCWSTR, s, w};

const H264_PROFILE_HIGH: u32 = 100;
const H264_PROFILE_MAIN: u32 = 77;
const MF_VERSION: u32 = (MF_SDK_VERSION << 16) | MF_API_VERSION;

type FnMfStartup = unsafe extern "system" fn(u32, u32) -> HRESULT;
type FnMfShutdown = unsafe extern "system" fn() -> HRESULT;
type FnMfCreateAttributes = unsafe extern "system" fn(*mut Option<IMFAttributes>, u32) -> HRESULT;
type FnMfCreateMediaType = unsafe extern "system" fn(*mut Option<IMFMediaType>) -> HRESULT;
type FnMfCreateMemoryBuffer = unsafe extern "system" fn(u32, *mut Option<IMFMediaBuffer>) -> HRESULT;
type FnMfCreateSample = unsafe extern "system" fn(*mut Option<IMFSample>) -> HRESULT;
type FnMfCreateSinkWriterFromUrl = unsafe extern "system" fn(
    PCWSTR,
    *mut c_void,
    *mut c_void,
    *mut Option<IMFSinkWriter>,
) -> HRESULT;

struct MfApi {
    _plat: HMODULE,
    _rw: HMODULE,
    startup: FnMfStartup,
    shutdown: FnMfShutdown,
    create_attributes: FnMfCreateAttributes,
    create_media_type: FnMfCreateMediaType,
    create_memory_buffer: FnMfCreateMemoryBuffer,
    create_sample: FnMfCreateSample,
    create_sink_writer_from_url: FnMfCreateSinkWriterFromUrl,
}

// SAFETY: HMODULE handles and resolved function pointers from loaded system DLLs are process-wide and immutable.
unsafe impl Send for MfApi {}
// SAFETY: HMODULE handles and resolved function pointers from loaded system DLLs are process-wide and immutable.
unsafe impl Sync for MfApi {}

fn load_symbol<T: Copy>(module: HMODULE, name: PCSTR) -> Result<T, String> {
    // SAFETY: module is a live HMODULE from LoadLibraryW and name is a NUL-terminated ASCII symbol name.
    let ptr = unsafe { GetProcAddress(module, name) }.ok_or_else(|| {
        format!(
            "Missing Media Foundation entry point {}",
            // SAFETY: name is a valid static NUL-terminated ASCII string.
            unsafe { name.to_string() }.unwrap_or_default()
        )
    })?;
    // SAFETY: Caller supplies the exact ABI function pointer type for the requested Media Foundation symbol.
    Ok(unsafe { std::mem::transmute_copy(&ptr) })
}

fn api() -> Result<&'static MfApi, String> {
    static API: OnceLock<Result<MfApi, String>> = OnceLock::new();
    API.get_or_init(|| {
        // SAFETY: Loading standard system DLLs dynamically avoids static import failure on Windows N editions.
        let plat = unsafe { LoadLibraryW(w!("mfplat.dll")) }.map_err(|_| {
            "Windows Media Foundation (mfplat.dll) is unavailable on this system. Windows N editions require the Media Feature Pack, or configure FFmpeg in Settings → Export encoding.".to_owned()
        })?;
        // SAFETY: Loading mfreadwrite.dll dynamically avoids static import failure on Windows N editions.
        let rw = unsafe { LoadLibraryW(w!("mfreadwrite.dll")) }.map_err(|_| {
            "Windows Media Foundation (mfreadwrite.dll) is unavailable on this system. Install the Media Feature Pack or configure FFmpeg in Settings → Export encoding.".to_owned()
        })?;
        Ok(MfApi {
            _plat: plat,
            _rw: rw,
            startup: load_symbol(plat, s!("MFStartup"))?,
            shutdown: load_symbol(plat, s!("MFShutdown"))?,
            create_attributes: load_symbol(plat, s!("MFCreateAttributes"))?,
            create_media_type: load_symbol(plat, s!("MFCreateMediaType"))?,
            create_memory_buffer: load_symbol(plat, s!("MFCreateMemoryBuffer"))?,
            create_sample: load_symbol(plat, s!("MFCreateSample"))?,
            create_sink_writer_from_url: load_symbol(rw, s!("MFCreateSinkWriterFromURL"))?,
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

struct SessionGuard {
    com_owned: bool,
    api: &'static MfApi,
}

impl SessionGuard {
    fn start(api: &'static MfApi) -> io::Result<Self> {
        // SAFETY: Initializing COM multithreaded apartment on the encoder thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let com_owned = if hr == S_OK || hr == S_FALSE {
            true
        } else if hr == RPC_E_CHANGED_MODE {
            false
        } else {
            return Err(io::Error::other(format!("COM initialization failed: {hr:?}")));
        };
        // SAFETY: api.startup is MFStartup loaded from mfplat.dll.
        let mf = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_LITE) };
        if mf.is_err() {
            if com_owned {
                // SAFETY: Matches successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
            return Err(io::Error::other(format!(
                "Windows Media Foundation startup failed: {mf:?}"
            )));
        }
        Ok(Self { com_owned, api })
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        // SAFETY: Pairs with successful MFStartup in SessionGuard::start.
        let _ = unsafe { (self.api.shutdown)() };
        if self.com_owned {
            // SAFETY: Pairs with successful CoInitializeEx in SessionGuard::start.
            unsafe { CoUninitialize() };
        }
    }
}

pub fn probe() -> Result<String, String> {
    let api = api()?;
    let _guard = SessionGuard::start(api).map_err(|error| error.to_string())?;
    Ok("Windows Media Foundation (H.264 / AAC)".to_owned())
}

fn pack_u64(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let abs = std::path::absolute(path)?;
    let mut wide: Vec<u16> = abs.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Output path contains NUL"));
    }
    wide.push(0);
    Ok(wide)
}

fn create_attributes(api: &MfApi, initial: u32) -> io::Result<IMFAttributes> {
    let mut out = None;
    // SAFETY: out is a valid pointer to Option<IMFAttributes>.
    let hr = unsafe { (api.create_attributes)(&raw mut out, initial) };
    hr.ok().map_err(io::Error::other)?;
    out.ok_or_else(|| io::Error::other("MFCreateAttributes returned null"))
}

fn create_media_type(api: &MfApi) -> io::Result<IMFMediaType> {
    let mut out = None;
    // SAFETY: out is a valid pointer to Option<IMFMediaType>.
    let hr = unsafe { (api.create_media_type)(&raw mut out) };
    hr.ok().map_err(io::Error::other)?;
    out.ok_or_else(|| io::Error::other("MFCreateMediaType returned null"))
}

fn create_sink_writer(
    api: &MfApi,
    path: &Path,
    hardware: bool,
) -> io::Result<IMFSinkWriter> {
    let attrs = create_attributes(api, 3)?;
    // SAFETY: Valid COM attribute setters on newly created IMFAttributes.
    unsafe {
        attrs
            .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
            .map_err(io::Error::other)?;
        attrs
            .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, u32::from(hardware))
            .map_err(io::Error::other)?;
        attrs
            .SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)
            .map_err(io::Error::other)?;
    }
    let wide = wide_path(path)?;
    let mut writer = None;
    // SAFETY: wide is NUL-terminated UTF-16 and attrs is a valid IMFAttributes COM pointer.
    let hr = unsafe {
        (api.create_sink_writer_from_url)(
            PCWSTR(wide.as_ptr()),
            std::ptr::null_mut(),
            attrs.as_raw(),
            &raw mut writer,
        )
    };
    hr.ok().map_err(io::Error::other)?;
    writer.ok_or_else(|| io::Error::other("MFCreateSinkWriterFromURL returned null"))
}

fn configure_writer(
    api: &MfApi,
    config: &VideoConfig,
    audio: Option<AudioConfig>,
    path: &Path,
    hardware: bool,
    profile: u32,
) -> io::Result<(IMFSinkWriter, u32, Option<u32>)> {
    let writer = create_sink_writer(api, path, hardware)
        .map_err(|e| io::Error::other(format!("MFCreateSinkWriterFromURL: {e}")))?;
    let out_video = create_media_type(api)?;
    let bitrate = config.quality.target_bitrate(config.width, config.height, config.fps);
    // SAFETY: Valid COM calls on out_video media type.
    unsafe {
        out_video.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(io::Error::other)?;
        out_video.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(io::Error::other)?;
        out_video.SetUINT32(&MF_MT_AVG_BITRATE, bitrate).map_err(io::Error::other)?;
        out_video
            .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(io::Error::other)?;
        out_video
            .SetUINT64(&MF_MT_FRAME_SIZE, pack_u64(config.width, config.height))
            .map_err(io::Error::other)?;
        out_video
            .SetUINT64(&MF_MT_FRAME_RATE, pack_u64(config.fps, 1))
            .map_err(io::Error::other)?;
        out_video
            .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u64(1, 1))
            .map_err(io::Error::other)?;
        out_video.SetUINT32(&MF_MT_MPEG2_PROFILE, profile).map_err(io::Error::other)?;
        out_video
            .SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)
            .map_err(io::Error::other)?;
        out_video
            .SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)
            .map_err(io::Error::other)?;
        out_video
            .SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)
            .map_err(io::Error::other)?;
        out_video
            .SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
            .map_err(io::Error::other)?;
    }
    // SAFETY: out_video is a valid configured IMFMediaType.
    let video_stream = unsafe { writer.AddStream(&out_video) }
        .map_err(|e| io::Error::other(format!("AddStream(video): {e}")))?;

    let in_video = create_media_type(api)?;
    // SAFETY: Valid COM calls on in_video media type.
    unsafe {
        in_video.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(io::Error::other)?;
        in_video.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(io::Error::other)?;
        in_video
            .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(io::Error::other)?;
        in_video
            .SetUINT64(&MF_MT_FRAME_SIZE, pack_u64(config.width, config.height))
            .map_err(io::Error::other)?;
        in_video
            .SetUINT64(&MF_MT_FRAME_RATE, pack_u64(config.fps, 1))
            .map_err(io::Error::other)?;
        in_video
            .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u64(1, 1))
            .map_err(io::Error::other)?;
        in_video
            .SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)
            .map_err(io::Error::other)?;
        in_video
            .SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)
            .map_err(io::Error::other)?;
        in_video
            .SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)
            .map_err(io::Error::other)?;
        in_video
            .SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
            .map_err(io::Error::other)?;
    }
    // SAFETY: in_video is a valid configured IMFMediaType for video_stream.
    unsafe { writer.SetInputMediaType(video_stream, &in_video, None) }
        .map_err(|e| io::Error::other(format!("SetInputMediaType(video): {e}")))?;

    let audio_stream = if let Some(audio) = audio {
        let out_audio = create_media_type(api)?;
        // SAFETY: Valid COM calls on out_audio media type.
        unsafe {
            out_audio.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(io::Error::other)?;
            out_audio.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC).map_err(io::Error::other)?;
            out_audio.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(io::Error::other)?;
            out_audio
                .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio.sample_rate)
                .map_err(io::Error::other)?;
            out_audio
                .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(audio.channels))
                .map_err(io::Error::other)?;
            out_audio
                .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, audio.bitrate / 8)
                .map_err(io::Error::other)?;
            out_audio.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0).map_err(io::Error::other)?;
            out_audio
                .SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)
                .map_err(io::Error::other)?;
        }
        // SAFETY: out_audio is a valid configured IMFMediaType.
        let idx = unsafe { writer.AddStream(&out_audio) }
            .map_err(|e| io::Error::other(format!("AddStream(audio): {e}")))?;

        let in_audio = create_media_type(api)?;
        let block_align = u32::from(audio.channels) * 2;
        // SAFETY: Valid COM calls on in_audio media type.
        unsafe {
            in_audio.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(io::Error::other)?;
            in_audio.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(io::Error::other)?;
            in_audio.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(io::Error::other)?;
            in_audio
                .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio.sample_rate)
                .map_err(io::Error::other)?;
            in_audio
                .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(audio.channels))
                .map_err(io::Error::other)?;
            in_audio.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align).map_err(io::Error::other)?;
            in_audio
                .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, audio.sample_rate * block_align)
                .map_err(io::Error::other)?;
        }
        // SAFETY: in_audio is a valid configured IMFMediaType for idx.
        unsafe { writer.SetInputMediaType(idx, &in_audio, None) }
            .map_err(|e| io::Error::other(format!("SetInputMediaType(audio): {e}")))?;
        Some(idx)
    } else {
        None
    };

    // SAFETY: All streams are configured on writer.
    unsafe { writer.BeginWriting() }
        .map_err(|e| io::Error::other(format!("BeginWriting: {e}")))?;
    Ok((writer, video_stream, audio_stream))
}

struct MfWorker {
    api: &'static MfApi,
    writer: Option<IMFSinkWriter>,
    config: VideoConfig,
    audio: Option<AudioConfig>,
    hardware: bool,
    video_stream: u32,
    audio_stream: Option<u32>,
    width: usize,
    height: usize,
    fps: u64,
    audio_rate: u64,
    nv12: Vec<u8>,
    staged_path: PathBuf,
    backend_label: String,
    frames: u64,
    audio_frames: u64,
    _session: SessionGuard,
}

impl MfWorker {
    fn open(
        config: VideoConfig,
        audio: Option<AudioConfig>,
        staged_path: PathBuf,
    ) -> io::Result<Self> {
        let api = api().map_err(io::Error::other)?;
        let session = SessionGuard::start(api)?;
        let attempts = [
            (true, H264_PROFILE_HIGH, "Windows Media Foundation (Hardware H.264 High)"),
            (true, H264_PROFILE_MAIN, "Windows Media Foundation (Hardware H.264 Main)"),
            (false, H264_PROFILE_HIGH, "Windows Media Foundation (Software H.264 High)"),
            (false, H264_PROFILE_MAIN, "Windows Media Foundation (Software H.264 Main)"),
        ];
        let mut last_error = None;
        for (hardware, profile, label) in attempts {
            let _ = fs::remove_file(&staged_path);
            match configure_writer(api, &config, audio, &staged_path, hardware, profile) {
                Ok((writer, video_stream, audio_stream)) => {
                    return Ok(Self {
                        api,
                        writer: Some(writer),
                        width: config.width as usize,
                        height: config.height as usize,
                        fps: u64::from(config.fps),
                        audio_rate: u64::from(audio.map_or(crate::AUDIO_SAMPLE_RATE, |a| a.sample_rate)),
                        config,
                        audio,
                        hardware,
                        video_stream,
                        audio_stream,
                        nv12: Vec::new(),
                        staged_path,
                        backend_label: label.to_owned(),
                        frames: 0,
                        audio_frames: 0,
                        _session: session,
                    });
                }
                Err(error) => {
                    last_error = Some(error);
                }
            }
        }
        let _ = fs::remove_file(&staged_path);
        let cause = last_error.map_or_else(|| "unknown error".to_owned(), |e| e.to_string());
        Err(io::Error::other(format!(
            "Media Foundation rejected {}x{} @ {} fps H.264 (hardware and software): {cause}",
            config.width, config.height, config.fps
        )))
    }

    fn fallback_to_software(&mut self) -> io::Result<()> {
        drop(self.writer.take());
        let _ = fs::remove_file(&self.staged_path);
        let attempts = [
            (H264_PROFILE_HIGH, "Windows Media Foundation (Software H.264 High)"),
            (H264_PROFILE_MAIN, "Windows Media Foundation (Software H.264 Main)"),
        ];
        let mut last_error = None;
        for (profile, label) in attempts {
            let _ = fs::remove_file(&self.staged_path);
            match configure_writer(self.api, &self.config, self.audio, &self.staged_path, false, profile) {
                Ok((writer, video_stream, audio_stream)) => {
                    self.writer = Some(writer);
                    self.video_stream = video_stream;
                    self.audio_stream = audio_stream;
                    self.hardware = false;
                    self.backend_label = label.to_owned();
                    return Ok(());
                }
                Err(error) => {
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| io::Error::other("Software H.264 fallback failed")))
    }

    fn write_sample(&self, stream: u32, kind: &str, data: &[u8], start_hns: i64, duration_hns: i64) -> io::Result<()> {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| io::Error::other("Media Foundation writer is already closed"))?;
        let mut buffer = None;
        let len = u32::try_from(data.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Sample exceeds 4 GiB"))?;
        // SAFETY: buffer is a valid out-pointer to Option<IMFMediaBuffer>.
        let hr = unsafe { (self.api.create_memory_buffer)(len, &raw mut buffer) };
        hr.ok().map_err(io::Error::other)?;
        let buffer = buffer.ok_or_else(|| io::Error::other("MFCreateMemoryBuffer returned null"))?;

        let mut dst = std::ptr::null_mut();
        // SAFETY: Locking newly created memory buffer of size len bytes, copying data, then unlocking.
        unsafe {
            buffer.Lock(&raw mut dst, None, None).map_err(io::Error::other)?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
            buffer.Unlock().map_err(io::Error::other)?;
            buffer.SetCurrentLength(len).map_err(io::Error::other)?;
        }

        let mut sample = None;
        // SAFETY: sample is a valid out-pointer to Option<IMFSample>.
        let hr = unsafe { (self.api.create_sample)(&raw mut sample) };
        hr.ok().map_err(io::Error::other)?;
        let sample = sample.ok_or_else(|| io::Error::other("MFCreateSample returned null"))?;
        // SAFETY: Attaching buffer, setting HNS timestamps, and writing sample to stream.
        unsafe {
            sample.AddBuffer(&buffer).map_err(io::Error::other)?;
            sample.SetSampleTime(start_hns).map_err(io::Error::other)?;
            sample.SetSampleDuration(duration_hns).map_err(io::Error::other)?;
            writer
                .WriteSample(stream, &sample)
                .map_err(|e| io::Error::other(format!("WriteSample({kind}): {e}")))?;
        }
        Ok(())
    }

    fn push_video(&mut self, rgba: &[u8]) -> io::Result<Option<String>> {
        rgba_to_nv12_bt709(rgba, self.width, self.height, &mut self.nv12);
        let start_hns = ((self.frames * 10_000_000) / self.fps) as i64;
        let next_hns = (((self.frames + 1) * 10_000_000) / self.fps) as i64;
        let duration_hns = (next_hns - start_hns).max(1);
        let mut updated_label = None;
        if let Err(err) = self.write_sample(self.video_stream, "video", &self.nv12, start_hns, duration_hns) {
            if self.hardware && self.frames == 0 && self.audio_frames == 0 {
                self.fallback_to_software()?;
                self.write_sample(self.video_stream, "video-sw", &self.nv12, start_hns, duration_hns)?;
                updated_label = Some(self.backend_label.clone());
            } else {
                return Err(err);
            }
        }
        self.frames += 1;
        Ok(updated_label)
    }

    fn push_audio(&mut self, pcm: &[i16]) -> io::Result<()> {
        let Some(stream) = self.audio_stream else {
            return Ok(());
        };
        if pcm.is_empty() {
            return Ok(());
        }
        let sample_frames = (pcm.len() / 2) as u64;
        let start_hns = ((self.audio_frames * 10_000_000) / self.audio_rate) as i64;
        let next_hns = (((self.audio_frames + sample_frames) * 10_000_000) / self.audio_rate) as i64;
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &sample in pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.write_sample(stream, "audio", &bytes, start_hns, (next_hns - start_hns).max(1))?;
        self.audio_frames += sample_frames;
        Ok(())
    }

    fn finish(mut self) -> io::Result<Finished> {
        let writer = self
            .writer
            .take()
            .ok_or_else(|| io::Error::other("Media Foundation writer was already finalized"))?;
        // SAFETY: Finalizing IMFSinkWriter flushes pending samples and writes the MP4 moov box.
        let finalized = unsafe { writer.Finalize() }
            .map_err(|e| io::Error::other(format!("Finalize: {e}")));
        drop(writer);
        if let Err(error) = finalized {
            let _ = fs::remove_file(&self.staged_path);
            return Err(error);
        }
        if let Err(error) = crate::mp4_boxes::relocate_moov_before_mdat(&self.staged_path) {
            let _ = fs::remove_file(&self.staged_path);
            return Err(error);
        }
        Ok(Finished {
            path: self.staged_path.clone(),
            frames: self.frames,
            audio_frames: self.audio_frames,
            backend: self.backend_label.clone(),
        })
    }

    fn abort(mut self) {
        drop(self.writer.take());
        let _ = fs::remove_file(&self.staged_path);
    }
}

enum MfCommand {
    PushVideo {
        rgba: Vec<u8>,
        reply: std::sync::mpsc::SyncSender<io::Result<Option<String>>>,
    },
    PushAudio {
        pcm: Vec<i16>,
        reply: std::sync::mpsc::SyncSender<io::Result<()>>,
    },
    Finish {
        reply: std::sync::mpsc::SyncSender<io::Result<Finished>>,
    },
    Abort,
}

pub struct MfSink {
    tx: Option<std::sync::mpsc::SyncSender<MfCommand>>,
    handle: Option<std::thread::JoinHandle<()>>,
    frame_bytes: usize,
    staged_path: PathBuf,
    backend_label: String,
    frames: u64,
    audio_frames: u64,
    finished: bool,
}

impl MfSink {
    pub fn open(
        config: &VideoConfig,
        audio: Option<AudioConfig>,
        staged_path: &Path,
    ) -> io::Result<Self> {
        let frame_bytes = config.rgba_frame_bytes()?;
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<io::Result<String>>(1);
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<MfCommand>(2);
        let worker_cfg = config.clone();
        let worker_path = staged_path.to_path_buf();
        let handle = std::thread::Builder::new()
            .name("mf-encoder".to_owned())
            .spawn(move || {
                let mut worker = match MfWorker::open(worker_cfg, audio, worker_path) {
                    Ok(worker) => {
                        let _ = init_tx.send(Ok(worker.backend_label.clone()));
                        worker
                    }
                    Err(error) => {
                        let _ = init_tx.send(Err(error));
                        return;
                    }
                };
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        MfCommand::PushVideo { rgba, reply } => {
                            let _ = reply.send(worker.push_video(&rgba));
                        }
                        MfCommand::PushAudio { pcm, reply } => {
                            let _ = reply.send(worker.push_audio(&pcm));
                        }
                        MfCommand::Finish { reply } => {
                            let _ = reply.send(worker.finish());
                            return;
                        }
                        MfCommand::Abort => {
                            worker.abort();
                            return;
                        }
                    }
                }
                worker.abort();
            })
            .map_err(|e| io::Error::other(format!("Couldn’t start Media Foundation thread: {e}")))?;

        let backend_label = match init_rx.recv() {
            Ok(Ok(label)) => label,
            Ok(Err(error)) => {
                let _ = handle.join();
                return Err(error);
            }
            Err(_) => {
                let _ = handle.join();
                return Err(io::Error::other("Media Foundation thread stopped during initialization"));
            }
        };

        Ok(Self {
            tx: Some(cmd_tx),
            handle: Some(handle),
            frame_bytes,
            staged_path: staged_path.to_path_buf(),
            backend_label,
            frames: 0,
            audio_frames: 0,
            finished: false,
        })
    }

    fn abort_inner(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(MfCommand::Abort);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let _ = fs::remove_file(&self.staged_path);
    }
}

impl Drop for MfSink {
    fn drop(&mut self) {
        self.abort_inner();
    }
}

impl VideoSink for MfSink {
    fn push_video_rgba(&mut self, rgba: &[u8]) -> io::Result<()> {
        if rgba.len() != self.frame_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expected {} RGBA bytes, got {}", self.frame_bytes, rgba.len()),
            ));
        }
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| io::Error::other("Media Foundation writer is already closed"))?;
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        tx.send(MfCommand::PushVideo {
            rgba: rgba.to_vec(),
            reply: reply_tx,
        })
        .map_err(|_| io::Error::other("Media Foundation thread stopped unexpectedly"))?;
        let updated = reply_rx
            .recv()
            .map_err(|_| io::Error::other("Media Foundation thread stopped unexpectedly"))??;
        if let Some(label) = updated {
            self.backend_label = label;
        }
        self.frames += 1;
        Ok(())
    }

    fn push_audio_stereo_i16(&mut self, pcm: &[i16]) -> io::Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        if pcm.len() % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Stereo PCM chunk must hold an even number of samples",
            ));
        }
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| io::Error::other("Media Foundation writer is already closed"))?;
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        tx.send(MfCommand::PushAudio {
            pcm: pcm.to_vec(),
            reply: reply_tx,
        })
        .map_err(|_| io::Error::other("Media Foundation thread stopped unexpectedly"))?;
        reply_rx
            .recv()
            .map_err(|_| io::Error::other("Media Foundation thread stopped unexpectedly"))??;
        self.audio_frames += (pcm.len() / 2) as u64;
        Ok(())
    }

    fn frames_written(&self) -> u64 {
        self.frames
    }

    fn audio_frames_written(&self) -> u64 {
        self.audio_frames
    }

    fn backend_name(&self) -> &str {
        &self.backend_label
    }

    fn finish(mut self: Box<Self>) -> io::Result<Finished> {
        self.finished = true;
        let tx = self
            .tx
            .take()
            .ok_or_else(|| io::Error::other("Media Foundation writer was already finalized"))?;
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        tx.send(MfCommand::Finish { reply: reply_tx })
            .map_err(|_| io::Error::other("Media Foundation thread stopped unexpectedly"))?;
        let result = reply_rx
            .recv()
            .map_err(|_| io::Error::other("Media Foundation thread stopped unexpectedly"))?;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        result
    }

    fn abort(mut self: Box<Self>) {
        self.abort_inner();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BackendChoice, Quality, mp4_boxes::inspect_mp4, schedule::audio_frames_for_index};

    #[test]
    fn encodes_synthetic_h264_and_aac_mp4_with_faststart_moov_before_mdat() {
        if probe().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("bp-mf-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let staged = dir.join("clip.mp4.tmp");
        let config = VideoConfig {
            width: 320,
            height: 180,
            fps: 30,
            quality: Quality::Standard,
            backend: BackendChoice::MediaFoundation,
            ffmpeg_path: None,
        };
        let mut sink = MfSink::open(&config, Some(AudioConfig::default()), &staged).unwrap();
        let mut rgba = vec![0u8; config.rgba_frame_bytes().unwrap()];
        for i in 0..30u64 {
            for pixel in rgba.chunks_exact_mut(4) {
                pixel[0] = ((i * 8) % 255) as u8;
                pixel[1] = 120;
                pixel[2] = 200;
                pixel[3] = 255;
            }
            sink.push_video_rgba(&rgba).unwrap();
            let audio_n = audio_frames_for_index(i, 30);
            let mut pcm = Vec::with_capacity(audio_n * 2);
            for s in 0..audio_n {
                let sample = (((i as usize * audio_n + s) as f32 * 0.05).sin() * 4000.0) as i16;
                pcm.push(sample);
                pcm.push(sample);
            }
            sink.push_audio_stereo_i16(&pcm).unwrap();
        }
        let finished = Box::new(sink).finish().unwrap();
        assert_eq!(finished.frames, 30);
        assert_eq!(finished.audio_frames, 44_100);
        let summary = inspect_mp4(&finished.path).unwrap();
        assert!(summary.moov_before_mdat, "expected moov before mdat, boxes={:?}", summary.boxes);
        assert!(summary.has_avc1, "expected avc1 video track");
        assert!(summary.has_mp4a, "expected mp4a audio track");
        let duration = summary.duration_ms.unwrap_or(0);
        assert!((900..=1200).contains(&duration), "unexpected duration_ms={duration}");
        let _ = fs::remove_dir_all(&dir);
    }
}
