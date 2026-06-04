# ADR-001: Choosing a Multi-Crate Workspace

**Date**: 2025-06-27

**Status**: Accepted

## Context

The project aims to build a large, scalable, and long-lived system for code analysis. The architecture must be resilient to change and enforce a strong separation of concerns between core business logic and infrastructure details. 

The initial single-crate structure, while following Clean Architecture conventions, relies on developer discipline to prevent architectural violations (e.g., referencing a web framework from a business use case).

## Decision

We will refactor the project from a single-crate application into a Cargo Workspace with two distinct crates:

1.  **`core`**: A library crate containing the `domain` and `application` layers. This crate will contain the timeless business logic and will have zero dependencies on infrastructure-specific libraries.

2.  **`graphengine`**: A binary crate that depends on `core`. It will contain the `infrastructure` and `interfaces` layers, acting as the application host and implementing the traits defined in `core`.

This decision is foundational and aligns with **Pillar I: The Fortress** from our architectural vision.

## Consequences

### Positive:

-   **Compiler-Enforced Boundaries**: The primary benefit. It becomes a compile-time error to violate the Dependency Rule, making the architecture robust and self-enforcing.
-   **Improved Modularity & Reusability**: The `core` crate becomes a standalone, reusable asset that could be used by other applications (e.g., a CLI) without pulling in web-server dependencies.
-   **Clarity of Purpose**: The structure of the workspace screams the intent of the architecture. It is immediately clear what is core logic and what is implementation detail.
-   **Potential for Faster Compile Times**: Changes in the `graphengine` crate (e.g., tweaking an API handler) will not require recompiling the entire `core` crate.

### Negative:

-   **Slightly Increased Complexity**: Developers new to Rust workspaces may face a small initial learning curve.
-   **More Boilerplate for Dependencies**: `Cargo.toml` files must be managed in two places. Workspace dependencies can mitigate this.

We accept these minor negative consequences as a worthwhile trade-off for the immense gain in architectural integrity and long-term maintainability.
