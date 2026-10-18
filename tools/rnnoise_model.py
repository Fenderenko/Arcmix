#!/usr/bin/env python3
"""Extracts the RNNoise model (classic xiph/rnnoise, commit 7f449bf, BSD-3) into assets/rnnoise.bin.

Usage: python3 tools/rnnoise_model.py <path to rnnoise/src>

The file holds the int8 weights of all layers in the order rust/src/rnnoise.rs expects; the tansig
table is printed as a Rust literal (it is already in rnnoise.rs)."""
import os
import re
import sys

ORDER = [
    ("input_dense_weights", 1008), ("input_dense_bias", 24),
    ("vad_gru_weights", 1728), ("vad_gru_recurrent_weights", 1728), ("vad_gru_bias", 72),
    ("noise_gru_weights", 12960), ("noise_gru_recurrent_weights", 6912), ("noise_gru_bias", 144),
    ("denoise_gru_weights", 32832), ("denoise_gru_recurrent_weights", 27648), ("denoise_gru_bias", 288),
    ("denoise_output_weights", 2112), ("denoise_output_bias", 22),
    ("vad_output_weights", 24), ("vad_output_bias", 1),
]

LAYERS = [
    ("input_dense", "42, 24, ACTIVATION_TANH"), ("vad_gru", "24, 24, ACTIVATION_RELU"),
    ("noise_gru", "90, 48, ACTIVATION_RELU"), ("denoise_gru", "114, 96, ACTIVATION_RELU"),
    ("denoise_output", "96, 22, ACTIVATION_SIGMOID"), ("vad_output", "24, 1, ACTIVATION_SIGMOID"),
]


def main(src):
    text = open(os.path.join(src, "rnn_data.c")).read()
    arrays = {}
    for m in re.finditer(r"static const rnn_weight (\w+)\[(\d+)\] = \{(.*?)\};", text, re.S):
        vals = [int(v) for v in m.group(3).replace("\n", " ").split(",") if v.strip()]
        assert len(vals) == int(m.group(2)), m.group(1)
        arrays[m.group(1)] = vals
    out = bytearray()
    for name, n in ORDER:
        vals = arrays[name]
        assert len(vals) == n, (name, len(vals))
        assert all(-128 <= v <= 127 for v in vals)
        out += bytes(v & 0xff for v in vals)
    # layer sizes and activations: the model is the one rnnoise.rs was written for
    for layer, spec in LAYERS:
        assert re.search(r"Layer %s = \{[^}]*%s" % (layer, re.escape(spec)), text), layer
    dst = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "rnnoise.bin")
    with open(dst, "wb") as f:
        f.write(out)
    print("// wrote %s (%d bytes)" % (os.path.normpath(dst), len(out)))
    tab = open(os.path.join(src, "tansig_table.h")).read()
    vals = re.findall(r"(-?\d+\.\d+)f", tab)
    assert len(vals) == 201
    print("static TANSIG_TABLE: [f32; 201] = [")
    for i in range(0, 201, 8):
        print("    " + ", ".join(vals[i:i + 8]) + ",")
    print("];")


if __name__ == "__main__":
    main(sys.argv[1])
