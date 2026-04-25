const LEGACY_PING_URL: &str = "http://mesh/legacy-service/ping";

#[no_mangle]
pub extern "C" fn faas_entry() {
    println!("MESH_FETCH:{LEGACY_PING_URL}");
}
