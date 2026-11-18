//! Own a child process and its group until it exits or Forge stops it.
use std::{
    io,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::WindowsJob;
#[cfg(windows)]
use windows::WindowsJob as Group;

/// A non-PTY child. The owner polls it to enforce the deadline and reap it.
pub struct ManagedChild {
    child: Child,
    group: Arc<GroupSlot>,
    deadline: Instant,
}

type GroupSlot = Mutex<Option<Group>>;

#[derive(Default)]
struct Registry {
    quitting: bool,
    groups: Vec<Weak<GroupSlot>>,
}

static CHILDREN: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    CHILDREN.get_or_init(Mutex::default)
}

/// Stop helpers still running on background threads when the last Forge
/// window closes. New spawns are refused once app shutdown has begun.
pub fn stop_all_children() {
    let groups = {
        let mut children = registry().lock().unwrap_or_else(|error| error.into_inner());
        children.quitting = true;
        std::mem::take(&mut children.groups)
    };
    for group in groups.into_iter().filter_map(|group| group.upgrade()) {
        let _ = terminate_group(&mut group.lock().unwrap_or_else(|error| error.into_inner()));
    }
}

fn terminate_group(slot: &mut Option<Group>) -> io::Result<()> {
    let Some(group) = slot.take() else { return Ok(()) };
    group.terminate()
}

impl ManagedChild {
    pub fn spawn(command: &mut Command, timeout: Duration) -> io::Result<Self> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Child timeout is too large"))?;
        let mut children = registry().lock().unwrap_or_else(|error| error.into_inner());
        if children.quitting {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Forge is closing"));
        }
        let (child, group) = Group::spawn(command)?;
        let group = Arc::new(Mutex::new(Some(group)));
        children.groups.retain(|child| child.strong_count() > 0);
        children.groups.push(Arc::downgrade(&group));
        Ok(Self { child, group, deadline })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let mut slot = self.group.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(_group) = slot.as_ref() {
            #[cfg(unix)]
            let exited = _group.leader_exited()?;
            #[cfg(windows)]
            let exited = self.child.try_wait()?.is_some();
            if exited {
                // Descendants can hold the pipes open even after the leader exits.
                terminate_group(&mut slot)?;
            } else if Instant::now() >= self.deadline {
                terminate_group(&mut slot)?;
                return Err(io::Error::new(io::ErrorKind::TimedOut, "The child process timed out"));
            } else {
                return Ok(None);
            }
        }
        self.child.try_wait()
    }

    /// Signal the whole group. Polling or dropping this handle reaps the leader.
    pub fn stop(&mut self) -> io::Result<()> {
        terminate_group(&mut self.group.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
struct Group(libc::pid_t);

#[cfg(unix)]
impl Group {
    fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        #[cfg(target_os = "linux")]
        {
            let parent = std::process::id();
            // The closure runs after fork. Use only libc calls and static errors.
            unsafe {
                command.pre_exec(move || set_parent_death_signal(parent));
            }
        }
        let child = command.spawn()?;
        let pid = child.id() as libc::pid_t;
        Ok((child, Self(pid)))
    }

    fn leader_exited(&self) -> io::Result<bool> {
        // WNOWAIT keeps the leader's PID reserved until after group termination.
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        loop {
            // SAFETY: info points to writable storage for a siginfo_t.
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.0 as libc::id_t,
                    info.as_mut_ptr(),
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } == 0
            {
                // waitid sets si_pid when it observed a child.
                return Ok(unsafe { info.assume_init().si_pid() } != 0);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    fn terminate(&self) -> io::Result<()> {
        debug_assert!(self.0 > 1);
        // SAFETY: the group's leader is owned and still unreaped. The direct
        // signal also covers a program that moved itself to another group.
        let group_result = unsafe { libc::killpg(self.0, libc::SIGKILL) };
        let error = (group_result == -1).then(io::Error::last_os_error);
        unsafe { libc::kill(self.0, libc::SIGKILL) };
        match error {
            Some(error) if error.raw_os_error() != Some(libc::ESRCH) => Err(error),
            _ => Ok(()),
        }
    }
}

/// Set the Linux caller's parent-death signal and check the fork-to-start race.
#[cfg(target_os = "linux")]
pub fn set_parent_death_signal(parent: u32) -> io::Result<()> {
    use libc::c_ulong;
    // SAFETY: PR_SET_PDEATHSIG takes a signal number and no pointers.
    if unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as c_ulong, 0 as c_ulong, 0 as c_ulong, 0 as c_ulong)
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: getppid takes no arguments.
    if unsafe { libc::getppid() } != parent as libc::pid_t {
        return Err(io::Error::from_raw_os_error(libc::ECHILD));
    }
    Ok(())
}
