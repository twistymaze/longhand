//! Counted processing leases. The platform assertion stays on one thread.

use std::sync::mpsc::{self, Sender};

#[derive(Clone)]
pub struct ProcessingPower {
    commands: Option<Sender<Command>>,
}

pub struct ProcessingLease {
    commands: Option<Sender<Command>>,
}

enum Command {
    Acquire(Sender<()>),
    Release(Sender<()>),
}

impl Default for ProcessingPower {
    fn default() -> Self {
        Self::with_factory(|| crate::IdleGuard::acquire_for("Longhand is processing"))
            .unwrap_or_else(|error| {
                log::warn!("could not start the processing power owner: {error}");
                Self { commands: None }
            })
    }
}

impl ProcessingPower {
    /// Start an owner with a platform assertion factory. The assertion can
    /// be non-Send: the factory creates it and the owner drops it in place.
    pub fn with_factory<G: 'static>(
        mut acquire: impl FnMut() -> Option<G> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("processing-power".into())
            .spawn(move || {
                let mut owners = 0usize;
                let mut guard = None;
                while let Ok(command) = receiver.recv() {
                    match command {
                        Command::Acquire(done) => {
                            if owners == 0 {
                                guard = acquire();
                            }
                            owners += 1;
                            let _ = done.send(());
                        }
                        Command::Release(done) => {
                            owners -= 1;
                            if owners == 0 {
                                drop(guard.take());
                            }
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        Ok(Self {
            commands: Some(commands),
        })
    }

    /// Hold idle-sleep protection until the lease is dropped. A platform
    /// refusal is logged by the factory and does not stop processing.
    pub fn acquire(&self) -> ProcessingLease {
        let commands = self
            .commands
            .as_ref()
            .filter(|commands| {
                let (done, completed) = mpsc::channel();
                commands.send(Command::Acquire(done)).is_ok() && completed.recv().is_ok()
            })
            .cloned();
        ProcessingLease { commands }
    }
}

impl Drop for ProcessingLease {
    fn drop(&mut self) {
        if let Some(commands) = &self.commands {
            let (done, completed) = mpsc::channel();
            if commands.send(Command::Release(done)).is_ok() {
                let _ = completed.recv();
            }
        }
    }
}
