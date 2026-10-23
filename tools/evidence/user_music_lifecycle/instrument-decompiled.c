// Read-only Ghidra Dec.java output; x2game SHA256
// 851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e.
// ItemWeapon/holdable loaders are retained to disambiguate them from general
// ItemDesc39695dc0. The actual general-items column9/copy/wrapper sequence is
// executed by user_music_catalog_oracle.py and retained in native-source.txt.
// Reproduce -readOnly -noanalysis -postScript Dec.java <output>
// 396bdbe0 396c7700 3971dee0 39695dc0 39703b50 using existing proj/x2game.
// ===== FUN_396bdbe0 @ 396bdbe0 (asked 396bdbe0) size=1338

void FUN_396bdbe0(undefined4 param_1,int param_2)

{
  uint uVar1;
  int iVar2;
  int *piVar3;
  char cVar4;
  uint uVar5;
  int iVar6;
  undefined4 uVar7;
  char *pcVar8;
  int *piVar9;
  uint uVar10;
  int **ppiVar11;
  uint *puVar12;
  undefined4 ****ppppuVar13;
  int iVar14;
  int *piVar15;
  float10 fVar16;
  undefined4 local_a8;
  int *local_a4;
  int *local_a0;
  int *local_9c;
  undefined4 local_98;
  undefined4 local_94;
  undefined1 local_90;
  undefined1 local_8f;
  undefined2 local_8e;
  undefined4 local_8c;
  float local_88;
  undefined4 local_84;
  undefined4 local_80;
  undefined4 local_7c;
  undefined4 local_78;
  undefined4 local_74;
  undefined1 local_70;
  undefined4 local_6c;
  undefined1 local_68;
  undefined4 local_64;
  undefined1 local_60;
  float local_5c;
  int **local_58;
  int **local_54;
  int **local_50;
  int local_48;
  int local_44;
  int local_40;
  int local_3c;
  int *local_38;
  int local_34;
  undefined4 ***local_30 [4];
  uint local_20;
  uint local_1c;
  uint local_14;
  void *local_10;
  undefined1 *puStack_c;
  uint local_8;

  local_8 = 0xffffffff;
  puStack_c = &LAB_3997b16b;
  local_10 = ExceptionList;
  uVar5 = DAT_39acb9c0 ^ (uint)&stack0xfffffffc;
  ExceptionList = &local_10;
  local_48 = param_2 + 0x399c;
  local_3c = param_2;
  local_14 = uVar5;
  cVar4 = FUN_396a7240(param_1,"item_weapons");
  if (cVar4 != '\0') {
    local_34 = 0;
    local_8 = 0;
    iVar6 = FUN_3955b3d0(param_1,
                         "SELECT id, item_id, asset_id, base_enchantable, base_equipment, charge_count, charge_lifetime, drawn_scale, durability_multiplier, eiset_id, fixed_visual_effect_id, holdable_id, mod_set_id, or_unit_reqs, recharge_buff_id, repairable, skin_kind_id, useAsStat, worn_scale FROM item_weapons"
                         ,0xffffffff,&local_34,0,uVar5);
    if (iVar6 == 0) {
      local_40 = 0;
      iVar6 = FUN_3955fb10(local_34);
      if (iVar6 == 100) {
        local_44 = 0;
        do {
          local_a8 = FUN_3953a390(local_34,0);
          uVar7 = FUN_3953a390(local_34,1);
          local_58 = (int **)0x0;
          local_54 = (int **)0x0;
          local_50 = (int **)0x0;
          local_8._0_1_ = 1;
          local_94 = FUN_3953a390(local_34,2);
          pcVar8 = (char *)FUN_3953a480(local_34,3);
          cVar4 = *pcVar8;
          if (((cVar4 == 'T') || (cVar4 == 't')) || (local_90 = 0, cVar4 == '1')) {
            local_90 = 1;
          }
          pcVar8 = (char *)FUN_3953a480(local_34,4);
          cVar4 = *pcVar8;
          if (((cVar4 == 'T') || (cVar4 == 't')) || (local_8f = 0, cVar4 == '1')) {
            local_8f = 1;
          }
          local_8e = FUN_3953a390(local_34,5);
          local_8c = FUN_3953a390(local_34,6);
          fVar16 = (float10)FUN_3953a310(local_34,7);
          local_88 = (float)fVar16;
          local_84 = FUN_3953a390(local_34,8);
          uVar5 = FUN_3953a390(local_34,9);
          iVar6 = local_3c;
          for (puVar12 = *(uint **)(local_3c + 0x352c);
              (puVar12 != *(uint **)(local_3c + 0x3530) && (*puVar12 <= uVar5));
              puVar12 = puVar12 + 3) {
            if (uVar5 <= puVar12[1]) {
              local_80 = *(undefined4 *)(puVar12[2] + (uVar5 - *puVar12) * 4);
              goto LAB_396bddbe;
            }
          }
          local_80 = 0;
LAB_396bddbe:
          local_7c = FUN_3953a390(local_34,10);
          local_78 = FUN_3953a390(local_34,0xb);
          local_74 = FUN_3953a390(local_34,0xc);
          pcVar8 = (char *)FUN_3953a480(local_34,0xd);
          cVar4 = *pcVar8;
          if (((cVar4 == 'T') || (cVar4 == 't')) || (local_70 = 0, cVar4 == '1')) {
            local_70 = 1;
          }
          local_6c = FUN_3953a390(local_34,0xe);
          pcVar8 = (char *)FUN_3953a480(local_34,0xf);
          cVar4 = *pcVar8;
          if (((cVar4 == 'T') || (cVar4 == 't')) || (local_68 = 0, cVar4 == '1')) {
            local_68 = 1;
          }
          local_64 = FUN_3953a390(local_34,0x10);
          pcVar8 = (char *)FUN_3953a480(local_34,0x11);
          cVar4 = *pcVar8;
          if (((cVar4 == 'T') || (cVar4 == 't')) || (local_60 = 0, cVar4 == '1')) {
            local_60 = 1;
          }
          fVar16 = (float10)FUN_3953a310(local_34,0x12);
          local_5c = (float)fVar16;
          local_1c = 0xf;
          local_20 = 0;
          local_30[0] = (undefined4 ***)((uint)local_30[0] & 0xffffff00);
          FUN_3901aa10("ItemWeapon",10);
          local_8 = CONCAT31(local_8._1_3_,2);
          local_38 = (int *)FUN_3962b590(local_30);
          uVar5 = local_20;
          piVar15 = *(int **)(iVar6 + 0x1204);
          if (local_38 == piVar15) {
LAB_396bdf20:
            local_9c = piVar15;
            ppiVar11 = &local_9c;
          }
          else {
            uVar1 = local_38[7];
            piVar9 = local_38 + 3;
            if (0xf < (uint)local_38[8]) {
              piVar9 = (int *)*piVar9;
            }
            uVar10 = local_20;
            if (uVar1 <= local_20) {
              uVar10 = uVar1;
            }
            ppppuVar13 = (undefined4 ****)local_30[0];
            if (local_1c < 0x10) {
              ppppuVar13 = local_30;
            }
            uVar10 = FUN_3901be80(ppppuVar13,piVar9,uVar10);
            if (uVar10 == 0) {
              if (uVar5 < uVar1) {
                uVar10 = 0xffffffff;
              }
              else {
                uVar10 = (uint)(uVar5 != uVar1);
              }
            }
            if ((int)uVar10 < 0) goto LAB_396bdf20;
            ppiVar11 = &local_38;
          }
          local_8 = CONCAT31(local_8._1_3_,1);
          if (0xf < local_1c) {
                    /* WARNING: Subroutine does not return */
            FUN_3950a1f0(local_30[0]);
          }
          local_1c = 0xf;
          local_20 = 0;
          local_30[0] = (undefined4 ***)((uint)local_30[0] & 0xffffff00);
          iVar6 = local_3c;
          if ((*ppiVar11 != *(int **)(local_3c + 0x1204)) &&
             (FUN_3962a810(&local_a4,&local_a8), iVar6 = local_3c, piVar15 = local_a4,
             local_a4 != local_a0)) {
            do {
              piVar9 = piVar15 + 4;
              local_38 = piVar9;
              if ((&local_38 < local_54) && (local_58 <= &local_38)) {
                iVar14 = (int)&local_38 - (int)local_58;
                if (local_54 == local_50) {
                  FUN_3963b700(1);
                }
                if (local_54 != (int **)0x0) {
                  *local_54 = local_58[iVar14 >> 2];
                }
              }
              else {
                if (local_54 == local_50) {
                  FUN_3963b700(1);
                }
                if (local_54 != (int **)0x0) {
                  *local_54 = piVar9;
                }
              }
              *(int *)(iVar6 + 0x10acc) = *(int *)(iVar6 + 0x10acc) + 1;
              local_54 = local_54 + 1;
              if (*(char *)((int)piVar15 + 0x1d) == '\0') {
                piVar9 = (int *)piVar15[2];
                if (*(char *)((int)piVar9 + 0x1d) == '\0') {
                  cVar4 = *(char *)(*piVar9 + 0x1d);
                  piVar15 = piVar9;
                  piVar9 = (int *)*piVar9;
                  while (cVar4 == '\0') {
                    cVar4 = *(char *)(*piVar9 + 0x1d);
                    piVar15 = piVar9;
                    piVar9 = (int *)*piVar9;
                  }
                }
                else {
                  cVar4 = *(char *)(piVar15[1] + 0x1d);
                  piVar3 = (int *)piVar15[1];
                  piVar9 = piVar15;
                  while ((piVar15 = piVar3, cVar4 == '\0' && (piVar9 == (int *)piVar15[2]))) {
                    cVar4 = *(char *)(piVar15[1] + 0x1d);
                    piVar3 = (int *)piVar15[1];
                    piVar9 = piVar15;
                  }
                }
              }
            } while (piVar15 != local_a0);
          }
          iVar2 = local_40;
          iVar14 = local_44;
          *(undefined4 *)(*(int *)(local_48 + 0x5c) + local_40 * 4) = uVar7;
          local_98 = uVar7;
          FUN_396a7590(&local_98);
          *(int *)(iVar6 + 0x10ac8) = *(int *)(iVar6 + 0x10ac8) + 1;
          local_40 = iVar2 + 1;
          local_44 = iVar14 + 0x50;
          local_8 = local_8 & 0xffffff00;
          if (local_58 != (int **)0x0) {
            std::_Container_base0::_Orphan_all((_Container_base0 *)&local_58);
                    /* WARNING: Subroutine does not return */
            FUN_3950a1f0(local_58);
          }
          iVar6 = FUN_3955fb10(local_34);
        } while (iVar6 == 100);
      }
      if (iVar6 == 0x65) {
        FUN_396b6680(local_40);
        if (local_34 != 0) {
          FUN_395519b0(local_34);
        }
        goto LAB_396be104;
      }
      uVar7 = FUN_395338f0(param_1);
      FUN_3967bbc0("failed: sqlite3_step: function=LoadItemWeaponDescs errcode=%d errmsg=%s",iVar6,
                   uVar7);
    }
    else {
      uVar7 = FUN_395338f0(param_1);
      X2Log(4,"failed: sqlite3_prepare_v2: sql=\'%s\' errcode=%d errmsg=%s",
            "SELECT id, item_id, asset_id, base_enchantable, base_equipment, charge_count, charge_lifetime, drawn_scale, durability_multiplier, eiset_id, fixed_visual_effect_id, holdable_id, mod_set_id, or_unit_reqs, recharge_buff_id, repairable, skin_kind_id, useAsStat, worn_scale FROM item_weapons"
            ,iVar6,uVar7);
    }
    if (local_34 != 0) {
      FUN_395519b0(local_34);
    }
  }
LAB_396be104:
  ExceptionList = local_10;
  __security_check_cookie(local_14 ^ (uint)&stack0xfffffffc);
  return;
}


