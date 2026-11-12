//! Speed and quality benchmark against LAME and ffmpeg (as external
//! binaries on `PATH`, if present).
//!
//! ```text
//! cargo run --release --example bench -- [--json] [--seconds 60] [--runs 5]
//!     [--no-reference] [--corpus DIR_OF_WAVS]
//! ```
//!
//! Prints a Markdown table (or JSON with `--json`). The corpus is generated
//! in code. See `docs/benchmarks.md` for what each column means and its
//! limits. LAME and ffmpeg are only ever run as separate programs.

use std::error::Error;
use std::f64::consts::PI;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use craft_mp3::analysis::{lowpass_hz, noise_to_mask_db};
use craft_mp3::{ENCODER_DELAY, Encoder, EncoderConfig};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[path = "bench/metrics.rs"]
#[allow(dead_code)]
pub mod metrics;

type Res<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Cbr(u32),
    Vbr(u8),
}

#[derive(Clone, Copy)]
struct Setting {
    rate: u32,
    channels: u16,
    mode: Mode,
}

impl Setting {
    fn name(&self) -> String {
        let m = match self.mode {
            Mode::Cbr(k) => format!("CBR {k}"),
            Mode::Vbr(q) => format!("VBR {q}"),
        };
        let ch = if self.channels == 1 { "mono" } else { "stereo" };
        format!("{m} {} kHz {ch}", f64::from(self.rate) / 1000.0)
    }

    fn config(&self) -> EncoderConfig {
        match self.mode {
            Mode::Cbr(k) => EncoderConfig::new(self.rate, self.channels, k),
            Mode::Vbr(q) => EncoderConfig::vbr(self.rate, self.channels, q),
        }
    }

    /// Arguments for `lame` (before the input and output paths).
    fn lame_args(&self) -> Vec<String> {
        match self.mode {
            Mode::Cbr(k) => vec!["-b".into(), k.to_string()],
            Mode::Vbr(q) => vec!["-V".into(), q.to_string()],
        }
    }

    /// Arguments for `ffmpeg` after the input.
    fn ffmpeg_args(&self) -> Vec<String> {
        let rate_mode = match self.mode {
            Mode::Cbr(k) => ["-b:a".to_string(), format!("{k}k")],
            Mode::Vbr(q) => ["-q:a".to_string(), q.to_string()],
        };
        let mut v = vec!["-c:a".to_string(), "libmp3lame".to_string()];
        v.extend(rate_mode);
        v
    }
}

const SETTINGS: [Setting; 7] = [
    Setting {
        rate: 44_100,
        channels: 2,
        mode: Mode::Cbr(128),
    },
    Setting {
        rate: 44_100,
        channels: 2,
        mode: Mode::Cbr(320),
    },
    Setting {
        rate: 44_100,
        channels: 2,
        mode: Mode::Vbr(0),
    },
    Setting {
        rate: 44_100,
        channels: 2,
        mode: Mode::Vbr(2),
    },
    Setting {
        rate: 44_100,
        channels: 2,
        mode: Mode::Vbr(5),
    },
    Setting {
        rate: 48_000,
        channels: 2,
        mode: Mode::Cbr(192),
    },
    Setting {
        rate: 22_050,
        channels: 1,
        mode: Mode::Cbr(64),
    },
];

// ---------------------------------------------------------------- corpus

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }
}

