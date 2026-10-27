//! Server health: CPU, memory, disk, load and top processes gathered over the existing SSH
//! connection, plus a cheap TCP reachability probe for the host cards.
//!
//! One short POSIX `sh` script prints labelled sections and `parse` turns them into a
//! `Snapshot`. Linux reports everything; other systems report what `df`, `ps` and `uptime`
//! provide and leave the rest empty. CPU use needs two samples, so it is computed from the
//! previous poll's counters instead of sleeping on the server.

use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tokio::net::TcpStream;

use crate::conn::ssh::SshClient;

const POLL_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Samples kept for the sparkline.
pub const HISTORY: usize = 60;

/// Prints `@section` headers followed by raw output; every command may fail silently.
const SCRIPT: &str = r#"
echo @os; uname -s 2>/dev/null
echo @uptime; cut -d' ' -f1 /proc/uptime 2>/dev/null
echo @load; cat /proc/loadavg 2>/dev/null || uptime 2>/dev/null
echo @cpu; grep '^cpu ' /proc/stat 2>/dev/null
echo @mem; grep -E '^(MemTotal|MemAvailable|MemFree|Buffers|Cached|SwapTotal|SwapFree):' /proc/meminfo 2>/dev/null
echo @disk; df -kP 2>/dev/null | tail -n +2
echo @proc; ps -eo pid=,pcpu=,pmem=,comm= 2>/dev/null | sort -k2 -nr | head -n 6
"#;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuTimes {
    pub total: u64,
    pub idle: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Disk {
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
}

impl Disk {
    pub fn percent(&self) -> f32 {
        percent(self.used_kb, self.total_kb)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub cpu: f32,
    pub mem: f32,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub os: String,
    pub uptime_secs: Option<u64>,
    pub load: Option<[f32; 3]>,
    /// Raw counters, kept so the next poll can compute CPU use.
    pub cpu_times: Option<CpuTimes>,
    /// Percent busy since the previous poll; `None` until there are two samples.
    pub cpu_percent: Option<f32>,
    pub mem_total_kb: Option<u64>,
    pub mem_used_kb: Option<u64>,
    pub swap_total_kb: Option<u64>,
    pub swap_used_kb: Option<u64>,
    pub disks: Vec<Disk>,
    pub processes: Vec<Process>,
}

impl Snapshot {
    pub fn mem_percent(&self) -> Option<f32> {
        Some(percent(self.mem_used_kb?, self.mem_total_kb?))
    }
}

/// Live state of one panel: the latest snapshot and the sparkline history.
#[derive(Default)]
pub struct Monitor {
    /// Session this state belongs to; switching panes starts over.
    pub session: u64,
    pub snapshot: Option<Snapshot>,
    pub error: Option<String>,
    pub cpu_history: Vec<f32>,
    pub mem_history: Vec<f32>,
    pub in_flight: bool,
    /// UI time of the last poll request.
    pub last_poll: f64,
    pub updated: Option<Instant>,
}

impl Monitor {
    pub fn reset(&mut self, session: u64) {
        *self = Self {
            session,
            ..Default::default()
        };
    }

    pub fn apply(&mut self, result: Result<Snapshot, String>) {
        self.in_flight = false;
        match result {
            Ok(snapshot) => {
                if let Some(cpu) = snapshot.cpu_percent {
                    push_capped(&mut self.cpu_history, cpu);
                }
                if let Some(mem) = snapshot.mem_percent() {
                    push_capped(&mut self.mem_history, mem);
                }
                self.snapshot = Some(snapshot);
                self.error = None;
                self.updated = Some(Instant::now());
            }
            Err(error) => self.error = Some(error),
        }
    }
}

fn push_capped(history: &mut Vec<f32>, value: f32) {
    history.push(value);
    if history.len() > HISTORY {
        history.remove(0);
    }
}

fn percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 * 100.0 / total as f64).clamp(0.0, 100.0) as f32
    }
}

