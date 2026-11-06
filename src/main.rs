#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Quby Render Queue 1.3
//! A batch render queue for Blender 3.x / 4.x / 5.x on Windows.

use eframe::egui::{self, *};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const APP_VERSION: &str = "1.3";

// ─── Colors ───────────────────────────────────────────────────────────────────
const BG: Color32 = Color32::from_rgb(8, 12, 35);
const PANEL: Color32 = Color32::from_rgb(13, 20, 55);
const ITEM: Color32 = Color32::from_rgb(18, 26, 65);
const ITEM_SEL: Color32 = Color32::from_rgb(30, 40, 110);
const BORDER: Color32 = Color32::from_rgb(30, 40, 90);
const PURPLE: Color32 = Color32::from_rgb(130, 50, 220);
const PURPLE_LT: Color32 = Color32::from_rgb(160, 90, 255);
const GREEN: Color32 = Color32::from_rgb(34, 197, 94);
const GREEN_DK: Color32 = Color32::from_rgb(22, 163, 74);
const TEXT: Color32 = Color32::from_rgb(220, 228, 255);
const MUTED: Color32 = Color32::from_rgb(110, 120, 170);
const RED: Color32 = Color32::from_rgb(239, 68, 68);
const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
const LOG_BG: Color32 = Color32::from_rgb(6, 10, 28);

// ─── Embedded Blender scripts ─────────────────────────────────────────────────
const APPLY_PY: &str = include_str!("qrq_apply.py");
const CORRIDORKEY_PY: &str = include_str!("qrq_corridorkey.py");
const PROBE_PY: &str = include_str!("qrq_probe.py");

// ─── Presets ──────────────────────────────────────────────────────────────────
const RES_PRESETS: &[(&str, i32, i32)] = &[
    ("HD 720p", 1280, 720),
    ("Full HD 1080p", 1920, 1080),
    ("QHD 1440p", 2560, 1440),
    ("UHD 4K", 3840, 2160),
    ("DCI 4K", 4096, 2160),
    ("8K UHD", 7680, 4320),
    ("Square 1080", 1080, 1080),
    ("Vertical 1080x1920", 1080, 1920),
];

const FPS_PRESETS: &[f32] = &[
    12.0, 15.0, 23.976, 24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0, 90.0, 120.0,
];

// ─── Fallback capability lists (used until Blender has been probed) ───────────
const D_FILE_FORMATS: &[&str] = &[
    "PNG", "JPEG", "OPEN_EXR", "OPEN_EXR_MULTILAYER", "TIFF", "TARGA", "TARGA_RAW",
    "BMP", "IRIS", "JPEG2000", "CINEON", "DPX", "HDR", "WEBP", "AVI_JPEG", "AVI_RAW",
    "FFMPEG",
];
const D_COLOR_MODES: &[&str] = &["BW", "RGB", "RGBA"];
const D_COLOR_DEPTHS: &[&str] = &["8", "10", "12", "16", "32"];
const D_EXR_CODECS: &[&str] = &[
    "NONE", "PXR24", "ZIP", "PIZ", "RLE", "ZIPS", "B44", "B44A", "DWAA", "DWAB",
];
const D_TIFF_CODECS: &[&str] = &["NONE", "DEFLATE", "LZW", "PACKBITS"];
const D_FF_CONTAINERS: &[&str] = &[
    "MPEG1", "MPEG2", "MPEG4", "AVI", "QUICKTIME", "DV", "OGG", "MKV", "FLASH", "WEBM",
];
const D_FF_VIDEO: &[&str] = &[
    "NONE", "H264", "HEVC", "AV1", "MPEG4", "MPEG2", "MPEG1", "WEBM", "THEORA", "FFV1",
    "HUFFYUV", "DNXHD", "PNG", "QTRLE", "DV", "FLASH",
];
const D_FF_AUDIO: &[&str] = &[
    "NONE", "AAC", "AC3", "FLAC", "MP2", "MP3", "OPUS", "PCM", "VORBIS",
];
const D_FF_CRF: &[&str] = &[
    "NONE", "LOSSLESS", "PERC_LOSSLESS", "HIGH", "MEDIUM", "LOW", "VERYLOW", "LOWEST",
];
const D_FF_PRESET: &[&str] = &["BEST", "GOOD", "REALTIME"];
const D_ENGINES: &[&str] = &["CYCLES", "BLENDER_EEVEE_NEXT", "BLENDER_EEVEE", "BLENDER_WORKBENCH"];
const D_VIEW_TRANSFORMS: &[&str] = &[
    "Standard", "AgX", "Khronos PBR Neutral", "Filmic", "Filmic Log", "False Color", "Raw",
];
const D_LOOKS: &[&str] = &["None"];
const D_DISPLAY: &[&str] = &["sRGB", "Display P3", "Rec.1886", "Rec.2020", "None"];
const D_COLORSPACES: &[&str] = &["sRGB", "Linear Rec.709", "Filmic Log", "Rec.1886", "Raw"];
const D_DENOISERS: &[&str] = &["OPENIMAGEDENOISE", "OPTIX"];

fn listed(v: &[String], fallback: &[&str]) -> Vec<String> {
    if v.is_empty() {
        fallback.iter().map(|s| s.to_string()).collect()
    } else {
        v.to_vec()
    }
}

// ─── Data types ───────────────────────────────────────────────────────────────
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Device {
    Auto,
    Cpu,
    Cuda,
    Optix,
    Hip,
    OneApi,
    Metal,
    CudaCpu,
    OptixCpu,
}

impl Default for Device {
    fn default() -> Self {
        Device::Cuda
    }
}

impl Device {
    const ALL: [Device; 9] = [
        Device::Auto,
        Device::Cpu,
        Device::Cuda,
        Device::Optix,
        Device::Hip,
        Device::OneApi,
        Device::Metal,
        Device::CudaCpu,
        Device::OptixCpu,
    ];

