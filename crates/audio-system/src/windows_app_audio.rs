//! The Windows app audio backend (KTD4, KTD5, R6, R7, R17, R19).
//!
//! - One per-process loopback client for each root process of each running
//!   app in the note's app set ([`ProcessTable::roots`]). The client
//!   records the process tree of the root. [`BlockMixer`] adds the clients
//!   together in 10 ms blocks.
//! - The thread finds the processes again every [`SCAN_INTERVAL`], when
//!   the app set changes, and when an output device gets a new audio
//!   session. So an app that starts or quits during the note needs no
//!   restart (R7).
//! - When Teams is in the app set, the thread reads the peak meter of each
//!   Teams render session every [`TEAMS_METER_INTERVAL`]. While Teams
//!   plays, [`TeamsMode`] selects device loopback on the output device of
//!   that session, in place of the per-process clients (R19). Windows gives
//!   only silence to a per-process client of Teams.
//! - A device loopback that does not open, or that fails and does not open
//!   again, ends the stream with [`Error::SpeakerStreamEnded`].
//!
//! [`render_sessions`] and [`parent_chain`] are public for the hardware
//! measurement test (U1).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use ::windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use ::windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, AudioSessionStateInactive,
    DEVICE_STATE_ACTIVE, IAudioSessionControl, IAudioSessionControl2, IAudioSessionManager2,
    IAudioSessionNotification, IAudioSessionNotification_Impl, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, eRender,
};
use ::windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use ::windows::core::{Interface, Ref, implement};
use tokio::sync::mpsc;
use wasapi::{
    AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle, SampleType, StreamMode,
    WaveFormat, initialize_mta,
};

use crate::apps::WINDOWS_TEAMS_ID;
use crate::convert::{Converter, SampleKind, downmix};
use crate::follow::{
    CaptureMode, ENDPOINT_POLL_INTERVAL, TEAMS_METER_INTERVAL, TeamsMode, TeamsReading,
};
use crate::rate::RateFollower;
use crate::windows::{LoopbackCapture, drain_packets, failed, open_loopback_capture};
use crate::windows_apps::process_table;
use crate::windows_mix::{BlockMixer, MIX_RATE};
use crate::windows_tree::ProcessTable;
use crate::{AppSet, CaptureFrame, Error, speaker_frame};

/// Time between two scans of the running processes.
const SCAN_INTERVAL: Duration = ENDPOINT_POLL_INTERVAL;

/// Time between two reads of the clients.
const READ_INTERVAL: Duration = Duration::from_millis(5);

/// Each per-process client records in stereo; the thread mixes it down.
const CLIENT_CHANNELS: usize = 2;

/// The WASAPI buffer of a per-process client: 100 ms, much longer than
/// [`READ_INTERVAL`].
const CLIENT_BUFFER_HNS: i64 = 1_000_000;

/// How long [`open`] waits for the capture thread to start.
const INIT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn open(
    sample_rate: u32,
    chunk_size: usize,
    apps: AppSet,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
) -> Result<(), Error> {
    let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), Error>>(1);
    std::thread::Builder::new()
        .name("longhand-app-audio".into())
        .spawn(move || run(sample_rate, chunk_size, apps, tx, init_tx))
        .map_err(|e| Error::SpeakerStreamInitializationFailed(e.to_string()))?;
    match init_rx.recv_timeout(INIT_TIMEOUT) {
        Ok(result) => result,
        Err(_) => Err(Error::SpeakerStreamInitializationFailed(
            "app audio did not start in time".into(),
        )),
    }
}

fn run(
    sample_rate: u32,
    chunk_size: usize,
    apps: AppSet,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
    init_tx: std_mpsc::SyncSender<Result<(), Error>>,
) {
    let opened = (|| {
        initialize_mta()
            .ok()
            .map_err(|e| failed("COM apartment", e))?;
        DeviceEnumerator::new().map_err(|e| failed("device enumerator", e))
    })();
    let enumerator = match opened {
        Ok(enumerator) => {
            let _ = init_tx.send(Ok(()));
            enumerator
        }
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };
    log::info!("app audio started");
    let out_rate = if sample_rate == 0 {
        MIX_RATE
    } else {
        sample_rate
    };
    let mut capture = AppCapture::new(enumerator, apps, out_rate, chunk_size);
    let end = capture.run(&tx);
    capture.stop_all();
    if let Some(error) = end {
        let _ = tx.blocking_send(Err(error));
    }
    log::info!("app audio stopped");
}

