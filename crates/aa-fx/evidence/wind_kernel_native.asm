
# 3504f84e..3504fb94
3504f84e f30f1090a0010000               movss    xmm2, dword ptr [eax + 0x1a0]
3504f856 f30f10358ca21f35               movss    xmm6, dword ptr [0x351fa28c]
3504f85e f30f59d1                       mulss    xmm2, xmm1
3504f862 f30f1088a4010000               movss    xmm1, dword ptr [eax + 0x1a4]
3504f86a f30f59cb                       mulss    xmm1, xmm3
3504f86e f30f1098a8010000               movss    xmm3, dword ptr [eax + 0x1a8]
3504f876 f30f59c9                       mulss    xmm1, xmm1
3504f87a f30f59dc                       mulss    xmm3, xmm4
3504f87e f30f59d2                       mulss    xmm2, xmm2
3504f882 f30f58ca                       addss    xmm1, xmm2
3504f886 f30f59db                       mulss    xmm3, xmm3
3504f88a f30f58cb                       addss    xmm1, xmm3
3504f88e 0f2fce                         comiss   xmm1, xmm6
3504f891 7613                           jbe      0x3504f8a6
3504f893 8b45a8                         mov      eax, dword ptr [ebp - 0x58]
3504f896 5f                             pop      edi
3504f897 89b574ffffff                   mov      dword ptr [ebp - 0x8c], esi
3504f89d 23c6                           and      eax, esi
3504f89f 5e                             pop      esi
3504f8a0 8be5                           mov      esp, ebp
3504f8a2 5d                             pop      ebp
3504f8a3 c22000                         ret      0x20
3504f8a6 0f2ff1                         comiss   xmm6, xmm1
3504f8a9 0f866a160000                   jbe      0x35050f19
3504f8af 83b8bc01000000                 cmp      dword ptr [eax + 0x1bc], 0
3504f8b6 0f8543150000                   jne      0x35050dff
3504f8bc 0f2fc8                         comiss   xmm1, xmm0
3504f8bf 760e                           jbe      0x3504f8cf
3504f8c1 0f57d2                         xorps    xmm2, xmm2
3504f8c4 f30f10d1                       movss    xmm2, xmm1
3504f8c8 f30f51d2                       sqrtss   xmm2, xmm2
3504f8cc 0f28ca                         movaps   xmm1, xmm2
3504f8cf f30f5c88b0010000               subss    xmm1, dword ptr [eax + 0x1b0]
3504f8d7 8bb0ac010000                   mov      esi, dword ptr [eax + 0x1ac]
3504f8dd 0f28d1                         movaps   xmm2, xmm1
3504f8e0 0f5715b0a21f35                 xorps    xmm2, xmmword ptr [0x351fa2b0]
3504f8e7 0f541590a21f35                 andps    xmm2, xmmword ptr [0x351fa290]
3504f8ee f30f58d1                       addss    xmm2, xmm1
3504f8f2 f30f591580a21f35               mulss    xmm2, dword ptr [0x351fa280]
3504f8fa f30f5990b4010000               mulss    xmm2, dword ptr [eax + 0x1b4]
3504f902 0f28de                         movaps   xmm3, xmm6
3504f905 f30f5cda                       subss    xmm3, xmm2
3504f909 f30f115d0c                     movss    dword ptr [ebp + 0xc], xmm3
3504f90e 85f6                           test     esi, esi
3504f910 0f842c010000                   je       0x3504fa42
3504f916 8bb888010000                   mov      edi, dword ptr [eax + 0x188]
3504f91c 81e70000a0ff                   and      edi, 0xffa00000
3504f922 81ff0000a0ff                   cmp      edi, 0xffa00000
3504f928 0f8466020000                   je       0x3504fb94
3504f92e 83fe02                         cmp      esi, 2
3504f931 0f85b8000000                   jne      0x3504f9ef
3504f937 f30f10b088010000               movss    xmm6, dword ptr [eax + 0x188]
3504f93f f30f10a88c010000               movss    xmm5, dword ptr [eax + 0x18c]
3504f947 f30f10b890010000               movss    xmm7, dword ptr [eax + 0x190]
3504f94f 0f28cd                         movaps   xmm1, xmm5
3504f952 f30f594864                     mulss    xmm1, dword ptr [eax + 0x64]
3504f957 0f28e6                         movaps   xmm4, xmm6
3504f95a f30f596060                     mulss    xmm4, dword ptr [eax + 0x60]
3504f95f f30f58e1                       addss    xmm4, xmm1
3504f963 0f28d5                         movaps   xmm2, xmm5
3504f966 f30f595070                     mulss    xmm2, dword ptr [eax + 0x70]
3504f96b f30f59687c                     mulss    xmm5, dword ptr [eax + 0x7c]
3504f970 0f28cf                         movaps   xmm1, xmm7
3504f973 f30f594868                     mulss    xmm1, dword ptr [eax + 0x68]
3504f978 f30f58e1                       addss    xmm4, xmm1
3504f97c f30f10486c                     movss    xmm1, dword ptr [eax + 0x6c]
3504f981 f30f59ce                       mulss    xmm1, xmm6
3504f985 f30f58ca                       addss    xmm1, xmm2
3504f989 f30f105074                     movss    xmm2, dword ptr [eax + 0x74]
3504f98e f30f59d7                       mulss    xmm2, xmm7
3504f992 f30f58ca                       addss    xmm1, xmm2
3504f996 f30f105078                     movss    xmm2, dword ptr [eax + 0x78]
3504f99b f30f59d6                       mulss    xmm2, xmm6
3504f99f f30f10358ca21f35               movss    xmm6, dword ptr [0x351fa28c]
3504f9a7 f30f58d5                       addss    xmm2, xmm5
3504f9ab f30f10a880000000               movss    xmm5, dword ptr [eax + 0x80]
3504f9b3 f30f59ef                       mulss    xmm5, xmm7
3504f9b7 f30f58d5                       addss    xmm2, xmm5
3504f9bb f30f59cb                       mulss    xmm1, xmm3
3504f9bf f30f584904                     addss    xmm1, dword ptr [ecx + 4]
3504f9c4 f30f114904                     movss    dword ptr [ecx + 4], xmm1
3504f9c9 f30f104908                     movss    xmm1, dword ptr [ecx + 8]
3504f9ce 0f28eb                         movaps   xmm5, xmm3
3504f9d1 f30f59ec                       mulss    xmm5, xmm4
3504f9d5 f30f5829                       addss    xmm5, dword ptr [ecx]
3504f9d9 f30f59d3                       mulss    xmm2, xmm3
3504f9dd f30f58ca                       addss    xmm1, xmm2
3504f9e1 f30f1129                       movss    dword ptr [ecx], xmm5
3504f9e5 f30f114908                     movss    dword ptr [ecx + 8], xmm1
3504f9ea e9a5010000                     jmp      0x3504fb94
3504f9ef f30f7e8888010000               movq     xmm1, qword ptr [eax + 0x188]
3504f9f7 8bb090010000                   mov      esi, dword ptr [eax + 0x190]
3504f9fd 660fd64df4                     movq     qword ptr [ebp - 0xc], xmm1
3504fa02 0f28cb                         movaps   xmm1, xmm3
3504fa05 f30f594df4                     mulss    xmm1, dword ptr [ebp - 0xc]
3504fa0a f30f5809                       addss    xmm1, dword ptr [ecx]
3504fa0e f30f1109                       movss    dword ptr [ecx], xmm1
3504fa12 f30f104908                     movss    xmm1, dword ptr [ecx + 8]
3504fa17 8975fc                         mov      dword ptr [ebp - 4], esi
3504fa1a 0f28d3                         movaps   xmm2, xmm3
3504fa1d f30f5955f8                     mulss    xmm2, dword ptr [ebp - 8]
3504fa22 f30f585104                     addss    xmm2, dword ptr [ecx + 4]
3504fa27 0f28e3                         movaps   xmm4, xmm3
3504fa2a f30f5965fc                     mulss    xmm4, dword ptr [ebp - 4]
3504fa2f f30f58cc                       addss    xmm1, xmm4
3504fa33 f30f115104                     movss    dword ptr [ecx + 4], xmm2
3504fa38 f30f114908                     movss    dword ptr [ecx + 8], xmm1
3504fa3d e952010000                     jmp      0x3504fb94
3504fa42 8b705c                         mov      esi, dword ptr [eax + 0x5c]
3504fa45 f30f7e4854                     movq     xmm1, qword ptr [eax + 0x54]
3504fa4a 660fd64df4                     movq     qword ptr [ebp - 0xc], xmm1
3504fa4f 8975fc                         mov      dword ptr [ebp - 4], esi
3504fa52 8b7508                         mov      esi, dword ptr [ebp + 8]
3504fa55 f30f1016                       movss    xmm2, dword ptr [esi]
3504fa59 f30f5c55f4                     subss    xmm2, dword ptr [ebp - 0xc]
3504fa5e f30f106604                     movss    xmm4, dword ptr [esi + 4]
3504fa63 f30f5c65f8                     subss    xmm4, dword ptr [ebp - 8]
3504fa68 f30f106e08                     movss    xmm5, dword ptr [esi + 8]
3504fa6d f30f5c6dfc                     subss    xmm5, dword ptr [ebp - 4]
3504fa72 0f28ca                         movaps   xmm1, xmm2
3504fa75 f30f59ca                       mulss    xmm1, xmm2
3504fa79 0f28fc                         movaps   xmm7, xmm4
3504fa7c f30f59fc                       mulss    xmm7, xmm4
3504fa80 f30f58cf                       addss    xmm1, xmm7
3504fa84 0f28fd                         movaps   xmm7, xmm5
3504fa87 f30f59fd                       mulss    xmm7, xmm5
3504fa8b f30f58cf                       addss    xmm1, xmm7
3504fa8f 0f2f0da0a21f35                 comiss   xmm1, dword ptr [0x351fa2a0]
3504fa96 7647                           jbe      0x3504fadf
3504fa98 0f57f6                         xorps    xmm6, xmm6
3504fa9b f30f10f1                       movss    xmm6, xmm1
3504fa9f f30f52f6                       rsqrtss  xmm6, xmm6
3504faa3 0f28fe                         movaps   xmm7, xmm6
3504faa6 f30f59f9                       mulss    xmm7, xmm1
3504faaa f30f100d84a21f35               movss    xmm1, dword ptr [0x351fa284]
3504fab2 f30f59fe                       mulss    xmm7, xmm6
3504fab6 f30f593d80a21f35               mulss    xmm7, dword ptr [0x351fa280]
3504fabe f30f5ccf                       subss    xmm1, xmm7
3504fac2 f30f59ce                       mulss    xmm1, xmm6
3504fac6 f30f10358ca21f35               movss    xmm6, dword ptr [0x351fa28c]
3504face f30f59d1                       mulss    xmm2, xmm1
3504fad2 0f28fa                         movaps   xmm7, xmm2
3504fad5 f30f59e1                       mulss    xmm4, xmm1
3504fad9 f30f59e9                       mulss    xmm5, xmm1
3504fadd eb09                           jmp      0x3504fae8
3504fadf 0f28f8                         movaps   xmm7, xmm0
3504fae2 0f28e0                         movaps   xmm4, xmm0
3504fae5 0f28ee                         movaps   xmm5, xmm6
3504fae8 8bb088010000                   mov      esi, dword ptr [eax + 0x188]
3504faee 81e60000a0ff                   and      esi, 0xffa00000
3504faf4 f30f116dfc                     movss    dword ptr [ebp - 4], xmm5
3504faf9 81fe0000a0ff                   cmp      esi, 0xffa00000
3504faff 744b                           je       0x3504fb4c
3504fb01 f30f108890010000               movss    xmm1, dword ptr [eax + 0x190]
3504fb09 f30f59cb                       mulss    xmm1, xmm3
3504fb0d f30f59e9                       mulss    xmm5, xmm1
3504fb11 0f28f7                         movaps   xmm6, xmm7
3504fb14 f30f59f1                       mulss    xmm6, xmm1
3504fb18 f30f5831                       addss    xmm6, dword ptr [ecx]
3504fb1c 0f28d4                         movaps   xmm2, xmm4
3504fb1f f30f59d1                       mulss    xmm2, xmm1
3504fb23 f30f104908                     movss    xmm1, dword ptr [ecx + 8]
3504fb28 f30f585104                     addss    xmm2, dword ptr [ecx + 4]
3504fb2d f30f58cd                       addss    xmm1, xmm5
3504fb31 f30f106dfc                     movss    xmm5, dword ptr [ebp - 4]
3504fb36 f30f1131                       movss    dword ptr [ecx], xmm6
3504fb3a f30f10358ca21f35               movss    xmm6, dword ptr [0x351fa28c]
3504fb42 f30f115104                     movss    dword ptr [ecx + 4], xmm2
3504fb47 f30f114908                     movss    dword ptr [ecx + 8], xmm1
3504fb4c 837da800                       cmp      dword ptr [ebp - 0x58], 0
3504fb50 0f8424120000                   je       0x35050d7a
3504fb56 f30f10885c010000               movss    xmm1, dword ptr [eax + 0x15c]
3504fb5e 8b7514                         mov      esi, dword ptr [ebp + 0x14]
3504fb61 8bca                           mov      ecx, edx
3504fb63 6bc94c                         imul     ecx, ecx, 0x4c
3504fb66 f30f59f9                       mulss    xmm7, xmm1
3504fb6a f30f59e1                       mulss    xmm4, xmm1
3504fb6e f30f59e9                       mulss    xmm5, xmm1
3504fb72 8d4c3118                       lea      ecx, [ecx + esi + 0x18]
3504fb76 f30f117df4                     movss    dword ptr [ebp - 0xc], xmm7
3504fb7b f30f1165f8                     movss    dword ptr [ebp - 8], xmm4
3504fb80 f30f7e4df4                     movq     xmm1, qword ptr [ebp - 0xc]
3504fb85 f30f116dfc                     movss    dword ptr [ebp - 4], xmm5
3504fb8a 8b75fc                         mov      esi, dword ptr [ebp - 4]
3504fb8d 660fd609                       movq     qword ptr [ecx], xmm1
3504fb91 897108                         mov      dword ptr [ecx + 8], esi

