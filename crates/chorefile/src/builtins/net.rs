//! `download` — the one builtin that touches the network.
//!
//! The transport is `ureq` over rustls, so there is no OpenSSL and the binary
//! stays static everywhere. Everything about a download that can be decided
//! without a socket — URL rewriting, where the bytes land — lives in a pure
//! function below so it can be tested offline.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::exec::{self, Ctx, Output};

/// The builtins defined in this module.
pub fn lookup(name: &str) -> Option<exec::Builtin> {
    match name {
        "download" => Some(download),
        _ => None,
    }
}

/// The scheme for a GitHub release asset: `gh://owner/repo/tag/asset`.
const GH_SCHEME: &str = "gh://";
/// Redirect cap. GitHub sends two hops (api → S3), so ten is generous.
const MAX_REDIRECTS: u32 = 10;
/// How often the progress line may be rewritten.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

fn run_err(message: impl Into<String>) -> Error {
    Error::Run {
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

/// A parsed `download` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub url: String,
    pub dest: String,
    pub retries: u32,
    pub timeout: u64,
    pub sha256: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            url: String::new(),
            dest: String::new(),
            // Three attempts after the first is enough to ride out a flaky
            // proxy without making a genuinely broken URL take a minute.
            retries: 3,
            timeout: 60,
            sha256: None,
        }
    }
}

/// Parse `download <url> <dest> [--retries n] [--timeout s] [--sha256 h]`.
pub fn parse_args(rest: &[String]) -> Result<Args> {
    let mut args = Args::default();
    let mut positional = Vec::new();
    let mut it = rest.iter();

    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| run_err(format!("download: {flag} needs a value")))
        };
        match arg.as_str() {
            "--retries" => {
                let v = value("--retries")?;
                args.retries = v
                    .parse()
                    .map_err(|_| run_err(format!("download: --retries {v} is not a number")))?;
            }
            "--timeout" => {
                let v = value("--timeout")?;
                args.timeout = v
                    .parse()
                    .map_err(|_| run_err(format!("download: --timeout {v} is not a number")))?;
            }
            "--sha256" => {
                let v = value("--sha256")?;
                if v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(run_err(format!(
                        "download: --sha256 {v} is not a 64-character hex digest"
                    )));
                }
                args.sha256 = Some(v.to_ascii_lowercase());
            }
            other if other.starts_with("--") => {
                return Err(run_err(format!("download: unknown option {other}")));
            }
            other => positional.push(other.to_string()),
        }
    }

    let [url, dest] = positional.as_slice() else {
        return Err(run_err("download: expected <url> <dest>"));
    };
    args.url = url.clone();
    args.dest = dest.clone();
    Ok(args)
}

// ---------------------------------------------------------------------------
// URL and destination rules — pure, so the tests never open a socket
// ---------------------------------------------------------------------------

/// Rewrite a chorefile URL into one the HTTP client understands.
///
/// `gh://owner/repo/tag/asset` becomes the release-asset download URL. That
/// spelling is short enough to write by hand and survives GitHub changing the
/// shape of the real URL.
pub fn resolve_url(url: &str) -> Result<String> {
    if let Some(rest) = url.strip_prefix(GH_SCHEME) {
        let parts: Vec<&str> = rest.split('/').collect();
        // The asset name itself may not contain `/`, so exactly four parts.
        let [owner, repo, tag, asset] = parts.as_slice() else {
            return Err(run_err(format!(
                "download: {url} is not gh://owner/repo/tag/asset"
            )));
        };
        if [owner, repo, tag, asset].iter().any(|p| p.is_empty()) {
            return Err(run_err(format!(
                "download: {url} is not gh://owner/repo/tag/asset"
            )));
        }
        return Ok(format!(
            "https://github.com/{owner}/{repo}/releases/download/{tag}/{asset}"
        ));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(url.to_string());
    }
    Err(run_err(format!(
        "download: {url} has no scheme chore can fetch (http, https or gh)"
    )))
}