    /// Value passed to Blender's `--cycles-device`. `None` = leave it to the file.
    fn cli(self) -> Option<&'static str> {
        match self {
            Device::Auto => None,
            Device::Cpu => Some("CPU"),
            Device::Cuda => Some("CUDA"),
            Device::Optix => Some("OPTIX"),
            Device::Hip => Some("HIP"),
            Device::OneApi => Some("ONEAPI"),
            Device::Metal => Some("METAL"),
            Device::CudaCpu => Some("CUDA+CPU"),
            Device::OptixCpu => Some("OPTIX+CPU"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Device::Auto => "From file",
            Device::Cpu => "CPU",
            Device::Cuda => "CUDA",
            Device::Optix => "OptiX",
            Device::Hip => "HIP (AMD)",
            Device::OneApi => "oneAPI (Intel)",
            Device::Metal => "Metal",
            Device::CudaCpu => "CUDA + CPU",
            Device::OptixCpu => "OptiX + CPU",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Status {
    Pending,
    Rendering { frame: i32 },
    Done,
    Failed(String),
}

impl Default for Status {
    fn default() -> Self {
        Status::Pending
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum JobMode {
    Animation,
    Still,
}

impl Default for JobMode {
    fn default() -> Self {
        JobMode::Animation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum RetimeMode {
    Off,
    Remap,
    Keys,
}

impl Default for RetimeMode {
    fn default() -> Self {
        RetimeMode::Remap
    }
}

impl RetimeMode {
    fn json(self) -> &'static str {
        match self {
            RetimeMode::Off => "NONE",
            RetimeMode::Remap => "REMAP",
            RetimeMode::Keys => "KEYS",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum TriState {
    Keep,
    On,
    Off,
}

impl Default for TriState {
    fn default() -> Self {
        TriState::Keep
    }
}

impl TriState {
    fn json(self) -> serde_json::Value {
        match self {
            TriState::Keep => serde_json::Value::Null,
            TriState::On => serde_json::Value::from(1),
            TriState::Off => serde_json::Value::from(0),
        }
    }
    fn label(self) -> &'static str {
        match self {
            TriState::Keep => "From file",
            TriState::On => "On",
            TriState::Off => "Off",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum PostAction {
    Nothing,
    Sleep,
    Hibernate,
    Shutdown,
}

impl Default for PostAction {
    fn default() -> Self {
        PostAction::Nothing
    }
}

impl PostAction {
    fn label(self) -> &'static str {
        match self {
            PostAction::Nothing => "Do nothing",
            PostAction::Sleep => "Sleep",
            PostAction::Hibernate => "Hibernate",
            PostAction::Shutdown => "Shut down",
        }
    }
}

// ─── Per-job settings ─────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct JobSettings {
    // scene
    scene: String,
    camera: String,
    engine: String,
    view_layer: String,

    // frames
    job_mode: JobMode,
    still_frame: i32,
    frame_step: i32,

    // output format
    override_format: bool,
    file_format: String,
    color_mode: String,
    color_depth: String,
    png_compression: i32,
    quality: i32,
    exr_codec: String,
    tiff_codec: String,

    // ffmpeg
    ff_container: String,
    ff_video_codec: String,
    ff_audio_codec: String,
    ff_crf: String,
    ff_preset: String,
    ff_gopsize: i32,
    ff_audio_bitrate: i32,
    ff_use_custom_bitrate: bool,
    ff_video_bitrate: i32,

    // resolution
    override_resolution: bool,
    res_x: i32,
    res_y: i32,
    res_pct: i32,

    // colour management
    override_color: bool,
    view_transform: String,
    look: String,
    exposure: f32,
    gamma: f32,
    display_device: String,
    output_colorspace: String,
    sequencer_colorspace: String,

    // fps
    override_fps: bool,
    fps: f32,
    retime: RetimeMode,
    retime_vse: bool,

    // post processing
    override_post: bool,
    use_compositing: bool,
    use_sequencer: bool,
    skip_3d: bool,

    // sampling
    override_quality: bool,
    samples: i32,
    eevee_samples: i32,
    adaptive_threshold: f32,
    use_denoise: bool,
    denoiser: String,
    motion_blur: TriState,
    film_transparent: TriState,

    // performance / output behaviour
    persistent_data: bool,
    threads: i32,
    overwrite: bool,
    placeholders: bool,
    use_stamp: TriState,
    max_retries: i32,
}

impl Default for JobSettings {
    fn default() -> Self {
        Self {
            scene: String::new(),
            camera: String::new(),
            engine: String::new(),
            view_layer: String::new(),

            job_mode: JobMode::Animation,
            still_frame: 1,
            frame_step: 1,

            override_format: false,
            file_format: "PNG".into(),
            color_mode: "RGBA".into(),
            color_depth: "8".into(),
            png_compression: 15,
            quality: 90,
            exr_codec: "ZIP".into(),
            tiff_codec: "DEFLATE".into(),

            ff_container: "MPEG4".into(),
            ff_video_codec: "H264".into(),
            ff_audio_codec: "AAC".into(),
            ff_crf: "HIGH".into(),
            ff_preset: "GOOD".into(),
            ff_gopsize: 18,
            ff_audio_bitrate: 192,
            ff_use_custom_bitrate: false,
            ff_video_bitrate: 6000,

            override_resolution: false,
            res_x: 1920,
            res_y: 1080,
            res_pct: 100,

            override_color: false,
            view_transform: "AgX".into(),
            look: "None".into(),
            exposure: 0.0,
            gamma: 1.0,
            display_device: "sRGB".into(),
            output_colorspace: String::new(),
            sequencer_colorspace: String::new(),

            override_fps: false,
            fps: 30.0,
            retime: RetimeMode::Remap,
            retime_vse: true,

            override_post: false,
            use_compositing: true,
            use_sequencer: true,
            skip_3d: false,

            override_quality: false,
            samples: 128,
            eevee_samples: 64,
            adaptive_threshold: 0.01,
            use_denoise: true,
            denoiser: "OPENIMAGEDENOISE".into(),
            motion_blur: TriState::Keep,
            film_transparent: TriState::Keep,

            persistent_data: true,
            threads: 0,
            overwrite: true,
            placeholders: false,
            use_stamp: TriState::Keep,
            max_retries: 0,
        }
    }
}

// ─── Queue item ───────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct RenderItem {
    id: usize,
    path: String,
    name: String,
    device: Device,
    frame_start: i32,
    frame_end: i32,
    output_path: String,
    status: Status,

    // new in 1.3
    enabled: bool,
    settings: JobSettings,
    last_secs: f64,
    scenes: Vec<String>,
    cameras: Vec<String>,
    view_layers: Vec<String>,
    source_fps: f32,
    probed: bool,
}

impl Default for RenderItem {
    fn default() -> Self {
        Self {
            id: 0,
            path: String::new(),
            name: String::new(),
            device: Device::Cuda,
            frame_start: 1,
            frame_end: 250,
            output_path: String::new(),
            status: Status::Pending,
            enabled: true,
            settings: JobSettings::default(),
            last_secs: 0.0,
            scenes: Vec::new(),
            cameras: Vec::new(),
            view_layers: Vec::new(),
            source_fps: 0.0,
            probed: false,
        }
    }
}

// ─── Capabilities probed from Blender ─────────────────────────────────────────
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Caps {
    version: String,
    version_string: String,
    file_formats: Vec<String>,
    color_modes: Vec<String>,
    color_depths: Vec<String>,
    exr_codecs: Vec<String>,
    tiff_codecs: Vec<String>,
    ff_containers: Vec<String>,
    ff_video_codecs: Vec<String>,
    ff_audio_codecs: Vec<String>,
    ff_crf: Vec<String>,
    ff_presets: Vec<String>,
    engines: Vec<String>,
    view_transforms: Vec<String>,
    looks: Vec<String>,
    display_devices: Vec<String>,
    colorspaces: Vec<String>,
    denoisers: Vec<String>,
    cycles_device_types: Vec<String>,
    has_time_remap: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct SceneInfo {
    name: String,
    frame_start: i32,
    frame_end: i32,
    frame_step: i32,
    fps: f32,
    resolution_x: i32,
    resolution_y: i32,
    resolution_percentage: i32,
    engine: String,
    file_format: String,
    filepath: String,
    camera: String,
    cameras: Vec<String>,
    view_layers: Vec<String>,
    use_compositing: bool,
    use_sequencer: bool,
    sequencer_strips: i32,
    compositor_nodes: i32,
    use_nodes: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct BlendInfo {
    version: String,
    active_scene: String,
    scenes: Vec<SceneInfo>,
}

// ─── Config ───────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    blender_path: Option<String>,
    caps: Caps,
    post_action: PostAction,
    auto_exec: bool,
    compact_log: bool,
    split_ratio: f32,
    presets: Vec<(String, JobSettings)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            blender_path: None,
            caps: Caps::default(),
            post_action: PostAction::Nothing,
            auto_exec: false,
            compact_log: true,
            split_ratio: 0.36,
            presets: Vec::new(),
        }
    }
}

// ─── Worker events ────────────────────────────────────────────────────────────
enum Event {
    Log(String),
    Frame { id: usize, frame: i32 },
    Progress { id: usize, elapsed: String, remaining: String },
    Done { id: usize, secs: f64 },
    Failed { id: usize, msg: String },
    Probed { id: usize, info: Box<BlendInfo> },
    ProbedCaps(Box<Caps>),
    Finished,
}

// ─── App state ────────────────────────────────────────────────────────────────
struct App {
    config: Config,
    queue: Vec<RenderItem>,
    selected: Option<usize>,
    next_id: usize,
    rendering: bool,
    cancel: Arc<AtomicBool>,
    child_pid: Arc<AtomicU32>,
    event_rx: Option<Receiver<Event>>,
    event_tx: Sender<Event>,
    probe_rx: Receiver<Event>,
    log: Vec<String>,
    setup_open: bool,
    about_open: bool,
    cfg_path: PathBuf,
    q_path: PathBuf,
    scripts_ok: bool,
    dirty: bool,
    last_save: Instant,
    job_started: Option<Instant>,
    cur_elapsed: String,
    cur_remaining: String,
    queue_started: Option<Instant>,
    preset_name: String,
    /// Serialises background Blender probes so adding 20 files at once does
    /// not launch 20 Blender processes simultaneously.
    probe_lock: Arc<Mutex<()>>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        setup_theme(&cc.egui_ctx);
        let dir = app_dir();
        let cfg_path = dir.join("config.json");
        let q_path = dir.join("queue.json");

        let config: Config = load_json(&cfg_path).unwrap_or_default();
        let mut queue: Vec<RenderItem> = load_json(&q_path).unwrap_or_default();

        // Reset rendering statuses from a previous session
        for item in &mut queue {
            if matches!(item.status, Status::Rendering { .. }) {
                item.status = Status::Pending;
            }
        }

        let next_id = queue.iter().map(|i| i.id + 1).max().unwrap_or(1);
        let setup_open = config.blender_path.is_none();

        let scripts_ok = write_scripts();
        let (tx, rx) = mpsc::channel::<Event>();

        let mut app = Self {
            config,
            queue,
            selected: None,
            next_id,
            rendering: false,
            cancel: Arc::new(AtomicBool::new(false)),
            child_pid: Arc::new(AtomicU32::new(0)),
            event_rx: None,
            event_tx: tx,
            probe_rx: rx,
            log: vec![format!("Quby Render Queue {} ready.", APP_VERSION)],
            setup_open,
            about_open: false,
            cfg_path,
            q_path,
            scripts_ok,
            dirty: false,
            last_save: Instant::now(),
            job_started: None,
            cur_elapsed: String::new(),
            cur_remaining: String::new(),
            queue_started: None,
            preset_name: String::new(),
            probe_lock: Arc::new(Mutex::new(())),
        };

        if !scripts_ok {
            app.log.push(
                "[!] Could not write the Blender helper scripts to AppData. \
                 Advanced settings will be skipped."
                    .into(),
            );
        }
        if app.config.blender_path.is_some() && app.config.caps.file_formats.is_empty() {
            app.probe_caps();
        }
        app
    }

    // ── persistence ──────────────────────────────────────────────────────────
    fn save(&mut self) {
        save_json(&self.cfg_path, &self.config);
        save_json(&self.q_path, &self.queue);
        self.dirty = false;
        self.last_save = Instant::now();
    }

    fn touch(&mut self) {
        self.dirty = true;
    }

    fn logline(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 3000 {
            self.log.drain(0..1000);
        }
    }

    // ── adding files ─────────────────────────────────────────────────────────
    fn add_files_dialog(&mut self) {
        let Some(paths) = rfd::FileDialog::new()
            .add_filter("Blender File", &["blend"])
            .set_title("Choose one or more .blend files")
            .pick_files()
        else {
            return;
        };
        for p in paths {
            self.add_path(p);
        }
        self.save();
    }

    fn add_path(&mut self, path: PathBuf) {
        if path.extension().map(|e| e.to_string_lossy().to_lowercase())
            != Some("blend".to_string())
        {
            self.logline(format!("[!] Skipped (not a .blend): {}", path.display()));
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.blend".into());

        let item = RenderItem {
            id: self.next_id,
            path: path.to_string_lossy().to_string(),
            name: name.clone(),
            ..Default::default()
        };
        let id = item.id;
        self.next_id += 1;
        self.selected = Some(self.queue.len());
        self.queue.push(item);
        self.logline(format!("  Added: {} — reading scene info…", name));
        self.probe_blend(id, path);
    }

    // ── background probes ────────────────────────────────────────────────────
    fn probe_caps(&mut self) {
        let Some(blender) = self.config.blender_path.clone() else {
            return;
        };
        if !self.scripts_ok {
            return;
        }
        let tx = self.event_tx.clone();
        let script = probe_script_path();
        let out = app_dir().join("caps.json");
        let lock = self.probe_lock.clone();
        thread::spawn(move || {
            let _guard = lock.lock();
            let mut cmd = Command::new(&blender);
            cmd.arg("--background")
                .arg("--python")
                .arg(&script)
                .arg("--")
                .arg("--qrq-mode")
                .arg("caps")
                .arg("--qrq-out")
                .arg(&out)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);
            let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
            if ok {
                if let Some(c) = read_json::<Caps>(&out) {
                    let _ = tx.send(Event::Log(format!(
                        "  Blender {} detected — format and codec lists updated.",
                        if c.version_string.is_empty() {
                            c.version.clone()
                        } else {
                            c.version_string.clone()
                        }
                    )));
                    let _ = tx.send(Event::ProbedCaps(Box::new(c)));
                    return;
                }
            }
            let _ = tx.send(Event::Log(
                "[!] Could not read Blender's format list; using the built-in list.".into(),
            ));
        });
    }

    fn probe_blend(&mut self, id: usize, path: PathBuf) {
        let Some(blender) = self.config.blender_path.clone() else {
            return;
        };
        if !self.scripts_ok {
            return;
        }
        let tx = self.event_tx.clone();
        let script = probe_script_path();
        let out = app_dir().join(format!("blend_{}.json", id));
        let auto_exec = self.config.auto_exec;
        let lock = self.probe_lock.clone();
        thread::spawn(move || {
            let _guard = lock.lock();
            let mut cmd = Command::new(&blender);
            cmd.arg("--background").arg(&path);
            if auto_exec {
                cmd.arg("-y");
            }
            cmd.arg("--python")
                .arg(&script)
                .arg("--")
                .arg("--qrq-mode")
                .arg("blend")
                .arg("--qrq-out")
                .arg(&out)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);
            let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
            if ok {
                if let Some(info) = read_json::<BlendInfo>(&out) {
                    let _ = std::fs::remove_file(&out);
                    let _ = tx.send(Event::Probed { id, info: Box::new(info) });
                    return;
                }
            }
            let _ = tx.send(Event::Log(format!(
                "[!] Could not read scene info for job #{} (frames left at their defaults).",
                id
            )));
        });
    }

    fn apply_probe(&mut self, id: usize, info: BlendInfo) {
        let Some(it) = self.queue.iter_mut().find(|i| i.id == id) else {
            return;
        };
        let active = info.active_scene.clone();
        let sc = info
            .scenes
            .iter()
            .find(|s| s.name == active)
            .or_else(|| info.scenes.first());
        let Some(sc) = sc else { return };

        it.scenes = info.scenes.iter().map(|s| s.name.clone()).collect();
        it.cameras = sc.cameras.clone();
        it.view_layers = sc.view_layers.clone();
        it.source_fps = sc.fps;
        it.probed = true;
        it.frame_start = sc.frame_start;
        it.frame_end = sc.frame_end;

        let s = &mut it.settings;
        if s.fps <= 0.0 || !s.override_fps {
            s.fps = if sc.fps > 0.0 { sc.fps } else { 30.0 };
        }
        if !s.override_resolution {
            s.res_x = sc.resolution_x;
            s.res_y = sc.resolution_y;
            s.res_pct = sc.resolution_percentage;
        }
        if !s.override_format && !sc.file_format.is_empty() {
            s.file_format = sc.file_format.clone();
        }
        if !s.override_post {
            s.use_compositing = sc.use_compositing;
            s.use_sequencer = sc.use_sequencer;
        }
        s.still_frame = sc.frame_start;

        let name = it.name.clone();
        let msg = format!(
            "  {} — frames {}-{} @ {:.3} fps, {}x{}, {} scene(s){}",
            name,
            sc.frame_start,
            sc.frame_end,
            sc.fps,
            sc.resolution_x,
            sc.resolution_y,
            info.scenes.len(),
            if sc.sequencer_strips > 0 {
                format!(", {} VSE strips", sc.sequencer_strips)
            } else {
                String::new()
            }
        );
        self.logline(msg);
        self.touch();
    }

    // ── rendering ────────────────────────────────────────────────────────────
    fn start(&mut self) {
        if self.rendering {
            return;
        }
        let Some(blender) = self.config.blender_path.clone() else {
            self.setup_open = true;
            return;
        };

        let items: Vec<RenderItem> = self
            .queue
            .iter()
            .filter(|i| i.enabled && !matches!(i.status, Status::Done))
            .cloned()
            .collect();

        if items.is_empty() {
            self.logline("  Nothing to render (queue empty, disabled or already done).");
            return;
        }

        let (ev_tx, ev_rx) = mpsc::channel::<Event>();
        self.event_rx = Some(ev_rx);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.child_pid = Arc::new(AtomicU32::new(0));
        self.rendering = true;
        self.queue_started = Some(Instant::now());
        self.job_started = None;
        self.cur_elapsed.clear();
        self.cur_remaining.clear();

        let cancel = self.cancel.clone();
        let pid_slot = self.child_pid.clone();
        let post_action = self.config.post_action;
        let auto_exec = self.config.auto_exec;
        let compact = self.config.compact_log;
        let scripts_ok = self.scripts_ok;

        thread::spawn(move || {
            for item in items {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let attempts = item.settings.max_retries.max(0) + 1;
                let mut last_err = String::new();
                let mut ok = false;

                for attempt in 1..=attempts {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    if attempt > 1 {
                        let _ = ev_tx.send(Event::Log(format!(
                            "  Retry {}/{} for {}",
                            attempt - 1,
                            attempts - 1,
                            item.name
                        )));
                    }
                    match run_one(
                        &blender,
                        &item,
                        auto_exec,
                        compact,
                        scripts_ok,
                        &ev_tx,
                        &cancel,
                        &pid_slot,
                    ) {
                        Ok(secs) => {
                            let _ = ev_tx.send(Event::Done { id: item.id, secs });
                            let _ = ev_tx.send(Event::Log(format!(
                                "✓  {} done in {}",
                                item.name,
                                fmt_secs(secs)
                            )));
                            ok = true;
                            break;
                        }
                        Err(e) => last_err = e,
                    }
                }

                if !ok && !cancel.load(Ordering::SeqCst) {
                    let _ = ev_tx.send(Event::Failed {
                        id: item.id,
                        msg: last_err.clone(),
                    });
                    let _ = ev_tx.send(Event::Log(format!("✗  {} — {}", item.name, last_err)));
                }
            }

            let _ = ev_tx.send(Event::Finished);

            if !cancel.load(Ordering::SeqCst) {
                run_post_action(post_action);
            }
        });
    }

    fn stop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        let pid = self.child_pid.load(Ordering::SeqCst);
        if pid != 0 {
            kill_pid(pid);
        }
        self.rendering = false;
        self.job_started = None;
        for it in &mut self.queue {
            if matches!(it.status, Status::Rendering { .. }) {
                it.status = Status::Pending;
            }
        }
        self.logline("  Render cancelled.");
        self.save();
    }

    fn poll(&mut self) {
        let mut events: Vec<Event> = std::iter::from_fn(|| self.probe_rx.try_recv().ok()).collect();
        if let Some(rx) = &self.event_rx {
            events.extend(std::iter::from_fn(|| rx.try_recv().ok()));
        }

        let mut finished = false;
        for ev in events {
            match ev {
                Event::Log(s) => self.logline(s),
                Event::Frame { id, frame } => {
                    if let Some(i) = self.queue.iter_mut().find(|i| i.id == id) {
                        if !matches!(i.status, Status::Rendering { .. }) {
                            self.job_started = Some(Instant::now());
                        }
                        i.status = Status::Rendering { frame };
                    }
                }
                Event::Progress { id, elapsed, remaining } => {
                    if self.queue.iter().any(|i| i.id == id) {
                        self.cur_elapsed = elapsed;
                        self.cur_remaining = remaining;
                    }
                }
                Event::Done { id, secs } => {
                    if let Some(i) = self.queue.iter_mut().find(|i| i.id == id) {
                        i.status = Status::Done;
                        i.last_secs = secs;
                    }
                    self.job_started = None;
                    self.cur_remaining.clear();
                    self.save();
                }
                Event::Failed { id, msg } => {
                    if let Some(i) = self.queue.iter_mut().find(|i| i.id == id) {
                        i.status = Status::Failed(msg);
                    }
                    self.job_started = None;
                    self.save();
                }
                Event::Probed { id, info } => self.apply_probe(id, *info),
                Event::ProbedCaps(c) => {
                    self.config.caps = *c;
                    self.touch();
                }
                Event::Finished => finished = true,
            }
        }

        if finished {
            self.rendering = false;
            self.event_rx = None;
            self.job_started = None;
            let total = self
                .queue_started
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            self.queue_started = None;
            self.logline(format!("★  Queue finished in {}.", fmt_secs(total)));
            self.save();
        }
    }

    // ── helpers used by the UI ───────────────────────────────────────────────
    fn open_output(&mut self, idx: usize) {
        let item = &self.queue[idx];
        let p = if item.output_path.is_empty() {
            PathBuf::from(&item.path)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default()
        } else {
            PathBuf::from(item.output_path.replace('/', "\\"))
        };
        open_folder(&p);
    }
}

// ─── Render one job ───────────────────────────────────────────────────────────
#[allow(clippy::too_many_arguments)]
fn run_one(
    blender: &str,
    item: &RenderItem,
    auto_exec: bool,
    compact: bool,
    scripts_ok: bool,
    ev_tx: &Sender<Event>,
    cancel: &Arc<AtomicBool>,
    pid_slot: &Arc<AtomicU32>,
) -> Result<f64, String> {
    let s = &item.settings;
    let dev = item.device.cli();

    let _ = ev_tx.send(Event::Log(format!(
        "▶  {} — {}  [{}]",
        item.name,
        match s.job_mode {
            JobMode::Animation => format!("frames {}-{}", item.frame_start, item.frame_end),
            JobMode::Still => format!("still frame {}", s.still_frame),
        },
        dev.unwrap_or("device from file")
    )));

    // Write the per-job settings file next to the scripts.
    let json_path = app_dir().join(format!("job_{}.json", item.id));
    let use_script = scripts_ok && write_job_json(&json_path, item).is_ok();
    if scripts_ok && !use_script {
        let _ = ev_tx.send(Event::Log(
            "[!] Could not write the job settings file; rendering with the file's own settings."
                .into(),
        ));
    }

    let (fs, fe) = match s.job_mode {
        JobMode::Animation => (item.frame_start, item.frame_end),
        JobMode::Still => (s.still_frame, s.still_frame),
    };

    let mut cmd = Command::new(blender);
    cmd.arg("--background").arg(&item.path);
    if auto_exec {
        cmd.arg("-y");
    }
    if !s.scene.is_empty() {
        cmd.arg("-S").arg(&s.scene);
    }
    if !item.output_path.is_empty() {
        cmd.arg("-o").arg(&item.output_path);
    }
    cmd.arg("-s").arg(fs.to_string());
    cmd.arg("-e").arg(fe.to_string());
    if use_script {
        cmd.arg("--python").arg(apply_script_path());
    }
    if !scripts_ok {
        return Err("Could not write render preparation scripts; rendering stopped.".into());
    }
    cmd.arg("--python-exit-code").arg("1")
        .arg("--python").arg(corridorkey_script_path());
    cmd.arg("-a").arg("--");
    if use_script {
        cmd.arg("--qrq-json").arg(&json_path);
    }
    if let Some(d) = dev {
        cmd.arg("--cycles-device").arg(d);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err(format!("could not start Blender ({})", e)),
    };
    pid_slot.store(child.id(), Ordering::SeqCst);

    let saw_error = Arc::new(AtomicBool::new(false));

    if let Some(stdout) = child.stdout.take() {
        let tx = ev_tx.clone();
        let id = item.id;
        thread::spawn(move || {
            let mut last_fra = Instant::now() - Duration::from_secs(1);
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let is_fra = line.starts_with("Fra:");
                if is_fra {
                    if let Some(f) = parse_frame(&line) {
                        let _ = tx.send(Event::Frame { id, frame: f });
                    }
                    let el = token_after(&line, "Time:").unwrap_or_default();
                    let rem = token_after(&line, "Remaining:").unwrap_or_default();
                    if !el.is_empty() || !rem.is_empty() {
                        let _ = tx.send(Event::Progress { id, elapsed: el, remaining: rem });
                    }
                    // the sample counter fires many times a second: throttle the log
                    if compact && last_fra.elapsed() < Duration::from_millis(250) {
                        continue;
                    }
                    last_fra = Instant::now();
                }
                if compact && !is_fra && !keep_line(&line) {
                    continue;
                }
                let _ = tx.send(Event::Log(line));
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let tx = ev_tx.clone();
        let flag = saw_error.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let low = line.to_lowercase();
                if low.contains("error") || low.contains("traceback") {
                    flag.store(true, Ordering::SeqCst);
                }
                let _ = tx.send(Event::Log(format!("[!] {}", line)));
            }
        });
    }

    let status = child.wait();
    pid_slot.store(0, Ordering::SeqCst);
    let secs = started.elapsed().as_secs_f64();

    if cancel.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }

    match status {
        Ok(st) if st.success() => Ok(secs),
        Ok(st) => Err(format!(
            "Blender exited with code {}{}",
            st.code().unwrap_or(-1),
            if saw_error.load(Ordering::SeqCst) {
                " (see the [!] lines above)"
            } else {
                ""
            }
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// Lines worth showing when the log is in compact mode.
fn keep_line(line: &str) -> bool {
    let l = line.trim();
    if l.is_empty() {
        return false;
    }
    let low = l.to_lowercase();
    low.starts_with("qrq")
        || low.contains("error")
        || low.contains("warning")
        || low.contains("saved:")
        || low.contains("time:")
        || low.contains("blender quit")
        || low.contains("append frame")
        || low.contains("skipping existing frame")
}

// ─── Job settings → JSON for the Blender script ───────────────────────────────
fn write_job_json(path: &PathBuf, item: &RenderItem) -> std::io::Result<()> {
    let s = &item.settings;

    let format = if s.override_format {
        serde_json::json!({
            "enabled": true,
            "file_format": s.file_format,
            "color_mode": s.color_mode,
            "color_depth": s.color_depth,
            "compression": s.png_compression,
            "quality": s.quality,
            "exr_codec": s.exr_codec,
            "tiff_codec": s.tiff_codec,
        })
    } else {
        serde_json::json!({ "enabled": false })
    };

    let ffmpeg = if s.override_format && s.file_format == "FFMPEG" {
        serde_json::json!({
            "enabled": true,
            "container": s.ff_container,
            "video_codec": s.ff_video_codec,
            "audio_codec": s.ff_audio_codec,
            "crf": if s.ff_use_custom_bitrate { "NONE".to_string() } else { s.ff_crf.clone() },
            "preset": s.ff_preset,
            "gopsize": s.ff_gopsize,
            "audio_bitrate": s.ff_audio_bitrate,
            "use_custom_bitrate": s.ff_use_custom_bitrate,
            "video_bitrate": s.ff_video_bitrate,
        })
    } else {
        serde_json::json!({ "enabled": false })
    };

    let value = serde_json::json!({
        "qrq_version": APP_VERSION,
        "scene": s.scene,
        "camera": s.camera,
        "engine": s.engine,
        "view_layer": s.view_layer,
        "resolution": {
            "enabled": s.override_resolution,
            "x": s.res_x,
            "y": s.res_y,
            "percentage": s.res_pct,
        },
        "color": {
            "enabled": s.override_color,
            "view_transform": s.view_transform,
            "look": s.look,
            "exposure": s.exposure,
            "gamma": s.gamma,
            "display_device": s.display_device,
            "output_colorspace": s.output_colorspace,
            "sequencer_colorspace": s.sequencer_colorspace,
        },
        "format": format,
        "ffmpeg": ffmpeg,
        "post": {
            "enabled": s.override_post,
            "use_compositing": s.use_compositing,
            "use_sequencer": s.use_sequencer,
            "skip_3d": s.skip_3d,
        },
        "frames": {
            "step": s.frame_step,
        },
        "quality": {
            "enabled": s.override_quality,
            "samples": s.samples,
            "eevee_samples": s.eevee_samples,
            "adaptive_threshold": s.adaptive_threshold,
            "use_denoise": s.use_denoise,
            "denoiser": s.denoiser,
            "motion_blur": s.motion_blur.json(),
            "film_transparent": s.film_transparent.json(),
        },
        "perf": {
            "persistent_data": s.persistent_data,
            "threads": s.threads,
        },
        "output": {
            "overwrite": s.overwrite,
            "placeholders": s.placeholders,
            "use_stamp": s.use_stamp.json(),
        },
        "fps": {
            "enabled": s.override_fps,
            "fps": s.fps,
            "retime": s.retime.json(),
            "retime_vse": s.retime_vse,
        },
    });

    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&value).unwrap_or_default())
}

// ─── eframe impl ──────────────────────────────────────────────────────────────
impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.rendering {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        // Drag & drop .blend files anywhere on the window
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            for p in dropped {
                self.add_path(p);
            }
            self.save();
        }

        // Debounced save
        let released = ctx.input(|i| i.pointer.any_released());
        if self.dirty && (released || self.last_save.elapsed() > Duration::from_millis(1500)) {
            self.save();
        }

        if self.setup_open {
            self.ui_setup(ctx);
            return;
        }
        if self.about_open {
            self.ui_about(ctx);
        }

        TopBottomPanel::top("hdr")
            .exact_height(54.0)
            .frame(egui::Frame::none().fill(PANEL).stroke(Stroke::new(1.0, PURPLE)))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("QUBY RENDER QUEUE")
                            .font(FontId::proportional(24.0))
                            .color(TEXT)
                            .strong(),
                    );
                    ui.label(RichText::new(format!("v{}", APP_VERSION)).color(PURPLE_LT).small());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        if btn_small(ui, "?", ITEM, MUTED) {
                            self.about_open = true;
                        }
                        let v = &self.config.caps.version_string;
                        if !v.is_empty() {
                            ui.label(RichText::new(format!("Blender {}", v)).color(MUTED).small());
                        }
                    });
                });
            });

        TopBottomPanel::bottom("btm")
            .min_height(190.0)
            .frame(egui::Frame::none().fill(BG).stroke(Stroke::new(1.0, BORDER)))
            .show(ctx, |ui| {
                self.ui_bottom(ui);
            });

        CentralPanel::default()
            .frame(egui::Frame::none().fill(BG))
            .show(ctx, |ui| {
                let total_w = ui.available_width();
                let total_h = ui.available_height();
                let div_w = 6.0;
                let left_w = (total_w * self.config.split_ratio)
                    .max(180.0)
                    .min((total_w - 260.0).max(180.0));
                let right_w = (total_w - left_w - div_w).max(0.0);

                ui.horizontal(|ui| {
                    ui.set_height(total_h);

                    ui.vertical(|ui| {
                        ui.set_width(left_w);
                        self.ui_queue(ui);
                    });

                    let div_rect = ui.available_rect_before_wrap();
                    let div_rect = Rect::from_min_size(div_rect.min, Vec2::new(div_w, total_h));
                    let div_resp = ui.allocate_rect(div_rect, Sense::click_and_drag());
                    let div_color = if div_resp.dragged() || div_resp.hovered() {
                        PURPLE
                    } else {
                        BORDER
                    };
                    ui.painter().rect_filled(div_rect, 0.0, div_color);
                    if div_resp.hovered() || div_resp.dragged() {
                        ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
                    }
                    if div_resp.dragged() {
                        let delta = div_resp.drag_delta().x;
                        self.config.split_ratio =
                            ((left_w + delta) / total_w).clamp(0.15, 0.80);
                        self.dirty = true;
                    }

                    ui.vertical(|ui| {
                        ui.set_width(right_w);
                        self.ui_settings(ui);
                    });
                });
            });
    }
}

