//! Minimal Windows Job Object bindings (no external crate needed).
//!
//! All child processes Vira spawns are assigned to one job object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. If Vira dies for any reason (crash,
//! forced termination, power loss), Windows terminates every process in the
//! job — so Vira never leaves orphaned Django/Node processes behind, and can
//! never terminate processes it did not spawn itself.
use std::sync::atomic::{AtomicUsize, Ordering};

pub static JOB_HANDLE: AtomicUsize = AtomicUsize::new(0);

const JOBOBJECTCLASS_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

#[repr(C)]
#[derive(Default)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

extern "system" {
    fn CreateJobObjectW(attr: *const u16, name: *const u16) -> usize;
    fn SetInformationJobObject(job: usize, class: i32, info: *const u8, len: u32) -> i32;
    fn AssignProcessToJobObject(job: usize, proc: usize) -> i32;
    fn TerminateJobObject(job: usize, exit_code: u32) -> i32;
}

/// Create the job object once at startup. Returns true on success.
pub fn init_vira_job() -> bool {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job == 0 {
            return false;
        }
        let info = JobObjectExtendedLimitInformation {
            basic_limit_information: JobObjectBasicLimitInformation {
                limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            io_info: IoCounters::default(),
            process_memory_limit: 0,
            job_memory_limit: 0,
            peak_process_memory_used: 0,
            peak_job_memory_used: 0,
        };
        let ok = SetInformationJobObject(
            job,
            JOBOBJECTCLASS_EXTENDED_LIMIT_INFORMATION,
            &info as *const _ as *const u8,
            std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        );
        if ok == 0 {
            return false;
        }
        JOB_HANDLE.store(job, Ordering::SeqCst);
        true
    }
}

/// Assign a spawned child to Vira's job object.
pub fn assign_to_job(process_handle: usize) -> bool {
    let job = JOB_HANDLE.load(Ordering::SeqCst);
    if job == 0 || process_handle == 0 {
        return false;
    }
    unsafe { AssignProcessToJobObject(job, process_handle) != 0 }
}

/// Terminate every process (and descendant) assigned to the job.
/// Only Vira-spawned processes are ever assigned, so unrelated Python/Node
/// processes on the machine are never touched.
pub fn terminate_job() {
    let job = JOB_HANDLE.load(Ordering::SeqCst);
    if job != 0 {
        unsafe {
            TerminateJobObject(job, 0);
        }
    }
}
