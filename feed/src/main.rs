use hound::WavReader;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::env;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 4 {
        eprintln!("Usage: {} <COM_PORT> <WAV_FILE_PATH> <VOLUME>", args[0]);
        std::process::exit(1);
    }

    let com_port = &args[1];
    let file_path = &args[2];
    let volume: f32 = args[3].parse().expect("Failed to parse volume");

    let mut port = serialport::new(com_port, 115200)
        .timeout(Duration::from_millis(1000))
        .open()
        .expect("Failed to open serial port");

    println!("Opened serial port {} at 115200 baud", com_port);

    let mut reader = WavReader::open(file_path).expect("Failed to open WAV file");
    let spec = reader.spec();

    if spec.channels != 2 || spec.sample_rate != 48000 || spec.bits_per_sample != 16 {
        eprintln!("Unsupported WAV file. Only 48000Hz, stereo, 16-bit PCM is supported.");
        return;
    }

    let original_samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();

    // real audio freq is lower than 48k (46.875k)
    // resample from 48k to 46.875k to compensate for decreased pitch
    let total_frames = original_samples.len() / 2;
    let mut left = Vec::with_capacity(total_frames);
    let mut right = Vec::with_capacity(total_frames);

    for i in (0..original_samples.len()).step_by(2) {
        left.push(original_samples[i] as f32 / 32768.0);
        right.push(original_samples[i + 1] as f32 / 32768.0);
    }

    let input = vec![left, right];
    let mut resampler = SincFixedIn::<f32>::new(
        46875.0 / 48000.0,
        1.0,
        SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        },
        total_frames,
        2,
    )
    .unwrap();

    let output = resampler.process(&input, None).unwrap();

    let mut samples = Vec::with_capacity(output[0].len() * 2);
    for frame in 0..output[0].len() {
        samples.push((output[0][frame] * 32767.0 * volume) as i16);
        samples.push((output[1][frame] * 32767.0 * volume) as i16);
    }

    let mut data = Vec::with_capacity(samples.len() * 2);
    for &sample in &samples {
        data.extend_from_slice(&sample.to_le_bytes());
    }

    let chunk_size = 4096;
    let samples_per_chunk = (chunk_size / 4) as f64;
    let start_time = Instant::now();
    let mut current_play_time = 0.0;
    let mut last_time = Instant::now();

    for (i, chunk) in data.chunks(chunk_size).enumerate() {
        let target_time = current_play_time;
        let elapsed = start_time.elapsed().as_secs_f64();
        if elapsed < target_time {
            thread::sleep(Duration::from_secs_f64(target_time - elapsed));
        }
        port.write_all(chunk)
            .expect("Failed to write to serial port");
        let now = Instant::now();
        let elapsed_since_last = now.duration_since(last_time);
        println!(
            "Chunk {i}, time since last chunk: {} ms",
            elapsed_since_last.as_millis()
        );
        last_time = now;
        current_play_time += samples_per_chunk / 46875.0;
    }

    println!("Closed serial port {}", com_port);
}