// ─── UI ───────────────────────────────────────────────────────────────────────
impl App {
    fn ui_setup(&mut self, ctx: &Context) {
        CentralPanel::default()
            .frame(egui::Frame::none().fill(BG))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.label(
                        RichText::new("QUBY RENDER QUEUE")
                            .font(FontId::proportional(30.0))
                            .color(TEXT)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("Setup — version {}", APP_VERSION))
                            .font(FontId::proportional(16.0))
                            .color(PURPLE_LT),
                    );
                    ui.add_space(28.0);
                    ui.label(RichText::new("Where is blender.exe located?").color(MUTED));
                    ui.add_space(10.0);

                    if let Some(p) = self.config.blender_path.clone() {
                        egui::Frame::none()
                            .fill(PANEL)
                            .stroke(Stroke::new(1.0, GREEN))
                            .inner_margin(Margin::symmetric(10.0, 6.0))
                            .rounding(4.0)
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(&p).color(GREEN).font(FontId::monospace(11.0)),
                                );
                            });
                        ui.add_space(10.0);
                    }

                    if btn(ui, "  Browse…  ", PURPLE, TEXT) {
                        let picked = rfd::FileDialog::new()
                            .add_filter("Blender", &["exe"])
                            .set_title("Select blender.exe")
                            .pick_file();
                        if let Some(p) = picked {
                            self.config.blender_path = Some(p.to_string_lossy().into());
                            self.config.caps = Caps::default();
                            self.save();
                            self.probe_caps();
                        }
                    }

                    ui.add_space(14.0);
                    let mut ae = self.config.auto_exec;
                    if ui
                        .checkbox(
                            &mut ae,
                            RichText::new("Allow Python auto-execution in .blend files (-y)")
                                .color(MUTED)
                                .small(),
                        )
                        .changed()
                    {
                        self.config.auto_exec = ae;
                        self.save();
                    }

                    if self.config.blender_path.is_some() {
                        ui.add_space(14.0);
                        if btn(ui, "  Continue →  ", GREEN_DK, Color32::BLACK) {
                            self.setup_open = false;
                        }
                        ui.add_space(8.0);
                        if btn_small(ui, "Re-detect Blender features", ITEM, MUTED) {
                            self.config.caps = Caps::default();
                            self.probe_caps();
                        }
                    }
                });
            });
    }

    fn ui_about(&mut self, ctx: &Context) {
        let mut open = true;
        Window::new("About Quby Render Queue")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("Version {}", APP_VERSION)).strong());
                ui.add_space(6.0);
                ui.label("Batch render queue for Blender 3.x, 4.x and 5.x.");
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Nothing is ever written back to your .blend files — every override \
                         is applied inside the background Blender session only.",
                    )
                    .color(MUTED)
                    .small(),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Tip: drag .blend files straight onto the window.")
                        .color(PURPLE_LT)
                        .small(),
                );
            });
        self.about_open = open;
    }

    fn ui_queue(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("FILE QUEUE").color(PURPLE_LT).strong().small());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                if !self.rendering && btn(ui, "+ Add", PURPLE, TEXT) {
                    self.add_files_dialog();
                }
                if btn_small(ui, "⚙", ITEM, MUTED) {
                    self.setup_open = true;
                }
            });
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if btn_small(ui, "Clear done", ITEM, MUTED) {
                self.queue.retain(|i| !matches!(i.status, Status::Done));
                self.selected = None;
                self.save();
            }
            if btn_small(ui, "Reset failed", ITEM, MUTED) {
                for i in &mut self.queue {
                    if matches!(i.status, Status::Failed(_)) {
                        i.status = Status::Pending;
                    }
                }
                self.save();
            }
            if btn_small(ui, "Reset all", ITEM, MUTED) {
                for i in &mut self.queue {
                    i.status = Status::Pending;
                }
                self.save();
            }
        });
        ui.add_space(4.0);

        let mut sel_idx: Option<usize> = None;
        let mut remove_idx: Option<usize> = None;
        let mut dup_idx: Option<usize> = None;
        let mut toggle_idx: Option<usize> = None;
        let mut swap: Option<(usize, usize)> = None;
        let len = self.queue.len();
        let rendering = self.rendering;
        let selected = self.selected;

        ScrollArea::vertical()
            .id_source("queue_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (idx, item) in self.queue.iter().enumerate() {
                    let is_sel = selected == Some(idx);
                    let bg = if is_sel { ITEM_SEL } else { ITEM };
                    let border = if is_sel { PURPLE } else { BORDER };

                    let (icon, icol) = match &item.status {
                        Status::Pending => ("○", MUTED),
                        Status::Rendering { .. } => ("◉", AMBER),
                        Status::Done => ("✓", GREEN),
                        Status::Failed(_) => ("✗", RED),
                    };
                    let sub = match &item.status {
                        Status::Rendering { frame } => {
                            format!("Fra {}/{}", frame, item.frame_end)
                        }
                        Status::Done if item.last_secs > 0.0 => fmt_secs(item.last_secs),
                        Status::Failed(m) => m.chars().take(40).collect::<String>(),
                        _ => match item.settings.job_mode {
                            JobMode::Animation => {
                                format!("{}-{}", item.frame_start, item.frame_end)
                            }
                            JobMode::Still => format!("still {}", item.settings.still_frame),
                        },
                    };

                    let name_col = if item.enabled { TEXT } else { MUTED };

                    let inner = egui::Frame::none()
                        .fill(bg)
                        .stroke(Stroke::new(1.0, border))
                        .inner_margin(Margin::symmetric(10.0, 8.0))
                        .rounding(5.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).color(icol));
                                ui.add_space(2.0);
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(&item.name)
                                            .color(name_col)
                                            .font(FontId::proportional(13.0)),
                                    );
                                    ui.label(
                                        RichText::new(&sub)
                                            .color(if matches!(item.status, Status::Failed(_)) {
                                                RED
                                            } else {
                                                MUTED
                                            })
                                            .font(FontId::monospace(9.0)),
                                    );
                                });
                                if !item.enabled {
                                    ui.with_layout(
                                        Layout::right_to_left(Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new("off").color(MUTED).small(),
                                            );
                                        },
                                    );
                                }
                            });
                        });

                    let resp = ui.interact(
                        inner.response.rect,
                        Id::new("qitem").with(idx),
                        Sense::click(),
                    );
                    if resp.clicked() {
                        sel_idx = Some(idx);
                    }
                    resp.context_menu(|ui| {
                        if ui
                            .button(if item.enabled { "Disable" } else { "Enable" })
                            .clicked()
                        {
                            toggle_idx = Some(idx);
                            ui.close_menu();
                        }
                        if !rendering {
                            if ui.button("⬆ Move up").clicked() {
                                if idx > 0 {
                                    swap = Some((idx - 1, idx));
                                }
                                ui.close_menu();
                            }
                            if ui.button("⬇ Move down").clicked() {
                                if idx + 1 < len {
                                    swap = Some((idx, idx + 1));
                                }
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Duplicate").clicked() {
                                dup_idx = Some(idx);
                                ui.close_menu();
                            }
                            if ui.button("🗑 Remove").clicked() {
                                remove_idx = Some(idx);
                                ui.close_menu();
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
            });

        if let Some(i) = sel_idx {
            self.selected = Some(i);
        }
        if let Some(i) = toggle_idx {
            self.queue[i].enabled = !self.queue[i].enabled;
            self.save();
        }
        if let Some(i) = dup_idx {
            let mut copy = self.queue[i].clone();
            copy.id = self.next_id;
            copy.status = Status::Pending;
            copy.last_secs = 0.0;
            self.next_id += 1;
            self.queue.insert(i + 1, copy);
            self.save();
        }
        if let Some(i) = remove_idx {
            self.queue.remove(i);
            if self.selected.map_or(false, |s| s >= self.queue.len()) {
                self.selected = if self.queue.is_empty() {
                    None
                } else {
                    Some(self.queue.len() - 1)
                };
            }
            self.save();
        }
        if let Some((a, b)) = swap {
            self.queue.swap(a, b);
            if self.selected == Some(a) {
                self.selected = Some(b);
            } else if self.selected == Some(b) {
                self.selected = Some(a);
            }
        }
    }

    fn ui_settings(&mut self, ui: &mut Ui) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("SETTINGS").color(PURPLE_LT).strong().small());
        });
        ui.add_space(6.0);

        let Some(idx) = self.selected else {
            ui.add_space(50.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("← Select a file").color(MUTED));
                ui.add_space(8.0);
                ui.label(
                    RichText::new("or drag .blend files onto this window")
                        .color(MUTED)
                        .small(),
                );
            });
            return;
        };
        if idx >= self.queue.len() {
            self.selected = None;
            return;
        }

        let caps = self.config.caps.clone();
        let rendering = self.rendering;
        let presets: Vec<String> = self.config.presets.iter().map(|p| p.0.clone()).collect();

        let item = self.queue[idx].clone();
        let mut s = item.settings.clone();
        let mut frame_start = item.frame_start;
        let mut frame_end = item.frame_end;
        let mut device = item.device;
        let mut output_path = item.output_path.clone();
        let mut enabled = item.enabled;
        let mut preset_name = self.preset_name.clone();

        // deferred actions that need &mut self
        let mut do_browse_out = false;
        let mut do_open_out = false;
        let mut do_apply_all = false;
        let mut do_save_preset = false;
        let mut do_load_preset: Option<usize> = None;
        let mut do_reprobe = false;

        ScrollArea::vertical()
            .id_source("settings_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());

                // ── file header ─────────────────────────────────────────────
                egui::Frame::none()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, PURPLE))
                    .inner_margin(Margin::symmetric(10.0, 8.0))
                    .rounding(6.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&item.name).color(TEXT).strong());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.checkbox(
                                    &mut enabled,
                                    RichText::new("in queue").color(MUTED).small(),
                                );
                            });
                        });
                        ui.label(
                            RichText::new(&item.path)
                                .color(MUTED)
                                .font(FontId::monospace(9.0)),
                        );
                        if item.source_fps > 0.0 {
                            ui.label(
                                RichText::new(format!(
                                    "source: {:.3} fps",
                                    item.source_fps
                                ))
                                .color(MUTED)
                                .small(),
                            );
                        }
                    });

                ui.add_space(6.0);

                // ── scene / device ──────────────────────────────────────────
                CollapsingHeader::new(RichText::new("Scene & device").color(PURPLE_LT))
                    .default_open(true)
                    .id_source("h_scene")
                    .show(ui, |ui| {
                        row(ui, "Scene", |ui| {
                            let mut opts = vec![String::new()];
                            opts.extend(item.scenes.clone());
                            combo(ui, "c_scene", &mut s.scene, &opts, 180.0, "Active scene");
                        });
                        row(ui, "Camera", |ui| {
                            let mut opts = vec![String::new()];
                            opts.extend(item.cameras.clone());
                            combo(ui, "c_cam", &mut s.camera, &opts, 180.0, "From scene");
                        });
                        row(ui, "View layer", |ui| {
                            let mut opts = vec![String::new()];
                            opts.extend(item.view_layers.clone());
                            combo(ui, "c_vl", &mut s.view_layer, &opts, 180.0, "All layers");
                        });
                        row(ui, "Engine", |ui| {
                            let mut opts = vec![String::new()];
                            opts.extend(listed(&caps.engines, D_ENGINES));
                            combo(ui, "c_eng", &mut s.engine, &opts, 180.0, "From file");
                        });
                        row(ui, "Cycles device", |ui| {
                            ComboBox::from_id_source("c_dev")
                                .selected_text(device.label())
                                .width(180.0)
                                .show_ui(ui, |ui| {
                                    for d in Device::ALL {
                                        ui.selectable_value(&mut device, d, d.label());
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            if btn_small(ui, "Re-read scene info", ITEM, MUTED) {
                                do_reprobe = true;
                            }
                        });
                    });

                // ── frames ──────────────────────────────────────────────────
                CollapsingHeader::new(RichText::new("Frames").color(PURPLE_LT))
                    .default_open(true)
                    .id_source("h_frames")
                    .show(ui, |ui| {
                        row(ui, "Mode", |ui| {
                            if radio_btn(ui, "Animation", s.job_mode == JobMode::Animation) {
                                s.job_mode = JobMode::Animation;
                            }
                            ui.add_space(4.0);
                            if radio_btn(ui, "Single image", s.job_mode == JobMode::Still) {
                                s.job_mode = JobMode::Still;
                            }
                        });
                        match s.job_mode {
                            JobMode::Animation => {
                                row(ui, "Range", |ui| {
                                    ui.label(RichText::new("start").color(MUTED).small());
                                    ui.add(
                                        DragValue::new(&mut frame_start)
                                            .speed(1.0)
                                            .clamp_range(0_i32..=1_000_000_i32),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("end").color(MUTED).small());
                                    ui.add(
                                        DragValue::new(&mut frame_end)
                                            .speed(1.0)
                                            .clamp_range(0_i32..=1_000_000_i32),
                                    );
                                });
                                row(ui, "Frame step", |ui| {
                                    ui.add(
                                        DragValue::new(&mut s.frame_step)
                                            .speed(1.0)
                                            .clamp_range(1_i32..=1000_i32),
                                    );
                                    ui.label(
                                        RichText::new("render every Nth frame")
                                            .color(MUTED)
                                            .small(),
                                    );
                                });
                            }
                            JobMode::Still => {
                                row(ui, "Frame", |ui| {
                                    ui.add(
                                        DragValue::new(&mut s.still_frame)
                                            .speed(1.0)
                                            .clamp_range(0_i32..=1_000_000_i32),
                                    );
                                });
                                ui.label(
                                    RichText::new(
                                        "  One image is written, named with the frame number.",
                                    )
                                    .color(MUTED)
                                    .small(),
                                );
                            }
                        }
                    });

                // ── output ──────────────────────────────────────────────────
                CollapsingHeader::new(RichText::new("Output & file format").color(PURPLE_LT))
                    .default_open(true)
                    .id_source("h_out")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            let avail = ui.available_width() - 150.0;
                            ui.add(
                                TextEdit::singleline(&mut output_path)
                                    .hint_text("empty = path saved in the .blend")
                                    .desired_width(avail.max(80.0))
                                    .font(FontId::monospace(10.0)),
                            );
                            if btn_small(ui, "Browse", ITEM, TEXT) {
                                do_browse_out = true;
                            }
                            if btn_small(ui, "Open", ITEM, TEXT) {
                                do_open_out = true;
                            }
                        });
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.override_format,
                                RichText::new("Override the file format").color(TEXT).small(),
                            );
                        });

                        if s.override_format {
                            row(ui, "File format", |ui| {
                                combo(
                                    ui,
                                    "c_ff",
                                    &mut s.file_format,
                                    &listed(&caps.file_formats, D_FILE_FORMATS),
                                    190.0,
                                    "",
                                );
                            });
                            row(ui, "Color", |ui| {
                                combo(
                                    ui,
                                    "c_cm",
                                    &mut s.color_mode,
                                    &listed(&caps.color_modes, D_COLOR_MODES),
                                    80.0,
                                    "",
                                );
                                ui.add_space(6.0);
                                ui.label(RichText::new("depth").color(MUTED).small());
                                combo(
                                    ui,
                                    "c_cd",
                                    &mut s.color_depth,
                                    &listed(&caps.color_depths, D_COLOR_DEPTHS),
                                    70.0,
                                    "",
                                );
                            });

                            match s.file_format.as_str() {
                                "PNG" => {
                                    row(ui, "Compression", |ui| {
                                        ui.add(
                                            Slider::new(&mut s.png_compression, 0..=100)
                                                .suffix(" %"),
                                        );
                                    });
                                    ui.label(
                                        RichText::new(
                                            "  PNG is lossless: compression only trades \
                                             file size against write speed.",
                                        )
                                        .color(MUTED)
                                        .small(),
                                    );
                                }
                                "JPEG" | "JPEG2000" | "WEBP" => {
                                    row(ui, "Quality", |ui| {
                                        ui.add(Slider::new(&mut s.quality, 0..=100).suffix(" %"));
                                    });
                                }
                                "OPEN_EXR" | "OPEN_EXR_MULTILAYER" => {
                                    row(ui, "EXR codec", |ui| {
                                        combo(
                                            ui,
                                            "c_exr",
                                            &mut s.exr_codec,
                                            &listed(&caps.exr_codecs, D_EXR_CODECS),
                                            140.0,
                                            "",
                                        );
                                    });
                                }
                                "TIFF" => {
                                    row(ui, "TIFF codec", |ui| {
                                        combo(
                                            ui,
                                            "c_tif",
                                            &mut s.tiff_codec,
                                            &listed(&caps.tiff_codecs, D_TIFF_CODECS),
                                            140.0,
                                            "",
                                        );
                                    });
                                }
                                "FFMPEG" => {
                                    row(ui, "Container", |ui| {
                                        combo(
                                            ui,
                                            "c_cont",
                                            &mut s.ff_container,
                                            &listed(&caps.ff_containers, D_FF_CONTAINERS),
                                            140.0,
                                            "",
                                        );
                                    });
                                    row(ui, "Video codec", |ui| {
                                        combo(
                                            ui,
                                            "c_vcod",
                                            &mut s.ff_video_codec,
                                            &listed(&caps.ff_video_codecs, D_FF_VIDEO),
                                            140.0,
                                            "",
                                        );
                                    });
                                    row(ui, "Audio codec", |ui| {
                                        combo(
                                            ui,
                                            "c_acod",
                                            &mut s.ff_audio_codec,
                                            &listed(&caps.ff_audio_codecs, D_FF_AUDIO),
                                            140.0,
                                            "",
                                        );
                                    });
                                    row(ui, "Quality", |ui| {
                                        ui.add_enabled_ui(!s.ff_use_custom_bitrate, |ui| {
                                            combo(
                                                ui,
                                                "c_crf",
                                                &mut s.ff_crf,
                                                &listed(&caps.ff_crf, D_FF_CRF),
                                                140.0,
                                                "",
                                            );
                                        });
                                    });
                                    row(ui, "Encoding speed", |ui| {
                                        combo(
                                            ui,
                                            "c_fpre",
                                            &mut s.ff_preset,
                                            &listed(&caps.ff_presets, D_FF_PRESET),
                                            140.0,
                                            "",
                                        );
                                    });
                                    row(ui, "Keyframe every", |ui| {
                                        ui.add(
                                            DragValue::new(&mut s.ff_gopsize)
                                                .speed(1.0)
                                                .clamp_range(0_i32..=500_i32),
                                        );
                                        ui.label(RichText::new("frames").color(MUTED).small());
                                    });
                                    row(ui, "Audio bitrate", |ui| {
                                        ui.add(
                                            DragValue::new(&mut s.ff_audio_bitrate)
                                                .speed(8.0)
                                                .clamp_range(32_i32..=384_i32)
                                                .suffix(" kb/s"),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.add_space(4.0);
                                        ui.checkbox(
                                            &mut s.ff_use_custom_bitrate,
                                            RichText::new("Use a fixed video bitrate instead")
                                                .color(TEXT)
                                                .small(),
                                        );
                                    });
                                    if s.ff_use_custom_bitrate {
                                        row(ui, "Video bitrate", |ui| {
                                            ui.add(
                                                DragValue::new(&mut s.ff_video_bitrate)
                                                    .speed(100.0)
                                                    .clamp_range(100_i32..=200_000_i32)
                                                    .suffix(" kb/s"),
                                            );
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.overwrite,
                                RichText::new("Overwrite existing frames").color(TEXT).small(),
                            );
                        });
                        if !s.overwrite {
                            ui.label(
                                RichText::new(
                                    "  Frames already on disk are skipped — \
                                     use this to resume an interrupted render.",
                                )
                                .color(GREEN)
                                .small(),
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.placeholders,
                                RichText::new("Create placeholder files (multi-machine renders)")
                                    .color(TEXT)
                                    .small(),
                            );
                        });
                        row(ui, "Burn metadata", |ui| {
                            tri(ui, "c_stamp", &mut s.use_stamp);
                        });
                    });

                // ── resolution ──────────────────────────────────────────────
                CollapsingHeader::new(RichText::new("Resolution").color(PURPLE_LT))
                    .id_source("h_res")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.override_resolution,
                                RichText::new("Override the resolution").color(TEXT).small(),
                            );
                        });
                        if s.override_resolution {
                            ui.add_space(2.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(4.0);
                                for (label, w, h) in RES_PRESETS {
                                    let on = s.res_x == *w && s.res_y == *h;
                                    if radio_btn(ui, *label, on) {
                                        s.res_x = *w;
                                        s.res_y = *h;
                                    }
                                }
                            });
                            ui.add_space(2.0);
                            row(ui, "Custom", |ui| {
                                ui.add(
                                    DragValue::new(&mut s.res_x)
                                        .speed(1.0)
                                        .clamp_range(4_i32..=65536_i32),
                                );
                                ui.label(RichText::new("×").color(MUTED));
                                ui.add(
                                    DragValue::new(&mut s.res_y)
                                        .speed(1.0)
                                        .clamp_range(4_i32..=65536_i32),
                                );
                            });
                            row(ui, "Scale", |ui| {
                                ui.add(Slider::new(&mut s.res_pct, 1..=100).suffix(" %"));
                            });
                            ui.label(
                                RichText::new(format!(
                                    "  Final image: {} × {} px",
                                    s.res_x * s.res_pct / 100,
                                    s.res_y * s.res_pct / 100
                                ))
                                .color(MUTED)
                                .small(),
                            );
                        }
                    });

                // ── colour management ───────────────────────────────────────
                CollapsingHeader::new(RichText::new("Color management").color(PURPLE_LT))
                    .id_source("h_col")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.override_color,
                                RichText::new("Override color management").color(TEXT).small(),
                            );
                        });
                        if s.override_color {
                            row(ui, "View transform", |ui| {
                                combo(
                                    ui,
                                    "c_vt",
                                    &mut s.view_transform,
                                    &listed(&caps.view_transforms, D_VIEW_TRANSFORMS),
                                    190.0,
                                    "",
                                );
                            });
                            row(ui, "Look", |ui| {
                                combo(
                                    ui,
                                    "c_look",
                                    &mut s.look,
                                    &listed(&caps.looks, D_LOOKS),
                                    190.0,
                                    "",
                                );
                            });
                            row(ui, "Display device", |ui| {
                                combo(
                                    ui,
                                    "c_disp",
                                    &mut s.display_device,
                                    &listed(&caps.display_devices, D_DISPLAY),
                                    190.0,
                                    "",
                                );
                            });
                            row(ui, "Exposure", |ui| {
                                ui.add(
                                    DragValue::new(&mut s.exposure)
                                        .speed(0.05)
                                        .clamp_range(-10.0_f32..=10.0_f32),
                                );
                                ui.add_space(8.0);
                                ui.label(RichText::new("gamma").color(MUTED).small());
                                ui.add(
                                    DragValue::new(&mut s.gamma)
                                        .speed(0.01)
                                        .clamp_range(0.0_f32..=5.0_f32),
                                );
                            });
                            row(ui, "Output space", |ui| {
                                let mut opts = vec![String::new()];
                                opts.extend(listed(&caps.colorspaces, D_COLORSPACES));
                                combo(
                                    ui,
                                    "c_ocs",
                                    &mut s.output_colorspace,
                                    &opts,
                                    190.0,
                                    "From format",
                                );
                            });
                            row(ui, "Sequencer space", |ui| {
                                let mut opts = vec![String::new()];
                                opts.extend(listed(&caps.colorspaces, D_COLORSPACES));
                                combo(
                                    ui,
                                    "c_scs",
                                    &mut s.sequencer_colorspace,
                                    &opts,
                                    190.0,
                                    "From file",
                                );
                            });
                        }
                    });

                // ── fps ─────────────────────────────────────────────────────
                CollapsingHeader::new(RichText::new("Frame rate & retiming").color(PURPLE_LT))
                    .id_source("h_fps")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.override_fps,
                                RichText::new("Override the frame rate").color(TEXT).small(),
                            );
                        });
                        if s.override_fps {
                            ui.add_space(2.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(4.0);
                                for f in FPS_PRESETS {
                                    let on = (s.fps - *f).abs() < 0.001;
                                    let label = if (*f - f.round()).abs() < 0.001 {
                                        format!("{}", *f as i32)
                                    } else {
                                        format!("{:.3}", f)
                                    };
                                    if radio_btn(ui, &label, on) {
                                        s.fps = *f;
                                    }
                                }
                            });
                            row(ui, "Custom fps", |ui| {
                                ui.add(
                                    DragValue::new(&mut s.fps)
                                        .speed(0.1)
                                        .clamp_range(0.1_f32..=1000.0_f32),
                                );
                            });
                            ui.add_space(4.0);
                            row(ui, "Keep timing", |ui| {
                                ComboBox::from_id_source("c_retime")
                                    .selected_text(match s.retime {
                                        RetimeMode::Off => "No — just change fps",
                                        RetimeMode::Remap => "Yes — Blender time remap",
                                        RetimeMode::Keys => "Yes — rescale keyframes",
                                    })
                                    .width(230.0)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut s.retime,
                                            RetimeMode::Remap,
                                            "Yes — Blender time remap",
                                        );
                                        ui.selectable_value(
                                            &mut s.retime,
                                            RetimeMode::Keys,
                                            "Yes — rescale keyframes",
                                        );
                                        ui.selectable_value(
                                            &mut s.retime,
                                            RetimeMode::Off,
                                            "No — just change fps",
                                        );
                                    });
                            });
                            let src = if item.source_fps > 0.0 {
                                item.source_fps
                            } else {
                                24.0
                            };
                            let k = s.fps / src;
                            match s.retime {
                                RetimeMode::Off => {
                                    ui.label(
                                        RichText::new(
                                            "  ⚠ Animation will play back faster or slower.",
                                        )
                                        .color(AMBER)
                                        .small(),
                                    );
                                }
                                _ => {
                                    ui.label(
                                        RichText::new(format!(
                                            "  {:.3} → {:.3} fps: every animation is stretched \
                                             ×{:.4}, so a {}-frame shot becomes {} frames and \
                                             looks exactly the same.",
                                            src,
                                            s.fps,
                                            k,
                                            frame_end - frame_start + 1,
                                            ((frame_end - frame_start + 1) as f32 * k).round()
                                                as i32
                                        ))
                                        .color(GREEN)
                                        .small(),
                                    );
                                }
                            }
                            if s.retime == RetimeMode::Keys {
                                ui.label(
                                    RichText::new(
                                        "  Keyframe rescaling touches f-curves, NLA strips, \
                                         grease pencil, markers and physics caches — inside \
                                         the render session only, your .blend is never saved.",
                                    )
                                    .color(MUTED)
                                    .small(),
                                );
                            }
                            if s.retime != RetimeMode::Off {
                                ui.horizontal(|ui| {
                                    ui.add_space(4.0);
                                    ui.checkbox(
                                        &mut s.retime_vse,
                                        RichText::new("Also retime video-editor strips")
                                            .color(TEXT)
                                            .small(),
                                    );
                                });
                            }
                        }
                    });

                // ── post processing ─────────────────────────────────────────
                CollapsingHeader::new(
                    RichText::new("Compositor & video editor").color(PURPLE_LT),
                )
                .id_source("h_post")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.checkbox(
                            &mut s.override_post,
                            RichText::new("Override post-processing").color(TEXT).small(),
                        );
                    });
                    if s.override_post {
                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(4.0);
                            if radio_btn(ui, "Both", s.use_compositing && s.use_sequencer) {
                                s.use_compositing = true;
                                s.use_sequencer = true;
                            }
                            if radio_btn(
                                ui,
                                "Compositor only",
                                s.use_compositing && !s.use_sequencer,
                            ) {
                                s.use_compositing = true;
                                s.use_sequencer = false;
                            }
                            if radio_btn(
                                ui,
                                "Video editor only",
                                !s.use_compositing && s.use_sequencer,
                            ) {
                                s.use_compositing = false;
                                s.use_sequencer = true;
                            }
                            if radio_btn(ui, "Neither", !s.use_compositing && !s.use_sequencer) {
                                s.use_compositing = false;
                                s.use_sequencer = false;
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.skip_3d,
                                RichText::new("Skip the 3D render entirely").color(TEXT).small(),
                            );
                        });
                        if s.skip_3d {
                            ui.label(
                                RichText::new(
                                    "  All view layers are disabled, so only the compositor / \
                                     video editor runs. Use this when your node tree or timeline \
                                     reads image and movie files rather than Render Layers.",
                                )
                                .color(AMBER)
                                .small(),
                            );
                        }
                    }
                });

                // ── sampling & performance ──────────────────────────────────
                CollapsingHeader::new(RichText::new("Sampling & performance").color(PURPLE_LT))
                    .id_source("h_q")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.override_quality,
                                RichText::new("Override sampling").color(TEXT).small(),
                            );
                        });
                        if s.override_quality {
                            row(ui, "Cycles samples", |ui| {
                                ui.add(
                                    DragValue::new(&mut s.samples)
                                        .speed(1.0)
                                        .clamp_range(1_i32..=100_000_i32),
                                );
                            });
                            row(ui, "Noise threshold", |ui| {
                                ui.add(
                                    DragValue::new(&mut s.adaptive_threshold)
                                        .speed(0.001)
                                        .clamp_range(0.0_f32..=1.0_f32),
                                );
                            });
                            row(ui, "EEVEE samples", |ui| {
                                ui.add(
                                    DragValue::new(&mut s.eevee_samples)
                                        .speed(1.0)
                                        .clamp_range(1_i32..=10_000_i32),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                ui.checkbox(
                                    &mut s.use_denoise,
                                    RichText::new("Denoise").color(TEXT).small(),
                                );
                            });
                            if s.use_denoise {
                                row(ui, "Denoiser", |ui| {
                                    combo(
                                        ui,
                                        "c_den",
                                        &mut s.denoiser,
                                        &listed(&caps.denoisers, D_DENOISERS),
                                        190.0,
                                        "",
                                    );
                                });
                            }
                            row(ui, "Motion blur", |ui| {
                                tri(ui, "c_mb", &mut s.motion_blur);
                            });
                            row(ui, "Transparent film", |ui| {
                                tri(ui, "c_ft", &mut s.film_transparent);
                            });
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut s.persistent_data,
                                RichText::new("Persistent data (faster animations, more RAM)")
                                    .color(TEXT)
                                    .small(),
                            );
                        });
                        row(ui, "CPU threads", |ui| {
                            ui.add(
                                DragValue::new(&mut s.threads)
                                    .speed(1.0)
                                    .clamp_range(0_i32..=256_i32),
                            );
                            ui.label(RichText::new("0 = automatic").color(MUTED).small());
                        });
                        row(ui, "Retries on failure", |ui| {
                            ui.add(
                                DragValue::new(&mut s.max_retries)
                                    .speed(1.0)
                                    .clamp_range(0_i32..=5_i32),
                            );
                        });
                    });

                // ── presets ─────────────────────────────────────────────────
                CollapsingHeader::new(RichText::new("Presets").color(PURPLE_LT))
                    .id_source("h_pre")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.add(
                                TextEdit::singleline(&mut preset_name)
                                    .hint_text("preset name")
                                    .desired_width(150.0),
                            );
                            if btn_small(ui, "Save", ITEM, TEXT) {
                                do_save_preset = true;
                            }
                        });
                        ui.add_space(4.0);
                        for (i, name) in presets.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                if btn_small(ui, name, ITEM, TEXT) {
                                    do_load_preset = Some(i);
                                }
                            });
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.add_enabled_ui(!rendering, |ui| {
                                if btn_small(ui, "Apply these settings to every file", PURPLE, TEXT)
                                {
                                    do_apply_all = true;
                                }
                            });
                        });
                    });

                ui.add_space(10.0);
            });

        // ── write everything back ────────────────────────────────────────────
        {
            let it = &mut self.queue[idx];
            let mut changed = false;
            if it.settings != s {
                it.settings = s.clone();
                changed = true;
            }
            if it.frame_start != frame_start {
                it.frame_start = frame_start;
                changed = true;
            }
            if it.frame_end != frame_end {
                it.frame_end = frame_end;
                changed = true;
            }
            if it.device != device {
                it.device = device;
                changed = true;
            }
            if it.output_path != output_path {
                it.output_path = output_path;
                changed = true;
            }
            if it.enabled != enabled {
                it.enabled = enabled;
                changed = true;
            }
            if changed {
                self.dirty = true;
            }
        }
        self.preset_name = preset_name;

        // ── deferred actions ────────────────────────────────────────────────
        if do_browse_out {
            let picked = rfd::FileDialog::new().set_title("Output folder").pick_folder();
            if let Some(p) = picked {
                let mut st = p.to_string_lossy().to_string();
                if !st.ends_with('/') && !st.ends_with('\\') {
                    st.push('\\');
                }
                self.queue[idx].output_path = st;
                self.save();
            }
        }
        if do_open_out {
            self.open_output(idx);
        }
        if do_reprobe {
            let id = self.queue[idx].id;
            let path = PathBuf::from(self.queue[idx].path.clone());
            self.probe_blend(id, path);
        }
        if do_save_preset {
            let name = if self.preset_name.trim().is_empty() {
                format!("Preset {}", self.config.presets.len() + 1)
            } else {
                self.preset_name.trim().to_string()
            };
            let set = self.queue[idx].settings.clone();
            if let Some(slot) = self.config.presets.iter_mut().find(|p| p.0 == name) {
                slot.1 = set;
            } else {
                self.config.presets.push((name.clone(), set));
            }
            self.preset_name.clear();
            self.logline(format!("  Preset '{}' saved.", name));
            self.save();
        }
        if let Some(i) = do_load_preset {
            if let Some((name, set)) = self.config.presets.get(i).cloned() {
                self.queue[idx].settings = set;
                let target = self.queue[idx].name.clone();
                self.logline(format!("  Preset '{}' applied to {}.", name, target));
                self.save();
            }
        }
        if do_apply_all {
            let set = self.queue[idx].settings.clone();
            let dev = self.queue[idx].device;
            for it in &mut self.queue {
                let still = it.settings.still_frame;
                let scene = it.settings.scene.clone();
                let cam = it.settings.camera.clone();
                let vl = it.settings.view_layer.clone();
                it.settings = set.clone();
                // per-file identity stays with the file
                it.settings.still_frame = still;
                it.settings.scene = scene;
                it.settings.camera = cam;
                it.settings.view_layer = vl;
                it.device = dev;
            }
            self.logline("  Settings copied to every file in the queue.");
            self.save();
        }
    }

    fn ui_bottom(&mut self, ui: &mut Ui) {
        let done = self.queue.iter().filter(|i| i.status == Status::Done).count();
        let total = self.queue.len();
        let rendering_state = self.queue.iter().find_map(|i| {
            if let Status::Rendering { frame } = i.status {
                Some((frame, i.frame_start, i.frame_end, i.name.clone()))
            } else {
                None
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Files {}/{}", done, total)).color(MUTED).small());
            let fp = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.add(ProgressBar::new(fp).desired_width(120.0).fill(PURPLE).show_percentage());

            if let Some((cur, fs, fe, _)) = &rendering_state {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("Frame {}/{}", cur, fe)).color(MUTED).small());
                let span = (fe - fs).max(1) as f32;
                let ffp = ((cur - fs) as f32 / span).clamp(0.0, 1.0);
                ui.add(
                    ProgressBar::new(ffp).desired_width(120.0).fill(AMBER).show_percentage(),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                if self.rendering {
                    if big_btn(ui, "⏹  STOP", RED) {
                        self.stop();
                    }
                } else if big_btn(ui, "▶  RENDER", GREEN_DK) {
                    self.start();
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("When finished").color(MUTED).small());
                    let mut pa = self.config.post_action;
                    ComboBox::from_id_source("c_post")
                        .selected_text(pa.label())
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for a in [
                                PostAction::Nothing,
                                PostAction::Sleep,
                                PostAction::Hibernate,
                                PostAction::Shutdown,
                            ] {
                                ui.selectable_value(&mut pa, a, a.label());
                            }
                        });
                    if pa != self.config.post_action {
                        self.config.post_action = pa;
                        self.dirty = true;
                    }
                });
            });
        });

        // timing line
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if let Some((_, _, _, name)) = &rendering_state {
                let el = self
                    .job_started
                    .map(|t| fmt_secs(t.elapsed().as_secs_f64()))
                    .unwrap_or_else(|| "—".into());
                let mut line = format!("Rendering {}   ·   elapsed {}", name, el);
                if !self.cur_remaining.is_empty() {
                    line.push_str(&format!("   ·   frame ETA {}", self.cur_remaining));
                }
                if !self.cur_elapsed.is_empty() {
                    line.push_str(&format!("   ·   frame time {}", self.cur_elapsed));
                }
                ui.label(RichText::new(line).color(AMBER).small());
            } else if let Some(t) = self.queue_started {
                ui.label(
                    RichText::new(format!("Queue running · {}", fmt_secs(t.elapsed().as_secs_f64())))
                        .color(MUTED)
                        .small(),
                );
            } else {
                ui.label(RichText::new("Idle").color(MUTED).small());
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                if btn_small(ui, "Save log", ITEM, MUTED) {
                    let picked = rfd::FileDialog::new()
                        .set_title("Save log")
                        .set_file_name("quby_render_log.txt")
                        .save_file();
                    if let Some(p) = picked {
                        let _ = std::fs::write(p, self.log.join("\r\n"));
                    }
                }
                if btn_small(ui, "Clear log", ITEM, MUTED) {
                    self.log.clear();
                }
                let mut compact = self.config.compact_log;
                if ui
                    .checkbox(&mut compact, RichText::new("Compact log").color(MUTED).small())
                    .changed()
                {
                    self.config.compact_log = compact;
                    self.dirty = true;
                }
            });
        });

        ui.add_space(4.0);

        egui::Frame::none()
            .fill(LOG_BG)
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(Margin::symmetric(8.0, 5.0))
            .rounding(4.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ScrollArea::vertical()
                    .id_source("log_scroll")
                    .stick_to_bottom(true)
                    .max_height(100.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.log {
                            let col = if line.starts_with('✓') || line.starts_with('★') {
                                GREEN
                            } else if line.starts_with('▶') || line.starts_with("  Added") {
                                AMBER
                            } else if line.starts_with('✗')
                                || line.contains("[!]")
                                || line.starts_with("QRQ-WARN")
                            {
                                RED
                            } else if line.starts_with("QRQ:") {
                                PURPLE_LT
                            } else {
                                MUTED
                            };
                            ui.label(
                                RichText::new(line).color(col).font(FontId::monospace(10.0)),
                            );
                        }
                    });
            });
    }
}

