# Contributing to GraphEngine Parsing

Thank you for your interest in contributing to GraphEngine Parsing! This document provides guidelines and information for contributors.

## Code of Conduct

This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct). Please be respectful and inclusive in all interactions.

## Getting Started

### Prerequisites

- Rust 1.80 or later
- Git
- A code editor with Rust support (VS Code with rust-analyzer recommended)

### Setting Up the Development Environment

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/your-username/graphengine-parsing.git
   cd graphengine-parsing
   ```

3. Build the project:
   ```bash
   cargo build
   ```

4. Run the tests:
   ```bash
   cargo test
   ```

## Architecture Overview

GraphEngine Parsing follows Clean Architecture principles with three main layers:

- **Domain Layer**: Pure business models and invariants
- **Application Layer**: Use cases and ports (traits)
- **Infrastructure Layer**: Concrete implementations

When contributing, please maintain this separation of concerns.

## Development Guidelines

### Code Style

- Follow Rust naming conventions
- Use `cargo fmt` to format code
- Use `cargo clippy` to check for linting issues
- Write comprehensive documentation for public APIs

### Testing

- Write unit tests for new functionality
- Add integration tests for complex workflows
- Ensure all tests pass before submitting
- Consider adding benchmarks for performance-critical code

### Documentation

- Update documentation for any API changes
- Add examples for new features
- Keep the README.md up to date
- Document any breaking changes

## Submitting Changes

### Pull Request Process

1. Create a feature branch from `main`:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. Make your changes and commit them:
   ```bash
   git add .
   git commit -m "Add your feature description"
   ```

3. Push your branch:
   ```bash
   git push origin feature/your-feature-name
   ```

4. Create a Pull Request on GitHub

### Commit Message Format

Use clear, descriptive commit messages:
- Use the imperative mood ("Add feature" not "Added feature")
- Keep the first line under 50 characters
- Provide more detail in the body if needed

### Pull Request Guidelines

- Provide a clear description of your changes
- Reference any related issues
- Ensure all tests pass
- Update documentation as needed
- Keep PRs focused and reasonably sized

## Areas for Contribution

### High Priority

- Additional language support (Go, Java, C++)
- Performance optimizations
- Enhanced error handling
- More comprehensive tests

### Medium Priority

- Graph visualization tools
- Plugin system for custom extractors
- Additional CLI commands
- Documentation improvements

### Low Priority

- Code style improvements
- Minor bug fixes
- Documentation typos

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test categories
cargo test --test e2e_tests
cargo test --test fuzzing_tests

# Run benchmarks
cargo bench
```

### Adding Tests

- Unit tests go in the same file as the code they test
- Integration tests go in the `tests/` directory
- E2E tests verify the complete pipeline
- Fuzzing tests ensure robustness against malformed input

## Performance Considerations

- Use `cargo bench` to measure performance impact
- Consider memory usage for large repositories
- Optimize hot paths in the parsing pipeline
- Profile with tools like `perf` or `flamegraph`

## Security

- Be mindful of security implications when adding new features
- Follow secure coding practices
- Consider input validation and sanitization
- Review LSP subprocess security measures

## Questions?

- Open an issue for questions or discussions
- Check existing issues and PRs for similar topics
- Join our community discussions (if available)

Thank you for contributing to GraphEngine Parsing!