# 350581e0..35059090
350581e0 55                             push     ebp
350581e1 8bec                           mov      ebp, esp
350581e3 81ec84000000                   sub      esp, 0x84
350581e9 56                             push     esi
350581ea 57                             push     edi
350581eb 8b7d08                         mov      edi, dword ptr [ebp + 8]
350581ee 8b07                           mov      eax, dword ptr [edi]
350581f0 8bf1                           mov      esi, ecx
350581f2 8975d4                         mov      dword ptr [ebp - 0x2c], esi
350581f5 83f801                         cmp      eax, 1
350581f8 0f852f040000                   jne      0x3505862d
350581fe 33c9                           xor      ecx, ecx
35058200 894f08                         mov      dword ptr [edi + 8], ecx
35058203 894f04                         mov      dword ptr [edi + 4], ecx
35058206 894f14                         mov      dword ptr [edi + 0x14], ecx
35058209 894f10                         mov      dword ptr [edi + 0x10], ecx
3505820c 894f0c                         mov      dword ptr [edi + 0xc], ecx
3505820f f30f7e4654                     movq     xmm0, qword ptr [esi + 0x54]
35058214 660fd64718                     movq     qword ptr [edi + 0x18], xmm0
35058219 8b465c                         mov      eax, dword ptr [esi + 0x5c]
3505821c 894720                         mov      dword ptr [edi + 0x20], eax
3505821f f30f7e4654                     movq     xmm0, qword ptr [esi + 0x54]
35058224 8b565c                         mov      edx, dword ptr [esi + 0x5c]
35058227 660fd645f0                     movq     qword ptr [ebp - 0x10], xmm0
3505822c f30f104604                     movss    xmm0, dword ptr [esi + 4]
35058231 f30f5c45f0                     subss    xmm0, dword ptr [ebp - 0x10]
35058236 f30f1145dc                     movss    dword ptr [ebp - 0x24], xmm0
3505823b f30f104608                     movss    xmm0, dword ptr [esi + 8]
35058240 f30f5c45f4                     subss    xmm0, dword ptr [ebp - 0xc]
35058245 f30f1145e0                     movss    dword ptr [ebp - 0x20], xmm0
3505824a f30f10460c                     movss    xmm0, dword ptr [esi + 0xc]
3505824f 0f57e4                         xorps    xmm4, xmm4
35058252 8955f8                         mov      dword ptr [ebp - 8], edx
35058255 f30f5c45f8                     subss    xmm0, dword ptr [ebp - 8]
3505825a f30f1145e4                     movss    dword ptr [ebp - 0x1c], xmm0
3505825f f30f7e45dc                     movq     xmm0, qword ptr [ebp - 0x24]
35058264 8b45e4                         mov      eax, dword ptr [ebp - 0x1c]
35058267 660fd64724                     movq     qword ptr [edi + 0x24], xmm0
3505826c 89472c                         mov      dword ptr [edi + 0x2c], eax
3505826f f30f7e4654                     movq     xmm0, qword ptr [esi + 0x54]
35058274 8b565c                         mov      edx, dword ptr [esi + 0x5c]
35058277 660fd645f0                     movq     qword ptr [ebp - 0x10], xmm0
3505827c f30f104610                     movss    xmm0, dword ptr [esi + 0x10]
35058281 f30f5c45f0                     subss    xmm0, dword ptr [ebp - 0x10]
35058286 f30f1145dc                     movss    dword ptr [ebp - 0x24], xmm0
3505828b f30f104614                     movss    xmm0, dword ptr [esi + 0x14]
35058290 f30f5c45f4                     subss    xmm0, dword ptr [ebp - 0xc]
35058295 f30f1145e0                     movss    dword ptr [ebp - 0x20], xmm0
3505829a f30f104618                     movss    xmm0, dword ptr [esi + 0x18]
3505829f 8955f8                         mov      dword ptr [ebp - 8], edx
350582a2 f30f5c45f8                     subss    xmm0, dword ptr [ebp - 8]
350582a7 f30f1145e4                     movss    dword ptr [ebp - 0x1c], xmm0
350582ac f30f7e45dc                     movq     xmm0, qword ptr [ebp - 0x24]
350582b1 8b45e4                         mov      eax, dword ptr [ebp - 0x1c]
350582b4 660fd64730                     movq     qword ptr [edi + 0x30], xmm0
350582b9 894738                         mov      dword ptr [edi + 0x38], eax
350582bc f30f104e60                     movss    xmm1, dword ptr [esi + 0x60]
350582c1 f30f104670                     movss    xmm0, dword ptr [esi + 0x70]
350582c6 f30f109680000000               movss    xmm2, dword ptr [esi + 0x80]
350582ce 0f28d9                         movaps   xmm3, xmm1
350582d1 f30f58d8                       addss    xmm3, xmm0
350582d5 f30f58da                       addss    xmm3, xmm2
350582d9 0f2fdc                         comiss   xmm3, xmm4
350582dc 7656                           jbe      0x35058334
350582de f30f581d8ca21f35               addss    xmm3, dword ptr [0x351fa28c]
350582e6 f30f102580a21f35               movss    xmm4, dword ptr [0x351fa280]
350582ee f30f104e7c                     movss    xmm1, dword ptr [esi + 0x7c]
350582f3 f30f105668                     movss    xmm2, dword ptr [esi + 0x68]
350582f8 f30f5c4e74                     subss    xmm1, dword ptr [esi + 0x74]
350582fd f30f5c5678                     subss    xmm2, dword ptr [esi + 0x78]
35058302 0f57c0                         xorps    xmm0, xmm0
35058305 f30f10c3                       movss    xmm0, xmm3
35058309 f30f105e6c                     movss    xmm3, dword ptr [esi + 0x6c]
3505830e f30f5c5e64                     subss    xmm3, dword ptr [esi + 0x64]
35058313 f30f51c0                       sqrtss   xmm0, xmm0
35058317 f30f5ee0                       divss    xmm4, xmm0
3505831b f30f590580a21f35               mulss    xmm0, dword ptr [0x351fa280]
35058323 f30f59cc                       mulss    xmm1, xmm4
35058327 f30f59d4                       mulss    xmm2, xmm4
3505832b f30f59dc                       mulss    xmm3, xmm4
3505832f e94c010000                     jmp      0x35058480
35058334 0f2fc8                         comiss   xmm1, xmm0
35058337 7266                           jb       0x3505839f
35058339 0f2fca                         comiss   xmm1, xmm2
3505833c 7261                           jb       0x3505839f
3505833e f30f102580a21f35               movss    xmm4, dword ptr [0x351fa280]
35058346 f30f105e78                     movss    xmm3, dword ptr [esi + 0x78]
3505834b f30f585e68                     addss    xmm3, dword ptr [esi + 0x68]
35058350 f30f5cc8                       subss    xmm1, xmm0
35058354 f30f5cca                       subss    xmm1, xmm2
35058358 f30f580d8ca21f35               addss    xmm1, dword ptr [0x351fa28c]
35058360 f30f10566c                     movss    xmm2, dword ptr [esi + 0x6c]
35058365 f30f585664                     addss    xmm2, dword ptr [esi + 0x64]
3505836a 0f57c0                         xorps    xmm0, xmm0
3505836d f30f10c1                       movss    xmm0, xmm1
35058371 f30f51c0                       sqrtss   xmm0, xmm0
35058375 0f28c8                         movaps   xmm1, xmm0
35058378 f30f10467c                     movss    xmm0, dword ptr [esi + 0x7c]
3505837d f30f5c4674                     subss    xmm0, dword ptr [esi + 0x74]
35058382 f30f5ee1                       divss    xmm4, xmm1
35058386 f30f590d80a21f35               mulss    xmm1, dword ptr [0x351fa280]
3505838e f30f59d4                       mulss    xmm2, xmm4
35058392 f30f59dc                       mulss    xmm3, xmm4
35058396 f30f59c4                       mulss    xmm0, xmm4
3505839a e9e1000000                     jmp      0x35058480
3505839f 0f2fc1                         comiss   xmm0, xmm1
350583a2 7263                           jb       0x35058407
350583a4 0f2fc2                         comiss   xmm0, xmm2
350583a7 725e                           jb       0x35058407
350583a9 f30f102580a21f35               movss    xmm4, dword ptr [0x351fa280]
350583b1 f30f105e7c                     movss    xmm3, dword ptr [esi + 0x7c]
350583b6 f30f585e74                     addss    xmm3, dword ptr [esi + 0x74]
350583bb f30f5cc2                       subss    xmm0, xmm2
350583bf f30f5cc1                       subss    xmm0, xmm1
350583c3 f30f58058ca21f35               addss    xmm0, dword ptr [0x351fa28c]
350583cb 0f57c9                         xorps    xmm1, xmm1
350583ce f30f10c8                       movss    xmm1, xmm0
350583d2 f30f104668                     movss    xmm0, dword ptr [esi + 0x68]
350583d7 f30f5c4678                     subss    xmm0, dword ptr [esi + 0x78]
350583dc f30f51c9                       sqrtss   xmm1, xmm1
350583e0 0f28d1                         movaps   xmm2, xmm1
350583e3 f30f104e6c                     movss    xmm1, dword ptr [esi + 0x6c]
350583e8 f30f584e64                     addss    xmm1, dword ptr [esi + 0x64]
350583ed f30f5ee2                       divss    xmm4, xmm2
350583f1 f30f591580a21f35               mulss    xmm2, dword ptr [0x351fa280]
350583f9 f30f59cc                       mulss    xmm1, xmm4
350583fd f30f59dc                       mulss    xmm3, xmm4
35058401 f30f59c4                       mulss    xmm0, xmm4
35058405 eb79                           jmp      0x35058480
35058407 0f2fd1                         comiss   xmm2, xmm1
3505840a 7263                           jb       0x3505846f
3505840c 0f2fd0                         comiss   xmm2, xmm0
3505840f 725e                           jb       0x3505846f
35058411 f30f102580a21f35               movss    xmm4, dword ptr [0x351fa280]
35058419 f30f5cd1                       subss    xmm2, xmm1
3505841d f30f104e78                     movss    xmm1, dword ptr [esi + 0x78]
35058422 f30f584e68                     addss    xmm1, dword ptr [esi + 0x68]
35058427 f30f5cd0                       subss    xmm2, xmm0
3505842b f30f58158ca21f35               addss    xmm2, dword ptr [0x351fa28c]
35058433 0f57c0                         xorps    xmm0, xmm0
35058436 f30f10c2                       movss    xmm0, xmm2
3505843a f30f10567c                     movss    xmm2, dword ptr [esi + 0x7c]
3505843f f30f585674                     addss    xmm2, dword ptr [esi + 0x74]
35058444 f30f51c0                       sqrtss   xmm0, xmm0
35058448 0f28d8                         movaps   xmm3, xmm0
3505844b f30f10466c                     movss    xmm0, dword ptr [esi + 0x6c]
35058450 f30f5c4664                     subss    xmm0, dword ptr [esi + 0x64]
35058455 f30f5ee3                       divss    xmm4, xmm3
35058459 f30f591d80a21f35               mulss    xmm3, dword ptr [0x351fa280]
35058461 f30f59cc                       mulss    xmm1, xmm4
35058465 f30f59d4                       mulss    xmm2, xmm4
35058469 f30f59c4                       mulss    xmm0, xmm4
3505846d eb11                           jmp      0x35058480
3505846f f30f10058ca21f35               movss    xmm0, dword ptr [0x351fa28c]
35058477 0f28cc                         movaps   xmm1, xmm4
3505847a 0f28d4                         movaps   xmm2, xmm4
3505847d 0f28dc                         movaps   xmm3, xmm4
35058480 f30f114f3c                     movss    dword ptr [edi + 0x3c], xmm1
35058485 f30f115740                     movss    dword ptr [edi + 0x40], xmm2
3505848a f30f115f44                     movss    dword ptr [edi + 0x44], xmm3
3505848f f30f114748                     movss    dword ptr [edi + 0x48], xmm0
35058494 d98684000000                   fld      dword ptr [esi + 0x84]
3505849a 8b4754                         mov      eax, dword ptr [edi + 0x54]
3505849d d95f4c                         fstp     dword ptr [edi + 0x4c]
350584a0 0fbe563b                       movsx    edx, byte ptr [esi + 0x3b]
350584a4 895750                         mov      dword ptr [edi + 0x50], edx
350584a7 3bc1                           cmp      eax, ecx
350584a9 0f84bf000000                   je       0x3505856e
350584af f30f108684000000               movss    xmm0, dword ptr [esi + 0x84]
350584b7 f30f107e78                     movss    xmm7, dword ptr [esi + 0x78]
350584bc f30f105664                     movss    xmm2, dword ptr [esi + 0x64]
350584c1 f30f105e68                     movss    xmm3, dword ptr [esi + 0x68]
350584c6 f30f10666c                     movss    xmm4, dword ptr [esi + 0x6c]
350584cb f30f106e70                     movss    xmm5, dword ptr [esi + 0x70]
350584d0 f30f107674                     movss    xmm6, dword ptr [esi + 0x74]
350584d5 f30f59f8                       mulss    xmm7, xmm0
350584d9 f30f117dc8                     movss    dword ptr [ebp - 0x38], xmm7
350584de f30f107e7c                     movss    xmm7, dword ptr [esi + 0x7c]
350584e3 f30f59f8                       mulss    xmm7, xmm0
350584e7 f30f117dcc                     movss    dword ptr [ebp - 0x34], xmm7
350584ec f30f10be80000000               movss    xmm7, dword ptr [esi + 0x80]
350584f4 0f28c8                         movaps   xmm1, xmm0
350584f7 f30f594e60                     mulss    xmm1, dword ptr [esi + 0x60]
350584fc f30f1108                       movss    dword ptr [eax], xmm1
35058500 f30f104dc8                     movss    xmm1, dword ptr [ebp - 0x38]
35058505 f30f114820                     movss    dword ptr [eax + 0x20], xmm1
3505850a f30f104dcc                     movss    xmm1, dword ptr [ebp - 0x34]
3505850f f30f59d0                       mulss    xmm2, xmm0
35058513 f30f59d8                       mulss    xmm3, xmm0
35058517 f30f59e0                       mulss    xmm4, xmm0
3505851b f30f59e8                       mulss    xmm5, xmm0
3505851f f30f59f0                       mulss    xmm6, xmm0
35058523 f30f59f8                       mulss    xmm7, xmm0
35058527 0f57c0                         xorps    xmm0, xmm0
3505852a f30f11400c                     movss    dword ptr [eax + 0xc], xmm0
3505852f f30f11401c                     movss    dword ptr [eax + 0x1c], xmm0
35058534 f30f11402c                     movss    dword ptr [eax + 0x2c], xmm0
35058539 f30f115004                     movss    dword ptr [eax + 4], xmm2
3505853e f30f115808                     movss    dword ptr [eax + 8], xmm3
35058543 f30f116010                     movss    dword ptr [eax + 0x10], xmm4
35058548 f30f116814                     movss    dword ptr [eax + 0x14], xmm5
3505854d f30f117018                     movss    dword ptr [eax + 0x18], xmm6
35058552 f30f114824                     movss    dword ptr [eax + 0x24], xmm1
35058557 f30f117828                     movss    dword ptr [eax + 0x28], xmm7
3505855c d94654                         fld      dword ptr [esi + 0x54]
3505855f d9580c                         fstp     dword ptr [eax + 0xc]
35058562 d94658                         fld      dword ptr [esi + 0x58]
35058565 d9581c                         fstp     dword ptr [eax + 0x1c]
35058568 d9465c                         fld      dword ptr [esi + 0x5c]
3505856b d9582c                         fstp     dword ptr [eax + 0x2c]
3505856e 8b4758                         mov      eax, dword ptr [edi + 0x58]
35058571 3bc1                           cmp      eax, ecx
35058573 0f849b000000                   je       0x35058614
35058579 f30f108684000000               movss    xmm0, dword ptr [esi + 0x84]
35058581 f30f107e78                     movss    xmm7, dword ptr [esi + 0x78]
35058586 f30f105664                     movss    xmm2, dword ptr [esi + 0x64]
3505858b f30f105e68                     movss    xmm3, dword ptr [esi + 0x68]
35058590 f30f10666c                     movss    xmm4, dword ptr [esi + 0x6c]
35058595 f30f106e70                     movss    xmm5, dword ptr [esi + 0x70]
3505859a f30f107674                     movss    xmm6, dword ptr [esi + 0x74]
3505859f f30f59f8                       mulss    xmm7, xmm0
350585a3 f30f117dc8                     movss    dword ptr [ebp - 0x38], xmm7
350585a8 f30f107e7c                     movss    xmm7, dword ptr [esi + 0x7c]
350585ad f30f59f8                       mulss    xmm7, xmm0
350585b1 f30f117dcc                     movss    dword ptr [ebp - 0x34], xmm7
350585b6 f30f10be80000000               movss    xmm7, dword ptr [esi + 0x80]
350585be 0f28c8                         movaps   xmm1, xmm0
350585c1 f30f594e60                     mulss    xmm1, dword ptr [esi + 0x60]
350585c6 f30f59d0                       mulss    xmm2, xmm0
350585ca f30f59d8                       mulss    xmm3, xmm0
350585ce f30f59e0                       mulss    xmm4, xmm0
350585d2 f30f59e8                       mulss    xmm5, xmm0
350585d6 f30f59f0                       mulss    xmm6, xmm0
350585da f30f59f8                       mulss    xmm7, xmm0
350585de f30f1045c8                     movss    xmm0, dword ptr [ebp - 0x38]
350585e3 f30f114018                     movss    dword ptr [eax + 0x18], xmm0
350585e8 f30f1045cc                     movss    xmm0, dword ptr [ebp - 0x34]
350585ed f30f1108                       movss    dword ptr [eax], xmm1
350585f1 f30f115004                     movss    dword ptr [eax + 4], xmm2
350585f6 f30f115808                     movss    dword ptr [eax + 8], xmm3
350585fb f30f11600c                     movss    dword ptr [eax + 0xc], xmm4
35058600 f30f116810                     movss    dword ptr [eax + 0x10], xmm5
35058605 f30f117014                     movss    dword ptr [eax + 0x14], xmm6
3505860a f30f11401c                     movss    dword ptr [eax + 0x1c], xmm0
3505860f f30f117820                     movss    dword ptr [eax + 0x20], xmm7
35058614 8bb6bc000000                   mov      esi, dword ptr [esi + 0xbc]
3505861a 897760                         mov      dword ptr [edi + 0x60], esi
3505861d 89775c                         mov      dword ptr [edi + 0x5c], esi
35058620 5f                             pop      edi
35058621 b801000000                     mov      eax, 1
35058626 5e                             pop      esi
35058627 8be5                           mov      esp, ebp
35058629 5d                             pop      ebp
3505862a c20400                         ret      4
3505862d 83f80d                         cmp      eax, 0xd
35058630 7525                           jne      0x35058657
35058632 8b4650                         mov      eax, dword ptr [esi + 0x50]
35058635 3bb0a44f0000                   cmp      esi, dword ptr [eax + 0x4fa4]
3505863b 74e3                           je       0x35058620
3505863d 0f57c0                         xorps    xmm0, xmm0
35058640 51                             push     ecx
35058641 83c704                         add      edi, 4
35058644 f30f110424                     movss    dword ptr [esp], xmm0
35058649 57                             push     edi
3505864a e8e188ffff                     call     0x35050f30
3505864f 5f                             pop      edi
35058650 5e                             pop      esi
35058651 8be5                           mov      esp, ebp
35058653 5d                             pop      ebp
35058654 c20400                         ret      4
35058657 83f817                         cmp      eax, 0x17
3505865a 0f85c1080000                   jne      0x35058f21
35058660 0f57e4                         xorps    xmm4, xmm4
35058663 8d8ec0010000                   lea      ecx, [esi + 0x1c0]
35058669 898f84000000                   mov      dword ptr [edi + 0x84], ecx
3505866f 83beac01000000                 cmp      dword ptr [esi + 0x1ac], 0
35058676 f30f100590a21f35               movss    xmm0, dword ptr [0x351fa290]
3505867e 7447                           je       0x350586c7
35058680 f30f108ea0010000               movss    xmm1, dword ptr [esi + 0x1a0]
35058688 0f54c8                         andps    xmm1, xmm0
3505868b 0f2fe1                         comiss   xmm4, xmm1
3505868e 7220                           jb       0x350586b0
35058690 f30f108ea4010000               movss    xmm1, dword ptr [esi + 0x1a4]
35058698 0f54c8                         andps    xmm1, xmm0
3505869b 0f2fe1                         comiss   xmm4, xmm1
3505869e 7210                           jb       0x350586b0
350586a0 f30f108ea8010000               movss    xmm1, dword ptr [esi + 0x1a8]
350586a8 0f54c8                         andps    xmm1, xmm0
350586ab 0f2fe1                         comiss   xmm4, xmm1
350586ae 7311                           jae      0x350586c1
350586b0 f30f108eb4010000               movss    xmm1, dword ptr [esi + 0x1b4]
350586b8 0f2ecc                         ucomiss  xmm1, xmm4
350586bb 9f                             lahf
350586bc f6c444                         test     ah, 0x44
350586bf 7a06                           jp       0x350586c7
350586c1 c645ef01                       mov      byte ptr [ebp - 0x11], 1
350586c5 eb04                           jmp      0x350586cb
350586c7 c645ef00                       mov      byte ptr [ebp - 0x11], 0
350586cb 8b5650                         mov      edx, dword ptr [esi + 0x50]
350586ce 39b2a44f0000                   cmp      dword ptr [edx + 0x4fa4], esi
350586d4 7419                           je       0x350586ef
350586d6 83bebc00000000                 cmp      dword ptr [esi + 0xbc], 0
350586dd 7509                           jne      0x350586e8
350586df 83bec400000000                 cmp      dword ptr [esi + 0xc4], 0
350586e6 7407                           je       0x350586ef
350586e8 32d2                           xor      dl, dl
350586ea 8855ff                         mov      byte ptr [ebp - 1], dl
350586ed eb07                           jmp      0x350586f6
350586ef c645ff01                       mov      byte ptr [ebp - 1], 1
350586f3 8a55ff                         mov      dl, byte ptr [ebp - 1]
350586f6 f30f104f10                     movss    xmm1, dword ptr [edi + 0x10]
350586fb 0f54c8                         andps    xmm1, xmm0
350586fe 0f2fe1                         comiss   xmm4, xmm1
35058701 721e                           jb       0x35058721
35058703 f30f104f14                     movss    xmm1, dword ptr [edi + 0x14]
35058708 0f54c8                         andps    xmm1, xmm0
3505870b 0f2fe1                         comiss   xmm4, xmm1
3505870e 7211                           jb       0x35058721
35058710 f30f104f18                     movss    xmm1, dword ptr [edi + 0x18]
35058715 0f54c8                         andps    xmm1, xmm0
35058718 0f2fe1                         comiss   xmm4, xmm1
3505871b 0f83ae050000                   jae      0x35058ccf
35058721 83be8401000000                 cmp      dword ptr [esi + 0x184], 0
35058728 f30f7e4710                     movq     xmm0, qword ptr [edi + 0x10]
3505872d 8b4718                         mov      eax, dword ptr [edi + 0x18]
35058730 f30f104f08                     movss    xmm1, dword ptr [edi + 8]
35058735 f30f10570c                     movss    xmm2, dword ptr [edi + 0xc]
3505873a f30f106f10                     movss    xmm5, dword ptr [edi + 0x10]
3505873f f30f107708                     movss    xmm6, dword ptr [edi + 8]
35058744 f30f105f0c                     movss    xmm3, dword ptr [edi + 0xc]
35058749 f30f586f04                     addss    xmm5, dword ptr [edi + 4]
3505874e f30f587714                     addss    xmm6, dword ptr [edi + 0x14]
35058753 f30f585f18                     addss    xmm3, dword ptr [edi + 0x18]
35058758 660fd645f0                     movq     qword ptr [ebp - 0x10], xmm0
3505875d f30f104704                     movss    xmm0, dword ptr [edi + 4]
35058762 f30f5c45f0                     subss    xmm0, dword ptr [ebp - 0x10]
35058767 f30f5c4df4                     subss    xmm1, dword ptr [ebp - 0xc]
3505876c 8945f8                         mov      dword ptr [ebp - 8], eax
3505876f f30f5c55f8                     subss    xmm2, dword ptr [ebp - 8]
35058774 f30f1145bc                     movss    dword ptr [ebp - 0x44], xmm0
35058779 f30f114dc0                     movss    dword ptr [ebp - 0x40], xmm1
3505877e f30f1155c4                     movss    dword ptr [ebp - 0x3c], xmm2
35058783 f30f116dc8                     movss    dword ptr [ebp - 0x38], xmm5
35058788 f30f1175cc                     movss    dword ptr [ebp - 0x34], xmm6
3505878d f30f115dd0                     movss    dword ptr [ebp - 0x30], xmm3
35058792 7542                           jne      0x350587d6
35058794 8b8e70010000                   mov      ecx, dword ptr [esi + 0x170]
3505879a 81e10000a0ff                   and      ecx, 0xffa00000
350587a0 81f90000a0ff                   cmp      ecx, 0xffa00000
350587a6 742e                           je       0x350587d6
350587a8 84d2                           test     dl, dl
350587aa 755e                           jne      0x3505880a
350587ac 0f2f4610                       comiss   xmm0, dword ptr [esi + 0x10]
350587b0 771a                           ja       0x350587cc
350587b2 f30f105604                     movss    xmm2, dword ptr [esi + 4]
350587b7 0f2fd5                         comiss   xmm2, xmm5
350587ba 7710                           ja       0x350587cc
350587bc 0f2f4e14                       comiss   xmm1, dword ptr [esi + 0x14]
350587c0 770a                           ja       0x350587cc
350587c2 f30f105608                     movss    xmm2, dword ptr [esi + 8]
350587c7 0f2fd6                         comiss   xmm2, xmm6
350587ca 763e                           jbe      0x3505880a
350587cc 5f                             pop      edi
350587cd 33c0                           xor      eax, eax
350587cf 5e                             pop      esi
350587d0 8be5                           mov      esp, ebp
350587d2 5d                             pop      ebp
350587d3 c20400                         ret      4
350587d6 84d2                           test     dl, dl
350587d8 7530                           jne      0x3505880a
350587da 0f2f4610                       comiss   xmm0, dword ptr [esi + 0x10]
350587de 77ec                           ja       0x350587cc
350587e0 f30f107e04                     movss    xmm7, dword ptr [esi + 4]
350587e5 0f2ffd                         comiss   xmm7, xmm5
350587e8 77e2                           ja       0x350587cc
350587ea 0f2f4e14                       comiss   xmm1, dword ptr [esi + 0x14]
350587ee 77dc                           ja       0x350587cc
350587f0 f30f107e08                     movss    xmm7, dword ptr [esi + 8]
350587f5 0f2ffe                         comiss   xmm7, xmm6
350587f8 77d2                           ja       0x350587cc
350587fa 0f2f5618                       comiss   xmm2, dword ptr [esi + 0x18]
350587fe 77cc                           ja       0x350587cc
35058800 f30f10560c                     movss    xmm2, dword ptr [esi + 0xc]
35058805 0f2fd3                         comiss   xmm2, xmm3
35058808 77c2                           ja       0x350587cc
3505880a 807f2800                       cmp      byte ptr [edi + 0x28], 0
3505880e 0f84bb040000                   je       0x35058ccf
35058814 807def00                       cmp      byte ptr [ebp - 0x11], 0
35058818 0f84d7030000                   je       0x35058bf5
3505881e 84d2                           test     dl, dl
35058820 0f85a9040000                   jne      0x35058ccf
35058826 83beec00000000                 cmp      dword ptr [esi + 0xec], 0
3505882d 0f85c2030000                   jne      0x35058bf5
35058833 83bec000000000                 cmp      dword ptr [esi + 0xc0], 0
3505883a 0f8491020000                   je       0x35058ad1
35058840 8b8ec4000000                   mov      ecx, dword ptr [esi + 0xc4]
35058846 85c9                           test     ecx, ecx
35058848 0f8483020000                   je       0x35058ad1
3505884e 8b86c0000000                   mov      eax, dword ptr [esi + 0xc0]
35058854 f30f105e64                     movss    xmm3, dword ptr [esi + 0x64]
35058859 f30f1010                       movss    xmm2, dword ptr [eax]
3505885d f30f107e60                     movss    xmm7, dword ptr [esi + 0x60]
35058862 f30f10670c                     movss    xmm4, dword ptr [edi + 0xc]
35058867 f30f11857cffffff               movss    dword ptr [ebp - 0x84], xmm0
3505886f f30f104668                     movss    xmm0, dword ptr [esi + 0x68]
35058874 f30f114d80                     movss    dword ptr [ebp - 0x80], xmm1
35058879 f30f104804                     movss    xmm1, dword ptr [eax + 4]
3505887e f30f1145a8                     movss    dword ptr [ebp - 0x58], xmm0
35058883 0f28c1                         movaps   xmm0, xmm1
35058886 f30f59c3                       mulss    xmm0, xmm3
3505888a f30f115d9c                     movss    dword ptr [ebp - 0x64], xmm3
3505888f 0f28da                         movaps   xmm3, xmm2
35058892 f30f59df                       mulss    xmm3, xmm7
35058896 f30f58c3                       addss    xmm0, xmm3
3505889a 0f28dc                         movaps   xmm3, xmm4
3505889d f30f595da8                     mulss    xmm3, dword ptr [ebp - 0x58]
350588a2 f30f58c3                       addss    xmm0, xmm3
350588a6 f30f105e70                     movss    xmm3, dword ptr [esi + 0x70]
350588ab f30f115da4                     movss    dword ptr [ebp - 0x5c], xmm3
350588b0 f30f594da4                     mulss    xmm1, dword ptr [ebp - 0x5c]
350588b5 f30f105e74                     movss    xmm3, dword ptr [esi + 0x74]
350588ba f30f117d98                     movss    dword ptr [ebp - 0x68], xmm7
350588bf f30f107e6c                     movss    xmm7, dword ptr [esi + 0x6c]
350588c4 f30f59d7                       mulss    xmm2, xmm7
350588c8 f30f58ca                       addss    xmm1, xmm2
350588cc f30f105654                     movss    xmm2, dword ptr [esi + 0x54]
350588d1 f30f1165a0                     movss    dword ptr [ebp - 0x60], xmm4
350588d6 f30f59e3                       mulss    xmm4, xmm3
350588da f30f58cc                       addss    xmm1, xmm4
350588de f30f10a684000000               movss    xmm4, dword ptr [esi + 0x84]
350588e6 f30f59c4                       mulss    xmm0, xmm4
350588ea f30f58c2                       addss    xmm0, xmm2
350588ee 8d41ff                         lea      eax, [ecx - 1]
350588f1 f30f115590                     movss    dword ptr [ebp - 0x70], xmm2
350588f6 f30f105658                     movss    xmm2, dword ptr [esi + 0x58]
350588fb f30f59cc                       mulss    xmm1, xmm4
350588ff f30f58ca                       addss    xmm1, xmm2
35058903 f30f116d84                     movss    dword ptr [ebp - 0x7c], xmm5
35058908 f30f117588                     movss    dword ptr [ebp - 0x78], xmm6
3505890d f30f117d94                     movss    dword ptr [ebp - 0x6c], xmm7
35058912 f30f115dac                     movss    dword ptr [ebp - 0x54], xmm3
35058917 f30f11658c                     movss    dword ptr [ebp - 0x74], xmm4
3505891c f30f1155d8                     movss    dword ptr [ebp - 0x28], xmm2
35058921 f30f1145e0                     movss    dword ptr [ebp - 0x20], xmm0
35058926 f30f114de4                     movss    dword ptr [ebp - 0x1c], xmm1
3505892b 8945e8                         mov      dword ptr [ebp - 0x18], eax
3505892e 85c0                           test     eax, eax
35058930 0f88e7000000                   js       0x35058a1d
35058936 eb1c                           jmp      0x35058954
35058938 eb06                           jmp      0x35058940
3505893a 8d9b00000000                   lea      ebx, [ebx]
35058940 f30f104de4                     movss    xmm1, dword ptr [ebp - 0x1c]
35058945 f30f1045e0                     movss    xmm0, dword ptr [ebp - 0x20]
3505894a f30f107d94                     movss    xmm7, dword ptr [ebp - 0x6c]
3505894f f30f10658c                     movss    xmm4, dword ptr [ebp - 0x74]
35058954 0f2f45bc                       comiss   xmm0, dword ptr [ebp - 0x44]
35058958 7214                           jb       0x3505896e
3505895a 0f2fe8                         comiss   xmm5, xmm0
3505895d 720f                           jb       0x3505896e
3505895f 0f2f4dc0                       comiss   xmm1, dword ptr [ebp - 0x40]
35058963 7209                           jb       0x3505896e
35058965 0f2ff1                         comiss   xmm6, xmm1
35058968 0f8387020000                   jae      0x35058bf5
3505896e 8b96c0000000                   mov      edx, dword ptr [esi + 0xc0]
35058974 8b45e8                         mov      eax, dword ptr [ebp - 0x18]
35058977 f30f1014c2                     movss    xmm2, dword ptr [edx + eax*8]
3505897c f30f104cc204                   movss    xmm1, dword ptr [edx + eax*8 + 4]
35058982 f30f105da0                     movss    xmm3, dword ptr [ebp - 0x60]
35058987 8d04c2                         lea      eax, [edx + eax*8]
3505898a 0f28c1                         movaps   xmm0, xmm1
3505898d f30f59459c                     mulss    xmm0, dword ptr [ebp - 0x64]
35058992 f30f594da4                     mulss    xmm1, dword ptr [ebp - 0x5c]
35058997 0f28ea                         movaps   xmm5, xmm2
3505899a f30f596d98                     mulss    xmm5, dword ptr [ebp - 0x68]
3505899f f30f58c5                       addss    xmm0, xmm5
350589a3 8d4df4                         lea      ecx, [ebp - 0xc]
350589a6 0f28eb                         movaps   xmm5, xmm3
350589a9 f30f596da8                     mulss    xmm5, dword ptr [ebp - 0x58]
350589ae f30f595dac                     mulss    xmm3, dword ptr [ebp - 0x54]
350589b3 f30f59d7                       mulss    xmm2, xmm7
350589b7 f30f58ca                       addss    xmm1, xmm2
350589bb 51                             push     ecx
350589bc 8d55e0                         lea      edx, [ebp - 0x20]
350589bf f30f58c5                       addss    xmm0, xmm5
350589c3 f30f58cb                       addss    xmm1, xmm3
350589c7 0f28d4                         movaps   xmm2, xmm4
350589ca 52                             push     edx
350589cb 8d857cffffff                   lea      eax, [ebp - 0x84]
350589d1 f30f59c2                       mulss    xmm0, xmm2
350589d5 f30f584590                     addss    xmm0, dword ptr [ebp - 0x70]
350589da f30f59ca                       mulss    xmm1, xmm2
350589de f30f584dd8                     addss    xmm1, dword ptr [ebp - 0x28]
350589e3 50                             push     eax
350589e4 f30f1145f4                     movss    dword ptr [ebp - 0xc], xmm0
350589e9 f30f114df8                     movss    dword ptr [ebp - 8], xmm1
350589ee e82d08ffff                     call     0x35049220
350589f3 83c40c                         add      esp, 0xc
350589f6 84c0                           test     al, al
350589f8 0f85f7010000                   jne      0x35058bf5
350589fe ff4de8                         dec      dword ptr [ebp - 0x18]
35058a01 8b4df4                         mov      ecx, dword ptr [ebp - 0xc]
35058a04 8b55f8                         mov      edx, dword ptr [ebp - 8]
35058a07 f30f1075cc                     movss    xmm6, dword ptr [ebp - 0x34]
35058a0c f30f106dc8                     movss    xmm5, dword ptr [ebp - 0x38]
35058a11 894de0                         mov      dword ptr [ebp - 0x20], ecx
35058a14 8955e4                         mov      dword ptr [ebp - 0x1c], edx
35058a17 0f8923ffffff                   jns      0x35058940
35058a1d f30f1065d0                     movss    xmm4, dword ptr [ebp - 0x30]
35058a22 f30f101d80a21f35               movss    xmm3, dword ptr [0x351fa280]
35058a2a 0f28d4                         movaps   xmm2, xmm4
35058a2d f30f5c55c4                     subss    xmm2, dword ptr [ebp - 0x3c]
35058a32 f30f5865c4                     addss    xmm4, dword ptr [ebp - 0x3c]
35058a37 0f28cd                         movaps   xmm1, xmm5
35058a3a f30f5c4dbc                     subss    xmm1, dword ptr [ebp - 0x44]
35058a3f f30f586dbc                     addss    xmm5, dword ptr [ebp - 0x44]
35058a44 f30f59d2                       mulss    xmm2, xmm2
35058a48 0f28c6                         movaps   xmm0, xmm6
35058a4b f30f5c45c0                     subss    xmm0, dword ptr [ebp - 0x40]
35058a50 f30f5875c0                     addss    xmm6, dword ptr [ebp - 0x40]
35058a55 f30f59c9                       mulss    xmm1, xmm1
35058a59 f30f59c0                       mulss    xmm0, xmm0
35058a5d f30f58c1                       addss    xmm0, xmm1
35058a61 f30f58c2                       addss    xmm0, xmm2
35058a65 0f28d4                         movaps   xmm2, xmm4
35058a68 f30f102584a21f35               movss    xmm4, dword ptr [0x351fa284]
35058a70 f30f59d3                       mulss    xmm2, xmm3
35058a74 0f57c9                         xorps    xmm1, xmm1
35058a77 f30f1155f8                     movss    dword ptr [ebp - 8], xmm2
35058a7c f30f10c8                       movss    xmm1, xmm0
35058a80 f30f52c9                       rsqrtss  xmm1, xmm1
35058a84 0f28d1                         movaps   xmm2, xmm1
35058a87 f30f59d0                       mulss    xmm2, xmm0
35058a8b f30f59d1                       mulss    xmm2, xmm1
35058a8f f30f59d3                       mulss    xmm2, xmm3
35058a93 f30f5ce2                       subss    xmm4, xmm2
35058a97 f30f59e1                       mulss    xmm4, xmm1
35058a9b 51                             push     ecx
35058a9c f30f59e0                       mulss    xmm4, xmm0
35058aa0 f30f59e3                       mulss    xmm4, xmm3
35058aa4 8d45f0                         lea      eax, [ebp - 0x10]
35058aa7 f30f112424                     movss    dword ptr [esp], xmm4
35058aac f30f59eb                       mulss    xmm5, xmm3
35058ab0 f30f59f3                       mulss    xmm6, xmm3
35058ab4 50                             push     eax
35058ab5 8bce                           mov      ecx, esi
35058ab7 f30f116df0                     movss    dword ptr [ebp - 0x10], xmm5
35058abc f30f1175f4                     movss    dword ptr [ebp - 0xc], xmm6
35058ac1 e86a84ffff                     call     0x35050f30
35058ac6 85c0                           test     eax, eax
35058ac8 0f84fefcffff                   je       0x350587cc
35058ace 0f57e4                         xorps    xmm4, xmm4
35058ad1 83bebc00000000                 cmp      dword ptr [esi + 0xbc], 0
35058ad8 0f84ea010000                   je       0x35058cc8
35058ade 8b8ebc000000                   mov      ecx, dword ptr [esi + 0xbc]
35058ae4 8b11                           mov      edx, dword ptr [ecx]
35058ae6 8b4204                         mov      eax, dword ptr [edx + 4]
35058ae9 ffd0                           call     eax
35058aeb 83f804                         cmp      eax, 4
35058aee 0f850c010000                   jne      0x35058c00
35058af4 f30f103580a21f35               movss    xmm6, dword ptr [0x351fa280]
35058afc f30f105e10                     movss    xmm3, dword ptr [esi + 0x10]
35058b01 f30f106614                     movss    xmm4, dword ptr [esi + 0x14]
35058b06 f30f104e08                     movss    xmm1, dword ptr [esi + 8]
35058b0b f30f106e18                     movss    xmm5, dword ptr [esi + 0x18]
35058b10 f30f10560c                     movss    xmm2, dword ptr [esi + 0xc]
35058b15 f30f104604                     movss    xmm0, dword ptr [esi + 4]
35058b1a f30f58cc                       addss    xmm1, xmm4
35058b1e f30f59ce                       mulss    xmm1, xmm6
35058b22 8b4e0c                         mov      ecx, dword ptr [esi + 0xc]
35058b25 f30f58d5                       addss    xmm2, xmm5
35058b29 f30f59d6                       mulss    xmm2, xmm6
35058b2d f30f58c3                       addss    xmm0, xmm3
35058b31 f30f59c6                       mulss    xmm0, xmm6
35058b35 f30f107704                     movss    xmm6, dword ptr [edi + 4]
35058b3a f30f5cf0                       subss    xmm6, xmm0
35058b3e f30f104708                     movss    xmm0, dword ptr [edi + 8]
35058b43 f30f5cc1                       subss    xmm0, xmm1
35058b47 f30f104f0c                     movss    xmm1, dword ptr [edi + 0xc]
35058b4c f30f5cca                       subss    xmm1, xmm2
35058b50 f30f105710                     movss    xmm2, dword ptr [edi + 0x10]
35058b55 f30f114de4                     movss    dword ptr [ebp - 0x1c], xmm1
35058b5a f30f104f14                     movss    xmm1, dword ptr [edi + 0x14]
35058b5f f30f59c9                       mulss    xmm1, xmm1
35058b63 f30f59d2                       mulss    xmm2, xmm2
35058b67 f30f58d1                       addss    xmm2, xmm1
35058b6b f30f104f18                     movss    xmm1, dword ptr [edi + 0x18]
35058b70 f30f59c9                       mulss    xmm1, xmm1
35058b74 f30f58d1                       addss    xmm2, xmm1
35058b78 0f57c9                         xorps    xmm1, xmm1
35058b7b f30f10ca                       movss    xmm1, xmm2
35058b7f f30f51c9                       sqrtss   xmm1, xmm1
35058b83 0f28d1                         movaps   xmm2, xmm1
35058b86 f30f7e4e04                     movq     xmm1, qword ptr [esi + 4]
35058b8b 660fd64df0                     movq     qword ptr [ebp - 0x10], xmm1
35058b90 f30f5c65f4                     subss    xmm4, dword ptr [ebp - 0xc]
35058b95 f30f5c5df0                     subss    xmm3, dword ptr [ebp - 0x10]
35058b9a 894df8                         mov      dword ptr [ebp - 8], ecx
35058b9d f30f5c6df8                     subss    xmm5, dword ptr [ebp - 8]
35058ba2 f30f59e4                       mulss    xmm4, xmm4
35058ba6 0f57c9                         xorps    xmm1, xmm1
35058ba9 f30f59db                       mulss    xmm3, xmm3
35058bad f30f59c0                       mulss    xmm0, xmm0
35058bb1 f30f58e3                       addss    xmm4, xmm3
35058bb5 f30f59ed                       mulss    xmm5, xmm5
35058bb9 f30f58e5                       addss    xmm4, xmm5
35058bbd f30f10cc                       movss    xmm1, xmm4
35058bc1 f30f59f6                       mulss    xmm6, xmm6
35058bc5 f30f58c6                       addss    xmm0, xmm6
35058bc9 f30f1145d8                     movss    dword ptr [ebp - 0x28], xmm0
35058bce f30f1045e4                     movss    xmm0, dword ptr [ebp - 0x1c]
35058bd3 f30f51c9                       sqrtss   xmm1, xmm1
35058bd7 f30f58ca                       addss    xmm1, xmm2
35058bdb f30f1055d8                     movss    xmm2, dword ptr [ebp - 0x28]
35058be0 f30f59c0                       mulss    xmm0, xmm0
35058be4 f30f58d0                       addss    xmm2, xmm0
35058be8 f30f59c9                       mulss    xmm1, xmm1
35058bec 0f2fd1                         comiss   xmm2, xmm1
35058bef 0f82d0000000                   jb       0x35058cc5
35058bf5 5f                             pop      edi
35058bf6 83c8ff                         or       eax, 0xffffffff
35058bf9 5e                             pop      esi
35058bfa 8be5                           mov      esp, ebp
35058bfc 5d                             pop      ebp
35058bfd c20400                         ret      4
35058c00 8b8ebc000000                   mov      ecx, dword ptr [esi + 0xbc]
35058c06 8b11                           mov      edx, dword ptr [ecx]
35058c08 0f57c0                         xorps    xmm0, xmm0
35058c0b 8b4254                         mov      eax, dword ptr [edx + 0x54]
35058c0e 51                             push     ecx
35058c0f f30f110424                     movss    dword ptr [esp], xmm0
35058c14 ffd0                           call     eax
35058c16 85c0                           test     eax, eax
35058c18 74db                           je       0x35058bf5
35058c1a 33c0                           xor      eax, eax
35058c1c 8945e8                         mov      dword ptr [ebp - 0x18], eax
35058c1f eb03                           jmp      0x35058c24
35058c21 8b45e8                         mov      eax, dword ptr [ebp - 0x18]
35058c24 8b4f0c                         mov      ecx, dword ptr [edi + 0xc]
35058c27 f30f7e4f04                     movq     xmm1, qword ptr [edi + 4]
35058c2c f30f10058ca21f35               movss    xmm0, dword ptr [0x351fa28c]
35058c34 f30f101588a21f35               movss    xmm2, dword ptr [0x351fa288]
35058c3c 660fd64df0                     movq     qword ptr [ebp - 0x10], xmm1
35058c41 894df8                         mov      dword ptr [ebp - 8], ecx
35058c44 a801                           test     al, 1
35058c46 7405                           je       0x35058c4d
35058c48 0f28c8                         movaps   xmm1, xmm0
35058c4b eb03                           jmp      0x35058c50
35058c4d 0f28ca                         movaps   xmm1, xmm2
35058c50 f30f105f10                     movss    xmm3, dword ptr [edi + 0x10]
35058c55 f30f59d9                       mulss    xmm3, xmm1
35058c59 f30f585df0                     addss    xmm3, dword ptr [ebp - 0x10]
35058c5e f30f115df0                     movss    dword ptr [ebp - 0x10], xmm3
35058c63 a802                           test     al, 2
35058c65 7405                           je       0x35058c6c
35058c67 0f28c8                         movaps   xmm1, xmm0
35058c6a eb03                           jmp      0x35058c6f
35058c6c 0f28ca                         movaps   xmm1, xmm2
35058c6f f30f105f14                     movss    xmm3, dword ptr [edi + 0x14]
35058c74 f30f59d9                       mulss    xmm3, xmm1
35058c78 f30f585df4                     addss    xmm3, dword ptr [ebp - 0xc]
35058c7d f30f115df4                     movss    dword ptr [ebp - 0xc], xmm3
35058c82 a804                           test     al, 4
35058c84 7503                           jne      0x35058c89
35058c86 0f28c2                         movaps   xmm0, xmm2
35058c89 f30f104f18                     movss    xmm1, dword ptr [edi + 0x18]
35058c8e 8b8ebc000000                   mov      ecx, dword ptr [esi + 0xbc]
35058c94 f30f59c8                       mulss    xmm1, xmm0
35058c98 f30f584df8                     addss    xmm1, dword ptr [ebp - 8]
35058c9d f30f114df8                     movss    dword ptr [ebp - 8], xmm1
35058ca2 8b11                           mov      edx, dword ptr [ecx]
35058ca4 8b5220                         mov      edx, dword ptr [edx + 0x20]
35058ca7 8d45f0                         lea      eax, [ebp - 0x10]
35058caa 50                             push     eax
35058cab ffd2                           call     edx
35058cad 85c0                           test     eax, eax
35058caf 0f8440ffffff                   je       0x35058bf5
35058cb5 8b45e8                         mov      eax, dword ptr [ebp - 0x18]
35058cb8 40                             inc      eax
35058cb9 8945e8                         mov      dword ptr [ebp - 0x18], eax
35058cbc 83f808                         cmp      eax, 8
35058cbf 0f8c5cffffff                   jl       0x35058c21
35058cc5 0f57e4                         xorps    xmm4, xmm4
35058cc8 c645ff01                       mov      byte ptr [ebp - 1], 1
35058ccc 8a55ff                         mov      dl, byte ptr [ebp - 1]
35058ccf 807def00                       cmp      byte ptr [ebp - 0x11], 0
35058cd3 0f84c8010000                   je       0x35058ea1
35058cd9 84d2                           test     dl, dl
35058cdb 751c                           jne      0x35058cf9
35058cdd 51                             push     ecx
35058cde 8d4704                         lea      eax, [edi + 4]
35058ce1 f30f112424                     movss    dword ptr [esp], xmm4
35058ce6 50                             push     eax
35058ce7 8bce                           mov      ecx, esi
35058ce9 e84282ffff                     call     0x35050f30
35058cee 85c0                           test     eax, eax
35058cf0 0f84d6faffff                   je       0x350587cc
35058cf6 8a55ff                         mov      dl, byte ptr [ebp - 1]
35058cf9 f30f7e8688010000               movq     xmm0, qword ptr [esi + 0x188]
35058d01 8b45d4                         mov      eax, dword ptr [ebp - 0x2c]
35058d04 660fd6472c                     movq     qword ptr [edi + 0x2c], xmm0
35058d09 8b8e90010000                   mov      ecx, dword ptr [esi + 0x190]
35058d0f 894f34                         mov      dword ptr [edi + 0x34], ecx
35058d12 81c63c010000                   add      esi, 0x13c
35058d18 83c738                         add      edi, 0x38
35058d1b b913000000                     mov      ecx, 0x13
35058d20 f3a5                           rep movsd dword ptr es:[edi], dword ptr [esi]
35058d22 83b8ac01000002                 cmp      dword ptr [eax + 0x1ac], 2
35058d29 0f855f010000                   jne      0x35058e8e
35058d2f 8b4d08                         mov      ecx, dword ptr [ebp + 8]
35058d32 8b712c                         mov      esi, dword ptr [ecx + 0x2c]
35058d35 81e60000a0ff                   and      esi, 0xffa00000
35058d3b 81fe0000a0ff                   cmp      esi, 0xffa00000
35058d41 0f8499000000                   je       0x35058de0
35058d47 f30f104130                     movss    xmm0, dword ptr [ecx + 0x30]
35058d4c f30f10492c                     movss    xmm1, dword ptr [ecx + 0x2c]
35058d51 f30f105864                     movss    xmm3, dword ptr [eax + 0x64]
35058d56 f30f105134                     movss    xmm2, dword ptr [ecx + 0x34]
35058d5b f30f59d8                       mulss    xmm3, xmm0
35058d5f 0f28e1                         movaps   xmm4, xmm1
35058d62 f30f596060                     mulss    xmm4, dword ptr [eax + 0x60]
35058d67 f30f58dc                       addss    xmm3, xmm4
35058d6b f30f106068                     movss    xmm4, dword ptr [eax + 0x68]
35058d70 f30f59e2                       mulss    xmm4, xmm2
35058d74 f30f58dc                       addss    xmm3, xmm4
35058d78 f30f106070                     movss    xmm4, dword ptr [eax + 0x70]
35058d7d f30f115df0                     movss    dword ptr [ebp - 0x10], xmm3
35058d82 f30f10586c                     movss    xmm3, dword ptr [eax + 0x6c]
35058d87 f30f59d9                       mulss    xmm3, xmm1
35058d8b f30f59e0                       mulss    xmm4, xmm0
35058d8f f30f58dc                       addss    xmm3, xmm4
35058d93 f30f106074                     movss    xmm4, dword ptr [eax + 0x74]
35058d98 f30f59e2                       mulss    xmm4, xmm2
35058d9c f30f58dc                       addss    xmm3, xmm4
35058da0 f30f115df4                     movss    dword ptr [ebp - 0xc], xmm3
35058da5 f30f105878                     movss    xmm3, dword ptr [eax + 0x78]
35058daa f30f59d9                       mulss    xmm3, xmm1
35058dae f30f10487c                     movss    xmm1, dword ptr [eax + 0x7c]
35058db3 f30f59c8                       mulss    xmm1, xmm0
35058db7 f30f108080000000               movss    xmm0, dword ptr [eax + 0x80]
35058dbf f30f59c2                       mulss    xmm0, xmm2
35058dc3 f30f58d9                       addss    xmm3, xmm1
35058dc7 f30f58d8                       addss    xmm3, xmm0
35058dcb f30f7e45f0                     movq     xmm0, qword ptr [ebp - 0x10]
35058dd0 f30f115df8                     movss    dword ptr [ebp - 8], xmm3
35058dd5 8b75f8                         mov      esi, dword ptr [ebp - 8]
35058dd8 660fd6412c                     movq     qword ptr [ecx + 0x2c], xmm0
35058ddd 897134                         mov      dword ptr [ecx + 0x34], esi
35058de0 8b7150                         mov      esi, dword ptr [ecx + 0x50]
35058de3 81e60000a0ff                   and      esi, 0xffa00000
35058de9 81fe0000a0ff                   cmp      esi, 0xffa00000
35058def 0f8499000000                   je       0x35058e8e
35058df5 f30f104154                     movss    xmm0, dword ptr [ecx + 0x54]
35058dfa f30f104950                     movss    xmm1, dword ptr [ecx + 0x50]
35058dff f30f105864                     movss    xmm3, dword ptr [eax + 0x64]
35058e04 f30f105158                     movss    xmm2, dword ptr [ecx + 0x58]
35058e09 f30f59d8                       mulss    xmm3, xmm0
35058e0d 0f28e1                         movaps   xmm4, xmm1
35058e10 f30f596060                     mulss    xmm4, dword ptr [eax + 0x60]
35058e15 f30f58dc                       addss    xmm3, xmm4
35058e19 f30f106068                     movss    xmm4, dword ptr [eax + 0x68]
35058e1e f30f59e2                       mulss    xmm4, xmm2
35058e22 f30f58dc                       addss    xmm3, xmm4
35058e26 f30f106070                     movss    xmm4, dword ptr [eax + 0x70]
35058e2b f30f115df0                     movss    dword ptr [ebp - 0x10], xmm3
35058e30 f30f10586c                     movss    xmm3, dword ptr [eax + 0x6c]
35058e35 f30f59d9                       mulss    xmm3, xmm1
35058e39 f30f59e0                       mulss    xmm4, xmm0
35058e3d f30f58dc                       addss    xmm3, xmm4
35058e41 f30f106074                     movss    xmm4, dword ptr [eax + 0x74]
35058e46 f30f59e2                       mulss    xmm4, xmm2
35058e4a f30f58dc                       addss    xmm3, xmm4
35058e4e f30f115df4                     movss    dword ptr [ebp - 0xc], xmm3
35058e53 f30f105878                     movss    xmm3, dword ptr [eax + 0x78]
35058e58 f30f59d9                       mulss    xmm3, xmm1
35058e5c f30f10487c                     movss    xmm1, dword ptr [eax + 0x7c]
35058e61 f30f59c8                       mulss    xmm1, xmm0
35058e65 f30f108080000000               movss    xmm0, dword ptr [eax + 0x80]
35058e6d f30f59c2                       mulss    xmm0, xmm2
35058e71 f30f58d9                       addss    xmm3, xmm1
35058e75 f30f58d8                       addss    xmm3, xmm0
35058e79 f30f7e45f0                     movq     xmm0, qword ptr [ebp - 0x10]
35058e7e f30f115df8                     movss    dword ptr [ebp - 8], xmm3
35058e83 8b45f8                         mov      eax, dword ptr [ebp - 8]
35058e86 660fd64150                     movq     qword ptr [ecx + 0x50], xmm0
35058e8b 894158                         mov      dword ptr [ecx + 0x58], eax
35058e8e 33c0                           xor      eax, eax
35058e90 84d2                           test     dl, dl
35058e92 0f95c0                         setne    al
35058e95 5f                             pop      edi
35058e96 5e                             pop      esi
35058e97 8d4400ff                       lea      eax, [eax + eax - 1]
35058e9b 8be5                           mov      esp, ebp
35058e9d 5d                             pop      ebp
35058e9e c20400                         ret      4
35058ea1 807f2800                       cmp      byte ptr [edi + 0x28], 0
35058ea5 0f854afdffff                   jne      0x35058bf5
35058eab 83bec400000000                 cmp      dword ptr [esi + 0xc4], 0
35058eb2 7419                           je       0x35058ecd
35058eb4 51                             push     ecx
35058eb5 8d4f04                         lea      ecx, [edi + 4]
35058eb8 f30f112424                     movss    dword ptr [esp], xmm4
35058ebd 51                             push     ecx
35058ebe 8bce                           mov      ecx, esi
35058ec0 e86b80ffff                     call     0x35050f30
35058ec5 85c0                           test     eax, eax
35058ec7 0f84fff8ffff                   je       0x350587cc
35058ecd 6a00                           push     0
35058ecf 8d5508                         lea      edx, [ebp + 8]
35058ed2 52                             push     edx
35058ed3 6a01                           push     1
35058ed5 6a00                           push     0
35058ed7 8d4f38                         lea      ecx, [edi + 0x38]
35058eda 51                             push     ecx
35058edb 8d571c                         lea      edx, [edi + 0x1c]
35058ede 8d472c                         lea      eax, [edi + 0x2c]
35058ee1 52                             push     edx
35058ee2 50                             push     eax
35058ee3 83c704                         add      edi, 4
35058ee6 57                             push     edi
35058ee7 8bce                           mov      ecx, esi
35058ee9 c74508ffffffff                 mov      dword ptr [ebp + 8], 0xffffffff
35058ef0 8945d4                         mov      dword ptr [ebp - 0x2c], eax
35058ef3 e85860ffff                     call     0x3504ef50
35058ef8 85c0                           test     eax, eax
35058efa 0f8520f7ffff                   jne      0x35058620
35058f00 8b45d4                         mov      eax, dword ptr [ebp - 0x2c]
35058f03 8b08                           mov      ecx, dword ptr [eax]
35058f05 81e10000a0ff                   and      ecx, 0xffa00000
35058f0b 81f90000a0ff                   cmp      ecx, 0xffa00000
35058f11 0f8509f7ffff                   jne      0x35058620
35058f17 5f                             pop      edi
35058f18 33c0                           xor      eax, eax
35058f1a 5e                             pop      esi
35058f1b 8be5                           mov      esp, ebp
35058f1d 5d                             pop      ebp
35058f1e c20400                         ret      4
35058f21 83f818                         cmp      eax, 0x18
35058f24 751c                           jne      0x35058f42
35058f26 8b5704                         mov      edx, dword ptr [edi + 4]
35058f29 8b4f08                         mov      ecx, dword ptr [edi + 8]
35058f2c 52                             push     edx
35058f2d 56                             push     esi
35058f2e e80de9ffff                     call     0x35057840
35058f33 ddd8                           fstp     st(0)
35058f35 5f                             pop      edi
35058f36 b801000000                     mov      eax, 1
35058f3b 5e                             pop      esi
35058f3c 8be5                           mov      esp, ebp
35058f3e 5d                             pop      ebp
35058f3f c20400                         ret      4
35058f42 83f819                         cmp      eax, 0x19
35058f45 0f8581f8ffff                   jne      0x350587cc
35058f4b 8b4f04                         mov      ecx, dword ptr [edi + 4]
35058f4e 8b5708                         mov      edx, dword ptr [edi + 8]
35058f51 8b06                           mov      eax, dword ptr [esi]
35058f53 8b8094000000                   mov      eax, dword ptr [eax + 0x94]
35058f59 51                             push     ecx
35058f5a 52                             push     edx
35058f5b 83c70c                         add      edi, 0xc
35058f5e 57                             push     edi
35058f5f 8bce                           mov      ecx, esi
35058f61 ffd0                           call     eax
35058f63 5f                             pop      edi
35058f64 b801000000                     mov      eax, 1
35058f69 5e                             pop      esi
35058f6a 8be5                           mov      esp, ebp
35058f6c 5d                             pop      ebp
35058f6d c20400                         ret      4
35058f70 55                             push     ebp
35058f71 8bec                           mov      ebp, esp
35058f73 6aff                           push     -1
35058f75 6883661f35                     push     0x351f6683
35058f7a 64a100000000                   mov      eax, dword ptr fs:[0]
35058f80 50                             push     eax
35058f81 64892500000000                 mov      dword ptr fs:[0], esp
35058f88 51                             push     ecx
35058f89 57                             push     edi
35058f8a 8b7d0c                         mov      edi, dword ptr [ebp + 0xc]
35058f8d 3b7d08                         cmp      edi, dword ptr [ebp + 8]
35058f90 0f8687000000                   jbe      0x3505901d
35058f96 56                             push     esi
35058f97 83ef18                         sub      edi, 0x18
35058f9a 897d0c                         mov      dword ptr [ebp + 0xc], edi
35058f9d 8d7714                         lea      esi, [edi + 0x14]
35058fa0 c745fc00000000                 mov      dword ptr [ebp - 4], 0
35058fa7 8975f0                         mov      dword ptr [ebp - 0x10], esi
35058faa 8b0e                           mov      ecx, dword ptr [esi]
35058fac 8b41fc                         mov      eax, dword ptr [ecx - 4]
35058faf 25ffffff3f                     and      eax, 0x3fffffff
35058fb4 8d0440                         lea      eax, [eax + eax*2]
35058fb7 8d14c1                         lea      edx, [ecx + eax*8]
35058fba 52                             push     edx
35058fbb 51                             push     ecx
35058fbc 8bce                           mov      ecx, esi
35058fbe c645fc01                       mov      byte ptr [ebp - 4], 1
35058fc2 e8a9ffffff                     call     0x35058f70
35058fc7 8b36                           mov      esi, dword ptr [esi]
35058fc9 8b46fc                         mov      eax, dword ptr [esi - 4]
35058fcc 8bc8                           mov      ecx, eax
35058fce c1e91e                         shr      ecx, 0x1e
35058fd1 83e101                         and      ecx, 1
35058fd4 25ffffff3f                     and      eax, 0x3fffffff
35058fd9 03c8                           add      ecx, eax
35058fdb 740c                           je       0x35058fe9
35058fdd 8d46fc                         lea      eax, [esi - 4]
35058fe0 50                             push     eax
35058fe1 e81ab01900                     call     0x351f4000
35058fe6 83c404                         add      esp, 4
35058fe9 8b4f10                         mov      ecx, dword ptr [edi + 0x10]
35058fec 8b41fc                         mov      eax, dword ptr [ecx - 4]
35058fef 8bd0                           mov      edx, eax
35058ff1 c1ea1e                         shr      edx, 0x1e
35058ff4 83e201                         and      edx, 1
35058ff7 25ffffff3f                     and      eax, 0x3fffffff
35058ffc 03d0                           add      edx, eax
35058ffe c745fcffffffff                 mov      dword ptr [ebp - 4], 0xffffffff
35059005 740c                           je       0x35059013
35059007 83c1fc                         add      ecx, -4
3505900a 51                             push     ecx
3505900b e8f0af1900                     call     0x351f4000
35059010 83c404                         add      esp, 4
35059013 3b7d08                         cmp      edi, dword ptr [ebp + 8]
35059016 0f877bffffff                   ja       0x35058f97
3505901c 5e                             pop      esi
3505901d 8b4df4                         mov      ecx, dword ptr [ebp - 0xc]
35059020 5f                             pop      edi
35059021 64890d00000000                 mov      dword ptr fs:[0], ecx
35059028 8be5                           mov      esp, ebp
3505902a 5d                             pop      ebp
3505902b c20800                         ret      8
3505902e cc                             int3
3505902f cc                             int3
35059030 55                             push     ebp
35059031 8bec                           mov      ebp, esp
35059033 56                             push     esi
35059034 8b7508                         mov      esi, dword ptr [ebp + 8]
35059037 57                             push     edi
35059038 8d7914                         lea      edi, [ecx + 0x14]
3505903b 89710c                         mov      dword ptr [ecx + 0xc], esi
3505903e 8b0f                           mov      ecx, dword ptr [edi]
35059040 8b41fc                         mov      eax, dword ptr [ecx - 4]
35059043 25ffffff3f                     and      eax, 0x3fffffff
35059048 3bf0                           cmp      esi, eax
3505904a 7e19                           jle      0x35059065
3505904c 2bf0                           sub      esi, eax
3505904e 56                             push     esi
3505904f 56                             push     esi
35059050 8bcf                           mov      ecx, edi
35059052 e859e3ffff                     call     0x350573b0
35059057 50                             push     eax
35059058 8bcf                           mov      ecx, edi
3505905a e801e9ffff                     call     0x35057960
3505905f 5f                             pop      edi
35059060 5e                             pop      esi
35059061 5d                             pop      ebp
35059062 c20400                         ret      4
35059065 8d0440                         lea      eax, [eax + eax*2]
35059068 8d14c1                         lea      edx, [ecx + eax*8]
3505906b 8d0476                         lea      eax, [esi + esi*2]
3505906e 8d0cc1                         lea      ecx, [ecx + eax*8]
35059071 52                             push     edx
35059072 51                             push     ecx
35059073 8bcf                           mov      ecx, edi
35059075 e8f6feffff                     call     0x35058f70
3505907a 56                             push     esi
3505907b 8bcf                           mov      ecx, edi
3505907d e8bee2ffff                     call     0x35057340
35059082 5f                             pop      edi
35059083 5e                             pop      esi
35059084 5d                             pop      ebp
35059085 c20400                         ret      4
35059088 cc                             int3
35059089 cc                             int3
3505908a cc                             int3
3505908b cc                             int3
3505908c cc                             int3
3505908d cc                             int3
3505908e cc                             int3
3505908f cc                             int3

