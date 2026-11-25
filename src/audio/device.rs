use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const TARGET_SAMPLE_RATE: u32 = 48_000;
const TARGET_CHANNELS: usize = 2;
const RING_CAPACITY_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize) * TARGET_CHANNELS;

#[allow(dead_code)]
pub struct AudioChannels {
    pub output_producer: Option<HeapProd<f32>>,
    pub input_consumer: Option<HeapCons<f32>>,
    pub output_sample_rate: u32,
    pub output_channels: u16,
    pub input_sample_rate: u32,
    pub input_channels: u16,
}

/// Converts arbitrary interleaved `f32` PCM (`src_rate`, `src_channels`) to interleaved
/// 48kHz stereo (`f32`) suitable for WebRTC Opus frames.
#[allow(dead_code)]
pub fn resample_to_48k_stereo(samples: &[f32], src_rate: u32, src_channels: u16) -> Vec<f32> {
    if samples.is_empty() || src_rate == 0 || src_channels == 0 {
        return Vec::new();
    }
    let ch = src_channels as usize;
    if src_rate == TARGET_SAMPLE_RATE && ch == TARGET_CHANNELS {
        return samples.to_vec();
    }
    let src_frames = samples.len() / ch;
    if src_frames == 0 {
        return Vec::new();
    }
    let dst_frames = ((src_frames as u64 * TARGET_SAMPLE_RATE as u64) / src_rate as u64).max(1) as usize;
    let mut out = Vec::with_capacity(dst_frames * TARGET_CHANNELS);

    for i in 0..dst_frames {
        let pos = if dst_frames > 1 && src_frames > 1 {
            (i as f64) * (src_rate as f64) / (TARGET_SAMPLE_RATE as f64)
        } else {
            0.0
        };
        let idx0 = (pos.floor() as usize).min(src_frames - 1);
        let idx1 = (idx0 + 1).min(src_frames - 1);
        let frac = (pos - idx0 as f64) as f32;

        let l0 = samples[idx0 * ch];
        let l1 = samples[idx1 * ch];
        let r0 = if ch >= 2 { samples[idx0 * ch + 1] } else { l0 };
        let r1 = if ch >= 2 { samples[idx1 * ch + 1] } else { l1 };

        out.push((l0 + (l1 - l0) * frac).clamp(-1.0, 1.0));
        out.push((r0 + (r1 - r0) * frac).clamp(-1.0, 1.0));
    }
    out
}

pub struct AudioEngine {
    output_stream: Option<Stream>,
    input_stream: Option<Stream>,
    is_muted: Arc<AtomicBool>,
    is_deafened: Arc<AtomicBool>,
}

impl AudioEngine {
    /// Initializes controller state lazily without opening any WASAPI hardware streams.
    pub fn new_idle() -> Self {
        Self {
            output_stream: None,
            input_stream: None,
            is_muted: Arc::new(AtomicBool::new(false)),
            is_deafened: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Kept for callers wanting eager device startup.
    #[allow(dead_code)]
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut engine = Self::new_idle();
        let _ = engine.start_streams()?;
        Ok(engine)
    }

    pub fn is_active(&self) -> bool {
        self.output_stream.is_some() || self.input_stream.is_some()
    }

    pub fn start_streams(&mut self) -> Result<AudioChannels, Box<dyn std::error::Error + Send + Sync>> {
        self.stop_streams();

        let host = cpal::default_host();
        log::info!("Audio Host: {:?}", host.id());

        let mut out_stream: Option<Stream> = None;
        let mut output_producer = None;
        let mut output_sample_rate = TARGET_SAMPLE_RATE;
        let mut output_channels = TARGET_CHANNELS as u16;

        if let Some(device) = host.default_output_device() {
            log::info!("Default Output Device: {:?}", device.name().unwrap_or_default());
            let config: StreamConfig = device.default_output_config()?.into();
            output_sample_rate = config.sample_rate.0;
            output_channels = config.channels;
            log::info!(
                "Output config: sample_rate={}, channels={}",
                output_sample_rate,
                output_channels
            );

            let rb = HeapRb::<f32>::new(RING_CAPACITY_SAMPLES);
            let (prod, mut cons) = rb.split();
            output_producer = Some(prod);

            let is_deaf = self.is_deafened.clone();
            let dst_rate = output_sample_rate.max(1);
            let dst_ch = (output_channels as usize).max(1);
            let step = (TARGET_SAMPLE_RATE as f64) / (dst_rate as f64);
            let mut phase: f64 = 0.0;
            let mut cur_lr = [0.0_f32; 2];

            let stream = device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if is_deaf.load(Ordering::Relaxed) {
                        data.fill(0.0);
                        while cons.try_pop().is_some() {}
                        return;
                    }

                    if dst_rate == TARGET_SAMPLE_RATE && dst_ch == TARGET_CHANNELS {
                        for pair in data.chunks_exact_mut(2) {
                            let mut stereo = [0.0_f32; 2];
                            if cons.pop_slice(&mut stereo) == 2 {
                                pair[0] = stereo[0];
                                pair[1] = stereo[1];
                            } else {
                                pair[0] = 0.0;
                                pair[1] = 0.0;
                            }
                        }
                        return;
                    }

                    for frame in data.chunks_mut(dst_ch) {
                        while phase >= 1.0 {
                            let mut stereo = [0.0_f32; 2];
                            if cons.pop_slice(&mut stereo) == 2 {
                                cur_lr = stereo;
                            } else {
                                cur_lr = [0.0, 0.0];
                            }
                            phase -= 1.0;
                        }
                        if dst_ch == 1 {
                            frame[0] = 0.5 * (cur_lr[0] + cur_lr[1]);
                        } else {
                            frame[0] = cur_lr[0];
                            frame[1] = cur_lr[1];
                            for ch in frame.iter_mut().skip(2) {
                                *ch = 0.0;
                            }
                        }
                        phase += step;
                    }
                },
                |err| log::error!("Audio output stream error: {:?}", err),
                None,
            )?;
            stream.play()?;
            out_stream = Some(stream);
        } else {
            log::warn!("No audio output device found!");
        }

