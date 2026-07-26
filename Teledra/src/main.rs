use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

use teledra_agents::AgentId;
use teledra_brain::{Brain, BrainConfig, QWEN_MODEL};
use teledra_court::Court;

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
    println!("Teledra V2 | {} | {}", health.model, health.endpoint);

    if arguments.iter().any(|argument| argument == "--check") {
        println!("Qwen-only brain check passed.");
        return Ok(());
    }

    println!("Active court member: {}", court.active().title());
    println!("Type /help for commands.\n");

    let stdin = io::stdin();
    loop {
        print!("you> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            println!();
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        if handle_command(input, &mut court, &brain).await? {
            break;
        }
        if input.starts_with('/') {
            continue;
        }

        let prepared = court.prepare_turn(input);
        match brain
            .complete(&prepared.messages, prepared.temperature)
            .await
        {
            Ok(answer) => {
                println!("{}> {}\n", court.active(), answer);
                court.record_exchange(input, answer);
            }
            Err(error) => {
                eprintln!("Qwen request failed: {error}\n");
            }
        }
    }
    Ok(())
}

async fn handle_command(
    input: &str,
    court: &mut Court,
    brain: &Brain,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut parts = input.split_whitespace();
    let command = parts.next().unwrap_or_default();
    match command {
        "/quit" | "/exit" => return Ok(true),
        "/help" => {
            println!(
                "/roles        list court members\n\
                 /role NAME     enter one member's role-local session\n\
                 /clear         clear the active member's history\n\
                 /health        verify Ollama and {QWEN_MODEL}\n\
                 /quit          leave the court\n"
            );
        }
        "/roles" => {
            for agent in AgentId::ALL {
                let marker = if agent == court.active() { "*" } else { " " };
                println!("{marker} {:<10} {}", agent.slug(), agent.title());
            }
            println!();
        }
        "/role" => {
            let Some(name) = parts.next() else {
                eprintln!("Usage: /role NAME\n");
                return Ok(false);
            };
            match name.parse::<AgentId>() {
                Ok(agent) => {
                    court.select(agent);
                    println!("Entered the session of {}.\n", agent.title());
                }
                Err(error) => eprintln!("{error}. Use /roles to list valid names.\n"),
            }
        }
        "/clear" => {
            court.clear_active_history();
            println!("Cleared {}'s session history.\n", court.active().title());
        }
        "/health" => match brain.check().await {
            Ok(health) => println!("{} is ready at {}.\n", health.model, health.endpoint),
            Err(error) => eprintln!("Brain check failed: {error}\n"),
        },
        _ if input.starts_with('/') => {
            eprintln!("Unknown command. Type /help.\n");
        }
        _ => {}
    }
    Ok(false)
}
