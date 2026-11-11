//! Child processes that cannot outlive this process.

use std::os::windows::io::AsRawHandle;
use std::sync::OnceLock;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// One job per process, kept open until the process ends. Windows closes the
/// handle when we exit or crash, and the kill-on-close limit then ends every
/// child assigned to it.
fn job() -> crate::Result<isize> {
    static JOB: OnceLock<isize> = OnceLock::new();
    if let Some(&h) = JOB.get() {
        return Ok(h);
    }
    // SAFETY: plain job creation; the handle is intentionally never closed.
    let h = unsafe {
        let job = CreateJobObjectW(None, None)?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )?;
        job.0 as isize
    };
    Ok(*JOB.get_or_init(|| h))
}

/// Makes Windows kill `child` when this process ends, even by a crash.
pub fn kill_with_parent(child: &std::process::Child) -> crate::Result<()> {
    let job = HANDLE(job()? as *mut _);
    // SAFETY: both handles are valid for the call.
    unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle()))? };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigns_a_child() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "exit 0"])
            .spawn()
            .unwrap();
        kill_with_parent(&child).unwrap();
        child.wait().unwrap();
    }
}