/// The filename a URL implies: its last non-empty path segment, with any
/// query or fragment removed.
pub fn remote_name(url: &str) -> Option<&str> {
    let path = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    path.rsplit('/').find(|s| !s.is_empty())
}

/// Where the bytes actually land.
///
/// A `dest` that ends in `/`, or that already names a directory, means "into
/// this directory, keep the remote filename". Anything else is the output
/// file path. `as_dir` is passed in rather than probed so this stays a pure
/// function; see [`looks_like_dir`] for the half of the answer the string
/// carries.
pub fn dest_file(dest: &Path, as_dir: bool, url: &str) -> Result<PathBuf> {
    if !as_dir {
        return Ok(dest.to_path_buf());
    }
    let name = remote_name(url).ok_or_else(|| {
        run_err(format!(
            "download: {url} has no filename, so {} needs to name the output file",
            crate::vars::display(dest)
        ))
    })?;
    Ok(dest.join(name))
}

/// A trailing separator is the writer saying "this is a directory". It has to
/// be read off the argument, because turning it into a `Path` drops it.
pub fn looks_like_dir(arg: &str) -> bool {
    arg.ends_with('/') || arg.ends_with('\\')
}

/// The bearer token to send, if the environment offers one. Honouring both
/// spellings means `gh auth` and CI both work without extra wiring.
///
/// Read through [`Ctx::env`] rather than `std::env::var`, so an
/// `env GITHUB_TOKEN $(read .token)` earlier in the task is a token this
/// download actually sends — chore's `env` binds a name for the frame, not
/// for the process.
fn github_token(ctx: &Ctx<'_>) -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .filter_map(|k| ctx.env.get(k))
        .find(|v| !v.trim().is_empty())
}

// ---------------------------------------------------------------------------
// the builtin
// ---------------------------------------------------------------------------

fn download(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = parse_args(ctx.rest())?;
    if ctx.dry {
        // The command line was already echoed; a preview must not open a
        // socket or leave a file behind.
        return Ok(Output::ok());
    }

    let url = resolve_url(&args.url)?;
    let dest_abs = ctx.path(&args.dest);
    let as_dir = looks_like_dir(&args.dest) || dest_abs.is_dir();
    let dest = dest_file(&dest_abs, as_dir, &url)?;

    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| {
            run_err(format!(
                "download: cannot create {}: {e}",
                crate::vars::display(parent)
            ))
        })?;
    }

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(args.timeout)))
        .max_redirects(MAX_REDIRECTS)
        // We inspect the status ourselves so 5xx can be retried and 4xx
        // cannot.
        .http_status_as_error(false)
        .build()
        .new_agent();

    let token = github_token(ctx);
    let mut last: Option<String> = None;

    for attempt in 0..=args.retries {
        if attempt > 0 {
            // Exponential, capped: 1s, 2s, 4s, 8s, 8s...
            let backoff = Duration::from_secs(1 << (attempt - 1).min(3));
            writeln!(
                ctx.out,
                "download: retrying {url} in {}s ({})",
                backoff.as_secs(),
                last.as_deref().unwrap_or("previous attempt failed")
            )?;
            std::thread::sleep(backoff);
        }

        match attempt_download(ctx, &agent, &url, &dest, token.as_deref(), &args) {
            Ok(()) => return Ok(Output::ok()),
            Err(Retry::Fatal(e)) => return Err(e),
            Err(Retry::Again(m)) => last = Some(m),
        }
    }

    Err(run_err(format!(
        "download: {url} failed after {} attempts: {}",
        args.retries + 1,
        last.unwrap_or_else(|| "unknown error".into())
    )))
}

/// A failed attempt: worth another go, or not.
enum Retry {
    Again(String),
    Fatal(Error),
}

impl From<Error> for Retry {
    fn from(e: Error) -> Self {
        Retry::Fatal(e)
    }
}

