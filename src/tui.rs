use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant, SystemTime},
};

use color_eyre::Result;
use rand::RngCore;
use ratatui::{
    crossterm::event::{self, Event, KeyCode},
    layout::{Alignment, Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};
use ratatui::{prelude::Stylize, style::Modifier};
use tracing::{error, info, trace};

use crate::{
    init_components, init_tracing, run_simulation_step, FaultType, FileFaultType, SimulatedIO,
};

pub async fn run_tui() -> Result<()> {
    color_eyre::install()?;
    init_tracing(crate::LogOptions::File);
    let mut terminal = ratatui::init();
    let mut game_state = GameState::new();
    let seed = match std::env::var("SEED") {
        Ok(seed) => seed.parse::<u64>().unwrap(),
        Err(_) => rand::thread_rng().next_u64(),
    };
    info!("Running game loop with seed {}", seed);
    let mut io = SimulatedIO::new(seed);
    let config_key = "config_key";
    let app_result = App::default()
        .run(&mut terminal, &mut io, &config_key)
        .await;
    ratatui::restore();
    Ok(app_result?)
}

impl FaultType {
    fn to_symbol(&self) -> &str {
        match self {
            FaultType::KafkaConnectionFailure => "⚔️",
            FaultType::RedisConnectionFailure => "🛡️",
            FaultType::KafkaReadFailure => "🔥",
            FaultType::RedisReadFailure => "❄️",
            FaultType::FileOpenFailure => "💥",
            FaultType::FileFaultType(_) => "⚡",
        }
    }

    fn to_log_message(&self) -> String {
        match self {
            FaultType::KafkaConnectionFailure => "Kafka connection failed".to_string(),
            FaultType::RedisConnectionFailure => "Redis connection failed".to_string(),
            FaultType::KafkaReadFailure => "Kafka read failed".to_string(),
            FaultType::RedisReadFailure => "Redis read failed".to_string(),
            FaultType::FileOpenFailure => "File open failed".to_string(),
            FaultType::FileFaultType(fault) => match fault {
                FileFaultType::FileReadFailure => "File read failed".to_string(),
                FileFaultType::FileWriteFailure => "File write failed".to_string(),
                FileFaultType::FileSizeExceededFailure => "File size exceeded".to_string(),
                FileFaultType::FileMetadataSyncFailure => "File metadata sync failed".to_string(),
            },
        }
    }
}

struct GameState {
    active_faults: VecDeque<(FaultType, u8)>,
    fault_log: VecDeque<String>,
    tick_count: u64,
}

impl GameState {
    fn new() -> Self {
        Self {
            active_faults: VecDeque::new(),
            fault_log: VecDeque::new(),
            tick_count: 0,
        }
    }

    fn add_fault(&mut self, fault: FaultType) {
        self.active_faults.push_back((fault.clone(), 0));
        self.fault_log.push_back(fault.to_log_message());
        if self.fault_log.len() > 20 {
            self.fault_log.pop_front();
        }
    }

    fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        for (_, pos) in self.active_faults.iter_mut() {
            *pos = pos.saturating_add(1);
        }
        while self
            .active_faults
            .front()
            .map_or(false, |(_, pos)| *pos >= 10)
        {
            self.active_faults.pop_front();
        }
    }
}

#[derive(Default)]
struct App {
    active_faults: VecDeque<(FaultType, u8)>,
    fault_log: VecDeque<String>,
    status_log: VecDeque<String>,
    status_log_counter: usize,
    tick_count: u64,
}

impl App {
    fn add_fault(&mut self, fault: FaultType) {
        self.active_faults.push_back((fault.clone(), 0));
        self.fault_log.push_back(fault.to_log_message());
        if self.fault_log.len() > 20 {
            self.fault_log.pop_front();
        }
    }

    fn add_status_messages(&mut self) {
        let messages = [
            format!("[{}] Read messages from Kafka", self.status_log_counter),
            format!("[{}] Read messages from Redis", self.status_log_counter + 1),
            format!("[{}] Wrote output to file", self.status_log_counter + 2),
        ];

        for msg in messages {
            self.status_log.push_back(msg);
        }

        while self.status_log.len() > 50 {
            // Keep more messages for scrolling effect
            self.status_log.pop_front();
        }
        self.status_log_counter += 3;
    }

    fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        for (_, pos) in self.active_faults.iter_mut() {
            *pos = pos.saturating_add(1);
        }
        while self
            .active_faults
            .front()
            .map_or(false, |(_, pos)| *pos >= 10)
        {
            let entry = self.active_faults.pop_front();
            if let Some(e) = entry {
                trace!("removing fault type {:?}", e.0);
            }
        }
    }
    //  Run the main loop and show the UI until the user presses quit
    pub async fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        io: &mut SimulatedIO,
        config_key: &str,
    ) -> io::Result<()> {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_secs(1);
        let mut written_messages = Vec::new();
        let mut counter = 0;
        let mut has_initialised = false;
        loop {
            //  listen for quit events
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                }
            }

            if !has_initialised {
                match init_components(io).await {
                    Ok(faults) => {
                        for fault in faults {
                            self.add_fault(fault);
                        }
                        has_initialised = true;
                    }
                    Err(e) => {
                        //  TODO: Found an error. What should I do? Log it?
                        break;
                    }
                }
            }
            info!("Done initialising the components while running game loop");

            match run_simulation_step(io, config_key, &mut counter, &mut written_messages).await {
                Ok(faults) => {
                    info!("the generated faults {:?}", faults);
                    for fault in faults {
                        self.add_fault(fault);
                    }
                    self.add_status_messages();
                }
                Err(e) => {
                    //  TODO: Found an error. What should I do? Log it?
                    error!("error while running run_simulation_step {:?}", e);
                    break;
                }
            }
            trace!("ran single step of the simulation");

            if last_tick.elapsed() >= tick_rate {
                self.tick();
                last_tick = Instant::now();
            }
            terminal.draw(|frame| {
                self.draw(frame);
            })?;
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) -> io::Result<()> {
        trace!("running the draw function");
        let size = frame.area();

        //  Split the screen horizontally into two main sections (top & bottom)
        let main_layout = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(size);

        //  Split the top section into two equal horizontal parts
        let top_layout = Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_layout[0]);

        let app_view = self.render_app_view();
        let fault_view = self.render_fault_log();
        let status_view = self.render_status_log();

        frame.render_widget(app_view, top_layout[0]);
        frame.render_widget(fault_view, top_layout[1]);
        frame.render_widget(status_view, main_layout[1]);

        Ok(())
    }

    fn render_app_view<'a>(&self) -> Paragraph<'a> {
        trace!("rendering the app view");
        let mut lines = vec![];
        let mut frame = vec![String::new(); 11];

        // Create a vector of characters we'll modify
        let mut display_chars: Vec<String> = vec!["   ".to_string(); 20];

        // Place robot in middle (position 10)
        display_chars[10] = "🤖".to_string();

        // Add attacks
        for (fault, pos) in &self.active_faults {
            let symbol = fault.to_symbol().to_string();
            let pos = *pos as usize;
            if pos < 5 {
                display_chars[pos] = symbol;
            }
        }

        // Join all characters into a single string
        frame[5] = display_chars.join("");
        lines.extend(frame);

        Paragraph::new(lines.join("\n"))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Application"))
    }

    fn render_fault_log<'a>(&self) -> Paragraph<'a> {
        trace!("rendering the fault log");
        let logs = self
            .fault_log
            .iter()
            .map(|msg| msg.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Paragraph::new(logs).block(Block::default().borders(Borders::ALL).title("Fault Log"))
    }

    fn render_status_log<'a>(&self) -> Paragraph<'a> {
        trace!("rendering the status log");

        let total_messages = self.status_log.len();
        let styled_statuses: Vec<Line> = self
            .status_log
            .iter()
            .enumerate()
            .map(|(idx, msg)| {
                // Calculate color intensity based on message age
                // Newer messages are brighter
                // let intensity = ((idx as f64 / total_messages as f64) * 200.0) as u8;
                let color = Color::Rgb(0, 255, 0);

                Line::styled(
                    format!(" ✅ {}", msg),
                    Style::default().fg(Color::Green).add_modifier(
                        if idx >= total_messages.saturating_sub(3) {
                            // Make newest messages bold
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        },
                    ),
                )
            })
            .collect();

        Paragraph::new(styled_statuses)
            .scroll(((self.status_log.len().saturating_sub(10)) as u16, 0)) // Auto-scroll to keep newest messages visible
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Operations")
                    .border_style(Style::default().fg(Color::Green)),
            )
    }
}
