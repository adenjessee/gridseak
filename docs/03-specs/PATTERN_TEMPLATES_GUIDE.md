# Pattern Templates Guide

This document provides comprehensive documentation for the JSON-based pattern template system used in GraphEngine for detecting design patterns, anti-patterns, and architectural smells.

## Overview

The pattern template system allows for dynamic, configurable pattern detection without hardcoding detection logic. Templates are defined in JSON format and can be loaded, updated, and versioned at runtime.

## Template Structure

### Basic Template Schema

```json
{
  "pattern_name": "string",
  "version": "string",
  "description": "string",
  "category": "DesignPattern|AntiPattern|ArchitecturalSmell",
  "detection_rules": [...],
  "impact_analysis": {...},
  "recommendations": [...],
  "metadata": {...},
  "version_history": [...]
}
```

### Field Descriptions

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `pattern_name` | string | Yes | Unique identifier for the pattern |
| `version` | string | Yes | Semantic version (e.g., "1.0", "2.1") |
| `description` | string | Yes | Human-readable description |
| `category` | enum | Yes | Pattern category |
| `detection_rules` | array | Yes | Array of detection rules |
| `impact_analysis` | object | Yes | Impact assessment |
| `recommendations` | array | Yes | Improvement suggestions |
| `metadata` | object | Yes | Template metadata |
| `version_history` | array | Yes | Version tracking information |

## Detection Rules

### Rule Structure

```json
{
  "rule_type": "Structural|Behavioral|Metric|Semantic",
  "conditions": [...],
  "min_confidence": 0.8,
  "weight": 1.0
}
```

### Rule Types

- **Structural**: Based on code structure (classes, methods, fields)
- **Behavioral**: Based on runtime behavior patterns
- **Metric**: Based on calculated metrics (complexity, coupling, etc.)
- **Semantic**: Based on naming conventions and semantic meaning

### Conditions

```json
{
  "field": "string",
  "value": "boolean|number|string|range|list",
  "weight": 1.0,
  "description": "string"
}
```

### Condition Value Types

#### Boolean
```json
{
  "field": "has_private_constructor",
  "value": true,
  "weight": 0.4,
  "description": "Class has private constructor"
}
```

#### Number
```json
{
  "field": "method_count",
  "value": 20,
  "weight": 0.3,
  "description": "Class has 20 or more methods"
}
```

#### Range
```json
{
  "field": "cyclomatic_complexity",
  "value": {"min": 10, "max": 50},
  "weight": 0.5,
  "description": "Function complexity between 10 and 50"
}
```

#### String
```json
{
  "field": "class_name",
  "value": "Manager",
  "weight": 0.2,
  "description": "Class name contains 'Manager'"
}
```

#### List
```json
{
  "field": "method_names",
  "value": ["getInstance", "getInstance", "getInstance"],
  "weight": 0.3,
  "description": "Method name is in the list"
}
```

## Impact Analysis

```json
{
  "maintainability": 0.5,
  "performance": 0.5,
  "scalability": 0.5,
  "testability": 0.5,
  "reusability": 0.5
}
```

Impact scores range from 0.0 (positive impact) to 1.0 (negative impact).

## Metadata

```json
{
  "author": "string",
  "created_at": "ISO-8601 timestamp",
  "modified_at": "ISO-8601 timestamp",
  "tags": ["string"],
  "priority": "high|medium|low",
  "status": "Active|Deprecated|Experimental|Draft"
}
```

## Version History

```json
[
  {
    "version": "1.0",
    "description": "Initial version",
    "changes": ["Initial template creation"],
    "author": "string",
    "created_at": "ISO-8601 timestamp",
    "deprecated": false,
    "migration_notes": "string|null"
  }
]
```

## Example Templates

### Singleton Pattern

