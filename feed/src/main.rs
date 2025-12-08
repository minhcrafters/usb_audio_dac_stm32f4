use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};
use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};
use rfd::FileDialog;
use serialport::SerialPort;
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FilterType {
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

impl FilterType {
    pub fn to_biquad_type(&self, gain: f32) -> Type<f32> {
        match self {
            FilterType::Peak => Type::PeakingEQ(gain),
            FilterType::LowShelf => Type::LowShelf(gain),
            FilterType::HighShelf => Type::HighShelf(gain),
            FilterType::LowPass => Type::LowPass,
            FilterType::HighPass => Type::HighPass,
        }
    }

    pub fn from_apo_string(s: &str) -> Option<Self> {
        match s {
            "PK" => Some(FilterType::Peak),
            "LS" | "LSC" => Some(FilterType::LowShelf),
            "HS" | "HSC" => Some(FilterType::HighShelf),
            "LP" => Some(FilterType::LowPass),
            "HP" => Some(FilterType::HighPass),
            _ => None,
        }
    }

    pub fn to_apo_string(&self) -> &'static str {
        match self {
            FilterType::Peak => "PK",
            FilterType::LowShelf => "LSC",
            FilterType::HighShelf => "HSC",
            FilterType::LowPass => "LP",
            FilterType::HighPass => "HP",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EqBand {
    pub enabled: bool,
    pub filter_type: FilterType,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

impl Default for EqBand {
    fn default() -> Self {
        Self {
            enabled: true,
            filter_type: FilterType::Peak,
            freq: 1000.0,
            gain: 0.0,
            q: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Equalizer {
    pub bands: Vec<EqBand>,
    pub preamp_gain: f32, // in dB
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            bands: Vec::new(),
            preamp_gain: 0.0,
        }
    }
}

impl Equalizer {
    pub fn load_from_apo(path: &str) -> io::Result<Self> {
        let path = Path::new(path);
        let file = File::open(path)?;
        let reader = io::BufReader::new(file);
        let mut eq = Equalizer::default();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }

            if line.starts_with("Preamp:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(idx) = parts.iter().position(|&x| x == "Preamp:") {
                    if idx + 1 < parts.len() {
                        if let Ok(val) = parts[idx + 1].parse::<f32>() {
                            eq.preamp_gain = val;
                        }
                    }
                }
            } else if line.starts_with("Filter") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                let mut band = EqBand::default();

                if let Some(idx) = parts.iter().position(|&x| x == "ON" || x == "OFF") {
                    band.enabled = parts[idx] == "ON";
                    if idx + 1 < parts.len() {
                        if let Some(t) = FilterType::from_apo_string(parts[idx + 1]) {
                            band.filter_type = t;
                        }
                    }
                }

                if let Some(idx) = parts.iter().position(|&x| x == "Fc") {
                    if idx + 1 < parts.len() {
                        if let Ok(val) = parts[idx + 1].parse::<f32>() {
                            band.freq = val;
                        }
                    }
                }

                if let Some(idx) = parts.iter().position(|&x| x == "Gain") {
                    if idx + 1 < parts.len() {
                        if let Ok(val) = parts[idx + 1].parse::<f32>() {
                            band.gain = val;
                        }
                    }
                }

                if let Some(idx) = parts.iter().position(|&x| x == "Q") {
                    if idx + 1 < parts.len() {
                        if let Ok(val) = parts[idx + 1].parse::<f32>() {
                            band.q = val;
                        }
                    }
                }

                eq.bands.push(band);
            }
        }
        Ok(eq)
    }

    pub fn save_to_apo(&self, path: &str) -> io::Result<()> {
        let mut file = File::create(path)?;
        writeln!(file, "Preamp: {} dB", self.preamp_gain)?;
        for (i, band) in self.bands.iter().enumerate() {
            let status = if band.enabled { "ON" } else { "OFF" };
            writeln!(
                file,
                "Filter {}: {} {} Fc {} Hz Gain {} dB Q {}",
                i + 1,
                status,
                band.filter_type.to_apo_string(),
                band.freq,
                band.gain,
                band.q
            )?;
        }
        Ok(())
    }

    pub fn evaluate_response(&self, freq: f32, sample_rate: f32) -> f32 {
        let mut total_db = self.preamp_gain;
        let omega = 2.0 * PI * freq / sample_rate;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let cos_2w = (2.0 * omega).cos();
        let sin_2w = (2.0 * omega).sin();

        for band in &self.bands {
            if !band.enabled {
                continue;
            }

            if let Ok(coeffs) = Coefficients::<f32>::from_params(
                band.filter_type.to_biquad_type(band.gain),
                sample_rate.hz(),
                band.freq.hz(),
                band.q,
            ) {
                let b0 = coeffs.b0;
                let b1 = coeffs.b1;
                let b2 = coeffs.b2;
                let a1 = coeffs.a1;
                let a2 = coeffs.a2;

                let num_r = b0 + b1 * cos_w + b2 * cos_2w;
                let num_i = -(b1 * sin_w + b2 * sin_2w);
                let den_r = 1.0 + a1 * cos_w + a2 * cos_2w;
                let den_i = -(a1 * sin_w + a2 * sin_2w);

                let mag_sq = (num_r * num_r + num_i * num_i) / (den_r * den_r + den_i * den_i);
                total_db += 10.0 * mag_sq.log10();
            }
        }
        total_db
    }
}

#[derive(Clone)]
struct AudioFile {
    path: String,
    name: String,
}

struct AudioPlayer {
    port: Option<Box<dyn SerialPort>>,
    queue: VecDeque<AudioFile>,
    current_file: Option<AudioFile>,
    is_playing: bool,
    volume: f32,
    progress: f32,
    total_duration: f32,
    current_duration: f32,
    equalizer: Equalizer,
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self {
            port: None,
            queue: VecDeque::new(),
            current_file: None,
            is_playing: false,
            volume: 1.0,
            progress: 0.0,
            total_duration: 0.0,
            current_duration: 0.0,
            equalizer: Equalizer::default(),
        }
    }
}

