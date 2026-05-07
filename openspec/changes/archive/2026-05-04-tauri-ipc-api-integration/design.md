# Design: Tauri IPC Integration

## 1. Rust Backend Command (`tachyon-ui/src/main.rs`)
We expose a secure command to the frontend. This command takes the raw JSON intent, simulates (for now) the handoff to the `system-faas-config-api`, and returns a strongly typed result.

    use tauri::{command, State, AppHandle};
    use serde_json::Value;

    // The response struct sent back to Vanilla JS
    #[derive(serde::Serialize)]
    pub struct ApiResponse {
        pub success: bool,
        pub message: String,
    }

    #[command]
    pub async fn apply_configuration(
        domain: String, 
        payload: Value,
        _app: AppHandle,
    ) -> Result<ApiResponse, String> {
        // Log the reception at the Rust boundary
        println!("Received IPC Intent for Domain: {}", domain);
        
        // TODO: In the next iteration, this is where we instantiate 
        // the Wasmtime engine and call `validate-traffic-config` from the WIT file.
        // For this vertical slice, we validate that the payload is well-formed JSON.
        
        if domain == "config-routing" {
            // Basic mock validation: Ensure API version exists
            if let Some(api_version) = payload.get("api_version") {
                if api_version.as_str() == Some("routing.tachyon.io/v1alpha1") {
                    return Ok(ApiResponse {
                        success: true,
                        message: "Configuration dynamically validated by Rust Backend".into(),
                    });
                }
            }
            return Err("Invalid Schema: Missing or incorrect api_version".into());
        }

        Err(format!("Unknown configuration domain: {}", domain))
    }

    // Register the command in the Tauri builder
    fn main() {
        tauri::Builder::default()
            .invoke_handler(tauri::generate_handler![apply_configuration])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }

## 2. Vanilla JS Invocation (`src/controllers/routingController.ts`)
Update the frontend controller to actually call the Rust backend using Tauri's IPC bridge.

    export class RoutingController {
        static init() {
            const deployBtn = document.getElementById('btn-deploy-routing');
            if (!deployBtn) return;

            deployBtn.addEventListener('click', async () => {
                // ... (payload construction remains identical to previous design) ...
                const payload = {
                    api_version: "routing.tachyon.io/v1alpha1",
                    kind: "TrafficConfiguration",
                    // ...
                };

                try {
                    // Using the global Tauri object (assuming no bundler overhead)
                    const { invoke } = (window as any).__TAURI__.core;
                    
                    // Call the Rust command
                    const response = await invoke('apply_configuration', { 
                        domain: 'config-routing', 
                        payload: payload 
                    });

                    if (response.success) {
                        console.log("Backend Success:", response.message);
                        RoutingController.showToast("Deployed successfully to Mesh", "success");
                    }
                } catch (error) {
                    console.error("Backend Error:", error);
                    RoutingController.showToast(String(error), "error");
                }
            });
        }
        
        static showToast(msg: string, type: 'success' | 'error') {
            // Implementation of UI notification
            alert(msg); // Placeholder for GSAP toast
        }
    }