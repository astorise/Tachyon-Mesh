"""Export deterministic Qwen 3.5 MoE fixtures from a local HF model.

The script never downloads a model. It records configuration, selected
intermediate outputs, cache state shapes, and logits for parity tests.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prompt", default="Tachyon")
    args = parser.parse_args()
    if not args.model.is_dir():
        raise SystemExit(f"local model directory does not exist: {args.model}")

    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer

    torch.manual_seed(0)
    tokenizer = AutoTokenizer.from_pretrained(
        args.model, local_files_only=True, trust_remote_code=False
    )
    model = AutoModelForCausalLM.from_pretrained(
        args.model,
        local_files_only=True,
        trust_remote_code=False,
        torch_dtype=torch.float32,
        device_map="cpu",
    ).eval()
    encoded = tokenizer(args.prompt, return_tensors="pt")
    captures: dict[str, object] = {}
    hooks = []

    def capture(name: str):
        def hook(_module, _inputs, output):
            tensor = output[0] if isinstance(output, tuple) else output
            if isinstance(tensor, torch.Tensor):
                flat = tensor.detach().float().cpu().flatten()
                captures[name] = {
                    "shape": list(tensor.shape),
                    "values": flat[: min(flat.numel(), 4096)].tolist(),
                }

        return hook

    text_model = getattr(getattr(model, "model", model), "language_model", None)
    text_model = text_model or getattr(model, "model", model)
    for index, layer in enumerate(text_model.layers):
        hooks.append(layer.register_forward_hook(capture(f"layers.{index}")))

    with torch.inference_mode():
        output = model(**encoded, use_cache=True)
    for hook in hooks:
        hook.remove()

    args.output.mkdir(parents=True, exist_ok=True)
    config = json.loads((args.model / "config.json").read_text(encoding="utf-8"))
    fixture = {
        "prompt": args.prompt,
        "input_ids": encoded["input_ids"].tolist(),
        "config": config,
        "captures": captures,
        "logits": output.logits.detach().float().cpu()[0, -1, :4096].tolist(),
        "cache_shapes": [
            [list(t.shape) for t in layer]
            for layer in output.past_key_values
            if isinstance(layer, (tuple, list))
        ],
    }
    (args.output / "qwen35_moe_golden.json").write_text(
        json.dumps(fixture, indent=2), encoding="utf-8"
    )


if __name__ == "__main__":
    main()
