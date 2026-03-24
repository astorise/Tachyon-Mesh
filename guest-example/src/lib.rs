use faas_sdk::faas_handler;
use std::io::{self, Read};

#[faas_handler]
#[no_mangle]
pub extern "C" fn faas_entry() {
    let mut payload = String::new();

    match io::stdin().read_to_string(&mut payload) {
        Ok(_) if payload.is_empty() => {
            tracing::info!("guest-example received an empty payload");
            println!("FaaS received an empty payload");
        }
        Ok(_) => {
            tracing::info!("guest-example received a request payload");
            println!("FaaS received: {payload}");
        }
        Err(error) => println!("FaaS failed to read request payload: {error}"),
    }
}