// ─── Small UI helpers ─────────────────────────────────────────────────────────
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.add_sized(
            [118.0, 18.0],
            egui::Label::new(RichText::new(label).color(MUTED).small()),
        );
        add(ui);
    });
}

fn combo(ui: &mut Ui, id: &str, current: &mut String, options: &[String], width: f32, empty: &str) {
    let shown = if current.is_empty() {
        if empty.is_empty() {
            "(none)".to_string()
        } else {
            empty.to_string()
        }
    } else {
        current.clone()
    };
    ComboBox::from_id_source(id)
        .selected_text(shown)
        .width(width)
        .show_ui(ui, |ui| {
            for o in options {
                let label = if o.is_empty() {
                    if empty.is_empty() {
                        "(none)".to_string()
                    } else {
                        empty.to_string()
                    }
                } else {
                    o.clone()
                };
                if ui.selectable_label(current.as_str() == o.as_str(), label).clicked() {
                    *current = o.clone();
                }
            }
        });
}

fn tri(ui: &mut Ui, id: &str, v: &mut TriState) {
    ComboBox::from_id_source(id)
        .selected_text(v.label())
        .width(110.0)
        .show_ui(ui, |ui| {
            for t in [TriState::Keep, TriState::On, TriState::Off] {
                ui.selectable_value(v, t, t.label());
            }
        });
}

