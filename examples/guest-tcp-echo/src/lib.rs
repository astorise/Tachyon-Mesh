#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "C" fn faas_entry() {}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn faas_entry() {
    let mut buffer = [0_u8; 1024];

    loop {
        let read = stdin_read(&mut buffer);
        if read == 0 {
            break;
        }

        stdout_write(&buffer[..read]);
    }
}

#[cfg(target_arch = "wasm32")]
#[repr(C)]
struct Iovec {
    buf: *mut u8,
    buf_len: usize,
}

#[cfg(target_arch = "wasm32")]
#[repr(C)]
struct Ciovec {
    buf: *const u8,
    buf_len: usize,
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn fd_read(fd: u32, iovs: *const Iovec, iovs_len: usize, nread: *mut usize) -> u16;
    fn fd_write(fd: u32, iovs: *const Ciovec, iovs_len: usize, nwritten: *mut usize) -> u16;
}

#[cfg(target_arch = "wasm32")]
fn stdin_read(buffer: &mut [u8]) -> usize {
    let mut read = 0_usize;
    let iovec = Iovec {
        buf: buffer.as_mut_ptr(),
        buf_len: buffer.len(),
    };
    let errno = unsafe { fd_read(0, &iovec, 1, &mut read) };
    assert_eq!(errno, 0, "fd_read should succeed");
    read
}

#[cfg(target_arch = "wasm32")]
fn stdout_write(buffer: &[u8]) {
    let mut written = 0_usize;
    let ciovec = Ciovec {
        buf: buffer.as_ptr(),
        buf_len: buffer.len(),
    };
    let errno = unsafe { fd_write(1, &ciovec, 1, &mut written) };
    assert_eq!(errno, 0, "fd_write should succeed");
    assert_eq!(
        written,
        buffer.len(),
        "fd_write should write the full chunk"
    );
}
