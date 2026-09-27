//! Framed transport and owned process lifetime for the NeMo helper.
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub const PROTOCOL_VERSION: u64 = 1;
pub const RUNTIME_REVISION: &str = "97a15afa5caa9bce5baaa86c1184103877af4101";
pub const MAX_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_PCM_SAMPLES: usize = 16000;

#[derive(Debug, Clone)]
pub struct Frame {
    pub metadata: Value,
    pub pcm: Vec<f32>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub fn read_frame(reader: &mut impl Read) -> io::Result<Frame> {
    let mut header = [0; 8];
    reader.read_exact(&mut header)?;
    let json_len = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    let pcm_len = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    if json_len == 0
        || json_len > MAX_JSON_BYTES
        || pcm_len > MAX_PCM_SAMPLES * 4
        || !pcm_len.is_multiple_of(4)
    {
        return Err(invalid("invalid frame size"));
    }
    let mut json = vec![0; json_len];
    reader.read_exact(&mut json)?;
    let metadata = serde_json::from_slice(&json).map_err(io::Error::other)?;
    let mut bytes = vec![0; pcm_len];
    reader.read_exact(&mut bytes)?;
    let pcm: Vec<_> = bytes
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    if pcm.iter().any(|v| !v.is_finite()) {
        return Err(invalid("non-finite PCM"));
    }
    Ok(Frame { metadata, pcm })
}
pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let json = serde_json::to_vec(&frame.metadata).map_err(io::Error::other)?;
    if json.len() > MAX_JSON_BYTES
        || frame.pcm.len() > MAX_PCM_SAMPLES
        || frame.pcm.iter().any(|v| !v.is_finite())
    {
        return Err(invalid("invalid frame size or PCM"));
    }
    writer.write_all(&(json.len() as u32).to_le_bytes())?;
    writer.write_all(&((frame.pcm.len() * 4) as u32).to_le_bytes())?;
    writer.write_all(&json)?;
    for sample in &frame.pcm {
        writer.write_all(&sample.to_le_bytes())?;
    }
    writer.flush()
}

/// A single generation and stream epoch. Output must be consumed continuously.
/// A full output queue terminates the helper. It never discards result frames.
pub struct Helper {
    child: Child,
    input: Option<mpsc::SyncSender<Frame>>,
    output: mpsc::Receiver<io::Result<Frame>>,
    stopped: Arc<AtomicBool>,
    diagnostics: Arc<Mutex<Vec<u8>>>,
    threads: Vec<thread::JoinHandle<()>>,
    generation: u64,
    epoch: u64,
    next_sequence: u64,
    last_response_sequence: Option<u64>,
    #[cfg(windows)]
    _job: WindowsJob,
}
impl Helper {
    pub fn spawn(path: &Path, generation: u64, epoch: u64) -> io::Result<Self> {
        let mut command = Command::new(path);
        command
            .arg("--parent")
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        #[cfg(windows)]
        let job = match WindowsJob::assign(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let (input, incoming) = mpsc::sync_channel::<Frame>(4);
        let (outgoing, output) = mpsc::sync_channel(32);
        let stop = stopped.clone();
        let writer = thread::spawn(move || {
            while let Ok(frame) = incoming.recv() {
                if stop.load(Ordering::Acquire) || write_frame(&mut stdin, &frame).is_err() {
                    break;
                }
            }
        });
        let stop = stopped.clone();
        let reader = thread::spawn(move || {
            loop {
                let frame = read_frame(&mut stdout);
                let failed = frame.is_err();
                if outgoing.try_send(frame).is_err() {
                    stop.store(true, Ordering::Release);
                    break;
                }
                if failed {
                    break;
                }
            }
        });
        let diagnostic_buffer = diagnostics.clone();
        let diagnostic_reader = thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let mut tail = diagnostic_buffer.lock().unwrap();
                tail.extend_from_slice(&buffer[..count]);
                if tail.len() > 16384 {
                    let remove = tail.len() - 16384;
                    tail.drain(..remove);
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            output,
            stopped,
            diagnostics,
            threads: vec![writer, reader, diagnostic_reader],
            generation,
            epoch,
            next_sequence: 0,
            last_response_sequence: None,
            #[cfg(windows)]
            _job: job,
        })
    }
    /// Queues one request without blocking the recording thread.
    pub fn send(&mut self, mut metadata: Value, pcm: Vec<f32>) -> io::Result<u64> {
        if self.stopped.load(Ordering::Acquire) {
            self.cancel()?;
            return Err(invalid("helper output queue overflow"));
        }
        if !metadata.is_object() {
            return Err(invalid("metadata must be an object"));
        }
        metadata["version"] = PROTOCOL_VERSION.into();
        metadata["generation"] = self.generation.into();
        metadata["epoch"] = self.epoch.into();
        metadata["seq"] = self.next_sequence.into();
        // Validate before queueing so an invalid request cannot terminate the writer.
        write_frame(
            &mut io::sink(),
            &Frame {
                metadata: metadata.clone(),
                pcm: pcm.clone(),
            },
        )?;
        self.input
            .as_ref()
            .ok_or_else(|| invalid("helper stopped"))?
            .try_send(Frame { metadata, pcm })
            .map_err(|e| io::Error::new(io::ErrorKind::WouldBlock, e.to_string()))?;
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        Ok(sequence)
    }
    pub fn receive(&mut self, timeout: Duration) -> io::Result<Frame> {
        if self.stopped.load(Ordering::Acquire) {
            self.cancel()?;
            return Err(invalid("helper output queue overflow"));
        }
        let frame = self
            .output
            .recv_timeout(timeout)
            .map_err(|e| io::Error::new(io::ErrorKind::TimedOut, e.to_string()))??;
        let m = &frame.metadata;
        let sequence = m["seq"].as_u64();
        if sequence.is_none_or(|sequence| {
            sequence >= self.next_sequence
                || self
                    .last_response_sequence
                    .is_some_and(|last| sequence < last)
        }) {
            self.cancel()?;
            return Err(invalid("helper response sequence mismatch"));
        }
        self.last_response_sequence = sequence;
        if m["version"].as_u64() != Some(PROTOCOL_VERSION)
            || m["generation"].as_u64() != Some(self.generation)
            || m["epoch"].as_u64() != Some(self.epoch)
            || !frame.pcm.is_empty()
        {
            self.cancel()?;
            return Err(invalid("helper protocol or generation mismatch"));
        }
        if m["event"] == "hello" && m["runtime_revision"] != RUNTIME_REVISION {
            self.cancel()?;
            return Err(invalid("helper runtime revision mismatch"));
        }
        Ok(frame)
    }
    pub fn diagnostics(&self) -> Vec<u8> {
        self.diagnostics.lock().unwrap().clone()
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn cancel(&mut self) -> io::Result<()> {
        self.stopped.store(true, Ordering::Release);
        self.input.take();
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        self.child.wait()?;
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
        Ok(())
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

#[cfg(windows)]
struct WindowsJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl WindowsJob {
    fn assign(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = Self(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
            if job.0.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(job.0, child.as_raw_handle()) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

// A job handle has no thread affinity. WindowsJob owns its only closing handle.
#[cfg(windows)]
unsafe impl Send for WindowsJob {}