fn attempt_download(
    ctx: &mut Ctx<'_>,
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    token: Option<&str>,
    args: &Args,
) -> std::result::Result<(), Retry> {
    let mut request = agent.get(url);
    if let Some(token) = token {
        // ureq only replays auth headers to the same host, so the redirect to
        // the asset CDN does not leak the token.
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let response = request
        .call()
        .map_err(|e| Retry::Again(format!("{url}: {e}")))?;

    let status = response.status().as_u16();
    if status >= 500 || status == 429 {
        return Err(Retry::Again(format!("{url}: HTTP {status}")));
    }
    if !(200..300).contains(&status) {
        let hint = if status == 404 && url.contains("/releases/download/") && token.is_none() {
            " (set GITHUB_TOKEN if the repository is private)"
        } else {
            ""
        };
        return Err(Retry::Fatal(run_err(format!(
            "download: {url}: HTTP {status}{hint}"
        ))));
    }

    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    // Stream into a sibling temp file and rename at the end, so an
    // interrupted or corrupt download never leaves a truncated artifact that
    // a later run would mistake for a good one.
    let temp = temp_path(dest);
    let mut sink = File::create(&temp).map_err(|e| {
        Retry::Fatal(run_err(format!(
            "download: cannot write {}: {e}",
            crate::vars::display(&temp)
        )))
    })?;

    let mut reader = response.into_body().into_reader();
    let outcome = copy(ctx, &mut reader, &mut sink, url, dest, total);
    let digest = match outcome {
        Ok(digest) => digest,
        Err(e) => {
            let _ = fs::remove_file(&temp);
            return Err(Retry::Again(format!("{url}: {e}")));
        }
    };
    if let Err(e) = sink.sync_all() {
        let _ = fs::remove_file(&temp);
        return Err(Retry::Again(format!("{url}: {e}")));
    }
    drop(sink);

    if let Some(want) = &args.sha256
        && &digest != want
    {
        let _ = fs::remove_file(&temp);
        return Err(Retry::Fatal(run_err(format!(
            "download: {url}: sha256 mismatch\n  expected {want}\n  actual   {digest}"
        ))));
    }

    fs::rename(&temp, dest).map_err(|e| {
        let _ = fs::remove_file(&temp);
        Retry::Fatal(run_err(format!(
            "download: cannot move into place {}: {e}",
            crate::vars::display(dest)
        )))
    })?;
    Ok(())
}

/// A temp name beside the destination — the same directory, so the final
/// rename is atomic instead of a cross-filesystem copy.
fn temp_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    let pid = std::process::id();
    dest.with_file_name(format!(".{name}.{pid}.part"))
}

/// Copy the body through a hasher, reporting progress, and return the hex
/// digest of what was written.
fn copy(
    ctx: &mut Ctx<'_>,
    reader: &mut dyn Read,
    sink: &mut File,
    url: &str,
    dest: &Path,
    total: Option<u64>,
) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;

    // `ctx.interactive`, never a direct tty probe: under `$(download ...)`
    // stdout can be a terminal while this builtin's sink is a buffer.
    let tty = ctx.interactive;
    let mut last_tick = Instant::now();
    writeln!(ctx.out, "download: {url} -> {}", crate::vars::display(dest))?;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        sink.write_all(&buf[..n])?;
        done += n as u64;

        // On a terminal, redraw one line in place. Off a terminal (CI, a pipe,
        // a `$(...)` capture) say nothing until the end, so a log is one line
        // per download instead of thousands.
        if tty && last_tick.elapsed() >= PROGRESS_EVERY {
            last_tick = Instant::now();
            write!(ctx.out, "\r  {}\x1b[K", progress(done, total))?;
            ctx.out.flush()?;
        }
    }

    if tty {
        write!(ctx.out, "\r\x1b[K")?;
    }
    writeln!(ctx.out, "  {}", progress(done, total))?;

    Ok(hex(&hasher.finalize()))
}

