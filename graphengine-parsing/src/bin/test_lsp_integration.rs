//! Test LSP integration with a simple example

use graphengine_parsing::domain::Range;
use graphengine_parsing::infrastructure::config::LanguageConfig;
use graphengine_parsing::infrastructure::lsp::session::SessionSupervisor;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("Testing LSP Integration");

    // Create a simple Rust language config
    let config = LanguageConfig {
        language: "rust".to_string(),
        file_extensions: vec![".rs".to_string()],
        queries: HashMap::new(),
        kind_mappings: HashMap::new(),
        grammar_path: None,
        lsp_command: Some("rust-analyzer".to_string()),
        lsp_args: Some(vec!["--stdio".to_string()]),
        version: "1.0".to_string(),
        receiver_type_detection: None,
        lsp_request_timeout_ms: None,
        lsp_max_concurrent_requests: None,
        lsp_initialization_options: None,
    };

    // Create session supervisor
    let supervisor = SessionSupervisor::new(config, None);

    println!("Created LSP session supervisor");

    // Test initialization
    match supervisor.initialize().await {
        Ok(_) => {
            println!("✅ LSP session initialized successfully");

            // Test if ready
            if supervisor.is_ready().await {
                println!("✅ LSP session is ready");

                // Test definition lookup
                let test_range = Range::with_file(1, 0, 1, 10, "test.rs".to_string());
                match supervisor
                    .find_definition("test_function", &test_range)
                    .await
                {
                    Ok(Some(definition)) => {
                        println!(
                            "✅ Found definition in {} at lines {}-{}",
                            definition.file, definition.start_line, definition.end_line
                        );
                    }
                    Ok(None) => {
                        println!("ℹ️  No definition found (expected for test function)");
                    }
                    Err(e) => {
                        println!("⚠️  Definition lookup failed: {}", e);
                    }
                }
            } else {
                println!("❌ LSP session is not ready");
            }
        }
        Err(e) => {
            println!("❌ LSP session initialization failed: {}", e);
            println!("This is expected if rust-analyzer is not installed");
        }
    }

    println!("LSP integration test complete");
    Ok(())
}