fn btn(ui: &mut Ui, label: &str, bg: Color32, fg: Color32) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).color(fg))
            .fill(bg)
            .stroke(Stroke::new(1.0, PURPLE)),
    )
    .clicked()
}

fn btn_small(ui: &mut Ui, label: &str, bg: Color32, fg: Color32) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).color(fg).small())
            .fill(bg)
            .stroke(Stroke::new(1.0, BORDER))
            .min_size(Vec2::new(20.0, 18.0)),
    )
    .clicked()
}

fn big_btn(ui: &mut Ui, label: &str, bg: Color32) -> bool {
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .color(Color32::WHITE)
                .strong()
                .font(FontId::proportional(14.0)),
        )
        .fill(bg)
        .stroke(Stroke::new(2.0, Color32::WHITE))
        .min_size(Vec2::new(120.0, 34.0)),
    )
    .clicked()
}

fn radio_btn(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let (bg, fg) = if selected { (PURPLE, TEXT) } else { (ITEM, MUTED) };
    ui.add(
        egui::Button::new(RichText::new(label).color(fg).small())
            .fill(bg)
            .stroke(Stroke::new(1.5, PURPLE)),
    )
    .clicked()
}

fn setup_theme(ctx: &Context) {
    let mut v = Visuals::dark();
    v.window_fill = BG;
    v.panel_fill = BG;
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, PURPLE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.bg_fill = ITEM;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, PURPLE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = Color32::from_rgb(35, 48, 120);
    v.widgets.hovered.bg_stroke = Stroke::new(1.5, PURPLE_LT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.active.bg_fill = Color32::from_rgb(60, 30, 140);
    v.widgets.active.bg_stroke = Stroke::new(2.0, PURPLE_LT);
    v.selection.bg_fill = Color32::from_rgb(70, 30, 160);
    v.selection.stroke = Stroke::new(1.0, PURPLE_LT);
    v.override_text_color = Some(TEXT);
    ctx.set_visuals(v);
}

// ─── Plain helpers ────────────────────────────────────────────────────────────
fn parse_frame(line: &str) -> Option<i32> {
    line.find("Fra:").and_then(|p| {
        let s = &line[p + 4..];
        let e = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        s[..e].parse().ok()
    })
}

/// Returns the token that follows `key`, stopping at whitespace or `|`.
fn token_after(line: &str, key: &str) -> Option<String> {
    let p = line.find(key)?;
    let rest = &line[p + key.len()..];
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '|')
        .unwrap_or(rest.len());
    let tok = rest[..end].trim();
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

fn fmt_secs(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{}h {:02}m {:02}s", h, m, sec)
    } else if m > 0 {
        format!("{}m {:02}s", m, sec)
    } else {
        format!("{}s", sec)
    }
}

