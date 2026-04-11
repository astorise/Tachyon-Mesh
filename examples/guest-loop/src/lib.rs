const SELF_ROUTE: &str = "/api/guest-loop";

#[no_mangle]
pub extern "C" fn faas_entry() {
    println!("MESH_FETCH:{SELF_ROUTE}");
}
