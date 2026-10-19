//! Console-only telemetry; never participates in campaign identity or selection.
use super::{CampaignState, GenerationRecord, Progress};
use std::{
    fmt::Write,
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct Snapshot {
    progress: Progress,
    stage_started: f64,
    last_completion: Option<f64>,
    completed_generations: u64,
    target: u64,
    champions: Vec<(String, f64, u64)>,
    challengers: Vec<(String, usize)>,
}

impl Snapshot {
    fn update(&mut self, progress: &Progress, now: f64) {
        if self.progress.label != progress.label
            || self.progress.track != progress.track
            || self.progress.plan != progress.plan
            || self.progress.stage_index != progress.stage_index
        {
            self.stage_started = now;
            self.last_completion = None;
        } else if progress.jobs_completed > self.progress.jobs_completed {
            self.last_completion = Some(now);
        }
        self.progress = progress.clone();
    }

    fn render(&self, now: f64) -> String {
        let p = &self.progress;
        let stage_elapsed = (now - self.stage_started).max(0.0);
        let mut text = format!(
            "campaign progress [{now:.0}s] {} | completed {}/{} generations | track {} | {} | {}: {}\n  {}/{} evaluations complete ({} cached, {} waiting) | stage {stage_elapsed:.0}s",
            p.status,
            self.completed_generations,
            self.target,
            p.track.as_deref().unwrap_or("pending"),
            p.label.as_deref().unwrap_or("setup"),
            p.plan.as_deref().unwrap_or("preparing"),
            p.stage.as_deref().unwrap_or("inputs"),
            p.jobs_completed,
            p.jobs_total,
            p.jobs_cached,
            p.jobs_waiting,
        );
        match self.last_completion {
            Some(last) => write!(text, " | last completion {:.0}s ago", now - last).unwrap(),
            None => text.push_str(" | awaiting first completion"),
        }
        // Cached work is near-instantaneous and must not inflate simulation speed.
        let measured = p.jobs_completed.saturating_sub(p.jobs_cached);
        let remaining = p.jobs_total.saturating_sub(p.jobs_completed);
        if measured >= 2 && stage_elapsed > 0.0 {
            let rate = measured as f64 / stage_elapsed;
            write!(text, " | {:.1} eval/min", rate * 60.0).unwrap();
            if p.jobs_waiting == 0 {
                write!(
                    text,
                    " | ~{:.0}s remaining in stage",
                    remaining as f64 / rate
                )
                .unwrap();
            }
        } else {
            text.push_str(" | stage ETA pending measurements");
        }
        for (track, fitness, generation) in &self.champions {
            write!(
                text,
                "\n  {track} champion validation {fitness:.6}, retained from gen {generation}"
            )
            .unwrap();
        }
        for (track, count) in &self.challengers {
            write!(
                text,
                "\n  {track}: {count} diagnostic alternatives retained"
            )
            .unwrap();
        }
        text
    }
}

pub(super) struct Reporter {
    snapshot: Arc<Mutex<Snapshot>>,
    started: Instant,
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Reporter {
    pub(super) fn start(
        started: Instant,
        interval: Duration,
        emit: impl Fn(&str) + Send + 'static,
    ) -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot {
            progress: Progress {
                status: "preparing".into(),
                ..Progress::default()
            },
            stage_started: 0.0,
            last_completion: None,
            completed_generations: 0,
            target: 0,
            champions: Vec::new(),
            challengers: Vec::new(),
        }));
        emit("campaign progress: preparing inputs (heartbeat every 30s)");
        let shared = Arc::clone(&snapshot);
        let (stop, receiver) = mpsc::channel();
        let thread = thread::spawn(move || {
            while let Err(mpsc::RecvTimeoutError::Timeout) = receiver.recv_timeout(interval) {
                let message = shared
                    .lock()
                    .unwrap()
                    .render(started.elapsed().as_secs_f64());
                emit(&message);
            }
        });
        Self {
            snapshot,
            started,
            stop,
            thread: Some(thread),
        }
    }

    pub(super) fn update(&self, progress: &Progress) {
        self.snapshot
            .lock()
            .unwrap()
            .update(progress, self.started.elapsed().as_secs_f64());
    }

    pub(super) fn checkpoint(&self, state: &CampaignState, target: u64) {
        let mut snapshot = self.snapshot.lock().unwrap();
        snapshot.completed_generations = state.completed_generations;
        snapshot.target = target;
        snapshot.challengers = state
            .tracks
            .iter()
            .filter_map(|t| {
                t.frontier
                    .as_ref()
                    .map(|f| (t.name.clone(), f.entries.len()))
            })
            .collect();
        snapshot.champions = state
            .tracks
            .iter()
            .filter_map(|track| {
                track
                    .best_report
                    .as_ref()
                    .map(|report| (track.name.clone(), report.fitness, track.best_generation))
            })
            .collect();
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn generation_report(record: &GenerationRecord, target: u64, seconds: f64) -> String {
    let mut text = format!(
        "campaign generation {}/{target} committed in {seconds:.1}s",
        record.generation
    );
    for track in &record.tracks {
        write!(
            text,
            "\n  {} champion validation {:.6}, retained from gen {}",
            track.name, track.champion_fitness, track.best_generation
        )
        .unwrap();
        if let Some(count) = track.diagnostic_challengers {
            write!(text, "\n    {count} diagnostic alternatives retained").unwrap();
        }
        for island in &track.islands {
            write!(
                text,
                "\n    {}: candidates mean {:.6}, best {:.6}, worst {:.6} | update norm {:.6}",
                island.name,
                island.optimizer.fitness_mean,
                island.optimizer.fitness_best,
                island.optimizer.fitness_worst,
                island.optimizer.update_norm
            )
            .unwrap();
            match (&island.center_validation, &island.sample_validation) {
                (Some(center), Some(sample)) => write!(
                    text,
                    "\n      validation center {:.6}, sample {:.6}",
                    center.fitness, sample.fitness
                )
                .unwrap(),
                _ => text.push_str("\n      validation not scheduled"),
            }
            write!(
                text,
                " | promotions: {} | restart: {}",
                if island.promoted.is_empty() {
                    "none".into()
                } else {
                    island.promoted.join(", ")
                },
                island.restarted_module.as_deref().unwrap_or("none")
            )
            .unwrap();
            for challenge in &island.promotion_challenges {
                let report = &challenge.report;
                write!(
                    text,
                    "\n      {} fresh promotion: {} | mean gain {:.6} | {}/{} pairs won",
                    challenge.candidate,
                    if report.accepted {
                        "accepted"
                    } else {
                        "rejected"
                    },
                    report.mean_gain,
                    report.wins,
                    report.seeds
                )
                .unwrap();
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_estimates_only_live_work_and_resets_for_new_stage() {
        let mut snapshot = Snapshot {
            progress: Progress::default(),
            stage_started: 0.0,
            last_completion: None,
            completed_generations: 4,
            target: 200,
            challengers: Vec::new(),
            champions: vec![("strategic".into(), 0.4793, 1)],
        };
        let mut progress = Progress {
            status: "running".into(),
            label: Some("generation-5/strategic/local/train/stage-2".into()),
            plan: Some("training".into()),
            stage: Some("objectives".into()),
            jobs_total: 32,
            ..Progress::default()
        };
        snapshot.update(&progress, 10.0);
        assert!(snapshot.render(20.0).contains("ETA pending measurements"));
        progress.jobs_completed = 18;
        progress.jobs_cached = 4;
        snapshot.update(&progress, 132.0);
        let message = snapshot.render(140.0);
        assert!(
            message.contains("18/32 evaluations complete (4 cached"),
            "{message}"
        );
        assert!(message.contains("last completion 8s ago"), "{message}");
        assert!(
            message.contains("6.5 eval/min | ~130s remaining"),
            "{message}"
        );
        assert!(message.contains("strategic champion validation 0.479300, retained from gen 1"));
        progress.label = Some("next-stage".into());
        progress.jobs_completed = 0;
        progress.jobs_cached = 0;
        snapshot.update(&progress, 150.0);
        let message = snapshot.render(155.0);
        assert!(
            message.contains("stage 5s | awaiting first completion"),
            "{message}"
        );
        assert!(message.contains("ETA pending measurements"));
    }

    #[test]
    fn initial_validation_resets_timing_when_single_stage_tracks_share_labels() {
        let mut snapshot = Snapshot {
            progress: Progress::default(),
            stage_started: 0.0,
            last_completion: None,
            completed_generations: 0,
            target: 200,
            challengers: Vec::new(),
            champions: vec![],
        };
        let mut progress = Progress {
            status: "running".into(),
            label: Some("initial/stage-0".into()),
            track: Some("tactical".into()),
            plan: Some("validation".into()),
            stage: Some("easy".into()),
            jobs_total: 1,
            ..Progress::default()
        };
        snapshot.update(&progress, 10.0);
        progress.jobs_completed = 1;
        snapshot.update(&progress, 30.0);
        progress.track = Some("strategic".into());
        progress.jobs_completed = 0;
        snapshot.update(&progress, 40.0);
        let message = snapshot.render(45.0);
        assert!(message.contains("strategic"), "{message}");
        assert!(
            message.contains("stage 5s | awaiting first completion"),
            "{message}"
        );
    }

    #[test]
    fn timer_reports_without_work_updates_and_stops_promptly() {
        let (sender, receiver) = mpsc::channel();
        let reporter = Reporter::start(Instant::now(), Duration::from_millis(20), move |message| {
            sender.send(message.to_owned()).unwrap();
        });
        receiver.recv_timeout(Duration::from_secs(2)).unwrap(); // startup
        for _ in 0..2 {
            let message = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(message.contains("preparing"), "{message}");
        }
        let before = Instant::now();
        drop(reporter);
        assert!(before.elapsed() < Duration::from_secs(1));
        while receiver.try_recv().is_ok() {}
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }
}
