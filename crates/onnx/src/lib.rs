mod error;
pub use error::*;

use std::sync::OnceLock;

use ort::{
    Result,
    environment::Environment,
    session::{
        Session,
        builder::{AutoDevicePolicy, GraphOptimizationLevel, SessionBuilder},
    },
};

pub use ndarray;
pub use ort;

pub fn load_model_from_bytes(bytes: &[u8]) -> Result<Session, Error> {
    Ok(session_builder()?.commit_from_memory(bytes)?)
}

// Opt-in per call site rather than global: small per-frame models on the
// live path (e.g. Silero VAD) are faster on plain CPU than through CoreML
// dispatch, so only batch workloads should ask for acceleration.
pub fn load_model_from_bytes_accelerated(bytes: &[u8]) -> Result<Session, Error> {
    Ok(load_model_with_backend(bytes)?
        .session
        .expect("loaded session"))
}

/// The selected execution provider. Core ML can delegate unsupported nodes to CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    CoreMl,
}

/// A session that changes permanently to CPU after an accelerated error.
pub struct BackendSession<S> {
    session: Option<S>,
    backend: Backend,
    fallback_reason: Option<String>,
}

impl<S> BackendSession<S> {
    pub fn cpu(session: S) -> Self {
        Self {
            session: Some(session),
            backend: Backend::Cpu,
            fallback_reason: None,
        }
    }

    pub fn load_with<E: std::fmt::Display>(
        accelerated: impl FnOnce() -> Result<S, E>,
        cpu: impl FnOnce() -> Result<S, E>,
    ) -> Result<Self, E> {
        match accelerated() {
            Ok(session) => Ok(Self {
                session: Some(session),
                backend: Backend::CoreMl,
                fallback_reason: None,
            }),
            Err(error) => {
                let reason = error.to_string();
                tracing::warn!(error = %reason, "Core ML initialization failed; using CPU");
                Ok(Self {
                    session: Some(cpu()?),
                    backend: Backend::Cpu,
                    fallback_reason: Some(reason),
                })
            }
        }
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    /// `run` must validate its full output before it returns success.
    /// On failure, discard that output and recompute the same input on CPU.
    pub fn run_with_cpu_fallback<T, E: std::fmt::Display>(
        &mut self,
        mut run: impl FnMut(&mut S) -> Result<T, E>,
        mut load_cpu: impl FnMut() -> Result<S, E>,
    ) -> Result<T, E> {
        if self.session.is_none() {
            self.session = Some(load_cpu()?);
        }
        let result = run(self.session.as_mut().expect("loaded session"));
        match result {
            Err(error) if self.backend == Backend::CoreMl => {
                self.fallback_reason = Some(error.to_string());
                tracing::warn!(error = %error, "Core ML inference failed; recomputing input on CPU");
                self.backend = Backend::Cpu;
                // Drop the failed provider before loading CPU. A failed CPU load
                // must not leave the accelerated session available for reuse.
                self.session = None;
                self.session = Some(load_cpu()?);
                run(self.session.as_mut().expect("loaded CPU session"))
            }
            result => result,
        }
    }
}

pub fn load_model_with_backend(bytes: &[u8]) -> Result<BackendSession<Session>, Error> {
    #[cfg(all(feature = "coreml", target_os = "macos"))]
    {
        BackendSession::load_with(
            || match commit_accelerated(bytes) {
                Ok(session) => Ok(session),
                Err(error) if clear_accelerator_cache() => {
                    tracing::warn!(error = %error, "accelerated session cache rebuilt");
                    commit_accelerated(bytes)
                }
                Err(error) => Err(error),
            },
            || load_model_from_bytes(bytes),
        )
    }
    #[cfg(not(all(feature = "coreml", target_os = "macos")))]
    {
        Ok(BackendSession::cpu(load_model_from_bytes(bytes)?))
    }
}

// One environment for the process. Telemetry is off: the pyke binaries send
// none, but a Microsoft build of ONNX Runtime would. Only one thread builds
// it: ort keeps a link to the first environment built, and a second one that
// is dropped would leave that link empty.
fn environment() -> Result<&'static Environment, Error> {
    static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
    static BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if let Some(environment) = ENVIRONMENT.get() {
        return Ok(environment);
    }
    let _guard = BUILD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(environment) = ENVIRONMENT.get() {
        return Ok(environment);
    }
    let environment = ort::init()
        .with_name("longhand")
        .with_telemetry(false)
        .build()?;
    Ok(ENVIRONMENT.get_or_init(|| environment))
}

// ort selects AutoDevicePolicy::MaxEfficiency by default, which adds Core ML
// on macOS to each session. PreferCPU adds no provider, so a plain session and
// the CPU fallback stay on CPU. commit_accelerated adds Core ML explicitly.
fn session_builder() -> Result<SessionBuilder, Error> {
    Ok(Session::builder(environment()?)?
        .with_auto_device(AutoDevicePolicy::PreferCPU)?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .with_optimization_level(GraphOptimizationLevel::Level3)?)
}

