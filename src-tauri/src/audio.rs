use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const TARGET_RATE: u32 = 16_000;
const MAX_RECORD_SECS: u64 = 300; // 5-minute safety auto-stop
const LEVEL_INTERVAL_MS: u64 = 60;

/// Handle to a running recording thread.
pub struct RecorderHandle {
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Vec<i16>, String>>,
}

impl RecorderHandle {
    /// Stop recording and take the captured samples (16kHz mono i16).
    pub fn stop_and_take(self) -> Result<Vec<i16>, String> {
        self.stop.store(true, Ordering::SeqCst);
        self.rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "録音の停止に失敗しました(タイムアウト)".to_string())?
    }

    /// Stop recording and discard the result.
    pub fn cancel(self) {
        self.stop.store(true, Ordering::SeqCst);
        // The thread will finish and the result is dropped with self.rx.
    }
}

impl Drop for RecorderHandle {
    /// Any dropped/overwritten handle must terminate its capture thread so a
    /// recorder can never be orphaned recording into the void.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Start recording from the default input device.
/// Emits `level` events (~60ms) with `{ rms: 0.0..1.0 }` while recording.
pub fn start(app: AppHandle) -> Result<RecorderHandle, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (result_tx, result_rx) = mpsc::channel::<Result<Vec<i16>, String>>();
    let stop2 = stop.clone();

    std::thread::spawn(move || {
        let result = record_thread(app, stop2, ready_tx);
        let _ = result_tx.send(result);
    });

    match ready_rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(())) => Ok(RecorderHandle {
            stop,
            rx: result_rx,
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Signal the (still initializing) capture thread to exit so it
            // does not keep recording after we report the timeout.
            stop.store(true, Ordering::SeqCst);
            Err("マイクの初期化がタイムアウトしました".to_string())
        }
    }
}

fn record_thread(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) -> Result<Vec<i16>, String> {
    let fail = |e: String, tx: &mpsc::Sender<Result<(), String>>| -> String {
        let _ = tx.send(Err(e.clone()));
        e
    };

    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            return Err(fail(
                "録音デバイス(マイク)が見つかりません。マイクを接続してください".to_string(),
                &ready_tx,
            ))
        }
    };
    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            return Err(fail(
                format!("マイク設定の取得に失敗しました: {e}"),
                &ready_tx,
            ))
        }
    };

    // If the caller already gave up (init timeout), exit before capturing.
    if stop.load(Ordering::SeqCst) {
        let msg = "録音は開始前に中断されました".to_string();
        return Err(fail(msg.clone(), &ready_tx));
    }

    let sample_rate: u32 = supported.sample_rate();
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    // mono samples at the device's native rate
    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    // most recent RMS level of the incoming audio
    let level: Arc<Mutex<f32>> = Arc::new(Mutex::new(0.0));
    let max_samples = (sample_rate as u64 * MAX_RECORD_SECS) as usize;

    let stream = {
        let build = |fmt: cpal::SampleFormat| -> Result<cpal::Stream, String> {
            match fmt {
                cpal::SampleFormat::F32 => build_stream::<f32>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                cpal::SampleFormat::I16 => build_stream::<i16>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                cpal::SampleFormat::U16 => build_stream::<u16>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                cpal::SampleFormat::I8 => build_stream::<i8>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                cpal::SampleFormat::U8 => build_stream::<u8>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                cpal::SampleFormat::I32 => build_stream::<i32>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                cpal::SampleFormat::U32 => build_stream::<u32>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                cpal::SampleFormat::F64 => build_stream::<f64>(
                    &device, &config, channels, max_samples, samples.clone(), level.clone(),
                ),
                other => Err(format!("未対応のサンプル形式です: {other:?}")),
            }
        };
        match build(sample_format) {
            Ok(s) => s,
            Err(e) => return Err(fail(e, &ready_tx)),
        }
    };

    if let Err(e) = stream.play() {
        return Err(fail(format!("録音の開始に失敗しました: {e}"), &ready_tx));
    }

    let _ = ready_tx.send(Ok(()));

    let started = Instant::now();
    while !stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(MAX_RECORD_SECS)
    {
        std::thread::sleep(Duration::from_millis(LEVEL_INTERVAL_MS));
        let rms = *level.lock().unwrap();
        // amplify a bit so normal speech is visible on the HUD, clamp to 0..1
        let display = (rms * 4.0).min(1.0);
        let _ = app.emit("level", serde_json::json!({ "rms": display }));
    }

    drop(stream);

    let mono = samples.lock().unwrap().clone();
    Ok(resample_to_16k_i16(&mono, sample_rate))
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    max_samples: usize,
    samples: Arc<Mutex<Vec<f32>>>,
    level: Arc<Mutex<f32>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = channels.max(1);
    device
        .build_input_stream(
            config.clone(),
            move |data: &[T], _info: &cpal::InputCallbackInfo| {
                let mut buf = samples.lock().unwrap();
                let mut sum_sq = 0f32;
                let mut n = 0usize;
                for frame in data.chunks(channels) {
                    let mut acc = 0f32;
                    for s in frame {
                        acc += f32::from_sample(*s);
                    }
                    let mono = acc / frame.len() as f32;
                    if buf.len() < max_samples {
                        buf.push(mono);
                    }
                    sum_sq += mono * mono;
                    n += 1;
                }
                drop(buf);
                if n > 0 {
                    *level.lock().unwrap() = (sum_sq / n as f32).sqrt();
                }
            },
            |_err| {
                // stream errors end the recording silently; the pipeline will
                // surface an empty-audio error if nothing was captured
            },
            None,
        )
        .map_err(|e| format!("録音ストリームの作成に失敗しました: {e}"))
}

fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// Linear-resample mono f32 samples from `src_rate` down/up to 16kHz i16.
fn resample_to_16k_i16(input: &[f32], src_rate: u32) -> Vec<i16> {
    if input.is_empty() {
        return Vec::new();
    }
    if src_rate == TARGET_RATE {
        return input.iter().map(|s| f32_to_i16(*s)).collect();
    }
    let ratio = src_rate as f64 / TARGET_RATE as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let i0 = pos as usize;
        let frac = (pos - i0 as f64) as f32;
        let s0 = input[i0];
        let s1 = if i0 + 1 < input.len() {
            input[i0 + 1]
        } else {
            s0
        };
        out.push(f32_to_i16(s0 + (s1 - s0) * frac));
    }
    out
}

/// Encode 16kHz mono i16 samples as an in-memory WAV file.
pub fn wav_bytes(samples: &[i16]) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .map_err(|e| format!("WAVの生成に失敗しました: {e}"))?;
        for s in samples {
            writer
                .write_sample(*s)
                .map_err(|e| format!("WAVの書き込みに失敗しました: {e}"))?;
        }
        writer
            .finalize()
            .map_err(|e| format!("WAVの生成に失敗しました: {e}"))?;
    }
    Ok(cursor.into_inner())
}
