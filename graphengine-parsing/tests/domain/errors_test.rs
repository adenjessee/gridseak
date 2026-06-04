//! Tests for ValidationError - TDD approach
//! 
//! Tests written first to define the expected behavior of our error types.

use graphengine_parsing::domain::ValidationError;

#[test]
fn test_dangling_edge_error_display() {
    let error = ValidationError::DanglingEdge {
        from_id: "node_a".to_string(),
        to_id: "node_b".to_string(),
    };
    
    assert_eq!(
        error.to_string(),
        "Dangling edge: from node_a to node_b"
    );
}

#[test]
fn test_low_confidence_error_display() {
    let error = ValidationError::LowConfidence(5);
    
    assert_eq!(
        error.to_string(),
        "Low confidence: 5 elements below threshold"
    );
}

#[test]
fn test_invalid_kind_error_display() {
    let error = ValidationError::InvalidKind("UnknownType".to_string());
    
    assert_eq!(
        error.to_string(),
        "Invalid kind: UnknownType"
    );
}

#[test]
fn test_error_equality() {
    let error1 = ValidationError::DanglingEdge {
        from_id: "a".to_string(),
        to_id: "b".to_string(),
    };
    let error2 = ValidationError::DanglingEdge {
        from_id: "a".to_string(),
        to_id: "b".to_string(),
    };
    let error3 = ValidationError::DanglingEdge {
        from_id: "x".to_string(),
        to_id: "y".to_string(),
    };
    
    assert_eq!(error1, error2);
    assert_ne!(error1, error3);
}

#[test]
fn test_error_debug() {
    let error = ValidationError::LowConfidence(3);
    let debug_str = format!("{:?}", error);
    
    assert!(debug_str.contains("LowConfidence"));
    assert!(debug_str.contains("3"));
}
