use legate_core::MemoryStatus;

/// Total physical memory and memory available for starting new processes, in bytes.
#[cfg(windows)]
pub fn memory_status() -> Option<MemoryStatus> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    // SAFETY: the structure contains only integers. Windows requires its size
    // in dwLength and fills the remaining fields when the call succeeds.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    Some(MemoryStatus {
        total: status.ullTotalPhys,
        available: status.ullAvailPhys,
    })
}

#[cfg(target_os = "linux")]
pub fn memory_status() -> Option<MemoryStatus> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let bytes = |field: &str| {
        let mut value = info
            .lines()
            .find_map(|line| line.strip_prefix(field))?
            .split_whitespace();
        let kib = value.next()?.parse::<u64>().ok()?;
        if value.next()? != "kB" {
            return None;
        }
        kib.checked_mul(1024)
    };
    Some(MemoryStatus {
        total: bytes("MemTotal:")?,
        available: bytes("MemAvailable:")?,
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn memory_status() -> Option<MemoryStatus> {
    None
}

#[cfg(all(test, any(windows, target_os = "linux")))]
#[test]
fn reads_available_physical_memory() {
    let status = memory_status().expect("physical memory status");
    assert!(status.total > 0);
    assert!(status.available <= status.total);
}
