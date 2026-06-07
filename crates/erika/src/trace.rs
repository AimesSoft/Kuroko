use std::{env, fs::OpenOptions, io::Write, path::PathBuf, time::Duration};

pub(crate) fn enabled() -> bool {
    env_flag("ERIKA_CLOCK_TRACE") || env_flag("ERIKA_DANMAKU_TRACE")
}

pub(crate) fn log(line: impl AsRef<str>) {
    if !enabled() {
        return;
    }
    let line = line.as_ref();
    eprintln!("{line}");
    let path = env::var_os("ERIKA_DANMAKU_TRACE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/erika_danmaku_trace.log"));
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{line}"));
}

pub(crate) fn duration_label(value: Option<Duration>) -> String {
    value
        .map(|duration| format!("{:.3}", duration.as_secs_f64()))
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn duration_regressed(next: Duration, previous: Duration) -> bool {
    previous
        .checked_sub(next)
        .is_some_and(|delta| delta > Duration::from_millis(5))
}

pub(crate) fn duration_diff(a: Duration, b: Duration) -> Duration {
    a.checked_sub(b)
        .or_else(|| b.checked_sub(a))
        .unwrap_or(Duration::ZERO)
}

pub(crate) fn env_flag(name: &str) -> bool {
    match env::var(name).ok().as_deref() {
        Some("0" | "false" | "FALSE" | "off" | "OFF" | "") | None => false,
        Some(_) => true,
    }
}
