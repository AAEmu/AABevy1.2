30089f80 55                       push ebp
30089f81 8bec                     mov ebp, esp
30089f83 a168813530               mov eax, dword ptr [0x30358168]
30089f88 83ec14                   sub esp, 0x14
30089f8b 56                       push esi
30089f8c 8bf1                     mov esi, ecx
30089f8e 8b8890000000             mov ecx, dword ptr [eax + 0x90]
30089f94 2b4e78                   sub ecx, dword ptr [esi + 0x78]
30089f97 83f903                   cmp ecx, 3
30089f9a 7632                     jbe 0x30089fce
30089f9c 8b4610                   mov eax, dword ptr [esi + 0x10]
30089f9f 8b0d20cd2d30             mov ecx, dword ptr [0x302dcd20]
30089fa5 0590000000               add eax, 0x90
30089faa 50                       push eax
30089fab 8b01                     mov eax, dword ptr [ecx]
30089fad 8b4024                   mov eax, dword ptr [eax + 0x24]
30089fb0 8d55ec                   lea edx, [ebp - 0x14]
30089fb3 52                       push edx
30089fb4 6a00                     push 0
30089fb6 8d55f4                   lea edx, [ebp - 0xc]
30089fb9 52                       push edx
30089fba ffd0                     call eax
30089fbc 8bc8                     mov ecx, eax
30089fbe e8dd5ffdff               call 0x3005ffa0
30089fc3 df28                     fild qword ptr [eax]
30089fc5 d80da4692530             fmul dword ptr [0x302569a4]
30089fcb d95e74                   fstp dword ptr [esi + 0x74]
30089fce f30f104674               movss xmm0, dword ptr [esi + 0x74]
30089fd3 0f2f463c                 comiss xmm0, dword ptr [esi + 0x3c]
30089fd7 f30f1145f0               movss dword ptr [ebp - 0x10], xmm0
30089fdc 7277                     jb 0x3008a055
30089fde 8b4e10                   mov ecx, dword ptr [esi + 0x10]
30089fe1 f30f108998000000         movss xmm1, dword ptr [ecx + 0x98]
30089fe9 f30f105640               movss xmm2, dword ptr [esi + 0x40]
30089fee 0f2fd1                   comiss xmm2, xmm1
30089ff1 8d4640                   lea eax, [esi + 0x40]
30089ff4 f30f114dfc               movss dword ptr [ebp - 4], xmm1
30089ff9 7603                     jbe 0x30089ffe
30089ffb 8d45fc                   lea eax, [ebp - 4]
30089ffe f30f1008                 movss xmm1, dword ptr [eax]
3008a002 0f2fc8                   comiss xmm1, xmm0
3008a005 f30f114df8               movss dword ptr [ebp - 8], xmm1
3008a00a 720a                     jb 0x3008a016
3008a00c b803000000               mov eax, 3
3008a011 5e                       pop esi
3008a012 8be5                     mov esp, ebp
3008a014 5d                       pop ebp
3008a015 c3                       ret
3008a016 8b4e08                   mov ecx, dword ptr [esi + 8]
3008a019 80790c00                 cmp byte ptr [ecx + 0xc], 0
3008a01d 740a                     je 0x3008a029
3008a01f b802000000               mov eax, 2
3008a024 5e                       pop esi
3008a025 8be5                     mov esp, ebp
3008a027 5d                       pop ebp
3008a028 c3                       ret
3008a029 6a01                     push 1
3008a02b e8a07bfdff               call 0x30061bd0
3008a030 d955fc                   fst dword ptr [ebp - 4]
3008a033 d9ee                     fldz
3008a035 d9c9                     fxch st(1)
3008a037 dfe9                     fucompi st(1)
3008a039 ddd8                     fstp st(0)
3008a03b 9f                       lahf
3008a03c f6c444                   test ah, 0x44
3008a03f 7bde                     jnp 0x3008a01f
3008a041 f30f104dfc               movss xmm1, dword ptr [ebp - 4]
3008a046 f30f584df8               addss xmm1, dword ptr [ebp - 8]
3008a04b f30f1045f0               movss xmm0, dword ptr [ebp - 0x10]
3008a050 0f2fc8                   comiss xmm1, xmm0
3008a053 73ca                     jae 0x3008a01f
3008a055 8b4610                   mov eax, dword ptr [esi + 0x10]
3008a058 80b89c00000000           cmp byte ptr [eax + 0x9c], 0
3008a05f 750d                     jne 0x3008a06e
3008a061 f30f108898000000         movss xmm1, dword ptr [eax + 0x98]
3008a069 0f2fc8                   comiss xmm1, xmm0
3008a06c 722f                     jb 0x3008a09d
3008a06e f30f104e3c               movss xmm1, dword ptr [esi + 0x3c]
3008a073 0f2fc8                   comiss xmm1, xmm0
3008a076 7732                     ja 0x3008a0aa
3008a078 f30f104840               movss xmm1, dword ptr [eax + 0x40]
3008a07d 0f57c0                   xorps xmm0, xmm0
3008a080 0f2ec8                   ucomiss xmm1, xmm0
3008a083 9f                       lahf
3008a084 f6c444                   test ah, 0x44
3008a087 7a21                     jp 0x3008a0aa
3008a089 8b5608                   mov edx, dword ptr [esi + 8]
3008a08c 8b4220                   mov eax, dword ptr [edx + 0x20]
3008a08f f30f104834               movss xmm1, dword ptr [eax + 0x34]
3008a094 0f2ec8                   ucomiss xmm1, xmm0
3008a097 9f                       lahf
3008a098 f6c444                   test ah, 0x44
3008a09b 7a0d                     jp 0x3008a0aa
3008a09d 33c0                     xor eax, eax
3008a09f 394634                   cmp dword ptr [esi + 0x34], eax
3008a0a2 5e                       pop esi
3008a0a3 0f95c0                   setne al
3008a0a6 8be5                     mov esp, ebp
3008a0a8 5d                       pop ebp
3008a0a9 c3                       ret
3008a0aa b801000000               mov eax, 1
3008a0af 5e                       pop esi
3008a0b0 8be5                     mov esp, ebp
3008a0b2 5d                       pop ebp
3008a0b3 c3                       ret
3008a0b4 cc                       int3
3008a0b5 cc                       int3
3008a0b6 cc                       int3
3008a0b7 cc                       int3
3008a0b8 cc                       int3
3008a0b9 cc                       int3
3008a0ba cc                       int3
3008a0bb cc                       int3
3008a0bc cc                       int3
3008a0bd cc                       int3
3008a0be cc                       int3
3008a0bf cc                       int3
3008a3b0 55                       push ebp
3008a3b1 8bec                     mov ebp, esp
3008a3b3 83ec0c                   sub esp, 0xc
3008a3b6 53                       push ebx
3008a3b7 57                       push edi
3008a3b8 8bf9                     mov edi, ecx
3008a3ba 8b4708                   mov eax, dword ptr [edi + 8]
3008a3bd 8b5820                   mov ebx, dword ptr [eax + 0x20]
3008a3c0 80bbbd02000003           cmp byte ptr [ebx + 0x2bd], 3
3008a3c7 755b                     jne 0x3008a424
3008a3c9 e8b2fbffff               call 0x30089f80
3008a3ce 83f803                   cmp eax, 3
3008a3d1 7c51                     jl 0x3008a424
3008a3d3 56                       push esi
3008a3d4 8b7508                   mov esi, dword ptr [ebp + 8]
3008a3d7 81c370010000             add ebx, 0x170
3008a3dd 53                       push ebx
3008a3de 8d4df4                   lea ecx, [ebp - 0xc]
3008a3e1 c6461801                 mov byte ptr [esi + 0x18], 1
3008a3e5 c6461c01                 mov byte ptr [esi + 0x1c], 1
3008a3e9 51                       push ecx
3008a3ea 8b4f10                   mov ecx, dword ptr [edi + 0x10]
3008a3ed e87e460000               call 0x3008ea70
3008a3f2 f30f7e00                 movq xmm0, qword ptr [eax]
3008a3f6 660fd606                 movq qword ptr [esi], xmm0
3008a3fa 8b5008                   mov edx, dword ptr [eax + 8]
3008a3fd 895608                   mov dword ptr [esi + 8], edx
3008a400 8b4710                   mov eax, dword ptr [edi + 0x10]
3008a403 f30f7e8080000000         movq xmm0, qword ptr [eax + 0x80]
3008a40b 660fd6460c               movq qword ptr [esi + 0xc], xmm0
3008a410 8b8088000000             mov eax, dword ptr [eax + 0x88]
3008a416 894614                   mov dword ptr [esi + 0x14], eax
3008a419 5e                       pop esi
3008a41a 5f                       pop edi
3008a41b b001                     mov al, 1
3008a41d 5b                       pop ebx
3008a41e 8be5                     mov esp, ebp
3008a420 5d                       pop ebp
3008a421 c20400                   ret 4
3008a424 5f                       pop edi
3008a425 32c0                     xor al, al
3008a427 5b                       pop ebx
3008a428 8be5                     mov esp, ebp
3008a42a 5d                       pop ebp
3008a42b c20400                   ret 4
3008a42e cc                       int3
3008a42f cc                       int3
3008a430 55                       push ebp
3008a431 8bec                     mov ebp, esp
3008a433 83ec20                   sub esp, 0x20
3008a436 56                       push esi
3008a437 8bf1                     mov esi, ecx
3008a439 8b4608                   mov eax, dword ptr [esi + 8]
3008a43c 8b4820                   mov ecx, dword ptr [eax + 0x20]
3008a43f 80b9d602000000           cmp byte ptr [ecx + 0x2d6], 0
3008a446 741d                     je 0x3008a465
3008a448 660fefc0                 pxor xmm0, xmm0
3008a44c 660fd64614               movq qword ptr [esi + 0x14], xmm0
3008a451 660fd6461c               movq qword ptr [esi + 0x1c], xmm0
3008a456 660fd64624               movq qword ptr [esi + 0x24], xmm0
3008a45b 660fd6462c               movq qword ptr [esi + 0x2c], xmm0
3008a460 5e                       pop esi
3008a461 8be5                     mov esp, ebp
3008a463 5d                       pop ebp
3008a464 c3                       ret
3008a465 807e2d00                 cmp byte ptr [esi + 0x2d], 0
3008a469 8b5610                   mov edx, dword ptr [esi + 0x10]
3008a46c 8b427c                   mov eax, dword ptr [edx + 0x7c]
3008a46f f30f7e8064010000         movq xmm0, qword ptr [eax + 0x164]
3008a477 660fd645e0               movq qword ptr [ebp - 0x20], xmm0
3008a47c f30f7e806c010000         movq xmm0, qword ptr [eax + 0x16c]
3008a484 660fd645e8               movq qword ptr [ebp - 0x18], xmm0
3008a489 f30f7e8074010000         movq xmm0, qword ptr [eax + 0x174]
3008a491 660fd645f0               movq qword ptr [ebp - 0x10], xmm0
3008a496 f30f7e807c010000         movq xmm0, qword ptr [eax + 0x17c]
3008a49e 660fd645f8               movq qword ptr [ebp - 8], xmm0
3008a4a3 7522                     jne 0x3008a4c7
3008a4a5 57                       push edi
3008a4a6 8b7e0c                   mov edi, dword ptr [esi + 0xc]
3008a4a9 85ff                     test edi, edi
3008a4ab 7419                     je 0x3008a4c6
3008a4ad 8d4900                   lea ecx, [ecx]
3008a4b0 8d45e0                   lea eax, [ebp - 0x20]
3008a4b3 50                       push eax
3008a4b4 8bcf                     mov ecx, edi
3008a4b6 e8f5feffff               call 0x3008a3b0
3008a4bb 84c0                     test al, al
3008a4bd 7507                     jne 0x3008a4c6
3008a4bf 8b7f0c                   mov edi, dword ptr [edi + 0xc]
3008a4c2 85ff                     test edi, edi
3008a4c4 75ea                     jne 0x3008a4b0
3008a4c6 5f                       pop edi
3008a4c7 8b4e08                   mov ecx, dword ptr [esi + 8]
3008a4ca 83791000                 cmp dword ptr [ecx + 0x10], 0
3008a4ce 7508                     jne 0x3008a4d8
3008a4d0 8a55f8                   mov dl, byte ptr [ebp - 8]
3008a4d3 3a562c                   cmp dl, byte ptr [esi + 0x2c]
3008a4d6 752d                     jne 0x3008a505
3008a4d8 f30f1045e0               movss xmm0, dword ptr [ebp - 0x20]
3008a4dd 0f2e4614                 ucomiss xmm0, dword ptr [esi + 0x14]
3008a4e1 9f                       lahf
3008a4e2 f6c444                   test ah, 0x44
3008a4e5 7a1e                     jp 0x3008a505
3008a4e7 f30f1045e4               movss xmm0, dword ptr [ebp - 0x1c]
3008a4ec 0f2e4618                 ucomiss xmm0, dword ptr [esi + 0x18]
3008a4f0 9f                       lahf
3008a4f1 f6c444                   test ah, 0x44
3008a4f4 7a0f                     jp 0x3008a505
3008a4f6 f30f1045e8               movss xmm0, dword ptr [ebp - 0x18]
3008a4fb 0f2e461c                 ucomiss xmm0, dword ptr [esi + 0x1c]
3008a4ff 9f                       lahf
3008a500 f6c444                   test ah, 0x44
3008a503 7b05                     jnp 0x3008a50a
3008a505 e8867afdff               call 0x30061f90
3008a50a f30f7e45e0               movq xmm0, qword ptr [ebp - 0x20]
3008a50f 660fd64614               movq qword ptr [esi + 0x14], xmm0
3008a514 f30f7e45e8               movq xmm0, qword ptr [ebp - 0x18]
3008a519 660fd6461c               movq qword ptr [esi + 0x1c], xmm0
3008a51e f30f7e45f0               movq xmm0, qword ptr [ebp - 0x10]
3008a523 660fd64624               movq qword ptr [esi + 0x24], xmm0
3008a528 f30f7e45f8               movq xmm0, qword ptr [ebp - 8]
3008a52d 660fd6462c               movq qword ptr [esi + 0x2c], xmm0
3008a532 5e                       pop esi
3008a533 8be5                     mov esp, ebp
3008a535 5d                       pop ebp
3008a536 c3                       ret
3008a537 cc                       int3
3008a538 cc                       int3
3008a539 cc                       int3
3008a53a cc                       int3
3008a53b cc                       int3
3008a53c cc                       int3
3008a53d cc                       int3
3008a53e cc                       int3
3008a53f cc                       int3
3008ea70 53                       push ebx
3008ea71 8bdc                     mov ebx, esp
3008ea73 83ec08                   sub esp, 8
3008ea76 83e4f0                   and esp, 0xfffffff0
3008ea79 83c404                   add esp, 4
3008ea7c 55                       push ebp
3008ea7d 8b6b04                   mov ebp, dword ptr [ebx + 4]
3008ea80 896c2404                 mov dword ptr [esp + 4], ebp
3008ea84 8bec                     mov ebp, esp
3008ea86 83ec10                   sub esp, 0x10
3008ea89 f30f105914               movss xmm3, dword ptr [ecx + 0x14]
3008ea8e f30f105110               movss xmm2, dword ptr [ecx + 0x10]
3008ea93 f30f104908               movss xmm1, dword ptr [ecx + 8]
3008ea98 f30f10410c               movss xmm0, dword ptr [ecx + 0xc]
3008ea9d 8b430c                   mov eax, dword ptr [ebx + 0xc]
3008eaa0 0f57e4                   xorps xmm4, xmm4
3008eaa3 0f14ca                   unpcklps xmm1, xmm2
3008eaa6 f30f1010                 movss xmm2, dword ptr [eax]
3008eaaa 0f14c3                   unpcklps xmm0, xmm3
3008eaad f30f105808               movss xmm3, dword ptr [eax + 8]
3008eab2 0f14c8                   unpcklps xmm1, xmm0
3008eab5 f30f104004               movss xmm0, dword ptr [eax + 4]
3008eaba 8b4308                   mov eax, dword ptr [ebx + 8]
3008eabd 0f14d3                   unpcklps xmm2, xmm3
3008eac0 0f28d9                   movaps xmm3, xmm1
3008eac3 0f14c4                   unpcklps xmm0, xmm4
3008eac6 0f14d0                   unpcklps xmm2, xmm0
3008eac9 0fc6d9c9                 shufps xmm3, xmm1, 0xc9
3008eacd 0f28e1                   movaps xmm4, xmm1
3008ead0 0fc6e112                 shufps xmm4, xmm1, 0x12
3008ead4 0fc6c9ff                 shufps xmm1, xmm1, 0xff
3008ead8 0f59ca                   mulps xmm1, xmm2
3008eadb 0f28c2                   movaps xmm0, xmm2
3008eade 0fc6c2c9                 shufps xmm0, xmm2, 0xc9
3008eae2 0f59c4                   mulps xmm0, xmm4
3008eae5 0f570520972a30           xorps xmm0, xmmword ptr [0x302a9720]
3008eaec 0f28f2                   movaps xmm6, xmm2
3008eaef 0fc6f212                 shufps xmm6, xmm2, 0x12
3008eaf3 0f59f3                   mulps xmm6, xmm3
3008eaf6 0f58c6                   addps xmm0, xmm6
3008eaf9 0f58c1                   addps xmm0, xmm1
3008eafc 0f28c8                   movaps xmm1, xmm0
3008eaff 0fc6c8c9                 shufps xmm1, xmm0, 0xc9
3008eb03 0fc6c0d2                 shufps xmm0, xmm0, 0xd2
3008eb07 0f59c3                   mulps xmm0, xmm3
3008eb0a 0f59cc                   mulps xmm1, xmm4
3008eb0d 0f570d20972a30           xorps xmm1, xmmword ptr [0x302a9720]
3008eb14 0f58c8                   addps xmm1, xmm0
3008eb17 f30f104124               movss xmm0, dword ptr [ecx + 0x24]
3008eb1c 0f58c9                   addps xmm1, xmm1
3008eb1f 0f58ca                   addps xmm1, xmm2
3008eb22 0f294df0                 movaps xmmword ptr [ebp - 0x10], xmm1
3008eb26 f30f104df0               movss xmm1, dword ptr [ebp - 0x10]
3008eb2b f30f1055f4               movss xmm2, dword ptr [ebp - 0xc]
3008eb30 f30f105df8               movss xmm3, dword ptr [ebp - 8]
3008eb35 f30f59c8                 mulss xmm1, xmm0
3008eb39 f30f59d0                 mulss xmm2, xmm0
3008eb3d f30f59d8                 mulss xmm3, xmm0
3008eb41 f30f104118               movss xmm0, dword ptr [ecx + 0x18]
3008eb46 f30f58c1                 addss xmm0, xmm1
3008eb4a f30f1100                 movss dword ptr [eax], xmm0
3008eb4e f30f10411c               movss xmm0, dword ptr [ecx + 0x1c]
3008eb53 f30f58c2                 addss xmm0, xmm2
3008eb57 f30f114004               movss dword ptr [eax + 4], xmm0
3008eb5c f30f104120               movss xmm0, dword ptr [ecx + 0x20]
3008eb61 f30f58c3                 addss xmm0, xmm3
3008eb65 f30f114008               movss dword ptr [eax + 8], xmm0
3008eb6a 8be5                     mov esp, ebp
3008eb6c 5d                       pop ebp
3008eb6d 8be3                     mov esp, ebx
3008eb6f 5b                       pop ebx
3008eb70 c20800                   ret 8
3008eb73 cc                       int3
3008eb74 cc                       int3
3008eb75 cc                       int3
3008eb76 cc                       int3
3008eb77 cc                       int3
3008eb78 cc                       int3
3008eb79 cc                       int3
3008eb7a cc                       int3
3008eb7b cc                       int3
3008eb7c cc                       int3
3008eb7d cc                       int3
3008eb7e cc                       int3
3008eb7f cc                       int3
30060420 55                       push ebp
30060421 8bec                     mov ebp, esp
30060423 f30f104d1c               movss xmm1, dword ptr [ebp + 0x1c]
30060428 83ec0c                   sub esp, 0xc
3006042b 0f2f0da4692530           comiss xmm1, dword ptr [0x302569a4]
30060432 0f86d5000000             jbe 0x3006050d
30060438 f30f104514               movss xmm0, dword ptr [ebp + 0x14]
3006043d f30f59c1                 mulss xmm0, xmm1
30060441 0f570560c52430           xorps xmm0, xmmword ptr [0x3024c560]
30060448 e8ab631d00               call 0x302367f8
3006044d f30f102d70c52430         movss xmm5, dword ptr [0x3024c570]
30060455 8b4510                   mov eax, dword ptr [ebp + 0x10]
30060458 f30f10651c               movss xmm4, dword ptr [ebp + 0x1c]
3006045d 0f28c8                   movaps xmm1, xmm0
30060460 0f28c5                   movaps xmm0, xmm5
30060463 f30f5cc1                 subss xmm0, xmm1
30060467 f30f7e08                 movq xmm1, qword ptr [eax]
3006046b 8b4008                   mov eax, dword ptr [eax + 8]
3006046e 660fd64df4               movq qword ptr [ebp - 0xc], xmm1
30060473 f30f1055f4               movss xmm2, dword ptr [ebp - 0xc]
30060478 f30f105df8               movss xmm3, dword ptr [ebp - 8]
3006047d 8945fc                   mov dword ptr [ebp - 4], eax
30060480 8b450c                   mov eax, dword ptr [ebp + 0xc]
30060483 f30f1008                 movss xmm1, dword ptr [eax]
30060487 f30f1075fc               movss xmm6, dword ptr [ebp - 4]
3006048c f30f5ec4                 divss xmm0, xmm4
30060490 f30f59d0                 mulss xmm2, xmm0
30060494 f30f5cca                 subss xmm1, xmm2
30060498 f30f105004               movss xmm2, dword ptr [eax + 4]
3006049d f30f59d8                 mulss xmm3, xmm0
300604a1 f30f5cd3                 subss xmm2, xmm3
300604a5 f30f105808               movss xmm3, dword ptr [eax + 8]
300604aa 8b4518                   mov eax, dword ptr [ebp + 0x18]
300604ad 8b4808                   mov ecx, dword ptr [eax + 8]
300604b0 f30f59f0                 mulss xmm6, xmm0
300604b4 f30f5cde                 subss xmm3, xmm6
300604b8 f30f107514               movss xmm6, dword ptr [ebp + 0x14]
300604bd f30f5cf0                 subss xmm6, xmm0
300604c1 f30f7e00                 movq xmm0, qword ptr [eax]
300604c5 8b4508                   mov eax, dword ptr [ebp + 8]
300604c8 f30f5eee                 divss xmm5, xmm6
300604cc f30f59cd                 mulss xmm1, xmm5
300604d0 f30f59d5                 mulss xmm2, xmm5
300604d4 f30f59dd                 mulss xmm3, xmm5
300604d8 660fd645f4               movq qword ptr [ebp - 0xc], xmm0
300604dd f30f5c4df4               subss xmm1, dword ptr [ebp - 0xc]
300604e2 f30f5c55f8               subss xmm2, dword ptr [ebp - 8]
300604e7 894dfc                   mov dword ptr [ebp - 4], ecx
300604ea f30f5c5dfc               subss xmm3, dword ptr [ebp - 4]
300604ef f30f59cc                 mulss xmm1, xmm4
300604f3 f30f59d4                 mulss xmm2, xmm4
300604f7 f30f59dc                 mulss xmm3, xmm4
300604fb f30f1108                 movss dword ptr [eax], xmm1
300604ff f30f115004               movss dword ptr [eax + 4], xmm2
30060504 f30f115808               movss dword ptr [eax + 8], xmm3
30060509 8be5                     mov esp, ebp
3006050b 5d                       pop ebp
3006050c c3                       ret
3006050d 8b4510                   mov eax, dword ptr [ebp + 0x10]
30060510 f30f7e00                 movq xmm0, qword ptr [eax]
30060514 8b5008                   mov edx, dword ptr [eax + 8]
30060517 8b450c                   mov eax, dword ptr [ebp + 0xc]
3006051a f30f105d14               movss xmm3, dword ptr [ebp + 0x14]
3006051f 660fd645f4               movq qword ptr [ebp - 0xc], xmm0
30060524 f30f104df4               movss xmm1, dword ptr [ebp - 0xc]
30060529 f30f1055f8               movss xmm2, dword ptr [ebp - 8]
3006052e f30f1000                 movss xmm0, dword ptr [eax]
30060532 f30f59cb                 mulss xmm1, xmm3
30060536 f30f59d3                 mulss xmm2, xmm3
3006053a 8955fc                   mov dword ptr [ebp - 4], edx
3006053d f30f1065fc               movss xmm4, dword ptr [ebp - 4]
30060542 f30f59e3                 mulss xmm4, xmm3
30060546 f30f5cc1                 subss xmm0, xmm1
3006054a f30f104804               movss xmm1, dword ptr [eax + 4]
3006054f f30f59db                 mulss xmm3, xmm3
30060553 f30f591d90c52430         mulss xmm3, dword ptr [0x3024c590]
3006055b f30f5cca                 subss xmm1, xmm2
3006055f f30f105008               movss xmm2, dword ptr [eax + 8]
30060564 8b4508                   mov eax, dword ptr [ebp + 8]
30060567 f30f115d1c               movss dword ptr [ebp + 0x1c], xmm3
3006056c f30f101d70c52430         movss xmm3, dword ptr [0x3024c570]
30060574 f30f5e5d1c               divss xmm3, dword ptr [ebp + 0x1c]
30060579 f30f5cd4                 subss xmm2, xmm4
3006057d f30f59c3                 mulss xmm0, xmm3
30060581 f30f59cb                 mulss xmm1, xmm3
30060585 f30f59d3                 mulss xmm2, xmm3
30060589 f30f1100                 movss dword ptr [eax], xmm0
3006058d f30f114804               movss dword ptr [eax + 4], xmm1
30060592 f30f115008               movss dword ptr [eax + 8], xmm2
30060597 8be5                     mov esp, ebp
30060599 5d                       pop ebp
3006059a c3                       ret
3006059b cc                       int3
3006059c cc                       int3
3006059d cc                       int3
3006059e cc                       int3
3006059f cc                       int3
3008b410 55                       push ebp
3008b411 8bec                     mov ebp, esp
3008b413 0f57c0                   xorps xmm0, xmm0
3008b416 83ec1c                   sub esp, 0x1c
3008b419 53                       push ebx
3008b41a 56                       push esi
3008b41b 8bf1                     mov esi, ecx
3008b41d c6463800                 mov byte ptr [esi + 0x38], 0
3008b421 f30f114650               movss dword ptr [esi + 0x50], xmm0
3008b426 f30f114674               movss dword ptr [esi + 0x74], xmm0
3008b42b c7467800000000           mov dword ptr [esi + 0x78], 0
3008b432 ff15fcc22430             call dword ptr [0x3024c2fc]
3008b438 89464c                   mov dword ptr [esi + 0x4c], eax
3008b43b 8b4608                   mov eax, dword ptr [esi + 8]
3008b43e 8b5820                   mov ebx, dword ptr [eax + 0x20]
3008b441 80bb6501000003           cmp byte ptr [ebx + 0x165], 3
3008b448 7553                     jne 0x3008b49d
3008b44a d94508                   fld dword ptr [ebp + 8]
3008b44d 8b4e10                   mov ecx, dword ptr [esi + 0x10]
3008b450 d80d545a2530             fmul dword ptr [0x30255a54]
3008b456 57                       push edi
3008b457 8b3d20cd2d30             mov edi, dword ptr [0x302dcd20]
3008b45d 894df4                   mov dword ptr [ebp - 0xc], ecx
3008b460 e811b31a00               call 0x30236776
3008b465 8955f0                   mov dword ptr [ebp - 0x10], edx
3008b468 8945ec                   mov dword ptr [ebp - 0x14], eax
3008b46b 8d55ec                   lea edx, [ebp - 0x14]
3008b46e 52                       push edx
3008b46f 8b17                     mov edx, dword ptr [edi]
3008b471 8b5224                   mov edx, dword ptr [edx + 0x24]
3008b474 8d45f8                   lea eax, [ebp - 8]
3008b477 50                       push eax
3008b478 6a00                     push 0
3008b47a 8d45e4                   lea eax, [ebp - 0x1c]
3008b47d 50                       push eax
3008b47e 8bcf                     mov ecx, edi
3008b480 ffd2                     call edx
3008b482 8bc8                     mov ecx, eax
3008b484 e8174bfdff               call 0x3005ffa0
3008b489 8b4df4                   mov ecx, dword ptr [ebp - 0xc]
3008b48c 50                       push eax
3008b48d 8d45ec                   lea eax, [ebp - 0x14]
3008b490 50                       push eax
3008b491 81c190000000             add ecx, 0x90
3008b497 e844b0fdff               call 0x300664e0
3008b49c 5f                       pop edi
3008b49d 8b0d68813530             mov ecx, dword ptr [0x30358168]
3008b4a3 8b9190000000             mov edx, dword ptr [ecx + 0x90]
3008b4a9 2b5678                   sub edx, dword ptr [esi + 0x78]
3008b4ac 83fa03                   cmp edx, 3
3008b4af 7632                     jbe 0x3008b4e3
3008b4b1 8b4610                   mov eax, dword ptr [esi + 0x10]
3008b4b4 8b0d20cd2d30             mov ecx, dword ptr [0x302dcd20]
3008b4ba 8b11                     mov edx, dword ptr [ecx]
3008b4bc 8b5224                   mov edx, dword ptr [edx + 0x24]
3008b4bf 0590000000               add eax, 0x90
3008b4c4 50                       push eax
3008b4c5 8d45e4                   lea eax, [ebp - 0x1c]
3008b4c8 50                       push eax
3008b4c9 6a00                     push 0
3008b4cb 8d45ec                   lea eax, [ebp - 0x14]
3008b4ce 50                       push eax
3008b4cf ffd2                     call edx
3008b4d1 8bc8                     mov ecx, eax
3008b4d3 e8c84afdff               call 0x3005ffa0
3008b4d8 df28                     fild qword ptr [eax]
3008b4da d80da4692530             fmul dword ptr [0x302569a4]
3008b4e0 d95e74                   fstp dword ptr [esi + 0x74]
3008b4e3 8b464c                   mov eax, dword ptr [esi + 0x4c]
3008b4e6 f30f104674               movss xmm0, dword ptr [esi + 0x74]
3008b4eb 50                       push eax
3008b4ec 8d4b2c                   lea ecx, [ebx + 0x2c]
3008b4ef f30f1145fc               movss dword ptr [ebp - 4], xmm0
3008b4f4 e8a7360000               call 0x3008eba0
3008b4f9 d955f4                   fst dword ptr [ebp - 0xc]
3008b4fc d9ee                     fldz
3008b4fe dff1                     fcompi st(1)
3008b500 ddd8                     fstp st(0)
3008b502 7605                     jbe 0x3008b509
3008b504 0f57c9                   xorps xmm1, xmm1
3008b507 eb05                     jmp 0x3008b50e
3008b509 f30f104df4               movss xmm1, dword ptr [ebp - 0xc]
3008b50e f30f1045fc               movss xmm0, dword ptr [ebp - 4]
3008b513 f30f5c4508               subss xmm0, dword ptr [ebp + 8]
3008b518 f30f58c1                 addss xmm0, xmm1
3008b51c f30f114640               movss dword ptr [esi + 0x40], xmm0
3008b521 f30f11463c               movss dword ptr [esi + 0x3c], xmm0
3008b526 807b0500                 cmp byte ptr [ebx + 5], 0
3008b52a 7450                     je 0x3008b57c
3008b52c f30f104324               movss xmm0, dword ptr [ebx + 0x24]
3008b531 0f2f05cce62430           comiss xmm0, dword ptr [0x3024e6cc]
3008b538 8d4b24                   lea ecx, [ebx + 0x24]
3008b53b 7632                     jbe 0x3008b56f
3008b53d 8b564c                   mov edx, dword ptr [esi + 0x4c]
3008b540 52                       push edx
3008b541 e85a360000               call 0x3008eba0
3008b546 d955fc                   fst dword ptr [ebp - 4]
3008b549 d9ee                     fldz
3008b54b dff1                     fcompi st(1)
3008b54d ddd8                     fstp st(0)
3008b54f 7608                     jbe 0x3008b559
3008b551 0f57c0                   xorps xmm0, xmm0
3008b554 f30f1145fc               movss dword ptr [ebp - 4], xmm0
3008b559 8b0d4ccd2d30             mov ecx, dword ptr [0x302dcd4c]
3008b55f e8ac3d0000               call 0x3008f310
3008b564 d84dfc                   fmul dword ptr [ebp - 4]
3008b567 d84640                   fadd dword ptr [esi + 0x40]
3008b56a d95e40                   fstp dword ptr [esi + 0x40]
3008b56d eb0d                     jmp 0x3008b57c
3008b56f f30f1005f4792530         movss xmm0, dword ptr [0x302579f4]
3008b577 f30f114640               movss dword ptr [esi + 0x40], xmm0
3008b57c 8b464c                   mov eax, dword ptr [esi + 0x4c]
3008b57f 50                       push eax
3008b580 8d4b34                   lea ecx, [ebx + 0x34]
3008b583 e818360000               call 0x3008eba0
3008b588 d955fc                   fst dword ptr [ebp - 4]
3008b58b d9ee                     fldz
3008b58d d9c9                     fxch st(1)
3008b58f dff1                     fcompi st(1)
3008b591 ddd8                     fstp st(0)
3008b593 0f8616010000             jbe 0x3008b6af
3008b599 8b0d68813530             mov ecx, dword ptr [0x30358168]
3008b59f 8b9190000000             mov edx, dword ptr [ecx + 0x90]
3008b5a5 2b5678                   sub edx, dword ptr [esi + 0x78]
3008b5a8 83fa03                   cmp edx, 3
3008b5ab 7632                     jbe 0x3008b5df
3008b5ad 8b4610                   mov eax, dword ptr [esi + 0x10]
3008b5b0 8b0d20cd2d30             mov ecx, dword ptr [0x302dcd20]
3008b5b6 8b11                     mov edx, dword ptr [ecx]
3008b5b8 8b5224                   mov edx, dword ptr [edx + 0x24]
3008b5bb 0590000000               add eax, 0x90
3008b5c0 50                       push eax
3008b5c1 8d45e4                   lea eax, [ebp - 0x1c]
3008b5c4 50                       push eax
3008b5c5 6a00                     push 0
3008b5c7 8d45ec                   lea eax, [ebp - 0x14]
3008b5ca 50                       push eax
3008b5cb ffd2                     call edx
3008b5cd 8bc8                     mov ecx, eax
3008b5cf e8cc49fdff               call 0x3005ffa0
3008b5d4 df28                     fild qword ptr [eax]
3008b5d6 d80da4692530             fmul dword ptr [0x302569a4]
3008b5dc d95e74                   fstp dword ptr [esi + 0x74]
3008b5df f30f104674               movss xmm0, dword ptr [esi + 0x74]
3008b5e4 f30f5c4508               subss xmm0, dword ptr [ebp + 8]
3008b5e9 f30f5845fc               addss xmm0, dword ptr [ebp - 4]
3008b5ee f30f114644               movss dword ptr [esi + 0x44], xmm0
3008b5f3 a168813530               mov eax, dword ptr [0x30358168]
3008b5f8 8b8890000000             mov ecx, dword ptr [eax + 0x90]
3008b5fe 2b4e78                   sub ecx, dword ptr [esi + 0x78]
3008b601 83f903                   cmp ecx, 3
3008b604 7632                     jbe 0x3008b638
3008b606 8b4610                   mov eax, dword ptr [esi + 0x10]
3008b609 8b0d20cd2d30             mov ecx, dword ptr [0x302dcd20]
3008b60f 0590000000               add eax, 0x90
3008b614 50                       push eax
3008b615 8b01                     mov eax, dword ptr [ecx]
3008b617 8b4024                   mov eax, dword ptr [eax + 0x24]
3008b61a 8d55e4                   lea edx, [ebp - 0x1c]
3008b61d 52                       push edx
3008b61e 6a00                     push 0
3008b620 8d55ec                   lea edx, [ebp - 0x14]
3008b623 52                       push edx
3008b624 ffd0                     call eax
3008b626 8bc8                     mov ecx, eax
3008b628 e87349fdff               call 0x3005ffa0
3008b62d df28                     fild qword ptr [eax]
3008b62f d80da4692530             fmul dword ptr [0x302569a4]
3008b635 d95e74                   fstp dword ptr [esi + 0x74]
3008b638 f30f104674               movss xmm0, dword ptr [esi + 0x74]
3008b63d 0f2f4644                 comiss xmm0, dword ptr [esi + 0x44]
3008b641 0f868b000000             jbe 0x3008b6d2
3008b647 d94644                   fld dword ptr [esi + 0x44]
3008b64a 8bce                     mov ecx, esi
3008b64c d95df4                   fstp dword ptr [ebp - 0xc]
3008b64f e86c4dfdff               call 0x300603c0
3008b654 d865f4                   fsub dword ptr [ebp - 0xc]
3008b657 d95508                   fst dword ptr [ebp + 8]
3008b65a d945fc                   fld dword ptr [ebp - 4]
3008b65d e8a8ae1a00               call 0x3023650a
3008b662 d955f0                   fst dword ptr [ebp - 0x10]
3008b665 d9ee                     fldz
3008b667 d9c9                     fxch st(1)
3008b669 dff1                     fcompi st(1)
3008b66b ddd8                     fstp st(0)
3008b66d 7625                     jbe 0x3008b694
3008b66f f30f1045fc               movss xmm0, dword ptr [ebp - 4]
3008b674 f30f5c45f0               subss xmm0, dword ptr [ebp - 0x10]
3008b679 f30f584508               addss xmm0, dword ptr [ebp + 8]
3008b67e f30f104df4               movss xmm1, dword ptr [ebp - 0xc]
3008b683 f30f58c8                 addss xmm1, xmm0
3008b687 f30f114e44               movss dword ptr [esi + 0x44], xmm1
3008b68c 5e                       pop esi
3008b68d 5b                       pop ebx
3008b68e 8be5                     mov esp, ebp
3008b690 5d                       pop ebp
3008b691 c20400                   ret 4
3008b694 f30f104508               movss xmm0, dword ptr [ebp + 8]
3008b699 f30f104df4               movss xmm1, dword ptr [ebp - 0xc]
3008b69e f30f58c8                 addss xmm1, xmm0
3008b6a2 f30f114e44               movss dword ptr [esi + 0x44], xmm1
3008b6a7 5e                       pop esi
3008b6a8 5b                       pop ebx
3008b6a9 8be5                     mov esp, ebp
3008b6ab 5d                       pop ebp
3008b6ac c20400                   ret 4
3008b6af 0f57c0                   xorps xmm0, xmm0
3008b6b2 0f2f45fc                 comiss xmm0, dword ptr [ebp - 4]
3008b6b6 7615                     jbe 0x3008b6cd
3008b6b8 8b460c                   mov eax, dword ptr [esi + 0xc]
3008b6bb 85c0                     test eax, eax
3008b6bd 740e                     je 0x3008b6cd
3008b6bf d94044                   fld dword ptr [eax + 0x44]
3008b6c2 d95e44                   fstp dword ptr [esi + 0x44]
3008b6c5 5e                       pop esi
3008b6c6 5b                       pop ebx
3008b6c7 8be5                     mov esp, ebp
3008b6c9 5d                       pop ebp
3008b6ca c20400                   ret 4
3008b6cd f30f114644               movss dword ptr [esi + 0x44], xmm0
3008b6d2 5e                       pop esi
3008b6d3 5b                       pop ebx
3008b6d4 8be5                     mov esp, ebp
3008b6d6 5d                       pop ebp
3008b6d7 c20400                   ret 4
3008b6da cc                       int3
3008b6db cc                       int3
3008b6dc cc                       int3
3008b6dd cc                       int3
3008b6de cc                       int3
3008b6df cc                       int3
