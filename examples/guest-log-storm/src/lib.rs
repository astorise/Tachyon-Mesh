const LOG_LINES: usize = 100_000;

#[no_mangle]
pub extern "C" fn faas_entry() {
    for index in 0..LOG_LINES {
        println!(
            r#"{{"level":"INFO","target":"guest-log-storm","fields":{{"message":"storm-{index}"}}}}"#
        );
    }

    println!("storm-complete");
}