// ===== FUN_396c7700 @ 396c7700 (asked 396c7700) size=1051

undefined4 FUN_396c7700(undefined4 param_1,int param_2)

{
  int iVar1;
  char cVar2;
  uint uVar3;
  int iVar4;
  undefined4 uVar5;
  undefined4 uVar6;
  int iVar7;
  undefined4 *puVar8;
  undefined4 *puVar9;
  float10 fVar10;
  undefined4 local_a0 [4];
  undefined4 local_90;
  undefined4 local_8c;
  undefined4 local_88;
  undefined4 local_84;
  undefined4 local_80;
  undefined4 local_7c;
  undefined4 local_78;
  undefined4 local_74;
  undefined4 local_70;
  float local_6c;
  undefined4 local_68;
  undefined4 local_64;
  undefined4 local_60;
  undefined4 local_5c;
  undefined4 local_58;
  undefined4 local_54;
  undefined4 local_50;
  undefined4 local_4c;
  undefined4 local_48;
  undefined4 local_44;
  undefined4 local_40;
  undefined4 local_3c;
  undefined4 local_38;
  undefined4 local_34;
  undefined4 local_30;
  undefined4 local_2c;
  undefined4 local_28;
  undefined4 local_24;
  undefined4 local_20;
  int local_1c;
  int local_18;
  int local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;

  iVar1 = param_2;
  local_8 = 0xffffffff;
  puStack_c = &LAB_3997b4d8;
  local_10 = ExceptionList;
  uVar3 = DAT_39acb9c0 ^ (uint)&stack0xfffffffc;
  ExceptionList = &local_10;
  iVar7 = param_2 + 0x2ca4;
  local_1c = iVar7;
  cVar2 = FUN_396b3a90(param_1,"holdables");
  if (cVar2 != '\0') {
    param_2 = 0;
    local_8 = 0;
    iVar4 = FUN_3955b3d0(param_1,
                         "SELECT id, angle, anim_l1_ratio, anim_l1_id, anim_l2_ratio, anim_l2_id, anim_l3_id, anim_r1_ratio, anim_r1_id, anim_r2_ratio, anim_r2_id, anim_r3_id, damage_scale, durability_ratio, enchanted_dps1000, extra_damage_blunt_factor, extra_damage_pierce_factor, extra_damage_slash_factor, formula_armor, formula_dps, formula_hdps, formula_mdps, item_proc_id, max_range, min_range, name, pose_id, renew_category, sheathe_priority, slot_type_id, sound_material_id, speed, stat_multiplier FROM holdables"
                         ,0xffffffff,&param_2,0,uVar3);
    if (iVar4 == 0) {
      local_14 = 0;
      iVar4 = FUN_3955fb10(param_2);
      if (iVar4 == 100) {
        local_18 = 0;
        do {
          uVar5 = FUN_3953a390(param_2,0);
          local_a0[1] = FUN_3953a390(param_2,1);
          local_a0[2] = FUN_3953a390(param_2,2);
          local_a0[3] = FUN_3953a390(param_2,3);
          local_90 = FUN_3953a390(param_2,4);
          local_8c = FUN_3953a390(param_2,5);
          local_88 = FUN_3953a390(param_2,6);
          local_84 = FUN_3953a390(param_2,7);
          local_80 = FUN_3953a390(param_2,8);
          local_7c = FUN_3953a390(param_2,9);
          local_78 = FUN_3953a390(param_2,10);
          local_74 = FUN_3953a390(param_2,0xb);
          local_70 = FUN_3953a390(param_2,0xc);
          fVar10 = (float10)FUN_3953a310(param_2,0xd);
          local_6c = (float)fVar10;
          local_68 = FUN_3953a390(param_2,0xe);
          local_64 = FUN_3953a390(param_2,0xf);
          local_60 = FUN_3953a390(param_2,0x10);
          local_5c = FUN_3953a390(param_2,0x11);
          uVar6 = FUN_3953a480(param_2,0x12);
          local_58 = FUN_398b17b0(uVar6);
          uVar6 = FUN_3953a480(param_2,0x13);
          local_54 = FUN_398b17b0(uVar6);
          uVar6 = FUN_3953a480(param_2,0x14);
          local_50 = FUN_398b17b0(uVar6);
          uVar6 = FUN_3953a480(param_2,0x15);
          local_4c = FUN_398b17b0(uVar6);
          local_48 = FUN_3953a390(param_2,0x16);
          local_44 = FUN_3953a390(param_2,0x17);
          local_40 = FUN_3953a390(param_2,0x18);
          uVar6 = FUN_3953a480(param_2,0x19);
          local_3c = FUN_395b1350("holdables",uVar5,&DAT_3999da20,uVar6);
          local_38 = FUN_3953a390(param_2,0x1a);
          local_34 = FUN_3953a390(param_2,0x1b);
          local_30 = FUN_3953a390(param_2,0x1c);
          local_2c = FUN_3953a390(param_2,0x1d);
          local_28 = FUN_3953a390(param_2,0x1e);
          local_24 = FUN_3953a390(param_2,0x1f);
          local_20 = FUN_3953a390(param_2,0x20);
          *(undefined4 *)(*(int *)(iVar7 + 0x60) + local_14 * 4) = uVar5;
          puVar9 = (undefined4 *)(*(int *)(iVar7 + 100) + local_18);
          local_18 = local_18 + 0x84;
          local_a0[0] = uVar5;
          local_14 = local_14 + 1;
          puVar8 = local_a0;
          for (iVar7 = 0x21; iVar7 != 0; iVar7 = iVar7 + -1) {
            *puVar9 = *puVar8;
            puVar8 = puVar8 + 1;
            puVar9 = puVar9 + 1;
          }
          *(int *)(iVar1 + 0x10ac8) = *(int *)(iVar1 + 0x10ac8) + 1;
          iVar4 = FUN_3955fb10(param_2);
          iVar7 = local_1c;
        } while (iVar4 == 100);
      }
      if (iVar4 == 0x65) {
        FUN_396bf810(local_14);
        if (param_2 != 0) {
          FUN_395519b0(param_2);
        }
        ExceptionList = local_10;
        return 1;
      }
      uVar5 = FUN_395338f0(param_1);
      FUN_3967bbc0("failed: sqlite3_step: function=LoadHoldableDescs errcode=%d errmsg=%s",iVar4,
                   uVar5);
      if (param_2 != 0) {
        FUN_395519b0(param_2);
        ExceptionList = local_10;
        return 0;
      }
    }
    else {
      uVar5 = FUN_395338f0(param_1);
      X2Log(4,"failed: sqlite3_prepare_v2: sql=\'%s\' errcode=%d errmsg=%s",
            "SELECT id, angle, anim_l1_ratio, anim_l1_id, anim_l2_ratio, anim_l2_id, anim_l3_id, anim_r1_ratio, anim_r1_id, anim_r2_ratio, anim_r2_id, anim_r3_id, damage_scale, durability_ratio, enchanted_dps1000, extra_damage_blunt_factor, extra_damage_pierce_factor, extra_damage_slash_factor, formula_armor, formula_dps, formula_hdps, formula_mdps, item_proc_id, max_range, min_range, name, pose_id, renew_category, sheathe_priority, slot_type_id, sound_material_id, speed, stat_multiplier FROM holdables"
            ,iVar4,uVar5);
      if (param_2 != 0) {
        FUN_395519b0(param_2);
      }
    }
  }
  ExceptionList = local_10;
  return 0;
}


