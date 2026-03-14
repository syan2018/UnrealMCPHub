use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(windows)]
    {
        let filter = format!("PID eq {pid}");
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
        else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        !stdout.contains("No tasks are running")
            && !stdout.contains("INFO:")
            && !stdout.trim().is_empty()
    }

    #[cfg(not(windows))]
    {
        let Ok(output) = Command::new("ps").args(["-p", &pid.to_string()]).output() else {
            return false;
        };
        output.status.success() && String::from_utf8_lossy(&output.stdout).lines().count() > 1
    }
}

pub fn terminate_process(pid: u32, force: bool) -> Result<()> {
    if pid == 0 {
        bail!("pid must be greater than 0");
    }

    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/T"]);
        if force {
            command.arg("/F");
        }
        let output = command
            .output()
            .with_context(|| format!("failed to invoke taskkill for PID {pid}"))?;
        if !output.status.success() {
            bail!(
                "taskkill failed for PID {pid}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let signal = if force { "-KILL" } else { "-TERM" };
        let output = Command::new("kill")
            .args([signal, &pid.to_string()])
            .output()
            .with_context(|| format!("failed to invoke kill for PID {pid}"))?;
        if !output.status.success() {
            bail!(
                "kill failed for PID {pid}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}
