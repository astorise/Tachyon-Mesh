use std::io::{self, Read};

#[no_mangle]
pub extern "C" fn faas_entry() {
    let mut payload = String::new();

    match io::stdin().read_to_string(&mut payload) {
        Ok(_) if payload.is_empty() => println!("FaaS received an empty payload"),
        Ok(_) => println!("FaaS received: {payload}"),
        Err(error) => println!("FaaS failed to read request payload: {error}"),
    }
}
