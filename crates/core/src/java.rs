use tokio::process::Command;

use crate::models::JavaCheckResult;

pub async fn check_java() -> JavaCheckResult {
    match Command::new("java").arg("-version").output().await {
        Ok(output) => {
            let raw_output = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let version = extract_java_version(&raw_output);
            let ok = version
                .as_deref()
                .and_then(java_major_version)
                .map(|major| major >= 17)
                .unwrap_or(false);

            JavaCheckResult {
                found: true,
                version,
                raw_output,
                ok,
            }
        }
        Err(err) => JavaCheckResult {
            found: false,
            version: None,
            raw_output: err.to_string(),
            ok: false,
        },
    }
}

fn extract_java_version(output: &str) -> Option<String> {
    let start = output.find('"')?;
    let end = output[start + 1..].find('"')?;
    Some(output[start + 1..start + 1 + end].to_string())
}

fn java_major_version(version: &str) -> Option<u32> {
    let first = version.split('.').next()?;
    if first == "1" {
        version.split('.').nth(1)?.parse().ok()
    } else {
        first.parse().ok()
    }
}
