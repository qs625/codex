use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::time::Duration;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessIdentity {
    pub pid: i32,
    pub start_identity: u64,
}

impl ProcessIdentity {
    pub fn observe(pid: i32) -> io::Result<Option<Self>> {
        Ok(process_record(pid)?.map(|record| record.identity))
    }

    pub fn is_alive(self) -> io::Result<bool> {
        Ok(Self::observe(self.pid)? == Some(self))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessGroupRecord {
    pub leader: ProcessIdentity,
    pub pgid: i32,
}

impl ProcessGroupRecord {
    pub fn observe(leader_pid: i32) -> io::Result<Option<Self>> {
        Ok(process_record(leader_pid)?.map(|record| Self {
            leader: record.identity,
            pgid: record.pgid,
        }))
    }

    pub fn require_dedicated(self) -> Result<Self, ProcessCleanupError> {
        if self.pgid != self.leader.pid {
            return Err(ProcessCleanupError::Blocked(format!(
                "process {} joined pgid {} instead of leading a dedicated process group",
                self.leader.pid, self.pgid
            )));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessRecord {
    pub identity: ProcessIdentity,
    pub parent_pid: i32,
    pub pgid: i32,
}

#[derive(Clone, Debug)]
pub struct TerminationTarget {
    pub root: ProcessIdentity,
    pub process_group: ProcessGroupRecord,
    pub guard: Option<ProcessIdentity>,
    pub known_descendants: BTreeSet<ProcessIdentity>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminationPolicy {
    pub term_timeout: Duration,
    pub kill_timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for TerminationPolicy {
    fn default() -> Self {
        Self {
            term_timeout: Duration::from_secs(5),
            kill_timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(25),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CooperativeCleanupEvidence {
    pub term_was_sent: bool,
    pub kill_was_sent: bool,
    pub observed_exited: BTreeSet<ProcessIdentity>,
}

#[derive(Debug)]
pub enum ProcessCleanupError {
    Io(io::Error),
    Blocked(String),
    TimedOut(String),
    Unsupported(String),
}

impl fmt::Display for ProcessCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Blocked(message) => write!(formatter, "process cleanup blocked: {message}"),
            Self::TimedOut(message) => write!(formatter, "process cleanup timed out: {message}"),
            Self::Unsupported(message) => {
                write!(formatter, "cooperative process cleanup is unsupported: {message}")
            }
        }
    }
}

impl std::error::Error for ProcessCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Blocked(_) | Self::TimedOut(_) | Self::Unsupported(_) => None,
        }
    }
}

impl From<io::Error> for ProcessCleanupError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn current_process_identity() -> io::Result<ProcessIdentity> {
    let pid = unsafe { libc::getpid() };
    ProcessIdentity::observe(pid)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("current process {pid} disappeared while reading its identity"),
        )
    })
}

pub fn current_parent_identity() -> io::Result<ProcessIdentity> {
    let pid = unsafe { libc::getppid() };
    ProcessIdentity::observe(pid)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent process {pid} disappeared while reading its identity"),
        )
    })
}

pub fn snapshot_processes() -> Result<Vec<ProcessRecord>, ProcessCleanupError> {
    platform::snapshot_processes()
}

pub fn descendants_of(
    root: ProcessIdentity,
    processes: &[ProcessRecord],
) -> BTreeSet<ProcessIdentity> {
    let by_parent = processes.iter().fold(
        BTreeMap::<i32, Vec<&ProcessRecord>>::new(),
        |mut index, process| {
            index
                .entry(process.parent_pid)
                .or_default()
                .push(process);
            index
        },
    );
    let mut pending = vec![root.pid];
    let mut descendants = BTreeSet::new();
    while let Some(parent) = pending.pop() {
        let Some(children) = by_parent.get(&parent) else {
            continue;
        };
        for child in children {
            if descendants.insert(child.identity) {
                pending.push(child.identity.pid);
            }
        }
    }
    descendants
}

