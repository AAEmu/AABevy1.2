//! Exact 1.2 CTimeOfDay variable catalog, raw f32 bits: scalar [default,min,max], RGB [r,g,b].
//! cry3dengine 300A9450 / 300A8EE0 / 300A92B0; consumed by env.rs at load time only.
pub(super) const VARIABLES: &[(&str, bool, [u32; 3])] = &[
    ("HDR dynamic power factor", false, [0x00000000, 0xc0800000, 0x40800000]), // 0
    ("Sky brightening (terrain occlusion)", false, [0x3e99999a, 0x00000000, 0x3f800000]), // 1
    ("SSDO Amount", false, [0x3f800000, 0x3dcccccd, 0x41200000]), // 2
    ("SSDO AmbientAmount", false, [0x3f800000, 0x3dcccccd, 0x41200000]), // 3
    ("Global illumination multiplier", false, [0x3f800000, 0x00000000, 0x42c80000]), // 4
    ("Sun color", true, [0x3f57d7d9, 0x3f48c8ca, 0x3f2aaaab]), // 5
    ("Sun color multiplier", false, [0x4019999a, 0x00000000, 0x41800000]), // 6
    ("Sun specular multiplier", false, [0x3f800000, 0x00000000, 0x40800000]), // 7
    ("Sky color", true, [0x3f20a0a1, 0x3f48c8ca, 0x3f70f0f2]), // 8
    ("Sky color multiplier", false, [0x3f8ccccd, 0x00000000, 0x41800000]), // 9
    ("Global Rim Amount", false, [0x3dcccccd, 0x00000000, 0x41800000]), // 10
    ("Ambient ground color", true, [0x3f20a0a1, 0x3f48c8ca, 0x3f70f0f2]), // 11
    ("Ambient ground color multiplier", false, [0x3f800000, 0x00000000, 0x41800000]), // 12
    ("Ambient min height", false, [0x00000000, 0x00000000, 0x46ea6000]), // 13
    ("Ambient max height", false, [0x3f800000, 0x3f800000, 0x46ea6000]), // 14
    ("Fog color", true, [0x00000000, 0x00000000, 0x00000000]), // 15
    ("Fog color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 16
    ("Fog height (bottom)", false, [0x00000000, 0xc59c4000, 0x46ea6000]), // 17
    ("Fog layer density (bottom)", false, [0x3f800000, 0x00000000, 0x3f800000]), // 18
    ("Fog color (top)", true, [0x00000000, 0x00000000, 0x00000000]), // 19
    ("Fog color (top) multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 20
    ("Fog height (top)", false, [0x457a0000, 0xc59c4000, 0x46ea6000]), // 21
    ("Fog layer density (top)", false, [0x00000000, 0x00000000, 0x3f800000]), // 22
    ("Fog color height offset", false, [0x00000000, 0xbf800000, 0x3f800000]), // 23
    ("Fog color (radial)", true, [0x00000000, 0x00000000, 0x00000000]), // 24
    ("Fog color (radial) multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 25
    ("Fog radial size", false, [0x3f400000, 0x00000000, 0x3f800000]), // 26
    ("Fog radial lobe", false, [0x3f000000, 0x00000000, 0x3f800000]), // 27
    ("Volumetric fog: Final density clamp", false, [0x3f800000, 0x00000000, 0x3f800000]), // 28
    ("Volumetric fog: Global density", false, [0x3ca3d70a, 0x00000000, 0x42c80000]), // 29
    ("Volumetric fog: Ramp start", false, [0x00000000, 0x00000000, 0x46ea6000]), // 30
    ("Volumetric fog: Ramp end", false, [0x42c80000, 0x00000000, 0x46ea6000]), // 31
    ("Volumetric fog: Ramp influence", false, [0x00000000, 0x00000000, 0x3f800000]), // 32
    ("Sky light: Sun intensity", true, [0x3f555555, 0x3f555555, 0x3f800000]), // 33
    ("Sky light: Sun intensity multiplier", false, [0x41f00000, 0x00000000, 0x447a0000]), // 34
    ("Sky light: Mie scattering", false, [0x4099999a, 0x00000000, 0x49742400]), // 35
    ("Sky light: Rayleigh scattering", false, [0x40000000, 0x00000000, 0x49742400]), // 36
    ("Sky light: Sun anisotropy factor", false, [0xbf7eb852, 0xbf7ff972, 0x3f7ff972]), // 37
    ("Sky light: Wavelength (R)", false, [0x443b8000, 0x43be0000, 0x44430000]), // 38
    ("Sky light: Wavelength (G)", false, [0x44164000, 0x43be0000, 0x44430000]), // 39
    ("Sky light: Wavelength (B)", false, [0x440ac000, 0x43be0000, 0x44430000]), // 40
    ("Night sky: Horizon color", true, [0x3f5edee0, 0x3f149495, 0x3e3cbcbd]), // 41
    ("Night sky: Horizon color multiplier", false, [0x3f800000, 0x00000000, 0x41800000]), // 42
    ("Night sky: Zenith color", true, [0x3d888889, 0x3e189899, 0x3e9c9c9d]), // 43
    ("Night sky: Zenith color multiplier", false, [0x3e800000, 0x00000000, 0x41800000]), // 44
    ("Night sky: Zenith shift", false, [0x3e800000, 0x00000000, 0x3f800000]), // 45
    ("Night sky: Star intensity", false, [0x00000000, 0x00000000, 0x40400000]), // 46
    ("Night sky: Moon color", true, [0x3f800000, 0x3f800000, 0x3f800000]), // 47
    ("Night sky: Moon color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 48
    ("Night sky: Moon inner corona color", true, [0x3f66e6e8, 0x3f800000, 0x3f800000]), // 49
    ("Night sky: Moon inner corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 50
    ("Night sky: Moon inner corona scale", false, [0x3eff7cee, 0x00000000, 0x40000000]), // 51
    ("Night sky: Moon outer corona color", true, [0x3f008081, 0x3f48c8ca, 0x3f800000]), // 52
    ("Night sky: Moon outer corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 53
    ("Night sky: Moon outer corona scale", false, [0x3bc49ba6, 0x00000000, 0x40000000]), // 54
    ("Night sky: Moon outer corona size", false, [0x3f000000, 0x00000000, 0x3f800000]), // 55
    ("Night sky: Moon1 color", true, [0x3f800000, 0x3f800000, 0x3f800000]), // 56
    ("Night sky: Moon1 color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 57
    ("Night sky: Moon1 inner corona color", true, [0x3f66e6e7, 0x3f800000, 0x3f800000]), // 58
    ("Night sky: Moon1 inner corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 59
    ("Night sky: Moon1 inner corona scale", false, [0x3eff7cee, 0x00000000, 0x3f800000]), // 60
    ("Night sky: Moon1 outer corona color", true, [0x3f008081, 0x3f48c8c9, 0x3f800000]), // 61
    ("Night sky: Moon1 outer corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 62
    ("Night sky: Moon1 outer corona scale", false, [0x3bc49ba6, 0x00000000, 0x3f800000]), // 63
    ("Night sky: Moon1 outer corona size", false, [0x00000000, 0x00000000, 0x3f800000]), // 64
    ("Night sky: Moon2 color", true, [0x3f800000, 0x3f800000, 0x3f800000]), // 65
    ("Night sky: Moon2 color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 66
    ("Night sky: Moon2 inner corona color", true, [0x3f66e6e7, 0x3f800000, 0x3f800000]), // 67
    ("Night sky: Moon2 inner corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 68
    ("Night sky: Moon2 inner corona scale", false, [0x3eff7cee, 0x00000000, 0x3f800000]), // 69
    ("Night sky: Moon2 outer corona color", true, [0x3f008081, 0x3f48c8c9, 0x3f800000]), // 70
    ("Night sky: Moon2 outer corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 71
    ("Night sky: Moon2 outer corona scale", false, [0x3bc49ba6, 0x00000000, 0x3f800000]), // 72
    ("Night sky: Moon2 outer corona size", false, [0x00000000, 0x00000000, 0x3f800000]), // 73
    ("Night sky: Moon3 color", true, [0x3f800000, 0x3f800000, 0x3f800000]), // 74
    ("Night sky: Moon3 color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 75
    ("Night sky: Moon3 inner corona color", true, [0x3f66e6e7, 0x3f800000, 0x3f800000]), // 76
    ("Night sky: Moon3 inner corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 77
    ("Night sky: Moon3 inner corona scale", false, [0x3eff7cee, 0x00000000, 0x3f800000]), // 78
    ("Night sky: Moon3 outer corona color", true, [0x3f008081, 0x3f48c8c9, 0x3f800000]), // 79
    ("Night sky: Moon3 outer corona color multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 80
    ("Night sky: Moon3 outer corona scale", false, [0x3bc49ba6, 0x00000000, 0x3f800000]), // 81
    ("Night sky: Moon3 outer corona size", false, [0x00000000, 0x00000000, 0x3f800000]), // 82
    ("Cloud shading: Sun light multiplier", false, [0x3ffae148, 0x00000000, 0x41800000]), // 83
    ("Cloud shading: Sky light multiplier", false, [0x3f4ccccd, 0x00000000, 0x41800000]), // 84
    ("Sun shafts visibility", false, [0x3e800000, 0x00000000, 0x3f800000]), // 85
    ("Sun rays visibility", false, [0x40200000, 0x00000000, 0x41200000]), // 86
    ("Sun rays attenuation", false, [0x40a00000, 0x00000000, 0x41200000]), // 87
    ("Ocean fog color", true, [0x3de8e8ea, 0x3eccccce, 0x3f0d8d8e]), // 88
    ("Ocean fog color multiplier", false, [0x3f800000, 0x00000000, 0x3f800000]), // 89
    ("Ocean fog density", false, [0x3e4ccccd, 0x00000000, 0x3f800000]), // 90
    ("Ocean fog density under water", false, [0x3d23d70a, 0x00000000, 0x3f800000]), // 91
    ("Ocean fog under water Scatter", false, [0x3f800000, 0x00000000, 0x3f800000]), // 92
    ("Ocean fog density into water", false, [0x3e4ccccd, 0x00000000, 0x3f800000]), // 93
    ("Ocean fog into water Scatter", false, [0x3f800000, 0x00000000, 0x3f800000]), // 94
    ("Skybox multiplier", false, [0x3f800000, 0x00000000, 0x3f800000]), // 95
    ("Film curve shoulder scale", false, [0x3f800000, 0x00000000, 0x41200000]), // 96
    ("Film curve midtones scale", false, [0x3f800000, 0x00000000, 0x41200000]), // 97
    ("Film curve toe scale", false, [0x3f800000, 0x00000000, 0x41200000]), // 98
    ("Film curve whitepoint", false, [0x3f800000, 0x00000000, 0x41200000]), // 99
    ("Blueshift", true, [0x3f4cccce, 0x3f4cccce, 0x3f800000]), // 100
    ("HDR blueshift threshold", false, [0x3e000000, 0x00000000, 0x3f800000]), // 101
    ("Saturation", false, [0x3f800000, 0x00000000, 0x41200000]), // 102
    ("Contrast", false, [0x3f800000, 0x00000000, 0x41200000]), // 103
    ("Color balance", true, [0x3f800000, 0x3f800000, 0x3f800000]), // 104
    ("Color: saturation", false, [0x3f800000, 0x00000000, 0x41200000]), // 105
    ("Color: contrast", false, [0x3f800000, 0x00000000, 0x41200000]), // 106
    ("Color: brightness", false, [0x3f800000, 0x00000000, 0x41200000]), // 107
    ("Levels: min input", false, [0x00000000, 0x00000000, 0x437f0000]), // 108
    ("Levels: gamma", false, [0x3f800000, 0x00000000, 0x41200000]), // 109
    ("Levels: max input", false, [0x437f0000, 0x00000000, 0x437f0000]), // 110
    ("Levels: min output", false, [0x00000000, 0x00000000, 0x00000000]), // 111
    ("Levels: max output", false, [0x437f0000, 0x00000000, 0x437f0000]), // 112
    ("Selective Color: color", true, [0x00000000, 0x3f800000, 0x3f800000]), // 113
    ("Selective Color: cyans", false, [0x00000000, 0xc2c80000, 0x42c80000]), // 114
    ("Selective Color: magentas", false, [0x00000000, 0xc2c80000, 0x42c80000]), // 115
    ("Selective Color: yellows", false, [0x00000000, 0xc2c80000, 0x42c80000]), // 116
    ("Selective Color: blacks", false, [0x00000000, 0xc2c80000, 0x42c80000]), // 117
    ("Filters: grain", false, [0x00000000, 0x00000000, 0x3f800000]), // 118
    ("Filters: sharpening", false, [0x00000000, 0x00000000, 0x3f800000]), // 119
    ("Filters: photofilter color", true, [0x3f73b646, 0x3f045a1d, 0x3db851ec]), // 120
    ("Filters: photofilter density", false, [0x00000000, 0x00000000, 0x3f800000]), // 121
    ("Dof: focus range", false, [0x447a0000, 0x00000000, 0x461c4000]), // 122
    ("Dof: blur amount", false, [0x00000000, 0x00000000, 0x3f800000]), // 123
    ("Shadow Intensity", false, [0x3f800000, 0x00000000, 0x3f800000]), // 124
    ("Extra Shadow Intensity", false, [0x3f800000, 0x00000000, 0x3f800000]), // 125
    ("Terrain Shadow Intensity", false, [0x3f800000, 0x00000000, 0x3f800000]), // 126
    ("InShadowIntensity", false, [0x3f000000, 0x00000000, 0x3f800000]), // 127
    ("1 Color", true, [0x3f800000, 0x3f800000, 0x3f800000]), // 128
    ("1 Diffuse Multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 129
    ("1 HDR dynamic", false, [0x00000000, 0xc0800000, 0x40800000]), // 130
    ("1 Specular Multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 131
    ("2 Color", true, [0x3f800000, 0x3f800000, 0x3f800000]), // 132
    ("2 Diffuse Multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 133
    ("2 HDR dynamic", false, [0x00000000, 0xc0800000, 0x40800000]), // 134
    ("2 Specular Multiplier", false, [0x00000000, 0x00000000, 0x41800000]), // 135
    ("Char Light Multiplier", false, [0x3f800000, 0x00000000, 0x40400000]), // 136
    ("Char DiffuseMap Saturation", false, [0x3f800000, 0x00000000, 0x40400000]), // 137
    ("Char Saturation in BrightArea", false, [0x00000000, 0x00000000, 0x40400000]), // 138
    ("Char Specular Color Intensity", false, [0x00000000, 0x00000000, 0x40400000]), // 139
    ("BG Light Multiplier", false, [0x3f800000, 0x00000000, 0x40400000]), // 140
    ("BG DiffuseMap Saturation", false, [0x3f800000, 0x00000000, 0x40400000]), // 141
    ("BG Output Balance Distance", false, [0x42c80000, 0x00000000, 0x43fa0000]), // 142
    ("BG Output Saturation in Start", false, [0x3f800000, 0x00000000, 0x40400000]), // 143
    ("BG Output Saturation in End", false, [0x3f800000, 0x00000000, 0x40400000]), // 144
    ("BG Output Contrast in Start", false, [0x3f800000, 0x00000000, 0x40400000]), // 145
    ("BG Output Contrast in End", false, [0x3f800000, 0x00000000, 0x40400000]), // 146
];
