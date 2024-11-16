use ratatui::crossterm;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use std::collections::VecDeque;
use std::time::Duration;

// Track different types of attacks (faults) on the application
#[derive(Debug, Clone)]
pub enum Attack {
    KafkaFailure(String),
    RedisFailure(String),
    FileFailure(String),
}

impl Attack {
    fn get_symbol(&self) -> &str {
        match self {
            Attack::KafkaFailure(_) => "⚡", // Lightning for Kafka
            Attack::RedisFailure(_) => "🔥", // Fire for Redis
            Attack::FileFailure(_) => "💥",  // Explosion for File
        }
    }
}

// Represents the current game state
pub struct GameState {
    health: u8,                       // Application health (0-100)
    active_attacks: VecDeque<Attack>, // Currently visible attacks
    messages: VecDeque<String>,       // Log messages
    tick_count: u64,                  // For animation timing
    pub is_dead: bool,                // If the application has crashed
}

impl GameState {
    pub fn new() -> Self {
        Self {
            health: 100,
            active_attacks: VecDeque::with_capacity(10),
            messages: VecDeque::with_capacity(50),
            tick_count: 0,
            is_dead: false,
        }
    }

    pub fn add_attack(&mut self, attack: Attack) {
        self.active_attacks.push_back(attack);
        self.health = self.health.saturating_sub(10);
        if self.health == 0 {
            self.is_dead = true;
        }
    }

    pub fn add_message(&mut self, message: String) {
        self.messages.push_back(message);
        if self.messages.len() > 50 {
            self.messages.pop_front();
        }
    }

    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        // Remove old attacks
        while self.active_attacks.len() > 5 {
            self.active_attacks.pop_front();
        }
    }
}

// UI state for handling terminal interface
pub struct Ui {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl Ui {
    pub fn new() -> Result<Self, std::io::Error> {
        let mut stdout = std::io::stdout();
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn draw(&mut self, game_state: &GameState) -> Result<(), std::io::Error> {
        self.terminal.draw(|frame| {
            let size = frame.size();

            // Split the screen into three sections
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(20), // Status bar
                    Constraint::Percentage(50), // Main game area
                    Constraint::Percentage(30), // Log messages
                ])
                .split(size);

            // Draw status bar
            let health_info = format!(
                "Health: {}% | Attacks: {}",
                game_state.health,
                game_state.active_attacks.len()
            );
            let status = Paragraph::new(health_info)
                .block(Block::default().borders(Borders::ALL).title("Status"));
            frame.render_widget(status, chunks[0]);

            // Draw main game area with application and attacks
            let game_area = self.render_game_area(game_state);
            frame.render_widget(game_area, chunks[1]);

            // Draw log messages
            let messages = game_state
                .messages
                .iter()
                .map(|m| Line::from(m.as_str()))
                .collect::<Vec<_>>();
            let log =
                Paragraph::new(messages).block(Block::default().borders(Borders::ALL).title("Log"));
            frame.render_widget(log, chunks[2]);
        })?;
        Ok(())
    }

    fn render_game_area<'a>(&self, game_state: &GameState) -> Paragraph<'a> {
        let mut content = if game_state.is_dead {
            vec![
                "   Game Over!   ",
                "              ",
                "     💀      ",
                "              ",
                " Press q to quit ",
            ]
        } else {
            vec![
                "              ",
                "     🤖      ", // Application representation
                "              ",
            ]
        };

        // Add active attacks
        for attack in &game_state.active_attacks {
            content.push(&format!("  {} ", attack.get_symbol()));
        }

        Paragraph::new(content.join("\n"))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Application"))
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::LeaveAlternateScreen
        );
    }
}