pub fn terminate_and_observe_empty(
    target: &TerminationTarget,
    policy: TerminationPolicy,
) -> Result<CooperativeCleanupEvidence, ProcessCleanupError> {
    target.process_group.require_dedicated()?;
    if target.process_group.leader != target.root {
        return Err(ProcessCleanupError::Blocked(
            "registered process-group leader does not match the cleanup root identity".to_string(),
        ));
    }

    let before = snapshot_processes()?;
    if !target.root.is_alive()? {
        if before
            .iter()
            .any(|process| process.pgid == target.process_group.pgid)
        {
            return Err(ProcessCleanupError::Blocked(format!(
                "registered root identity is gone while pgid {} is still occupied; refusing a potentially reused process group",
                target.process_group.pgid
            )));
        }
        if let Some(guard) = target.guard {
            terminate_identity_and_observe_empty(guard, policy)?;
        }
        let survivors = live_identities(&target.known_descendants)?;
        if !survivors.is_empty() {
            return Err(ProcessCleanupError::Blocked(format!(
                "registered root identity is gone but known descendants remain alive: {survivors:?}"
            )));
        }
        let mut observed_exited = target.known_descendants.clone();
        observed_exited.insert(target.root);
        if let Some(guard) = target.guard {
            observed_exited.insert(guard);
        }
        return Ok(CooperativeCleanupEvidence {
            term_was_sent: false,
            kill_was_sent: false,
            observed_exited,
        });
    }
    let mut tracked = target.known_descendants.clone();
    tracked.insert(target.root);
    tracked.extend(descendants_of(target.root, &before));
    if let Some(guard) = target.guard {
        tracked.insert(guard);
        tracked.extend(descendants_of(guard, &before));
    }
    require_cooperative_group(target, &tracked, &before)?;

    let mut term_was_sent = signal_group(target.process_group.pgid, libc::SIGTERM)?;
    if let Some(guard) = target.guard
        && guard != target.root
        && guard.is_alive()?
    {
        term_was_sent |= signal_identity(guard, libc::SIGTERM)?;
    }
    if wait_for_exit(
        target,
        &mut tracked,
        policy.term_timeout,
        policy.poll_interval,
    )? {
        return Ok(CooperativeCleanupEvidence {
            term_was_sent,
            kill_was_sent: false,
            observed_exited: tracked,
        });
    }

    let mut kill_was_sent = signal_group(target.process_group.pgid, libc::SIGKILL)?;
    if let Some(guard) = target.guard
        && guard != target.root
        && guard.is_alive()?
    {
        kill_was_sent |= signal_identity(guard, libc::SIGKILL)?;
    }
    if wait_for_exit(
        target,
        &mut tracked,
        policy.kill_timeout,
        policy.poll_interval,
    )? {
        return Ok(CooperativeCleanupEvidence {
            term_was_sent,
            kill_was_sent,
            observed_exited: tracked,
        });
    }

    let survivors = live_identities(&tracked)?;
    Err(ProcessCleanupError::TimedOut(format!(
        "identities still alive after SIGKILL: {survivors:?}"
    )))
}

pub fn terminate_identity_and_observe_empty(
    identity: ProcessIdentity,
    policy: TerminationPolicy,
) -> Result<CooperativeCleanupEvidence, ProcessCleanupError> {
    let term_was_sent = signal_identity(identity, libc::SIGTERM)?;
    let term_deadline = Instant::now() + policy.term_timeout;
    while identity.is_alive()? && Instant::now() < term_deadline {
        std::thread::sleep(
            policy
                .poll_interval
                .min(term_deadline.saturating_duration_since(Instant::now())),
        );
    }
    if !identity.is_alive()? {
        return Ok(CooperativeCleanupEvidence {
            term_was_sent,
            kill_was_sent: false,
            observed_exited: BTreeSet::from([identity]),
        });
    }
    let kill_was_sent = signal_identity(identity, libc::SIGKILL)?;
    let kill_deadline = Instant::now() + policy.kill_timeout;
    while identity.is_alive()? && Instant::now() < kill_deadline {
        std::thread::sleep(
            policy
                .poll_interval
                .min(kill_deadline.saturating_duration_since(Instant::now())),
        );
    }
    if identity.is_alive()? {
        return Err(ProcessCleanupError::TimedOut(format!(
            "identity {}/{} is still alive after SIGKILL",
            identity.pid, identity.start_identity
        )));
    }
    Ok(CooperativeCleanupEvidence {
        term_was_sent,
        kill_was_sent,
        observed_exited: BTreeSet::from([identity]),
    })
}

