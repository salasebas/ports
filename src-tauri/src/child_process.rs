//! Reap child processes without tying a window action to their lifetime.

use std::process::Child;
#[cfg(any(not(target_os = "macos"), test))]
use std::process::Command;
#[cfg(any(not(target_os = "macos"), test))]
use std::time::Duration;

/// Waits for the child on its own thread, so it never lingers as a zombie.
pub fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
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
