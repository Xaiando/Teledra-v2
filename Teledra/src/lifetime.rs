//! Ties every child process to the life of the court.
//!
//! The runtime's destructors kill the speech worker, the studios, and the
//! sidecars — but destructors only run on a graceful exit. Closing the console
//! window, a crash, or an external kill all skip them, and the Python children
//! are left running with no parent. Six of them were found orphaned after a
//! session ended, holding the microphone and the audio device against the next
//! launch, which is exactly what the migration notes promise cannot happen.
//!
//! A job object moves that guarantee into the kernel. Every process the court
//! starts inherits membership, and the job is configured to terminate its
//! members when its last handle closes — which happens when this process dies,
//! however it dies.

/// Kills every child when this process ends, by any means.
pub fn bind_children_to_this_process() {
    bind();
}

/// Closes an older court before this one opens.
///
/// Two courts running at once fight over the audio device, the microphone, and
/// the data directory, and the operator sees duplicate studio windows with no
/// way to tell which is live. Launching is the moment to make that
/// unambiguous, so any previous instance is closed first.
///
/// The recorded process is only terminated when its executable is the same one
/// running now, so a recycled process id can never take an unrelated program
/// down with it.
pub fn close_previous_instance(data_root: &std::path::Path) {
    let record = data_root.join("teledra.pid");
    if let Ok(raw) = std::fs::read_to_string(&record)
        && let Ok(previous) = raw.trim().parse::<u32>()
        && previous != std::process::id()
    {
        terminate_if_same_program(previous);
    }
    let _ = std::fs::create_dir_all(data_root);
    let _ = std::fs::write(&record, std::process::id().to_string());
}

#[cfg(windows)]
fn terminate_if_same_program(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess,
    };

    let Ok(mine) = std::env::current_exe() else {
        return;
    };

    // Safety: the handle is opened and closed here, and the buffer length is
    // passed by the same variable the call writes back into.
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            0,
            pid,
        );
        if handle.is_null() {
            return;
        }
        let mut buffer = [0u16; 32_768];
        let mut length = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) != 0;
        if named {
            let theirs = String::from_utf16_lossy(&buffer[..length as usize]);
            if std::path::Path::new(&theirs) == mine {
                TerminateProcess(handle, 0);
            }
        }
        CloseHandle(handle);
    }
}

#[cfg(not(windows))]
fn terminate_if_same_program(_pid: u32) {}

#[cfg(windows)]
fn bind() {
    use std::ptr;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // Safety: every call below operates on handles this function owns or on
    // the pseudo-handle for the current process, and the limit structure is
    // fully initialised before it is passed.
    unsafe {
        let job: HANDLE = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return;
        }

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let assigned = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0
            && AssignProcessToJobObject(job, GetCurrentProcess()) != 0;

        // The handle is deliberately never closed. It is released when the
        // process exits, and that release is what terminates the job's members.
        if !assigned {
            // An older Windows that refuses nested jobs leaves the destructors
            // as the only cleanup, which is the behaviour that existed before.
            windows_sys::Win32::Foundation::CloseHandle(job);
        }
    }
}

#[cfg(not(windows))]
fn bind() {}
