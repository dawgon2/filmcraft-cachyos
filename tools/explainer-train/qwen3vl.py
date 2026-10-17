"""Qwen3-VL's text model for the picture-model tune (fork AI-PLAN AI4 item 3), with LoRA adapters.

Offline training tooling only; never part of the product. Needs only `torch` and `safetensors`
(like `tinylm.py`, whose LoRA, RMSNorm and MLP it reuses). The picture side is not here: FilmCraft's
own encoder already turned every frame into picture rows and DeepStack taps
(`crates/engine/examples/vision_traindump.rs`, via `VisionLanguage::training_inputs`, the same
prompt code the app's `ask` uses), so this file only has to match FilmCraft's text forward
(`crates/llm/src/forward.rs` `forward_multimodal`):

- pre-norm decoder, grouped-query attention with a per-head RMSNorm on q and k before the rotary
  embedding, SwiGLU MLP, tied output head;
- multimodal rotary positions (M-RoPE, interleaved): pair j of a head takes its position from the
  h axis when j % 3 == 1 and j < 3 * sections[1], from w when j % 3 == 2 and j < 3 * sections[2],
  else from t (`crates/llm/src/mrope.rs` `angles`); rotate-half layout;
- picture rows replace the token embeddings at the picture positions; after decoder layer l <
  number of taps, tap l is added to the hidden states at those positions (DeepStack).

Weights are read from the release's sharded safetensors under `model.language_model.*`, and the
merged tune is written back under the same names, so `filmcraft_llm::convert` turns it into the
app's GGUF exactly as it does the base.
"""

import json
from pathlib import Path

import torch
import torch.utils.checkpoint
import torch.nn as nn
import torch.nn.functional as F
from safetensors import safe_open

from tinylm import LORA_TARGETS, LoRALinear, MLP, RMSNorm, rotate_half

PREFIX = "model.language_model."


