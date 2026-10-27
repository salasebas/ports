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

#[cfg(target_os = "macos")]
fn start(pid: u32) -> Option<String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some(format!(
        "{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
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
}