/// One per-process loopback client.
struct ProcessClient {
    audio_client: AudioClient,
    _event: Handle,
    capture_client: AudioCaptureClient,
    app_id: String,
}

fn open_process_client(pid: u32, app_id: &str) -> Result<ProcessClient, Error> {
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        MIX_RATE as usize,
        CLIENT_CHANNELS,
        None,
    );
    // `true`: the client records the process and all the processes that
    // it starts. `false` records all sound except that tree (KTD4).
    let mut audio_client = AudioClient::new_application_loopback_client(pid, true)
        .map_err(|e| failed("process loopback client", e))?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: CLIENT_BUFFER_HNS,
    };
    audio_client
        .initialize_client(&format, &Direction::Capture, &mode)
        .map_err(|e| failed("process loopback initialize", e))?;
    // The event mode needs an event handle. The thread reads the clients
    // at fixed times and does not wait for the event.
    let event = audio_client
        .set_get_eventhandle()
        .map_err(|e| failed("WASAPI event handle", e))?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| failed("WASAPI capture client", e))?;
    audio_client
        .start_stream()
        .map_err(|e| failed("process loopback start", e))?;
    Ok(ProcessClient {
        audio_client,
        _event: event,
        capture_client,
        app_id: app_id.to_owned(),
    })
}

/// The device loopback of Teams mode, with its converter.
struct DeviceCapture {
    endpoint: String,
    capture: LoopbackCapture,
    converter: Converter,
}

struct AppCapture {
    enumerator: DeviceEnumerator,
    apps: AppSet,
    ids: Vec<String>,
    out_rate: u32,
    chunk_size: usize,
    table: ProcessTable,
    clients: HashMap<u32, ProcessClient>,
    /// Root processes whose client did not open. The thread does not try
    /// them again while they run.
    failed: HashSet<u32>,
    mixer: BlockMixer,
    rate: RateFollower,
    teams: TeamsMode,
    device: Option<DeviceCapture>,
    new_session: Arc<AtomicBool>,
    _sessions: SessionWatch,
    bytes: VecDeque<u8>,
    mono: Vec<f32>,
}

impl AppCapture {
    fn new(enumerator: DeviceEnumerator, apps: AppSet, out_rate: u32, chunk_size: usize) -> Self {
        let new_session = Arc::new(AtomicBool::new(false));
        let sessions = SessionWatch::new(new_session.clone());
        Self {
            enumerator,
            apps,
            ids: Vec::new(),
            out_rate,
            chunk_size,
            table: ProcessTable::default(),
            clients: HashMap::new(),
            failed: HashSet::new(),
            mixer: BlockMixer::new(MIX_RATE),
            rate: RateFollower::new(MIX_RATE, out_rate, chunk_size),
            teams: TeamsMode::new(),
            device: None,
            new_session,
            _sessions: sessions,
            bytes: VecDeque::new(),
            mono: Vec::new(),
        }
    }

    /// Returns the error that ends the stream, or `None` when the consumer
    /// closed it.
    fn run(&mut self, tx: &mpsc::Sender<Result<CaptureFrame, Error>>) -> Option<Error> {
        let mut next_scan = Instant::now();
        let mut next_meter = Instant::now();
        loop {
            if tx.is_closed() {
                return None;
            }
            let now = Instant::now();
            if now >= next_meter {
                next_meter = now + TEAMS_METER_INTERVAL;
                let ids = self.apps.get();
                if ids != self.ids {
                    log::info!("the app set changed to {ids:?}");
                    self.ids = ids;
                    next_scan = now;
                }
                if let Err(e) = self.check_teams(now) {
                    return Some(e);
                }
            }
            let new_session = self.new_session.swap(false, Ordering::Relaxed);
            if new_session || now >= next_scan {
                next_scan = now + SCAN_INTERVAL;
                self.table = process_table();
                if self.device.is_none()
                    && let Err(e) = self.sync_clients()
                {
                    return Some(e);
                }
            }

            let sent = match self.device {
                Some(_) => self.read_device(tx),
                None => Ok(self.read_clients(now, tx)),
            };
            match sent {
                Ok(true) => {}
                Ok(false) => return None,
                Err(e) => return Some(e),
            }
            std::thread::sleep(READ_INTERVAL);
        }
    }

