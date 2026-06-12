//! File content analyzer for extracting function calls
//!
//! This module provides a service for analyzing file content to extract
//! actual function call relationships from the source code.

use crate::domain::{Node, Range};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

/// Represents a function call found in the code
#[derive(Debug, Clone)]
pub struct FunctionCall {
    /// The function being called
    pub callee_name: String,
    /// Location of the call
    pub location: Range,
    /// The function that contains this call (if we can determine it)
    pub caller_function: Option<String>,
}

/// Service for analyzing file content to extract function calls
pub struct FileAnalyzer {
    /// Cache of parsed file contents
    file_cache: HashMap<String, String>,
}

impl FileAnalyzer {
    /// Create a new file analyzer
    pub fn new() -> Self {
        Self {
            file_cache: HashMap::new(),
        }
    }

    /// Analyze a file to extract function calls
    pub fn analyze_file(
        &mut self,
        file_path: &Path,
        functions: &[Node],
    ) -> Result<Vec<FunctionCall>> {
        let content = self.load_file_content(file_path)?;
        let mut calls = Vec::new();

        // For each function in the file, analyze its body for function calls
        for function in functions {
            if let Some(function_calls) = self.extract_calls_from_function(&content, function)? {
                calls.extend(function_calls);
            }
        }

        info!(
            "Extracted {} function calls from {}",
            calls.len(),
            file_path.display()
        );
        Ok(calls)
    }

    /// Load file content (with caching)
    fn load_file_content(&mut self, file_path: &Path) -> Result<String> {
        let path_str = file_path.to_string_lossy().to_string();

        if let Some(content) = self.file_cache.get(&path_str) {
            return Ok(content.clone());
        }

        let content = std::fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

        self.file_cache.insert(path_str, content.clone());
        Ok(content)
    }

    /// Extract function calls from a specific function's body
    fn extract_calls_from_function(
        &self,
        content: &str,
        function: &Node,
    ) -> Result<Option<Vec<FunctionCall>>> {
        let function_range = FunctionRange {
            start_line: function.location.start_line as usize,
            end_line: function.location.end_line as usize,
        };

        // Extract the function body
        let function_body = self.extract_function_body(content, &function_range)?;

        // Find function calls in the body
        let calls = self.find_function_calls_in_text(&function_body, &function_range)?;

        Ok(Some(calls))
    }

    /// Extract function body from content
    fn extract_function_body(&self, content: &str, range: &FunctionRange) -> Result<String> {
        let lines: Vec<&str> = content.lines().collect();

        // Protect against 0-index underflows and starting out of bounds
        if range.start_line == 0 || range.start_line > lines.len() {
            return Ok(String::new());
        }

        // Gracefully cap the end line to the end of the file
        let safe_end = std::cmp::min(range.end_line, lines.len());
        let body_lines = &lines[range.start_line - 1..safe_end];
        Ok(body_lines.join("\n"))
    }

    /// Find function calls in text using simple pattern matching
    fn find_function_calls_in_text(
        &self,
        text: &str,
        base_range: &FunctionRange,
    ) -> Result<Vec<FunctionCall>> {
        let mut calls = Vec::new();

        // Simple regex-like pattern matching for function calls
        // This is a basic implementation - in production, we'd use proper AST parsing
        let lines: Vec<&str> = text.lines().collect();

        for (line_idx, line) in lines.iter().enumerate() {
            // Look for patterns like: function_name(
            if let Some(call) = self.extract_call_from_line(line, line_idx + base_range.start_line)
            {
                calls.push(call);
            }
        }

        Ok(calls)
    }

    /// Extract a function call from a single line
    fn extract_call_from_line(&self, line: &str, line_number: usize) -> Option<FunctionCall> {
        // Look for patterns like: function_name(
        // This is a very basic implementation
        if let Some(start) = line.find('(') {
            let before_paren = &line[..start];
            let parts: Vec<&str> = before_paren.split_whitespace().collect();

            if let Some(last_part) = parts.last() {
                // Check if it looks like a function call
                if last_part.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    return Some(FunctionCall {
                        callee_name: last_part.to_string(),
                        location: Range {
                            start_line: line_number as u32,
                            start_char: 0,
                            end_line: line_number as u32,
                            end_char: line.len() as u32,
                            file: "unknown".to_string(),
                        },
                        caller_function: None, // Will be set by caller
                    });
                }
            }
        }

        None
    }
}

/// Represents a function's range in the file
#[derive(Debug, Clone)]
struct FunctionRange {
    start_line: usize,
    end_line: usize,
}

impl Default for FileAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}