fn app_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
            .join("BlenderRenderQueue")
    }
    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".blender-render-queue")
    }
}

fn apply_script_path() -> PathBuf {
    app_dir().join("qrq_apply.py")
}

fn corridorkey_script_path() -> PathBuf {
    app_dir().join("qrq_corridorkey.py")
}

fn probe_script_path() -> PathBuf {
    app_dir().join("qrq_probe.py")
}

/// Write the two Blender helper scripts to AppData on every launch so they
/// always match the running version.
fn write_scripts() -> bool {
    let dir = app_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    std::fs::write(apply_script_path(), APPLY_PY).is_ok()
        && std::fs::write(probe_script_path(), PROBE_PY).is_ok()
        && std::fs::write(corridorkey_script_path(), CORRIDORKEY_PY).is_ok()
}

fn load_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Option<T> {
    load_json(path)
}

fn save_json<T: Serialize>(path: &PathBuf, val: &T) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Ok(s) = serde_json::to_string_pretty(val) {
        let _ = std::fs::write(path, s);
    }
}

fn kill_pid(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn open_folder(path: &std::path::Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer").arg(path).spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

fn run_post_action(action: PostAction) {
    #[cfg(target_os = "windows")]
    {
        match action {
            PostAction::Nothing => {}
            PostAction::Shutdown => {
                let _ = Command::new("shutdown").args(["/s", "/t", "60"]).spawn();
            }
            PostAction::Hibernate => {
                let _ = Command::new("shutdown").args(["/h"]).spawn();
            }
            PostAction::Sleep => {
                let _ = Command::new("rundll32.exe")
                    .args(["powrprof.dll,SetSuspendState", "0,1,0"])
                    .spawn();
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        match action {
            PostAction::Nothing => {}
            PostAction::Shutdown => {
                let _ = Command::new("shutdown").args(["-h", "+1"]).spawn();
            }
            PostAction::Hibernate => {
                let _ = Command::new("systemctl").arg("hibernate").spawn();
            }
            PostAction::Sleep => {
                let _ = Command::new("systemctl").arg("suspend").spawn();
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Quby Render Queue",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_title(format!("Quby Render Queue {}", APP_VERSION))
                .with_icon(
                    eframe::icon_data::from_png_bytes(include_bytes!("../icon.png"))
                        .expect("embedded icon.png is valid"),
                )
                .with_inner_size([1040.0, 740.0])
                .with_min_inner_size([760.0, 560.0])
                .with_drag_and_drop(true),
            ..Default::default()
        },
        Box::new(|cc| Box::new(App::new(cc))),
    )
}