/// Runs the script on the server and parses the result. `previous` supplies the CPU counters
/// from the last poll.
pub async fn poll(client: &SshClient, previous: Option<CpuTimes>) -> Result<Snapshot> {
    let command = format!("sh -c {}", shell_quote(SCRIPT));
    let (output, _) = tokio::time::timeout(POLL_TIMEOUT, client.exec(&command))
        .await
        .map_err(|_| anyhow!("The server did not answer in {}s", POLL_TIMEOUT.as_secs()))??;
    if output.trim().is_empty() {
        return Err(anyhow!("The server returned no data"));
    }
    Ok(parse(&output, previous))
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub fn parse(output: &str, previous: Option<CpuTimes>) -> Snapshot {
    let mut sections: Vec<(&str, Vec<&str>)> = vec![];
    for line in output.lines() {
        if let Some(name) = line.strip_prefix('@') {
            sections.push((name.trim(), vec![]));
        } else if let Some((_, lines)) = sections.last_mut() {
            if !line.trim().is_empty() {
                lines.push(line);
            }
        }
    }
    let section = |name: &str| -> &[&str] {
        sections
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(&[][..], |(_, l)| l.as_slice())
    };

    let mut snapshot = Snapshot {
        os: section("os")
            .first()
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        uptime_secs: section("uptime")
            .first()
            .and_then(|l| l.trim().parse::<f64>().ok())
            .map(|s| s as u64),
        load: section("load").first().and_then(|l| parse_load(l)),
        ..Default::default()
    };

    if let Some(line) = section("cpu").first() {
        let fields: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .map(|f| f.parse().unwrap_or(0))
            .collect();
        // user nice system idle iowait irq softirq steal; guest time is already inside user.
        if fields.len() >= 4 {
            let total: u64 = fields.iter().take(8).sum();
            let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
            let now = CpuTimes { total, idle };
            snapshot.cpu_times = Some(now);
            snapshot.cpu_percent = previous.and_then(|before| {
                let total = now.total.checked_sub(before.total)?;
                let idle = now.idle.checked_sub(before.idle)?;
                (total > 0).then(|| percent(total - idle.min(total), total))
            });
        }
    }

    let meminfo = |key: &str| -> Option<u64> {
        section("mem").iter().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k == key).then(|| v.split_whitespace().next()?.parse().ok())?
        })
    };
    if let Some(total) = meminfo("MemTotal") {
        let available = meminfo("MemAvailable").or_else(|| {
            Some(
                meminfo("MemFree")?
                    + meminfo("Buffers").unwrap_or(0)
                    + meminfo("Cached").unwrap_or(0),
            )
        });
        snapshot.mem_total_kb = Some(total);
        snapshot.mem_used_kb = available.map(|a| total.saturating_sub(a));
    }
    if let (Some(total), Some(free)) = (meminfo("SwapTotal"), meminfo("SwapFree")) {
        if total > 0 {
            snapshot.swap_total_kb = Some(total);
            snapshot.swap_used_kb = Some(total.saturating_sub(free));
        }
    }

    snapshot.disks = parse_disks(section("disk"));
    snapshot.processes = section("proc")
        .iter()
        .filter_map(|l| parse_process(l))
        .collect();
    snapshot
}

fn parse_load(line: &str) -> Option<[f32; 3]> {
    // /proc/loadavg: "0.52 0.58 0.59 1/467 12345"; uptime: "... load average: 0.52, 0.58, 0.59",
    // which some locales print with decimal commas ("0,52, 0,58, 0,59").
    let numbers = line
        .split_once("load average")
        .map_or(line, |(_, rest)| rest);
    let mut values = numbers
        .split_whitespace()
        .map(|token| token.trim_matches([':', 's']).trim_end_matches(','))
        .filter(|token| token.starts_with(|c: char| c.is_ascii_digit()))
        .map(|token| token.replace(',', ".").parse::<f32>());
    Some([
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
    ])
}

