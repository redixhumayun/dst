use super::Errors;
use super::IO;
use crate::game::*;
use ratatui::crossterm::event::{self, Event, KeyCode};
use std::time::{Duration, Instant};

async fn run_game(mut io: impl IO) -> Result<(), Box<dyn std::error::Error>> {
    let mut ui = Ui::new()?;
    let mut game_state = GameState::new();
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(100);

    loop {
        // Handle input
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }

        // Update game state
        if last_tick.elapsed() >= tick_rate {
            game_state.tick();
            last_tick = Instant::now();

            // Run one iteration of your simulation
            match run_simulation_step(&mut io, &mut game_state).await {
                Ok(_) => {
                    game_state.add_message("Processed message successfully".to_string());
                }
                Err(e) => {
                    let attack = match e {
                        Errors::KafkaConnectionError | Errors::NoKafkaMessage => {
                            Attack::KafkaFailure("Kafka connection failed".to_string())
                        }
                        Errors::RedisConnectionError | Errors::RedisKeyRetrievalError => {
                            Attack::RedisFailure("Redis error".to_string())
                        }
                        Errors::FileOpenError | Errors::FileReadError | Errors::FileWriteError => {
                            Attack::FileFailure("File error".to_string())
                        }
                        _ => Attack::FileFailure("Unknown error".to_string()),
                    };
                    game_state.add_attack(attack);
                    game_state.add_message(format!("Error: {:?}", e));
                }
            }
        }

        // Render
        ui.draw(&game_state)?;

        if game_state.is_dead {
            std::thread::sleep(Duration::from_secs(2));
            break;
        }
    }

    Ok(())
}

// Run a single step of your simulation
async fn run_simulation_step(io: &mut impl IO, game_state: &mut GameState) -> Result<(), Errors> {
    let kafka_message = io.read_kafka_message().await?;
    let redis_config = io.get_redis_config("config_key").await?;

    if let Some(message) = kafka_message {
        let output = format!("Config: {}, Message: {}\n", redis_config, message);
        io.write_to_file(&output).await?;
    }

    Ok(())
}
