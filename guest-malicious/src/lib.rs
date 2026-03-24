use std::io::{self, Read};

const EXCESSIVE_ALLOCATION_BYTES: usize = 100 * 1024 * 1024;

#[no_mangle]
pub extern "C" fn faas_entry() {
    let mut payload = String::new();
    let _ = io::stdin().read_to_string(&mut payload);

    if payload.trim().eq_ignore_ascii_case("allocate") {
        allocate_excessive_memory();
        return;
    }

    loop_forever();
}

fn loop_forever() -> ! {
    loop {
        std::hint::spin_loop();
    }
}

fn allocate_excessive_memory() {
    let mut buffer = Vec::with_capacity(EXCESSIVE_ALLOCATION_BYTES);
    buffer.resize(EXCESSIVE_ALLOCATION_BYTES, 0x41);

    println!("unexpectedly allocated {} bytes", buffer.len());
}