# 30087f40..30088310
30087f40 55                             push     ebp
30087f41 8bec                           mov      ebp, esp
30087f43 64a100000000                   mov      eax, dword ptr fs:[0]
30087f49 6aff                           push     -1
30087f4b 6816ac2330                     push     0x3023ac16
30087f50 50                             push     eax
30087f51 64892500000000                 mov      dword ptr fs:[0], esp
30087f58 83ec3c                         sub      esp, 0x3c
30087f5b 56                             push     esi
30087f5c 8bf1                           mov      esi, ecx
30087f5e 807e0c00                       cmp      byte ptr [esi + 0xc], 0
30087f62 7524                           jne      0x30087f88
30087f64 8b4510                         mov      eax, dword ptr [ebp + 0x10]
30087f67 8b4d0c                         mov      ecx, dword ptr [ebp + 0xc]
30087f6a 8b5508                         mov      edx, dword ptr [ebp + 8]
30087f6d 50                             push     eax
30087f6e 51                             push     ecx
30087f6f 52                             push     edx
30087f70 8bce                           mov      ecx, esi
30087f72 e849feffff                     call     0x30087dc0
30087f77 5e                             pop      esi
30087f78 8b4df4                         mov      ecx, dword ptr [ebp - 0xc]
30087f7b 64890d00000000                 mov      dword ptr fs:[0], ecx
30087f82 8be5                           mov      esp, ebp
30087f84 5d                             pop      ebp
30087f85 c20c00                         ret      0xc
30087f88 b801000000                     mov      eax, 1
30087f8d 8405b88f2a30                   test     byte ptr [0x302a8fb8], al
30087f93 752d                           jne      0x30087fc2
30087f95 0905b88f2a30                   or       dword ptr [0x302a8fb8], eax
30087f9b a168813530                     mov      eax, dword ptr [0x30358168]
30087fa0 8b08                           mov      ecx, dword ptr [eax]
30087fa2 6a0c                           push     0xc
30087fa4 6888a62530                     push     0x3025a688
30087fa9 51                             push     ecx
30087faa b9388c2a30                     mov      ecx, 0x302a8c38
30087faf c745fc00000000                 mov      dword ptr [ebp - 4], 0
30087fb6 e805f4f8ff                     call     0x300173c0
30087fbb c745fcffffffff                 mov      dword ptr [ebp - 4], 0xffffffff
30087fc2 8b0d68813530                   mov      ecx, dword ptr [0x30358168]
30087fc8 660f57c0                       xorpd    xmm0, xmm0
30087fcc 33d2                           xor      edx, edx
30087fce 660f1345b8                     movlpd   qword ptr [ebp - 0x48], xmm0
30087fd3 660f57c0                       xorpd    xmm0, xmm0
30087fd7 660f1345c0                     movlpd   qword ptr [ebp - 0x40], xmm0
30087fdc 8955c8                         mov      dword ptr [ebp - 0x38], edx
30087fdf 8955cc                         mov      dword ptr [ebp - 0x34], edx
30087fe2 389197000000                   cmp      byte ptr [ecx + 0x97], dl
30087fe8 741f                           je       0x30088009
30087fea 8d55b8                         lea      edx, [ebp - 0x48]
30087fed c745c8388c2a30                 mov      dword ptr [ebp - 0x38], 0x302a8c38
30087ff4 8b8198000000                   mov      eax, dword ptr [ecx + 0x98]
30087ffa 52                             push     edx
30087ffb ffd0                           call     eax
30087ffd 8b55c8                         mov      edx, dword ptr [ebp - 0x38]
30088000 8b0d68813530                   mov      ecx, dword ptr [0x30358168]
30088006 83c404                         add      esp, 4
30088009 8b4508                         mov      eax, dword ptr [ebp + 8]
3008800c f30f106004                     movss    xmm4, dword ptr [eax + 4]
30088011 f30f1018                       movss    xmm3, dword ptr [eax]
30088015 f30f106808                     movss    xmm5, dword ptr [eax + 8]
3008801a f30f104620                     movss    xmm0, dword ptr [esi + 0x20]
3008801f f30f105630                     movss    xmm2, dword ptr [esi + 0x30]
30088024 f30f107640                     movss    xmm6, dword ptr [esi + 0x40]
30088029 f30f59c4                       mulss    xmm0, xmm4
3008802d f30f59d4                       mulss    xmm2, xmm4
30088031 0f28cb                         movaps   xmm1, xmm3
30088034 f30f594e1c                     mulss    xmm1, dword ptr [esi + 0x1c]
30088039 f30f58c1                       addss    xmm0, xmm1
3008803d f30f104e24                     movss    xmm1, dword ptr [esi + 0x24]
30088042 f30f59cd                       mulss    xmm1, xmm5
30088046 f30f58c1                       addss    xmm0, xmm1
3008804a f30f104e2c                     movss    xmm1, dword ptr [esi + 0x2c]
3008804f f30f584628                     addss    xmm0, dword ptr [esi + 0x28]
30088054 f30f59cb                       mulss    xmm1, xmm3
30088058 f30f58ca                       addss    xmm1, xmm2
3008805c f30f105634                     movss    xmm2, dword ptr [esi + 0x34]
30088061 f30f59d5                       mulss    xmm2, xmm5
30088065 f30f58ca                       addss    xmm1, xmm2
30088069 f30f10563c                     movss    xmm2, dword ptr [esi + 0x3c]
3008806e f30f584e38                     addss    xmm1, dword ptr [esi + 0x38]
30088073 f30f59d3                       mulss    xmm2, xmm3
30088077 f30f59f4                       mulss    xmm6, xmm4
3008807b f30f106644                     movss    xmm4, dword ptr [esi + 0x44]
30088080 f30f58d6                       addss    xmm2, xmm6
30088084 f30f59e5                       mulss    xmm4, xmm5
30088088 f30f102d70c52430               movss    xmm5, dword ptr [0x3024c570]
30088090 f30f58d4                       addss    xmm2, xmm4
30088094 f30f585648                     addss    xmm2, dword ptr [esi + 0x48]
30088099 f30f59c9                       mulss    xmm1, xmm1
3008809d f30f59c0                       mulss    xmm0, xmm0
300880a1 f30f58c8                       addss    xmm1, xmm0
300880a5 f30f59d2                       mulss    xmm2, xmm2
300880a9 f30f58ca                       addss    xmm1, xmm2
300880ad 0f2fcd                         comiss   xmm1, xmm5
300880b0 722f                           jb       0x300880e1
300880b2 c745fcffffffff                 mov      dword ptr [ebp - 4], 0xffffffff
300880b9 85d2                           test     edx, edx
300880bb 0f8430020000                   je       0x300882f1
300880c1 8b819c000000                   mov      eax, dword ptr [ecx + 0x9c]
300880c7 8d55b8                         lea      edx, [ebp - 0x48]
300880ca 52                             push     edx
300880cb ffd0                           call     eax
300880cd 83c404                         add      esp, 4
300880d0 5e                             pop      esi
300880d1 8b4df4                         mov      ecx, dword ptr [ebp - 0xc]
300880d4 64890d00000000                 mov      dword ptr fs:[0], ecx
300880db 8be5                           mov      esp, ebp
300880dd 5d                             pop      ebp
300880de c20c00                         ret      0xc
300880e1 0f57c0                         xorps    xmm0, xmm0
300880e4 f30f10c1                       movss    xmm0, xmm1
300880e8 f30f51c0                       sqrtss   xmm0, xmm0
300880ec 0f28cd                         movaps   xmm1, xmm5
300880ef f30f5cc8                       subss    xmm1, xmm0
300880f3 0f28c5                         movaps   xmm0, xmm5
300880f6 f30f5c464c                     subss    xmm0, dword ptr [esi + 0x4c]
300880fb 0f28d1                         movaps   xmm2, xmm1
300880fe 0f28e0                         movaps   xmm4, xmm0
30088101 f30f59d0                       mulss    xmm2, xmm0
30088105 f30f59e0                       mulss    xmm4, xmm0
30088109 0f2fe2                         comiss   xmm4, xmm2
3008810c 7607                           jbe      0x30088115
3008810e f30f5ec8                       divss    xmm1, xmm0
30088112 0f28e9                         movaps   xmm5, xmm1
30088115 807e0d00                       cmp      byte ptr [esi + 0xd], 0
30088119 0f84e3000000                   je       0x30088202
3008811f f30f7e4610                     movq     xmm0, qword ptr [esi + 0x10]
30088124 8b4e18                         mov      ecx, dword ptr [esi + 0x18]
30088127 f30f104808                     movss    xmm1, dword ptr [eax + 8]
3008812c 660fd645e8                     movq     qword ptr [ebp - 0x18], xmm0
30088131 f30f5c5de8                     subss    xmm3, dword ptr [ebp - 0x18]
30088136 f30f104004                     movss    xmm0, dword ptr [eax + 4]
3008813b f30f5c45ec                     subss    xmm0, dword ptr [ebp - 0x14]
30088140 0f28e0                         movaps   xmm4, xmm0
30088143 f30f59e0                       mulss    xmm4, xmm0
30088147 0f28d3                         movaps   xmm2, xmm3
3008814a f30f59d3                       mulss    xmm2, xmm3
3008814e f30f58d4                       addss    xmm2, xmm4
30088152 894df0                         mov      dword ptr [ebp - 0x10], ecx
30088155 f30f5c4df0                     subss    xmm1, dword ptr [ebp - 0x10]
3008815a 0f28e1                         movaps   xmm4, xmm1
3008815d f30f59e1                       mulss    xmm4, xmm1
30088161 f30f58d4                       addss    xmm2, xmm4
30088165 f30f58151c552530               addss    xmm2, dword ptr [0x3025551c]
3008816d 0f57e4                         xorps    xmm4, xmm4
30088170 f30f10e2                       movss    xmm4, xmm2
30088174 f30f52e4                       rsqrtss  xmm4, xmm4
30088178 0f28f4                         movaps   xmm6, xmm4
3008817b f30f59f2                       mulss    xmm6, xmm2
3008817f f30f10152cd42430               movss    xmm2, dword ptr [0x3024d42c]
30088187 f30f59f4                       mulss    xmm6, xmm4
3008818b f30f593590c52430               mulss    xmm6, dword ptr [0x3024c590]
30088193 f30f5cd6                       subss    xmm2, xmm6
30088197 f30f59d4                       mulss    xmm2, xmm4
3008819b f30f59c2                       mulss    xmm0, xmm2
3008819f f30f59da                       mulss    xmm3, xmm2
300881a3 f30f59ca                       mulss    xmm1, xmm2
300881a7 f30f105658                     movss    xmm2, dword ptr [esi + 0x58]
300881ac f30f59d5                       mulss    xmm2, xmm5
300881b0 0f28e3                         movaps   xmm4, xmm3
300881b3 f30f59e2                       mulss    xmm4, xmm2
300881b7 f30f1165e8                     movss    dword ptr [ebp - 0x18], xmm4
300881bc 0f28e0                         movaps   xmm4, xmm0
300881bf f30f59e2                       mulss    xmm4, xmm2
300881c3 f30f1165ec                     movss    dword ptr [ebp - 0x14], xmm4
300881c8 0f28e1                         movaps   xmm4, xmm1
300881cb f30f59e2                       mulss    xmm4, xmm2
300881cf f30f105664                     movss    xmm2, dword ptr [esi + 0x64]
300881d4 f30f59d5                       mulss    xmm2, xmm5
300881d8 f30f59c2                       mulss    xmm0, xmm2
300881dc f30f59da                       mulss    xmm3, xmm2
300881e0 f30f59ca                       mulss    xmm1, xmm2
300881e4 f30f115ddc                     movss    dword ptr [ebp - 0x24], xmm3
300881e9 f30f1145e0                     movss    dword ptr [ebp - 0x20], xmm0
300881ee f30f7e45dc                     movq     xmm0, qword ptr [ebp - 0x24]
300881f3 f30f114de4                     movss    dword ptr [ebp - 0x1c], xmm1
300881f8 8b4de4                         mov      ecx, dword ptr [ebp - 0x1c]
300881fb f30f1165f0                     movss    dword ptr [ebp - 0x10], xmm4
30088200 eb7c                           jmp      0x3008827e
30088202 f30f7e4650                     movq     xmm0, qword ptr [esi + 0x50]
30088207 8b4658                         mov      eax, dword ptr [esi + 0x58]
3008820a 8b4e64                         mov      ecx, dword ptr [esi + 0x64]
3008820d 660fd645dc                     movq     qword ptr [ebp - 0x24], xmm0
30088212 f30f1045dc                     movss    xmm0, dword ptr [ebp - 0x24]
30088217 f30f59c5                       mulss    xmm0, xmm5
3008821b f30f1145e8                     movss    dword ptr [ebp - 0x18], xmm0
30088220 f30f1045e0                     movss    xmm0, dword ptr [ebp - 0x20]
30088225 f30f59c5                       mulss    xmm0, xmm5
30088229 f30f1145ec                     movss    dword ptr [ebp - 0x14], xmm0
3008822e 8945e4                         mov      dword ptr [ebp - 0x1c], eax
30088231 f30f1045e4                     movss    xmm0, dword ptr [ebp - 0x1c]
30088236 f30f59c5                       mulss    xmm0, xmm5
3008823a f30f1145f0                     movss    dword ptr [ebp - 0x10], xmm0
3008823f f30f7e465c                     movq     xmm0, qword ptr [esi + 0x5c]
30088244 660fd645dc                     movq     qword ptr [ebp - 0x24], xmm0
30088249 f30f1045dc                     movss    xmm0, dword ptr [ebp - 0x24]
3008824e f30f59c5                       mulss    xmm0, xmm5
30088252 f30f1145d0                     movss    dword ptr [ebp - 0x30], xmm0
30088257 f30f1045e0                     movss    xmm0, dword ptr [ebp - 0x20]
3008825c f30f59c5                       mulss    xmm0, xmm5
30088260 f30f1145d4                     movss    dword ptr [ebp - 0x2c], xmm0
30088265 894de4                         mov      dword ptr [ebp - 0x1c], ecx
30088268 f30f1045e4                     movss    xmm0, dword ptr [ebp - 0x1c]
3008826d f30f59c5                       mulss    xmm0, xmm5
30088271 f30f1145d8                     movss    dword ptr [ebp - 0x28], xmm0
30088276 f30f7e45d0                     movq     xmm0, qword ptr [ebp - 0x30]
3008827b 8b4dd8                         mov      ecx, dword ptr [ebp - 0x28]
3008827e f6460402                       test     byte ptr [esi + 4], 2
30088282 8b45f0                         mov      eax, dword ptr [ebp - 0x10]
30088285 f30f7e65e8                     movq     xmm4, qword ptr [ebp - 0x18]
3008828a 894df0                         mov      dword ptr [ebp - 0x10], ecx
3008828d 660fd645e8                     movq     qword ptr [ebp - 0x18], xmm0
30088292 740a                           je       0x3008829e
30088294 8b4d0c                         mov      ecx, dword ptr [ebp + 0xc]
30088297 660fd621                       movq     qword ptr [ecx], xmm4
3008829b 894108                         mov      dword ptr [ecx + 8], eax
3008829e f6460404                       test     byte ptr [esi + 4], 4
300882a2 742e                           je       0x300882d2
300882a4 8b4510                         mov      eax, dword ptr [ebp + 0x10]
300882a7 f30f1000                       movss    xmm0, dword ptr [eax]
300882ab f30f5845e8                     addss    xmm0, dword ptr [ebp - 0x18]
300882b0 f30f1100                       movss    dword ptr [eax], xmm0
300882b4 f30f104004                     movss    xmm0, dword ptr [eax + 4]
300882b9 f30f5845ec                     addss    xmm0, dword ptr [ebp - 0x14]
300882be f30f114004                     movss    dword ptr [eax + 4], xmm0
300882c3 f30f104008                     movss    xmm0, dword ptr [eax + 8]
300882c8 f30f5845f0                     addss    xmm0, dword ptr [ebp - 0x10]
300882cd f30f114008                     movss    dword ptr [eax + 8], xmm0
300882d2 c745fcffffffff                 mov      dword ptr [ebp - 4], 0xffffffff
300882d9 85d2                           test     edx, edx
300882db 7414                           je       0x300882f1
300882dd a168813530                     mov      eax, dword ptr [0x30358168]
300882e2 8b889c000000                   mov      ecx, dword ptr [eax + 0x9c]
300882e8 8d55b8                         lea      edx, [ebp - 0x48]
300882eb 52                             push     edx
300882ec ffd1                           call     ecx
300882ee 83c404                         add      esp, 4
300882f1 8b4df4                         mov      ecx, dword ptr [ebp - 0xc]
300882f4 5e                             pop      esi
300882f5 64890d00000000                 mov      dword ptr fs:[0], ecx
300882fc 8be5                           mov      esp, ebp
300882fe 5d                             pop      ebp
300882ff c20c00                         ret      0xc
30088302 cc                             int3
30088303 cc                             int3
30088304 cc                             int3
30088305 cc                             int3
30088306 cc                             int3
30088307 cc                             int3
30088308 cc                             int3
30088309 cc                             int3
3008830a cc                             int3
3008830b cc                             int3
3008830c cc                             int3
3008830d cc                             int3
3008830e cc                             int3
3008830f cc                             int3

