use crate::backend::{Descriptor, Format};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Serialize)]
pub struct Issue {
    pub path: PathBuf,
    pub error: String,
}
#[derive(Serialize)]
pub struct Discovery {
    pub candidates: Vec<PathBuf>,
    pub issues: Vec<Issue>,
}
#[derive(Serialize)]
pub struct ScannedPlugin {
    pub path: PathBuf,
    pub descriptors: Vec<Descriptor>,
}
#[derive(Serialize)]
pub struct ScanReport {
    pub format: &'static str,
    pub plugins: Vec<ScannedPlugin>,
    pub issues: Vec<Issue>,
}
pub enum ProbeOutcome {
    Plugins(Vec<Descriptor>),
    Failed(String),
}
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub const OUTPUT_LIMIT: u64 = 1024 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 16384;
const MAX_CANDIDATES: usize = 4096;
const MAX_ISSUES: usize = 128;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
fn format_name(format: Format) -> &'static str {
    match format {
        Format::Clap => "clap",
        Format::Vst3 => "vst3",
    }
}
fn issue(issues: &mut Vec<Issue>, path: &Path, error: impl ToString) {
    if issues.len() < MAX_ISSUES {
        issues.push(Issue {
            path: path.to_path_buf(),
            error: error.to_string(),
        });
    }
}
pub fn discover(format: Format, roots: &[PathBuf]) -> Discovery {
    let mut result = Discovery {
        candidates: Vec::new(),
        issues: Vec::new(),
    };
    let mut pending: Vec<_> = roots
        .iter()
        .take(64)
        .map(|path| (path.clone(), 0))
        .collect();
    if roots.len() > 64 {
        issue(
            &mut result.issues,
            Path::new("."),
            "search root capacity exceeded",
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = pending.len();
    while let Some((path, depth)) = pending.pop() {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                issue(&mut result.issues, &path, error);
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let canonical = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) => {
                issue(&mut result.issues, &path, error);
                continue;
            }
        };
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let extension = canonical
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        let is_bundle = matches!(extension, "vst3" | "clap");
        if extension == format_name(format)
            && (metadata.is_file() || matches!(format, Format::Vst3) && metadata.is_dir())
        {
            if result.candidates.len() == MAX_CANDIDATES {
                issue(
                    &mut result.issues,
                    &canonical,
                    "plugin candidate capacity exceeded",
                );
                break;
            }
            result.candidates.push(canonical);
            continue;
        }
        if !metadata.is_dir() || is_bundle {
            continue;
        }
        if depth == MAX_DEPTH {
            issue(
                &mut result.issues,
                &canonical,
                "directory depth limit exceeded",
            );
            continue;
        }
        let directory = match std::fs::read_dir(&canonical) {
            Ok(directory) => directory,
            Err(error) => {
                issue(&mut result.issues, &canonical, error);
                continue;
            }
        };
        for entry in directory {
            if entries >= MAX_ENTRIES {
                issue(
                    &mut result.issues,
                    &canonical,
                    "directory entry capacity exceeded",
                );
                break;
            }
            entries += 1;
            match entry {
                Ok(entry) => pending.push((entry.path(), depth + 1)),
                Err(error) => issue(&mut result.issues, &canonical, error),
            }
        }
    }
    result.candidates.sort();
    result
}
struct Capture {
    directory: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}
impl Capture {
    fn create() -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let directory = std::env::temp_dir().join(format!(
                "opendaw-scan-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        stdout: directory.join("stdout"),
                        stderr: directory.join("stderr"),
                        directory,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "cannot create unique probe capture",
        ))
    }
    fn file(path: &Path) -> std::io::Result<std::fs::File> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path)
    }
    fn read(path: &Path) -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(std::io::Error::other("probe output limit exceeded"));
        }
        Ok(bytes)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
