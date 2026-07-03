#[no_mangle]
pub extern "C" fn faas_entry() {
    println!("legacy guest stdout");
}