/// One channel of the named signal, `n` samples at `rate`.
fn generate(kind: &str, rate: u32, n: usize, seed: u64) -> Vec<f32> {
    let r = f64::from(rate);
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    match kind {
        "pink noise" => {
            // Paul Kellet's economy pink filter.
            let (mut b0, mut b1, mut b2) = (0.0, 0.0, 0.0);
            (0..n)
                .map(|_| {
                    let w = rng.next();
                    b0 = 0.99765 * b0 + w * 0.099_046;
                    b1 = 0.963 * b1 + w * 0.296_516_4;
                    b2 = 0.57 * b2 + w * 1.052_691_3;
                    ((b0 + b1 + b2 + w * 0.1848) * 0.12) as f32
                })
                .collect()
        }
        "sine sweep" => {
            // Logarithmic, 20 Hz up to 20 kHz (or 95 % of Nyquist if lower).
            let (f0, f1) = (20.0, 20_000f64.min(r / 2.0 * 0.95));
            let t = n as f64 / r;
            let k = (f1 / f0).ln();
            (0..n)
                .map(|i| {
                    let x = i as f64 / r;
                    (0.5 * (2.0 * PI * f0 * t / k * ((x / t * k).exp() - 1.0)).sin()) as f32
                })
                .collect()
        }
        "tones and gaps" => {
            // Seven tones; the whole sum is silent for 0.5 s of every 2 s.
            let tones = [220.0, 440.0, 880.0, 1760.0, 3520.0, 7040.0, 12_000.0];
            let phase = seed as f64 * 0.7;
            (0..n)
                .map(|i| {
                    let x = i as f64 / r;
                    if x % 2.0 >= 1.5 {
                        return 0.0;
                    }
                    let s: f64 = tones
                        .iter()
                        .filter(|&&f| f < r / 2.0 * 0.9)
                        .map(|f| (2.0 * PI * f * x + phase).sin())
                        .sum();
                    (s * 0.06) as f32
                })
                .collect()
        }
        _ => {
            // Impulse train: a decaying click every 150 ms over faint noise.
            let period = (r * 0.15) as usize;
            (0..n)
                .map(|i| {
                    let since = (i % period) as f64;
                    let click = if since < 200.0 {
                        0.8 * (-since / 30.0).exp() * rng_sign(i)
                    } else {
                        0.0
                    };
                    (click + rng.next() * 0.003) as f32
                })
                .collect()
        }
    }
}

fn rng_sign(i: usize) -> f64 {
    if i & 1 == 0 { 1.0 } else { -1.0 }
}

const KINDS: [&str; 4] = [
    "pink noise",
    "sine sweep",
    "tones and gaps",
    "impulse train",
];

struct Clip {
    name: String,
    rate: u32,
    channels: u16,
    /// Interleaved 16-bit samples.
    pcm: Vec<i16>,
}

impl Clip {
    fn frames(&self) -> usize {
        self.pcm.len() / usize::from(self.channels)
    }
    fn seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.rate)
    }
    fn channel(&self, c: usize) -> Vec<f32> {
        self.pcm
            .iter()
            .skip(c)
            .step_by(usize::from(self.channels))
            .map(|&s| f32::from(s) / 32768.0)
            .collect()
    }
}

fn make_clip(kind: &str, rate: u32, channels: u16, seconds: f64) -> Clip {
    let n = (f64::from(rate) * seconds) as usize;
    let chans: Vec<Vec<f32>> = (0..channels)
        .map(|c| generate(kind, rate, n, u64::from(c) + 1))
        .collect();
    let mut pcm = Vec::with_capacity(n * usize::from(channels));
    for i in 0..n {
        for ch in &chans {
            pcm.push((ch[i] * 32767.0).round() as i16);
        }
    }
    Clip {
        name: kind.to_string(),
        rate,
        channels,
        pcm,
    }
}

