## Approach

Keep the existing bounded dense fallback unchanged. Add the native kernel path behind `nvfp4-cuda`, select it only after capability and memory checks, and emit the selected execution path through a small inference telemetry record. Hardware proof runs only on the labeled CUDA runner.
