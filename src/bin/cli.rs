use clap::{Parser, Subcommand};
use futures::StreamExt;
use sqlx::SqlitePool;
use std::io::Write;
use std::sync::Arc;
use yalr::{
    api, config, db::{Database, Provider}, metrics, providers::openai::OpenAiProvider, ChatCompletionRequest,
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, Router,
};

#[derive(Parser)]
#[command(name = "yalr-cli")]
#[command(about = "YALR CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the router server
    Serve {
        #[arg(long, default_value = "0.0.0.0:3000")]
        addr: String,
    },
    /// Show version info
    Version,
    /// Check configuration
    CheckConfig,
    /// Manage providers
    #[command(subcommand)]
    Provider(ProviderCommands),
    /// Chat with the router
    Chat {
        /// Routing strategy to use
        #[arg(long, default_value = "round_robin")]
        strategy: String,
        /// Initial message (optional - starts interactive mode anyway)
        #[arg(long)]
        message: Option<String>,
        /// Model name
        #[arg(long, default_value = "default")]
        model: String,
    },
}

#[derive(Subcommand)]
enum ProviderCommands {
    /// List all providers
    List,
    /// Add a new provider
    Add {
        /// Provider name
        #[arg(long)]
        name: String,
        /// Provider slug (URL-safe identifier)
        #[arg(long)]
        slug: String,
        /// Provider base URL
        #[arg(long)]
        base_url: String,
        /// API key for the provider
        #[arg(long)]
        api_key: String,
    },
    /// Remove a provider
    Remove {
        /// Provider name to remove
        #[arg(long)]
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { addr } => {
            let (emitter, _receiver) = metrics::MetricsEmitter::new(10000);
            let metrics_store = metrics::MetricsStore::new(emitter.clone(), 10000);

            let config = config::AppConfig::load(metrics_store.clone())
                .await
                .expect("Failed to load config");
            config
                .load_providers()
                .await
                .expect("Failed to load providers");

            tracing::info!("Starting YALR on {}", addr);

            api::server::run(config, &addr, emitter, metrics_store).await?;
        }
        Commands::Version => {
            println!("yalr-cli 0.1.0");
        }
        Commands::CheckConfig => {
            let (emitter, _receiver) = metrics::MetricsEmitter::new(10000);
            let metrics_store = metrics::MetricsStore::new(emitter.clone(), 10000);
            let _config = config::AppConfig::load(metrics_store.clone())
                .await
                .expect("Failed to load config");
            println!("Configuration loaded successfully");
            println!("Database configured");
            println!("Router configured");
        }
        Commands::Provider(provider_cmd) => {
            let (emitter, _receiver) = metrics::MetricsEmitter::new(10000);
            let metrics_store = metrics::MetricsStore::new(emitter.clone(), 10000);
            let config = config::AppConfig::load(metrics_store.clone())
                .await
                .expect("Failed to load config");
            let db = config.db;

            match provider_cmd {
                ProviderCommands::List => list_providers(&db.pool).await,
                ProviderCommands::Add {
                    name,
                    slug,
                    base_url,
                    api_key,
                } => {
                    add_provider(&db.pool, &name, &slug, &base_url, &api_key).await;
                }
                ProviderCommands::Remove { name } => {
                    remove_provider(&db.pool, &name).await;
                }
            }
        }
        Commands::Chat {
            strategy,
            message,
            model,
        } => {
            let (emitter, _receiver) = metrics::MetricsEmitter::new(10000);
            let metrics_store = metrics::MetricsStore::new(emitter.clone(), 10000);
            let config = config::AppConfig::load(metrics_store.clone())
                .await
                .expect("Failed to load config");
            let db = config.db;
            chat_with_providers(
                &db.pool,
                &strategy,
                message.as_deref().unwrap_or(""),
                &model,
            )
            .await;
        }
    }

    Ok(())
}