fn parse_disks(lines: &[&str]) -> Vec<Disk> {
    const PSEUDO: &[&str] = &[
        "tmpfs", "devtmpfs", "squashfs", "overlay", "efivarfs", "udev", "none", "shm", "devfs",
        "map", "cgroup", "proc", "sysfs",
    ];
    const SKIPPED_MOUNTS: &[&str] = &["/snap", "/run", "/sys", "/proc", "/dev", "/var/lib/docker"];
    let mut seen = std::collections::HashSet::new();
    let mut disks = vec![];
    for line in lines {
        // df -kP: Filesystem 1024-blocks Used Available Capacity Mounted-on (the mount may
        // contain spaces, so only the first five columns are split).
        let mut parts = line.split_whitespace();
        let (Some(fs), Some(total), Some(used), Some(_), Some(_)) = (
            parts.next(),
            parts.next().and_then(|v| v.parse::<u64>().ok()),
            parts.next().and_then(|v| v.parse::<u64>().ok()),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        let mount = parts.collect::<Vec<_>>().join(" ");
        // macOS splits one disk into many system volumes; only the user data volume matters.
        let system_volume =
            mount.starts_with("/System/Volumes/") && mount != "/System/Volumes/Data";
        if total == 0
            || mount.is_empty()
            || system_volume
            || PSEUDO.contains(&fs)
            || SKIPPED_MOUNTS
                .iter()
                .any(|m| mount == *m || mount.starts_with(&format!("{m}/")))
            || !seen.insert(fs.to_string())
        {
            continue;
        }
        disks.push(Disk {
            mount,
            total_kb: total,
            used_kb: used,
        });
    }
    disks.sort_by(|a, b| b.total_kb.cmp(&a.total_kb).then(b.used_kb.cmp(&a.used_kb)));
    disks.truncate(5);
    disks
}

fn parse_process(line: &str) -> Option<Process> {
    let mut parts = line.split_whitespace();
    Some(Process {
        pid: parts.next()?.parse().ok()?,
        cpu: parts.next()?.replace(',', ".").parse().ok()?,
        mem: parts.next()?.replace(',', ".").parse().ok()?,
        name: short_name(&parts.collect::<Vec<_>>().join(" ")),
    })
    .filter(|p| !p.name.is_empty())
}

/// macOS and BSD print the full executable path; keep only the program name.
fn short_name(command: &str) -> String {
    if command.starts_with('/') {
        command.rsplit('/').next().unwrap_or(command).to_string()
    } else {
        command.to_string()
    }
}

/// Round-trip time of a TCP connect to `host:port`, or `None` if it fails or times out.
pub async fn probe(host: &str, port: u16) -> Option<Duration> {
    // One deadline for DNS and connect together; a slow name lookup must not stall a round.
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let address = tokio::net::lookup_host((host, port)).await.ok()?.next()?;
        let start = Instant::now();
        let stream = TcpStream::connect(address).await.ok()?;
        let elapsed = start.elapsed();
        drop(stream);
        Some(elapsed)
    })
    .await
    .ok()
    .flatten()
}

