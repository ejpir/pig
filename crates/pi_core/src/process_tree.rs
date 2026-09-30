//! Own the subprocess tree as well as the pi process itself.

/// Run an explicit argv command with bounded output/time and owned descendants.
/// Called from a background executor, never during rendering.
pub fn bounded_output(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
) -> anyhow::Result<std::process::Output> {
    use std::{io::Read, process::Stdio, thread, time::Instant};
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let tree = match ProcessTree::new(&child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
    };
    fn collect(mut stream: impl Read) -> Vec<u8> {
        let mut result = Vec::new();
        let mut block = [0; 8192];
        while let Ok(count) = stream.read(&mut block) {
            if count == 0 {
                break;
            }
            let keep = count.min((64 * 1024usize).saturating_sub(result.len()));
            result.extend_from_slice(&block[..keep]);
        }
        result
    }
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || collect(stdout));
    let err = thread::spawn(move || collect(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(anyhow::Error::from(error)),
            Ok(None) if started.elapsed() >= timeout => {
                break Err(anyhow::anyhow!(
                    "Command timed out; any created worktree/files are retained."
                ));
            }
            Ok(None) => thread::sleep(std::time::Duration::from_millis(20)),
        }
    };
    drop(tree);
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    Ok(std::process::Output {
        status: status?,
        stdout,
        stderr,
    })
}

#[cfg(unix)]
pub struct ProcessTree {
    pid: u32,
}

#[cfg(unix)]
impl ProcessTree {
    pub fn new(child: &std::process::Child) -> std::io::Result<Self> {
        Ok(Self { pid: child.id() })
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // spawn() creates a fresh process group, never the desktop's process group.
        if unsafe { libc::kill(-(self.pid as i32), libc::SIGKILL) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                eprintln!("Stopping RPC process group: {error}");
            }
        }
    }
}

#[cfg(windows)]
pub struct ProcessTree {
    _job: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl ProcessTree {
    pub fn new(child: &std::process::Child) -> std::io::Result<Self> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        // An unnamed, non-inheritable job keeps descendants from outliving their session.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { _job: job })
    }
}
