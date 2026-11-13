//! Plays the game's audio on the chosen output device, at the volume
//! Settings ▸ Sound sets.
//!
//! The emulator thread pushes each frame's samples into a short shared queue,
//! resampled to the device rate; the device callback drains it. Above 1x the
//! game makes sound faster than the device plays it, so the oldest samples are
//! dropped to keep the delay short.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pactlocke_emu::AudioSink;

/// How much audio may wait in the queue before the oldest is dropped.
const MAX_QUEUE_SECONDS: f32 = 0.1;

type Queue = Arc<Mutex<VecDeque<[f32; 2]>>>;

/// The volume every stream plays at, 0 to 1, as an `f32`'s bits; 0 mutes.
pub type Gain = Arc<AtomicU32>;

/// A gain at `volume`.
pub fn gain(volume: f32) -> Gain {
    Arc::new(AtomicU32::new(volume.to_bits()))
}

/// Sets a gain to `volume`.
pub fn set_gain(gain: &Gain, volume: f32) {
    gain.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// The output devices by name, for Settings ▸ Sound.
pub fn output_devices() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|devices| {
            devices
                .filter_map(|d| d.description().ok().map(|desc| desc.name().to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// The open output stream. Dropping it stops the sound.
pub struct AudioOut {
    _stream: cpal::Stream,
    queue: Queue,
    device_rate: u32,
}

impl AudioOut {
    /// Opens the output device named `device` (the default one when it is
    /// `None` or gone), or returns `None` when there is none (the game then
    /// runs silently). Every sample is scaled by `gain`.
    pub fn open(device: Option<&str>, gain: Gain) -> Option<Self> {
        let host = cpal::default_host();
        let named = device.and_then(|name| {
            host.output_devices()
                .ok()?
                .find(|d| d.description().is_ok_and(|desc| desc.name() == name))
        });
        let device = named.or_else(|| host.default_output_device())?;
        let supported = device.default_output_config().ok()?;
        let config = supported.config();
        let channels = usize::from(config.channels);
        let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
        let on_error = |e| eprintln!("audio stream error: {e}");

        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                let (queue, gain) = (queue.clone(), gain.clone());
                device.build_output_stream(
                    config,
                    move |out: &mut [f32], _| fill(out, channels, &queue, &gain, |s| s),
                    on_error,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let (queue, gain) = (queue.clone(), gain.clone());
                device.build_output_stream(
                    config,
                    move |out: &mut [i16], _| fill(out, channels, &queue, &gain, |s| (s * f32::from(i16::MAX)) as i16),
                    on_error,
                    None,
                )
            }
            other => {
                eprintln!("audio: unsupported sample format {other:?}, running silently");
                return None;
            }
        }
        .ok()?;
        stream.play().ok()?;
        Some(Self {
            _stream: stream,
            queue,
            device_rate: config.sample_rate,
        })
    }

    /// A sink for the emulator thread that feeds this stream.
    pub fn sink(&self) -> QueueSink {
        QueueSink {
            queue: self.queue.clone(),
            device_rate: self.device_rate,
            position: 0.0,
            last: [0.0; 2],
        }
    }
}

fn fill<T: Copy + Default>(out: &mut [T], channels: usize, queue: &Queue, gain: &Gain, convert: impl Fn(f32) -> T) {
    let volume = f32::from_bits(gain.load(Ordering::Relaxed));
    let mut queue = queue.lock().unwrap_or_else(|e| e.into_inner());
    for frame in out.chunks_mut(channels) {
        let [l, r] = queue.pop_front().unwrap_or([0.0; 2]);
        let (l, r) = (l * volume, r * volume);
        for (i, sample) in frame.iter_mut().enumerate() {
            *sample = match i {
                0 => convert(l),
                1 => convert(r),
                _ => T::default(),
            };
        }
    }
}

/// Resamples the game's stereo audio to the device rate by linear interpolation
/// and queues it for the device.
pub struct QueueSink {
    queue: Queue,
    device_rate: u32,
    /// Where the next output sample falls, in input samples from `last`.
    position: f64,
    /// The final input sample of the previous push, to interpolate across pushes.
    last: [f32; 2],
}

impl AudioSink for QueueSink {
    fn push(&mut self, samples: &[i16], sample_rate: u32) {
        if samples.is_empty() || sample_rate == 0 {
            return;
        }
        let input: Vec<[f32; 2]> = samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[l, r]| [f32::from(*l) / 32768.0, f32::from(*r) / 32768.0])
            .collect();
        if input.is_empty() {
            return;
        }
        let step = f64::from(sample_rate) / f64::from(self.device_rate);
        let mut resampled = Vec::with_capacity((input.len() as f64 / step) as usize + 1);
        // Sample k of this push sits at k + 1, with `last` at 0.
        while self.position <= input.len() as f64 {
            let i = self.position.floor() as usize;
            let t = (self.position - i as f64) as f32;
            let a = if i == 0 { self.last } else { input[i - 1] };
            let b = input.get(i).copied().unwrap_or(a);
            resampled.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            self.position += step;
        }
        self.position -= input.len() as f64;
        self.last = *input.last().unwrap();

        let max = (MAX_QUEUE_SECONDS * self.device_rate as f32) as usize;
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.extend(resampled);
        let excess = queue.len().saturating_sub(max);
        queue.drain(..excess);
    }
}
