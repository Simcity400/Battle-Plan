use crate::{AudioConfig, Finished, VideoConfig, VideoSink};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    thread::{self, JoinHandle},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegInfo {
    pub path: PathBuf,
    pub version: String,
    pub encoder: String,
    pub has_aac: bool,
}

const PREFERRED_ENCODERS: [&str; 6] = [
    "libx264",
    "h264_nvenc",
    "h264_qsv",
    "h264_amf",
    "h264_vaapi",
    "libopenh264",
];

pub fn locate(explicit: Option<&Path>) -> io::Result<PathBuf> {
    if let Some(path) = explicit
        && !path.as_os_str().is_empty()
    {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Configured FFmpeg executable not found: {}", path.display()),
        ));
    }
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(paths) = std::env::var_os("PATH") {
        for folder in std::env::split_paths(&paths) {
            let candidate = folder.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "FFmpeg was not found on PATH. Install FFmpeg or choose its path in Settings → Export encoding.",
    ))
}

pub fn probe(explicit: Option<&Path>) -> io::Result<FfmpegInfo> {
    let path = locate(explicit)?;
    let version_out = hidden_command(&path).arg("-version").output()?;
    if !version_out.status.success() {
        return Err(io::Error::other("FFmpeg did not answer -version"));
    }
    let version = String::from_utf8_lossy(&version_out.stdout)
        .lines()
        .next()
        .unwrap_or("ffmpeg")
        .trim()
        .to_owned();
    let enc_out = hidden_command(&path).args(["-hide_banner", "-encoders"]).output()?;
    if !enc_out.status.success() {
        return Err(io::Error::other("FFmpeg did not list its encoders"));
    }
    let text = String::from_utf8_lossy(&enc_out.stdout);
    let encoder = PREFERRED_ENCODERS
        .iter()
        .find(|&&name| has_encoder(&text, name))
        .copied()
        .ok_or_else(|| {
            io::Error::other(
                "This FFmpeg build has no supported H.264 encoder (libx264, h264_nvenc, h264_qsv, h264_amf, h264_vaapi, or libopenh264)",
            )
        })?
        .to_owned();
    let has_aac = has_encoder(&text, "aac");
    Ok(FfmpegInfo {
        path,
        version,
        encoder,
        has_aac,
    })
}

fn has_encoder(output: &str, name: &str) -> bool {
    output.lines().any(|line| {
        let mut parts = line.split_whitespace();
        let Some(flags) = parts.next() else {
            return false;
        };
        flags.len() == 6 && parts.next() == Some(name)
    })
}

fn hidden_command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(windows)]
struct JobGuard(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
// SAFETY: A Windows Job Object HANDLE is an opaque OS kernel handle safe to transfer across threads.
unsafe impl Send for JobGuard {}

#[cfg(windows)]
impl JobGuard {
    fn attach(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
        };
        // SAFETY: CreateJobObjectW with null security attributes and name creates an anonymous job object.
        let job = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::other)?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: info points to a properly sized JOBOBJECT_EXTENDED_LIMIT_INFORMATION struct.
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if let Err(error) = configured {
            // SAFETY: job is a valid handle returned by CreateJobObjectW.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
            return Err(io::Error::other(error));
        }
        let process = HANDLE(child.as_raw_handle());
        // SAFETY: job and process are live handles for the child we just spawned.
        if let Err(error) = unsafe { AssignProcessToJobObject(job, process) } {
            // SAFETY: job is a valid handle returned by CreateJobObjectW.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
            return Err(io::Error::other(error));
        }
        Ok(Self(job))
    }
}

#[cfg(windows)]
impl Drop for JobGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 is owned exclusively by this JobGuard and closed once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

pub struct FfmpegSink {
    ffmpeg_path: PathBuf,
    encoder_name: String,
    frame_bytes: usize,
    staged_final: PathBuf,
    video_output: PathBuf,
    pcm_path: Option<PathBuf>,
    pcm_writer: Option<BufWriter<File>>,
    audio: Option<AudioConfig>,
    child: Child,
    #[cfg(windows)]
    _job: Option<JobGuard>,
    stdin: Option<ChildStdin>,
    stderr_thread: Option<JoinHandle<String>>,
    frames: u64,
    audio_frames: u64,
    finished: bool,
}

