//! The keys, the endpoints and the model list of a harness run (KTD12). A
//! key comes from its environment variable, else from a `.env` file in the
//! current folder or a parent folder. It never comes from the Keychain, and
//! no error text holds it.

use std::path::{Path, PathBuf};

use summarize::{Endpoint, Provider};

/// The name of the OpenRouter key variable.
pub const KEY_VARIABLE: &str = "OPENROUTER_API_KEY";
/// The base address of the user's LiteLLM server.
pub const LITELLM_BASE: &str = "LITELLM_BASE_URL";
/// The key of the user's LiteLLM server.
pub const LITELLM_KEY: &str = "LITELLM_API_KEY";
/// The start of a model entry that goes to the LiteLLM server, not to
/// OpenRouter.
pub const LITELLM_PREFIX: &str = "litellm:";

/// The value of the variable `name`: the environment value `env`, else the
/// value in the first `.env` file at `start` or above it that sets it.
pub fn find_value(name: &str, env: Option<String>, start: &Path) -> Result<String, String> {
    if let Some(value) = env.map(|value| value.trim().to_string())
        && !value.is_empty()
    {
        return Ok(value);
    }
    let mut folder = Some(start);
    while let Some(dir) = folder {
        let file = dir.join(".env");
        if let Ok(text) = std::fs::read_to_string(&file)
            && let Some(value) = value_in_env_file(&text, name)
        {
            return Ok(value);
        }
        folder = dir.parent();
    }
    Err(format!(
        "{name} is not set: set it, or put {name}=<value> in a .env file"
    ))
}

/// The endpoint of the model entry `entry`. An entry that starts with
/// [`LITELLM_PREFIX`] goes to the LiteLLM server as an OpenAI-compatible
/// endpoint at `<base>/v1`. Every other entry is an OpenRouter model id.
/// `lookup` gives the value of a variable.
pub fn endpoint_for(
    entry: &str,
    lookup: impl Fn(&str) -> Result<String, String>,
) -> Result<Endpoint, String> {
    if let Some(model) = entry.strip_prefix(LITELLM_PREFIX) {
        let base = lookup(LITELLM_BASE)?;
        let base = base.trim_end_matches('/');
        let base = if base.ends_with("/v1") {
            base.to_string()
        } else {
            format!("{base}/v1")
        };
        return Ok(Endpoint {
            provider: Provider::Custom,
            base_url: base,
            model: model.to_string(),
            credential: Some(lookup(LITELLM_KEY)?),
        });
    }
    Ok(Endpoint {
        provider: Provider::OpenRouter,
        base_url: String::new(),
        model: entry.to_string(),
        credential: Some(lookup(KEY_VARIABLE)?),
    })
}

/// The value of the variable `name` in the text of a `.env` file. A line
/// can start with `export`, and the value can have quotes.
pub fn value_in_env_file(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let value = line.strip_prefix(name)?.trim_start();
        let value = value.strip_prefix('=')?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The model ids in the text of a model file: one per line, with `#`
/// comments and blank lines skipped.
pub fn models_in(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// The arguments of `check-hosts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckHosts {
    pub models: Vec<String>,
    /// The number of tries for the first tool call.
    pub tries: u32,
    /// Send only to hosts with zero data retention.
    pub zero_retention: bool,
}

/// Parse the arguments after `check-hosts`. `read` reads a model file.
pub fn parse_check_hosts(
    args: &[String],
    read: impl Fn(&Path) -> Result<String, String>,
) -> Result<CheckHosts, String> {
    let mut models = Vec::new();
    let mut tries = 3;
    let mut zero_retention = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => models.push(args.next().ok_or("--model needs a model id")?.clone()),
            "--models" => {
                let path = PathBuf::from(args.next().ok_or("--models needs a file")?);
                models.extend(models_in(&read(&path)?));
            }
            "--tries" => {
                tries = args
                    .next()
                    .ok_or("--tries needs a number")?
                    .parse()
                    .map_err(|_| "--tries needs a number from 1".to_string())?;
                if tries == 0 {
                    return Err("--tries needs a number from 1".into());
                }
            }
            "--zdr" => zero_retention = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if models.is_empty() {
        return Err("give at least one --model or a --models file".into());
    }
    Ok(CheckHosts {
        models,
        tries,
        zero_retention,
    })
}
