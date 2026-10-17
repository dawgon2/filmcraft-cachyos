"""A small, dependency-light PyTorch implementation of the two decoder families FilmCraft's
explainer and assistant use (Llama: SmolLM2; Qwen2: Qwen2.5; SmolLM3: Llama-style with no rotary
positions in every 4th layer), with LoRA adapters.

Offline training tooling only (GRADE-PLAN G2.11b); never part of the product. It needs only
`torch`, `safetensors` and `tokenizers`, so it does not depend on the `transformers` /
`peft` versions installed in the venv. Weights are read from and written to the base model's own
Hugging Face tensor names, so the merged checkpoint loads anywhere the base does (and FilmCraft's
Rust GGUF converter maps the same names).

Architecture (from the published configs and model code conventions of both families):
pre-norm decoder; RMSNorm (computed in float32); grouped-query attention with rotary position
embeddings (non-interleaved halves); SwiGLU MLP (down(silu(gate(x)) * up(x))); Qwen2 adds biases
to the q/k/v projections; both checkpoints here tie the output head to the token embeddings.
"""

import json
import math
from pathlib import Path

import torch
import torch.utils.checkpoint
import torch.nn as nn
import torch.nn.functional as F
from safetensors.torch import load_file

LORA_TARGETS = ("q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj")


class Config:
    def __init__(self, d: dict):
        self.family = d.get("model_type", "llama")
        if self.family not in ("llama", "qwen2", "smollm3"):
            raise ValueError(f"unsupported model_type {self.family!r}")
        self.hidden = int(d["hidden_size"])
        self.inter = int(d["intermediate_size"])
        self.layers = int(d["num_hidden_layers"])
        self.heads = int(d["num_attention_heads"])
        self.kv_heads = int(d.get("num_key_value_heads", self.heads))
        self.eps = float(d.get("rms_norm_eps", 1e-6))
        self.theta = float(d.get("rope_theta", 10000.0))
        self.vocab = int(d["vocab_size"])
        self.tied = bool(d.get("tie_word_embeddings", False))
        self.qkv_bias = self.family == "qwen2" or bool(d.get("attention_bias", False))
        if d.get("rope_scaling"):
            raise ValueError("rope_scaling is not supported")
        if self.hidden % self.heads or self.heads % self.kv_heads:
            raise ValueError("bad head layout")
        self.head_dim = self.hidden // self.heads
        # SmolLM3 (NoPE): `no_rope_layers[i]` is 1 where layer i uses rotary positions, 0 where it
        # uses none (every 4th layer in SmolLM3-3B); the other families rotate in every layer
        flags = d.get("no_rope_layers") if self.family == "smollm3" else None
        if flags is None:
            self.use_rope = [True] * self.layers
        else:
            if len(flags) != self.layers:
                raise ValueError("no_rope_layers length differs from num_hidden_layers")
            self.use_rope = [bool(f) for f in flags]


class LoRALinear(nn.Module):
    """A frozen linear layer plus a trainable low-rank update B·A (scaled by alpha / r)."""

    def __init__(self, base: nn.Linear, r: int, alpha: float, dropout: float):
        super().__init__()
        self.base = base
        for p in self.base.parameters():
            p.requires_grad_(False)
        self.r = r
        self.scale = alpha / r
        dev = base.weight.device
        self.lora_a = nn.Parameter(torch.empty(r, base.in_features, device=dev, dtype=torch.float32))
        self.lora_b = nn.Parameter(torch.zeros(base.out_features, r, device=dev, dtype=torch.float32))
        nn.init.kaiming_uniform_(self.lora_a, a=math.sqrt(5))
        self.drop = nn.Dropout(dropout) if dropout > 0 else nn.Identity()

    def forward(self, x):
        y = self.base(x)
        # activations stay in the base dtype (bf16 on the GPU); the adapter's master weights are
        # float32 and receive float32 gradients through the casts
        u = self.drop(x) @ self.lora_a.to(x.dtype).t()
        return y + (u @ self.lora_b.to(x.dtype).t()) * self.scale

    @torch.no_grad()
    def merged_weight(self) -> torch.Tensor:
        w = self.base.weight.to(torch.float32) + (self.lora_b @ self.lora_a) * self.scale
        return w.to(self.base.weight.dtype)


