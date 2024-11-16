use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use color_eyre::Result;
use rand::RngCore;
use ratatui::{
    crossterm::event::{self, Event, KeyCode},
    layout::{Alignment, Constraint, Layout},
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};
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
                    for fault in faults {
                        self.add_fault(fault);
                    }
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
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(size);

        // Left panel: Application and attacks
        trace!("going to render the left panel");
        let app_view = self.render_app_view();
        frame.render_widget(app_view, chunks[0]);

        // Right panel: Fault log
        trace!("going to render the right panel");
        let log_view = self.render_fault_log();
        frame.render_widget(log_view, chunks[1]);

        Ok(())
    }

    fn render_app_view<'a>(&self) -> Paragraph<'a> {
        let mut lines = vec![];
        let mut frame = vec![String::new(); 11];

        // Create a base line with spaces
        let base_line = "          🤖          ".to_string(); // More spaces for positioning
        frame[5] = base_line;

        // Add each active fault to the animation
        for (fault, pos) in &self.active_faults {
            let symbol = fault.to_symbol();
            let pos = *pos as usize;
            if pos < 5 {
                // Calculate proper position for the symbol
                let insert_pos = pos * 3; // Multiply by 3 to account for spacing
                if insert_pos + symbol.len() <= frame[5].len() {
                    frame[5].replace_range(insert_pos..insert_pos + symbol.len(), symbol);
                }
            }
        }

        lines.extend(frame);

        Paragraph::new(lines.join("\n"))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Application"))
    }

    fn render_fault_log<'a>(&self) -> Paragraph<'a> {
        let logs = self
            .fault_log
            .iter()
            .map(|msg| msg.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Paragraph::new(logs).block(Block::default().borders(Borders::ALL).title("Fault Log"))
    }
}
