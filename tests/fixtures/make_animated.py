"""Builds animated.glb: a skinned box figure with two bones and two clips,
"Idle" (still) and "Walking" (the upper bone swings), the way a rigged and
animated Meshy/Mixamo export looks to the game. Pure Python, no deps."""
import json, struct, math

def pad(b, n=4, fill=b'\0'):
    return b + fill * ((n - len(b) % n) % n)

# A 0.4 × 1.8 × 0.4 box standing on the origin, split at y = 0.9: the lower
# half follows bone 0 (root), the upper half bone 1 (its child at y = 0.9).
verts, joints, weights = [], [], []
for y in (0.0, 0.9, 1.8):
    for x, z in ((-0.2, -0.2), (0.2, -0.2), (0.2, 0.2), (-0.2, 0.2)):
        verts.append((x, y, z))
        j = 1 if y > 0.9 else 0
        joints.append((j, 0, 0, 0)); weights.append((1.0, 0.0, 0.0, 0.0))
idx = []
for ring in (0, 4):
    for k in range(4):
        a, b = ring + k, ring + (k + 1) % 4
        c, d = a + 4, b + 4
        idx += [a, b, c, b, d, c]
# Caps.
idx += [0, 2, 1, 0, 3, 2, 8, 9, 10, 8, 10, 11]

times = [0.0, 0.5, 1.0]
def quat_x(a):
    return (math.sin(a / 2), 0.0, 0.0, math.cos(a / 2))
walk_rot = [quat_x(0.0), quat_x(0.6), quat_x(0.0)]
idle_rot = [quat_x(0.0), quat_x(0.0), quat_x(0.0)]

ibm = [  # inverse bind matrices, column major
    (1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1),
    (1,0,0,0, 0,1,0,0, 0,0,1,0, 0,-0.9,0,1),
]

blob = b''
views, accessors = [], []
def add(data, fmt, count, ctype, atype, target=None, minmax=None):
    global blob
    raw = b''.join(struct.pack(fmt, *d) if isinstance(d, tuple) else struct.pack(fmt, d) for d in data)
    off = len(blob); blob = pad(blob + raw)
    v = {"buffer": 0, "byteOffset": off, "byteLength": len(raw)}
    if target: v["target"] = target
    views.append(v)
    acc = {"bufferView": len(views) - 1, "componentType": ctype, "count": count, "type": atype}
    if minmax: acc["min"], acc["max"] = minmax
    accessors.append(acc)
    return len(accessors) - 1

xs, ys, zs = zip(*verts)
a_pos = add(verts, '<fff', len(verts), 5126, "VEC3", 34962, ([min(xs), min(ys), min(zs)], [max(xs), max(ys), max(zs)]))
a_joint = add(joints, '<HHHH', len(joints), 5123, "VEC4", 34962)
a_weight = add(weights, '<ffff', len(weights), 5126, "VEC4", 34962)
a_idx = add(idx, '<H', len(idx), 5123, "SCALAR", 34963)
a_ibm = add(ibm, '<' + 'f' * 16, 2, 5126, "MAT4")
a_time = add(times, '<f', 3, 5126, "SCALAR", None, ([0.0], [1.0]))
a_walk = add(walk_rot, '<ffff', 3, 5126, "VEC4")
a_idle = add(idle_rot, '<ffff', 3, 5126, "VEC4")

def clip(name, acc):
    return {"name": name,
            "samplers": [{"input": a_time, "output": acc, "interpolation": "LINEAR"}],
            "channels": [{"sampler": 0, "target": {"node": 2, "path": "rotation"}}]}

gltf = {
    "asset": {"version": "2.0", "generator": "make_animated.py"},
    "scene": 0,
    "scenes": [{"nodes": [0, 1]}],
    "nodes": [
        {"name": "Figure", "mesh": 0, "skin": 0},
        {"name": "Hips", "children": [2], "translation": [0, 0, 0]},
        {"name": "Spine", "translation": [0, 0.9, 0]},
    ],
    "skins": [{"inverseBindMatrices": a_ibm, "joints": [1, 2], "skeleton": 1}],
    "meshes": [{"primitives": [{"attributes": {"POSITION": a_pos, "JOINTS_0": a_joint, "WEIGHTS_0": a_weight}, "indices": a_idx}]}],
    "animations": [clip("Idle", a_idle), clip("Walking", a_walk)],
    "buffers": [{"byteLength": len(blob)}],
    "bufferViews": views,
    "accessors": accessors,
}
js = pad(json.dumps(gltf).encode(), fill=b' ')
out = b'glTF' + struct.pack('<II', 2, 12 + 8 + len(js) + 8 + len(blob))
out += struct.pack('<I', len(js)) + b'JSON' + js
out += struct.pack('<I', len(blob)) + b'BIN\0' + blob
open('animated.glb', 'wb').write(out)
print('animated.glb', len(out), 'bytes')
