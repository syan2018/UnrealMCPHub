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

pub fn find_process_pid_by_command_line(
    process_name: &str,
    command_line_fragment: &str,
) -> Option<u32> {
    let process_name = process_name.trim();
    let command_line_fragment = command_line_fragment.trim();
    if process_name.is_empty() || command_line_fragment.is_empty() {
        return None;
    }

    #[cfg(windows)]
    {
        let escaped_name = process_name.replace('\'', "''");
        let escaped_fragment = command_line_fragment.replace('\'', "''");
        let script = format!(
            "$p = Get-CimInstance Win32_Process -Filter \"Name = '{escaped_name}'\" | Where-Object {{ $_.CommandLine -like '*{escaped_fragment}*' }} | Select-Object -First 1 -ExpandProperty ProcessId; if ($p) {{ Write-Output $p }}"
        );
        let Ok(output) = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
        else {
            return None;
        };
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .lines()
            .next()
            .and_then(|value| value.trim().parse::<u32>().ok())
    }

    #[cfg(not(windows))]
    {
        let Ok(output) = Command::new("ps")
            .args(["-eo", "pid=,comm=,args="])
            .output()
        else {
            return None;
        };
        if !output.status.success() {
            return None;
        }

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| {
                if !line.contains(process_name) || !line.contains(command_line_fragment) {
                    return None;
                }
                line.split_whitespace().next()?.parse::<u32>().ok()
            })
    }
}

pub fn terminate_process(pid: u32, force: bool) -> Result<bool> {
    if pid == 0 {
        bail!("pid must be greater than 0");
    }

    #[cfg(windows)]
    {
        if force {
            invoke_taskkill(pid, true)?;
            return Ok(true);
        }

        match invoke_taskkill(pid, false) {
            Ok(()) => Ok(false),
            Err(initial_error) => match invoke_taskkill(pid, true) {
                Ok(()) => Ok(true),
                Err(force_error) => bail!(
                    "taskkill failed for PID {pid}: graceful={initial_error}; forced={force_error}"
                ),
            },
        }
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
        Ok(force)
    }
}

#[cfg(windows)]
fn invoke_taskkill(pid: u32, force: bool) -> Result<()> {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if force {
        command.arg("/F");
    }

    let output = command
        .output()
        .with_context(|| format!("failed to invoke taskkill for PID {pid}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let details = [stderr, stdout]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    bail!(
        "{}",
        if details.is_empty() {
            format!("taskkill returned exit code {}", output.status)
        } else {
            details
        }
    )
}