    /// Open a client for each new root process of the app set, and stop
    /// the clients of processes that ended or of apps that left the set.
    /// When apps of the set run and no client opens, the stream ends, so
    /// the capture manager opens it again and reports the failure (R16).
    fn sync_clients(&mut self) -> Result<(), Error> {
        let roots = self.table.roots(&self.ids, std::process::id());
        let wanted: HashSet<u32> = roots.iter().map(|(pid, _)| *pid).collect();
        let ended: Vec<u32> = self
            .clients
            .keys()
            .filter(|pid| !wanted.contains(pid))
            .copied()
            .collect();
        for pid in ended {
            if let Some(client) = self.clients.remove(&pid) {
                let _ = client.audio_client.stop_stream();
                log::info!(
                    "app audio stopped the client of {} (PID {pid})",
                    client.app_id
                );
            }
            self.mixer.remove(pid);
        }
        self.failed.retain(|pid| wanted.contains(pid));
        for (pid, app_id) in roots {
            if self.clients.contains_key(&pid) || self.failed.contains(&pid) {
                continue;
            }
            match open_process_client(pid, &app_id) {
                Ok(client) => {
                    log::info!("app audio opened a client of {app_id} (PID {pid})");
                    self.clients.insert(pid, client);
                }
                Err(e) => {
                    log::warn!("app audio could not open a client of {app_id} (PID {pid}): {e}");
                    self.failed.insert(pid);
                }
            }
        }
        if !wanted.is_empty() && self.clients.is_empty() {
            log::warn!("no client of the app set opened; app audio ends");
            return Err(Error::SpeakerStreamEnded);
        }
        Ok(())
    }

    fn stop_clients(&mut self) {
        for (_, client) in self.clients.drain() {
            let _ = client.audio_client.stop_stream();
        }
        self.failed.clear();
        self.mixer.clear();
        self.rate.reset_at(MIX_RATE);
    }

    fn stop_all(&mut self) {
        self.stop_clients();
        if let Some(device) = self.device.take() {
            let _ = device.capture.audio_client.stop_stream();
        }
    }

    /// Read the Teams meters and change the mode when the rule says so.
    fn check_teams(&mut self, now: Instant) -> Result<(), Error> {
        let teams_in_set = self.ids.iter().any(|id| id == WINDOWS_TEAMS_ID);
        let readings = if teams_in_set {
            teams_readings(&self.table)
        } else {
            Vec::new()
        };
        let mode = self.teams.observe(teams_in_set, &readings, now).clone();
        let current = self.device.as_ref().map(|d| d.endpoint.as_str());
        match (&mode, current) {
            (CaptureMode::PerProcess, None) => Ok(()),
            (CaptureMode::PerProcess, Some(_)) => {
                if let Some(device) = self.device.take() {
                    let _ = device.capture.audio_client.stop_stream();
                }
                log::info!("Teams stopped playing; app audio uses per-process clients again");
                self.sync_clients()
            }
            (CaptureMode::Device(endpoint), current) if current == Some(endpoint.as_str()) => {
                Ok(())
            }
            (CaptureMode::Device(endpoint), _) => {
                let endpoint = endpoint.clone();
                self.open_device(&endpoint)
            }
        }
    }

