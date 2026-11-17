//! Resource limiter.
//!
//! On Windows the whole browser (this process and every Chromium child it
//! starts) lives in one Job Object, which lets the kernel enforce a hard
//! memory cap, a hard CPU rate cap, a network bandwidth cap and a scheduling
//! priority for all of them at once. Other platforms report telemetry only.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, System};

use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceLimits {
    /// Hard RAM cap for the whole browser, in MiB. `None` = unlimited.
    pub ram_limit_mb: Option<u64>,
    /// Hard CPU cap as a percentage of all cores (1-100). `None` = unlimited.
    pub cpu_limit_percent: Option<u8>,
    /// Bandwidth cap in kilobits per second. `None` = unlimited.
    pub network_limit_kbps: Option<u64>,
    /// Tighten everything automatically while a fullscreen game runs.
    pub game_mode: bool,
    pub game_ram_limit_mb: u64,
    pub game_cpu_limit_percent: u8,
    pub game_network_limit_kbps: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            ram_limit_mb: None,
            cpu_limit_percent: None,
            network_limit_kbps: None,
            game_mode: true,
            game_ram_limit_mb: 2048,
            game_cpu_limit_percent: 15,
            game_network_limit_kbps: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Normal,
    BelowNormal,
    Idle,
}

/// The concrete limits to apply to the Job Object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobPlan {
    pub memory_limit_bytes: Option<u64>,
    /// CPU rate in hundredths of a percent of all cores (1..=10000).
    pub cpu_rate: Option<u32>,
    pub max_bandwidth_bps: Option<u64>,
    pub priority: Priority,
}

/// Floor for the RAM cap: below this Chromium cannot keep a single tab alive.
pub const MIN_RAM_LIMIT_MB: u64 = 512;

