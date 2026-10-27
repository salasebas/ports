//! Reap child processes without tying a window action to their lifetime.

use std::process::Child;
#[cfg(any(not(target_os = "macos"), test))]
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

struct Reaper {
    children: Mutex<Vec<Child>>,
    wake: Condvar,
}

fn reaper() -> &'static Reaper {
    static REAPER: OnceLock<Reaper> = OnceLock::new();
    REAPER.get_or_init(|| {
        std::thread::Builder::new()
            .name("ports-reaper".into())
            .spawn(|| {
                let reaper = reaper();
                loop {
                    let children = reaper.children.lock().unwrap_or_else(|e| e.into_inner());
                    let mut children = reaper
                        .wake
                        .wait_timeout(children, Duration::from_millis(100))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                    children.retain_mut(|child| match child.try_wait() {
                        Ok(Some(_)) => false,
                        Ok(None) => true,
                        Err(error) if error.raw_os_error() == Some(libc::ECHILD) => false,
                        Err(error) => {
                            eprintln!("Could not reap child {}: {error}", child.id());
                            true
                        }
                    });
                }
            })
            .expect("could not start the child reaper");
        Reaper {
            children: Mutex::new(Vec::new()),
            wake: Condvar::new(),
        }
    })
}

pub fn reap(child: Child) {
    let reaper = reaper();
    reaper
        .children
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(child);
    reaper.wake.notify_one();
}

#[cfg(any(not(target_os = "macos"), test))]
pub fn launch(mut command: Command) -> Result<(), String> {
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(50));
    match child.try_wait() {
        Ok(Some(status)) => {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("The launcher exited with {status}"))
            };
        }
        Ok(None) => {}
        Err(error) => {
            reap(child);
            return Err(error.to_string());
        }
    }
    reap(child);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_launcher_that_exits_unsuccessfully() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        assert!(launch(command).unwrap_err().contains('7'));
    }

    #[test]
    fn reaps_an_exited_child() {
        let child = Command::new("sh")
            .args(["-c", "sleep 0.1"])
            .spawn()
            .unwrap();
        let pid = child.id();
        reap(child);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("exited launcher {pid} was not reaped");
    }
}