        let mut in_stream: Option<Stream> = None;
        let mut input_consumer = None;
        let mut input_sample_rate = TARGET_SAMPLE_RATE;
        let mut input_channels = TARGET_CHANNELS as u16;

        if let Some(device) = host.default_input_device() {
            log::info!("Default Input Device: {:?}", device.name().unwrap_or_default());
            let config: StreamConfig = device.default_input_config()?.into();
            input_sample_rate = config.sample_rate.0;
            input_channels = config.channels;
            log::info!(
                "Input config: sample_rate={}, channels={}",
                input_sample_rate,
                input_channels
            );

            let rb = HeapRb::<f32>::new(RING_CAPACITY_SAMPLES);
            let (mut prod, cons) = rb.split();
            input_consumer = Some(cons);

            let is_mute = self.is_muted.clone();
            let src_rate = input_sample_rate.max(1);
            let src_ch = input_channels.max(1);
            let mut phase: f64 = 0.0;
            let step = (src_rate as f64) / (TARGET_SAMPLE_RATE as f64);

            let stream = device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if is_mute.load(Ordering::Relaxed) {
                        return;
                    }

                    if src_rate == TARGET_SAMPLE_RATE && src_ch == TARGET_CHANNELS as u16 {
                        for pair in data.chunks_exact(2) {
                            let _ = prod.push_slice(pair);
                        }
                        return;
                    }

                    // Zero-alloc real-time linear sample-rate/channel conversion to atomic stereo pairs
                    let ch = src_ch as usize;
                    let frames = data.len() / ch;
                    if frames == 0 {
                        return;
                    }
                    while (phase as usize) < frames {
                        let idx0 = (phase.floor() as usize).min(frames - 1);
                        let idx1 = (idx0 + 1).min(frames - 1);
                        let frac = (phase - idx0 as f64) as f32;
                        let l0 = data[idx0 * ch];
                        let l1 = data[idx1 * ch];
                        let r0 = if ch >= 2 { data[idx0 * ch + 1] } else { l0 };
                        let r1 = if ch >= 2 { data[idx1 * ch + 1] } else { l1 };
                        let stereo = [
                            (l0 + (l1 - l0) * frac).clamp(-1.0, 1.0),
                            (r0 + (r1 - r0) * frac).clamp(-1.0, 1.0),
                        ];
                        let _ = prod.push_slice(&stereo);
                        phase += step;
                    }
                    phase -= frames as f64;
                },
                |err| log::error!("Audio input stream error: {:?}", err),
                None,
            )?;
            stream.play()?;
            in_stream = Some(stream);
        } else {
            log::warn!("No audio input device found!");
        }

        // Commit both streams atomically so any setup failure drops prior local streams cleanly
        self.output_stream = out_stream;
        self.input_stream = in_stream;

        Ok(AudioChannels {
            output_producer,
            input_consumer,
            output_sample_rate,
            output_channels,
            input_sample_rate,
            input_channels,
        })
    }

    pub fn stop_streams(&mut self) {
        let was_active = self.is_active();
        self.output_stream = None;
        self.input_stream = None;
        if was_active {
            log::info!("WASAPI audio streams stopped (0% idle CPU)");
        }
    }

    pub fn controller(&self) -> AudioController {
        AudioController {
            is_muted: self.is_muted.clone(),
            is_deafened: self.is_deafened.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AudioController {
    is_muted: Arc<AtomicBool>,
    is_deafened: Arc<AtomicBool>,
}

impl AudioController {
    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::SeqCst);
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.is_deafened.store(deafened, Ordering::SeqCst);
    }
}