// ===== FUN_3971dee0 @ 3971dee0 (asked 3971dee0) size=380

undefined4 FUN_3971dee0(undefined4 param_1,int param_2)

{
  char cVar1;
  uint uVar2;
  int iVar3;
  undefined4 uVar4;
  int iVar5;
  undefined4 uVar6;
  int local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;

  local_8 = 0xffffffff;
  puStack_c = &LAB_3997f128;
  local_10 = ExceptionList;
  uVar2 = DAT_39acb9c0 ^ (uint)&stack0xfffffffc;
  ExceptionList = &local_10;
  cVar1 = FUN_396ff660(param_1,"instrument_sounds");
  if (cVar1 != '\0') {
    local_14 = 0;
    local_8 = 0;
    iVar3 = FUN_3955b3d0(param_1,"SELECT item_id, midi FROM instrument_sounds",0xffffffff,&local_14,
                         0,uVar2);
    if (iVar3 == 0) {
      iVar3 = 0;
      iVar5 = FUN_3955fb10(local_14);
      while (iVar5 == 100) {
        uVar4 = FUN_3953a390(local_14,0);
        uVar6 = FUN_3953a390(local_14,1);
        *(undefined4 *)(*(int *)(param_2 + 0xe48) + iVar3 * 4) = uVar4;
        *(undefined4 *)(*(int *)(param_2 + 0xe4c) + iVar3 * 4) = uVar6;
        *(int *)(param_2 + 0x10ac8) = *(int *)(param_2 + 0x10ac8) + 1;
        iVar3 = iVar3 + 1;
        iVar5 = FUN_3955fb10(local_14);
      }
      if (iVar5 == 0x65) {
        FUN_39713a50(iVar3);
        if (local_14 != 0) {
          FUN_395519b0(local_14);
        }
        ExceptionList = local_10;
        return 1;
      }
      uVar4 = FUN_395338f0(param_1);
      FUN_396dcb10("failed: sqlite3_step: function=LoadInstrumentSoundDescs errcode=%d errmsg=%s",
                   iVar5,uVar4);
      if (local_14 != 0) {
        FUN_395519b0(local_14);
        ExceptionList = local_10;
        return 0;
      }
    }
    else {
      uVar4 = FUN_395338f0(param_1);
      X2Log(4,"failed: sqlite3_prepare_v2: sql=\'%s\' errcode=%d errmsg=%s",
            "SELECT item_id, midi FROM instrument_sounds",iVar3,uVar4);
      if (local_14 != 0) {
        FUN_395519b0(local_14);
      }
    }
  }
  ExceptionList = local_10;
  return 0;
}


// ===== FUN_39695dc0 @ 39695dc0 (asked 39695dc0) size=62

undefined4 FUN_39695dc0(uint param_1)

{
  uint *puVar1;

  for (puVar1 = *(uint **)(PTR_DAT_39acb0e8 + 0x386c);
      (puVar1 != *(uint **)(PTR_DAT_39acb0e8 + 0x3870) && (*puVar1 <= param_1)); puVar1 = puVar1 + 3
      ) {
    if (param_1 <= puVar1[1]) {
      return *(undefined4 *)(puVar1[2] + (param_1 - *puVar1) * 4);
    }
  }
  return 0;
}


// ===== FUN_39703b50 @ 39703b50 (asked 39703b50) size=63

undefined4 FUN_39703b50(undefined4 param_1)

{
  undefined *puVar1;
  int local_c;
  undefined4 local_8;

  puVar1 = PTR_DAT_39acb0e8;
  local_8 = param_1;
  FUN_396f6fb0(&local_c,&local_8);
  if (local_c == *(int *)(puVar1 + 0xdf4)) {
    return 0;
  }
  return *(undefined4 *)(local_c + 0x10);
}
