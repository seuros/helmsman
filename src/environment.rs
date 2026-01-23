//! Environment detection for dynamic skills.
//!
//! # Reliability
//!
//! **Best effort only. Never authoritative.**
//!
//! Known edge cases:
//! - SSH + Docker may report wrong shell (`$SHELL` lies under nested contexts)
//! - Alpine/busybox may lack expected binaries despite capability
//! - Container detection uses heuristics (cgroup parsing, marker files)
//! - Binary detection via `which` may fail on non-standard PATH setups
//!
//! Use these values for optimization hints, not hard requirements.

use serde::Serialize;
use std::env;
use std::path::Path;
use std::process::Command;

/// Detected runtime environment.
///
/// **Best effort detection.** These values help templates adapt to the runtime
/// context but should not be treated as authoritative. Nested environments
/// (SSH into Docker, containers within VMs) may produce misleading results.
///
/// # Caveats
///
/// - `shell`: Reads `$SHELL` which may not reflect actual shell in use
/// - `in_docker`: Checks `/.dockerenv` and `/proc/1/cgroup` heuristics
/// - `has_*`: Uses `which` command, fails silently if PATH is non-standard
#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    // OS info
    pub os: String,
    pub os_family: String,
    pub arch: String,

    // Shell
    pub shell: String,

    // Container/remote detection
    pub in_docker: bool,
    pub in_ssh: bool,
    pub in_container: bool, // docker, podman, lxc, etc.

    // Available tools
    pub has_gh: bool,
    pub has_git: bool,
    pub has_curl: bool,
    pub has_wget: bool,
    pub has_jq: bool,
    pub has_mise: bool,
    pub has_brew: bool,
    pub has_apt: bool,
    pub has_dnf: bool,
    pub has_pkg: bool, // FreeBSD

    // Runtime managers
    pub has_nvm: bool,
    pub has_rbenv: bool,
    pub has_pyenv: bool,
}

impl Environment {
    /// Detect the current environment.
    pub fn detect() -> Self {
        let cgroup = read_cgroup();
        let in_docker = is_in_docker(cgroup.as_deref());
        let in_container = is_in_container(cgroup.as_deref(), in_docker);

        Self {
            os: detect_os(),
            os_family: detect_os_family(),
            arch: env::consts::ARCH.to_string(),

            shell: detect_shell(),

            in_docker,
            in_ssh: is_in_ssh(),
            in_container,

            has_gh: has_binary("gh"),
            has_git: has_binary("git"),
            has_curl: has_binary("curl"),
            has_wget: has_binary("wget"),
            has_jq: has_binary("jq"),
            has_mise: has_binary("mise"),
            has_brew: has_binary("brew"),
            has_apt: has_binary("apt"),
            has_dnf: has_binary("dnf"),
            has_pkg: has_binary("pkg"),

            has_nvm: env::var("NVM_DIR").is_ok() || has_binary("nvm"),
            has_rbenv: has_binary("rbenv"),
            has_pyenv: has_binary("pyenv"),
        }
    }
}

fn detect_os() -> String {
    // More specific than just env::consts::OS
    if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(target_os = "linux") {
        // Check for specific distro
        if Path::new("/etc/alpine-release").exists() {
            "alpine".to_string()
        } else if Path::new("/etc/debian_version").exists() {
            "debian".to_string()
        } else if Path::new("/etc/fedora-release").exists() {
            "fedora".to_string()
        } else if Path::new("/etc/arch-release").exists() {
            "arch".to_string()
        } else {
            "linux".to_string()
        }
    } else if cfg!(target_os = "freebsd") {
        "freebsd".to_string()
    } else if cfg!(target_os = "windows") {
        "windows".to_string()
    } else {
        env::consts::OS.to_string()
    }
}

fn detect_os_family() -> String {
    if cfg!(target_family = "unix") {
        "unix".to_string()
    } else if cfg!(target_family = "windows") {
        "windows".to_string()
    } else {
        "unknown".to_string()
    }
}

fn detect_shell() -> String {
    // Check SHELL env var
    if let Ok(shell) = env::var("SHELL") {
        if shell.contains("zsh") {
            return "zsh".to_string();
        } else if shell.contains("fish") {
            return "fish".to_string();
        } else if shell.contains("bash") {
            return "bash".to_string();
        } else if shell.contains("sh") {
            return "sh".to_string();
        }
    }
    "unknown".to_string()
}

fn is_in_docker(cgroup: Option<&str>) -> bool {
    // Check /.dockerenv file
    if Path::new("/.dockerenv").exists() {
        return true;
    }

    // Check cgroup for docker
    if let Some(cgroup) = cgroup
        && cgroup.contains("docker")
    {
        return true;
    }

    false
}

fn is_in_ssh() -> bool {
    // SSH_CLIENT or SSH_TTY indicates SSH session
    env::var("SSH_CLIENT").is_ok() || env::var("SSH_TTY").is_ok()
}

fn is_in_container(cgroup: Option<&str>, in_docker: bool) -> bool {
    // Docker
    if in_docker {
        return true;
    }

    // Check for container env var (podman, etc.)
    if env::var("container").is_ok() {
        return true;
    }

    // Check cgroup for various container runtimes
    if let Some(cgroup) = cgroup
        && (cgroup.contains("lxc") || cgroup.contains("kubepods") || cgroup.contains("podman"))
    {
        return true;
    }

    false
}

fn read_cgroup() -> Option<String> {
    std::fs::read_to_string("/proc/1/cgroup").ok()
}

fn has_binary(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_environment() {
        let env = Environment::detect();
        // Should at least detect OS
        assert!(!env.os.is_empty());
        assert!(!env.shell.is_empty() || env.shell == "unknown");
    }

    #[test]
    fn test_has_binary() {
        // 'ls' should exist on unix systems
        #[cfg(target_family = "unix")]
        assert!(has_binary("ls"));
    }
}