struct RunningChild {
    child: std::process::Child,
    terminated: bool,
}
impl RunningChild {
    fn exited(&mut self) -> std::io::Result<bool> {
        #[cfg(target_os = "linux")]
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            if libc::waitid(
                libc::P_PID,
                self.child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(info.si_pid() != 0)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(self.child.try_wait()?.is_some())
        }
    }
    fn terminate(&mut self) -> std::io::Result<std::process::ExitStatus> {
        if !self.terminated {
            self.terminated = true;
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
        }
        self.child.wait()
    }
}
impl Drop for RunningChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}
pub fn probe(executable: &Path, path: &Path, format: Format, timeout: Duration) -> ProbeOutcome {
    match probe_inner(executable, path, format, timeout.min(PROBE_TIMEOUT)) {
        Ok(descriptors) => ProbeOutcome::Plugins(descriptors),
        Err(error) => ProbeOutcome::Failed(error.to_string()),
    }
}
fn probe_inner(
    executable: &Path,
    path: &Path,
    format: Format,
    timeout: Duration,
) -> std::io::Result<Vec<Descriptor>> {
    #[cfg(not(unix))]
    {
        let _ = (executable, path, format, timeout);
        return Err(std::io::Error::other(
            "isolated scanner currently requires Unix resource limits",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let capture = Capture::create()?;
        let mut command = std::process::Command::new(executable);
        command
            .arg("probe")
            .arg(path)
            .arg("--format")
            .arg(format_name(format))
            .stdin(std::process::Stdio::null())
            .stdout(Capture::file(&capture.stdout)?)
            .stderr(Capture::file(&capture.stderr)?)
            .process_group(0);
        unsafe {
            command.pre_exec(|| {
                let file_limit = libc::rlimit {
                    rlim_cur: OUTPUT_LIMIT as libc::rlim_t,
                    rlim_max: OUTPUT_LIMIT as libc::rlim_t,
                };
                let core_limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &file_limit) != 0
                    || libc::setrlimit(libc::RLIMIT_CORE, &core_limit) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let start = std::time::Instant::now();
        let mut running = RunningChild {
            child: command.spawn()?,
            terminated: false,
        };
        loop {
            if running.exited()? {
                break;
            }
            if start.elapsed() >= timeout {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "plugin probe timeout",
                ));
            }
            std::thread::sleep(
                Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
            );
        }
        let status = running.terminate()?;
        let stdout = Capture::read(&capture.stdout)?;
        let stderr = Capture::read(&capture.stderr)?;
        if !status.success() {
            let detail = String::from_utf8_lossy(&stderr[..stderr.len().min(2048)]);
            return Err(std::io::Error::other(format!(
                "plugin probe exited {status}: {detail}"
            )));
        }
        let marker = stdout
            .split(|byte| *byte == b'\n')
            .filter_map(|line| line.strip_prefix(b"OPENDAW_SCAN "))
            .next_back()
            .ok_or_else(|| std::io::Error::other("plugin probe did not emit metadata marker"))?;
        let descriptors: Vec<Descriptor> =
            serde_json::from_slice(marker).map_err(std::io::Error::other)?;
        if descriptors.len() > MAX_CANDIDATES {
            return Err(std::io::Error::other("plugin descriptor capacity exceeded"));
        }
        Ok(descriptors)
    }
}
pub fn scan(format: Format, roots: &[PathBuf]) -> ScanReport {
    match std::env::current_exe() {
        Ok(executable) => scan_using(
            format,
            roots,
            &executable,
            PROBE_TIMEOUT,
            MAX_METADATA_BYTES,
        ),
        Err(error) => ScanReport {
            format: format_name(format),
            plugins: Vec::new(),
            issues: vec![Issue {
                path: PathBuf::from("."),
                error: error.to_string(),
            }],
        },
    }
}
fn scan_using(
    format: Format,
    roots: &[PathBuf],
    executable: &Path,
    timeout: Duration,
    metadata_budget: usize,
) -> ScanReport {
    let Discovery { candidates, issues } = discover(format, roots);
    let mut report = ScanReport {
        format: format_name(format),
        plugins: Vec::new(),
        issues,
    };
    let mut bytes = 0_usize;
    let mut count = 0_usize;
    let candidate_count = candidates.len();
    for (index, path) in candidates.into_iter().enumerate() {
        match probe(executable, &path, format, timeout) {
            ProbeOutcome::Plugins(descriptors) => {
                let size = descriptors
                    .iter()
                    .fold(path.as_os_str().len(), |size, descriptor| {
                        size.saturating_add(std::mem::size_of::<Descriptor>())
                            .saturating_add(descriptor.id.len())
                            .saturating_add(descriptor.name.len())
                            .saturating_add(descriptor.vendor.len())
                            .saturating_add(descriptor.version.len())
                    });
                if size > metadata_budget.saturating_sub(bytes)
                    || descriptors.len() > MAX_CANDIDATES.saturating_sub(count)
                {
                    if report.issues.len() == MAX_ISSUES {
                        report.issues.pop();
                    }
                    issue(
                        &mut report.issues,
                        &path,
                        format!(
                            "scan metadata budget exceeded; {} remaining candidates skipped",
                            candidate_count - index
                        ),
                    );
                    break;
                }
                bytes += size;
                count += descriptors.len();
                report.plugins.push(ScannedPlugin { path, descriptors });
            }
            ProbeOutcome::Failed(error) => issue(&mut report.issues, &path, error),
        }
    }
    report
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{Instant, SystemTime, UNIX_EPOCH},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "opendaw-scanner-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        #[cfg(unix)]
        fn helper(&self, script: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = self.0.join("helper");
            fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn discovers_matching_candidates_without_recursing_bundles_or_symlinks() {
        let directory = Directory::new();
        fs::create_dir(directory.0.join("nested")).unwrap();
        fs::write(directory.0.join("nested/gain.clap"), []).unwrap();
        fs::write(directory.0.join("ignored.so"), []).unwrap();
        fs::create_dir(directory.0.join("effect.vst3")).unwrap();
        fs::write(directory.0.join("effect.vst3/inside.clap"), []).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&directory.0, directory.0.join("nested/cycle")).unwrap();
        let result = discover(Format::Clap, &[directory.0.clone(), directory.0.clone()]);
        assert_eq!(result.candidates, [directory.0.join("nested/gain.clap")]);
        let result = discover(Format::Vst3, std::slice::from_ref(&directory.0));
        assert_eq!(result.candidates, [directory.0.join("effect.vst3")]);
    }
    #[cfg(unix)]
    #[test]
    fn reads_json_marker_after_plugin_logs() {
        let directory = Directory::new();
        let helper = directory.helper("printf 'plugin log\\nOPENDAW_SCAN [{\"id\":\"gain\",\"name\":\"Gain\",\"vendor\":\"Test\",\"version\":\"1\"}]\\n'");
        match probe(
            &helper,
            Path::new("gain.clap"),
            Format::Clap,
            Duration::from_secs(1),
        ) {
            ProbeOutcome::Plugins(descriptors) => assert_eq!(descriptors[0].id, "gain"),
            ProbeOutcome::Failed(error) => panic!("{error}"),
        }
    }
    #[cfg(unix)]
    #[test]
    fn crashes_timeouts_and_inherited_output_do_not_block_scanning() {
        let directory = Directory::new();
        let helper = directory.helper("kill -SEGV $$");
        assert!(matches!(
            probe(
                &helper,
                Path::new("crash.clap"),
                Format::Clap,
                Duration::from_secs(1)
            ),
            ProbeOutcome::Failed(_)
        ));
        let helper = directory.helper("sleep 20 &\nwait");
        let start = Instant::now();
        match probe(
            &helper,
            Path::new("hang.clap"),
            Format::Clap,
            Duration::from_millis(100),
        ) {
            ProbeOutcome::Failed(error) => assert!(error.contains("timeout")),
            _ => panic!("hanging plugin unexpectedly succeeded"),
        }
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[cfg(unix)]
    #[test]
    fn rejects_missing_malformed_and_oversized_probe_output() {
        let directory = Directory::new();
        for script in [
            "echo no-marker",
            "echo 'OPENDAW_SCAN garbage'",
            "head -c 2000000 /dev/zero",
        ] {
            let helper = directory.helper(script);
            assert!(matches!(
                probe(
                    &helper,
                    Path::new("bad.clap"),
                    Format::Clap,
                    Duration::from_secs(1)
                ),
                ProbeOutcome::Failed(_)
            ));
        }
    }
    #[cfg(unix)]
    #[test]
    fn scan_continues_after_crashing_and_hanging_candidates() {
        let directory = Directory::new();
        for name in ["a-crash.clap", "b-hang.clap", "c-good.clap"] {
            fs::write(directory.0.join(name), []).unwrap();
        }
        let helper = directory.helper("case \"$2\" in *crash.clap) kill -SEGV $$;; *hang.clap) sleep 20 & wait;; *) printf 'OPENDAW_SCAN [{\"id\":\"gain\",\"name\":\"Gain\",\"vendor\":\"Test\",\"version\":\"1\"}]\\n';; esac");
        let report = scan_using(
            Format::Clap,
            std::slice::from_ref(&directory.0),
            &helper,
            Duration::from_millis(100),
            MAX_METADATA_BYTES,
        );
        assert_eq!(report.plugins.len(), 1);
        assert_eq!(report.plugins[0].descriptors[0].id, "gain");
        assert_eq!(report.issues.len(), 2);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.error.contains("timeout")));
    }
    #[cfg(unix)]
    #[test]
    fn aggregate_metadata_budget_stops_retaining_probe_results() {
        let directory = Directory::new();
        for name in ["a-good.clap", "b-good.clap"] {
            fs::write(directory.0.join(name), []).unwrap();
        }
        let helper = directory.helper("printf 'OPENDAW_SCAN [{\"id\":\"gain\",\"name\":\"Gain\",\"vendor\":\"Test\",\"version\":\"1\"}]\\n'");
        let budget = directory.0.join("a-good.clap").as_os_str().len()
            + std::mem::size_of::<Descriptor>()
            + 13;
        let report = scan_using(
            Format::Clap,
            std::slice::from_ref(&directory.0),
            &helper,
            Duration::from_secs(1),
            budget,
        );
        assert_eq!(report.plugins.len(), 1);
        assert!(report.issues[0]
            .error
            .contains("metadata budget exceeded; 1 remaining"));
    }
}
