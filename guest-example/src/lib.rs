#[no_mangle]
pub extern "C" fn faas_entry() {
    println!("Hello from WASM FaaS!");
}
