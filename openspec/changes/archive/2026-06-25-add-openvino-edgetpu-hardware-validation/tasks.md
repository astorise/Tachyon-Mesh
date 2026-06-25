- [x] Integrate the OpenVINO SDK and initialize a real NPU device.
- [x] Integrate `libedgetpu` and initialize a Coral USB TPU.
- [x] Wire fallback selection through the live WIT-facing load and compute path.
- [x] Add SDK-optional unit and integration tests.
- [ ] Run and record CPU+GPU+NPU hardware validation.
- [ ] Run and record Coral USB TPU validation.
- [x] Document supported vendors, formats, op sets, and fallback behavior.

## Physical acceptance status

The implementation and fail-closed acceptance workflow are complete. Physical
execution evidence is not available as of 2026-06-25 because the repository has
no registered self-hosted runners labeled `tachyon-openvino-npu` or
`tachyon-coral-edgetpu`, and the local host has neither vendor runner, model,
SDK tool, nor detected device. The two acceptance tasks intentionally remain
unchecked; no hardware result has been simulated or inferred.
