const LEGACY_PING_URL: &str = "http://legacy-service:8081/ping";

#[no_mangle]
pub extern "C" fn faas_entry() {
    println!("MESH_FETCH:{LEGACY_PING_URL}");
}