    /// Start device loopback on `endpoint` in place of the per-process
    /// clients (R19).
    fn open_device(&mut self, endpoint: &str) -> Result<(), Error> {
        if let Some(device) = self.device.take() {
            let _ = device.capture.audio_client.stop_stream();
        }
        let capture = self
            .enumerator
            .get_device(endpoint)
            .map_err(|e| failed("Teams output device", e))
            .and_then(open_loopback_capture)
            .map_err(|e| {
                log::warn!("app audio could not record the Teams output device: {e}");
                Error::SpeakerStreamEnded
            })?;
        self.stop_clients();
        log::info!(
            "Teams plays; app audio records all sound of {:?}",
            capture.device_name
        );
        let converter = Converter::new(capture.format, self.out_rate, self.chunk_size);
        self.device = Some(DeviceCapture {
            endpoint: endpoint.to_owned(),
            capture,
            converter,
        });
        Ok(())
    }

    /// Read every client and send the mixed blocks. Returns false when the
    /// consumer closed the stream.
    fn read_clients(
        &mut self,
        now: Instant,
        tx: &mpsc::Sender<Result<CaptureFrame, Error>>,
    ) -> bool {
        let mut ended = Vec::new();
        for (pid, client) in &self.clients {
            self.bytes.clear();
            if let Err(e) = drain_packets(&client.capture_client, &mut self.bytes) {
                log::info!(
                    "app audio read of {} (PID {pid}) failed: {e}",
                    client.app_id
                );
                ended.push(*pid);
                continue;
            }
            self.mono.clear();
            downmix(
                self.bytes.make_contiguous(),
                CLIENT_CHANNELS,
                SampleKind::F32,
                &mut self.mono,
            );
            self.mixer.push(*pid, &self.mono);
        }
        // The next scan opens a client again when the process still runs.
        for pid in ended {
            if let Some(client) = self.clients.remove(&pid) {
                let _ = client.audio_client.stop_stream();
            }
            self.mixer.remove(pid);
        }
        let mut closed = false;
        let rate = &mut self.rate;
        self.mixer.mix(now, |block| {
            rate.push(block, |frame| {
                if !closed {
                    closed = tx.blocking_send(Ok(speaker_frame(frame))).is_err();
                }
            });
        });
        !closed
    }

    /// Read the device loopback and send its frames. Returns false when
    /// the consumer closed the stream.
    fn read_device(
        &mut self,
        tx: &mpsc::Sender<Result<CaptureFrame, Error>>,
    ) -> Result<bool, Error> {
        let Some(device) = self.device.as_mut() else {
            return Ok(true);
        };
        self.bytes.clear();
        if let Err(e) = drain_packets(&device.capture.capture_client, &mut self.bytes) {
            log::warn!(
                "app audio read of {:?} failed: {e}",
                device.capture.device_name
            );
            let endpoint = device.endpoint.clone();
            self.open_device(&endpoint)?;
            return Ok(true);
        }
        if self.bytes.is_empty() {
            return Ok(true);
        }
        let mut closed = false;
        device
            .converter
            .push(self.bytes.make_contiguous(), |frame| {
                if !closed {
                    closed = tx.blocking_send(Ok(speaker_frame(frame))).is_err();
                }
            });
        Ok(!closed)
    }
}

/// The meters of the render sessions whose process, or a parent in its
/// chain, is Teams (KTD5).
fn teams_readings(table: &ProcessTable) -> Vec<TeamsReading> {
    render_sessions()
        .into_iter()
        .filter(|s| table.in_chain(s.pid, WINDOWS_TEAMS_ID))
        .map(|s| TeamsReading {
            endpoint: s.endpoint_id,
            peak: s.peak,
        })
        .collect()
}

/// One audio session on an output device.
#[derive(Debug, Clone)]
pub struct RenderSession {
    pub endpoint_id: String,
    pub endpoint_name: Option<String>,
    pub pid: u32,
    /// "active", "inactive" or "expired".
    pub state: &'static str,
    /// The peak meter value of the session, 0 to 1.
    pub peak: f32,
}

