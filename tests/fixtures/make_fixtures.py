"""Builds the GLB fixtures the tests load (needs trimesh, pillow, numpy).

model.glb     — a textured sphere with a PBR material, like any Meshy export
mosquito.glb  — a body along Z, two flat wings out to the sides above it,
                six legs hanging below; facing +Z like Meshy's exports
humanoid.glb  — a T-posed figure, feet at the origin, facing +Z
"""
import numpy as np, trimesh
from PIL import Image

def pbr():
    tex = Image.fromarray((np.random.RandomState(1).rand(32, 32, 3) * 255).astype('uint8'))
    return trimesh.visual.material.PBRMaterial(baseColorTexture=tex, name="Material_0")

def export(meshes, path):
    m = trimesh.util.concatenate(meshes)
    uv = (m.vertices[:, [0, 2]] - m.vertices[:, [0, 2]].min(0)) / np.ptp(m.vertices[:, [0, 2]], 0)
    m.visual = trimesh.visual.TextureVisuals(uv=uv, material=pbr())
    s = trimesh.Scene(); s.add_geometry(m, node_name="mesh_0", geom_name="mesh_0")
    open(path, 'wb').write(s.export(file_type='glb'))

def box(c, size):
    b = trimesh.creation.box(extents=size); b.apply_translation(c); return b

sphere = trimesh.creation.icosphere(subdivisions=2, radius=0.5); sphere.apply_translation([0, 0.5, 0])
export([sphere], 'model.glb')

# Mosquito: ~2 long, wingspan ~2, origin at the bottom.
body = trimesh.creation.cylinder(radius=0.12, height=1.6, sections=12)
body.apply_transform(trimesh.transformations.rotation_matrix(np.pi / 2, [1, 0, 0]))
body.apply_translation([0, 0.55, 0])
head = trimesh.creation.icosphere(subdivisions=2, radius=0.14); head.apply_translation([0, 0.6, 0.85])
wings = [box([s * 0.55, 0.7, 0.0], [0.85, 0.01, 0.35]) for s in (1, -1)]
legs = []
for s in (1, -1):
    for z in (-0.3, 0.0, 0.3):
        l = trimesh.creation.cylinder(radius=0.015, height=0.5, sections=6)
        l.apply_transform(trimesh.transformations.rotation_matrix(s * 0.4, [0, 0, 1]))
        l.apply_translation([s * 0.12, 0.25, z]); legs.append(l)
export([body, head, *wings, *legs], 'mosquito.glb')

# Humanoid, 1.8 tall, T-pose.
parts = [box([0, 1.2, 0], [0.36, 0.55, 0.2]), box([0, 1.66, 0], [0.2, 0.25, 0.2]),
         box([0.52, 1.42, 0], [0.7, 0.1, 0.1]), box([-0.52, 1.42, 0], [0.7, 0.1, 0.1]),
         box([0.11, 0.45, 0], [0.14, 0.9, 0.14]), box([-0.11, 0.45, 0], [0.14, 0.9, 0.14])]
# Subdivide so there are vertices along the limbs to weight.
parts = [p.subdivide().subdivide() for p in parts]
export(parts, 'humanoid.glb')
print("fixtures written")
