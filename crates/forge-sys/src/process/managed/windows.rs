//! Use a kill-on-close job and attach the child before it can spawn descendants.
use std::{
    ffi::c_void,
    io,
    mem::size_of,
    os::windows::{
        io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Child, Command},
    ptr::null,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
            OpenThread, ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP,
            CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
        },
    },
};

pub struct WindowsJob(OwnedHandle);

impl WindowsJob {
    fn new() -> io::Result<Self> {
        // SAFETY: no name or optional security attributes are passed.
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned an owned handle.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: handle is valid and info is initialized and of the requested type.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }

    pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        let job = Self::new()?;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        if let Err(error) = job.attach(child.as_raw_handle()).and_then(|()| resume_primary_thread(child.id())) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, job))
    }

    fn attach(&self, process: HANDLE) -> io::Result<()> {
        // SAFETY: both job and process handles are live.
        if unsafe { AssignProcessToJobObject(self.raw(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Adopt the child owned by a pseudo-terminal. None means it already exited.
    pub fn for_terminal(process: BorrowedHandle<'_>) -> io::Result<Option<Self>> {
        let handle = process.as_raw_handle();
        match Self::new().and_then(|job| {
            job.attach(handle)?;
            Ok(job)
        }) {
            Ok(job) => Ok(Some(job)),
            Err(error) => {
                // SAFETY: the caller borrows the live ConPTY process handle.
                if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                    return Ok(None);
                }
                // The child cannot safely be left running without a job.
                unsafe { TerminateProcess(handle, 1) };
                Err(error)
            }
        }
    }

    pub(super) fn terminate(&self) -> io::Result<()> {
        // SAFETY: our owned job handle remains open through the call.
        if unsafe { TerminateJobObject(self.raw(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// std::process exposes the process handle but not its initial thread handle.
fn resume_primary_thread(pid: u32) -> io::Result<()> {
    // SAFETY: TH32CS_SNAPTHREAD requests a snapshot of all threads.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the toolhelp snapshot handle is owned.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
    // SAFETY: the entry has the correct size and is writable.
    let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
    while found != 0 {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: we request a handle to the suspended child's sole thread.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: OpenThread returned an owned handle with resume access.
            let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
            if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            return Ok(());
        }
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        // SAFETY: same initialized buffer and open snapshot.
        found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
    }
    Err(io::Error::other("Could not find the suspended child's initial thread"))
}