async fn list_providers(pool: &SqlitePool) {
    let providers = sqlx::query_as::<_, Provider>(
        "SELECT id, name, slug, base_url, api_key, created_at, updated_at FROM providers ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("Failed to fetch providers");

    if providers.is_empty() {
        println!("No providers configured");
        return;
    }

    println!("{:<20} {:<20} {:<50}", "NAME", "SLUG", "BASE_URL");
    println!("{}", "-".repeat(100));
    for provider in providers {
        println!(
            "{:<20} {:<20} {:<50}",
            provider.name, provider.slug, provider.base_url
        );
    }
}

async fn add_provider(pool: &SqlitePool, name: &str, slug: &str, base_url: &str, api_key: &str) {
    match sqlx::query("INSERT INTO providers (name, slug, base_url, api_key) VALUES (?, ?, ?, ?)")
        .bind(name)
        .bind(slug)
        .bind(base_url)
        .bind(api_key)
        .execute(pool)
        .await
    {
        Ok(_) => println!("Provider '{}' added successfully", name),
        Err(e) => eprintln!("Failed to add provider: {}", e),
    }
}

async fn remove_provider(pool: &SqlitePool, name: &str) {
    match sqlx::query("DELETE FROM providers WHERE name = ?")
        .bind(name)
        .execute(pool)
        .await
    {
        Ok(result) => {
            if result.rows_affected() > 0 {
                println!("Provider '{}' removed successfully", name);
            } else {
                println!("Provider '{}' not found", name);
            }
        }
        Err(e) => eprintln!("Failed to remove provider: {}", e),
    }
}

fn create_user_message(content: &str) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: ChatCompletionRequestUserMessageContent::Text(content.to_string()),
        name: None,
    })
}

fn create_assistant_message(content: &str) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
        content: Some(ChatCompletionRequestAssistantMessageContent::Text(
            content.to_string(),
        )),
        refusal: None,
        name: None,
        tool_calls: None,
        audio: None,
        ..Default::default()
    })
}

async fn chat_with_providers(pool: &SqlitePool, strategy: &str, message: &str, model: &str) {
    let providers = sqlx::query_as::<_, Provider>(
        "SELECT id, name, slug, base_url, api_key, created_at, updated_at FROM providers ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("Failed to fetch providers");

    if providers.is_empty() {
        println!("No providers configured");
        return;
    }

    let (emitter, _receiver) = metrics::MetricsEmitter::new(10000);
    let metrics_store = metrics::MetricsStore::new(emitter.clone(), 10000);

    let strategy: Box<dyn yalr::router::strategies::RoutingStrategy> = match strategy {
        "round_robin" => Box::new(yalr::router::strategies::round_robin::RoundRobinStrategy::new()),
        _ => {
            eprintln!("Unknown strategy: {}. Using round_robin.", strategy);
            Box::new(yalr::router::strategies::round_robin::RoundRobinStrategy::new())
        }
    };

    let strategy_name = strategy.name().to_string();
    let router = Router::new(strategy, metrics_store);

    for provider_record in &providers {
        let provider = Arc::new(OpenAiProvider::new(
            &provider_record.name,
            Some(&provider_record.slug),
            &provider_record.base_url,
            provider_record.api_key.as_deref(),
        ));
        router.add_provider(provider).await;
    }

    println!("Using router (strategy: {})", strategy_name);
    println!("Type 'exit' or 'quit' to end the conversation.");
    println!("{}", "-".repeat(50));

    let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();

    // If an initial message was provided, use it
    if !message.is_empty() {
        messages.push(create_user_message(message));
        println!("Initial message: {}", message);
    }

    let stdin = std::io::stdin();
    let mut input = String::new();

    // If we started with a message, process it first
    let mut first_input = !message.is_empty();

    loop {
        let user_input = if first_input {
            first_input = false;
            input = message.to_string();
            input.as_str()
        } else {
            print!("\n> ");
            let _ = std::io::stdout().flush();
            input.clear();
            if stdin.read_line(&mut input).is_err() {
                break;
            }
            input.trim()
        };

        if user_input.eq_ignore_ascii_case("exit") || user_input.eq_ignore_ascii_case("quit") {
            println!("Goodbye!");
            break;
        }

        if user_input.is_empty() {
            continue;
        }

        messages.push(create_user_message(user_input));

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: messages.clone(),
            stream: Some(true),
            ..Default::default()
        };

        print!("Assistant: ");
        let _ = std::io::stdout().flush();

        match router.chat_completions_stream(&request).await {
            Ok(stream) => {
                let mut response_text = String::new();
                let mut first_token = true;
                let mut pinned_stream = Box::pin(stream);

                while let Some(result) = pinned_stream.next().await {
                    match result {
                        Ok(chunk) => {
                            if let Some(choice) = chunk.choices.first() {
                                if first_token {
                                    first_token = false;
                                    print!("\rAssistant: ");
                                    let _ = std::io::stdout().flush();
                                }
                                if let Some(content) = &choice.delta.content {
                                    print!("{}", content);
                                    let _ = std::io::stdout().flush();
                                    response_text.push_str(content);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("\nError: {}", e);
                            break;
                        }
                    }
                }

                if !response_text.is_empty() {
                    messages.push(create_assistant_message(&response_text));
                }
                println!();
            }
            Err(e) => {
                eprintln!("\nError: {}", e);
                messages.pop();
            }
        }
    }
}
