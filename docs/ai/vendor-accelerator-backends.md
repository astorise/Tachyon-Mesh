# Vendor accelerator backends

Tachyon supports optional out-of-process runners linked to vendor SDKs. The
default host remains free of OpenVINO and `libedgetpu` link-time dependencies.

## OpenVINO NPU

- Configure `TACHYON_OPENVINO_RUNNER`.
- The runner must initialize OpenVINO, discover an `NPU` device, and print
  `OPENVINO_NPU` for `--probe`.
- Supported model inputs are OpenVINO IR `.xml` files and compiled `.blob`
  artifacts. Supported operations are those accepted by the installed
  OpenVINO NPU plugin.

## Coral Edge TPU

- Configure `TACHYON_EDGETPU_RUNNER`.
- The runner must initialize `libedgetpu`, open a physical Coral device, and
  print `EDGE_TPU` for `--probe`.
- Models must be Edge-TPU-compiled `*_edgetpu.tflite` artifacts. Operation
  support is the subset accepted by the Edge TPU compiler/runtime.

## Runner contract

Both runners implement:

```text
runner --probe
runner --model <path> --input-hex <hex-bytes>
```

Inference output is written to stdout; diagnostics go to stderr. Probe failure,
missing device markers, unsupported formats, and non-zero inference exit codes
are hard failures.

The WIT load path resolves unavailable NPU/TPU requests to the CPU fallback
lane. Fallback succeeds only when the sealed alias is actually bound to a CPU
model; a model bound exclusively to unavailable NPU/TPU hardware remains
rejected, preventing silent relabeling.

Physical acceptance runs in
`.github/workflows/hardware-accelerator-acceptance.yml` on labeled self-hosted
runners. Missing SDK runners, devices, or acceptance models fail the job.
