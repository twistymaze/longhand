//! The key lookup and the `check-hosts` arguments (U2).

use std::path::Path;

use summarize::Provider;
use summary_lab::config::{
    KEY_VARIABLE, endpoint_for, find_value, parse_check_hosts, value_in_env_file,
};

fn args(text: &[&str]) -> Vec<String> {
    text.iter().map(|s| s.to_string()).collect()
}

fn no_file(_: &Path) -> Result<String, String> {
    Err("no file".into())
}

#[test]
fn the_environment_key_comes_first() {
    let dir = std::env::temp_dir().join(format!("summary-lab-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".env"), format!("{KEY_VARIABLE}=from-file\n")).unwrap();
    assert_eq!(
        find_value(KEY_VARIABLE, Some("from-env".into()), &dir).unwrap(),
        "from-env"
    );
    assert_eq!(find_value(KEY_VARIABLE, None, &dir).unwrap(), "from-file");
    assert_eq!(
        find_value(KEY_VARIABLE, Some("  ".into()), &dir).unwrap(),
        "from-file"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_env_file_in_a_parent_folder_is_found() {
    let dir = std::env::temp_dir().join(format!("summary-lab-parent-{}", std::process::id()));
    let child = dir.join("a").join("b");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(dir.join(".env"), format!("{KEY_VARIABLE}=up-here\n")).unwrap();
    assert_eq!(find_value(KEY_VARIABLE, None, &child).unwrap(), "up-here");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn no_key_is_a_clear_error() {
    let dir = std::env::temp_dir().join(format!("summary-lab-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".env"), "OTHER=x\n").unwrap();
    let error = find_value(KEY_VARIABLE, None, &dir).unwrap_err();
    assert!(error.contains(KEY_VARIABLE), "{error}");
    assert!(!error.contains("OTHER=x"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_env_file_line_can_have_export_and_quotes() {
    assert_eq!(
        value_in_env_file(
            &format!("# c\nexport {KEY_VARIABLE}=\"sk-1\"\n"),
            KEY_VARIABLE
        )
        .as_deref(),
        Some("sk-1")
    );
    assert_eq!(
        value_in_env_file(&format!("{KEY_VARIABLE} = 'sk-2'"), KEY_VARIABLE).as_deref(),
        Some("sk-2")
    );
    assert_eq!(
        value_in_env_file(&format!("{KEY_VARIABLE}_OLD=sk-3"), KEY_VARIABLE),
        None
    );
    assert_eq!(
        value_in_env_file(&format!("{KEY_VARIABLE}="), KEY_VARIABLE),
        None
    );
}

#[test]
fn check_hosts_reads_models_from_flags_and_a_file() {
    let parsed = parse_check_hosts(
        &args(&[
            "--model", "a/one", "--models", "list.txt", "--tries", "2", "--zdr",
        ]),
        |_| Ok("# small\nb/two  # 8B\n\nc/three\n".into()),
    )
    .unwrap();
    assert_eq!(parsed.models, ["a/one", "b/two", "c/three"]);
    assert_eq!(parsed.tries, 2);
    assert!(parsed.zero_retention);
}

#[test]
fn check_hosts_refuses_bad_arguments() {
    assert!(parse_check_hosts(&args(&[]), no_file).is_err());
    assert!(parse_check_hosts(&args(&["--model"]), no_file).is_err());
    assert!(parse_check_hosts(&args(&["--model", "a", "--tries", "0"]), no_file).is_err());
    assert!(parse_check_hosts(&args(&["--model", "a", "--bogus"]), no_file).is_err());
    assert!(parse_check_hosts(&args(&["--models", "x"]), no_file).is_err());
    let parsed = parse_check_hosts(&args(&["--model", "a"]), no_file).unwrap();
    assert_eq!(parsed.tries, 3);
    assert!(!parsed.zero_retention);
}

#[test]
fn a_litellm_entry_goes_to_the_litellm_server_and_others_to_openrouter() {
    let lookup = |name: &str| match name {
        "LITELLM_BASE_URL" => Ok("https://llm.example.net/".to_string()),
        "LITELLM_API_KEY" => Ok("lk".to_string()),
        "OPENROUTER_API_KEY" => Ok("ok".to_string()),
        other => Err(format!("{other} is not set")),
    };
    let local = endpoint_for("litellm:qwen-next", lookup).unwrap();
    assert_eq!(local.provider, Provider::Custom);
    assert_eq!(local.base_url, "https://llm.example.net/v1");
    assert_eq!(local.model, "qwen-next");
    assert_eq!(local.credential.as_deref(), Some("lk"));
    let routed = endpoint_for("openai/gpt-6-luna", lookup).unwrap();
    assert_eq!(routed.provider, Provider::OpenRouter);
    assert_eq!(routed.model, "openai/gpt-6-luna");
    assert_eq!(routed.credential.as_deref(), Some("ok"));
    let missing = endpoint_for("litellm:m", |name: &str| Err(format!("{name} is not set")));
    assert!(missing.unwrap_err().contains("LITELLM_BASE_URL"));
}
