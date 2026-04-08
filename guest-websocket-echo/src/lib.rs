mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "websocket-faas-guest",
    });

    export!(Component);
}

struct Component;

impl bindings::Guest for Component {
    fn on_connect(connection: &bindings::tachyon::mesh::websocket::Connection) {
        while let Some(frame) = connection.receive() {
            match frame {
                bindings::tachyon::mesh::websocket::Frame::Close => break,
                other => {
                    if connection.send(&other).is_err() {
                        break;
                    }
                }
            }
        }
    }
}
