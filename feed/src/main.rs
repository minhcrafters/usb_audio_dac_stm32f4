use eframe::egui;
use rfd::FileDialog;
use serialport::SerialPort;
use std::collections::VecDeque;
use std::io::Write;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

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
    last_error: Option<String>,
    playback_id: u64,
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
            last_error: None,
            playback_id: 0,
        }
    }
}

impl AudioPlayer {
    fn get_duration(file_path: &str) -> Option<f32> {
        let output = Command::new("ffprobe")
            .args(&[
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                file_path,
            ])
            .output()
            .ok()?;

        let duration_str = String::from_utf8_lossy(&output.stdout);
        duration_str.trim().parse::<f32>().ok()
    }

    fn play_file(player: Arc<Mutex<AudioPlayer>>, file: AudioFile) {
        use std::io::Read;

        let my_playback_id = {
            let mut p = player.lock().unwrap();
            p.current_file = Some(file.clone());
            p.is_playing = true;
            p.progress = 0.0;
            p.current_duration = 0.0;
            p.total_duration = 0.0;
            p.playback_id
        };

        if let Some(duration) = Self::get_duration(&file.path) {
            let mut p = player.lock().unwrap();
            p.total_duration = duration;
        }

        let mut child = match Command::new("ffmpeg")
            .args(&[
                "-i",
                &file.path,
                "-ar",
                "48000",
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
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let mut p = player.lock().unwrap();
                p.last_error = Some(format!("Failed to start ffmpeg: {}", e));
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        let mut stdout = if let Some(out) = child.stdout.take() {
            out
        } else {
            let mut p = player.lock().unwrap();
            p.last_error = Some("Failed to open ffmpeg stdout".to_string());
            p.is_playing = false;
            p.current_file = None;
            return;
        };

        {
            let p = player.lock().unwrap();
            if p.port.is_none() {
                let mut p = player.lock().unwrap();
                p.is_playing = false;
                p.current_file = None;
                let _ = child.kill();
                return;
            }
        }

        let chunk_size = 4096;
        let mut buffer = vec![0u8; chunk_size];
        let start_time = Instant::now();
        let mut current_play_time = 0.0;

        loop {
            {
                let p = player.lock().unwrap();
                if !p.is_playing || p.playback_id != my_playback_id {
                    let _ = child.kill();
                    break;
                }
            }

            let mut bytes_read = 0;
            while bytes_read < chunk_size {
                match stdout.read(&mut buffer[bytes_read..]) {
                    Ok(0) => break,
                    Ok(n) => bytes_read += n,
                    Err(_) => break,
                }
            }

            if bytes_read == 0 {
                break;
            }

            let chunk = &mut buffer[..bytes_read];
            let chunk_duration = (chunk.len() as f32 / 4.0) / 48000.0;

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

            for i in (0..samples.len()).step_by(2) {
                let left = samples[i] as f32 * current_volume;
                let right = samples[i + 1] as f32 * current_volume;

                samples[i] = left.clamp(-32768.0, 32767.0) as i16;
                samples[i + 1] = right.clamp(-32768.0, 32767.0) as i16;
            }

            {
                let mut p = player.lock().unwrap();
                if let Some(ref mut port) = p.port {
                    if let Err(e) = port.write_all(chunk) {
                        p.last_error = Some(format!("Failed to write to serial port: {}", e));
                        p.port = None;
                        let _ = child.kill();
                        break;
                    }
                } else {
                    let _ = child.kill();
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

        let _ = child.wait();

        let mut p = player.lock().unwrap();
        p.is_playing = false;
        p.current_file = None;
        p.progress = 0.0;
        p.current_duration = 0.0;
        p.total_duration = 0.0;
    }

    fn capture_system_audio(player: Arc<Mutex<AudioPlayer>>) {
        let (writer_port, my_playback_id) = {
            let mut p = player.lock().unwrap();
            p.current_file = Some(AudioFile {
                path: String::new(),
                name: "System Audio (Loopback)".to_string(),
            });
            p.is_playing = true;
            p.progress = 0.0;
            p.total_duration = 0.0;
            p.current_duration = 0.0;

            if let Some(ref mut port) = p.port {
                match port.try_clone() {
                    Ok(cloned) => (Some(cloned), p.playback_id),
                    Err(e) => {
                        p.last_error = Some(format!("Failed to clone serial port: {}", e));
                        p.is_playing = false;
                        p.current_file = None;
                        return;
                    }
                }
            } else {
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);

        let player_for_writer = player.clone();
        thread::spawn(move || {
            let mut port = writer_port.unwrap();
            while let Ok(chunk) = rx.recv() {
                if let Err(e) = port.write_all(&chunk) {
                    let mut p = player_for_writer.lock().unwrap();
                    p.last_error = Some(format!("Writer thread: write failed: {}", e));
                    p.port = None;
                    p.is_playing = false;
                    break;
                }
            }
        });

        if initialize_mta().is_err() {
            let mut p = player.lock().unwrap();
            p.last_error = Some("WASAPI: failed to initialize COM (MTA)".to_string());
            p.is_playing = false;
            p.current_file = None;
            return;
        }

        let enumerator = DeviceEnumerator::new().unwrap();

        let device = match enumerator.get_default_device(&Direction::Render) {
            Ok(d) => d,
            Err(e) => {
                let mut p = player.lock().unwrap();
                p.last_error = Some(format!("WASAPI: get_default_device failed: {e}"));
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        let mut client = match device.get_iaudioclient() {
            Ok(c) => c,
            Err(e) => {
                let mut p = player.lock().unwrap();
                p.last_error = Some(format!("WASAPI: get_iaudioclient failed: {e}"));
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        let desired = WaveFormat::new(16, 16, &SampleType::Int, 48_000, 2, None);

        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 200_000,
        };

        let fmt = match client.initialize_client(&desired, &Direction::Capture, &mode) {
            Ok(_) => desired,
            Err(_) => {
                let mix = match client.get_mixformat() {
                    Ok(m) => m,
                    Err(e) => {
                        let mut p = player.lock().unwrap();
                        p.last_error = Some(format!("WASAPI: get_mixformat failed: {e}"));
                        p.is_playing = false;
                        p.current_file = None;
                        return;
                    }
                };
                if let Err(e) = client.initialize_client(&mix, &Direction::Capture, &mode) {
                    let mut p = player.lock().unwrap();
                    p.last_error = Some(format!(
                        "WASAPI: initialize_client failed for desired and mix: {e}"
                    ));
                    p.is_playing = false;
                    p.current_file = None;
                    return;
                }
                mix
            }
        };

        let bytes_per_frame = fmt.get_blockalign() as usize;
        let sample_type = match fmt.get_subformat() {
            Ok(s) => s,
            Err(e) => {
                let mut p = player.lock().unwrap();
                p.last_error = Some(format!("WASAPI: get_subformat failed: {e}"));
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        let evt = match client.set_get_eventhandle() {
            Ok(h) => h,
            Err(e) => {
                let mut p = player.lock().unwrap();
                p.last_error = Some(format!("WASAPI: set_get_eventhandle failed: {e}"));
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        let capture = match client.get_audiocaptureclient() {
            Ok(c) => c,
            Err(e) => {
                let mut p = player.lock().unwrap();
                p.last_error = Some(format!("WASAPI: get_audiocaptureclient failed: {e}"));
                p.is_playing = false;
                p.current_file = None;
                return;
            }
        };

        if let Err(e) = client.start_stream() {
            let mut p = player.lock().unwrap();
            p.last_error = Some(format!("WASAPI: start_stream failed: {e}"));
            p.is_playing = false;
            p.current_file = None;
            return;
        }

        let start_time = Instant::now();
        let mut deque: VecDeque<u8> = VecDeque::new();

        let mut chunk_bytes = 4096;
        chunk_bytes -= chunk_bytes % bytes_per_frame;
        if chunk_bytes == 0 {
            chunk_bytes = bytes_per_frame * 64;
        }

        loop {
            {
                let p = player.lock().unwrap();
                if !p.is_playing || p.playback_id != my_playback_id {
                    break;
                }
                if p.port.is_none() {
                    break;
                }
            }

            if evt.wait_for_event(2000).is_err() {}

            loop {
                let next = match capture.get_next_packet_size() {
                    Ok(n) => n.unwrap_or(0),
                    Err(e) => {
                        let mut p = player.lock().unwrap();
                        p.last_error = Some(format!("WASAPI: get_next_packet_size failed: {e}"));
                        p.is_playing = false;
                        break;
                    }
                };
                if next == 0 {
                    break;
                }
                if let Err(e) = capture.read_from_device_to_deque(&mut deque) {
                    let mut p = player.lock().unwrap();
                    p.last_error = Some(format!("WASAPI: read_from_device_to_deque failed: {e}"));
                    p.is_playing = false;
                    break;
                }
            }

            while deque.len() >= chunk_bytes {
                let mut chunk: Vec<u8> = deque.drain(..chunk_bytes).collect();

                let vol = { player.lock().unwrap().volume };

                match sample_type {
                    SampleType::Int => {
                        if fmt.get_bitspersample() == 16 {
                            let samples = unsafe {
                                std::slice::from_raw_parts_mut(
                                    chunk.as_mut_ptr() as *mut i16,
                                    chunk.len() / 2,
                                )
                            };
                            for s in samples {
                                let v = (*s as f32 * vol).clamp(-32768.0, 32767.0);
                                *s = v as i16;
                            }
                        }
                    }
                    SampleType::Float => {
                        if fmt.get_bitspersample() == 32 {
                            let samples = unsafe {
                                std::slice::from_raw_parts_mut(
                                    chunk.as_mut_ptr() as *mut f32,
                                    chunk.len() / 4,
                                )
                            };
                            for s in samples {
                                *s = (*s * vol).clamp(-1.0, 1.0);
                            }
                        }
                    }
                }

                if tx.send(chunk).is_err() {
                    break;
                }

                {
                    let mut p = player.lock().unwrap();
                    p.current_duration = start_time.elapsed().as_secs_f32();
                    p.progress = 0.0;
                }
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlaybackMode {
    Idle,
    File,
    Loopback,
}

#[derive(Clone)]
struct PlayerSnapshot {
    connected: bool,
    is_playing: bool,
    volume: f32,

    queue: Vec<AudioFile>,
    current_file: Option<AudioFile>,

    progress: f32,
    current_duration: f32,
    total_duration: f32,

    last_error: Option<String>,
    mode: PlaybackMode,
}

impl PlayerSnapshot {
    fn from_player(p: &AudioPlayer) -> Self {
        let mode = if p.is_playing {
            match p.current_file.as_ref().map(|f| f.name.as_str()) {
                Some("System Audio (Loopback)") => PlaybackMode::Loopback,
                Some(_) => PlaybackMode::File,
                None => PlaybackMode::File,
            }
        } else {
            PlaybackMode::Idle
        };

        Self {
            connected: p.port.is_some(),
            is_playing: p.is_playing,
            volume: p.volume,
            queue: p.queue.iter().cloned().collect(),
            current_file: p.current_file.clone(),
            progress: p.progress,
            current_duration: p.current_duration,
            total_duration: p.total_duration,
            last_error: p.last_error.clone(),
            mode,
        }
    }
}

struct App {
    player: Arc<Mutex<AudioPlayer>>,

    available_ports: Vec<String>,
    selected_port: Option<String>,

    playback_thread: Option<thread::JoinHandle<()>>,
    system_thread: Option<thread::JoinHandle<()>>,

    autoscroll_queue: bool,
}

impl Default for App {
    fn default() -> Self {
        let mut app = Self {
            player: Arc::new(Mutex::new(AudioPlayer::default())),
            available_ports: vec![],
            selected_port: None,
            playback_thread: None,
            system_thread: None,
            autoscroll_queue: true,
        };
        app.refresh_ports();
        if let Some(first) = app.available_ports.first().cloned() {
            app.selected_port = Some(first);
        }
        app
    }
}

impl App {
    fn refresh_ports(&mut self) {
        self.available_ports = serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.port_name)
            .collect();

        if let Some(sel) = self.selected_port.clone() {
            if !self.available_ports.iter().any(|p| p == &sel) {
                self.selected_port = self.available_ports.first().cloned();
            }
        } else {
            self.selected_port = self.available_ports.first().cloned();
        }
    }

    fn connect_selected(&mut self) {
        let Some(port_name) = self.selected_port.clone() else {
            return;
        };

        if let Ok(mut p) = self.player.lock() {
            p.last_error = None;
        }

        match serialport::new(&port_name, 115200)
            .timeout(Duration::from_millis(1000))
            .open()
        {
            Ok(port) => {
                if let Ok(mut p) = self.player.lock() {
                    p.port = Some(port);
                }
            }
            Err(e) => {
                if let Ok(mut p) = self.player.lock() {
                    p.last_error = Some(format!("Failed to open port {}: {}", port_name, e));
                }
            }
        }
    }

    fn disconnect(&mut self) {
        if let Ok(mut p) = self.player.lock() {
            p.is_playing = false;
            p.port = None;
        }
    }

    fn cleanup_finished_threads(&mut self) {
        if let Some(h) = &self.playback_thread {
            if h.is_finished() {
                let _ = self.playback_thread.take().unwrap().join();
            }
        }
        if let Some(h) = &self.system_thread {
            if h.is_finished() {
                let _ = self.system_thread.take().unwrap().join();
            }
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
        self.cleanup_finished_threads();

        let snap = self
            .player
            .lock()
            .ok()
            .map(|p| PlayerSnapshot::from_player(&p));

        let Some(snap) = snap else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.colored_label(egui::Color32::RED, "Player mutex poisoned / unavailable.");
            });
            return;
        };

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("USB Audio Player");
                // ui.add_space(12.0);

                ui.separator();

                ui.label("Serial port:");
                egui::ComboBox::from_id_salt("port_combo")
                    .selected_text(self.selected_port.clone().unwrap_or_else(|| "None".into()))
                    .show_ui(ui, |ui| {
                        for p in &self.available_ports {
                            ui.selectable_value(&mut self.selected_port, Some(p.clone()), p);
                        }
                    });

                if ui.button("↻").clicked() {
                    self.refresh_ports();
                }

                if snap.connected {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                    ui.colored_label(egui::Color32::GREEN, "Connected");
                } else {
                    let can_connect = self.selected_port.is_some();
                    if ui
                        .add_enabled(can_connect, egui::Button::new("Connect"))
                        .clicked()
                    {
                        self.connect_selected();
                    }
                    ui.colored_label(egui::Color32::RED, "Not connected");
                }
            });

            if let Some(err) = &snap.last_error {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(60, 0, 0))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("Error: {err}"));
                    });
            }

            ui.add_space(6.0);
        });

        egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
            let port_connected = snap.connected;
            let has_queue = !snap.queue.is_empty();
            let is_playing = snap.is_playing;
            let is_loopback = snap.mode == PlaybackMode::Loopback;

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let play_label = if is_playing { "Pause" } else { "Play" };

                let play_clicked = ui
                    .add_enabled(
                        port_connected && (has_queue || is_playing) && !is_loopback,
                        egui::Button::new(play_label),
                    )
                    .on_hover_text("Play the next queued file (or pause current file playback).")
                    .clicked();

                if play_clicked {
                    if let Ok(mut p) = self.player.lock() {
                        p.last_error = None;
                        p.is_playing = !p.is_playing;

                        if p.is_playing && p.current_file.is_none() {
                            if let Some(file) = p.queue.pop_front() {
                                p.playback_id += 1;
                                let player_clone = Arc::clone(&self.player);
                                self.playback_thread = Some(thread::spawn(move || {
                                    AudioPlayer::play_file(player_clone, file);
                                }));
                            } else {
                                p.is_playing = false;
                            }
                        }
                    }
                }

                if ui
                    .add_enabled(is_playing, egui::Button::new("Stop"))
                    .on_hover_text("Stop playback/capture.")
                    .clicked()
                {
                    if let Ok(mut p) = self.player.lock() {
                        p.is_playing = false;
                    }
                }

                if ui
                    .add_enabled(
                        port_connected && has_queue && !is_loopback,
                        egui::Button::new("Skip"),
                    )
                    .on_hover_text("Stop current file and start the next one in queue.")
                    .clicked()
                {
                    if let Ok(mut p) = self.player.lock() {
                        p.is_playing = false;
                        p.current_file = None;
                    }
                    if let Ok(mut p) = self.player.lock() {
                        if let Some(file) = p.queue.pop_front() {
                            p.is_playing = true;
                            p.playback_id += 1;
                            let player_clone = Arc::clone(&self.player);
                            self.playback_thread = Some(thread::spawn(move || {
                                AudioPlayer::play_file(player_clone, file);
                            }));
                        }
                    }
                }

                ui.separator();

                if ui
                    .add_enabled(
                        port_connected && !is_playing,
                        egui::Button::new("Capture System Audio"),
                    )
                    .on_hover_text("Starts WASAPI loopback streaming.")
                    .clicked()
                {
                    if let Ok(mut p) = self.player.lock() {
                        p.last_error = None;
                        p.is_playing = false;
                        p.playback_id += 1;
                    }
                    let player_clone = Arc::clone(&self.player);
                    self.system_thread = Some(thread::spawn(move || {
                        AudioPlayer::capture_system_audio(player_clone);
                    }));
                }

                ui.separator();

                let mut new_vol = snap.volume;
                ui.label("Volume");
                ui.add(egui::Slider::new(&mut new_vol, 0.0..=2.0).show_value(true));
                if (new_vol - snap.volume).abs() > f32::EPSILON {
                    if let Ok(mut p) = self.player.lock() {
                        p.volume = new_vol;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.autoscroll_queue, "Auto-scroll queue");
                });
            });

            ui.add_space(4.0);
            let title = match (&snap.current_file, snap.mode) {
                (Some(f), PlaybackMode::Loopback) => format!("Capturing: {}", f.name),
                (Some(f), PlaybackMode::File) => format!("Playing: {}", f.name),
                _ => "Idle".to_string(),
            };
            ui.label(title);

            if snap.mode == PlaybackMode::File && snap.total_duration > 0.0 {
                ui.horizontal(|ui| {
                    ui.label(format_duration(snap.current_duration));
                    ui.add(
                        egui::ProgressBar::new(snap.progress.clamp(0.0, 1.0))
                            .desired_width(f32::INFINITY)
                            .show_percentage(),
                    );
                    ui.label(format_duration(snap.total_duration));
                });
            } else if snap.mode == PlaybackMode::Loopback {
                ui.horizontal(|ui| {
                    ui.label(format_duration(snap.current_duration));
                    ui.add(
                        egui::ProgressBar::new(0.5)
                            .desired_width(f32::INFINITY)
                            .animate(true)
                            .text("Streaming…"),
                    );
                });
            }

            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);

            // egui::Frame::group(ui.style())
            //     // .inner_margin(egui::Margin::same(10))
            //     .show(ui, |ui| {

            //     });

            ui.horizontal(|ui| {
                if ui.button("Add files").clicked() {
                    if let Some(paths) = FileDialog::new()
                        .add_filter("Audio files", &["mp3", "wav", "flac", "ogg", "m4a", "aac"])
                        .pick_files()
                    {
                        if let Ok(mut p) = self.player.lock() {
                            for path in paths {
                                let file_name = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("Unknown")
                                    .to_string();

                                let audio_file = AudioFile {
                                    path: path.to_string_lossy().to_string(),
                                    name: file_name,
                                };

                                p.queue.push_back(audio_file);
                            }
                        }
                    }
                }

                if ui.button("Clear queue").clicked() {
                    if let Ok(mut p) = self.player.lock() {
                        p.queue.clear();
                    }
                }

                ui.add_space(10.0);
            });

            ui.add_space(8.0);

            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Queue");
                        ui.add_space(8.0);
                        ui.small(format!("{} item(s)", snap.queue.len()));
                    });

                    ui.add_space(6.0);

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(240.0)
                        .show(ui, |ui| {
                            let mut remove_idx: Option<usize> = None;

                            for (i, file) in snap.queue.iter().enumerate() {
                                let row = egui::Frame::new()
                                    .fill(if i % 2 == 0 {
                                        ui.visuals().faint_bg_color
                                    } else {
                                        ui.visuals().extreme_bg_color
                                    })
                                    .corner_radius(egui::CornerRadius::same(6))
                                    .inner_margin(egui::Margin::symmetric(8, 6));

                                row.show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.monospace(format!("{:>2}.", i + 1));
                                        ui.label(&file.name);

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.button("Remove").clicked() {
                                                    remove_idx = Some(i);
                                                }

                                                if ui.button("Play now").clicked() {
                                                    if let Ok(mut p) = self.player.lock() {
                                                        p.last_error = None;
                                                        p.is_playing = false;
                                                        p.playback_id += 1;
                                                    }
                                                    let player_clone = Arc::clone(&self.player);
                                                    let chosen = file.clone();
                                                    self.playback_thread =
                                                        Some(thread::spawn(move || {
                                                            AudioPlayer::play_file(
                                                                player_clone,
                                                                chosen,
                                                            );
                                                        }));
                                                }
                                            },
                                        );
                                    });
                                });

                                ui.add_space(4.0);
                            }

                            if let Some(idx) = remove_idx {
                                if let Ok(mut p) = self.player.lock() {
                                    if idx < p.queue.len() {
                                        p.queue.remove(idx);
                                    }
                                }
                            }
                        });
                });
        });

        if snap.is_playing {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 480.0])
            .with_min_inner_size([720.0, 480.0])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "USB audio player",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