class Config:
    def __init__(self, d: dict):
        t = d.get("text_config", d)
        self.hidden = int(t["hidden_size"])
        self.inter = int(t["intermediate_size"])
        self.layers = int(t["num_hidden_layers"])
        self.heads = int(t["num_attention_heads"])
        self.kv_heads = int(t.get("num_key_value_heads", self.heads))
        self.head_dim = int(t.get("head_dim", self.hidden // self.heads))
        self.eps = float(t.get("rms_norm_eps", 1e-6))
        self.theta = float(t.get("rope_theta", 5_000_000))
        self.vocab = int(t["vocab_size"])
        rs = t.get("rope_scaling") or {}
        self.sections = [int(x) for x in rs.get("mrope_section", [24, 20, 20])]
        if not rs.get("mrope_interleaved", True):
            raise ValueError("only interleaved M-RoPE is supported")
        if sum(self.sections) != self.head_dim // 2 or self.heads % self.kv_heads:
            raise ValueError("bad head / rotary layout")
        if not d.get("tie_word_embeddings", t.get("tie_word_embeddings", True)):
            raise ValueError("expected a tied output head")


class Attention(nn.Module):
    def __init__(self, c: Config):
        super().__init__()
        self.c = c
        self.q_proj = nn.Linear(c.hidden, c.heads * c.head_dim, bias=False)
        self.k_proj = nn.Linear(c.hidden, c.kv_heads * c.head_dim, bias=False)
        self.v_proj = nn.Linear(c.hidden, c.kv_heads * c.head_dim, bias=False)
        self.o_proj = nn.Linear(c.heads * c.head_dim, c.hidden, bias=False)
        self.q_norm = RMSNorm(c.head_dim, c.eps)
        self.k_norm = RMSNorm(c.head_dim, c.eps)

    def forward(self, x, cos, sin):
        b, t, _ = x.shape
        c = self.c
        q = self.q_norm(self.q_proj(x).view(b, t, c.heads, c.head_dim)).transpose(1, 2)
        k = self.k_norm(self.k_proj(x).view(b, t, c.kv_heads, c.head_dim)).transpose(1, 2)
        v = self.v_proj(x).view(b, t, c.kv_heads, c.head_dim).transpose(1, 2)
        q = q * cos + rotate_half(q) * sin
        k = k * cos + rotate_half(k) * sin
        rep = c.heads // c.kv_heads
        if rep > 1:
            k = k.repeat_interleave(rep, dim=1)
            v = v.repeat_interleave(rep, dim=1)
        y = F.scaled_dot_product_attention(q, k, v, is_causal=True)
        return self.o_proj(y.transpose(1, 2).reshape(b, t, c.heads * c.head_dim))


class Layer(nn.Module):
    def __init__(self, c: Config):
        super().__init__()
        self.input_layernorm = RMSNorm(c.hidden, c.eps)
        self.self_attn = Attention(c)
        self.post_attention_layernorm = RMSNorm(c.hidden, c.eps)
        self.mlp = MLP(c)

    def forward(self, x, cos, sin):
        x = x + self.self_attn(self.input_layernorm(x), cos, sin)
        return x + self.mlp(self.post_attention_layernorm(x))


class Inner(nn.Module):
    def __init__(self, c: Config):
        super().__init__()
        self.embed_tokens = nn.Embedding(c.vocab, c.hidden)
        self.layers = nn.ModuleList(Layer(c) for _ in range(c.layers))
        self.norm = RMSNorm(c.hidden, c.eps)


def mrope_axes(c: Config, device) -> torch.Tensor:
    """For each of the head_dim / 2 rotary pairs, which position axis it reads (0 t, 1 h, 2 w)."""
    half = c.head_dim // 2
    axes = []
    for j in range(half):
        if j % 3 == 1 and j < 3 * c.sections[1]:
            axes.append(1)
        elif j % 3 == 2 and j < 3 * c.sections[2]:
            axes.append(2)
        else:
            axes.append(0)
    return torch.tensor(axes, device=device)


class TextModel(nn.Module):
    """Parameter names under `model.` match `model.language_model.*` with the prefix swapped."""

    def __init__(self, c: Config):
        super().__init__()
        self.c = c
        self.model = Inner(c)
        self.checkpoint = False

    def rope(self, positions: torch.Tensor, dtype):
        """positions: (t, 3) int → cos, sin of shape (1, 1, t, head_dim)."""
        c = self.c
        d = c.head_dim
        inv = 1.0 / (c.theta ** (torch.arange(0, d, 2, device=positions.device, dtype=torch.float32) / d))
        axes = mrope_axes(c, positions.device)
        pos = positions.to(torch.float32)[:, axes]  # (t, half)
        f = pos * inv[None, :]
        emb = torch.cat((f, f), dim=-1)
        return emb.cos().to(dtype)[None, None], emb.sin().to(dtype)[None, None]

    def hidden(self, ids: torch.Tensor, positions: torch.Tensor, picture: torch.Tensor, taps: list):
        """ids: (t,) with -1 at picture rows; positions: (t, 3); picture: (count, hidden); taps: list of (count, hidden)."""
        is_pic = ids < 0
        x = self.model.embed_tokens(ids.clamp(min=0))
        x = x.clone()
        x[is_pic] = picture.to(x.dtype)
        x = x[None]
        cos, sin = self.rope(positions, x.dtype)
        for l, layer in enumerate(self.model.layers):
            if self.checkpoint and self.training and torch.is_grad_enabled():
                x = torch.utils.checkpoint.checkpoint(layer, x, cos, sin, use_reentrant=False)
            else:
                x = layer(x, cos, sin)
            if l < len(taps):
                add = torch.zeros_like(x)
                add[0, is_pic] = taps[l].to(x.dtype)
                x = x + add
        return self.model.norm(x)[0]

    def logits(self, h):
        return (h @ self.model.embed_tokens.weight.t()).to(torch.float32)


def text_tensor_files(release: Path) -> dict:
    """`model.language_model.*` tensor name → shard file of the release."""
    index = json.loads((release / "model.safetensors.index.json").read_text())
    return {k: release / f for k, f in index["weight_map"].items() if k.startswith(PREFIX)}


def load(release: Path, device: str, dtype) -> TextModel:
    cfg = Config(json.loads((release / "config.json").read_text()))
    # built in the target dtype: a float32 copy of 4 B parameters would not fit a 12 GB card
    before = torch.get_default_dtype()
    torch.set_default_dtype(dtype)
    try:
        with torch.device("meta"):
            m = TextModel(cfg)
    finally:
        torch.set_default_dtype(before)
    m = m.to_empty(device=device)
    sd = {}
    by_file = {}
    for k, f in text_tensor_files(release).items():
        by_file.setdefault(f, []).append(k)
    for f, keys in by_file.items():
        with safe_open(str(f), framework="pt") as g:
            for k in keys:
                sd["model." + k[len(PREFIX):]] = g.get_tensor(k).to(dtype)
    missing, unexpected = m.load_state_dict(sd, strict=False)
    if missing or unexpected:
        raise ValueError(f"checkpoint mismatch: missing {missing[:5]}, unexpected {unexpected[:5]}")
    m.to(device=device, dtype=dtype)
    for p in m.parameters():
        p.requires_grad_(False)
    return m


def add_lora(m: TextModel, r: int, alpha: float, dropout: float):
    for layer in m.model.layers:
        for parent in (layer.self_attn, layer.mlp):
            for name in LORA_TARGETS:
                lin = getattr(parent, name, None)
                if isinstance(lin, nn.Linear):
                    setattr(parent, name, LoRALinear(lin, r, alpha, dropout))
    return [p for p in m.parameters() if p.requires_grad]


def merged_text_state(m: TextModel) -> dict:
    """Every adapted weight merged into its base, under the release's `model.language_model.*` names
    (only the tensors LoRA changed: the rest stay in the release's own shards)."""
    out = {}
    for name, mod in m.named_modules():
        if isinstance(mod, LoRALinear):
            out[PREFIX + name[len("model."):] + ".weight"] = mod.merged_weight().contiguous().cpu()
    return out
