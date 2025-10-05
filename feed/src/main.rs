use eframe::egui;
use rfd::FileDialog;
use serialport::SerialPort;
use std::collections::VecDeque;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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

        {
            let mut p = player.lock().unwrap();
            p.total_duration = total_duration;
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
            for sample in samples.iter_mut() {
                *sample = (*sample as f32 * current_volume) as i16;
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

struct AudioQueueApp {
    player: Arc<Mutex<AudioPlayer>>,
    available_ports: Vec<String>,
    selected_port: String,
    _file_path: String,
    playback_thread: Option<thread::JoinHandle<()>>,
}

impl Default for AudioQueueApp {
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

impl eframe::App for AudioQueueApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
                let (can_play, _, port_connected) = if let Ok(player) = self.player.lock() {
                    (
                        !player.queue.is_empty(),
                        player.is_playing,
                        player.port.is_some(),
                    )
                } else {
                    (false, false, false)
                };

                if ui.button("Play").clicked() && can_play && port_connected {
                    if let Ok(mut player) = self.player.lock() {
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
        });

        ctx.request_repaint();
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 300.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_min_inner_size([500.0, 300.0])
            .with_max_inner_size([500.0, 300.0]),
        ..Default::default()
    };

    eframe::run_native(
        "USB audio player",
        options,
        Box::new(|_cc| Ok(Box::new(AudioQueueApp::default()))),
    )
}