pub fn terminate_identity_tree_and_observe_empty(
    root: ProcessIdentity,
    policy: TerminationPolicy,
) -> Result<CooperativeCleanupEvidence, ProcessCleanupError> {
    if !root.is_alive()? {
        return Ok(CooperativeCleanupEvidence {
            term_was_sent: false,
            kill_was_sent: false,
            observed_exited: BTreeSet::from([root]),
        });
    }
    signal_identity(root, libc::SIGSTOP)?;
    let mut tracked = BTreeSet::from([root]);
    let freeze_deadline = Instant::now() + policy.term_timeout;
    loop {
        if Instant::now() >= freeze_deadline {
            return Err(ProcessCleanupError::TimedOut(format!(
                "guard tree rooted at {}/{} did not converge while freezing descendants",
                root.pid, root.start_identity
            )));
        }
        let snapshot = snapshot_processes()?;
        let discovered = descendants_of(root, &snapshot)
            .difference(&tracked)
            .copied()
            .collect::<Vec<_>>();
        if discovered.is_empty() {
            break;
        }
        for identity in discovered {
            signal_identity(identity, libc::SIGSTOP)?;
            tracked.insert(identity);
        }
    }
    let mut kill_was_sent = false;
    for identity in tracked.iter().rev() {
        kill_was_sent |= signal_identity(*identity, libc::SIGKILL)?;
    }
    let deadline = Instant::now() + policy.kill_timeout;
    loop {
        let survivors = live_identities(&tracked)?;
        if survivors.is_empty() {
            return Ok(CooperativeCleanupEvidence {
                term_was_sent: false,
                kill_was_sent,
                observed_exited: tracked,
            });
        }
        if Instant::now() >= deadline {
            return Err(ProcessCleanupError::TimedOut(format!(
                "guard tree identities still alive after SIGKILL: {survivors:?}"
            )));
        }
        std::thread::sleep(
            policy
                .poll_interval
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn process_record(pid: i32) -> io::Result<Option<ProcessRecord>> {
    platform::process_record(pid)
}

fn require_cooperative_group(
    target: &TerminationTarget,
    tracked: &BTreeSet<ProcessIdentity>,
    processes: &[ProcessRecord],
) -> Result<(), ProcessCleanupError> {
    let records = processes
        .iter()
        .map(|record| (record.identity, record))
        .collect::<BTreeMap<_, _>>();
    for record in processes
        .iter()
        .filter(|record| record.pgid == target.process_group.pgid)
    {
        if !tracked.contains(&record.identity) {
            return Err(ProcessCleanupError::Blocked(format!(
                "pgid {} contains an untracked process {}/{}",
                target.process_group.pgid,
                record.identity.pid,
                record.identity.start_identity
            )));
        }
    }
    // macOS Electron can hand off the app's main process into a separate
    // process group. Every such process is already in `tracked`, keyed by a
    // PID plus start identity, and cleanup signals tracked identities directly
    // after terminating the original group. An untracked group member still
    // blocks cleanup above, because its ownership is ambiguous.
    let _ = records;
    Ok(())
}

fn wait_for_exit(
    target: &TerminationTarget,
    tracked: &mut BTreeSet<ProcessIdentity>,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<bool, ProcessCleanupError> {
    let deadline = Instant::now() + timeout;
    loop {
        let processes = snapshot_processes()?;
        if target.root.is_alive()? {
            tracked.extend(descendants_of(target.root, &processes));
        }
        if let Some(guard) = target.guard
            && guard.is_alive()?
        {
            tracked.extend(descendants_of(guard, &processes));
        }
        require_cooperative_group(target, tracked, &processes)?;
        let group_alive = processes
            .iter()
            .any(|process| process.pgid == target.process_group.pgid);
        if !group_alive && live_identities(tracked)?.is_empty() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(poll_interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn live_identities(
    identities: &BTreeSet<ProcessIdentity>,
) -> Result<BTreeSet<ProcessIdentity>, ProcessCleanupError> {
    let mut live = BTreeSet::new();
    for identity in identities {
        if identity.is_alive()? {
            live.insert(*identity);
        }
    }
    Ok(live)
}

fn signal_group(pgid: i32, signal: i32) -> io::Result<bool> {
    if pgid <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid process group id {pgid}"),
        ));
    }
    let result = unsafe { libc::killpg(pgid, signal) };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

fn signal_identity(identity: ProcessIdentity, signal: i32) -> io::Result<bool> {
    if !identity.is_alive()? {
        return Ok(false);
    }
    let result = unsafe { libc::kill(identity.pid, signal) };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::fs;

    pub(super) fn process_record(pid: i32) -> io::Result<Option<ProcessRecord>> {
        let path = format!("/proc/{pid}/stat");
        let stat = match fs::read_to_string(&path) {
            Ok(stat) => stat,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
            Err(error) => return Err(error),
        };
        parse_proc_stat(&stat).map(Some)
    }

    pub(super) fn snapshot_processes() -> Result<Vec<ProcessRecord>, ProcessCleanupError> {
        let mut records = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
            else {
                continue;
            };
            match process_record(pid) {
                Ok(Some(record)) => records.push(record),
                Ok(None) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(records)
    }

    fn parse_proc_stat(stat: &str) -> io::Result<ProcessRecord> {
        let open = stat.find('(').ok_or_else(invalid_stat)?;
        let close = stat.rfind(')').ok_or_else(invalid_stat)?;
        if close <= open {
            return Err(invalid_stat());
        }
        let pid = stat[..open]
            .trim()
            .parse::<i32>()
            .map_err(|_| invalid_stat())?;
        let fields = stat[close + 1..].split_whitespace().collect::<Vec<_>>();
        if fields.len() <= 19 {
            return Err(invalid_stat());
        }
        let parent_pid = fields[1].parse::<i32>().map_err(|_| invalid_stat())?;
        let pgid = fields[2].parse::<i32>().map_err(|_| invalid_stat())?;
        let start_identity = fields[19].parse::<u64>().map_err(|_| invalid_stat())?;
        Ok(ProcessRecord {
            identity: ProcessIdentity {
                pid,
                start_identity,
            },
            parent_pid,
            pgid,
        })
    }

    fn invalid_stat() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "invalid /proc process stat")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_comm_with_spaces_and_parentheses() {
            let stat = "42 (a tricky) name) S 7 42 42 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 9001 0";
            let record = parse_proc_stat(stat).expect("parse");
            assert_eq!(record.identity.pid, 42);
            assert_eq!(record.parent_pid, 7);
            assert_eq!(record.pgid, 42);
            assert_eq!(record.identity.start_identity, 9001);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::mem;

    const PROC_ALL_PIDS: u32 = 1;
    const PROC_PIDTBSDINFO: i32 = 3;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct ProcBsdInfo {
        pbi_flags: u32,
        pbi_status: u32,
        pbi_xstatus: u32,
        pbi_pid: u32,
        pbi_ppid: u32,
        pbi_uid: libc::uid_t,
        pbi_gid: libc::gid_t,
        pbi_ruid: libc::uid_t,
        pbi_rgid: libc::gid_t,
        pbi_svuid: libc::uid_t,
        pbi_svgid: libc::gid_t,
        rfu_1: u32,
        pbi_comm: [u8; 16],
        pbi_name: [u8; 32],
        pbi_nfiles: u32,
        pbi_pgid: u32,
        pbi_pjobc: u32,
        e_tdev: u32,
        e_tpgid: u32,
        pbi_nice: i32,
        pbi_start_tvsec: u64,
        pbi_start_tvusec: u64,
    }

    unsafe extern "C" {
        fn proc_listpids(
            process_type: u32,
            type_info: u32,
            buffer: *mut libc::c_void,
            buffersize: i32,
        ) -> i32;
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: i32,
        ) -> i32;
    }

    pub(super) fn process_record(pid: i32) -> io::Result<Option<ProcessRecord>> {
        let mut info = ProcBsdInfo::default();
        let expected = mem::size_of::<ProcBsdInfo>() as i32;
        let read = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut ProcBsdInfo).cast(),
                expected,
            )
        };
        if read == 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::ESRCH || code == libc::ENOENT
            ) {
                return Ok(None);
            }
            return Err(error);
        }
        if read != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("proc_pidinfo returned {read} bytes, expected {expected}"),
            ));
        }
        Ok(Some(ProcessRecord {
            identity: ProcessIdentity {
                pid: info.pbi_pid as i32,
                start_identity: info
                    .pbi_start_tvsec
                    .saturating_mul(1_000_000)
                    .saturating_add(info.pbi_start_tvusec),
            },
            parent_pid: info.pbi_ppid as i32,
            pgid: info.pbi_pgid as i32,
        }))
    }

    pub(super) fn snapshot_processes() -> Result<Vec<ProcessRecord>, ProcessCleanupError> {
        let required = unsafe { proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
        if required <= 0 {
            return Err(io::Error::last_os_error().into());
        }
        let capacity = required as usize / mem::size_of::<i32>() + 64;
        let mut pids = vec![0_i32; capacity];
        let bytes = unsafe {
            proc_listpids(
                PROC_ALL_PIDS,
                0,
                pids.as_mut_ptr().cast(),
                (pids.len() * mem::size_of::<i32>()) as i32,
            )
        };
        if bytes < 0 {
            return Err(io::Error::last_os_error().into());
        }
        pids.truncate(bytes as usize / mem::size_of::<i32>());
        let mut records = Vec::new();
        for pid in pids.into_iter().filter(|pid| *pid > 0) {
            match process_record(pid) {
                Ok(Some(record)) => records.push(record),
                Ok(None) => {}
                Err(error)
                    if matches!(
                        error.raw_os_error(),
                        Some(code)
                            if code == libc::ESRCH
                                || code == libc::ENOENT
                                || code == libc::EPERM
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(records)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use super::*;

    pub(super) fn process_record(_pid: i32) -> io::Result<Option<ProcessRecord>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process identity is only implemented on Linux and macOS",
        ))
    }

    pub(super) fn snapshot_processes() -> Result<Vec<ProcessRecord>, ProcessCleanupError> {
        Err(ProcessCleanupError::Unsupported(
            "process snapshots are only implemented on Linux and macOS".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: i32, parent_pid: i32) -> ProcessRecord {
        ProcessRecord {
            identity: ProcessIdentity {
                pid,
                start_identity: pid as u64 * 10,
            },
            parent_pid,
            pgid: 1,
        }
    }

    #[test]
    fn descendant_walk_is_transitive_and_excludes_unrelated_processes() {
        let root = record(10, 1).identity;
        let processes = vec![
            record(10, 1),
            record(11, 10),
            record(12, 11),
            record(20, 1),
        ];
        assert_eq!(
            descendants_of(root, &processes),
            BTreeSet::from([record(11, 10).identity, record(12, 11).identity])
        );
    }

    #[test]
    fn cooperative_cleanup_allows_an_observed_process_group_handoff() {
        let root = ProcessIdentity {
            pid: 10,
            start_identity: 100,
        };
        let escaped = ProcessIdentity {
            pid: 11,
            start_identity: 110,
        };
        let target = TerminationTarget {
            root,
            process_group: ProcessGroupRecord {
                leader: root,
                pgid: 10,
            },
            guard: None,
            known_descendants: BTreeSet::from([escaped]),
        };
        let processes = vec![
            ProcessRecord {
                identity: root,
                parent_pid: 1,
                pgid: 10,
            },
            ProcessRecord {
                identity: escaped,
                parent_pid: 10,
                pgid: 11,
            },
        ];

        require_cooperative_group(
            &target,
            &BTreeSet::from([root, escaped]),
            &processes,
        )
        .expect("an observed handoff remains safely trackable by identity");
    }
}
