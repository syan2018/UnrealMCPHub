use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::task::JoinHandle;
use tokio::time::{Duration, MissedTickBehavior};

use crate::orchestrator;
use crate::process::is_process_alive;
use crate::state::StateStore;
use crate::ue_client::UeClient;

const DEFAULT_INTERVAL_SECS: u64 = 5;
const DISCOVER_INTERVAL_CYCLES: u64 = 6;
const PURGE_INTERVAL_CYCLES: u64 = 60;
const STALE_HOURS: f64 = 1.0;

pub struct ProcessWatcher {
    stop: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl ProcessWatcher {
    pub fn spawn() -> Self {
        Self::spawn_with_interval(DEFAULT_INTERVAL_SECS)
    }

    pub fn spawn_with_interval(interval_secs: u64) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let task_stop = Arc::clone(&stop);
        let task = tokio::spawn(async move {
            let mut cycles = 0_u64;
            let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs.max(1)));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;
                if task_stop.load(Ordering::Relaxed) {
                    break;
                }

                if let Err(error) = refresh_instances().await {
                    eprintln!("unreal watcher refresh failed: {error}");
                }

                cycles = cycles.saturating_add(1);
                if cycles % DISCOVER_INTERVAL_CYCLES == 0 {
                    if let Err(error) = orchestrator::discover_instances().await {
                        eprintln!("unreal watcher discover failed: {error}");
                    }
                }
                if cycles % PURGE_INTERVAL_CYCLES == 0 {
                    if let Err(error) = purge_stale_instances() {
                        eprintln!("unreal watcher cleanup failed: {error}");
                    }
                }
            }
        });

        Self { stop, task }
    }

    pub async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.task.abort();
        let _ = self.task.await;
    }
}

async fn refresh_instances() -> anyhow::Result<()> {
    let snapshot = StateStore::load()?;
    let known_instances = snapshot
        .list_instances()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();

    for instance in known_instances {
        if instance.url.is_empty() || instance.port == 0 {
            if let Some(pid) = instance.pid {
                if !is_process_alive(pid) {
                    let mut state = StateStore::load()?;
                    state.update_instance_status(&instance.key, "offline", None)?;
                }
            }
            continue;
        }

        match UeClient::health_check(&instance.url).await {
            Ok(_) => {
                let mut state = StateStore::load()?;
                state.update_instance_status(&instance.key, "online", instance.pid)?;
            }
            Err(_) => {
                let mut state = StateStore::load()?;
                if let Some(pid) = instance.pid {
                    if !is_process_alive(pid)
                        && matches!(instance.status.as_str(), "online" | "starting")
                    {
                        state.mark_crashed(&instance.key)?;
                    } else {
                        state.update_instance_status(&instance.key, "offline", Some(pid))?;
                    }
                } else {
                    state.update_instance_status(&instance.key, "offline", None)?;
                }
            }
        }
    }

    Ok(())
}

fn purge_stale_instances() -> anyhow::Result<()> {
    let mut state = StateStore::load()?;
    let _ = state.cleanup(STALE_HOURS)?;
    Ok(())
}
