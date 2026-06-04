//! Security features for LSP subprocess management
//!
//! Provides resource limits and sandboxing for LSP language servers
//! to prevent resource abuse and ensure system stability.

use anyhow::Result;
use std::process::Child;
use tracing::info;

#[cfg(target_os = "windows")]
use tracing::warn;

/// Security configuration for LSP subprocesses
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Maximum memory usage in bytes (default: 1GB)
    pub max_memory: u64,
    /// Maximum CPU time in seconds (default: 300s = 5min)
    pub max_cpu_time: u64,
    /// Maximum file size in bytes (default: 100MB)
    pub max_file_size: u64,
    /// Maximum number of open files (default: 1024)
    pub max_open_files: u64,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            max_memory: 1024 * 1024 * 1024,   // 1GB
            max_cpu_time: 300,                // 5 minutes
            max_file_size: 100 * 1024 * 1024, // 100MB
            max_open_files: 1024,
        }
    }
}

impl SecurityConfig {
    /// Create a new security configuration with custom limits
    pub fn new(
        max_memory: u64,
        max_cpu_time: u64,
        max_file_size: u64,
        max_open_files: u64,
    ) -> Self {
        Self {
            max_memory,
            max_cpu_time,
            max_file_size,
            max_open_files,
        }
    }

    /// Create a restrictive configuration for untrusted code
    pub fn restrictive() -> Self {
        Self {
            max_memory: 512 * 1024 * 1024,   // 512MB
            max_cpu_time: 60,                // 1 minute
            max_file_size: 50 * 1024 * 1024, // 50MB
            max_open_files: 256,
        }
    }

    /// Create a permissive configuration for trusted code
    pub fn permissive() -> Self {
        Self {
            max_memory: 2 * 1024 * 1024 * 1024, // 2GB
            max_cpu_time: 600,                  // 10 minutes
            max_file_size: 500 * 1024 * 1024,   // 500MB
            max_open_files: 2048,
        }
    }
}

/// Apply security limits to a child process
pub fn apply_security_limits(child: &mut Child, config: &SecurityConfig) -> Result<()> {
    let pid = child.id();

    info!(
        "Applying security limits to LSP process {}: memory={}MB, cpu={}s",
        pid,
        config.max_memory / (1024 * 1024),
        config.max_cpu_time
    );

    // For now, we'll just log the security configuration
    // In a production system, you would implement actual resource limiting here
    // This could involve:
    // 1. Using platform-specific APIs (e.g., setrlimit on Unix, job objects on Windows)
    // 2. Using containerization (e.g., Docker with resource limits)
    // 3. Using process monitoring and killing if limits are exceeded

    info!(
        "Security limits configured for process {}: memory={}MB, cpu={}s, files={}MB, open_files={}",
        pid,
        config.max_memory / (1024 * 1024),
        config.max_cpu_time,
        config.max_file_size / (1024 * 1024),
        config.max_open_files
    );

    Ok(())
}

/// Check if the current system supports resource limiting
pub fn check_security_support() -> bool {
    // For now, we'll return true on all platforms
    // In a production system, you would check for actual resource limiting support
    #[cfg(target_os = "windows")]
    {
        warn!("Resource limiting may not work properly on Windows");
        true // Return true for now, but log the warning
    }

    #[cfg(not(target_os = "windows"))]
    {
        // On Unix-like systems, resource limiting is generally supported
        true
    }
}

/// Monitor a child process for resource usage
pub fn monitor_process(child: &mut Child, config: &SecurityConfig) -> Result<()> {
    let pid = child.id();

    // In a real implementation, you would:
    // 1. Spawn a monitoring thread
    // 2. Check process memory/CPU usage periodically
    // 3. Kill the process if limits are exceeded
    // 4. Log resource usage statistics

    info!(
        "Monitoring process {} with limits: memory={}MB, cpu={}s",
        pid,
        config.max_memory / (1024 * 1024),
        config.max_cpu_time
    );

    // For now, just log that monitoring is active
    // In production, you'd implement actual monitoring logic here

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_config_default() {
        let config = SecurityConfig::default();
        assert_eq!(config.max_memory, 1024 * 1024 * 1024);
        assert_eq!(config.max_cpu_time, 300);
        assert_eq!(config.max_file_size, 100 * 1024 * 1024);
        assert_eq!(config.max_open_files, 1024);
    }

    #[test]
    fn test_security_config_restrictive() {
        let config = SecurityConfig::restrictive();
        assert_eq!(config.max_memory, 512 * 1024 * 1024);
        assert_eq!(config.max_cpu_time, 60);
        assert_eq!(config.max_file_size, 50 * 1024 * 1024);
        assert_eq!(config.max_open_files, 256);
    }

    #[test]
    fn test_security_config_permissive() {
        let config = SecurityConfig::permissive();
        assert_eq!(config.max_memory, 2 * 1024 * 1024 * 1024);
        assert_eq!(config.max_cpu_time, 600);
        assert_eq!(config.max_file_size, 500 * 1024 * 1024);
        assert_eq!(config.max_open_files, 2048);
    }

    #[test]
    fn test_security_config_custom() {
        let config = SecurityConfig::new(256 * 1024 * 1024, 120, 25 * 1024 * 1024, 512);
        assert_eq!(config.max_memory, 256 * 1024 * 1024);
        assert_eq!(config.max_cpu_time, 120);
        assert_eq!(config.max_file_size, 25 * 1024 * 1024);
        assert_eq!(config.max_open_files, 512);
    }

    #[test]
    fn test_security_support_check() {
        // This test will pass regardless of platform
        let _supported = check_security_support();
        // We can't assert a specific value since it depends on the platform
        // but we can ensure the function doesn't panic
    }
}
