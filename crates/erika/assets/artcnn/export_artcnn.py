#!/usr/bin/env python3
"""Export ArtCNN C-series ONNX weights into Erika's Metal compute blob format.

Usage:
    python3 export_artcnn.py ArtCNN_C4F16.onnx artcnn_c4f16.bin [--test-vector DIR]

Blob layout (little-endian):
    u32 magic = 0x4E4E4341 ("ACNN")
    u32 version = 1
    u32 feature_count F (16 or 32)
    u32 reserved = 0
    conv0:    F/4 slices x 9 taps x half4          (input luma -> F feats)
    bias0:    F halfs
    conv1..5: F/4 out-slices x 9 taps x F/4 in-slices x half4x4 (column-major:
              column j = weights of input channel j for the 4 output channels)
    bias1..5: F halfs each
    conv6:    9 taps x F/4 in-slices x half4x4     (F feats -> 4 subpixels)
    bias6:    4 halfs
Tap order: t = (dy+1)*3 + (dx+1), dy/dx in -1..1 (matches ONNX [ky][kx]).
DepthToSpace is DCR: subpixel channel c = dy*2 + dx.
"""

import argparse
import struct
import sys

import numpy as np
import onnx
from onnx import numpy_helper

MAGIC = 0x4E4E4341
VERSION = 1


def load_convs(model):
    inits = {i.name: numpy_helper.to_array(i) for i in model.graph.initializer}
    convs = []
    for node in model.graph.node:
        if node.op_type == "Conv":
            kernel = inits[node.input[1]].astype(np.float32)  # [O, I, 3, 3]
            bias = inits[node.input[2]].astype(np.float32)  # [O]
            convs.append((kernel, bias))
    return convs


def half4x4_col_major(kernel, out_base, in_base, ky, kx):
    """16 halfs: column j = kernel[out_base..+4, in_base+j, ky, kx]."""
    block = np.empty(16, dtype=np.float32)
    for j in range(4):
        for o in range(4):
            block[j * 4 + o] = kernel[out_base + o, in_base + j, ky, kx]
    return block


def export(convs, feats):
    slices = feats // 4
    out = bytearray()
    out += struct.pack("<IIII", MAGIC, VERSION, feats, 0)

    def emit(arr):
        out.extend(np.asarray(arr, dtype=np.float32).astype(np.float16).tobytes())

    kernel0, bias0 = convs[0]
    assert kernel0.shape == (feats, 1, 3, 3), kernel0.shape
    for s in range(slices):
        for ky in range(3):
            for kx in range(3):
                emit(kernel0[s * 4 : s * 4 + 4, 0, ky, kx])
    emit(bias0)

    for kernel, bias in convs[1:6]:
        assert kernel.shape == (feats, feats, 3, 3), kernel.shape
        for s in range(slices):
            for ky in range(3):
                for kx in range(3):
                    for i in range(slices):
                        emit(half4x4_col_major(kernel, s * 4, i * 4, ky, kx))
        emit(bias)

    kernel6, bias6 = convs[6]
    assert kernel6.shape == (4, feats, 3, 3), kernel6.shape
    for ky in range(3):
        for kx in range(3):
            for i in range(slices):
                emit(half4x4_col_major(kernel6, 0, i * 4, ky, kx))
    emit(bias6)
    return bytes(out)


def make_test_input(height=72, width=128):
    """Deterministic synthetic luma patch: gradients, line art, noise."""
    rng = np.random.default_rng(20260611)
    y, x = np.mgrid[0:height, 0:width].astype(np.float32)
    img = 0.35 + 0.3 * np.sin(x / 9.0) * np.cos(y / 7.0)
    img += 0.25 * ((x + y * 1.7) % 23.0 < 2.0)  # diagonal "ink lines"
    img += 0.05 * rng.standard_normal((height, width)).astype(np.float32)
    img = np.clip(img, 0.0, 1.0)
    # Quantize to the 8-bit grid so an R8Unorm texture holds the input exactly.
    return (np.round(img * 255.0) / 255.0).astype(np.float32)


def write_test_vector(onnx_path, directory):
    import onnxruntime as ort

    img = make_test_input()
    session = ort.InferenceSession(onnx_path, providers=["CPUExecutionProvider"])
    output = session.run(None, {"input": img[None, None]})[0][0, 0]
    height, width = img.shape
    with open(f"{directory}/input_{width}x{height}.f32", "wb") as f:
        f.write(img.tobytes())
    with open(f"{directory}/output_{width * 2}x{height * 2}.f32", "wb") as f:
        f.write(output.astype(np.float32).tobytes())
    print(f"test vector: input {width}x{height} -> output {width * 2}x{height * 2}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("onnx_path")
    parser.add_argument("blob_path")
    parser.add_argument("--test-vector", metavar="DIR")
    args = parser.parse_args()

    model = onnx.load(args.onnx_path)
    convs = load_convs(model)
    if len(convs) != 7:
        sys.exit(f"expected 7 convs, found {len(convs)}")
    feats = convs[0][0].shape[0]
    blob = export(convs, feats)
    with open(args.blob_path, "wb") as f:
        f.write(blob)
    print(f"{args.blob_path}: F={feats}, {len(blob)} bytes")
    if args.test_vector:
        write_test_vector(args.onnx_path, args.test_vector)


if __name__ == "__main__":
    main()
