//! Runs a CLI attached to a Windows pseudo console (ConPTY), so its stdout is a terminal
//! rather than a pipe or `NUL`. Claude Code only acts interactively, and so only renews an
//! expired login from `claude auth status`, when `process.stdout.isTTY` is true (#52).
//!
//! The pseudo console is headless (no window). Everything it prints is read and dropped,
//! never stored or logged: it contains account details.

use std::ffi::c_void;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

/// How long to wait for the child to exit after it was killed.
const REAP_MS: u32 = 5_000;

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// Takes ownership of a raw handle so it is closed on every path.
///
/// # Safety
/// `h` must be a valid handle owned by the caller and not closed elsewhere.
unsafe fn owned(h: HANDLE) -> OwnedHandle {
    // SAFETY: guaranteed by the caller.
    unsafe { OwnedHandle::from_raw_handle(h.0) }
}

fn raw(h: &OwnedHandle) -> HANDLE {
    use std::os::windows::io::AsRawHandle;
    HANDLE(h.as_raw_handle())
}

/// Closes the pseudo console, which also ends any process still attached to it.
struct PseudoConsole(HPCON);

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreatePseudoConsole and is closed only here.
        unsafe { ClosePseudoConsole(self.0) };
    }
}

/// A process/thread attribute list holding the pseudo console attribute.
struct Attributes {
    buf: Vec<usize>,
}

impl Attributes {
    fn with_pseudo_console(pc: &PseudoConsole) -> Option<Self> {
        let mut size = 0usize;
        // SAFETY: a size query with no list; it fails with ERROR_INSUFFICIENT_BUFFER by design.
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) };
        if size == 0 {
            return None;
        }
        // usize elements keep the buffer pointer-aligned.
        let mut attrs = Attributes {
            buf: vec![0usize; size.div_ceil(size_of::<usize>())],
        };
        let list = attrs.list();
        // SAFETY: `list` points to at least `size` writable bytes.
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }.ok()?;
        // SAFETY: the list was initialized above; for this attribute the value is the HPCON
        // itself (not a pointer to it), and the console outlives the list's use.
        let set = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(pc.0.0 as *const c_void),
                size_of::<HPCON>(),
                None,
                None,
            )
        };
        if set.is_err() {
            // SAFETY: initialized above and deleted once, here.
            unsafe { DeleteProcThreadAttributeList(list) };
            attrs.buf.clear();
            return None;
        }
        Some(attrs)
    }

    fn list(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.buf.as_mut_ptr().cast())
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        if !self.buf.is_empty() {
            let list = self.list();
            // SAFETY: the list was initialized successfully and is deleted only here.
            unsafe { DeleteProcThreadAttributeList(list) };
        }
    }
}

/// Starts `application` with the full `command_line` attached to a new pseudo console, from
/// `cwd`, and waits up to `timeout_ms`. Returns the exit code, or `None` if it could not be
/// started or overran the timeout (it is then killed and reaped).
pub fn run_in_terminal(
    application: &Path,
    command_line: &str,
    cwd: &Path,
    timeout_ms: u32,
) -> Option<u32> {
    // Pipes: we keep the input writer (nothing is ever sent) and the output reader.
    let (mut in_read, mut in_write) = (HANDLE::default(), HANDLE::default());
    let (mut out_read, mut out_write) = (HANDLE::default(), HANDLE::default());
    // SAFETY: valid out-pointers; the handles are owned right after creation.
    unsafe { CreatePipe(&mut in_read, &mut in_write, None, 0) }.ok()?;
    let (in_read, _in_write) = unsafe { (owned(in_read), owned(in_write)) };
    unsafe { CreatePipe(&mut out_read, &mut out_write, None, 0) }.ok()?;
    let (out_read, out_write) = unsafe { (owned(out_read), owned(out_write)) };

    let size = COORD { X: 120, Y: 30 };
    // SAFETY: both handles are valid pipe ends; the console duplicates them.
    let pc = PseudoConsole(
        unsafe { CreatePseudoConsole(size, raw(&in_read), raw(&out_write), 0) }.ok()?,
    );
    // The console holds its own copies; ours must close so reads end when it does.
    drop((in_read, out_write));

    // Drain and drop everything the console prints, so it never blocks on a full pipe. The
    // thread ends when the console closes its end of the pipe.
    let mut output = std::fs::File::from(out_read);
    std::thread::spawn(move || {
        let mut sink = [0u8; 4096];
        while matches!(output.read(&mut sink), Ok(n) if n > 0) {}
    });

    let mut attrs = Attributes::with_pseudo_console(&pc)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    // Without this, a child of a process whose own stdio is redirected (e.g. under a test
    // runner) inherits those handles instead of the console's. Null handles make it attach to
    // the pseudo console's input and screen buffer.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attrs.list();

    let app = wide(application.as_os_str());
    let dir = wide(cwd.as_os_str());
    let mut line = wide(std::ffi::OsStr::new(command_line));
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: all strings are NUL-terminated and outlive the call; the command line buffer is
    // mutable as CreateProcessW requires; no handles are inherited.
    unsafe {
        CreateProcessW(
            PCWSTR(app.as_ptr()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            EXTENDED_STARTUPINFO_PRESENT,
            None,
            PCWSTR(dir.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )
    }
    .ok()?;
    // SAFETY: both handles were just returned by CreateProcessW and are owned here.
    let (process, _thread) = unsafe { (owned(info.hProcess), owned(info.hThread)) };

    // SAFETY: valid process handle for the whole block.
    let code = unsafe {
        if WaitForSingleObject(raw(&process), timeout_ms) == WAIT_OBJECT_0 {
            let mut code = 0u32;
            GetExitCodeProcess(raw(&process), &mut code)
                .ok()
                .map(|_| code)
        } else {
            let _ = TerminateProcess(raw(&process), 1);
            WaitForSingleObject(raw(&process), REAP_MS);
            None
        }
    };
    // Closing the console ends anything still attached (e.g. the CLI under a `.cmd` shim).
    drop(attrs);
    drop(pc);
    code
}