```json
{
  "pattern_name": "Singleton",
  "version": "1.0",
  "description": "Ensures a class has only one instance",
  "category": "DesignPattern",
  "detection_rules": [
    {
      "rule_type": "Structural",
      "conditions": [
        {
          "field": "has_private_constructor",
          "value": true,
          "weight": 0.4,
          "description": "Class has private constructor"
        },
        {
          "field": "has_static_instance_field",
          "value": true,
          "weight": 0.3,
          "description": "Class has static instance field"
        },
        {
          "field": "has_static_get_instance_method",
          "value": true,
          "weight": 0.3,
          "description": "Class has static get instance method"
        }
      ],
      "min_confidence": 0.8,
      "weight": 1.0
    }
  ],
  "impact_analysis": {
    "maintainability": 0.3,
    "performance": 0.9,
    "testability": 0.2,
    "scalability": 0.1,
    "reusability": 0.4
  },
  "recommendations": [
    "Consider if singleton is really needed",
    "Use dependency injection for better testability",
    "Consider thread safety for concurrent access"
  ],
  "metadata": {
    "author": "GraphEngine",
    "created_at": "2025-01-27T00:00:00Z",
    "modified_at": "2025-01-27T00:00:00Z",
    "tags": ["creational", "singleton"],
    "priority": "medium",
    "status": "Active"
  },
  "version_history": [
    {
      "version": "1.0",
      "description": "Initial version",
      "changes": ["Initial template creation"],
      "author": "GraphEngine",
      "created_at": "2025-01-27T00:00:00Z",
      "deprecated": false,
      "migration_notes": null
    }
  ]
}
```

### God Class Anti-Pattern

```json
{
  "pattern_name": "GodClass",
  "version": "1.0",
  "description": "Class that knows too much or does too much",
  "category": "AntiPattern",
  "detection_rules": [
    {
      "rule_type": "Metric",
      "conditions": [
        {
          "field": "method_count",
          "value": {"min": 20, "max": 999999},
          "weight": 0.4,
          "description": "Class has too many methods"
        },
        {
          "field": "line_count",
          "value": {"min": 500, "max": 999999},
          "weight": 0.3,
          "description": "Class has too many lines"
        },
        {
          "field": "dependency_count",
          "value": {"min": 10, "max": 999999},
          "weight": 0.3,
          "description": "Class has too many dependencies"
        }
      ],
      "min_confidence": 0.7,
      "weight": 1.0
    }
  ],
  "impact_analysis": {
    "maintainability": 0.9,
    "performance": 0.3,
    "testability": 0.8,
    "scalability": 0.7,
    "reusability": 0.6
  },
  "recommendations": [
    "Break down the class into smaller, focused classes",
    "Apply Single Responsibility Principle",
    "Extract related functionality into separate classes"
  ],
  "metadata": {
    "author": "GraphEngine",
    "created_at": "2025-01-27T00:00:00Z",
    "modified_at": "2025-01-27T00:00:00Z",
    "tags": ["anti-pattern", "god-class"],
    "priority": "high",
    "status": "Active"
  },
  "version_history": [
    {
      "version": "1.0",
      "description": "Initial version",
      "changes": ["Initial template creation"],
      "author": "GraphEngine",
      "created_at": "2025-01-27T00:00:00Z",
      "deprecated": false,
      "migration_notes": null
    }
  ]
}
```

### High Coupling Architectural Smell

```json
{
  "pattern_name": "HighCoupling",
  "version": "1.0",
  "description": "Class has too many dependencies on other classes",
  "category": "ArchitecturalSmell",
  "detection_rules": [
    {
      "rule_type": "Metric",
      "conditions": [
        {
          "field": "afferent_coupling",
          "value": {"min": 10, "max": 999999},
          "weight": 0.5,
          "description": "High afferent coupling"
        },
        {
          "field": "efferent_coupling",
          "value": {"min": 10, "max": 999999},
          "weight": 0.5,
          "description": "High efferent coupling"
        }
      ],
      "min_confidence": 0.6,
      "weight": 1.0
    }
  ],
  "impact_analysis": {
    "maintainability": 0.8,
    "performance": 0.4,
    "testability": 0.7,
    "scalability": 0.6,
    "reusability": 0.5
  },
  "recommendations": [
    "Reduce dependencies by applying Dependency Inversion Principle",
    "Use interfaces to decouple classes",
    "Consider breaking down the class"
  ],
  "metadata": {
    "author": "GraphEngine",
    "created_at": "2025-01-27T00:00:00Z",
    "modified_at": "2025-01-27T00:00:00Z",
    "tags": ["architectural-smell", "coupling"],
    "priority": "high",
    "status": "Active"
  },
  "version_history": [
    {
      "version": "1.0",
      "description": "Initial version",
      "changes": ["Initial template creation"],
      "author": "GraphEngine",
      "created_at": "2025-01-27T00:00:00Z",
      "deprecated": false,
      "migration_notes": null
    }
  ]
}
```