impl AudioPlayer {
    fn load_file_raw(&self, file_path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use std::io::Read;

        let mut child = Command::new("ffmpeg")
            .args(&[
                "-i",
                file_path,
                "-ar",
                "46875",
                "-ac",
                "2",
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-hide_banner",
                "-loglevel",
                "error",
                "pipe:1",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()?;

        let mut data = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_end(&mut data)?;
        }

        let exit_status = child.wait()?;
        if !exit_status.success() {
            return Err("ffmpeg conversion failed".into());
        }

        Ok(data)
    }

    #[allow(dead_code)]
    fn load_file(&self, file_path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut data = self.load_file_raw(file_path)?;

        let samples = unsafe {
            std::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut i16, data.len() / 2)
        };
        for sample in samples.iter_mut() {
            *sample = (*sample as f32 * self.volume) as i16;
        }

        Ok(data)
    }

    fn play_file(player: Arc<Mutex<AudioPlayer>>, file: AudioFile) {
        {
            let mut p = player.lock().unwrap();
            p.current_file = Some(file.clone());
            p.is_playing = true;
            p.progress = 0.0;
            p.current_duration = 0.0;
            p.total_duration = 0.0;
        }

        let mut data = match {
            let p = player.lock().unwrap();
            p.load_file_raw(&file.path)
        } {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Failed to load file {}: {}", file.path, e);
                let mut p = player.lock().unwrap();
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        let total_samples = data.len() / 4;
        let total_duration = total_samples as f32 / 46875.0;

        // Initialize EQ filters
        let mut filters_l: Vec<DirectForm1<f32>> = Vec::new();
        let mut filters_r: Vec<DirectForm1<f32>> = Vec::new();
        let mut active_eq = {
            let p = player.lock().unwrap();
            p.equalizer.clone()
        };

        let mut preamp_gain_linear = 10.0f32.powf(active_eq.preamp_gain / 20.0);

        {
            let mut p = player.lock().unwrap();
            p.total_duration = total_duration;
            for band in &active_eq.bands {
                if band.enabled {
                    if let Ok(coeffs) = Coefficients::<f32>::from_params(
                        band.filter_type.to_biquad_type(band.gain),
                        46875.hz(),
                        band.freq.hz(),
                        band.q,
                    ) {
                        filters_l.push(DirectForm1::<f32>::new(coeffs));
                        filters_r.push(DirectForm1::<f32>::new(coeffs));
                    }
                }
            }
        }

        {
            let p = player.lock().unwrap();
            if p.port.is_none() {
                let mut p = player.lock().unwrap();
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        }

        let chunk_size = 4096;
        let samples_per_chunk = (chunk_size / 4) as f32;
        let chunk_duration = samples_per_chunk / 46875.0;
        let start_time = Instant::now();
        let mut current_play_time = 0.0;

        for (_i, chunk) in data.chunks_mut(chunk_size).enumerate() {
            {
                let p = player.lock().unwrap();
                if !p.is_playing {
                    break;
                }
            }

            // Check for EQ changes
            let current_eq = {
                let p = player.lock().unwrap();
                p.equalizer.clone()
            };

            if current_eq != active_eq {
                active_eq = current_eq;
                preamp_gain_linear = 10.0f32.powf(active_eq.preamp_gain / 20.0);

                // Rebuild filters
                filters_l.clear();
                filters_r.clear();
                for band in &active_eq.bands {
                    if band.enabled {
                        if let Ok(coeffs) = Coefficients::<f32>::from_params(
                            band.filter_type.to_biquad_type(band.gain),
                            46875.hz(),
                            band.freq.hz(),
                            band.q,
                        ) {
                            filters_l.push(DirectForm1::<f32>::new(coeffs));
                            filters_r.push(DirectForm1::<f32>::new(coeffs));
                        }
                    }
                }
            }

            let target_time = current_play_time;
            let elapsed = start_time.elapsed().as_secs_f32();
            if elapsed < target_time {
                thread::sleep(Duration::from_secs_f32(target_time - elapsed));
            }

            let current_volume = {
                let p = player.lock().unwrap();
                p.volume
            };

            let samples = unsafe {
                std::slice::from_raw_parts_mut(chunk.as_mut_ptr() as *mut i16, chunk.len() / 2)
            };

            // Apply EQ and Volume
            for i in (0..samples.len()).step_by(2) {
                let mut left = samples[i] as f32 * current_volume * preamp_gain_linear;
                let mut right = samples[i + 1] as f32 * current_volume * preamp_gain_linear;

                for filter in &mut filters_l {
                    left = filter.run(left);
                }
                for filter in &mut filters_r {
                    right = filter.run(right);
                }

                samples[i] = left.clamp(-32768.0, 32767.0) as i16;
                samples[i + 1] = right.clamp(-32768.0, 32767.0) as i16;
            }

            {
                let mut p = player.lock().unwrap();
                if let Some(ref mut port) = p.port {
                    if let Err(e) = port.write_all(chunk) {
                        eprintln!("Failed to write to serial port: {}", e);
                        break;
                    }
                } else {
                    break;
                }
            }

            current_play_time += chunk_duration;

            {
                let mut p = player.lock().unwrap();
                p.current_duration = current_play_time;
                p.progress = if p.total_duration > 0.0 {
                    p.current_duration / p.total_duration
                } else {
                    0.0
                };
            }
        }

        let mut p = player.lock().unwrap();
        p.is_playing = false;
        p.current_file = None;
        p.progress = 0.0;
        p.current_duration = 0.0;
        p.total_duration = 0.0;
    }
}

struct App {
    player: Arc<Mutex<AudioPlayer>>,
    available_ports: Vec<String>,
    selected_port: String,
    _file_path: String,
    playback_thread: Option<thread::JoinHandle<()>>,
}

impl Default for App {
    fn default() -> Self {
        let ports = serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.port_name)
            .collect();

        Self {
            player: Arc::new(Mutex::new(AudioPlayer::default())),
            available_ports: ports,
            selected_port: String::new(),
            _file_path: String::new(),
            playback_thread: None,
        }
    }
}

fn format_duration(seconds: f32) -> String {
    let total_seconds = seconds as u32;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let secs = total_seconds % 60;
    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut content_rect = egui::Rect::NOTHING;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Port:");
                egui::ComboBox::from_label("")
                    .selected_text(&self.selected_port)
                    .show_ui(ui, |ui| {
                        for port in &self.available_ports {
                            ui.selectable_value(&mut self.selected_port, port.clone(), port);
                        }
                    });
                if ui.button("Connect").clicked() {
                    if !self.selected_port.is_empty() {
                        match serialport::new(&self.selected_port, 115200)
                            .timeout(Duration::from_millis(1000))
                            .open()
                        {
                            Ok(port) => {
                                if let Ok(mut player) = self.player.lock() {
                                    player.port = Some(port);
                                    println!("Connected to {}", self.selected_port);
                                }
                            }
                            Err(e) => {
                                eprintln!("Failed to open port {}: {}", self.selected_port, e);
                            }
                        }
                    }
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Select audio file").clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Audio files", &["mp3", "wav", "flac", "ogg", "m4a", "aac"])
                        .pick_file()
                    {
                        let file_name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("Unknown")
                            .to_string();
                        let audio_file = AudioFile {
                            path: path.to_string_lossy().to_string(),
                            name: file_name,
                        };
                        if let Ok(mut player) = self.player.lock() {
                            player.queue.push_back(audio_file);
                        }
                    }
                }
            });

            ui.label("Queue:");
            let mut to_remove = None;
            if let Ok(player) = self.player.lock() {
                let queue = &player.queue;
                for (i, file) in queue.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}. {}", i + 1, file.name));
                        if ui.button("Remove").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
            }
            if let Some(index) = to_remove {
                if let Ok(mut player) = self.player.lock() {
                    player.queue.remove(index);
                }
            }

            ui.separator();

            ui.horizontal(|ui| {
                let (queue_has_tracks, is_playing, port_connected) =
                    if let Ok(player) = self.player.lock() {
                        (
                            !player.queue.is_empty(),
                            player.is_playing,
                            player.port.is_some(),
                        )
                    } else {
                        (false, false, false)
                    };

                if ui.button("Play").clicked() && queue_has_tracks && port_connected {
                    if let Ok(mut player) = self.player.lock() {
                        if is_playing {
                            // Stop and clear current track
                            player.is_playing = false;
                        }
                        // if let Some(thread) = self.playback_thread.take() {
                        //     let _ = thread.join();
                        // }
                        // Start next track
                        if let Some(file) = player.queue.pop_front() {
                            let player_clone = Arc::clone(&self.player);
                            self.playback_thread = Some(thread::spawn(move || {
                                AudioPlayer::play_file(player_clone, file);
                            }));
                        }
                    }
                }
                if ui.button("Stop").clicked() {
                    if let Ok(mut player) = self.player.lock() {
                        player.is_playing = false;
                    }
                }
                let mut volume = 1.0;
                if let Ok(mut player) = self.player.lock() {
                    ui.add(egui::Slider::new(&mut player.volume, 0.0..=2.0).text("Volume"));
                } else {
                    ui.add(egui::Slider::new(&mut volume, 0.0..=2.0).text("Volume"));
                }
            });

            ui.separator();
            ui.collapsing("Equalizer", |ui| {
                let mut load_path = None;
                let mut save_path = None;

                ui.horizontal(|ui| {
                    if ui.button("Load Config").clicked() {
                        if let Some(path) =
                            FileDialog::new().add_filter("Text", &["txt"]).pick_file()
                        {
                            load_path = Some(path.to_string_lossy().to_string());
                        }
                    }
                    if ui.button("Save Config").clicked() {
                        if let Some(path) =
                            FileDialog::new().add_filter("Text", &["txt"]).save_file()
                        {
                            save_path = Some(path.to_string_lossy().to_string());
                        }
                    }
                    if ui.button("Add Band").clicked() {
                        if let Ok(mut player) = self.player.lock() {
                            player.equalizer.bands.push(EqBand::default());
                        }
                    }
                });

                if let Some(path) = load_path {
                    if let Ok(eq) = Equalizer::load_from_apo(&path) {
                        if let Ok(mut player) = self.player.lock() {
                            player.equalizer = eq;
                        }
                    }
                }

                if let Some(path) = save_path {
                    if let Ok(player) = self.player.lock() {
                        let _ = player.equalizer.save_to_apo(&path);
                    }
                }

                // Plot
                let points = if let Ok(player) = self.player.lock() {
                    (0..=200)
                        .map(|i| {
                            let x = i as f32;
                            // Logarithmic scale 20Hz to 20kHz
                            // log10(20) = 1.301
                            // log10(20000) = 4.301
                            // range = 3.0
                            let log_f = 1.301 + (x / 200.0) * 3.0;
                            let f = 10.0f32.powf(log_f);
                            let db = player.equalizer.evaluate_response(f, 46875.0);
                            [log_f as f64, db as f64]
                        })
                        .collect::<PlotPoints>()
                } else {
                    PlotPoints::default()
                };

                Plot::new("eq_plot")
                    .view_aspect(3.0)
                    .x_axis_label("Frequency (Hz)")
                    .y_axis_label("Gain (dB)")
                    .x_axis_formatter(|x, _range| {
                        let f = 10.0f64.powf(x.value);
                        if f >= 1000.0 {
                            format!("{:.0}k", f / 1000.0)
                        } else {
                            format!("{:.0}", f)
                        }
                    })
                    .include_x(1.301)
                    .include_x(4.301)
                    .include_y(-20.0)
                    .include_y(20.0)
                    .show(ui, |plot_ui| {
                        plot_ui.line(Line::new("Response", points));
                    });

                // Controls
                if let Ok(mut player) = self.player.lock() {
                    ui.horizontal(|ui| {
                        ui.label("Preamp (dB):");
                        ui.add(egui::DragValue::new(&mut player.equalizer.preamp_gain).speed(0.1));
                    });

                    let mut to_remove = None;
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for (i, band) in player.equalizer.bands.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut band.enabled, format!("#{}", i + 1));

                                    egui::ComboBox::from_id_salt(format!("type_{}", i))
                                        .selected_text(format!("{:?}", band.filter_type))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut band.filter_type,
                                                FilterType::Peak,
                                                "Peak (PK)",
                                            );
                                            ui.selectable_value(
                                                &mut band.filter_type,
                                                FilterType::LowShelf,
                                                "LowShelf (LSC)",
                                            );
                                            ui.selectable_value(
                                                &mut band.filter_type,
                                                FilterType::HighShelf,
                                                "HighShelf (HSC)",
                                            );
                                            ui.selectable_value(
                                                &mut band.filter_type,
                                                FilterType::LowPass,
                                                "LowPass (LP)",
                                            );
                                            ui.selectable_value(
                                                &mut band.filter_type,
                                                FilterType::HighPass,
                                                "HighPass (HP)",
                                            );
                                        });

                                    ui.add(
                                        egui::DragValue::new(&mut band.freq)
                                            .speed(10.0)
                                            .range(20.0..=22000.0)
                                            .prefix("Fc: ")
                                            .suffix(" Hz"),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut band.gain)
                                            .speed(0.1)
                                            .prefix("Gain: ")
                                            .suffix(" dB"),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut band.q)
                                            .speed(0.01)
                                            .range(0.1..=100.0)
                                            .prefix("Q: "),
                                    );

                                    if ui.button("X").clicked() {
                                        to_remove = Some(i);
                                    }
                                });
                            }
                        });

                    if let Some(idx) = to_remove {
                        player.equalizer.bands.remove(idx);
                    }
                }
            });

            if let Ok(player) = self.player.lock() {
                if player.is_playing {
                    if let Some(ref file) = player.current_file {
                        ui.label(format!("Now playing: {}", file.name));
                        ui.label(format!(
                            "{} / {}",
                            format_duration(player.current_duration),
                            format_duration(player.total_duration)
                        ));
                    }
                }
                if player.port.is_some() {
                    ui.colored_label(egui::Color32::GREEN, "Connected");
                } else {
                    ui.colored_label(egui::Color32::RED, "Not connected");
                }
            }
            content_rect = ui.min_rect();
        });

        let current_size = ctx
            .input(|i| i.viewport().inner_rect)
            .map(|r| r.size())
            .unwrap_or(egui::Vec2::new(500.0, 300.0));
        let desired_height = content_rect.bottom() + 8.0;
        if (current_size.y - desired_height).abs() > 5.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
                current_size.x,
                desired_height,
            )));
        }

        ctx.request_repaint();
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 300.0])
            .with_min_inner_size([500.0, 150.0]),
        ..Default::default()
    };

    eframe::run_native(
        "USB audio player",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
