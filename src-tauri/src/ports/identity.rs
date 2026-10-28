//! Kernel process identity, retained from discovery through every control action.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start: String,
}

impl ProcessIdentity {
    pub fn read(pid: u32) -> Option<Self> {
        Some(Self {
            pid,
            start: start(pid)?,
        })
    }

    pub fn key(&self) -> String {
        format!("{}:{}", self.pid, self.start)
    }

    pub fn is_current(&self) -> bool {
        Self::read(self.pid).as_ref() == Some(self)
    }

    /// When the process started, in seconds since the epoch, where the
    /// identity records wall-clock time (macOS). Linux records ticks since boot.
    pub fn started_secs(&self) -> Option<u64> {
        if cfg!(target_os = "macos") {
            self.start.split_once(':')?.0.parse().ok()
        } else {
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    linux_start(&stat).map(str::to_string)
}

#[cfg(any(target_os = "linux", test))]
fn linux_start(stat: &str) -> Option<&str> {
    // comm may itself contain spaces and parentheses; starttime is field 22.
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)
}

/// `sysctl(KERN_PROC_PID)` reads any user's process, unlike `proc_pidinfo`,
/// so root listeners keep their identity too.
#[cfg(target_os = "macos")]
fn start(pid: u32) -> Option<String> {
    // `struct kinfo_proc` is 648 bytes and begins with `p_starttime`, a timeval.
    let mut info = [0u64; 81];
    let mut size = std::mem::size_of_val(&info);
    let mut mib = [
        libc::CTL_KERN,
        libc::KERN_PROC,
        libc::KERN_PROC_PID,
        pid as libc::c_int,
    ];
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            info.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size != std::mem::size_of_val(&info) {
        return None;
    }
    let start: libc::timeval = unsafe { std::ptr::read(info.as_ptr().cast()) };
    Some(format!("{}:{}", start.tv_sec, start.tv_usec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_kernel_precision_and_parses_parentheses_in_process_names() {
        let stat = "42 (a tricky ) name) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 123456 0";
        assert_eq!(linux_start(stat), Some("123456"));
        let current = ProcessIdentity::read(std::process::id()).unwrap();
        assert!(current.is_current());
        let stale = ProcessIdentity {
            start: format!("{}-old", current.start),
            ..current
        };
        assert!(!stale.is_current());
    }

    // launchd belongs to root; sysinfo can't read its start time, the kernel can.
    #[cfg(target_os = "macos")]
    #[test]
    fn knows_when_another_users_process_started() {
        let started = ProcessIdentity::read(1).unwrap().started_secs().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(started > 0 && started <= now);
    }

    // Another user's process, such as launchd or init, still has an identity.
    #[test]
    fn reads_the_identity_of_another_users_process() {
        assert!(ProcessIdentity::read(1).is_some());
    }
}