impl FfmpegSink {
    pub fn open(
        config: &VideoConfig,
        audio: Option<AudioConfig>,
        staged_path: &Path,
    ) -> io::Result<Self> {
        let info = probe(config.ffmpeg_path.as_deref())?;
        if audio.is_some() && !info.has_aac {
            return Err(io::Error::other(
                "This FFmpeg build has no AAC audio encoder; disable map audio or install an FFmpeg build with AAC",
            ));
        }
        let frame_bytes = config.rgba_frame_bytes()?;
        let (video_output, pcm_path, pcm_writer) = if audio.is_some() {
            let mut v = staged_path.as_os_str().to_owned();
            v.push(".video.mp4");
            let mut a = staged_path.as_os_str().to_owned();
            a.push(".audio.pcm");
            let pcm_path = PathBuf::from(a);
            let file = File::create(&pcm_path)?;
            (PathBuf::from(v), Some(pcm_path), Some(BufWriter::new(file)))
        } else {
            (staged_path.to_path_buf(), None, None)
        };

        let mut cmd = hidden_command(&info.path);
        cmd.args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s:v",
            &format!("{}x{}", config.width, config.height),
            "-r",
            &config.fps.to_string(),
            "-i",
            "pipe:0",
            "-vf",
            "scale=in_color_matrix=bt709:out_color_matrix=bt709:out_range=tv,format=yuv420p",
            "-c:v",
            &info.encoder,
        ]);
        match info.encoder.as_str() {
            "libx264" => {
                cmd.args([
                    "-preset",
                    "medium",
                    "-crf",
                    &config.quality.ffmpeg_crf().to_string(),
                ]);
            }
            "h264_nvenc" | "h264_qsv" | "h264_amf" | "h264_vaapi" | "libopenh264" => {
                let bitrate = config.quality.target_bitrate(config.width, config.height, config.fps);
                cmd.args(["-b:v", &bitrate.to_string()]);
            }
            _ => {}
        }
        cmd.args([
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
            "-f",
            "mp4",
            "-movflags",
            "+faststart",
        ])
        .arg(&video_output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|error| {
            if let Some(pcm) = &pcm_path {
                let _ = fs::remove_file(pcm);
            }
            io::Error::other(format!("Failed to start FFmpeg: {error}"))
        })?;
        #[cfg(windows)]
        let job = JobGuard::attach(&child).ok();
        let stdin = child.stdin.take();
        let stderr_thread = child.stderr.take().map(drain_stderr);

        Ok(Self {
            ffmpeg_path: info.path,
            encoder_name: format!("FFmpeg ({})", info.encoder),
            frame_bytes,
            staged_final: staged_path.to_path_buf(),
            video_output,
            pcm_path,
            pcm_writer,
            audio,
            child,
            #[cfg(windows)]
            _job: job,
            stdin,
            stderr_thread,
            frames: 0,
            audio_frames: 0,
            finished: false,
        })
    }

    fn cleanup_temps(&self) {
        if let Some(pcm) = &self.pcm_path {
            let _ = fs::remove_file(pcm);
        }
        if self.video_output != self.staged_final {
            let _ = fs::remove_file(&self.video_output);
        }
    }

    fn abort_inner(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        drop(self.stdin.take());
        drop(self.pcm_writer.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
        self.cleanup_temps();
        let _ = fs::remove_file(&self.staged_final);
    }
}

impl Drop for FfmpegSink {
    fn drop(&mut self) {
        self.abort_inner();
    }
}

fn drain_stderr(mut stderr: std::process::ChildStderr) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        let text = String::from_utf8_lossy(&buf).trim().to_owned();
        if text.len() > 4096 {
            text[text.len() - 4096..].to_owned()
        } else {
            text
        }
    })
}

impl VideoSink for FfmpegSink {
    fn push_video_rgba(&mut self, rgba: &[u8]) -> io::Result<()> {
        if rgba.len() != self.frame_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expected {} RGBA bytes, got {}", self.frame_bytes, rgba.len()),
            ));
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "FFmpeg input pipe is closed"))?;
        stdin.write_all(rgba)?;
        self.frames += 1;
        Ok(())
    }

    fn push_audio_stereo_i16(&mut self, pcm: &[i16]) -> io::Result<()> {
        let Some(writer) = self.pcm_writer.as_mut() else {
            return Ok(());
        };
        if pcm.len() % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Stereo PCM chunk must hold an even number of samples",
            ));
        }
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &sample in pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        writer.write_all(&bytes)?;
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
        &self.encoder_name
    }

    fn finish(mut self: Box<Self>) -> io::Result<Finished> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        let stderr = self
            .stderr_thread
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        if !status.success() {
            self.cleanup_temps();
            let _ = fs::remove_file(&self.staged_final);
            return Err(io::Error::other(if stderr.is_empty() {
                format!("FFmpeg exited with status {status}")
            } else {
                format!("FFmpeg failed: {stderr}")
            }));
        }
        if let Some(mut writer) = self.pcm_writer.take() {
            writer.flush()?;
            drop(writer);
            let pcm_path = self.pcm_path.as_ref().expect("pcm path set with writer");
            let audio = self.audio.unwrap_or_default();
            let mut remux = hidden_command(&self.ffmpeg_path);
            remux
                .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
                .arg(&self.video_output)
                .args([
                    "-f",
                    "s16le",
                    "-ar",
                    &audio.sample_rate.to_string(),
                    "-ac",
                    &audio.channels.to_string(),
                    "-i",
                ])
                .arg(pcm_path)
                .args([
                    "-c:v",
                    "copy",
                    "-c:a",
                    "aac",
                    "-b:a",
                    &audio.bitrate.to_string(),
                    "-shortest",
                    "-f",
                    "mp4",
                    "-movflags",
                    "+faststart",
                ])
                .arg(&self.staged_final)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            let mut remux_child = remux.spawn()?;
            #[cfg(windows)]
            let _remux_job = JobGuard::attach(&remux_child).ok();
            let remux_stderr = remux_child.stderr.take().map(drain_stderr);
            let remux_status = remux_child.wait()?;
            let remux_err = remux_stderr
                .and_then(|handle| handle.join().ok())
                .unwrap_or_default();
            self.cleanup_temps();
            if !remux_status.success() {
                let _ = fs::remove_file(&self.staged_final);
                return Err(io::Error::other(if remux_err.is_empty() {
                    format!("FFmpeg audio remux exited with status {remux_status}")
                } else {
                    format!("FFmpeg audio remux failed: {remux_err}")
                }));
            }
        }
        self.finished = true;
        Ok(Finished {
            path: self.staged_final.clone(),
            frames: self.frames,
            audio_frames: self.audio_frames,
            backend: self.encoder_name.clone(),
        })
    }

    fn abort(mut self: Box<Self>) {
        self.abort_inner();
    }
}