# 30087dc0..30087f40
30087dc0 55                             push     ebp
30087dc1 8bec                           mov      ebp, esp
30087dc3 64a100000000                   mov      eax, dword ptr fs:[0]
30087dc9 6aff                           push     -1
30087dcb 68f6ab2330                     push     0x3023abf6
30087dd0 50                             push     eax
30087dd1 64892500000000                 mov      dword ptr fs:[0], esp
30087dd8 81eca0000000                   sub      esp, 0xa0
30087dde 53                             push     ebx
30087ddf bb01000000                     mov      ebx, 1
30087de4 56                             push     esi
30087de5 8bf1                           mov      esi, ecx
30087de7 841d308c2a30                   test     byte ptr [0x302a8c30], bl
30087ded 752d                           jne      0x30087e1c
30087def 091d308c2a30                   or       dword ptr [0x302a8c30], ebx
30087df5 a168813530                     mov      eax, dword ptr [0x30358168]
30087dfa 8b08                           mov      ecx, dword ptr [eax]
30087dfc 6a0c                           push     0xc
30087dfe 6864a62530                     push     0x3025a664
30087e03 51                             push     ecx
30087e04 b9b0882a30                     mov      ecx, 0x302a88b0
30087e09 c745fc00000000                 mov      dword ptr [ebp - 4], 0
30087e10 e8abf5f8ff                     call     0x300173c0
30087e15 c745fcffffffff                 mov      dword ptr [ebp - 4], 0xffffffff
30087e1c a168813530                     mov      eax, dword ptr [0x30358168]
30087e21 660f57c0                       xorpd    xmm0, xmm0
30087e25 660f1345dc                     movlpd   qword ptr [ebp - 0x24], xmm0
30087e2a 660f57c0                       xorpd    xmm0, xmm0
30087e2e 660f1345e4                     movlpd   qword ptr [ebp - 0x1c], xmm0
30087e33 c745ec00000000                 mov      dword ptr [ebp - 0x14], 0
30087e3a c745f000000000                 mov      dword ptr [ebp - 0x10], 0
30087e41 80b89700000000                 cmp      byte ptr [eax + 0x97], 0
30087e48 7416                           je       0x30087e60
30087e4a 8d55dc                         lea      edx, [ebp - 0x24]
30087e4d c745ecb0882a30                 mov      dword ptr [ebp - 0x14], 0x302a88b0
30087e54 8b8098000000                   mov      eax, dword ptr [eax + 0x98]
30087e5a 52                             push     edx
30087e5b ffd0                           call     eax
30087e5d 83c404                         add      esp, 4
30087e60 8d8d54ffffff                   lea      ecx, [ebp - 0xac]
30087e66 895dfc                         mov      dword ptr [ebp - 4], ebx
30087e69 e8821b0000                     call     0x300899f0
30087e6e 8b4508                         mov      eax, dword ptr [ebp + 8]
30087e71 8b4808                         mov      ecx, dword ptr [eax + 8]
30087e74 f30f7e00                       movq     xmm0, qword ptr [eax]
30087e78 898d60ffffff                   mov      dword ptr [ebp - 0xa0], ecx
30087e7e 8b0e                           mov      ecx, dword ptr [esi]
30087e80 660fd68558ffffff               movq     qword ptr [ebp - 0xa8], xmm0
30087e88 8b11                           mov      edx, dword ptr [ecx]
30087e8a 8b5224                         mov      edx, dword ptr [edx + 0x24]
30087e8d 8d8554ffffff                   lea      eax, [ebp - 0xac]
30087e93 50                             push     eax
30087e94 ffd2                           call     edx
30087e96 85c0                           test     eax, eax
30087e98 7465                           je       0x30087eff
30087e9a 8b4580                         mov      eax, dword ptr [ebp - 0x80]
30087e9d 250000a0ff                     and      eax, 0xffa00000
30087ea2 3d0000a0ff                     cmp      eax, 0xffa00000
30087ea7 7412                           je       0x30087ebb
30087ea9 8b450c                         mov      eax, dword ptr [ebp + 0xc]
30087eac f30f7e4580                     movq     xmm0, qword ptr [ebp - 0x80]
30087eb1 8b4d88                         mov      ecx, dword ptr [ebp - 0x78]
30087eb4 660fd600                       movq     qword ptr [eax], xmm0
30087eb8 894808                         mov      dword ptr [eax + 8], ecx
30087ebb 395dd4                         cmp      dword ptr [ebp - 0x2c], ebx
30087ebe 753f                           jne      0x30087eff
30087ec0 8b55a4                         mov      edx, dword ptr [ebp - 0x5c]
30087ec3 81e20000a0ff                   and      edx, 0xffa00000
30087ec9 81fa0000a0ff                   cmp      edx, 0xffa00000
30087ecf 742e                           je       0x30087eff
30087ed1 8b4510                         mov      eax, dword ptr [ebp + 0x10]
30087ed4 f30f1000                       movss    xmm0, dword ptr [eax]
30087ed8 f30f5845a4                     addss    xmm0, dword ptr [ebp - 0x5c]
30087edd f30f1100                       movss    dword ptr [eax], xmm0
30087ee1 f30f104004                     movss    xmm0, dword ptr [eax + 4]
30087ee6 f30f5845a8                     addss    xmm0, dword ptr [ebp - 0x58]
30087eeb f30f114004                     movss    dword ptr [eax + 4], xmm0
30087ef0 f30f104008                     movss    xmm0, dword ptr [eax + 8]
30087ef5 f30f5845ac                     addss    xmm0, dword ptr [ebp - 0x54]
30087efa f30f114008                     movss    dword ptr [eax + 8], xmm0
30087eff 837dec00                       cmp      dword ptr [ebp - 0x14], 0
30087f03 5e                             pop      esi
30087f04 c745fcffffffff                 mov      dword ptr [ebp - 4], 0xffffffff
30087f0b 5b                             pop      ebx
30087f0c 7415                           je       0x30087f23
30087f0e 8b0d68813530                   mov      ecx, dword ptr [0x30358168]
30087f14 8b919c000000                   mov      edx, dword ptr [ecx + 0x9c]
30087f1a 8d45dc                         lea      eax, [ebp - 0x24]
30087f1d 50                             push     eax
30087f1e ffd2                           call     edx
30087f20 83c404                         add      esp, 4
30087f23 8b4df4                         mov      ecx, dword ptr [ebp - 0xc]
30087f26 64890d00000000                 mov      dword ptr fs:[0], ecx
30087f2d 8be5                           mov      esp, ebp
30087f2f 5d                             pop      ebp
30087f30 c20c00                         ret      0xc
30087f33 cc                             int3
30087f34 cc                             int3
30087f35 cc                             int3
30087f36 cc                             int3
30087f37 cc                             int3
30087f38 cc                             int3
30087f39 cc                             int3
30087f3a cc                             int3
30087f3b cc                             int3
30087f3c cc                             int3
30087f3d cc                             int3
30087f3e cc                             int3
30087f3f cc                             int3