// onnxruntime compiles the model into the shared cache with a non-atomic move,
// so two sessions built at the same time race and all but one fail to commit.
#[cfg(all(feature = "coreml", target_os = "macos"))]
static COREML_COMPILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Keyed by executable so an installed build and a dev build (or two app
// versions during an update) never compile into each other's cache; the
// in-process lock cannot protect against another process.
#[cfg(all(feature = "coreml", target_os = "macos"))]
fn accelerator_cache_dir() -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::hash::DefaultHasher::new();
    std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
        .hash(&mut hasher);
    std::env::temp_dir()
        .join("anlg-onnx-coreml-cache")
        .join(format!("{:016x}", hasher.finish()))
}

/// Returns whether there was a cache to remove.
#[cfg(all(feature = "coreml", target_os = "macos"))]
fn clear_accelerator_cache() -> bool {
    let _guard = COREML_COMPILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::fs::remove_dir_all(accelerator_cache_dir()).is_ok()
}

#[cfg(all(feature = "coreml", target_os = "macos"))]
fn commit_accelerated(bytes: &[u8]) -> Result<Session, Error> {
    use ort::ep::{
        CoreML,
        coreml::{ComputeUnits, ModelFormat},
    };

    // MLProgram cannot bound the unbounded input dims these models declare (the
    // speaker embedding extractor takes a variable frame count) and returns
    // non-finite output instead of refusing the graph; the GPU unit fails to
    // resize dynamically. NeuralNetwork on the ANE is the one combination that
    // both compiles and matches CPU output, and it runs ~10x faster than CPU.
    let provider = CoreML::default()
        .with_model_format(ModelFormat::NeuralNetwork)
        .with_compute_units(ComputeUnits::CPUAndNeuralEngine)
        .with_model_cache_dir(accelerator_cache_dir().to_string_lossy());
    let mut builder =
        session_builder()?.with_execution_providers([provider.build().error_on_failure()])?;

    let _guard = COREML_COMPILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(builder.commit_from_memory(bytes)?)
}

pub fn load_model_from_path(path: impl AsRef<std::path::Path>) -> Result<Session, Error> {
    let bytes = std::fs::read(path)?;
    load_model_from_bytes(&bytes)
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    #[test]
    fn unavailable_acceleration_loads_cpu_and_keeps_reason() {
        let session =
            BackendSession::load_with(|| Err("Core ML unavailable"), || Ok::<_, &'static str>(7))
                .unwrap();
        assert_eq!(session.backend(), Backend::Cpu);
        assert_eq!(session.fallback_reason(), Some("Core ML unavailable"));
    }

    #[test]
    fn inference_failure_recomputes_same_input_on_cpu() {
        let mut session =
            BackendSession::load_with(|| Ok::<_, &'static str>(false), || Ok(true)).unwrap();
        let input = [2, 3, 5];
        let output = session
            .run_with_cpu_fallback(
                |cpu| {
                    if *cpu {
                        Ok(input.iter().sum::<i32>())
                    } else {
                        Err("inference failed")
                    }
                },
                || Ok(true),
            )
            .unwrap();
        assert_eq!(output, 10);
        assert_eq!(session.backend(), Backend::Cpu);
        assert_eq!(session.fallback_reason(), Some("inference failed"));
        assert!(
            session
                .run_with_cpu_fallback(|cpu| Ok::<_, &str>(*cpu), || panic!("CPU must be reused"))
                .unwrap()
        );
    }

    #[test]
    fn nonfinite_result_is_discarded_before_cpu_result_is_returned() {
        let mut session = BackendSession::load_with(|| Ok::<_, &str>(false), || Ok(true)).unwrap();
        let output = session
            .run_with_cpu_fallback(
                |cpu| {
                    let output = if *cpu {
                        vec![3.0_f32, 4.0]
                    } else {
                        vec![1.0, f32::NAN]
                    };
                    if output.iter().all(|v| v.is_finite()) {
                        Ok(output)
                    } else {
                        Err("non-finite embedding")
                    }
                },
                || Ok(true),
            )
            .unwrap();
        assert_eq!(output, vec![3.0, 4.0]);
        assert_eq!(session.backend(), Backend::Cpu);
    }

    #[test]
    fn failed_cpu_retry_returns_error_and_never_retries_acceleration() {
        let mut session = BackendSession::load_with(|| Ok::<_, &str>(false), || Ok(true)).unwrap();
        let output = session.run_with_cpu_fallback(
            |cpu| {
                if *cpu {
                    Err::<(), _>("CPU failed")
                } else {
                    Err("Core ML failed")
                }
            },
            || Ok(true),
        );
        assert_eq!(output.unwrap_err(), "CPU failed");
        assert_eq!(session.backend(), Backend::Cpu);
    }
}