fn progress(done: u64, total: Option<u64>) -> String {
    match total {
        Some(total) if total > 0 => format!(
            "{} / {} ({}%)",
            human(done),
            human(total),
            done.saturating_mul(100) / total
        ),
        _ => human(done),
    }
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn gh_scheme_maps_to_a_release_asset() {
        assert_eq!(
            resolve_url("gh://cli/cli/v2.62.0/gh_2.62.0_macOS_arm64.zip").unwrap(),
            "https://github.com/cli/cli/releases/download/v2.62.0/gh_2.62.0_macOS_arm64.zip"
        );
    }

    #[test]
    fn gh_scheme_needs_all_four_parts() {
        assert!(resolve_url("gh://cli/cli/v2.62.0").is_err());
        assert!(resolve_url("gh://cli/cli/v2.62.0/a/b").is_err());
        assert!(resolve_url("gh://cli//v2.62.0/a").is_err());
    }

    #[test]
    fn http_urls_pass_through_and_others_are_rejected() {
        assert_eq!(resolve_url("http://x/y").unwrap(), "http://x/y");
        assert_eq!(resolve_url("https://x/y").unwrap(), "https://x/y");
        assert!(resolve_url("ftp://x/y").is_err());
        assert!(resolve_url("/tmp/file").is_err());
    }

    #[test]
    fn remote_name_ignores_query_and_trailing_slash() {
        assert_eq!(remote_name("https://x/a/b.tar.gz"), Some("b.tar.gz"));
        assert_eq!(remote_name("https://x/a/b.zip?token=1"), Some("b.zip"));
        assert_eq!(remote_name("https://x/a/b.zip#frag"), Some("b.zip"));
        assert_eq!(remote_name("https://x/a/"), Some("a"));
        assert_eq!(remote_name("https://x/"), Some("x"));
    }

    #[test]
    fn a_trailing_slash_or_existing_dir_keeps_the_remote_filename() {
        let url = "https://x/a/tool.tar.gz";
        let into = PathBuf::from("vendor").join("tool.tar.gz");

        // A trailing separator, read off the argument before it became a path.
        assert!(looks_like_dir("vendor/"));
        assert!(!looks_like_dir("vendor/tool.tgz"));

        // ...or an existing directory: same answer either way.
        assert_eq!(dest_file(Path::new("vendor"), true, url).unwrap(), into);
        assert_eq!(
            dest_file(Path::new("vendor/tool.tgz"), false, url).unwrap(),
            PathBuf::from("vendor/tool.tgz")
        );
    }

    #[test]
    fn defaults_and_flags() {
        let a = parse_args(&args(&["https://x/y", "out"])).unwrap();
        assert_eq!(a.retries, 3);
        assert_eq!(a.timeout, 60);
        assert_eq!(a.sha256, None);

        let digest = "a".repeat(64);
        let a = parse_args(&args(&[
            "https://x/y",
            "out",
            "--retries",
            "1",
            "--timeout",
            "5",
            "--sha256",
            &digest.to_uppercase(),
        ]))
        .unwrap();
        assert_eq!((a.retries, a.timeout), (1, 5));
        assert_eq!(a.sha256.as_deref(), Some(digest.as_str()));
    }

    #[test]
    fn bad_arguments_are_rejected() {
        assert!(parse_args(&args(&["https://x/y"])).is_err());
        assert!(parse_args(&args(&["a", "b", "c"])).is_err());
        assert!(parse_args(&args(&["a", "b", "--nope"])).is_err());
        assert!(parse_args(&args(&["a", "b", "--retries"])).is_err());
        assert!(parse_args(&args(&["a", "b", "--sha256", "short"])).is_err());
    }

    #[test]
    fn progress_is_readable() {
        assert_eq!(progress(512, None), "512 B");
        assert_eq!(progress(1024, Some(2048)), "1.0 KB / 2.0 KB (50%)");
    }
}