class RMSNorm(nn.Module):
    def __init__(self, n: int, eps: float):
        super().__init__()
        self.weight = nn.Parameter(torch.ones(n))
        self.eps = eps

    def forward(self, x):
        dt = x.dtype
        x = x.to(torch.float32)
        x = x * torch.rsqrt(x.pow(2).mean(-1, keepdim=True) + self.eps)
        return self.weight * x.to(dt)


def rotate_half(x):
    a, b = x.chunk(2, dim=-1)
    return torch.cat((-b, a), dim=-1)


class Attention(nn.Module):
    def __init__(self, c: Config):
        super().__init__()
        self.c = c
        self.q_proj = nn.Linear(c.hidden, c.heads * c.head_dim, bias=c.qkv_bias)
        self.k_proj = nn.Linear(c.hidden, c.kv_heads * c.head_dim, bias=c.qkv_bias)
        self.v_proj = nn.Linear(c.hidden, c.kv_heads * c.head_dim, bias=c.qkv_bias)
        self.o_proj = nn.Linear(c.heads * c.head_dim, c.hidden, bias=False)

    def forward(self, x, cos, sin):
        b, t, _ = x.shape
        c = self.c
        q = self.q_proj(x).view(b, t, c.heads, c.head_dim).transpose(1, 2)
        k = self.k_proj(x).view(b, t, c.kv_heads, c.head_dim).transpose(1, 2)
        v = self.v_proj(x).view(b, t, c.kv_heads, c.head_dim).transpose(1, 2)
        q = q * cos + rotate_half(q) * sin
        k = k * cos + rotate_half(k) * sin
        rep = c.heads // c.kv_heads
        if rep > 1:
            k = k.repeat_interleave(rep, dim=1)
            v = v.repeat_interleave(rep, dim=1)
        y = F.scaled_dot_product_attention(q, k, v, is_causal=True)
        return self.o_proj(y.transpose(1, 2).reshape(b, t, c.heads * c.head_dim))


class MLP(nn.Module):
    def __init__(self, c: Config):
        super().__init__()
        self.gate_proj = nn.Linear(c.hidden, c.inter, bias=False)
        self.up_proj = nn.Linear(c.hidden, c.inter, bias=False)
        self.down_proj = nn.Linear(c.inter, c.hidden, bias=False)

    def forward(self, x):
        return self.down_proj(F.silu(self.gate_proj(x)) * self.up_proj(x))


class Layer(nn.Module):
    def __init__(self, c: Config, rope: bool = True):
        super().__init__()
        self.rope = rope
        self.input_layernorm = RMSNorm(c.hidden, c.eps)
        self.self_attn = Attention(c)
        self.post_attention_layernorm = RMSNorm(c.hidden, c.eps)
        self.mlp = MLP(c)

    def forward(self, x, cos, sin):
        if not self.rope:
            cos, sin = torch.ones_like(cos), torch.zeros_like(sin)
        x = x + self.self_attn(self.input_layernorm(x), cos, sin)
        return x + self.mlp(self.post_attention_layernorm(x))


class Inner(nn.Module):
    def __init__(self, c: Config):
        super().__init__()
        self.embed_tokens = nn.Embedding(c.vocab, c.hidden)
        self.layers = nn.ModuleList(Layer(c, c.use_rope[i]) for i in range(c.layers))
        self.norm = RMSNorm(c.hidden, c.eps)