pub fn format_bytes_kb(kb: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    let mut value = kb as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 || unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_uptime(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX: &str = "@os\nLinux\n@uptime\n93784.52\n@load\n0.52 0.58 0.59 1/467 12345\n@cpu\ncpu  4705 150 1120 16250 120 0 30 0 0 0\n@mem\nMemTotal:        8000000 kB\nMemFree:          500000 kB\nMemAvailable:    5000000 kB\nBuffers:          200000 kB\nCached:          2000000 kB\nSwapTotal:       2000000 kB\nSwapFree:        1500000 kB\n@disk\n/dev/vda1       51474044 20000000 31474044  39% /\ntmpfs             400000       0   400000   0% /run/user/1000\n/dev/vda15       106858    6000    100858   6% /boot/efi\n/dev/loop3        56960   56960         0 100% /snap/core/123\nserver:/export 1000000 250000 750000 25% /mnt/shared data\n@proc\n 812 12.5  3.1 node\n 977  3.2 10.0 postgres\n   1  0.0  0.1 systemd\n";

    #[test]
    fn parses_a_linux_report() {
        let s = parse(LINUX, None);
        assert_eq!(s.os, "Linux");
        assert_eq!(s.uptime_secs, Some(93784));
        assert_eq!(s.load, Some([0.52, 0.58, 0.59]));
        assert_eq!(s.mem_total_kb, Some(8_000_000));
        assert_eq!(s.mem_used_kb, Some(3_000_000));
        assert_eq!(s.mem_percent(), Some(37.5));
        assert_eq!(
            (s.swap_total_kb, s.swap_used_kb),
            (Some(2_000_000), Some(500_000))
        );
        assert_eq!(s.cpu_percent, None, "needs a previous sample");
        assert_eq!(s.processes.len(), 3);
        assert_eq!(
            s.processes[0],
            Process {
                pid: 812,
                cpu: 12.5,
                mem: 3.1,
                name: "node".into()
            }
        );
    }

    #[test]
    fn disk_list_skips_pseudo_snap_and_run_mounts_and_keeps_spaces_in_mounts() {
        let disks = parse(LINUX, None).disks;
        let mounts: Vec<_> = disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/mnt/shared data", "/boot/efi"]);
        assert_eq!(disks[0].percent().round(), 39.0);
    }

    /// Captured from a Mac: no /proc, long executable paths, many system volumes.
    const MACOS: &str = "@os\nDarwin\n@uptime\n@load\n 0:21  up 1 day,  8:08, 9 users, load averages: 7.02 5.52 4.68\n@cpu\n@mem\n@disk\n/dev/disk3s1s1   482746452  13339804  66140504    17%    /\ndevfs                  203       203         0   100%    /dev\n/dev/disk3s6     482746452   4195244  66140504     6%    /System/Volumes/VM\n/dev/disk3s5     482746452 385690952  66140504    86%    /System/Volumes/Data\nmap auto_home            0         0         0   100%    /System/Volumes/Data/home\n@proc\n 2253  95.4  0.1 /System/Library/PrivateFrameworks/StorageManagement.framework/PlugIns/ApplicationsStorageExtension.appex/Contents/MacOS/ApplicationsStorageExtension\n60282  33.5  0.6 postgres: syntra_bench syntra_enabled [local] idle\n26537  15.6  1.2 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome Helper (Renderer)\n";

    #[test]
    fn parses_a_macos_report_without_proc() {
        let s = parse(MACOS, None);
        assert_eq!(s.os, "Darwin");
        assert_eq!(s.load, Some([7.02, 5.52, 4.68]));
        assert_eq!(
            (s.uptime_secs, s.mem_total_kb, s.cpu_times),
            (None, None, None)
        );
        let mounts: Vec<_> = s.disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(
            mounts,
            ["/System/Volumes/Data", "/"],
            "same-size volumes list the fullest first"
        );
        let names: Vec<_> = s.processes.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "ApplicationsStorageExtension",
                "postgres: syntra_bench syntra_enabled [local] idle",
                "Google Chrome Helper (Renderer)"
            ]
        );
    }

    #[test]
    fn cpu_percent_comes_from_the_difference_between_polls() {
        let first = parse(LINUX, None).cpu_times.unwrap();
        // 1000 more ticks, of which 750 idle: 25% busy.
        let later = LINUX.replace(
            "4705 150 1120 16250 120 0 30 0",
            "4830 150 1245 17000 120 0 30 0",
        );
        assert_eq!(parse(&later, Some(first)).cpu_percent, Some(25.0));
    }

    #[test]
    fn cpu_counters_going_backwards_after_a_reboot_give_no_percentage() {
        let before = CpuTimes {
            total: 1_000_000,
            idle: 900_000,
        };
        assert_eq!(parse(LINUX, Some(before)).cpu_percent, None);
    }

    #[test]
    fn falls_back_to_free_buffers_and_cache_without_memavailable() {
        let output = LINUX.replace("MemAvailable:    5000000 kB\n", "");
        assert_eq!(
            parse(&output, None).mem_used_kb,
            Some(8_000_000 - 2_700_000)
        );
    }

    #[test]
    fn parses_uptime_style_load_on_systems_without_proc() {
        let output =
            "@os\nDarwin\n@load\n 9:41  up 3 days, 2:11, 2 users, load averages: 1.20 1.05 0.98\n";
        let s = parse(output, None);
        assert_eq!(s.os, "Darwin");
        assert_eq!(s.load, Some([1.20, 1.05, 0.98]));
        assert_eq!(s.mem_total_kb, None);
        let german = "@load\n 09:41:00 up 3 days, load average: 0,50, 0,40, 0,30\n";
        assert_eq!(parse(german, None).load, Some([0.5, 0.4, 0.3]));
        let dotted = "@load\n 09:41:00 up 3:11, 2 users,  load average: 0.52, 0.58, 0.59\n";
        assert_eq!(parse(dotted, None).load, Some([0.52, 0.58, 0.59]));
    }

    #[test]
    fn empty_and_garbled_output_yields_an_empty_snapshot() {
        assert_eq!(parse("", None), Snapshot::default());
        let s = parse(
            "@cpu\ncpu x y z\n@mem\nMemTotal: abc kB\n@disk\nnonsense\n@proc\nfoo bar\n",
            None,
        );
        assert_eq!(s, Snapshot::default());
    }

    #[test]
    fn script_survives_being_quoted_for_sh() {
        let quoted = shell_quote(SCRIPT);
        assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
        // Every single quote in the script must be escaped as '\'' .
        assert_eq!(
            quoted.matches(r"'\''").count(),
            SCRIPT.matches('\'').count()
        );
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn monitor_keeps_a_bounded_history_and_recovers_from_errors() {
        let mut monitor = Monitor::default();
        for i in 0..HISTORY + 10 {
            let snapshot = Snapshot {
                cpu_percent: Some(i as f32),
                ..Default::default()
            };
            monitor.apply(Ok(snapshot));
        }
        assert_eq!(monitor.cpu_history.len(), HISTORY);
        assert_eq!(monitor.cpu_history[0], 10.0);
        monitor.apply(Err("timeout".into()));
        assert_eq!(monitor.error.as_deref(), Some("timeout"));
        assert!(
            monitor.snapshot.is_some(),
            "the last good data stays visible"
        );
        monitor.apply(Ok(Snapshot::default()));
        assert_eq!(monitor.error, None);
    }

    #[test]
    fn formats_sizes_and_uptime() {
        assert_eq!(format_bytes_kb(512), "512 KB");
        assert_eq!(format_bytes_kb(8_000_000), "7.6 GB");
        assert_eq!(format_bytes_kb(51_474_044), "49.1 GB");
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(3 * 3600 + 120), "3h 2m");
        assert_eq!(format_uptime(93_784), "1d 2h");
    }

    #[tokio::test]
    async fn probe_gives_up_on_unroutable_addresses_within_its_deadline() {
        // 192.0.2.0/24 is reserved for documentation, so nothing answers.
        let start = Instant::now();
        assert!(probe("192.0.2.1", 22).await.is_none());
        assert!(start.elapsed() < PROBE_TIMEOUT + Duration::from_secs(1));
    }

    #[tokio::test]
    async fn probe_measures_open_ports_and_reports_closed_ones() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(probe("127.0.0.1", port).await.is_some());
        drop(listener);
        assert!(probe("127.0.0.1", port).await.is_none());
    }
}