/// The audio sessions of each active output device. A value that cannot be
/// read counts as 0 or as "unknown". The calling thread must be in a COM
/// apartment; this function joins the multithreaded apartment when it is
/// in none.
pub fn render_sessions() -> Vec<RenderSession> {
    let _ = initialize_mta();
    let mut sessions = Vec::new();
    for device in render_devices() {
        let Some(endpoint_id) = device_id(&device) else {
            continue;
        };
        let endpoint_name = wasapi::Device::from_immdevice(device.clone())
            .ok()
            .and_then(|d| d.get_friendlyname().ok());
        // SAFETY: COM calls on interfaces that this thread owns.
        unsafe {
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(list) = manager.GetSessionEnumerator() else {
                continue;
            };
            for index in 0..list.GetCount().unwrap_or(0) {
                let Ok(session) = list.GetSession(index) else {
                    continue;
                };
                let pid = session
                    .cast::<IAudioSessionControl2>()
                    .and_then(|s| s.GetProcessId())
                    .unwrap_or(0);
                let state = match session.GetState() {
                    Ok(s) if s == AudioSessionStateActive => "active",
                    Ok(s) if s == AudioSessionStateInactive => "inactive",
                    Ok(s) if s == AudioSessionStateExpired => "expired",
                    _ => "unknown",
                };
                let peak = session
                    .cast::<IAudioMeterInformation>()
                    .and_then(|m| m.GetPeakValue())
                    .unwrap_or(0.0);
                sessions.push(RenderSession {
                    endpoint_id: endpoint_id.clone(),
                    endpoint_name: endpoint_name.clone(),
                    pid,
                    state,
                    peak,
                });
            }
        }
    }
    sessions
}

/// The process with `pid`, then its parent, and so on, as (PID, app ID)
/// pairs.
pub fn parent_chain(pid: u32) -> Vec<(u32, String)> {
    process_table()
        .chain(pid)
        .into_iter()
        .map(|e| (e.pid, e.app_id.clone()))
        .collect()
}

fn render_devices() -> Vec<IMMDevice> {
    // SAFETY: COM calls on interfaces that this thread owns.
    unsafe {
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return Vec::new();
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return Vec::new();
        };
        (0..collection.GetCount().unwrap_or(0))
            .filter_map(|index| collection.Item(index).ok())
            .collect()
    }
}

fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: GetId returns a string that the caller frees with
    // CoTaskMemFree.
    unsafe {
        let raw = device.GetId().ok()?;
        let id = raw.to_string();
        CoTaskMemFree(Some(raw.0 as *const _));
        id.ok()
    }
}

/// Sets a flag when an output device gets a new audio session, so the
/// thread scans the processes at once.
#[implement(IAudioSessionNotification)]
struct SessionCreated {
    flag: Arc<AtomicBool>,
}

impl IAudioSessionNotification_Impl for SessionCreated_Impl {
    fn OnSessionCreated(&self, _session: Ref<IAudioSessionControl>) -> ::windows::core::Result<()> {
        self.flag.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// The session notifications of each output device that is active when
/// the capture starts. The 2 s scan finds the sessions of other devices.
struct SessionWatch {
    registrations: Vec<(IAudioSessionManager2, IAudioSessionNotification)>,
}

impl SessionWatch {
    fn new(flag: Arc<AtomicBool>) -> Self {
        let mut registrations = Vec::new();
        for device in render_devices() {
            let notification: IAudioSessionNotification =
                SessionCreated { flag: flag.clone() }.into();
            // SAFETY: COM calls on interfaces that this thread owns.
            unsafe {
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                    continue;
                };
                // Windows sends session notifications only after the first
                // call to GetSessionEnumerator.
                let _ = manager.GetSessionEnumerator();
                match manager.RegisterSessionNotification(&notification) {
                    Ok(()) => registrations.push((manager, notification)),
                    Err(e) => log::warn!("app audio could not watch for new sessions: {e}"),
                }
            }
        }
        Self { registrations }
    }
}

impl Drop for SessionWatch {
    fn drop(&mut self) {
        for (manager, notification) in &self.registrations {
            // SAFETY: the registration was made on this thread.
            let _ = unsafe { manager.UnregisterSessionNotification(notification) };
        }
    }
}
