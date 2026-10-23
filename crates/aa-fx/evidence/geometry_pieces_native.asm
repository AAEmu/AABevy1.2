3008ab30 53                       push ebx
3008ab31 8bdc                     mov ebx, esp
3008ab33 83ec08                   sub esp, 8
3008ab36 83e4f0                   and esp, 0xfffffff0
3008ab39 83c404                   add esp, 4
3008ab3c 55                       push ebp
3008ab3d 8b6b04                   mov ebp, dword ptr [ebx + 4]
3008ab40 896c2404                 mov dword ptr [esp + 4], ebp
3008ab44 8bec                     mov ebp, esp
3008ab46 81ec08010000             sub esp, 0x108
3008ab4c 56                       push esi
3008ab4d 8bf1                     mov esi, ecx
3008ab4f 807e3900                 cmp byte ptr [esi + 0x39], 0
3008ab53 57                       push edi
3008ab54 8975f0                   mov dword ptr [ebp - 0x10], esi
3008ab57 746f                     je 0x3008abc8
3008ab59 f30f104310               movss xmm0, dword ptr [ebx + 0x10]
3008ab5e 0f2f05cce62430           comiss xmm0, dword ptr [0x3024e6cc]
3008ab65 7661                     jbe 0x3008abc8
3008ab67 e8443e0000               call 0x3008e9b0
3008ab6c d955f8                   fst dword ptr [ebp - 8]
3008ab6f d9ee                     fldz
3008ab71 d9c9                     fxch st(1)
3008ab73 dff1                     fcompi st(1)
3008ab75 ddd8                     fstp st(0)
3008ab77 760c                     jbe 0x3008ab85
3008ab79 f30f104310               movss xmm0, dword ptr [ebx + 0x10]
3008ab7e f30f5e45f8               divss xmm0, dword ptr [ebp - 8]
3008ab83 eb03                     jmp 0x3008ab88
3008ab85 0f57c0                   xorps xmm0, xmm0
3008ab88 f30f100d70c52430         movss xmm1, dword ptr [0x3024c570]
3008ab90 0f2fc1                   comiss xmm0, xmm1
3008ab93 f30f1145f4               movss dword ptr [ebp - 0xc], xmm0
3008ab98 f30f114df8               movss dword ptr [ebp - 8], xmm1
3008ab9d 8d45f8                   lea eax, [ebp - 8]
3008aba0 7703                     ja 0x3008aba5
3008aba2 8d45f4                   lea eax, [ebp - 0xc]
3008aba5 f30f1000                 movss xmm0, dword ptr [eax]
3008aba9 51                       push ecx
3008abaa 8b4e10                   mov ecx, dword ptr [esi + 0x10]
3008abad f30f110424               movss dword ptr [esp], xmm0
3008abb2 8d4654                   lea eax, [esi + 0x54]
3008abb5 50                       push eax
3008abb6 83c108                   add ecx, 8
3008abb9 51                       push ecx
3008abba 8d55d0                   lea edx, [ebp - 0x30]
3008abbd 52                       push edx
3008abbe e87df9ffff               call 0x3008a540
3008abc3 83c410                   add esp, 0x10
3008abc6 eb48                     jmp 0x3008ac10
3008abc8 8b4610                   mov eax, dword ptr [esi + 0x10]
3008abcb f30f104008               movss xmm0, dword ptr [eax + 8]
3008abd0 83c008                   add eax, 8
3008abd3 f30f1145d0               movss dword ptr [ebp - 0x30], xmm0
3008abd8 f30f104004               movss xmm0, dword ptr [eax + 4]
3008abdd f30f1145d4               movss dword ptr [ebp - 0x2c], xmm0
3008abe2 f30f104008               movss xmm0, dword ptr [eax + 8]
3008abe7 f30f1145d8               movss dword ptr [ebp - 0x28], xmm0
3008abec f30f10400c               movss xmm0, dword ptr [eax + 0xc]
3008abf1 f30f1145dc               movss dword ptr [ebp - 0x24], xmm0
3008abf6 f30f7e4010               movq xmm0, qword ptr [eax + 0x10]
3008abfb 660fd645e0               movq qword ptr [ebp - 0x20], xmm0
3008ac00 8b4818                   mov ecx, dword ptr [eax + 0x18]
3008ac03 894de8                   mov dword ptr [ebp - 0x18], ecx
3008ac06 f30f10401c               movss xmm0, dword ptr [eax + 0x1c]
3008ac0b f30f1145ec               movss dword ptr [ebp - 0x14], xmm0
3008ac10 837b1800                 cmp dword ptr [ebx + 0x18], 0
3008ac14 8b5608                   mov edx, dword ptr [esi + 8]
3008ac17 8b7a20                   mov edi, dword ptr [edx + 0x20]
3008ac1a 897dbc                   mov dword ptr [ebp - 0x44], edi
3008ac1d 0f853e070000             jne 0x3008b361
3008ac23 837b1c00                 cmp dword ptr [ebx + 0x1c], 0
3008ac27 0f8534070000             jne 0x3008b361
3008ac2d 8b8f20030000             mov ecx, dword ptr [edi + 0x320]
3008ac33 85c9                     test ecx, ecx
3008ac35 0f8426070000             je 0x3008b361
3008ac3b 8b01                     mov eax, dword ptr [ecx]
3008ac3d 8b90fc000000             mov edx, dword ptr [eax + 0xfc]
3008ac43 ffd2                     call edx
3008ac45 85c0                     test eax, eax
3008ac47 0f8414070000             je 0x3008b361
3008ac4d c745f8ffffffff           mov dword ptr [ebp - 8], 0xffffffff
3008ac54 33f6                     xor esi, esi
3008ac56 8975fc                   mov dword ptr [ebp - 4], esi
3008ac59 8975f4                   mov dword ptr [ebp - 0xc], esi
3008ac5c 8d642400                 lea esp, [esp]
3008ac60 8b8f20030000             mov ecx, dword ptr [edi + 0x320]
3008ac66 85c9                     test ecx, ecx
3008ac68 740c                     je 0x3008ac76
3008ac6a 8b01                     mov eax, dword ptr [ecx]
3008ac6c 8b90fc000000             mov edx, dword ptr [eax + 0xfc]
3008ac72 ffd2                     call edx
3008ac74 eb02                     jmp 0x3008ac78
3008ac76 33c0                     xor eax, eax
3008ac78 3bf0                     cmp esi, eax
3008ac7a 0f8db8060000             jge 0x3008b338
3008ac80 8b8f20030000             mov ecx, dword ptr [edi + 0x320]
3008ac86 8b01                     mov eax, dword ptr [ecx]
3008ac88 8b9008010000             mov edx, dword ptr [eax + 0x108]
3008ac8e 56                       push esi
3008ac8f ffd2                     call edx
3008ac91 8bf0                     mov esi, eax
3008ac93 85f6                     test esi, esi
3008ac95 0f84dc060000             je 0x3008b377
3008ac9b 833e00                   cmp dword ptr [esi], 0
3008ac9e 0f85d3060000             jne 0x3008b377
3008aca4 837e7000                 cmp dword ptr [esi + 0x70], 0
3008aca8 0f84c9060000             je 0x3008b377
3008acae 807f7400                 cmp byte ptr [edi + 0x74], 0
3008acb2 0f8472060000             je 0x3008b32a
3008acb8 8b45f0                   mov eax, dword ptr [ebp - 0x10]
3008acbb 8b4808                   mov ecx, dword ptr [eax + 8]
3008acbe 6a00                     push 0
3008acc0 6a01                     push 1
3008acc2 e89991fdff               call 0x30063e60
3008acc7 8bf8                     mov edi, eax
3008acc9 85ff                     test edi, edi
3008accb 0f84b1060000             je 0x3008b382
3008acd1 f30f107e6c               movss xmm7, dword ptr [esi + 0x6c]
3008acd6 f30f106644               movss xmm4, dword ptr [esi + 0x44]
3008acdb f30f105658               movss xmm2, dword ptr [esi + 0x58]
3008ace0 f30f106e5c               movss xmm5, dword ptr [esi + 0x5c]
3008ace5 f30f104648               movss xmm0, dword ptr [esi + 0x48]
3008acea f30f104e54               movss xmm1, dword ptr [esi + 0x54]
3008acef f30f105e40               movss xmm3, dword ptr [esi + 0x40]
3008acf4 f30f107664               movss xmm6, dword ptr [esi + 0x64]
3008acf9 f30f117da8               movss dword ptr [ebp - 0x58], xmm7
3008acfe f30f114584               movss dword ptr [ebp - 0x7c], xmm0
3008ad03 f30f10464c               movss xmm0, dword ptr [esi + 0x4c]
3008ad08 f30f116d98               movss dword ptr [ebp - 0x68], xmm5
3008ad0d f30f106e60               movss xmm5, dword ptr [esi + 0x60]
3008ad12 0f28fa                   movaps xmm7, xmm2
3008ad15 f30f59fc                 mulss xmm7, xmm4
3008ad19 f30f59fd                 mulss xmm7, xmm5
3008ad1d f30f116d9c               movss dword ptr [ebp - 0x64], xmm5
3008ad22 f30f1175a0               movss dword ptr [ebp - 0x60], xmm6
3008ad27 f30f107668               movss xmm6, dword ptr [esi + 0x68]
3008ad2c 0f28e9                   movaps xmm5, xmm1
3008ad2f f30f594d84               mulss xmm1, dword ptr [ebp - 0x7c]
3008ad34 f30f594d9c               mulss xmm1, dword ptr [ebp - 0x64]
3008ad39 f30f114588               movss dword ptr [ebp - 0x78], xmm0
3008ad3e f30f104650               movss xmm0, dword ptr [esi + 0x50]
3008ad43 8b7670                   mov esi, dword ptr [esi + 0x70]
3008ad46 f30f1175a4               movss dword ptr [ebp - 0x5c], xmm6
3008ad4b f30f59eb                 mulss xmm5, xmm3
3008ad4f f30f59ee                 mulss xmm5, xmm6
3008ad53 0f28f0                   movaps xmm6, xmm0
3008ad56 f30f597584               mulss xmm6, dword ptr [ebp - 0x7c]
3008ad5b f30f5975a0               mulss xmm6, dword ptr [ebp - 0x60]
3008ad60 f30f59c4                 mulss xmm0, xmm4
3008ad64 f30f5945a4               mulss xmm0, dword ptr [ebp - 0x5c]
3008ad69 f30f58fd                 addss xmm7, xmm5
3008ad6d f30f58fe                 addss xmm7, xmm6
3008ad71 f30f5cf9                 subss xmm7, xmm1
3008ad75 f20f100dd0692530         movsd xmm1, qword ptr [0x302569d0]
3008ad7d f30f59d3                 mulss xmm2, xmm3
3008ad81 f30f5955a0               mulss xmm2, dword ptr [ebp - 0x60]
3008ad86 f30f5cfa                 subss xmm7, xmm2
3008ad8a f30f5cf8                 subss xmm7, xmm0
3008ad8e f30f119d7cffffff         movss dword ptr [ebp - 0x84], xmm3
3008ad96 f30f116580               movss dword ptr [ebp - 0x80], xmm4
3008ad9b 0f5ac7                   cvtps2pd xmm0, xmm7
3008ad9e e897b91a00               call 0x3023673a
3008ada3 f30f107d80               movss xmm7, dword ptr [ebp - 0x80]
3008ada8 f30f10a57cffffff         movss xmm4, dword ptr [ebp - 0x84]
3008adb0 f30f106d84               movss xmm5, dword ptr [ebp - 0x7c]
3008adb5 f30f1075a4               movss xmm6, dword ptr [ebp - 0x5c]
3008adba f20f5ac0                 cvtsd2ss xmm0, xmm0
3008adbe f30f11851cffffff         movss dword ptr [ebp - 0xe4], xmm0
3008adc6 0f28cf                   movaps xmm1, xmm7
3008adc9 f30f59cf                 mulss xmm1, xmm7
3008adcd 0f28c4                   movaps xmm0, xmm4
3008add0 f30f59c4                 mulss xmm0, xmm4
3008add4 f30f58c1                 addss xmm0, xmm1
3008add8 0f28cd                   movaps xmm1, xmm5
3008addb f30f59cd                 mulss xmm1, xmm5
3008addf f30f58c1                 addss xmm0, xmm1
3008ade3 f30f58051c552530         addss xmm0, dword ptr [0x3025551c]
3008adeb 0f57c9                   xorps xmm1, xmm1
3008adee f30f10c8                 movss xmm1, xmm0
3008adf2 f30f52c9                 rsqrtss xmm1, xmm1
3008adf6 0f28d1                   movaps xmm2, xmm1
3008adf9 f30f100d2cd42430         movss xmm1, dword ptr [0x3024d42c]
3008ae01 0f28da                   movaps xmm3, xmm2
3008ae04 f30f59d8                 mulss xmm3, xmm0
3008ae08 f30f59da                 mulss xmm3, xmm2
3008ae0c f30f591d90c52430         mulss xmm3, dword ptr [0x3024c590]
3008ae14 f30f5ccb                 subss xmm1, xmm3
3008ae18 f30f59ca                 mulss xmm1, xmm2
3008ae1c 0f28d1                   movaps xmm2, xmm1
3008ae1f 0f28df                   movaps xmm3, xmm7
3008ae22 f30f59d9                 mulss xmm3, xmm1
3008ae26 0f28c5                   movaps xmm0, xmm5
3008ae29 f30f59c1                 mulss xmm0, xmm1
3008ae2d f30f104da0               movss xmm1, dword ptr [ebp - 0x60]
3008ae32 f30f59cd                 mulss xmm1, xmm5
3008ae36 f30f59f7                 mulss xmm6, xmm7
3008ae3a f30f59d4                 mulss xmm2, xmm4
3008ae3e f30f5cce                 subss xmm1, xmm6
3008ae42 f30f1075a4               movss xmm6, dword ptr [ebp - 0x5c]
3008ae47 f30f59f4                 mulss xmm6, xmm4
3008ae4b f30f11b578ffffff         movss dword ptr [ebp - 0x88], xmm6
3008ae53 f30f10759c               movss xmm6, dword ptr [ebp - 0x64]
3008ae58 f30f59f5                 mulss xmm6, xmm5
3008ae5c f30f10ad78ffffff         movss xmm5, dword ptr [ebp - 0x88]
3008ae64 f30f5cee                 subss xmm5, xmm6
3008ae68 f30f10759c               movss xmm6, dword ptr [ebp - 0x64]
3008ae6d f30f59f7                 mulss xmm6, xmm7
3008ae71 f30f107da0               movss xmm7, dword ptr [ebp - 0x60]
3008ae76 f30f59fc                 mulss xmm7, xmm4
3008ae7a f30f5cf7                 subss xmm6, xmm7
3008ae7e 0f28f9                   movaps xmm7, xmm1
3008ae81 f30f59f9                 mulss xmm7, xmm1
3008ae85 0f28e5                   movaps xmm4, xmm5
3008ae88 f30f59e5                 mulss xmm4, xmm5
3008ae8c f30f58e7                 addss xmm4, xmm7
3008ae90 0f28fe                   movaps xmm7, xmm6
3008ae93 f30f59fe                 mulss xmm7, xmm6
3008ae97 f30f58e7                 addss xmm4, xmm7
3008ae9b f30f58251c552530         addss xmm4, dword ptr [0x3025551c]
3008aea3 0f57ff                   xorps xmm7, xmm7
3008aea6 f30f10fc                 movss xmm7, xmm4
3008aeaa f30f52ff                 rsqrtss xmm7, xmm7
3008aeae f30f117dc0               movss dword ptr [ebp - 0x40], xmm7
3008aeb3 f30f59fc                 mulss xmm7, xmm4
3008aeb7 f30f597dc0               mulss xmm7, dword ptr [ebp - 0x40]
3008aebc f30f593d90c52430         mulss xmm7, dword ptr [0x3024c590]
3008aec4 f30f10252cd42430         movss xmm4, dword ptr [0x3024d42c]
3008aecc f30f5ce7                 subss xmm4, xmm7
3008aed0 f30f5965c0               mulss xmm4, dword ptr [ebp - 0x40]
3008aed5 f30f59cc                 mulss xmm1, xmm4
3008aed9 f30f59f4                 mulss xmm6, xmm4
3008aedd 0f28f9                   movaps xmm7, xmm1
3008aee0 f30f59ec                 mulss xmm5, xmm4
3008aee4 0f28ce                   movaps xmm1, xmm6
3008aee7 f30f59cb                 mulss xmm1, xmm3
3008aeeb f30f1175cc               movss dword ptr [ebp - 0x34], xmm6
3008aef0 0f28e5                   movaps xmm4, xmm5
3008aef3 f30f59e0                 mulss xmm4, xmm0
3008aef7 f30f5ccc                 subss xmm1, xmm4
3008aefb f30f59f2                 mulss xmm6, xmm2
3008aeff f30f118d6cffffff         movss dword ptr [ebp - 0x94], xmm1
3008af07 0f28c8                   movaps xmm1, xmm0
3008af0a f30f59cf                 mulss xmm1, xmm7
3008af0e f30f5cce                 subss xmm1, xmm6
3008af12 0f28e5                   movaps xmm4, xmm5
3008af15 0f28f3                   movaps xmm6, xmm3
3008af18 f30f59f7                 mulss xmm6, xmm7
3008af1c f30f59e2                 mulss xmm4, xmm2
3008af20 f30f5ce6                 subss xmm4, xmm6
3008af24 0f28f5                   movaps xmm6, xmm5
3008af27 f30f117dc4               movss dword ptr [ebp - 0x3c], xmm7
3008af2c 0f57ff                   xorps xmm7, xmm7
3008af2f f30f58f2                 addss xmm6, xmm2
3008af33 f30f58f4                 addss xmm6, xmm4
3008af37 0f2ff7                   comiss xmm6, xmm7
3008af3a 765f                     jbe 0x3008af9b
3008af3c f30f583570c52430         addss xmm6, dword ptr [0x3024c570]
3008af44 f30f5c856cffffff         subss xmm0, dword ptr [ebp - 0x94]
3008af4c f30f5c4dcc               subss xmm1, dword ptr [ebp - 0x34]
3008af51 0f57d2                   xorps xmm2, xmm2
3008af54 f30f10d6                 movss xmm2, xmm6
3008af58 f30f51d2                 sqrtss xmm2, xmm2
3008af5c 0f28ea                   movaps xmm5, xmm2
3008af5f f30f101590c52430         movss xmm2, dword ptr [0x3024c590]
3008af67 f30f5ed5                 divss xmm2, xmm5
3008af6b f30f592d90c52430         mulss xmm5, dword ptr [0x3024c590]
3008af73 f30f59c2                 mulss xmm0, xmm2
3008af77 0f28e0                   movaps xmm4, xmm0
3008af7a f30f1045c4               movss xmm0, dword ptr [ebp - 0x3c]
3008af7f f30f5cc3                 subss xmm0, xmm3
3008af83 f30f59c2                 mulss xmm0, xmm2
3008af87 f30f59ca                 mulss xmm1, xmm2
3008af8b f30f1145b4               movss dword ptr [ebp - 0x4c], xmm0
3008af90 0f28f1                   movaps xmm6, xmm1
3008af93 0f28c5                   movaps xmm0, xmm5
3008af96 e94b010000               jmp 0x3008b0e6
3008af9b 0f2fd5                   comiss xmm2, xmm5
3008af9e 7265                     jb 0x3008b005
3008afa0 0f2fd4                   comiss xmm2, xmm4
3008afa3 7260                     jb 0x3008b005
3008afa5 f30f58856cffffff         addss xmm0, dword ptr [ebp - 0x94]
3008afad f30f585dc4               addss xmm3, dword ptr [ebp - 0x3c]
3008afb2 f30f5c4dcc               subss xmm1, dword ptr [ebp - 0x34]
3008afb7 f30f5cd5                 subss xmm2, xmm5
3008afbb f30f5cd4                 subss xmm2, xmm4
3008afbf f30f581570c52430         addss xmm2, dword ptr [0x3024c570]
3008afc7 0f57e4                   xorps xmm4, xmm4
3008afca f30f10e2                 movss xmm4, xmm2
3008afce f30f101590c52430         movss xmm2, dword ptr [0x3024c590]
3008afd6 f30f51e4                 sqrtss xmm4, xmm4
3008afda 0f28f4                   movaps xmm6, xmm4
3008afdd f30f5ed6                 divss xmm2, xmm6
3008afe1 f30f593590c52430         mulss xmm6, dword ptr [0x3024c590]
3008afe9 f30f59c2                 mulss xmm0, xmm2
3008afed f30f59da                 mulss xmm3, xmm2
3008aff1 f30f1145b4               movss dword ptr [ebp - 0x4c], xmm0
3008aff6 f30f59ca                 mulss xmm1, xmm2
3008affa 0f28e3                   movaps xmm4, xmm3
3008affd 0f28c1                   movaps xmm0, xmm1
3008b000 e9e1000000               jmp 0x3008b0e6
3008b005 0f2fea                   comiss xmm5, xmm2
3008b008 725e                     jb 0x3008b068
3008b00a 0f2fec                   comiss xmm5, xmm4
3008b00d 7259                     jb 0x3008b068
3008b00f f30f585dc4               addss xmm3, dword ptr [ebp - 0x3c]
3008b014 f30f584dcc               addss xmm1, dword ptr [ebp - 0x34]
3008b019 f30f5c856cffffff         subss xmm0, dword ptr [ebp - 0x94]
3008b021 f30f5cec                 subss xmm5, xmm4
3008b025 f30f5cea                 subss xmm5, xmm2
3008b029 f30f582d70c52430         addss xmm5, dword ptr [0x3024c570]
3008b031 0f57d2                   xorps xmm2, xmm2
3008b034 f30f10d5                 movss xmm2, xmm5
3008b038 f30f102d90c52430         movss xmm5, dword ptr [0x3024c590]
3008b040 f30f51d2                 sqrtss xmm2, xmm2
3008b044 0f28e2                   movaps xmm4, xmm2
3008b047 0f28d5                   movaps xmm2, xmm5
3008b04a f30f5ed4                 divss xmm2, xmm4
3008b04e f30f59da                 mulss xmm3, xmm2
3008b052 f30f59ca                 mulss xmm1, xmm2
3008b056 0f28f3                   movaps xmm6, xmm3
3008b059 f30f59e5                 mulss xmm4, xmm5
3008b05d f30f114db4               movss dword ptr [ebp - 0x4c], xmm1
3008b062 f30f59c2                 mulss xmm0, xmm2
3008b066 eb7e                     jmp 0x3008b0e6
3008b068 0f2fe2                   comiss xmm4, xmm2
3008b06b 7266                     jb 0x3008b0d3
3008b06d 0f2fe5                   comiss xmm4, xmm5
3008b070 7261                     jb 0x3008b0d3
3008b072 f30f58856cffffff         addss xmm0, dword ptr [ebp - 0x94]
3008b07a f30f584dcc               addss xmm1, dword ptr [ebp - 0x34]
3008b07f f30f5ce2                 subss xmm4, xmm2
3008b083 f30f5ce5                 subss xmm4, xmm5
3008b087 f30f582570c52430         addss xmm4, dword ptr [0x3024c570]
3008b08f 0f57d2                   xorps xmm2, xmm2
3008b092 f30f10d4                 movss xmm2, xmm4
3008b096 f30f51d2                 sqrtss xmm2, xmm2
3008b09a 0f28ea                   movaps xmm5, xmm2
3008b09d f30f101590c52430         movss xmm2, dword ptr [0x3024c590]
3008b0a5 f30f5ed5                 divss xmm2, xmm5
3008b0a9 f30f592d90c52430         mulss xmm5, dword ptr [0x3024c590]
3008b0b1 f30f59c2                 mulss xmm0, xmm2
3008b0b5 0f28f0                   movaps xmm6, xmm0
3008b0b8 f30f1045c4               movss xmm0, dword ptr [ebp - 0x3c]
3008b0bd f30f59ca                 mulss xmm1, xmm2
3008b0c1 f30f5cc3                 subss xmm0, xmm3
3008b0c5 0f28e1                   movaps xmm4, xmm1
3008b0c8 f30f116db4               movss dword ptr [ebp - 0x4c], xmm5
3008b0cd f30f59c2                 mulss xmm0, xmm2
3008b0d1 eb13                     jmp 0x3008b0e6
3008b0d3 f30f100570c52430         movss xmm0, dword ptr [0x3024c570]
3008b0db 0f28f7                   movaps xmm6, xmm7
3008b0de 0f28e7                   movaps xmm4, xmm7
3008b0e1 f30f117db4               movss dword ptr [ebp - 0x4c], xmm7
3008b0e6 f30f1055d8               movss xmm2, dword ptr [ebp - 0x28]
3008b0eb f30f106ddc               movss xmm5, dword ptr [ebp - 0x24]
3008b0f0 f30f105dd0               movss xmm3, dword ptr [ebp - 0x30]
3008b0f5 f30f104dd4               movss xmm1, dword ptr [ebp - 0x2c]
3008b0fa 0f14da                   unpcklps xmm3, xmm2
3008b0fd 0f14cd                   unpcklps xmm1, xmm5
3008b100 0f14d9                   unpcklps xmm3, xmm1
3008b103 f30f1055a8               movss xmm2, dword ptr [ebp - 0x58]
3008b108 f30f106d88               movss xmm5, dword ptr [ebp - 0x78]
3008b10d f30f104d98               movss xmm1, dword ptr [ebp - 0x68]
3008b112 0f14ea                   unpcklps xmm5, xmm2
3008b115 0f14cf                   unpcklps xmm1, xmm7
3008b118 0f14e9                   unpcklps xmm5, xmm1
3008b11b 0f28cb                   movaps xmm1, xmm3
3008b11e 0fc6cb12                 shufps xmm1, xmm3, 0x12
3008b122 0f298d20ffffff           movaps xmmword ptr [ebp - 0xe0], xmm1
3008b129 0f28fb                   movaps xmm7, xmm3
3008b12c 0fc6fbc9                 shufps xmm7, xmm3, 0xc9
3008b130 0f28d5                   movaps xmm2, xmm5
3008b133 0fc6d5c9                 shufps xmm2, xmm5, 0xc9
3008b137 0f59d1                   mulps xmm2, xmm1
3008b13a 0f571520972a30           xorps xmm2, xmmword ptr [0x302a9720]
3008b141 0fc6dbff                 shufps xmm3, xmm3, 0xff
3008b145 0f59dd                   mulps xmm3, xmm5
3008b148 0f28cd                   movaps xmm1, xmm5
3008b14b 0fc6cd12                 shufps xmm1, xmm5, 0x12
3008b14f 0f59cf                   mulps xmm1, xmm7
3008b152 0f58d1                   addps xmm2, xmm1
3008b155 0f58d3                   addps xmm2, xmm3
3008b158 f30f105dec               movss xmm3, dword ptr [ebp - 0x14]
3008b15d 0f28ca                   movaps xmm1, xmm2
3008b160 0fc6cac9                 shufps xmm1, xmm2, 0xc9
3008b164 0f598d20ffffff           mulps xmm1, xmmword ptr [ebp - 0xe0]
3008b16b 0f570d20972a30           xorps xmm1, xmmword ptr [0x302a9720]
3008b172 0fc6d2d2                 shufps xmm2, xmm2, 0xd2
3008b176 0f59d7                   mulps xmm2, xmm7
3008b179 0f58ca                   addps xmm1, xmm2
3008b17c 0f58c9                   addps xmm1, xmm1
3008b17f 0f58cd                   addps xmm1, xmm5
3008b182 f30f106de0               movss xmm5, dword ptr [ebp - 0x20]
3008b187 0f298d30ffffff           movaps xmmword ptr [ebp - 0xd0], xmm1
3008b18e 0f28cb                   movaps xmm1, xmm3
3008b191 f30f598d30ffffff         mulss xmm1, dword ptr [ebp - 0xd0]
3008b199 f30f58e9                 addss xmm5, xmm1
3008b19d f30f104de4               movss xmm1, dword ptr [ebp - 0x1c]
3008b1a2 0f28d3                   movaps xmm2, xmm3
3008b1a5 f30f599534ffffff         mulss xmm2, dword ptr [ebp - 0xcc]
3008b1ad f30f599d38ffffff         mulss xmm3, dword ptr [ebp - 0xc8]
3008b1b5 f30f58ca                 addss xmm1, xmm2
3008b1b9 f30f118d64ffffff         movss dword ptr [ebp - 0x9c], xmm1
3008b1c1 f30f104de8               movss xmm1, dword ptr [ebp - 0x18]
3008b1c6 f30f58cb                 addss xmm1, xmm3
3008b1ca f30f105db4               movss xmm3, dword ptr [ebp - 0x4c]
3008b1cf f30f118d68ffffff         movss dword ptr [ebp - 0x98], xmm1
3008b1d7 f30f104db4               movss xmm1, dword ptr [ebp - 0x4c]
3008b1dc f30f594dd4               mulss xmm1, dword ptr [ebp - 0x2c]
3008b1e1 0f28d4                   movaps xmm2, xmm4
3008b1e4 f30f5955d8               mulss xmm2, dword ptr [ebp - 0x28]
3008b1e9 f30f5cca                 subss xmm1, xmm2
3008b1ed f30f1055dc               movss xmm2, dword ptr [ebp - 0x24]
3008b1f2 f30f59d6                 mulss xmm2, xmm6
3008b1f6 f30f58ca                 addss xmm1, xmm2
3008b1fa f30f11ad60ffffff         movss dword ptr [ebp - 0xa0], xmm5
3008b202 f30f106dd0               movss xmm5, dword ptr [ebp - 0x30]
3008b207 f30f59dd                 mulss xmm3, xmm5
3008b20b 0f28d0                   movaps xmm2, xmm0
3008b20e f30f59d5                 mulss xmm2, xmm5
3008b212 f30f58ca                 addss xmm1, xmm2
3008b216 0f28d6                   movaps xmm2, xmm6
3008b219 f30f5955d8               mulss xmm2, dword ptr [ebp - 0x28]
3008b21e f30f5cd3                 subss xmm2, xmm3
3008b222 0f28dc                   movaps xmm3, xmm4
3008b225 f30f595ddc               mulss xmm3, dword ptr [ebp - 0x24]
3008b22a f30f58d3                 addss xmm2, xmm3
3008b22e 0f28d8                   movaps xmm3, xmm0
3008b231 f30f595dd4               mulss xmm3, dword ptr [ebp - 0x2c]
3008b236 f30f58d3                 addss xmm2, xmm3
3008b23a 0f28dc                   movaps xmm3, xmm4
3008b23d f30f59dd                 mulss xmm3, xmm5
3008b241 0f28ee                   movaps xmm5, xmm6
3008b244 f30f596dd4               mulss xmm5, dword ptr [ebp - 0x2c]
3008b249 f30f5965d4               mulss xmm4, dword ptr [ebp - 0x2c]
3008b24e f30f5975d0               mulss xmm6, dword ptr [ebp - 0x30]
3008b253 8b8d68ffffff             mov ecx, dword ptr [ebp - 0x98]
3008b259 8b45f0                   mov eax, dword ptr [ebp - 0x10]
3008b25c f30f5cdd                 subss xmm3, xmm5
3008b260 f30f106db4               movss xmm5, dword ptr [ebp - 0x4c]
3008b265 f30f596ddc               mulss xmm5, dword ptr [ebp - 0x24]
3008b26a f30f58dd                 addss xmm3, xmm5
3008b26e 0f28e8                   movaps xmm5, xmm0
3008b271 f30f596dd8               mulss xmm5, dword ptr [ebp - 0x28]
3008b276 f30f5945dc               mulss xmm0, dword ptr [ebp - 0x24]
3008b27b f30f58dd                 addss xmm3, xmm5
3008b27f f30f106db4               movss xmm5, dword ptr [ebp - 0x4c]
3008b284 f30f596dd8               mulss xmm5, dword ptr [ebp - 0x28]
3008b289 f30f58e6                 addss xmm4, xmm6
3008b28d 51                       push ecx
3008b28e f30f58e5                 addss xmm4, xmm5
3008b292 f30f5cc4                 subss xmm0, xmm4
3008b296 f30f11854cffffff         movss dword ptr [ebp - 0xb4], xmm0
3008b29e f30f7e8560ffffff         movq xmm0, qword ptr [ebp - 0xa0]
3008b2a6 660fd68550ffffff         movq qword ptr [ebp - 0xb0], xmm0
3008b2ae f30f10851cffffff         movss xmm0, dword ptr [ebp - 0xe4]
3008b2b6 f30f5945ec               mulss xmm0, dword ptr [ebp - 0x14]
3008b2bb f30f11855cffffff         movss dword ptr [ebp - 0xa4], xmm0
3008b2c3 f30f104314               movss xmm0, dword ptr [ebx + 0x14]
3008b2c8 f30f110424               movss dword ptr [esp], xmm0
3008b2cd f30f104310               movss xmm0, dword ptr [ebx + 0x10]
3008b2d2 6a00                     push 0
3008b2d4 56                       push esi
3008b2d5 8d9540ffffff             lea edx, [ebp - 0xc0]
3008b2db 52                       push edx
3008b2dc 50                       push eax
3008b2dd 51                       push ecx
3008b2de 898d58ffffff             mov dword ptr [ebp - 0xa8], ecx
3008b2e4 8b4b08                   mov ecx, dword ptr [ebx + 8]
3008b2e7 f30f110424               movss dword ptr [esp], xmm0
3008b2ec 51                       push ecx
3008b2ed 8bcf                     mov ecx, edi
3008b2ef f30f118d40ffffff         movss dword ptr [ebp - 0xc0], xmm1
3008b2f7 f30f119544ffffff         movss dword ptr [ebp - 0xbc], xmm2
3008b2ff f30f119d48ffffff         movss dword ptr [ebp - 0xb8], xmm3
3008b307 e85445fdff               call 0x3005f860
3008b30c 84c0                     test al, al
3008b30e 7561                     jne 0x3008b371
3008b310 8b55f0                   mov edx, dword ptr [ebp - 0x10]
3008b313 8b4a08                   mov ecx, dword ptr [edx + 8]
3008b316 57                       push edi
3008b317 e8248dfdff               call 0x30064040
3008b31c ff45f4                   inc dword ptr [ebp - 0xc]
3008b31f 8b7dbc                   mov edi, dword ptr [ebp - 0x44]
3008b322 8b75f4                   mov esi, dword ptr [ebp - 0xc]
3008b325 e936f9ffff               jmp 0x3008ac60
3008b32a 8b45fc                   mov eax, dword ptr [ebp - 4]
3008b32d 3b45f8                   cmp eax, dword ptr [ebp - 8]
3008b330 7542                     jne 0x3008b374
3008b332 8b4e70                   mov ecx, dword ptr [esi + 0x70]
3008b335 894b18                   mov dword ptr [ebx + 0x18], ecx
3008b338 807f7400                 cmp byte ptr [edi + 0x74], 0
3008b33c 7544                     jne 0x3008b382
3008b33e 837dfc00                 cmp dword ptr [ebp - 4], 0
3008b342 0f84ac000000             je 0x3008b3f4
3008b348 ff15fcc22430             call dword ptr [0x3024c2fc]
3008b34e f765fc                   mul dword ptr [ebp - 4]
3008b351 837b1800                 cmp dword ptr [ebx + 0x18], 0
3008b355 8955f8                   mov dword ptr [ebp - 8], edx
3008b358 0f84f6f8ffff             je 0x3008ac54
3008b35e 8b75f0                   mov esi, dword ptr [ebp - 0x10]
3008b361 807b0c00                 cmp byte ptr [ebx + 0xc], 0
3008b365 7529                     jne 0x3008b390
3008b367 807e2f00                 cmp byte ptr [esi + 0x2f], 0
3008b36b 7523                     jne 0x3008b390
3008b36d 32c0                     xor al, al
3008b36f eb24                     jmp 0x3008b395
3008b371 8b7dbc                   mov edi, dword ptr [ebp - 0x44]
3008b374 ff45fc                   inc dword ptr [ebp - 4]
3008b377 ff45f4                   inc dword ptr [ebp - 0xc]
3008b37a 8b75f4                   mov esi, dword ptr [ebp - 0xc]
3008b37d e9def8ffff               jmp 0x3008ac60
3008b382 8b45fc                   mov eax, dword ptr [ebp - 4]
3008b385 5f                       pop edi
3008b386 5e                       pop esi
3008b387 8be5                     mov esp, ebp
3008b389 5d                       pop ebp
3008b38a 8be3                     mov esp, ebx
3008b38c 5b                       pop ebx
3008b38d c21800                   ret 0x18
3008b390 b801000000               mov eax, 1
3008b395 8b4e08                   mov ecx, dword ptr [esi + 8]
3008b398 6a01                     push 1
3008b39a 50                       push eax
3008b39b e8c08afdff               call 0x30063e60
3008b3a0 8bf0                     mov esi, eax
3008b3a2 85f6                     test esi, esi
3008b3a4 744e                     je 0x3008b3f4
3008b3a6 f30f104314               movss xmm0, dword ptr [ebx + 0x14]
3008b3ab 8b531c                   mov edx, dword ptr [ebx + 0x1c]
3008b3ae 8b4318                   mov eax, dword ptr [ebx + 0x18]
3008b3b1 8b7df0                   mov edi, dword ptr [ebp - 0x10]
3008b3b4 51                       push ecx
3008b3b5 f30f110424               movss dword ptr [esp], xmm0
3008b3ba f30f104310               movss xmm0, dword ptr [ebx + 0x10]
3008b3bf 52                       push edx
3008b3c0 8b5308                   mov edx, dword ptr [ebx + 8]
3008b3c3 50                       push eax
3008b3c4 8d4dd0                   lea ecx, [ebp - 0x30]
3008b3c7 51                       push ecx
3008b3c8 57                       push edi
3008b3c9 51                       push ecx
3008b3ca f30f110424               movss dword ptr [esp], xmm0
3008b3cf 52                       push edx
3008b3d0 8bce                     mov ecx, esi
3008b3d2 e88944fdff               call 0x3005f860
3008b3d7 84c0                     test al, al
3008b3d9 7509                     jne 0x3008b3e4
3008b3db 8b4f08                   mov ecx, dword ptr [edi + 8]
3008b3de 56                       push esi
3008b3df e85c8cfdff               call 0x30064040
3008b3e4 b801000000               mov eax, 1
3008b3e9 5f                       pop edi
3008b3ea 5e                       pop esi
3008b3eb 8be5                     mov esp, ebp
3008b3ed 5d                       pop ebp
3008b3ee 8be3                     mov esp, ebx
3008b3f0 5b                       pop ebx
3008b3f1 c21800                   ret 0x18
3008b3f4 5f                       pop edi
3008b3f5 33c0                     xor eax, eax
3008b3f7 5e                       pop esi
3008b3f8 8be5                     mov esp, ebp
3008b3fa 5d                       pop ebp
3008b3fb 8be3                     mov esp, ebx
3008b3fd 5b                       pop ebx
300619b0 55                       push ebp
300619b1 8bec                     mov ebp, esp
300619b3 83ec08                   sub esp, 8
300619b6 56                       push esi
300619b7 8bf1                     mov esi, ecx
300619b9 837e1000                 cmp dword ptr [esi + 0x10], 0
300619bd 7510                     jne 0x300619cf
300619bf 8b8ea4000000             mov ecx, dword ptr [esi + 0xa4]
300619c5 e8f6860200               call 0x3008a0c0
300619ca d95e34                   fstp dword ptr [esi + 0x34]
300619cd eb0d                     jmp 0x300619dc
300619cf f30f100570c52430         movss xmm0, dword ptr [0x3024c570]
300619d7 f30f114634               movss dword ptr [esi + 0x34], xmm0
300619dc 8b0d4ccd2d30             mov ecx, dword ptr [0x302dcd4c]
300619e2 e899d80200               call 0x3008f280
300619e7 d84e34                   fmul dword ptr [esi + 0x34]
300619ea 8b4620                   mov eax, dword ptr [esi + 0x20]
300619ed 0f57c9                   xorps xmm1, xmm1
300619f0 d955fc                   fst dword ptr [ebp - 4]
300619f3 d95e34                   fstp dword ptr [esi + 0x34]
300619f6 f30f104018               movss xmm0, dword ptr [eax + 0x18]
300619fb f30f5945fc               mulss xmm0, dword ptr [ebp - 4]
30061a00 f30f2cc8                 cvttss2si ecx, xmm0
30061a04 f30f2ac9                 cvtsi2ss xmm1, ecx
30061a08 0f2fc1                   comiss xmm0, xmm1
30061a0b 7601                     jbe 0x30061a0e
30061a0d 41                       inc ecx
30061a0e 894e30                   mov dword ptr [esi + 0x30], ecx
30061a11 80787400                 cmp byte ptr [eax + 0x74], 0
30061a15 0f847c000000             je 0x30061a97
30061a1b 83b82003000000           cmp dword ptr [eax + 0x320], 0
30061a22 7473                     je 0x30061a97
30061a24 8b8020030000             mov eax, dword ptr [eax + 0x320]
30061a2a 53                       push ebx
30061a2b 33db                     xor ebx, ebx
30061a2d 57                       push edi
30061a2e 895dfc                   mov dword ptr [ebp - 4], ebx
30061a31 85c0                     test eax, eax
30061a33 740e                     je 0x30061a43
30061a35 8b10                     mov edx, dword ptr [eax]
30061a37 8bc8                     mov ecx, eax
30061a39 8b82fc000000             mov eax, dword ptr [edx + 0xfc]
30061a3f ffd0                     call eax
30061a41 eb02                     jmp 0x30061a45
30061a43 33c0                     xor eax, eax
30061a45 8d78ff                   lea edi, [eax - 1]
30061a48 85ff                     test edi, edi
30061a4a 782e                     js 0x30061a7a
30061a4c 8d642400                 lea esp, [esp]
30061a50 8b4e20                   mov ecx, dword ptr [esi + 0x20]
30061a53 8b8920030000             mov ecx, dword ptr [ecx + 0x320]
30061a59 8b11                     mov edx, dword ptr [ecx]
30061a5b 8b8208010000             mov eax, dword ptr [edx + 0x108]
30061a61 57                       push edi
30061a62 ffd0                     call eax
30061a64 85c0                     test eax, eax
30061a66 740c                     je 0x30061a74
30061a68 833800                   cmp dword ptr [eax], 0
30061a6b 7507                     jne 0x30061a74
30061a6d 83787000                 cmp dword ptr [eax + 0x70], 0
30061a71 7401                     je 0x30061a74
30061a73 43                       inc ebx
30061a74 4f                       dec edi
30061a75 79d9                     jns 0x30061a50
30061a77 895dfc                   mov dword ptr [ebp - 4], ebx
30061a7a 5f                       pop edi
30061a7b 83fb01                   cmp ebx, 1
30061a7e c745f801000000           mov dword ptr [ebp - 8], 1
30061a85 5b                       pop ebx
30061a86 8d45f8                   lea eax, [ebp - 8]
30061a89 7c03                     jl 0x30061a8e
30061a8b 8d45fc                   lea eax, [ebp - 4]
30061a8e 8b4e30                   mov ecx, dword ptr [esi + 0x30]
30061a91 0faf08                   imul ecx, dword ptr [eax]
30061a94 894e30                   mov dword ptr [esi + 0x30], ecx
30061a97 8b4610                   mov eax, dword ptr [esi + 0x10]
30061a9a 85c0                     test eax, eax
30061a9c 740a                     je 0x30061aa8
30061a9e 8b5030                   mov edx, dword ptr [eax + 0x30]
30061aa1 0faf5630                 imul edx, dword ptr [esi + 0x30]
30061aa5 895630                   mov dword ptr [esi + 0x30], edx
30061aa8 8b4620                   mov eax, dword ptr [esi + 0x20]
30061aab 8b8020030000             mov eax, dword ptr [eax + 0x320]
30061ab1 8b8ea4000000             mov ecx, dword ptr [esi + 0xa4]
30061ab7 f7d8                     neg eax
30061ab9 1bc0                     sbb eax, eax
30061abb 250010ffff               and eax, 0xffff1000
30061ac0 0500000100               add eax, 0x10000
30061ac5 83792800                 cmp dword ptr [ecx + 0x28], 0
30061ac9 8945f8                   mov dword ptr [ebp - 8], eax
30061acc 7423                     je 0x30061af1
30061ace 83795800                 cmp dword ptr [ecx + 0x58], 0
30061ad2 750c                     jne 0x30061ae0
30061ad4 83795c00                 cmp dword ptr [ecx + 0x5c], 0
30061ad8 7506                     jne 0x30061ae0
30061ada 83796000                 cmp dword ptr [ecx + 0x60], 0
30061ade 7411                     je 0x30061af1
30061ae0 3d00400000               cmp eax, 0x4000
30061ae5 b8307a2530               mov eax, 0x30257a30
30061aea 7f03                     jg 0x30061aef
30061aec 8d45f8                   lea eax, [ebp - 8]
30061aef 8b00                     mov eax, dword ptr [eax]
30061af1 8b4e30                   mov ecx, dword ptr [esi + 0x30]
30061af4 3bc8                     cmp ecx, eax
30061af6 7e1f                     jle 0x30061b17
30061af8 0f57c0                   xorps xmm0, xmm0
30061afb f30f2ac0                 cvtsi2ss xmm0, eax
30061aff 0f57c9                   xorps xmm1, xmm1
30061b02 f30f2ac9                 cvtsi2ss xmm1, ecx
30061b06 f30f5ec1                 divss xmm0, xmm1
30061b0a f30f594634               mulss xmm0, dword ptr [esi + 0x34]
30061b0f f30f114634               movss dword ptr [esi + 0x34], xmm0
30061b14 894630                   mov dword ptr [esi + 0x30], eax
30061b17 5e                       pop esi
30061b18 8be5                     mov esp, ebp
30061b1a 5d                       pop ebp
30061b1b c3                       ret
3005bfe0 53                       push ebx
3005bfe1 8bdc                     mov ebx, esp
3005bfe3 83ec08                   sub esp, 8
3005bfe6 83e4f0                   and esp, 0xfffffff0
3005bfe9 83c404                   add esp, 4
3005bfec 55                       push ebp
3005bfed 8b6b04                   mov ebp, dword ptr [ebx + 4]
3005bff0 896c2404                 mov dword ptr [esp + 4], ebp
3005bff4 8bec                     mov ebp, esp
3005bff6 83ec38                   sub esp, 0x38
3005bff9 56                       push esi
3005bffa 8b7308                   mov esi, dword ptr [ebx + 8]
3005bffd 57                       push edi
3005bffe 8bf9                     mov edi, ecx
3005c000 8b87a8000000             mov eax, dword ptr [edi + 0xa8]
3005c006 8b4808                   mov ecx, dword ptr [eax + 8]
3005c009 8b5120                   mov edx, dword ptr [ecx + 0x20]
3005c00c 8bcf                     mov ecx, edi
3005c00e 8955fc                   mov dword ptr [ebp - 4], edx
3005c011 e81a540000               call 0x30061430
3005c016 d95e1c                   fstp dword ptr [esi + 0x1c]
3005c019 f30f7e07                 movq xmm0, qword ptr [edi]
3005c01d 660fd64610               movq qword ptr [esi + 0x10], xmm0
3005c022 8b4708                   mov eax, dword ptr [edi + 8]
3005c025 894618                   mov dword ptr [esi + 0x18], eax
3005c028 d94750                   fld dword ptr [edi + 0x50]
3005c02b d91e                     fstp dword ptr [esi]
3005c02d d94754                   fld dword ptr [edi + 0x54]
3005c030 d95e04                   fstp dword ptr [esi + 4]
3005c033 d94758                   fld dword ptr [edi + 0x58]
3005c036 d95e08                   fstp dword ptr [esi + 8]
3005c039 d9475c                   fld dword ptr [edi + 0x5c]
3005c03c d95e0c                   fstp dword ptr [esi + 0xc]
3005c03f 83bfac00000000           cmp dword ptr [edi + 0xac], 0
3005c046 0f8456010000             je 0x3005c1a2
3005c04c 8b45fc                   mov eax, dword ptr [ebp - 4]
3005c04f 80b8d802000000           cmp byte ptr [eax + 0x2d8], 0
3005c056 0f8546010000             jne 0x3005c1a2
3005c05c 80b8a402000003           cmp byte ptr [eax + 0x2a4], 3
3005c063 0f8439010000             je 0x3005c1a2
3005c069 8b8fac000000             mov ecx, dword ptr [edi + 0xac]
3005c06f 8b11                     mov edx, dword ptr [ecx]
3005c071 8b929c000000             mov edx, dword ptr [edx + 0x9c]
3005c077 8d45d0                   lea eax, [ebp - 0x30]
3005c07a 50                       push eax
3005c07b ffd2                     call edx
3005c07d f30f10400c               movss xmm0, dword ptr [eax + 0xc]
3005c082 f30f5800                 addss xmm0, dword ptr [eax]
3005c086 f30f104810               movss xmm1, dword ptr [eax + 0x10]
3005c08b f30f584804               addss xmm1, dword ptr [eax + 4]
3005c090 f30f105014               movss xmm2, dword ptr [eax + 0x14]
3005c095 f30f585008               addss xmm2, dword ptr [eax + 8]
3005c09a f30f101d90c52430         movss xmm3, dword ptr [0x3024c590]
3005c0a2 f30f10760c               movss xmm6, dword ptr [esi + 0xc]
3005c0a7 f30f106e08               movss xmm5, dword ptr [esi + 8]
3005c0ac f30f59c3                 mulss xmm0, xmm3
3005c0b0 f30f59cb                 mulss xmm1, xmm3
3005c0b4 f30f59d3                 mulss xmm2, xmm3
3005c0b8 f30f101d60c52430         movss xmm3, dword ptr [0x3024c560]
3005c0c0 0f57c3                   xorps xmm0, xmm3
3005c0c3 0f57cb                   xorps xmm1, xmm3
3005c0c6 0f28e1                   movaps xmm4, xmm1
3005c0c9 f30f100e                 movss xmm1, dword ptr [esi]
3005c0cd 0f57da                   xorps xmm3, xmm2
3005c0d0 f30f105604               movss xmm2, dword ptr [esi + 4]
3005c0d5 0f14cd                   unpcklps xmm1, xmm5
3005c0d8 0f14d6                   unpcklps xmm2, xmm6
3005c0db 0f14ca                   unpcklps xmm1, xmm2
3005c0de 0f28d0                   movaps xmm2, xmm0
3005c0e1 0f57c0                   xorps xmm0, xmm0
3005c0e4 0f14d3                   unpcklps xmm2, xmm3
3005c0e7 0f14e0                   unpcklps xmm4, xmm0
3005c0ea 0f14d4                   unpcklps xmm2, xmm4
3005c0ed 0f28e1                   movaps xmm4, xmm1
3005c0f0 0f28d9                   movaps xmm3, xmm1
3005c0f3 0fc6d9c9                 shufps xmm3, xmm1, 0xc9
3005c0f7 0fc6e112                 shufps xmm4, xmm1, 0x12
3005c0fb 0fc6c9ff                 shufps xmm1, xmm1, 0xff
3005c0ff 0f59ca                   mulps xmm1, xmm2
3005c102 0f28c2                   movaps xmm0, xmm2
3005c105 0fc6c2c9                 shufps xmm0, xmm2, 0xc9
3005c109 0f59c4                   mulps xmm0, xmm4
3005c10c 0f570550132a30           xorps xmm0, xmmword ptr [0x302a1350]
3005c113 0f28f2                   movaps xmm6, xmm2
3005c116 0fc6f212                 shufps xmm6, xmm2, 0x12
3005c11a 0f59f3                   mulps xmm6, xmm3
3005c11d 0f58c6                   addps xmm0, xmm6
3005c120 0f58c1                   addps xmm0, xmm1
3005c123 0f28c8                   movaps xmm1, xmm0
3005c126 0fc6c8c9                 shufps xmm1, xmm0, 0xc9
3005c12a 0fc6c0d2                 shufps xmm0, xmm0, 0xd2
3005c12e 0f59c3                   mulps xmm0, xmm3
3005c131 0f59cc                   mulps xmm1, xmm4
3005c134 0f570d50132a30           xorps xmm1, xmmword ptr [0x302a1350]
3005c13b 0f58c8                   addps xmm1, xmm0
3005c13e f30f10461c               movss xmm0, dword ptr [esi + 0x1c]
3005c143 0f58c9                   addps xmm1, xmm1
3005c146 0f58ca                   addps xmm1, xmm2
3005c149 0f294dd0                 movaps xmmword ptr [ebp - 0x30], xmm1
3005c14d f30f104dd0               movss xmm1, dword ptr [ebp - 0x30]
3005c152 f30f1055d4               movss xmm2, dword ptr [ebp - 0x2c]
3005c157 f30f105dd8               movss xmm3, dword ptr [ebp - 0x28]
3005c15c f30f59c8                 mulss xmm1, xmm0
3005c160 f30f59d0                 mulss xmm2, xmm0
3005c164 f30f59d8                 mulss xmm3, xmm0
3005c168 f30f104610               movss xmm0, dword ptr [esi + 0x10]
3005c16d f30f58c1                 addss xmm0, xmm1
3005c171 f30f1145dc               movss dword ptr [ebp - 0x24], xmm0
3005c176 f30f104614               movss xmm0, dword ptr [esi + 0x14]
3005c17b f30f58c2                 addss xmm0, xmm2
3005c17f f30f1145e0               movss dword ptr [ebp - 0x20], xmm0
3005c184 f30f104618               movss xmm0, dword ptr [esi + 0x18]
3005c189 f30f58c3                 addss xmm0, xmm3
3005c18d f30f1145e4               movss dword ptr [ebp - 0x1c], xmm0
3005c192 f30f7e45dc               movq xmm0, qword ptr [ebp - 0x24]
3005c197 8b45e4                   mov eax, dword ptr [ebp - 0x1c]
3005c19a 660fd64610               movq qword ptr [esi + 0x10], xmm0
3005c19f 894618                   mov dword ptr [esi + 0x18], eax
3005c1a2 5f                       pop edi
3005c1a3 5e                       pop esi
3005c1a4 8be5                     mov esp, ebp
3005c1a6 5d                       pop ebp
3005c1a7 8be3                     mov esp, ebx
3005c1a9 5b                       pop ebx
3005c1aa c20400                   ret 4
3005c1ad cc                       int3
3005c1ae cc                       int3
3005c1af cc                       int3