class CausalLM(nn.Module):
    """Parameter names match the Hugging Face checkpoints (`model.layers.N.self_attn.q_proj...`)."""

    def __init__(self, c: Config):
        super().__init__()
        self.c = c
        self.model = Inner(c)
        self.lm_head = None if c.tied else nn.Linear(c.hidden, c.vocab, bias=False)
        self._rope = {}
        # recompute each layer's activations in the backward pass (training memory ~ one layer)
        self.checkpoint = False

    def head_weight(self):
        return self.model.embed_tokens.weight if self.lm_head is None else self.lm_head.weight

    def rope(self, t: int, device, dtype):
        key = (t, device, dtype)
        if key not in self._rope:
            d = self.c.head_dim
            inv = 1.0 / (self.c.theta ** (torch.arange(0, d, 2, device=device, dtype=torch.float32) / d))
            pos = torch.arange(t, device=device, dtype=torch.float32)
            f = torch.outer(pos, inv)
            emb = torch.cat((f, f), dim=-1)
            self._rope = {key: (emb.cos().to(dtype)[None, None], emb.sin().to(dtype)[None, None])}
        return self._rope[key]

    def hidden(self, ids):
        x = self.model.embed_tokens(ids)
        cos, sin = self.rope(ids.shape[1], ids.device, x.dtype)
        for layer in self.model.layers:
            if self.checkpoint and self.training and torch.is_grad_enabled():
                x = torch.utils.checkpoint.checkpoint(layer, x, cos, sin, use_reentrant=False)
            else:
                x = layer(x, cos, sin)
        return self.model.norm(x)

    def logits_at(self, h):
        return (h @ self.head_weight().t()).to(torch.float32)


def load(base_dir: Path, device: str, dtype) -> CausalLM:
    cfg = Config(json.loads((base_dir / "config.json").read_text()))
    # build in the target dtype: a 3B model allocated as float32 first doesn't fit a 12 GB GPU
    old = torch.get_default_dtype()
    torch.set_default_dtype(dtype)
    try:
        with torch.device("meta"):
            m = CausalLM(cfg)
    finally:
        torch.set_default_dtype(old)
    index = base_dir / "model.safetensors.index.json"
    if index.exists():
        # sharded checkpoint (SmolLM3-3B: two files)
        sd = {}
        for shard in sorted(set(json.loads(index.read_text())["weight_map"].values())):
            sd.update(load_file(str(base_dir / shard)))
    else:
        sd = load_file(str(base_dir / "model.safetensors"))
    m = m.to_empty(device=device)
    missing, unexpected = m.load_state_dict({k: v.to(dtype) for k, v in sd.items()}, strict=False)
    missing = [k for k in missing if k != "lm_head.weight"]
    if missing or unexpected:
        raise ValueError(f"checkpoint mismatch: missing {missing[:5]}, unexpected {unexpected[:5]}")
    m.to(device=device, dtype=dtype)
    m._rope = {}
    for p in m.parameters():
        p.requires_grad_(False)
    return m


def add_lora(m: CausalLM, r: int, alpha: float, dropout: float):
    for layer in m.model.layers:
        for parent in (layer.self_attn, layer.mlp):
            for name in LORA_TARGETS:
                lin = getattr(parent, name, None)
                if isinstance(lin, nn.Linear):
                    setattr(parent, name, LoRALinear(lin, r, alpha, dropout))
    return [p for p in m.parameters() if p.requires_grad]


def lora_state(m: CausalLM) -> dict:
    return {k: v.detach().clone() for k, v in m.state_dict().items() if ".lora_" in k}


def merged_state(m: CausalLM, original_keys) -> dict:
    """The base checkpoint's tensors with every LoRA update merged in, under the original names."""
    out = {}
    for name, mod in m.named_modules():
        if isinstance(mod, LoRALinear):
            out[f"{name}.weight"] = mod.merged_weight().contiguous().cpu()
            if mod.base.bias is not None:
                out[f"{name}.bias"] = mod.base.bias.detach().contiguous().cpu()
    sd = m.state_dict()
    for k in original_keys:
        if k in out:
            continue
        alt = k
        if k not in sd:
            # a LoRA-wrapped layer's own weight lives under `.base.`
            parts = k.rsplit(".", 1)
            alt = f"{parts[0]}.base.{parts[1]}"
        out[k] = sd[alt].detach().contiguous().cpu()
    return {k: out[k] for k in original_keys}


@torch.no_grad()
def greedy(m: CausalLM, ids: list, stop: set, max_new: int = 96, device="cuda") -> list:
    """Greedy decoding without a cache (answers are short); for spot checks only."""
    x = torch.tensor([ids], device=device)
    out = []
    for _ in range(max_new):
        h = m.hidden(x)[:, -1]
        nxt = int(m.logits_at(h).argmax(-1).item())
        if nxt in stop:
            break
        out.append(nxt)
        x = torch.cat([x, torch.tensor([[nxt]], device=device)], dim=1)
    return out
