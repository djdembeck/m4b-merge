use std::process::Command;

fn release_version(value: &str) -> Option<&str> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let mut components = value.split('.');
    for _ in 0..3 {
        let part = components.next()?;
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
    }
    components.next().is_none().then_some(value)
}

fn git(args: &[&str]) -> Option<String> {
    let result = Command::new("git").args(args).output().ok()?;
    result.status.success().then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=M4B_MERGE_VERSION");
    println!("cargo:rerun-if-env-changed=M4B_MERGE_SOURCE");
    if let Some(path) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={path}");
    }
    if let Some(path) = git(&["symbolic-ref", "--quiet", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(path) = git(&["rev-parse", "--git-path", "packed-refs"]) {
        println!("cargo:rerun-if-changed={path}");
    }
    let version = match std::env::var("M4B_MERGE_VERSION") {
        Ok(value) if !value.is_empty() => {
            release_version(&value).expect("M4B_MERGE_VERSION must be X.Y.Z or vX.Y.Z").to_owned()
        }
        _ => {
            let source = std::env::var("M4B_MERGE_SOURCE")
                .ok()
                .filter(|value| !value.is_empty())
                .or_else(|| git(&["rev-parse", "HEAD"]));
            match source {
                Some(source) => {
                    assert!(
                        source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()),
                        "M4B_MERGE_SOURCE must be a full commit SHA"
                    );
                    format!("dev-{}", &source[..12])
                }
                None => "dev".to_owned(),
            }
        }
    };
    println!("cargo:rustc-env=M4B_MERGE_APP_VERSION={version}");
}