fn wav_bytes(clip: &Clip) -> Vec<u8> {
    let data = clip.pcm.len() * 2;
    let mut w = Vec::with_capacity(44 + data);
    w.extend(b"RIFF");
    w.extend((36 + data as u32).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(clip.channels.to_le_bytes());
    w.extend(clip.rate.to_le_bytes());
    w.extend((clip.rate * u32::from(clip.channels) * 2).to_le_bytes());
    w.extend((clip.channels * 2).to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend((data as u32).to_le_bytes());
    for s in &clip.pcm {
        w.extend(s.to_le_bytes());
    }
    w
}

fn read_wav(path: &Path) -> Res<Clip> {
    let b = std::fs::read(path)?;
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let (mut pos, mut fmt, mut pcm) = (12, None, None);
    while pos + 8 <= b.len() {
        let size = u32::from_le_bytes(b[pos + 4..pos + 8].try_into()?) as usize;
        let body = &b[pos + 8..(pos + 8 + size).min(b.len())];
        match &b[pos..pos + 4] {
            b"fmt " if body.len() >= 16 => {
                let tag = u16::from_le_bytes(body[0..2].try_into()?);
                let bits = u16::from_le_bytes(body[14..16].try_into()?);
                if tag != 1 || bits != 16 {
                    return Err("only 16-bit PCM WAV is supported".into());
                }
                fmt = Some((
                    u16::from_le_bytes(body[2..4].try_into()?),
                    u32::from_le_bytes(body[4..8].try_into()?),
                ));
            }
            b"data" => {
                pcm = Some(
                    body.as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| i16::from_le_bytes(*c))
                        .collect::<Vec<_>>(),
                )
            }
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    let ((channels, rate), pcm) = fmt.zip(pcm).ok_or("missing fmt or data chunk")?;
    let name = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    Ok(Clip {
        name,
        rate,
        channels,
        pcm,
    })
}

// -------------------------------------------------------- encode / decode

fn encode_craft(clip: &Clip, setting: &Setting) -> Res<Vec<u8>> {
    let mut encoder = Encoder::new(setting.config())?;
    let mut mp3 = Vec::new();
    encoder.encode_i16(&clip.pcm, &mut mp3);
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }
    Ok(mp3)
}

/// Interleaved f32 samples from any MP3 stream, decoded by symphonia.
fn decode(mp3: Vec<u8>) -> Res<(Vec<f32>, usize)> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(mp3)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe().format(
        &hint,
        source,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let (mut out, mut channels) = (Vec::new(), 1);
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        let decoded = decoder.decode(&packet)?;
        channels = decoded.spec().channels.count();
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buf.copy_interleaved_ref(decoded);
        out.extend_from_slice(buf.samples());
    }
    Ok((out, channels))
}

/// The LAME-tag encoder delay plus the decoder's 529, if the stream has one
/// (after any ID3v2 tag), otherwise LAME's usual 576 + 529.
fn stated_delay(mp3: &[u8]) -> usize {
    let head = &mp3[..mp3.len().min(4096)];
    for key in [b"Xing", b"Info"] {
        if let Some(p) = head.windows(4).position(|w| w == key)
            && let Some(f) = head.get(p + 120 + 21..p + 120 + 24)
        {
            return (usize::from(f[0]) << 4 | usize::from(f[1]) >> 4) + 529;
        }
    }
    576 + 529
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

// ------------------------------------------------------------------- rows

#[derive(Default, Clone)]
struct Quality {
    snr: Option<f64>,
    nmr: f64,
    lsd: f64,
    bandwidth_khz: f64,
    delay: usize,
}

struct Row {
    clip: String,
    setting: String,
    encoder: &'static str,
    /// `None` when the tool is not installed or failed.
    seconds_per_minute: Option<f64>,
    kbps: f64,
    quality: Option<Quality>,
    note: String,
}

fn quality(clip: &Clip, mp3: Vec<u8>, stated: usize, cutoff_hz: f64) -> Res<Quality> {
    let (dec, channels) = decode(mp3)?;
    if channels != usize::from(clip.channels) {
        return Err("decoded channel count differs".into());
    }
    let chans = usize::from(clip.channels);
    let (mut q, mut snr_n) = (Quality::default(), 0.0);
    let mut snr_sum = 0.0;
    for c in 0..chans {
        let orig = clip.channel(c);
        let d: Vec<f32> = dec.iter().skip(c).step_by(chans).copied().collect();
        let lag = metrics::find_delay(&orig, &d, stated, 16);
        let d = metrics::align(&d, lag, orig.len());
        if let Some(s) = metrics::segmental_snr_db(&orig, &d, clip.rate) {
            snr_sum += s;
            snr_n += 1.0;
        }
        q.nmr += noise_to_mask_db(clip.rate, &orig, &d) / chans as f64;
        q.lsd += metrics::log_spectral_distance_db(&orig, &d, clip.rate, cutoff_hz) / chans as f64;
        q.bandwidth_khz +=
            metrics::effective_bandwidth_hz(&orig, &d, clip.rate) / 1000.0 / chans as f64;
        q.delay = lag;
    }
    q.snr = (snr_n > 0.0).then(|| snr_sum / snr_n);
    Ok(q)
}

fn run_craft(clip: &Clip, setting: &Setting, runs: usize) -> Res<Row> {
    let mut times = Vec::new();
    let mut mp3 = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        mp3 = encode_craft(clip, setting)?;
        times.push(t.elapsed().as_secs_f64());
    }
    let kbps = mp3.len() as f64 * 8.0 / clip.seconds() / 1000.0;
    let cutoff = f64::from(lowpass_hz(&setting.config()));
    let q = quality(clip, mp3, ENCODER_DELAY, cutoff)?;
    Ok(Row {
        clip: clip.name.clone(),
        setting: setting.name(),
        encoder: "craft-mp3",
        seconds_per_minute: Some(median(times) * 60.0 / clip.seconds()),
        kbps,
        quality: Some(q),
        note: String::new(),
    })
}

fn tool_found(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn run_reference(
    tool: &'static str,
    found: bool,
    clip: &Clip,
    setting: &Setting,
    wav: &Path,
    dir: &Path,
    runs: usize,
) -> Row {
    let mut row = Row {
        clip: clip.name.clone(),
        setting: setting.name(),
        encoder: tool,
        seconds_per_minute: None,
        kbps: 0.0,
        quality: None,
        note: "not found".into(),
    };
    if !found {
        return row;
    }
    let out = dir.join(format!("{tool}.mp3"));
    let mut times = Vec::new();
    for _ in 0..runs {
        let mut cmd = Command::new(tool);
        if tool == "lame" {
            cmd.arg("--silent")
                .args(setting.lame_args())
                .arg(wav)
                .arg(&out);
        } else {
            cmd.args(["-v", "error", "-y", "-threads", "1", "-i"])
                .arg(wav)
                .args(setting.ffmpeg_args())
                .arg(&out);
        }
        let t = Instant::now();
        let ok = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
        if !matches!(ok, Ok(s) if s.success()) {
            row.note = "failed".into();
            return row;
        }
        times.push(t.elapsed().as_secs_f64());
    }
    row.note = String::new();
    row.seconds_per_minute = Some(median(times) * 60.0 / clip.seconds());
    let done = std::fs::read(&out).inspect(|mp3| {
        row.kbps = mp3.len() as f64 * 8.0 / clip.seconds() / 1000.0;
    });
    match done.map_err(Into::into).and_then(|mp3| {
        let stated = stated_delay(&mp3);
        let cutoff = f64::from(lowpass_hz(&setting.config()));
        quality(clip, mp3, stated, cutoff)
    }) {
        Ok(q) => row.quality = Some(q),
        Err(e) => row.note = format!("quality failed: {e}"),
    }
    row
}

// ----------------------------------------------------------------- output

fn machine() -> String {
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    format!(
        "{} {}, {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        rustc
    )
}

fn version_line(tool: &str) -> String {
    Command::new(tool)
        .arg(if tool == "lame" {
            "--version"
        } else {
            "-version"
        })
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "not found".into())
}

const HEADER: [&str; 10] = [
    "file",
    "setting",
    "encoder",
    "s per min",
    "x realtime",
    "kbit/s",
    "seg SNR dB",
    "NMR dB",
    "LSD dB",
    "bandwidth kHz",
];

fn cells(r: &Row) -> Vec<String> {
    let mut c = vec![r.clip.clone(), r.setting.clone(), r.encoder.to_string()];
    match r.seconds_per_minute {
        Some(s) => {
            c.push(format!("{s:.3}"));
            c.push(format!("{:.1}", 60.0 / s));
        }
        None => {
            let note = if r.note.is_empty() {
                "not found"
            } else {
                &r.note
            };
            c.push(note.to_string());
            c.push("-".into());
        }
    }
    match &r.quality {
        Some(q) => {
            c.push(format!("{:.1}", r.kbps));
            c.push(q.snr.map_or("-".into(), |s| format!("{s:.1}")));
            c.push(format!("{:.1}", q.nmr));
            c.push(format!("{:.1}", q.lsd));
            c.push(format!("{:.1}", q.bandwidth_khz));
        }
        None => c.extend(std::iter::repeat_n("-".to_string(), 5)),
    }
    c
}

fn markdown(rows: &[Row]) -> String {
    let mut s = format!(
        "| {} |\n|{}\n",
        HEADER.join(" | "),
        "---|".repeat(HEADER.len())
    );
    for r in rows {
        s += &format!("| {} |\n", cells(r).join(" | "));
    }
    s
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn json(rows: &[Row], meta: &str) -> String {
    let num = |v: Option<f64>| v.map_or("null".to_string(), |x| format!("{x:.4}"));
    let items: Vec<String> = rows
        .iter()
        .map(|r| {
            let q = r.quality.as_ref();
            format!(
                "{{\"file\":{},\"setting\":{},\"encoder\":{},\"seconds_per_minute\":{},\
                 \"realtime_factor\":{},\"kbps\":{},\"segmental_snr_db\":{},\"nmr_db\":{},\
                 \"lsd_db\":{},\"bandwidth_khz\":{},\"delay\":{},\"note\":{}}}",
                json_str(&r.clip),
                json_str(&r.setting),
                json_str(r.encoder),
                num(r.seconds_per_minute),
                num(r.seconds_per_minute.map(|s| 60.0 / s)),
                num(q.map(|_| r.kbps)),
                num(q.and_then(|q| q.snr)),
                num(q.map(|q| q.nmr)),
                num(q.map(|q| q.lsd)),
                num(q.map(|q| q.bandwidth_khz)),
                q.map_or("null".to_string(), |q| q.delay.to_string()),
                json_str(&r.note),
            )
        })
        .collect();
    format!(
        "{{\"machine\":{},\"rows\":[{}]}}\n",
        json_str(meta),
        items.join(",\n")
    )
}

// ------------------------------------------------------------------- main

/// Runs the whole benchmark for `args` (without the program name) and
/// returns the report text.
pub fn run(args: &[String]) -> Res<String> {
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let seconds: f64 = value("--seconds").map_or(Ok(60.0), |v| v.parse())?;
    let runs: usize = value("--runs").map_or(Ok(5), |v| v.parse())?;
    let references = !flag("--no-reference");
    let (lame, ffmpeg) = (
        references && tool_found("lame"),
        references && tool_found("ffmpeg"),
    );

    // Pairs of (clip, setting).
    let mut cases: Vec<(Clip, Setting)> = Vec::new();
    for setting in SETTINGS {
        for kind in KINDS {
            cases.push((
                make_clip(kind, setting.rate, setting.channels, seconds),
                setting,
            ));
        }
    }
    if let Some(dir) = value("--corpus") {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")))
            .collect();
        paths.sort();
        for p in paths {
            let clip = read_wav(&p)?;
            let kbps = if clip.rate < 32_000 { 64 } else { 128 };
            for mode in [Mode::Cbr(kbps), Mode::Vbr(2)] {
                let setting = Setting {
                    rate: clip.rate,
                    channels: clip.channels,
                    mode,
                };
                let copy = Clip {
                    name: clip.name.clone(),
                    pcm: clip.pcm.clone(),
                    ..clip
                };
                cases.push((copy, setting));
            }
        }
    }

    // One folder per run, not per process: the smoke tests call `run` from
    // parallel threads of one process, and a shared folder let one run delete
    // the other's input (NotFound on Linux and Windows CI).
    static RUN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let run_id = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("craft-mp3-bench-{}-{run_id}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let mut rows = Vec::new();
    let mut last_wav: Option<(String, u32, PathBuf)> = None;
    for (clip, setting) in &cases {
        let key = format!("{}-{}-{}", clip.name, clip.rate, clip.channels);
        let wav = match &last_wav {
            Some((k, _, p)) if *k == key => p.clone(),
            _ => {
                let p = dir.join("input.wav");
                std::fs::write(&p, wav_bytes(clip))?;
                last_wav = Some((key, clip.rate, p.clone()));
                p
            }
        };
        eprintln!("{} / {}", clip.name, setting.name());
        rows.push(run_craft(clip, setting, runs)?);
        rows.push(run_reference("lame", lame, clip, setting, &wav, &dir, runs));
        rows.push(run_reference(
            "ffmpeg", ffmpeg, clip, setting, &wav, &dir, runs,
        ));
    }
    let _ = std::fs::remove_dir_all(&dir);

    let meta = machine();
    if flag("--json") {
        return Ok(json(&rows, &meta));
    }
    let mut out = format!(
        "Machine: {meta}\n\nlame: {}\n\nffmpeg: {}\n\nClip length {seconds} s, median of {runs} runs.\n\n",
        if lame {
            version_line("lame")
        } else {
            "not found".into()
        },
        if ffmpeg {
            version_line("ffmpeg")
        } else {
            "not found".into()
        },
    );
    out += &markdown(&rows);
    Ok(out)
}

#[allow(dead_code)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => print!("{text}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
