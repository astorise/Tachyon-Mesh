use std::io::{self, Read};

const EXCESSIVE_ALLOCATION_BYTES: usize = 100 * 1024 * 1024;

#[no_mangle]
pub extern "C" fn faas_entry() {
    let mut payload = String::new();
    let _ = io::stdin().read_to_string(&mut payload);

    match payload.trim().to_ascii_lowercase().as_str() {
        "allocate" | "oom" | "memory" => {
            allocate_excessive_memory();
        }
        "panic" | "unwind" => {
            panic!("intentional guest panic for chaos testing");
        }
        _ => {
            loop_forever();
        }
    }
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