## Available Fields

### AST-Based Fields

These fields are extracted from the Abstract Syntax Tree:

- `type`: Node type (function, class, method, etc.)
- `name`: Node name
- `visibility`: Access modifier (public, private, protected)
- `is_static`: Whether the element is static
- `is_abstract`: Whether the element is abstract
- `is_final`: Whether the element is final
- `parameters`: Number of parameters
- `return_type`: Return type
- `line_count`: Number of lines
- `nesting_level`: Nesting depth

### Structural Fields

- `has_private_constructor`: Boolean
- `has_static_instance_field`: Boolean
- `has_static_get_instance_method`: Boolean
- `method_count`: Number
- `field_count`: Number
- `dependency_count`: Number
- `import_count`: Number
- `export_count`: Number

### Metric Fields

- `cyclomatic_complexity`: Number
- `cognitive_complexity`: Number
- `afferent_coupling`: Number
- `efferent_coupling`: Number
- `instability`: Number
- `abstractness`: Number
- `distance_from_main_sequence`: Number

## Template Management

### Loading Templates

```rust
use graphengine_infra::analysis::PatternTemplateLoader;

let mut loader = PatternTemplateLoader::new();

// Load from JSON string
loader.load_template_from_json(template_json)?;

// Load from file
loader.load_template_from_file("templates/singleton.json")?;

// Load from directory
loader.load_templates_from_directory("templates/")?;
```

### Template Operations

```rust
// Get all templates
let templates = loader.get_all_templates();

// Get template by name
let singleton = loader.get_template("Singleton");

// Get templates by category
let design_patterns = loader.get_templates_by_category(&PatternCategory::DesignPattern);

// Get active templates only
let active_templates = loader.get_active_templates();

// Deprecate a template
loader.deprecate_template("OldPattern", "1.0", "Better alternative available")?;

// Remove a template
loader.remove_template("UnusedPattern");
```

### Evolution Tracking

```rust
// Get evolution history for a template
let history = loader.get_evolution_history("Singleton");

// Get all evolution history
let all_history = loader.get_all_evolution_history();
```

## Best Practices

### Template Design

1. **Start Simple**: Begin with basic structural conditions
2. **Use Multiple Rules**: Combine different rule types for better accuracy
3. **Weight Appropriately**: Assign weights based on importance
4. **Set Realistic Thresholds**: Use min_confidence to avoid false positives
5. **Document Clearly**: Provide clear descriptions and recommendations

### Field Selection

1. **Choose Relevant Fields**: Use fields that actually indicate the pattern
2. **Consider Language Differences**: Some fields may not apply to all languages
3. **Test Thoroughly**: Validate templates against known examples
4. **Iterate**: Refine templates based on detection results

### Version Management

1. **Use Semantic Versioning**: Follow semver for version numbers
2. **Document Changes**: Always update version history
3. **Provide Migration Notes**: Help users upgrade templates
4. **Deprecate Gracefully**: Mark old versions as deprecated

## Validation

Templates are automatically validated when loaded:

- Pattern name must not be empty
- At least one detection rule is required
- All conditions must have valid field names
- Confidence and weight values must be between 0.0 and 1.0
- Metadata must include required fields

## Error Handling

Common validation errors:

- `Pattern name cannot be empty`
- `Template must have at least one detection rule`
- `Detection rule must have at least one condition`
- `Min confidence must be between 0.0 and 1.0`
- `Rule weight must be between 0.0 and 1.0`

## Integration with API

The template system is exposed through REST API endpoints:

- `GET /templates` - List all templates
- `POST /templates` - Create new template
- `GET /templates/{name}` - Get specific template
- `PUT /templates/{name}` - Update template
- `DELETE /templates/{name}` - Delete template
- `POST /templates/load-from-file` - Load from file
- `POST /templates/load-from-directory` - Load from directory

## Performance Considerations

- Templates are cached in memory for fast access
- Detection results are cached to avoid re-computation
- Large template directories may impact startup time
- Consider lazy loading for rarely-used templates

## Future Enhancements

- Template inheritance and composition
- Machine learning-based template generation
- Template sharing and marketplace
- Advanced condition types (regex, custom functions)
- Template performance profiling
- A/B testing for template effectiveness 