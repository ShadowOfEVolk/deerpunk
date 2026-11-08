//! Optional async `queue.submit` + `present` while the main thread runs the next frame sim/extract.

use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, Receiver, Sender};
use deerpunk_engine_renderer::renderer::PendingSurfaceTail;
use wgpu::{CommandBuffer, Queue, SurfaceTexture};

pub struct SubmitThread {
    job_tx: Sender<SubmitJob>,
    done_rx: Receiver<SubmitDone>,
    join: Option<JoinHandle<()>>,
}

struct SubmitJob {
    cmds: Vec<CommandBuffer>,
    surface: SurfaceTexture,
}

pub struct SubmitDone {
    pub submit_ms: f32,
    pub present_ms: f32,
}

impl SubmitThread {
    pub fn spawn(queue: Queue) -> Self {
        let (job_tx, job_rx) = bounded::<SubmitJob>(1);
        let (done_tx, done_rx) = bounded::<SubmitDone>(1);
        let queue_worker = queue.clone();

        let join = thread::Builder::new()
            .name("deerpunk-submit".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let submit_start = std::time::Instant::now();
                    queue_worker.submit(job.cmds);
                    let submit_ms = submit_start.elapsed().as_secs_f32() * 1000.0;
                    let present_start = std::time::Instant::now();
                    queue_worker.present(job.surface);
                    let present_ms = present_start.elapsed().as_secs_f32() * 1000.0;
                    let _ = done_tx.send(SubmitDone {
                        submit_ms,
                        present_ms,
                    });
                }
            })
            .expect("spawn deerpunk-submit thread");

        Self {
            job_tx,
            done_rx,
            join: Some(join),
        }
    }

    pub fn wait(&self) -> SubmitDone {
        self.done_rx.recv().unwrap_or(SubmitDone {
            submit_ms: 0.0,
            present_ms: 0.0,
        })
    }

    fn submit(&self, cmds: Vec<CommandBuffer>, surface: SurfaceTexture) {
        self.job_tx
            .send(SubmitJob { cmds, surface })
            .expect("submit job channel (prior present still in flight?)");
    }

    pub fn shutdown(mut self) {
        drop(self.job_tx);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub struct PipelinedSubmit {
    thread: SubmitThread,
    in_flight: bool,
    completion_meta: Option<CompletionMeta>,
}

struct CompletionMeta {
    lum_copied: bool,
    occlusion_queries: bool,
    tail: PendingSurfaceTail,
}

impl PipelinedSubmit {
    pub fn new(queue: Queue) -> Self {
        Self {
            thread: SubmitThread::spawn(queue),
            in_flight: false,
            completion_meta: None,
        }
    }

    pub fn wait_and_complete(
        &mut self,
        renderer: &mut deerpunk_engine_renderer::renderer::Renderer,
    ) {
        if !self.in_flight {
            return;
        }
        let done = self.thread.wait();
        if let Some(meta) = self.completion_meta.take() {
            renderer.finish_deferred_surface_submit(
                meta.lum_copied,
                meta.occlusion_queries,
                meta.tail,
                done.submit_ms,
                done.present_ms,
            );
        }
        self.in_flight = false;
    }

    pub fn kick(&mut self, renderer: &mut deerpunk_engine_renderer::renderer::Renderer) {
        let Some(pending) = renderer.take_pending_surface_present() else {
            return;
        };
        self.completion_meta = Some(CompletionMeta {
            lum_copied: pending.lum_copied,
            occlusion_queries: pending.occlusion_queries,
            tail: pending.tail,
        });
        self.thread.submit(pending.cmds, pending.surface);
        self.in_flight = true;
    }

    pub fn shutdown(self) {
        self.thread.shutdown();
    }
}
