"""Validate timed original-map captures, raw opacity UV, clock and unchanged controls.

The matrix offsets come from executed original x86 (native_oracle.json). Pixels use
the original DDS base mip with bilinear sRGB sampling, black-background blending,
and the fixture's recorded camera. This does not emulate native scene lighting.
"""
from pathlib import Path
import json
import math
import numpy as np
from PIL import Image
from capture_waterfall import OUT, ROOT
from waterfall_source_oracle import Native, read_pak

NATIVE = Native(ROOT / 'data/extract/aa-cry/ida/render/cryrenderd3d10.dll')


def pixels(name):
    return np.array(Image.open(OUT / (name + '.png')).convert('RGB'))


def changed(a, b):
    return int(np.count_nonzero(np.any(pixels(a) != pixels(b), axis=2)))


def decode(path, srgb):
    raw = read_pak(path)
    import io
    value = np.array(Image.open(io.BytesIO(raw)).convert('RGBA')) / 255.
    if srgb:
        rgb = value[:, :, :3]
        value[:, :, :3] = np.where(rgb <= .04045, rgb / 12.92, ((rgb + .055) / 1.055)**2.4)
    return value


def sample(image, uv):
    height, width, _ = image.shape
    x, y = uv * [width, height] - .5
    ix, iy = int(np.floor(x)), int(np.floor(y))
    fx, fy = x - ix, y - iy
    return ((image[iy % height, ix % width] * (1-fx) + image[iy % height, (ix+1) % width] * fx) * (1-fy)
            + (image[(iy+1) % height, ix % width] * (1-fx) + image[(iy+1) % height, (ix+1) % width] * fx) * fy)


def srgb(value):
    return np.where(value <= .0031308, value * 12.92, 1.055 * value**(1/2.4) - .055) * 255


def errors(name, time, native, diffuse, opacity, color, alpha, wrong_opacity=False):
    capture = pixels(name)
    height, width, _ = capture.shape
    # Rectangle 2m wide/high, z=3m; Bevy's default vertical FOV is pi/4.
    extent = height / (3 * math.tan(math.pi/8))
    matrix = NATIVE.matrix(time, **{k: v for k, v in native.items() if k != 'path'})
    tile = matrix[[0, 1], [0, 1]]
    translation = matrix[3, :2]
    results = []
    for py in range(160, height-160, 29):
        for px in range(280, width-280, 31):
            raw_uv = np.array([.5 + (px+.5-width/2)/extent, .5 + (py+.5-height/2)/extent])
            diffuse_uv = raw_uv * tile + translation
            texel = sample(diffuse, diffuse_uv)
            coverage = sample(opacity, diffuse_uv if wrong_opacity else raw_uv)[3]
            expected = srgb(texel[:3] * color * (alpha * texel[3] * coverage))
            results.append(float(np.max(np.abs(expected-capture[py, px]))))
    return dict(samples=len(results), max_srgb_error=max(results), mean_srgb_error=float(np.mean(results)))


def main():
    result = {}
    for kind in ['quad', 'mesh-yaw-90']:
        assert changed(f'{kind}-t0-before', f'{kind}-t2-before') == 0
        count = changed(f'{kind}-t0-after', f'{kind}-t2-after')
        assert count > 10000, (kind, count)
        result[kind + '_motion_changed_pixels'] = count
    for a, b in [('quad-token-zero-after', 'quad-t0-before'),
                 ('quad-t2-after', 'quad-t2-ordinary-after'),
                 ('second-t3600.25-after', 'second-t3600.25-ordinary-after'),
                 ('colors-before', 'colors-after'),
                 ('light-point-before', 'light-point-after'),
                 ('light-spot-before', 'light-spot-after')]:
        count = changed(a, b)
        assert count == 0, (a, b, count)
        result[a + '_equals_' + b] = count
    golden = json.loads((OUT / 'native_oracle.json').read_text())
    primary, _, second = golden['cases'][:3]
    diffuse = decode('game/textures/p_effects/water_fall_forest_foam_small.dds', True)
    opacity = decode('game/objects/env/05_nature/08_cry_source/ice/waterfall/waterfall_op_3.dds', False)
    color = np.array([.30588236, .48627454, .52156866])
    for time in [0, 1, 2, 4]:
        values = errors(f'quad-t{time}-after', time, primary, diffuse, opacity, color, .7)
        assert values['max_srgb_error'] < 2.1, (time, values)
        result[f'native_pixels_t{time}'] = values
    wrong = errors('quad-t2-after', 2, primary, diffuse, opacity, color, .7, wrong_opacity=True)
    assert wrong['mean_srgb_error'] > result['native_pixels_t2']['mean_srgb_error'] * 10
    result['wrong_moving_opacity_model'] = wrong
    opacity = decode('game/objects/env/05_nature/08_cry_source/ice/waterfall/waterfall_op2.dds', False)
    color = np.array([.18039216, .24705884, .24313727])
    for time in [3599.75, 3600.25, 3601.25]:
        correct = errors(f'second-t{time}-after', time, second, diffuse, opacity, color, .842)
        wrapped = errors(f'second-t{time}-after', time % 3600, second, diffuse, opacity, color, .842)
        # Second material slightly minifies the DDS with anisotropic filtering;
        # the base-mip CPU model is a discriminator, not a pixel-exact filter oracle.
        if time > 3600:
            assert correct['mean_srgb_error'] < wrapped['mean_srgb_error'] * .3, (time, correct, wrapped)
        result[f'clock_t{time}'] = dict(native_unwrapped=correct, incorrect_wrapped=wrapped)
    result['limits'] = ('Primary samples magnify mip0; remaining error includes BC3 decode/filter rounding. '
                        'Clock control uses native source parameters and a base-mip discriminator, not exact anisotropic minification. '
                        'No matched real-client waterfall screenshot or complete native scene lighting.')
    (OUT / 'validation.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
