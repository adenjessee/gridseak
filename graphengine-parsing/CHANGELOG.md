# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2024-01-XX

### Added
- Initial release of GraphEngine Parsing
- Clean architecture implementation with Domain, Application, and Infrastructure layers
- Multi-language support for Rust, JavaScript/TypeScript, and Python
- Tree-sitter based syntactic extraction for fast symbol discovery
- LSP-based semantic resolution for accurate relationship mapping
- SQLite-based persistent storage with querying capabilities
- Command-line interface with comprehensive commands
- Security features including LSP subprocess resource limits
- Comprehensive testing suite including E2E tests, fuzzing, and benchmarks
- YAML-based language configuration system
- Parallel processing with Rayon for performance
- Incremental parsing support with content hashing
- Error handling and logging throughout the system
- Documentation and examples

### Features
- **Domain Layer**: Pure models (Node, Edge, Graph, Provenance, Confidence) with validation
- **Application Layer**: Use cases and ports for orchestration
- **Infrastructure Layer**: Config, Syntax, Semantic, Storage, and Security adapters
- **CLI Commands**: parse, query, languages, stats
- **Security**: Process limits, sandboxing, monitoring, and resource protection
- **Performance**: Parallel processing, batched operations, memory efficiency
- **Testing**: E2E tests, fuzzing, benchmarks, failure simulation

### Technical Details
- Built with Rust for performance and reliability
- Follows the "1024 plan" for clean, maintainable architecture
- Uses async/await for concurrent operations
- Implements dependency inversion with trait-based ports
- Provides both programmatic and CLI interfaces
- Supports extensible language configurations
- Includes comprehensive error handling and logging

## [Unreleased]

### Planned
- Additional language support (Go, Java, C++)
- Graph visualization tools
- Incremental parsing optimizations
- Distributed processing support
- Plugin system for custom extractors
- Performance improvements and optimizations
- Enhanced security features
- Additional CLI commands and options