fn tighter<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl ResourceLimits {
    /// Limits to enforce now. While a game is active and game mode is on, the
    /// tighter of the user's limit and the game-mode limit wins.
    pub fn plan(&self, game_active: bool) -> JobPlan {
        let gaming = game_active && self.game_mode;
        let ram = if gaming { tighter(self.ram_limit_mb, Some(self.game_ram_limit_mb)) } else { self.ram_limit_mb };
        let cpu = if gaming {
            tighter(self.cpu_limit_percent, Some(self.game_cpu_limit_percent))
        } else {
            self.cpu_limit_percent
        };
        let network = if gaming {
            tighter(self.network_limit_kbps, self.game_network_limit_kbps)
        } else {
            self.network_limit_kbps
        };
        JobPlan {
            memory_limit_bytes: ram.map(|mb| mb.max(MIN_RAM_LIMIT_MB) * 1024 * 1024),
            cpu_rate: cpu.filter(|percent| *percent < 100).map(|percent| u32::from(percent.clamp(1, 100)) * 100),
            max_bandwidth_bps: network.filter(|kbps| *kbps > 0).map(|kbps| kbps * 1000),
            priority: if gaming { Priority::Idle } else { Priority::Normal },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Telemetry {
    /// Resident memory of the browser and all its child processes, MiB.
    pub ram_mb: u64,
    /// CPU use of the browser as a percentage of all cores.
    pub cpu_percent: f32,
    pub process_count: usize,
    pub system_ram_total_mb: u64,
    pub system_ram_used_mb: u64,
    /// The RAM cap currently enforced, MiB.
    pub ram_limit_mb: Option<u64>,
    /// Whether limits are enforced by the OS on this platform.
    pub enforced: bool,
}

impl Telemetry {
    /// True when memory use is within 10% of the cap: time to hibernate tabs
    /// before the kernel starts refusing allocations.
    pub fn memory_critical(&self) -> bool {
        self.ram_limit_mb.is_some_and(|limit| self.ram_mb * 10 >= limit * 9)
    }
}

pub struct Governor {
    system: Mutex<System>,
    plan: Mutex<Option<JobPlan>>,
    #[cfg(windows)]
    job: Option<windows_job::Job>,
}

impl Governor {
    /// Creates the governor. On Windows this puts the current process in a
    /// new Job Object; call it before Chromium starts any child process so
    /// they all inherit it.
    pub fn new() -> Self {
        Self {
            system: Mutex::new(System::new()),
            plan: Mutex::new(None),
            #[cfg(windows)]
            job: match windows_job::Job::for_current_process() {
                Ok(job) => Some(job),
                Err(error) => {
                    log::warn!("resource limits unavailable: {error}");
                    None
                }
            },
        }
    }

    /// Whether limits are enforced by the operating system here.
    pub fn enforced(&self) -> bool {
        #[cfg(windows)]
        {
            self.job.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Applies limits. Returns the plan that is now in force.
    pub fn apply(&self, limits: &ResourceLimits, game_active: bool) -> Result<JobPlan> {
        let plan = limits.plan(game_active);
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.apply(&plan)?;
        }
        *self.plan.lock().unwrap_or_else(|p| p.into_inner()) = Some(plan.clone());
        Ok(plan)
    }

    /// Asks the OS to page out everything the browser is not actively using.
    /// Returns the number of processes trimmed.
    pub fn trim_memory(&self) -> usize {
        #[cfg(windows)]
        {
            let pids = self.process_tree();
            windows_job::trim_working_sets(&pids)
        }
        #[cfg(not(windows))]
        {
            0
        }
    }

    #[cfg(windows)]
    fn process_tree(&self) -> Vec<u32> {
        let mut system = self.system.lock().unwrap_or_else(|p| p.into_inner());
        system.refresh_processes();
        descendants(&system, std::process::id()).into_iter().map(|pid| pid.as_u32()).collect()
    }

    /// Current resource use of the browser's whole process tree.
    pub fn telemetry(&self) -> Telemetry {
        let mut system = self.system.lock().unwrap_or_else(|p| p.into_inner());
        system.refresh_memory();
        system.refresh_processes();
        let tree = descendants(&system, std::process::id());
        let (mut memory, mut cpu) = (0u64, 0f32);
        for pid in &tree {
            if let Some(process) = system.process(*pid) {
                memory += process.memory();
                cpu += process.cpu_usage();
            }
        }
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
        let plan = self.plan.lock().unwrap_or_else(|p| p.into_inner());
        Telemetry {
            ram_mb: memory / (1024 * 1024),
            cpu_percent: (cpu / cores).clamp(0.0, 100.0),
            process_count: tree.len(),
            system_ram_total_mb: system.total_memory() / (1024 * 1024),
            system_ram_used_mb: system.used_memory() / (1024 * 1024),
            ram_limit_mb: plan.as_ref().and_then(|plan| plan.memory_limit_bytes).map(|bytes| bytes / (1024 * 1024)),
            enforced: self.enforced(),
        }
    }
}

impl Default for Governor {
    fn default() -> Self {
        Self::new()
    }
}

/// `root` and every process descended from it, excluding sysinfo thread entries.
/// On Linux sysinfo lists threads as processes (with a thread_kind). Counting
/// them inflates both the process count and memory totals.
fn descendants(system: &System, root: u32) -> Vec<Pid> {
    let root = Pid::from_u32(root);
    let mut tree = vec![root];
    let mut index = 0;
    while index < tree.len() {
        let parent = tree[index];
        for (pid, process) in system.processes() {
            if process.thread_kind().is_some() {
                continue; // skip thread entries, real processes only
            }
            if process.parent() == Some(parent) && !tree.contains(pid) {
                tree.push(*pid);
            }
        }
        index += 1;
    }
    tree
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectCpuRateControlInformation,
        JobObjectExtendedLimitInformation, JobObjectNetRateControlInformation, SetInformationJobObject,
        JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOBOBJECT_NET_RATE_CONTROL_INFORMATION, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
        JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_PRIORITY_CLASS,
        JOB_OBJECT_NET_RATE_CONTROL_ENABLE, JOB_OBJECT_NET_RATE_CONTROL_MAX_BANDWIDTH,
    };
    use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, BELOW_NORMAL_PRIORITY_CLASS, IDLE_PRIORITY_CLASS,
        NORMAL_PRIORITY_CLASS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    };

    use super::{JobPlan, Priority};
    use crate::{Error, Result};

    fn os(error: windows::core::Error) -> Error {
        Error::Os(error.to_string())
    }

    pub struct Job(HANDLE);

    // The handle is only used with thread-safe kernel calls.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn for_current_process() -> Result<Self> {
            // SAFETY: plain Win32 calls; the handle is owned by the returned Job.
            unsafe {
                let job = CreateJobObjectW(None, PCWSTR::null()).map_err(os)?;
                if let Err(error) = AssignProcessToJobObject(job, GetCurrentProcess()) {
                    let _ = CloseHandle(job);
                    return Err(os(error));
                }
                Ok(Self(job))
            }
        }

        fn set<T>(&self, class: windows::Win32::System::JobObjects::JOBOBJECTINFOCLASS, info: &T) -> Result<()> {
            // SAFETY: `info` is a valid, fully initialised structure of the
            // size the information class expects.
            unsafe {
                SetInformationJobObject(self.0, class, info as *const T as *const c_void, size_of::<T>() as u32)
                    .map_err(os)
            }
        }

        pub fn apply(&self, plan: &JobPlan) -> Result<()> {
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PRIORITY_CLASS;
            limits.BasicLimitInformation.PriorityClass = match plan.priority {
                Priority::Normal => NORMAL_PRIORITY_CLASS.0,
                Priority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS.0,
                Priority::Idle => IDLE_PRIORITY_CLASS.0,
            };
            if let Some(bytes) = plan.memory_limit_bytes {
                limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                limits.JobMemoryLimit = bytes as usize;
            }
            self.set(JobObjectExtendedLimitInformation, &limits)?;

            let mut cpu = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
            if let Some(rate) = plan.cpu_rate {
                cpu.ControlFlags = JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
                cpu.Anonymous.CpuRate = rate;
            }
            self.set(JobObjectCpuRateControlInformation, &cpu)?;

            let mut network = JOBOBJECT_NET_RATE_CONTROL_INFORMATION::default();
            if let Some(bps) = plan.max_bandwidth_bps {
                network.ControlFlags = JOB_OBJECT_NET_RATE_CONTROL_ENABLE | JOB_OBJECT_NET_RATE_CONTROL_MAX_BANDWIDTH;
                // The API takes bytes per second.
                network.MaxBandwidth = bps / 8;
            }
            // Network rate control needs Windows 10 1607+; ignore failures on older systems.
            if let Err(error) = self.set(JobObjectNetRateControlInformation, &network) {
                log::debug!("network rate control unavailable: {error}");
            }
            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is owned by this Job and closed exactly once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub fn trim_working_sets(pids: &[u32]) -> usize {
        let mut trimmed = 0;
        for pid in pids {
            // SAFETY: the handle is checked and closed before leaving the loop body.
            unsafe {
                if let Ok(process) = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, false, *pid) {
                    if EmptyWorkingSet(process).is_ok() {
                        trimmed += 1;
                    }
                    let _ = CloseHandle(process);
                }
            }
        }
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_limits_means_no_caps() {
        let plan = ResourceLimits::default().plan(false);
        assert_eq!(
            plan,
            JobPlan { memory_limit_bytes: None, cpu_rate: None, max_bandwidth_bps: None, priority: Priority::Normal }
        );
    }

    #[test]
    fn user_limits_are_converted() {
        let limits = ResourceLimits {
            ram_limit_mb: Some(4096),
            cpu_limit_percent: Some(50),
            network_limit_kbps: Some(8000),
            ..Default::default()
        };
        let plan = limits.plan(false);
        assert_eq!(plan.memory_limit_bytes, Some(4096 * 1024 * 1024));
        assert_eq!(plan.cpu_rate, Some(5000));
        assert_eq!(plan.max_bandwidth_bps, Some(8_000_000));
    }

    #[test]
    fn game_mode_takes_the_tighter_limit_and_drops_priority() {
        let limits = ResourceLimits { ram_limit_mb: Some(1024), cpu_limit_percent: Some(60), ..Default::default() };
        let plan = limits.plan(true);
        assert_eq!(plan.memory_limit_bytes, Some(1024 * 1024 * 1024), "user's 1 GiB is tighter than game 2 GiB");
        assert_eq!(plan.cpu_rate, Some(1500), "game 15% is tighter than user 60%");
        assert_eq!(plan.priority, Priority::Idle);

        let off = ResourceLimits { game_mode: false, ..limits };
        assert_eq!(off.plan(true).priority, Priority::Normal);
        assert_eq!(off.plan(true).cpu_rate, Some(6000));
    }

    #[test]
    fn nonsense_values_are_clamped() {
        let limits = ResourceLimits {
            ram_limit_mb: Some(16),
            cpu_limit_percent: Some(0),
            network_limit_kbps: Some(0),
            ..Default::default()
        };
        let plan = limits.plan(false);
        assert_eq!(plan.memory_limit_bytes, Some(MIN_RAM_LIMIT_MB * 1024 * 1024));
        assert_eq!(plan.cpu_rate, Some(100), "0% would freeze the browser; floor at 1%");
        assert_eq!(plan.max_bandwidth_bps, None);
        let full = ResourceLimits { cpu_limit_percent: Some(100), ..Default::default() };
        assert_eq!(full.plan(false).cpu_rate, None, "100% is no cap");
    }

    #[test]
    fn telemetry_sees_this_process() {
        let governor = Governor::new();
        let telemetry = governor.telemetry();
        assert!(telemetry.process_count >= 1);
        assert!(telemetry.ram_mb >= 1);
        assert!(telemetry.system_ram_total_mb > telemetry.ram_mb);
        assert!(!telemetry.memory_critical());
        let critical = Telemetry { ram_mb: 950, ram_limit_mb: Some(1000), ..Default::default() };
        assert!(critical.memory_critical());
    }
}
