# GraphEngine Parsing

A production-ready, language-agnostic code parsing system that extracts semantic graphs from source code repositories. Built with Rust for performance and reliability, following clean architecture principles.

## Overview

GraphEngine Parsing implements the "1024 plan" - a comprehensive refactoring that creates a modular, extensible system for parsing code into semantic graphs. The system uses a two-phase approach:

1. **FAST**: Tree-sitter based syntactic extraction for quick symbol discovery
2. **SEM**: LSP-based semantic resolution for accurate relationship mapping

## Features

- **Multi-language Support**: Rust, JavaScript/TypeScript, Python (extensible)
- **Clean Architecture**: Domain, Application, and Infrastructure layers
- **Production Ready**: Comprehensive error handling, logging, and security features
- **CLI Interface**: Command-line tool for parsing and querying
- **Persistent Storage**: SQLite-based graph storage with querying capabilities
- **Security**: LSP subprocess resource limits and sandboxing
- **Testing**: E2E tests, fuzzing, benchmarks, and failure simulation

## Architecture

The system follows clean architecture principles with clear separation of concerns:

### Domain Layer
- Pure models: `Node`, `Edge`, `Graph`, `Provenance`, `Confidence`
- Business invariants and validation
- Language-agnostic abstractions

### Application Layer
- Use cases: `ParseRepositoryUseCase`
- Ports (traits): `SyntaxExtractor`, `SemanticResolver`, `GraphRepository`
- Orchestration and business logic

### Infrastructure Layer
- **Config**: YAML-based language configurations
- **Syntax**: Tree-sitter based extractors
- **Semantic**: LSP-based resolvers
- **Storage**: SQLite repositories
- **Security**: Process limits and monitoring

## Installation

```bash
# Clone the repository
git clone <repository-url>
cd graphengine-parsing

# Build the project
cargo build --release

# Install the CLI tool
cargo install --path .
```

## Usage

### CLI Commands

#### Parse a Repository
```bash
graphengine-parsing parse \
  --root /path/to/repo \
  --lang rust \
  --db /path/to/database.db \
  --min-confidence Medium \
  --output-json graph.json
```

#### Query the Database
```bash
# List all nodes
graphengine-parsing query --db database.db --query-type list-nodes

# Find call relationships
graphengine-parsing query --db database.db --query-type calls-from --params node_id
```

#### List Supported Languages
```bash
graphengine-parsing languages
```

#### Database Statistics
```bash
graphengine-parsing stats --db database.db
```

### Programmatic Usage

```rust
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::Confidence;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create use case with SQLite storage
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        "rust".to_string(),
        Confidence::Medium,
        "database.db",
    ).await?;

    // Parse repository
    let graph = use_case.parse(
        std::path::PathBuf::from("/path/to/repo"),
        "rust".to_string(),
    ).await?;

    println!("Parsed {} nodes and {} edges", 
             graph.graph().node_count(), 
             graph.graph().edge_count());

    Ok(())
}
```

## Configuration

Language configurations are stored in YAML files under `configs/`. Each configuration defines:

- File extensions
- LSP server commands
- Tree-sitter queries for symbol extraction
- Kind mappings to universal domain models

Example Rust configuration:
```yaml
language: rust
file_extensions:
  - ".rs"
lsp_command: "rust-analyzer"
lsp_args:
  - "--stdio"
queries:
  functions: |
    (function_item
      name: (identifier) @name
      parameters: (parameters) @params
    ) @func
kind_mappings:
  function_item: Function
  struct_item: Struct
```

## Testing

The project includes comprehensive testing:

```bash
# Run all tests
cargo test

# Run E2E tests
cargo test --test e2e_tests

# Run fuzzing tests
cargo test --test fuzzing_tests

# Run benchmarks
cargo bench
```

## Security Features

- **Process Limits**: LSP subprocesses are limited in CPU, memory, and file usage
- **Sandboxing**: Isolated execution environments for language servers
- **Monitoring**: Health checks and automatic restart capabilities
- **Resource Protection**: Prevents resource exhaustion attacks

## Performance

- **Parallel Processing**: Multi-threaded file parsing using Rayon
- **Incremental Parsing**: Skip unchanged files using content hashing
- **Batched Operations**: Efficient database operations with transactions
- **Memory Efficient**: Streaming processing for large repositories

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests for new functionality
5. Run the test suite
6. Submit a pull request

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Roadmap

- [ ] Additional language support (Go, Java, C++)
- [ ] Graph visualization tools
- [ ] Incremental parsing optimizations
- [ ] Distributed processing support
- [ ] Plugin system for custom extractors

## Support

For questions, issues, or contributions, please:
- Open an issue on GitHub
- Check the documentation
- Review the test examples

## Acknowledgments

Built following the "1024 plan" for clean, maintainable code architecture. Inspired by modern language server protocols and semantic analysis techniques.