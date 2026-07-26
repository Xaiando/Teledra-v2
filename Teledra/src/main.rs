mod app;
mod terminal;
mod ui;

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use app::{App, AppAction, ModelReply};
use crossterm::event;
use teledra_brain::{Brain, BrainConfig};
use teledra_court::Court;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Teledra could not start: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let mut court = Court::default();
    if let Some(index) = arguments.iter().position(|argument| argument == "--role") {
        let requested = arguments
            .get(index + 1)
            .ok_or("--role requires a court member")?;
        court.select(requested.parse()?);
    }

    let brain = Brain::new(BrainConfig::default())?;
    let health = brain.check().await?;
    if arguments.iter().any(|argument| argument == "--check") {
        println!("Teledra V2 | {} | {}", health.model, health.endpoint);
        println!("Qwen-only brain check passed.");
        return Ok(());
    }

    let mut app = App::new(court, health);
    let mut terminal = terminal::TerminalSession::enter()?;
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<ModelReply>();

    loop {
        while let Ok(reply) = reply_rx.try_recv() {
            app.finish_turn(reply);
        }
        app.tick();
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(70))? {
            let event = event::read()?;
            match app.handle_event(event) {
                AppAction::Continue => {}
                AppAction::Quit => break,
                AppAction::Submit => {
                    let Some(request) = app.begin_turn() else {
                        continue;
                    };
                    let request_brain = brain.clone();
                    let request_tx = reply_tx.clone();
                    tokio::spawn(async move {
                        let result = request_brain
                            .complete(&request.messages, request.temperature)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = request_tx.send(ModelReply {
                            id: request.id,
                            agent: request.agent,
                            user: request.user,
                            result,
                        });
                    });
                }
            }
        }
    }

    Ok(())
}
