"""Reference outputs for FilmCraft's Qwen3-VL vision encoder (AI-PLAN AI4 item 2).

Runs the published Hugging Face implementation (transformers' Qwen3-VL, Apache-2.0) from an
existing Python environment; installs nothing. Writes into OUT:
  image.rgb        256 x 256 x 3 uint8 test picture (a fixed pattern)
  pixel_values.f32 the reference processor's patches (if the processor can run here)
  tokens.f32       the vision tower's merged tokens for those patches (f32)
  deepstack.f32    its DeepStack taps, concatenated
  meta.json        shapes
Usage: python make_ref.py MODEL_DIR OUT
"""
import json, os, sys
import numpy as np
import torch
from safetensors import safe_open
from transformers import AutoConfig
from transformers.models.qwen3_vl.modeling_qwen3_vl import Qwen3VLVisionModel

model_dir, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
w = h = 256
y, x = np.mgrid[0:h, 0:w]
img = np.stack([(x * 255 // (w - 1)), (y * 255 // (h - 1)), ((x ^ y) & 255)], axis=-1).astype(np.uint8)
img.tofile(os.path.join(out, "image.rgb"))
meta = {"w": w, "h": h}

pv = None
try:
    from transformers import AutoImageProcessor
    proc = AutoImageProcessor.from_pretrained(model_dir)
    r = proc(images=[img], return_tensors="pt")
    pv, grid = r["pixel_values"].float(), r["image_grid_thw"]
    pv.numpy().astype(np.float32).tofile(os.path.join(out, "pixel_values.f32"))
    meta["processor"] = type(proc).__name__
    meta["grid"] = grid.tolist()
except Exception as e:  # no PIL / torchvision here: build the patches the documented way
    meta["processor_error"] = repr(e)[:300]

cfg = AutoConfig.from_pretrained(model_dir).vision_config
cfg._attn_implementation = "eager"
model = Qwen3VLVisionModel(cfg).float().eval()
state = {}
idx = json.load(open(os.path.join(model_dir, "model.safetensors.index.json")))["weight_map"]
for name, f in idx.items():
    if name.startswith("model.visual."):
        with safe_open(os.path.join(model_dir, f), "pt") as sf:
            state[name[len("model.visual."):]] = sf.get_tensor(name).float()
missing, unexpected = model.load_state_dict(state, strict=False)
meta["missing"], meta["unexpected"] = list(missing), list(unexpected)

if pv is None:
    sys.exit("processor unavailable: " + meta["processor_error"])
with torch.no_grad():
    o = model(pv, grid_thw=grid)
tokens = getattr(o, "pooler_output", None)
if tokens is None:
    tokens = o[1] if isinstance(o, tuple) else o.last_hidden_state
ds = getattr(o, "deepstack_features", None)
if ds is None and isinstance(o, tuple):
    ds = o[2] if len(o) > 2 else o[1]
tokens.float().numpy().tofile(os.path.join(out, "tokens.f32"))
torch.cat(list(ds), 0).float().numpy().tofile(os.path.join(out, "deepstack.f32"))
meta["tokens"], meta["deepstack"] = list(tokens.shape), [list(t.shape) for t in ds]
json.dump(meta, open(os.path.join(out, "meta.json"), "w"), indent=1)
print(json.dumps(meta))
